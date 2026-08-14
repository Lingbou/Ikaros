from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Any

from ..domain import CommandOutcome, WorkspaceSummary
from ..errors import InvalidParamsError
from ..storage import SqliteRuntimeStore
from ..storage.thread_catalog import THREAD_CATALOG_DEFAULT_LIMIT, THREAD_CATALOG_MAX_LIMIT

RequestSafetyCheck = Callable[[object], None]
_MAX_WORKSPACE_ID_LENGTH = 200
_MAX_WORKSPACE_NAME_LENGTH = 200
_MAX_WORKSPACE_ROOT_LENGTH = 4096


class ThreadService:
    def __init__(self, store: SqliteRuntimeStore, assert_request_safe: RequestSafetyCheck) -> None:
        self._store = store
        self._assert_request_safe = assert_request_safe

    def create(self, params: dict[str, Any]) -> CommandOutcome:
        unknown = set(params) - {"title", "workspace", "clientRequestId"}
        if unknown:
            raise InvalidParamsError("thread.create contains unsupported parameters")
        title = params.get("title")
        if title is not None:
            if not isinstance(title, str):
                raise InvalidParamsError("title must be a string or null")
            title = title.strip()
            if not title:
                title = None
            elif len(title) > 200:
                raise InvalidParamsError("title must not exceed 200 characters")
        client_request_id = client_request_id_from(params)
        workspace = workspace_from_wire(params.get("workspace"))
        self._assert_request_safe(
            (title, workspace.to_wire() if workspace else None, client_request_id)
        )
        if client_request_id is None:
            thread, event = self._store.create_thread(title, workspace=workspace)
            created = True
        else:
            try:
                thread, event, created = self._store.create_thread_once(
                    title,
                    client_request_id,
                    workspace=workspace,
                )
            except LookupError as error:
                raise InvalidParamsError(str(error)) from error
        return CommandOutcome(
            result={"thread": thread.to_wire(), "event": event.to_wire()},
            events_after_ack=(event,) if created else (),
        )

    def list(self, params: dict[str, Any]) -> dict[str, Any]:
        unknown = set(params) - {"cursor", "limit"}
        if unknown:
            raise InvalidParamsError("thread.list contains unsupported parameters")
        cursor: str | None = None
        if "cursor" in params:
            raw_cursor = params["cursor"]
            if not isinstance(raw_cursor, str) or not raw_cursor:
                raise InvalidParamsError("thread.list cursor must be a non-empty string")
            cursor = raw_cursor
        limit = params.get("limit", THREAD_CATALOG_DEFAULT_LIMIT)
        if isinstance(limit, bool) or not isinstance(limit, int):
            raise InvalidParamsError("thread.list limit must be an integer")
        if limit < 1 or limit > THREAD_CATALOG_MAX_LIMIT:
            raise InvalidParamsError(
                f"thread.list limit must be between 1 and {THREAD_CATALOG_MAX_LIMIT}"
            )
        try:
            return self._store.list_thread_page(cursor=cursor, limit=limit).to_wire()
        except ValueError as error:
            raise InvalidParamsError(str(error)) from error


def client_request_id_from(params: dict[str, Any]) -> str | None:
    value = params.get("clientRequestId")
    if value is None:
        return None
    if not isinstance(value, str) or not value or len(value) > 200:
        raise InvalidParamsError(
            "clientRequestId must be a non-empty string of at most 200 characters"
        )
    return value


def workspace_from_wire(value: object) -> WorkspaceSummary | None:
    if value is None:
        return None
    if not isinstance(value, dict) or set(value) != {"id", "name", "rootUri"}:
        raise InvalidParamsError("workspace requires exactly id, name, and rootUri")
    workspace_id = value["id"]
    name = value["name"]
    root_uri = value["rootUri"]
    if not isinstance(workspace_id, str) or not isinstance(name, str):
        raise InvalidParamsError("workspace id and name must be strings")
    workspace_id = workspace_id.strip()
    name = name.strip()
    if not workspace_id or len(workspace_id) > _MAX_WORKSPACE_ID_LENGTH:
        raise InvalidParamsError(
            f"workspace id must be non-empty and at most {_MAX_WORKSPACE_ID_LENGTH} characters"
        )
    if not name or len(name) > _MAX_WORKSPACE_NAME_LENGTH:
        raise InvalidParamsError(
            f"workspace name must be non-empty and at most {_MAX_WORKSPACE_NAME_LENGTH} characters"
        )
    if root_uri is None:
        return WorkspaceSummary(workspace_id, name, None)
    if not isinstance(root_uri, str) or not root_uri or len(root_uri) > _MAX_WORKSPACE_ROOT_LENGTH:
        raise InvalidParamsError(
            "workspace rootUri must be null or a non-empty string of at most "
            f"{_MAX_WORKSPACE_ROOT_LENGTH} characters"
        )
    root = Path(root_uri)
    if not root.is_absolute() or not root.is_dir():
        raise InvalidParamsError("workspace rootUri must be an existing absolute directory")
    try:
        resolved_root = root.resolve(strict=True)
    except OSError:
        raise InvalidParamsError(
            "workspace rootUri must be an existing absolute directory"
        ) from None
    if not resolved_root.is_dir():
        raise InvalidParamsError("workspace rootUri must be an existing absolute directory")
    return WorkspaceSummary(workspace_id, name, str(resolved_root))
