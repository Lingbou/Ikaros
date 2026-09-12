from __future__ import annotations

from typing import Any

from ..errors import InvalidParamsError
from ..tools.process_manager import ProcessError, ProcessManager


class ProcessService:
    def __init__(self, manager: ProcessManager) -> None:
        self._manager = manager

    def _params(self, params: dict[str, Any], *, cursor: bool) -> tuple[str, str, int]:
        allowed = {"threadId", "processId", "cursor"} if cursor else {"threadId", "processId"}
        if set(params) - allowed or not {"threadId", "processId"} <= set(params):
            raise InvalidParamsError("process control fields are invalid")
        thread_id, process_id = params["threadId"], params["processId"]
        value = params.get("cursor", 0)
        if not isinstance(thread_id, str) or not thread_id or not isinstance(process_id, str) or not process_id:
            raise InvalidParamsError("threadId and processId must be non-empty strings")
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise InvalidParamsError("cursor must be a non-negative integer")
        return thread_id, process_id, value

    def read(self, params: dict[str, Any]) -> dict[str, object]:
        thread_id, process_id, cursor = self._params(params, cursor=True)
        try:
            return self._manager.read_for_thread(process_id, thread_id=thread_id, cursor=cursor)
        except ProcessError as error:
            raise InvalidParamsError(str(error)) from error

    async def stop(self, params: dict[str, Any]) -> dict[str, object]:
        thread_id, process_id, _ = self._params(params, cursor=False)
        try:
            return await self._manager.stop_for_thread(process_id, thread_id=thread_id)
        except ProcessError as error:
            raise InvalidParamsError(str(error)) from error

