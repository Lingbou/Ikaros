"""Provider-neutral planning for one model input step."""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import Literal

from ..domain import ContextItem
from ..run_input import (
    ContextDataBlockV1,
    InputAuthority,
    InputLifetime,
    InstructionAuthority,
    InstructionBlockV1,
    SubmissionFrameV1,
)
from ..tools.core import ToolDefinition


@dataclass(frozen=True, slots=True)
class GenerationOptionsV1:
    """Gate 1 delegates generation controls to the selected Provider defaults."""

    mode: Literal["provider_defaults"] = "provider_defaults"


@dataclass(frozen=True, slots=True)
class InputBudgetSnapshotV1:
    """Gate 1 records that the legacy, unbounded history behavior is retained."""

    mode: Literal["legacy_unbounded"] = "legacy_unbounded"


@dataclass(frozen=True, slots=True, kw_only=True)
class ModelInputPlanV1:
    """Immutable ordered structure consumed by :class:`ContextBuilder`.

    Plan-owned sequences are copied to tuples.  Gate 2 supplies messages and Tool
    definitions from the durable Run snapshots before this short-lived plan is built.
    """

    version: Literal[1] = field(default=1, init=False)
    model_id: str
    instructions: tuple[InstructionBlockV1, ...]
    context_data: tuple[ContextDataBlockV1, ...]
    messages: tuple[ContextItem, ...]
    tools: tuple[ToolDefinition, ...]
    generation_options: GenerationOptionsV1
    budget_snapshot: InputBudgetSnapshotV1

    def __post_init__(self) -> None:
        """Defensively enforce the public Plan's sequence invariants at runtime."""

        object.__setattr__(self, "instructions", tuple(self.instructions))
        object.__setattr__(self, "context_data", tuple(self.context_data))
        object.__setattr__(self, "messages", tuple(self.messages))
        object.__setattr__(self, "tools", tuple(self.tools))


class ModelInputPlanner:
    """Build one deterministic plan from a persisted Submission Frame."""

    def build_plan(
        self,
        *,
        frame: SubmissionFrameV1,
        items: Sequence[ContextItem],
    ) -> ModelInputPlanV1:
        return ModelInputPlanV1(
            model_id=frame.model_id,
            instructions=frame.instructions,
            context_data=(),
            messages=tuple(items),
            tools=frame.tool_definitions,
            generation_options=GenerationOptionsV1(),
            budget_snapshot=InputBudgetSnapshotV1(),
        )


__all__ = [
    "ContextDataBlockV1",
    "GenerationOptionsV1",
    "InputAuthority",
    "InputBudgetSnapshotV1",
    "InputLifetime",
    "InstructionAuthority",
    "InstructionBlockV1",
    "ModelInputPlanV1",
    "ModelInputPlanner",
]
