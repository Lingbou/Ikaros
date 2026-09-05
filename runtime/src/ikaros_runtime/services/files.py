"""Thread-bound, read-only file access for the Desktop inspector."""

from __future__ import annotations

import asyncio
import re
from pathlib import Path
from typing import Any

from ..domain import JsonObject
from ..errors import InvalidParamsError
from ..file_preview import preview_text_file, unavailable_preview
from ..paths import RuntimePaths
from ..security import RuntimeSecurity
from ..storage import SqliteRuntimeStore
from ..tools.file_common import FileToolError, validate_file_path
from .threads import record_id_from


class FileService:
    def __init__(
        self,
        store: SqliteRuntimeStore,
        security: RuntimeSecurity,
        paths: RuntimePaths,
    ) -> None:
        self._store = store
        self._security = security
        self._paths = paths

    async def preview(self, params: dict[str, Any]) -> JsonObject:
        if not {"threadId", "path"} <= set(params) or set(params) - {
            "threadId",
            "path",
            "sourceToolCallItemId",
            "offset",
            "expectedRevision",
        }:
            raise InvalidParamsError("file.preview requires Thread, path and optional pagination")
        thread_id = record_id_from(params["threadId"], name="threadId")
        offset = params.get("offset", 1)
        expected = params.get("expectedRevision")
        if type(offset) is not int or not 1 <= offset <= 9_007_199_254_740_991:
            raise InvalidParamsError("file.preview offset must be a positive integer")
        if (
            "expectedRevision" in params
            and (not isinstance(expected, str) or re.fullmatch(r"[a-f0-9]{64}", expected) is None)
        ) or (offset > 1 and expected is None):
            raise InvalidParamsError("later file pages require a valid expectedRevision")
        self._security.assert_request_safe(params)
        try:
            thread = self._store.get_thread(thread_id)
            workspace = thread.thread.workspace
            recorded_path: str | None = None
            if "sourceToolCallItemId" in params:
                source_id = record_id_from(
                    params["sourceToolCallItemId"], name="sourceToolCallItemId"
                )
                recorded_path = self._store.resolve_file_tool_path(thread_id, source_id)
            path = await asyncio.to_thread(
                _resolve_path,
                workspace.root_uri if workspace else None,
                params["path"],
                recorded_path,
            )
        except (LookupError, FileToolError, OSError, ValueError, RuntimeError):
            raise InvalidParamsError("file path or source tool call is unavailable") from None
        if self._security.response_contains_protected_value(str(path)):
            return unavailable_preview(thread_id, None, "protected_content")
        if await asyncio.to_thread(self._is_internal_file, path):
            return unavailable_preview(thread_id, str(path), "protected_content")
        return await asyncio.to_thread(
            preview_text_file,
            thread_id=thread_id,
            path=path,
            offset=offset,
            expected_revision=expected,
            protected_values=self._security.protected_values(),
        )

    def get_change(self, params: dict[str, Any]) -> JsonObject:
        if set(params) != {"threadId", "toolCallItemId"}:
            raise InvalidParamsError("file.change.get requires Thread and toolCallItemId")
        thread_id = record_id_from(params["threadId"], name="threadId")
        item_id = record_id_from(params["toolCallItemId"], name="toolCallItemId")
        self._security.assert_request_safe(params)
        try:
            return self._store.get_file_change(thread_id, item_id)
        except (LookupError, ValueError):
            raise InvalidParamsError("file change source tool call is unavailable") from None

    def _is_internal_file(self, path: Path) -> bool:
        if path.is_relative_to(self._paths.home / "backups"):
            return True
        for owned in (self._paths.config, self._paths.state_db, self._paths.memory_db):
            aliases = {str(owned) + suffix for suffix in ("-wal", "-shm", "-journal")}
            if path == owned or str(path) in aliases:
                return True
            try:
                if path.samefile(owned):
                    return True
            except OSError:
                pass
        return False


def _resolve_path(root_uri: str | None, raw_path: object, recorded_path: str | None) -> Path:
    root = Path(root_uri).resolve() if root_uri else None
    if root is None and isinstance(raw_path, str) and not Path(raw_path).expanduser().is_absolute():
        raise InvalidParamsError("a relative file path requires a saved workspace")
    path = validate_file_path(raw_path, default_cwd=str(root) if root else None)
    if recorded_path is not None:
        # The recorded tool result already contains the resolved path. Resolving
        # it again would authorize a symlink that was retargeted after the tool.
        if Path(recorded_path) != path:
            raise InvalidParamsError("file path does not match its source tool call")
    elif root is None or not path.is_relative_to(root):
        raise InvalidParamsError("a file outside the workspace requires a recorded tool call")
    return path
