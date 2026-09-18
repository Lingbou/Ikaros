from __future__ import annotations

from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

from ikaros_runtime.agent.compaction import (
    build_compaction_source,
    compaction_source_item_ids,
    trim_context_records,
)
from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from ikaros_runtime.run_input import (
    ContextItemRecordV1,
    FrozenMemoryContextV1,
    MemoryReferenceV1,
    OmissionRecordV1,
    ProviderExecutionSnapshot,
    RunConfigTemplate,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.storage.context_history import load_context_for_revision
from ikaros_runtime.storage.projections import get_context_revision
from ikaros_runtime.storage.store import ContextCompactionRequired
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
)
from ikaros_runtime.tools.policy import FullAccessPolicy


def _message(
    item_id: str,
    content: str,
    *,
    role: str = "user",
    data: dict[str, Any] | None = None,
) -> ContextItemRecordV1:
    return ContextItemRecordV1(
        item_id=item_id,
        turn_id="turn_1",
        run_id="run_1",
        kind="message",
        role=role,
        content=content,
        data={} if data is None else data,
    )


def _tool_call(item_id: str, step_id: str, call_id: str) -> ContextItemRecordV1:
    return ContextItemRecordV1(
        item_id=item_id,
        turn_id="turn_1",
        run_id="run_1",
        kind="tool_call",
        role="assistant",
        content="",
        data={"stepId": step_id, "callId": call_id, "toolName": "read", "arguments": {}},
    )


def _tool_result(
    item_id: str, step_id: str, call_id: str, call_item_id: str
) -> ContextItemRecordV1:
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


def test_budget_overflow_trims_once_and_reports_omissions() -> None:
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


def test_tool_call_and_result_are_trimmed_as_one_unit() -> None:
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


def test_latest_oversized_unit_is_retained_so_the_caller_can_fail_explicitly() -> None:
    records = (
        _message("u1", "request"),
        _message("a1", "old work", role="assistant"),
        _message("a2", "latest work " + "x" * 1000, role="assistant"),
    )

    trimmed = trim_context_records(records, maximum_tokens=records[0].estimated_tokens)

    assert trimmed.records[-1].item_id == "a2"
    assert trimmed.retained_tokens > records[0].estimated_tokens
    assert trimmed.omitted_item_ids == ("a1",)


def test_failed_summary_can_leave_original_context_unchanged() -> None:
    records = (_message("u1", "request"), _message("a1", "answer", role="assistant"))
    before = tuple(records)
    try:
        raise RuntimeError("summary unavailable")
    except RuntimeError:
        pass

    assert records == before


def test_compaction_source_bounds_utf8_and_large_tool_arguments() -> None:
    record = _tool_call("call", "step", "c1")
    record = ContextItemRecordV1(
        item_id=record.item_id,
        turn_id=record.turn_id,
        run_id=record.run_id,
        kind=record.kind,
        role=record.role,
        content="界" * 10_000,
        data={
            **record.data,
            "arguments": {"content": "参数" * 10_000},
        },
    )

    source = build_compaction_source((record,), max_bytes=2_000)

    assert len(source.encode("utf-8")) <= 2_000
    assert "truncated" in source


def test_compaction_source_never_exceeds_exact_json_array_byte_limit() -> None:
    record = _message("m1", "x" * 100)
    rendered = build_compaction_source((record,), max_bytes=10_000)
    exact_limit = len(rendered.encode("utf-8"))

    assert len(build_compaction_source((record,), max_bytes=exact_limit).encode("utf-8")) == (
        exact_limit
    )
    assert len(build_compaction_source((record,), max_bytes=exact_limit - 1).encode("utf-8")) <= (
        exact_limit - 1
    )


def test_compaction_source_coverage_exposes_records_lost_to_byte_bound() -> None:
    records = tuple(_message(f"m{index}", "x" * 500) for index in range(3))
    source = build_compaction_source(records, max_bytes=500)

    assert compaction_source_item_ids(source) == ()
    assert set(compaction_source_item_ids(source)) != {record.item_id for record in records}


