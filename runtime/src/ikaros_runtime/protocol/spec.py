from __future__ import annotations

from typing import Final

from ..errors import MEMORY_OPERATION_REASON_CODES

PROTOCOL_SPEC_SCHEMA_VERSION: Final = 2
PROTOCOL_VERSION: Final = 5
JSONRPC_VERSION: Final = "2.0"
SERVER_NAME: Final = "ikaros-runtime"
INITIALIZE_METHOD: Final = "initialize"
EVENT_NOTIFICATION_METHOD: Final = "event"
JOURNAL_EVENT_SCHEMA_VERSION: Final = 8
MEMORY_OPERATION_ERROR_CODE: Final = -32020
MEMORY_OPERATION_ERROR_MESSAGE: Final = "memory operation failed"

RPC_METHODS: Final = (
    "runtime.shutdown",
    "thread.create",
    "thread.rename",
    "thread.archive",
    "thread.unarchive",
    "thread.get",
    "thread.list",
    "provider.list",
    "provider.configure",
    "provider.discover_models",
    "provider.disconnect",
    "provider.remove",
    "model.list",
    "model.set_enabled",
    "model.set_limits",
    "skill.list",
    "skill.set_enabled",
    "memory.create",
    "memory.correct",
    "memory.forget",
    "memory.list",
    "memory.get",
    "file.preview",
    "file.change.get",
    "turn.start",
    "turn.list",
    "run.cancel",
    "process.read",
    "process.stop",
    "run.steer",
    "event.replay",
    "usage.read",
)
RPC_METHOD_SET: Final = frozenset(RPC_METHODS)

JOURNAL_EVENT_TYPES: Final = (
    "thread.created",
    "thread.renamed",
    "thread.archived",
    "thread.unarchived",
    "run.state_changed",
    "item.started",
    "item.delta",
    "item.completed",
    "file.change_recorded",
    "process.recorded",
    "model.input_prepared",
    "context.compacted",
    "model.response_finished",
    "run.settled",
    "run.steered",
)
JOURNAL_EVENT_TYPE_SET: Final = frozenset(JOURNAL_EVENT_TYPES)

# These are Provider-facing Tool IDs. Renderer labels such as ``process.run``
# are presentation details and deliberately do not belong to the wire contract.
PROVIDER_TOOL_IDS: Final = (
    "process_start",
    "process_read",
    "process_wait",
    "process_stop",
    "history_read",
    "read",
    "write",
    "edit",
)
PROVIDER_TOOL_ID_SET: Final = frozenset(PROVIDER_TOOL_IDS)
EXECUTION_POLICY: Final = "full_access"

CAPABILITY_FLAGS: Final = (
    ("threads", True),
    ("turns", True),
    ("eventReplay", True),
    ("streaming", True),
    ("scriptedProvider", True),
    ("runCancellation", True),
    ("providers", True),
    ("models", True),
    ("usage", True),
    ("skills", True),
    ("memory", True),
)


def initialize_capabilities() -> dict[str, object]:
    capabilities: dict[str, object] = dict(CAPABILITY_FLAGS)
    capabilities["tools"] = list(PROVIDER_TOOL_IDS)
    capabilities["executionPolicy"] = EXECUTION_POLICY
    return capabilities


def protocol_manifest() -> dict[str, object]:
    return {
        "schemaVersion": PROTOCOL_SPEC_SCHEMA_VERSION,
        "protocolVersion": PROTOCOL_VERSION,
        "jsonrpcVersion": JSONRPC_VERSION,
        "serverName": SERVER_NAME,
        "initializeMethod": INITIALIZE_METHOD,
        "rpcMethods": list(RPC_METHODS),
        "notificationMethods": [EVENT_NOTIFICATION_METHOD],
        "journal": {
            "schemaVersion": JOURNAL_EVENT_SCHEMA_VERSION,
            "eventTypes": list(JOURNAL_EVENT_TYPES),
        },
        "errors": {
            "memoryOperation": {
                "code": MEMORY_OPERATION_ERROR_CODE,
                "message": MEMORY_OPERATION_ERROR_MESSAGE,
                "reasonCodes": list(MEMORY_OPERATION_REASON_CODES),
            }
        },
        "capabilities": initialize_capabilities(),
        "providerToolIds": list(PROVIDER_TOOL_IDS),
    }


__all__ = [
    "CAPABILITY_FLAGS",
    "EVENT_NOTIFICATION_METHOD",
    "EXECUTION_POLICY",
    "INITIALIZE_METHOD",
    "JOURNAL_EVENT_SCHEMA_VERSION",
    "JOURNAL_EVENT_TYPES",
    "JOURNAL_EVENT_TYPE_SET",
    "JSONRPC_VERSION",
    "MEMORY_OPERATION_ERROR_CODE",
    "MEMORY_OPERATION_ERROR_MESSAGE",
    "MEMORY_OPERATION_REASON_CODES",
    "PROTOCOL_SPEC_SCHEMA_VERSION",
    "PROTOCOL_VERSION",
    "PROVIDER_TOOL_IDS",
    "PROVIDER_TOOL_ID_SET",
    "RPC_METHODS",
    "RPC_METHOD_SET",
    "SERVER_NAME",
    "initialize_capabilities",
    "protocol_manifest",
]
