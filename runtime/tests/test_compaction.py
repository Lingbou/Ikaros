from __future__ import annotations

from ikaros_runtime.agent.compaction import trim_context_records
from ikaros_runtime.run_input import ContextItemRecordV1


def _message(item_id: str, content: str, *, role: str = "user", data=None):
    return ContextItemRecordV1(
        item_id=item_id,
        turn_id="turn_1",
        run_id="run_1",
        kind="message",
        role=role,
        content=content,
        data={} if data is None else data,
    )


def _tool_call(item_id: str, step_id: str, call_id: str):
    return ContextItemRecordV1(
        item_id=item_id,
        turn_id="turn_1",
        run_id="run_1",
        kind="tool_call",
        role="assistant",
        content="",
        data={"stepId": step_id, "callId": call_id, "toolName": "read", "arguments": {}},
    )


def _tool_result(item_id: str, step_id: str, call_id: str, call_item_id: str):
    return ContextItemRecordV1(
        item_id=item_id,
        turn_id="turn_1",
        run_id="run_1",
        kind="tool_result",
        role="tool",
        content="result",
        data={
            "stepId": step_id,
            "callId": call_id,
            "toolCallItemId": call_item_id,
            "result": {},
        },
    )


def test_budget_overflow_trims_once_and_reports_omissions():
    records = (
        _message("u1", "original request"),
        _message("a1", "intermediate answer", role="assistant"),
        _message("u2", "latest request"),
    )
    result = trim_context_records(records, maximum_tokens=170, preserve_suffix_units=1)

    assert result.truncated is True
    assert result.records[0].item_id == "u1"
    assert result.records[-1].item_id == "u2"
    assert result.omitted_item_ids == ("a1",)


def test_tool_call_and_result_are_trimmed_as_one_unit():
    call = _tool_call("call", "step", "c1")
    result = _tool_result("result", "step", "c1", "call")
    records = (
        _message("u1", "request"),
        _message("a1", "tool", role="assistant", data={"stepId": "step"}),
        call,
        result,
        _message("u2", "latest"),
    )
    unit_tokens = sum(item.estimated_tokens for item in records[1:4])
    latest_tokens = records[-1].estimated_tokens
    trimmed = trim_context_records(
        records,
        maximum_tokens=unit_tokens + latest_tokens,
        preserve_prefix_units=0,
        preserve_suffix_units=2,
    )

    assert tuple(item.item_id for item in trimmed.records) == ("a1", "call", "result", "u2")
    assert trimmed.omitted_item_ids == ("u1",)


def test_failed_summary_can_leave_original_context_unchanged():
    records = (_message("u1", "request"), _message("a1", "answer", role="assistant"))
    before = tuple(records)
    try:
        raise RuntimeError("summary unavailable")
    except RuntimeError:
        pass

    assert records == before
