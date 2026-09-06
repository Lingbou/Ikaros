"""Provider-neutral planning for one model input step."""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

from ..domain import ContextItem
from ..run_input import (
    ContextDataBlockV1,
    InputAuthority,
    InputBudgetRecord,
    InputLifetime,
    InstructionAuthority,
    InstructionBlockV1,
    RunConfig,
)
from ..tools.core import ToolDefinition


@dataclass(frozen=True, slots=True)
class GenerationOptions:
    max_output_tokens: int = 4096


@dataclass(frozen=True, slots=True, kw_only=True)
class ModelInputPlan:
    """Immutable ordered structure consumed by :class:`ContextBuilder`.

    Plan-owned sequences are copied to tuples; durable Run configuration and
    context revisions supply the exact input for this short-lived structure.
    """

    model_id: str
    instructions: tuple[InstructionBlockV1, ...]
    context_data: tuple[ContextDataBlockV1, ...]
    messages: tuple[ContextItem, ...]
    tools: tuple[ToolDefinition, ...]
    generation_options: GenerationOptions
    budget_snapshot: InputBudgetRecord

    def __post_init__(self) -> None:
        """Defensively enforce the public Plan's sequence invariants at runtime."""

        object.__setattr__(self, "instructions", tuple(self.instructions))
        object.__setattr__(self, "context_data", tuple(self.context_data))
        object.__setattr__(self, "messages", tuple(self.messages))
        object.__setattr__(self, "tools", tuple(self.tools))


class ModelInputPlanner:
    """Build one deterministic plan from a persisted Run configuration."""

    def build_plan(
        self,
        *,
        config: RunConfig,
        items: Sequence[ContextItem],
        context_data: Sequence[ContextDataBlockV1] = (),
        budget_snapshot: InputBudgetRecord,
    ) -> ModelInputPlan:
        return ModelInputPlan(
            model_id=config.model_id,
            instructions=config.instructions,
            context_data=tuple(context_data),
            messages=tuple(items),
            tools=config.tool_definitions,
            generation_options=GenerationOptions(max_output_tokens=config.max_output_tokens),
            budget_snapshot=budget_snapshot,
        )


__all__ = [
    "ContextDataBlockV1",
    "GenerationOptions",
    "InputAuthority",
    "InputLifetime",
    "InstructionAuthority",
    "InstructionBlockV1",
    "ModelInputPlan",
    "ModelInputPlanner",
]
