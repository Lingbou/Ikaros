from ikaros_runtime.agent.compaction import trim_context_records
from ikaros_runtime.run_input import ContextItemRecordV1


def rec(i: str, content: str, *, kind='message', role='user', data=None):
    return ContextItemRecordV1(i, 'turn', 'run', kind, role, content, data or {})


def test_trim_keeps_head_and_newest_units():
    records = tuple(rec(str(i), 'x' * 200) for i in range(8))
    result = trim_context_records(records, maximum_tokens=300, preserve_prefix_units=1, preserve_suffix_units=2)
    assert result.truncated
    assert result.records[0].item_id == '0'
    assert result.records[-1].item_id == '7'
    assert result.omitted_item_ids
    assert result.retained_tokens <= 300


def test_trim_keeps_tool_exchange_together():
    records = (
        rec('u', 'request'),
        rec('a', '', role='assistant', data={'stepId': 's'}),
        rec('c', '', kind='tool_call', role='assistant', data={'stepId': 's', 'callId': 'c', 'toolName': 'x', 'arguments': {}}),
        rec('r', 'result', kind='tool_result', role='tool', data={'stepId': 's', 'callId': 'c', 'result': {}}),
        rec('z', 'latest'),
    )
    result = trim_context_records(records, maximum_tokens=500, preserve_prefix_units=1, preserve_suffix_units=1)
    ids = {item.item_id for item in result.records}
    assert {'a', 'c', 'r'} <= ids or not result.truncated