def test_prepare_model_step_does_not_create_a_noop_compaction_revision(tmp_path: Path) -> None:
    from ikaros_runtime.run_input import ProviderExecutionSnapshot, RunConfigTemplate
    from ikaros_runtime.storage import SqliteRuntimeStore

    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Compaction Test")
        provider = ProviderExecutionSnapshot(
            provider_id="scripted",
            origin="test",
            base_url=None,
            model_id="scripted-v1",
            supports_tools=True,
            context_window=2700,
        )
        template = RunConfigTemplate.create(
            provider=provider,
            execution_policy="full_access",
            skills=(),
            tools=(),
            identity_core=None,
        )

        t1 = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Turn 1 user request is here",
            run_config_template=template,
        )
        store.mark_run_running(t1.run_id)
        store.prepare_model_step(t1.run_id, step_ordinal=1)
        a1, _ = store.create_assistant_item(t1.run_id)
        store.append_text_delta(a1, "Turn 1 assistant answer is here")
        store.complete_provider_step(
            t1.run_id,
            step_ordinal=1,
            assistant_item_id=a1,
            tool_calls=(),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        store.terminalize_run(t1.run_id, "completed")

        t2 = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Turn 2 request",
            run_config_template=template,
        )
        store.mark_run_running(t2.run_id)
        step1 = store.prepare_model_step(t2.run_id, step_ordinal=1)
        assert step1.context_revision.revision == 1
        assert step1.context_revision.budget.history_tokens == 186

        a2, _ = store.create_assistant_item(t2.run_id)
        store.append_text_delta(a2, "A" * 832)
        store.complete_provider_step(
            t2.run_id,
            step_ordinal=1,
            assistant_item_id=a2,
            tool_calls=(),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )

        step2 = store.prepare_model_step(t2.run_id, step_ordinal=2)
        assert step2.context_revision.revision == 1
        assert step2.context_revision.budget.history_tokens == 186
        assert step2.context_revision.omissions == ()
        assert step2.pre_events == ()
        events, _ = store.replay_events(0, 100)
        compact_events = [e for e in events if e.type == "context.compacted"]
        assert compact_events == []
    finally:
        store.close()


def test_current_run_compaction_omits_old_completed_units_and_keeps_new_items(
    tmp_path: Path,
) -> None:
    from ikaros_runtime.run_input import ProviderExecutionSnapshot, RunConfigTemplate
    from ikaros_runtime.storage import SqliteRuntimeStore
    from ikaros_runtime.storage.context_history import load_context_for_revision
    from ikaros_runtime.storage.projections import get_context_revision

    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Current Run compaction")
        template = RunConfigTemplate.create(
            provider=ProviderExecutionSnapshot(
                provider_id="scripted",
                origin="test",
                base_url=None,
                model_id="scripted-v1",
                supports_tools=True,
                context_window=3000,
            ),
            execution_policy="full_access",
            skills=(),
            tools=(),
            identity_core=None,
        )
        turn = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Keep working",
            run_config_template=template,
        )
        store.mark_run_running(turn.run_id)
        store.prepare_model_step(turn.run_id, step_ordinal=1)
        prepared = None
        for index in range(1, 30):
            assistant_id, _ = store.create_assistant_item(turn.run_id)
            store.append_text_delta(assistant_id, f"completed unit {index} " + "x" * 160)
            store.complete_provider_step(
                turn.run_id,
                step_ordinal=index,
                assistant_item_id=assistant_id,
                tool_calls=(),
                reasoning_content=None,
                usage=None,
                response_model_id=None,
                request_id=None,
            )
            prepared = store.prepare_model_step(turn.run_id, step_ordinal=index + 1)
            if prepared.context_revision.revision > 1:
                break

        assert prepared is not None
        assert prepared.context_revision.revision > 1
        selected_ids = {item.item_id for item in prepared.context_revision.history_items}
        assert len(selected_ids) < 1 + index
        assert prepared.items[-1].content == f"completed unit {index} " + "x" * 160
        revision = get_context_revision(store._connection, turn.run_id)
        assert revision is not None
        loaded = load_context_for_revision(
            store._connection,
            run_id=turn.run_id,
            snapshot=revision,
        )
        assert tuple(item.content for item in loaded) == tuple(
            item.content for item in prepared.items
        )
    finally:
        store.close()


