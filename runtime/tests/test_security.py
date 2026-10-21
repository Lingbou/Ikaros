"""Credential redaction for Runtime log sinks."""

from __future__ import annotations

import logging

from ikaros_runtime.security import (
    ProtectedValueRedactionFilter,
    install_protected_value_redaction,
)

_SECRET = "8990669018:AAGueT781iRF9L3UCd2QjqqqDse1IQlN5HU"


def _record(message: str, *args: object) -> logging.LogRecord:
    return logging.LogRecord(
        name="httpx",
        level=logging.INFO,
        pathname=__file__,
        lineno=1,
        msg=message,
        args=args,
        exc_info=None,
    )


def test_filter_rewrites_the_bot_token_out_of_request_urls() -> None:
    record = _record(
        'HTTP Request: POST https://api.telegram.org/bot%s/getMe "HTTP/1.1 200 OK"',
        _SECRET,
    )

    assert ProtectedValueRedactionFilter(lambda: (_SECRET,)).filter(record) is True

    message = record.getMessage()
    assert _SECRET not in message
    assert "https://api.telegram.org/bot***/getMe" in message


def test_install_is_idempotent_and_leaves_clean_records_untouched() -> None:
    handler = logging.Handler()
    install_protected_value_redaction(lambda: (_SECRET,), handlers=(handler,))
    install_protected_value_redaction(lambda: (_SECRET,), handlers=(handler,))

    assert len(handler.filters) == 1
    record = _record("plain message without secrets")
    assert handler.filter(record)
    assert record.getMessage() == "plain message without secrets"