from __future__ import annotations

import math

from .. import __version__
from .spec import (
    JSONRPC_VERSION,
    PROTOCOL_VERSION,
    SERVER_NAME,
    initialize_capabilities,
)


def jsonrpc_error(
    request_id: object,
    code: int,
    message: str,
    *,
    data: dict[str, object] | None = None,
) -> dict[str, object]:
    error: dict[str, object] = {"code": code, "message": message}
    if data is not None:
        error["data"] = data
    return {
        "jsonrpc": JSONRPC_VERSION,
        "id": request_id,
        "error": error,
    }


def valid_request_id(value: object) -> bool:
    if value is None or isinstance(value, str):
        return True
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    return not isinstance(value, float) or math.isfinite(value)


def initialize_result() -> dict[str, object]:
    return {
        "protocolVersion": PROTOCOL_VERSION,
        "server": {"name": SERVER_NAME, "version": __version__},
        "capabilities": initialize_capabilities(),
    }


def configuration_write_values(method: object, params: object) -> tuple[str, ...]:
    if method not in {"provider.configure", "provider.discover_models"} or not isinstance(
        params, dict
    ):
        return ()
    values: list[str] = []
    api_key = params.get("apiKey")
    if isinstance(api_key, str) and api_key:
        values.append(api_key)
    headers = params.get("headers")
    if isinstance(headers, dict):
        values.extend(value for value in headers.values() if isinstance(value, str) and value)
    return tuple(dict.fromkeys(values))
