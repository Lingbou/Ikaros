"""Provider-neutral planning for one model input step."""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import Literal

from ..domain import ContextItem, SkillDescriptor
from ..skills import build_skill_prompt
from ..tools.core import ToolDefinition

type InstructionAuthority = Literal[
    "runtime_identity",
    "runtime_instruction",
    "user_instruction",
]
type InputAuthority = InstructionAuthority | Literal["contextual_data"]
type InputLifetime = Literal["release", "run"]

_OUTPUT_STYLE_CONTENT = (
    "Use a restrained, professional response style. Do not use emoji or decorative "
    "Unicode symbols unless the user explicitly asks for them. Never use them for "
    "decoration, headings, or list markers. Use Markdown hyphen bullets (`- item`) "
    "for ordinary unordered lists; the client will render them as simple round bullets."
)


@dataclass(frozen=True, slots=True)
class InstructionBlockV1:
    """One ordered Runtime instruction with explicit provenance and lifetime."""

    id: str
    version: int
    source: str
    authority: InstructionAuthority
    scope: str
    lifetime: InputLifetime
    content: str


@dataclass(frozen=True, slots=True)
class ContextDataBlockV1:
    """One ordered, non-instruction context block.

    Gate 1 deliberately creates no context-data blocks.  The separate type reserves a
    safe input boundary for later Memory retrieval without treating retrieved data as a
    Runtime instruction.
    """

    id: str
    version: int
    source: str
    authority: Literal["contextual_data"]
    scope: str
    lifetime: InputLifetime
    content: str


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

    Plan-owned sequences are copied to tuples.  Payload objects inside existing
    ``ContextItem`` and ``ToolDefinition`` records are intentionally not recursively
    frozen until Gate 2 introduces a durable Run snapshot contract.
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
    """Build a deterministic provider-neutral plan from one Run snapshot."""

    def build_plan(
        self,
        *,
        model_id: str,
        items: Sequence[ContextItem],
        tools: Sequence[ToolDefinition] = (),
        skills: Sequence[SkillDescriptor] = (),
    ) -> ModelInputPlanV1:
        instructions = [
            InstructionBlockV1(
                id="output-style",
                version=1,
                source="ikaros-runtime:output-style-v1",
                authority="runtime_instruction",
                scope="global",
                lifetime="release",
                content=_OUTPUT_STYLE_CONTENT,
            )
        ]
        skill_prompt = build_skill_prompt(skills)
        if skill_prompt is not None:
            instructions.append(
                InstructionBlockV1(
                    id="skill-catalog",
                    version=1,
                    source="run:skill-descriptors",
                    authority="runtime_instruction",
                    scope="run",
                    lifetime="run",
                    content=skill_prompt,
                )
            )
        return ModelInputPlanV1(
            model_id=model_id,
            instructions=tuple(instructions),
            context_data=(),
            messages=tuple(items),
            tools=tuple(tools),
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
