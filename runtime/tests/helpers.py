from __future__ import annotations

from collections.abc import Sequence

from ikaros_runtime.domain import PreparedTurn, SkillDescriptor, WorkspaceSummary
from ikaros_runtime.run_input import (
    INPUT_BUDGET_MEASUREMENT_VERSION,
    InputBudgetRecordV1,
    ProviderExecutionSnapshotV1,
    SubmissionFrameTemplateV1,
    SubmissionFrameV1,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.core import ToolDefinition


def frame_template(
    provider_id: str,
    model_id: str,
    *,
    tools: Sequence[ToolDefinition] = (),
    skills: Sequence[SkillDescriptor] = (),
    max_steps: int = 16,
) -> SubmissionFrameTemplateV1:
    return SubmissionFrameTemplateV1.create(
        provider=ProviderExecutionSnapshotV1(
            provider_id=provider_id,
            origin="test",
            base_url=None,
            model_id=model_id,
            supports_tools=True,
        ),
        execution_policy="full_access",
        skills=skills,
        tools=tools,
        max_steps=max_steps,
    )


def prepare_turn(
    store: SqliteRuntimeStore,
    *,
    thread_id: str,
    branch_id: str,
    content: str,
    provider_id: str,
    model_id: str,
    client_request_id: str | None = None,
    skills: Sequence[SkillDescriptor] = (),
    tools: Sequence[ToolDefinition] = (),
    max_steps: int = 16,
) -> PreparedTurn:
    return store.prepare_turn(
        thread_id=thread_id,
        branch_id=branch_id,
        content=content,
        frame_template=frame_template(
            provider_id,
            model_id,
            tools=tools,
            skills=skills,
            max_steps=max_steps,
        ),
        client_request_id=client_request_id,
    )


def submission_frame(
    provider_id: str,
    model_id: str,
    *,
    tools: Sequence[ToolDefinition] = (),
    skills: Sequence[SkillDescriptor] = (),
    workspace: WorkspaceSummary | None = None,
    max_steps: int = 16,
) -> SubmissionFrameV1:
    return SubmissionFrameV1.from_template(
        frame_template(
            provider_id,
            model_id,
            tools=tools,
            skills=skills,
            max_steps=max_steps,
        ),
        user_item_id="item_test",
        thread_id="thread_test",
        branch_id="branch_test",
        turn_id="turn_test",
        run_id="run_test",
        workspace=workspace,
    )


def bounded_budget() -> InputBudgetRecordV1:
    return InputBudgetRecordV1(
        mode="bounded",
        measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
        maximum_characters=48_000,
        reserved_current_run_characters=12_000,
        instruction_characters=0,
        context_data_characters=0,
        tool_characters=0,
        history_characters=0,
        current_run_characters=0,
        memory_characters=0,
        total_characters=0,
    )


__all__ = ["bounded_budget", "frame_template", "prepare_turn", "submission_frame"]
