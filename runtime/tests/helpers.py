from __future__ import annotations

from collections.abc import Sequence

from ikaros_runtime.domain import PreparedTurn, SkillDescriptor, WorkspaceSummary
from ikaros_runtime.run_input import (
    INPUT_BUDGET_MEASUREMENT_VERSION,
    InputBudgetRecord,
    InstructionBlockV1,
    ProviderExecutionSnapshot,
    RunConfig,
    RunConfigTemplate,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.core import ToolDefinition


def run_config_template(
    provider_id: str,
    model_id: str,
    *,
    tools: Sequence[ToolDefinition] = (),
    skills: Sequence[SkillDescriptor] = (),
    identity_core: InstructionBlockV1 | None = None,
) -> RunConfigTemplate:
    return RunConfigTemplate.create(
        provider=ProviderExecutionSnapshot(
            provider_id=provider_id,
            origin="test",
            base_url=None,
            model_id=model_id,
            supports_tools=True,
        ),
        execution_policy="full_access",
        skills=skills,
        tools=tools,
        identity_core=identity_core,
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
    identity_core: InstructionBlockV1 | None = None,
    tools: Sequence[ToolDefinition] = (),
) -> PreparedTurn:
    return store.prepare_turn(
        thread_id=thread_id,
        branch_id=branch_id,
        content=content,
        run_config_template=run_config_template(
            provider_id,
            model_id,
            tools=tools,
            skills=skills,
            identity_core=identity_core,
        ),
        client_request_id=client_request_id,
    )


def run_config(
    provider_id: str,
    model_id: str,
    *,
    tools: Sequence[ToolDefinition] = (),
    skills: Sequence[SkillDescriptor] = (),
    identity_core: InstructionBlockV1 | None = None,
    workspace: WorkspaceSummary | None = None,
) -> RunConfig:
    return RunConfig.from_template(
        run_config_template(
            provider_id,
            model_id,
            tools=tools,
            skills=skills,
            identity_core=identity_core,
        ),
        user_item_id="item_test",
        thread_id="thread_test",
        branch_id="branch_test",
        turn_id="turn_test",
        run_id="run_test",
        workspace=workspace,
    )


def bounded_budget() -> InputBudgetRecord:
    return InputBudgetRecord(
        mode="bounded",
        measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
        maximum_tokens=28_672,
        reserved_current_run_tokens=7_168,
        instruction_tokens=0,
        context_data_tokens=0,
        tool_tokens=0,
        history_tokens=0,
        current_run_tokens=0,
        memory_tokens=0,
        total_tokens=0,
    )


__all__ = ["bounded_budget", "run_config_template", "prepare_turn", "run_config"]