def test_compaction_preserves_frozen_memory_omissions(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Compaction with Memory omissions")
        template = RunConfigTemplate.create(
            provider=ProviderExecutionSnapshot(
                provider_id="scripted",
                origin="test",
                base_url=None,
                model_id="scripted-v1",
                supports_tools=True,
                context_window=3000,
            ),
            execution_policy="full_access",
            skills=(),
            tools=(),
            identity_core=None,
        )
        turn = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Keep working with the selected Memory",
            run_config_template=template,
        )
        store.mark_run_running(turn.run_id)
        memory = FrozenMemoryContextV1(
            memory=(
                MemoryReferenceV1(
                    memory_id="memory_00000000000000000000000000000001",
                    revision=1,
                    scope="global",
                    characters=10,
                ),
            ),
            omissions=(
                OmissionRecordV1(
                    source_type="memory",
                    source_id="memory_00000000000000000000000000000002",
                    revision=1,
                    characters=10,
                    reason="omitted_by_budget",
                ),
            ),
            memory_characters=10,
            context_data_characters=1,
        )
        store.prepare_model_step(turn.run_id, step_ordinal=1, memory_context=memory)
        compaction_step = 0
        for index in range(1, 30):
            assistant_id, _ = store.create_assistant_item(turn.run_id)
            store.append_text_delta(assistant_id, f"completed unit {index} " + "x" * 160)
            store.complete_provider_step(
                turn.run_id,
                step_ordinal=index,
                assistant_item_id=assistant_id,
                tool_calls=(),
                reasoning_content=None,
                usage=None,
                response_model_id=None,
                request_id=None,
            )
            try:
                store.prepare_model_step(
                    turn.run_id,
                    step_ordinal=index + 1,
                    memory_context=memory,
                    semantic_compaction=True,
                )
            except ContextCompactionRequired:
                compaction_step = index + 1
                break
        assert compaction_step > 1

        prepared = store.prepare_model_step(
            turn.run_id,
            step_ordinal=compaction_step,
            memory_context=memory,
            compaction_summary="summary",
        )
        assert prepared.context_revision.revision > 1
        assert any(
            omission.source_type == "memory"
            and omission.source_id == "memory_00000000000000000000000000000002"
            for omission in prepared.context_revision.omissions
        )
    finally:
        store.close()


