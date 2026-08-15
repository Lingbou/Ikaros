from __future__ import annotations

from dataclasses import FrozenInstanceError
from pathlib import Path
from typing import Any, cast

import pytest

from ikaros_runtime.agent import ModelInputPlanner, ModelInputPlanV1
from ikaros_runtime.domain import ContextItem, SkillDescriptor
from ikaros_runtime.tools.core import ToolDefinition


def test_model_input_plan_has_versioned_ordered_blocks_and_empty_future_slots(
    tmp_path: Path,
) -> None:
    skill_file = tmp_path / "skills" / "research" / "SKILL.md"
    skill_file.parent.mkdir(parents=True)
    skill_file.write_text("BODY-MUST-STAY-LAZY", encoding="utf-8")
    skill = SkillDescriptor(
        name="research",
        description="Research <carefully> & cite sources.",
        location=str(skill_file.resolve()),
    )

    plan = ModelInputPlanner().build_plan(
        model_id="model-1",
        items=(ContextItem(kind="message", role="user", content="hello", data={}),),
        skills=(skill,),
    )

    assert plan.version == 1
    assert [block.id for block in plan.instructions] == ["output-style", "skill-catalog"]
    assert (
        plan.instructions[0].version,
        plan.instructions[0].source,
        plan.instructions[0].authority,
        plan.instructions[0].scope,
        plan.instructions[0].lifetime,
    ) == (
        1,
        "ikaros-runtime:output-style-v1",
        "runtime_instruction",
        "global",
        "release",
    )
    assert (
        plan.instructions[1].version,
        plan.instructions[1].source,
        plan.instructions[1].authority,
        plan.instructions[1].scope,
        plan.instructions[1].lifetime,
    ) == (1, "run:skill-descriptors", "runtime_instruction", "run", "run")
    assert "<name>research</name>" in plan.instructions[1].content
    assert "Research &lt;carefully&gt; &amp; cite sources." in plan.instructions[1].content
    assert str(skill_file.resolve()) in plan.instructions[1].content
    assert "BODY-MUST-STAY-LAZY" not in plan.instructions[1].content
    assert plan.context_data == ()
    assert all(block.authority != "runtime_identity" for block in plan.instructions)
    assert plan.generation_options.mode == "provider_defaults"
    assert plan.budget_snapshot.mode == "legacy_unbounded"


def test_model_input_plan_owns_immutable_sequence_structure() -> None:
    item = ContextItem(kind="message", role="user", content="hello", data={})
    tool = ToolDefinition(
        name="process_run",
        description="Run a process.",
        input_schema={"type": "object"},
    )
    items = [item]
    tools = [tool]
    skills: list[SkillDescriptor] = []

    plan = ModelInputPlanner().build_plan(
        model_id="model-1",
        items=items,
        tools=tools,
        skills=skills,
    )
    items.append(ContextItem(kind="message", role="user", content="late", data={}))
    tools.clear()
    skills.append(SkillDescriptor("late", "Late Skill", "/skills/late/SKILL.md"))

    assert isinstance(plan.instructions, tuple)
    assert isinstance(plan.context_data, tuple)
    assert isinstance(plan.messages, tuple)
    assert isinstance(plan.tools, tuple)
    assert plan.messages == (item,)
    assert plan.tools == (tool,)
    assert [block.id for block in plan.instructions] == ["output-style"]
    with pytest.raises(FrozenInstanceError):
        plan.model_id = "changed"  # type: ignore[misc]


def test_model_input_plan_is_deterministic_for_equal_inputs() -> None:
    items = (ContextItem(kind="message", role="user", content="hello", data={}),)
    tools = (
        ToolDefinition(
            name="process_run",
            description="Run a process.",
            input_schema={"type": "object"},
        ),
    )
    planner = ModelInputPlanner()

    first = planner.build_plan(model_id="model-1", items=items, tools=tools)
    second = planner.build_plan(model_id="model-1", items=items, tools=tools)

    assert isinstance(first, ModelInputPlanV1)
    assert first == second


def test_model_input_plan_constructor_defensively_copies_runtime_sequences() -> None:
    baseline = ModelInputPlanner().build_plan(model_id="model-1", items=())
    instructions = list(baseline.instructions)
    context_data = list(baseline.context_data)
    messages: list[ContextItem] = []
    tools: list[ToolDefinition] = []

    plan = ModelInputPlanV1(
        model_id="model-1",
        instructions=cast(Any, instructions),
        context_data=cast(Any, context_data),
        messages=cast(Any, messages),
        tools=cast(Any, tools),
        generation_options=baseline.generation_options,
        budget_snapshot=baseline.budget_snapshot,
    )
    instructions.clear()
    messages.append(ContextItem(kind="message", role="user", content="late", data={}))
    tools.append(
        ToolDefinition(name="late", description="Late Tool", input_schema={"type": "object"})
    )

    assert plan.instructions == baseline.instructions
    assert plan.context_data == ()
    assert plan.messages == ()
    assert plan.tools == ()
