from __future__ import annotations

from pathlib import Path
from typing import Any

from ..errors import InvalidParamsError
from ..storage.store import SqliteRuntimeStore
from ..tools.process_log import ProcessLog, ProcessLogError
from ..tools.process_manager import ProcessError, ProcessManager, read_fact_page


class ProcessService:
    def __init__(
        self,
        manager: ProcessManager,
        store: SqliteRuntimeStore | None = None,
        *,
        logs_root: Path | None = None,
    ) -> None:
        self._manager = manager
        self._store = store
        self._logs_root = logs_root

    def _params(self, params: dict[str, Any], *, cursor: bool) -> tuple[str, str, int]:
        allowed = {"threadId", "processId", "cursor"} if cursor else {"threadId", "processId"}
        if set(params) - allowed or not {"threadId", "processId"} <= set(params):
            raise InvalidParamsError("process control fields are invalid")
        thread_id, process_id = params["threadId"], params["processId"]
        value = params.get("cursor", 0)
        if (
            not isinstance(thread_id, str)
            or not thread_id
            or not isinstance(process_id, str)
            or not process_id
        ):
            raise InvalidParamsError("threadId and processId must be non-empty strings")
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise InvalidParamsError("cursor must be a non-negative integer")
        return thread_id, process_id, value

    def read(self, params: dict[str, Any]) -> dict[str, object]:
        thread_id, process_id, cursor = self._params(params, cursor=True)
        try:
            return self._manager.read_for_thread(process_id, thread_id=thread_id, cursor=cursor)
        except ProcessError as error:
            fact = self._persisted_fact(thread_id, process_id)
            if fact is None:
                raise InvalidParamsError(str(error)) from error
            log = self._log_for(thread_id, process_id)
            try:
                page, next_cursor, has_more = read_fact_page(fact, cursor, log=log)
            except IndexError:
                raise InvalidParamsError(
                    "cursor is outside the retained command output."
                ) from error
            finally:
                if log is not None:
                    log.close()
            return {
                **fact,
                "output": page,
                "cursor": cursor,
                "nextCursor": next_cursor,
                "hasMore": has_more,
            }

    def _persisted_fact(self, thread_id: str, process_id: str) -> dict[str, Any] | None:
        if self._store is None:
            return None
        records = [
            record
            for record in self._store.process_records()
            if record.get("processId") == process_id and record.get("threadId") == thread_id
        ]
        return records[-1] if records else None

    def _log_for(self, thread_id: str, process_id: str) -> ProcessLog | None:
        if self._logs_root is None:
            return None
        try:
            return ProcessLog.open(self._logs_root, thread_id, process_id)
        except (ProcessLogError, OSError):
            return None

    async def stop(self, params: dict[str, Any]) -> dict[str, object]:
        thread_id, process_id, _ = self._params(params, cursor=False)
        try:
            return await self._manager.stop_for_thread(process_id, thread_id=thread_id)
        except ProcessError as error:
            raise InvalidParamsError(str(error)) from error
