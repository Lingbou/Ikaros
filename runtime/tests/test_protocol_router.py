from __future__ import annotations

from typing import Any, cast

import pytest

from ikaros_runtime.errors import (
    InvalidParamsError,
    MemoryOperationError,
    MemoryOperationReasonCode,
)
from ikaros_runtime.protocol.jsonrpc import jsonrpc_error
from ikaros_runtime.protocol.router import RuntimeRouter
from ikaros_runtime.protocol.spec import (
    MEMORY_OPERATION_ERROR_CODE,
    MEMORY_OPERATION_ERROR_MESSAGE,
    MEMORY_OPERATION_REASON_CODES,
)
from ikaros_runtime.services.memories import MemoryService
from ikaros_runtime.services.providers import ProviderService
from ikaros_runtime.services.skills import SkillService
from ikaros_runtime.services.threads import ThreadService
from ikaros_runtime.services.turns import TurnService
from ikaros_runtime.services.usage import UsageService


class _MemoryServiceStub:
    def __init__(self, failure: Exception | None = None) -> None:
        self.failure = failure
        self.calls: list[tuple[str, dict[str, Any]]] = []

    def correct(self, params: dict[str, Any]) -> dict[str, object]:
        return self._mutate("correct", params)

    def forget(self, params: dict[str, Any]) -> dict[str, object]:
        return self._mutate("forget", params)

    def _mutate(self, name: str, params: dict[str, Any]) -> dict[str, object]:
        self.calls.append((name, params))
        if self.failure is not None:
            raise self.failure
        return {"memoryId": "memory_00000000000000000000000000000000"}


def _router(memories: _MemoryServiceStub) -> RuntimeRouter:
    return RuntimeRouter(
        cast(ThreadService, object()),
        cast(TurnService, object()),
        cast(ProviderService, object()),
        cast(UsageService, object()),
        cast(SkillService, object()),
        cast(MemoryService, memories),
    )


def test_jsonrpc_error_includes_only_explicit_structured_data() -> None:
    assert jsonrpc_error(1, -32602, "invalid") == {
        "jsonrpc": "2.0",
        "id": 1,
        "error": {"code": -32602, "message": "invalid"},
    }
    assert jsonrpc_error(
        2,
        MEMORY_OPERATION_ERROR_CODE,
        MEMORY_OPERATION_ERROR_MESSAGE,
        data={"reasonCode": "memory_forgotten"},
    ) == {
        "jsonrpc": "2.0",
        "id": 2,
        "error": {
            "code": MEMORY_OPERATION_ERROR_CODE,
            "message": MEMORY_OPERATION_ERROR_MESSAGE,
            "data": {"reasonCode": "memory_forgotten"},
        },
    }


@pytest.mark.asyncio
@pytest.mark.parametrize("method", ["memory.correct", "memory.forget"])
async def test_memory_mutation_methods_dispatch_to_the_memory_service(method: str) -> None:
    memories = _MemoryServiceStub()
    routed = await _router(memories).dispatch(6, method, {"sentinel": True})

    assert routed.response == {
        "jsonrpc": "2.0",
        "id": 6,
        "result": {"memoryId": "memory_00000000000000000000000000000000"},
    }
    assert memories.calls == [(method.removeprefix("memory."), {"sentinel": True})]


@pytest.mark.asyncio
@pytest.mark.parametrize("method", ["memory.correct", "memory.forget"])
@pytest.mark.parametrize("reason_code", MEMORY_OPERATION_REASON_CODES)
async def test_memory_operation_errors_use_one_stable_safe_envelope(
    method: str,
    reason_code: MemoryOperationReasonCode,
) -> None:
    memories = _MemoryServiceStub(MemoryOperationError(reason_code))
    routed = await _router(memories).dispatch(7, method, {"sentinel": True})

    assert routed.response == {
        "jsonrpc": "2.0",
        "id": 7,
        "error": {
            "code": MEMORY_OPERATION_ERROR_CODE,
            "message": MEMORY_OPERATION_ERROR_MESSAGE,
            "data": {"reasonCode": reason_code},
        },
    }
    assert memories.calls == [(method.removeprefix("memory."), {"sentinel": True})]


def test_memory_operation_error_rejects_unregistered_reason_codes() -> None:
    with pytest.raises(ValueError, match="unsupported Memory operation reason code"):
        MemoryOperationError(cast(MemoryOperationReasonCode, "memory_unknown"))


@pytest.mark.asyncio
@pytest.mark.parametrize("method", ["memory.correct", "memory.forget"])
async def test_memory_invalid_params_remain_standard_invalid_params(method: str) -> None:
    memories = _MemoryServiceStub(InvalidParamsError("invalid memory parameters"))
    routed = await _router(memories).dispatch(8, method, {})

    assert routed.response == {
        "jsonrpc": "2.0",
        "id": 8,
        "error": {"code": -32602, "message": "invalid memory parameters"},
    }