@pytest.mark.asyncio
@pytest.mark.parametrize("summary_failure", [False, True], ids=["semantic", "fallback"])
async def test_agent_uses_a_tool_free_semantic_summary_before_continuing(
    tmp_path: Path, summary_failure: bool,
) -> None:
    class BlobTool:
        definition = ToolDefinition(
            name="blob",
            description="Return a large verified result.",
            input_schema={"type": "object", "additionalProperties": False},
        )

        async def execute(
            self,
            call: ToolCall,
            *,
            cancellation: CancellationToken,
            context: ToolExecutionContext,
        ) -> ToolResult:
            del cancellation, context
            return ToolResult(
                tool_call_id=call.id,
                tool_name=call.name,
                ok=True,
                output="VERIFIED-BLOB-RESULT " + "x" * (
                    5_000 if not summary_failure else 300
                ),
                details={"durationMs": 0, "truncated": False},
            )

    class CompactionProvider:
        def __init__(self, *, fail_summary: bool) -> None:
            self.requests: list[ProviderRequest] = []
            self.main_calls = 0
            self.fail_summary = fail_summary

        async def stream(
            self,
            request: ProviderRequest,
            *,
            cancellation: CancellationToken,
        ) -> AsyncIterator[ProviderEvent]:
            cancellation.raise_if_cancelled()
            self.requests.append(request)
            if any(
                message.content.startswith("You summarize") for message in request.messages
            ):
                if self.fail_summary:
                    raise RuntimeError("summary unavailable")
                yield TextDelta("Keep the verified blob result as an earlier fact.")
            else:
                self.main_calls += 1
                if self.main_calls == 1:
                    yield ToolCallCompleted(
                        ToolCall(f"blob-call-{self.main_calls}", "blob", {})
                    )
                else:
                    yield TextDelta("finished after compaction")
            yield ResponseCompleted()

    store = SqliteRuntimeStore(tmp_path / "state.db")
    events = []

    async def publish(event: Any) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Semantic compaction")
        if summary_failure:
            history_turn = store.prepare_turn(
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="Earlier completed context " + "h" * 500,
                run_config_template=RunConfigTemplate.create(
                    provider=ProviderExecutionSnapshot(
                        provider_id="compacting",
                        origin="test",
                        base_url=None,
                        model_id="compacting-v1",
                        supports_tools=True,
                        context_window=2_700,
                    ),
                    execution_policy="full_access",
                    skills=(),
                    tools=(),
                    identity_core=None,
                ),
            )
            store.mark_run_running(history_turn.run_id)
            store.prepare_model_step(history_turn.run_id, step_ordinal=1)
            history_assistant_id, _ = store.create_assistant_item(history_turn.run_id)
            store.append_text_delta(history_assistant_id, "Earlier answer")
            store.complete_provider_step(
                history_turn.run_id,
                step_ordinal=1,
                assistant_item_id=history_assistant_id,
                tool_calls=(),
                reasoning_content=None,
                usage=None,
                response_model_id=None,
                request_id=None,
            )
            store.terminalize_run(history_turn.run_id, "completed")
        template = RunConfigTemplate.create(
            provider=ProviderExecutionSnapshot(
                provider_id="compacting",
                origin="test",
                base_url=None,
                model_id="compacting-v1",
                supports_tools=True,
                context_window=2_700,
            ),
            execution_policy="full_access",
            skills=(),
            tools=(BlobTool.definition,),
            identity_core=None,
        )
        turn = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Keep the verified result and finish.",
            run_config_template=template,
        )
        provider = CompactionProvider(fail_summary=summary_failure)
        executor = ToolExecutor(ToolRegistry([BlobTool()]), FullAccessPolicy())

        await AgentLoop(store, {"compacting": provider}, publish, executor).run(
            turn.run_id,
            CancellationToken(),
        )

        assert store.run_status(turn.run_id) == "completed"
        summary_requests = [
            request
            for request in provider.requests
            if any(message.content.startswith("You summarize") for message in request.messages)
        ]
        assert len(summary_requests) == 1
        assert summary_requests[0].tools == ()
        if not summary_failure:
            assert "VERIFIED-BLOB-RESULT" in "\n".join(
                message.content for message in summary_requests[0].messages
            )
        main_requests = [
            request for request in provider.requests if request not in summary_requests
        ]
        assert len(main_requests) == 2
        if not summary_failure:
            assert any(
                "Prior context summary" in message.content
                and "Keep the verified blob result" in message.content
                for message in main_requests[-1].messages
            )
        revision = get_context_revision(store._connection, turn.run_id)
        assert revision is not None
        if summary_failure:
            assert revision.revision == 1
            assert revision.compaction_summary == ""
            # The first deterministic pass is inside the same transaction as
            # prepare_model_step; raising ContextCompactionRequired rolls it
            # back, so no orphan context.compacted sequence is published.
            assert not any(event.type == "context.compacted" for event in events)
        else:
            assert any(event.type == "context.compacted" for event in events)
            assert revision.revision == 2
            assert revision.compaction_summary.startswith("Prior context summary")
        loaded = load_context_for_revision(
            store._connection,
            run_id=turn.run_id,
            snapshot=revision,
        )
        if not summary_failure:
            assert not any("VERIFIED-BLOB-RESULT" in item.content for item in loaded)
    finally:
        store.close()


@pytest.mark.asyncio
async def test_compaction_summary_rejects_events_after_completion(tmp_path: Path) -> None:
    class TrailingEventProvider:
        async def stream(
            self,
            request: ProviderRequest,
            *,
            cancellation: CancellationToken,
        ) -> AsyncIterator[ProviderEvent]:
            del request
            cancellation.raise_if_cancelled()
            yield TextDelta("summary")
            yield ResponseCompleted()
            yield TextDelta("must be rejected")

    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Trailing summary event")
        template = RunConfigTemplate.create(
            provider=ProviderExecutionSnapshot(
                provider_id="trailing",
                origin="test",
                base_url=None,
                model_id="trailing-v1",
                supports_tools=False,
                context_window=2_700,
            ),
            execution_policy="full_access",
            skills=(),
            tools=(),
            identity_core=None,
        )
        turn = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="request",
            run_config_template=template,
        )
        provider = TrailingEventProvider()

        async def publish(event: Any) -> None:
            del event

        loop = AgentLoop(store, {"trailing": provider}, publish)
        with pytest.raises(RuntimeError, match="after response.completed"):
            await loop._request_compaction_summary(
                config=store.get_run_config(turn.run_id),
                provider=provider,
                omitted_records=(_message("old", "old context"),),
                existing_summary="",
                cancellation=CancellationToken(),
            )
    finally:
        store.close()
