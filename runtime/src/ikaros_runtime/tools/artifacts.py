"""Spill oversized Tool results into per-Thread artifact files."""

from __future__ import annotations

import asyncio
import re
from collections.abc import Callable
from dataclasses import dataclass, field, replace
from pathlib import Path

from ..domain import JsonObject
from .core import ToolResult
from .file_common import atomic_write_bytes

MAX_INLINE_RESULT_BYTES = 64 * 1024
PREVIEW_LINE_COUNT = 200
_PREVIEW_MAX_BYTES = 32 * 1024
_UNSAFE_SEGMENT = re.compile(r"[^A-Za-z0-9_-]+")
_MAX_SEGMENT_LENGTH = 128


def _always_safe(text: str) -> bool:
    del text
    return True


@dataclass(frozen=True, slots=True)
class ToolArtifactStore:
    """Write oversized Tool results to ``<root>/<threadId>/<itemId>.txt``.

    The Tool result keeps a bounded preview of the head so the model can continue with
    ``read`` and the exact line offset it needs, while the full output stays on disk.
    Results that fail the ``is_safe`` predicate, such as output carrying protected
    credential values, are never written and stay on the existing rejection path.
    """

    root: Path
    is_safe: Callable[[str], bool] = field(default=_always_safe)

    async def materialize(
        self,
        result: ToolResult,
        *,
        thread_id: str,
        item_id: str,
    ) -> ToolResult:
        payload = result.output.encode("utf-8")
        if len(payload) <= MAX_INLINE_RESULT_BYTES or not self.is_safe(result.output):
            return result
        preview, preview_lines, total_lines = build_preview(result.output)
        path = self.root / safe_segment(thread_id) / f"{safe_segment(item_id)}.txt"
        try:
            await asyncio.to_thread(atomic_write_bytes, path, payload)
        except OSError:
            return result
        notice = (
            f"(Showing the first {preview_lines} of {total_lines} lines; the full result is "
            f"saved at {path}. Continue with read offset={preview_lines + 1}.)"
        )
        details: JsonObject = {
            **result.details,
            "artifactPath": str(path),
            "artifactBytes": len(payload),
            "artifactLines": total_lines,
            "artifactPreviewLines": preview_lines,
            "truncated": True,
        }
        return replace(
            result,
            output=f"{preview}\n\n{notice}" if preview else notice,
            details=details,
        )


def build_preview(text: str) -> tuple[str, int, int]:
    """Return the preview head, its line count, and the total line count."""

    source_lines = text.splitlines()
    preview_lines: list[str] = []
    retained_bytes = 0
    for line in source_lines:
        if len(preview_lines) >= PREVIEW_LINE_COUNT or retained_bytes >= _PREVIEW_MAX_BYTES:
            break
        encoded_size = len(line.encode("utf-8")) + 1
        if retained_bytes + encoded_size > _PREVIEW_MAX_BYTES:
            allowance = _PREVIEW_MAX_BYTES - retained_bytes
            if allowance > 0:
                preview_lines.append(
                    _truncate_to_bytes(line, allowance) + "... (preview line truncated)"
                )
            break
        preview_lines.append(line)
        retained_bytes += encoded_size
    return "\n".join(preview_lines), len(preview_lines), len(source_lines)


def safe_segment(value: str) -> str:
    segment = _UNSAFE_SEGMENT.sub("_", value).strip("._-")
    return segment[:_MAX_SEGMENT_LENGTH] or "unknown"


def _truncate_to_bytes(line: str, limit: int) -> str:
    return line.encode("utf-8")[:limit].decode("utf-8", errors="ignore")
