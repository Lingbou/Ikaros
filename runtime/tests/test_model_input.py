from __future__ import annotations

from copy import deepcopy
from dataclasses import FrozenInstanceError, replace
from pathlib import Path
from typing import Any, cast

import pytest

import ikaros_runtime.run_input as run_input_module
from ikaros_runtime.agent import ModelInputPlan, ModelInputPlanner
from ikaros_runtime.domain import ContextItem, SkillDescriptor
from ikaros_runtime.errors import ModelInputUnavailableError
from ikaros_runtime.identity import load_identity_core
from ikaros_runtime.run_input import (
    EMPTY_FROZEN_MEMORY_CONTEXT_V1,
    INPUT_BUDGET_MEASUREMENT_VERSION,
    ContextItemRecordV1,
    ContextRevision,
    FrozenMemoryContextV1,
    HistoryItemReferenceV1,
    InputBudgetRecord,
    MemoryReferenceV1,
    MemoryScope,
    OmissionRecordV1,
    StepInput,
    build_context_revision,
    build_step_input,
    canonical_json,
)
from ikaros_runtime.tools.core import ToolDefinition

from .helpers import bounded_budget, run_config


def _memory_reference(
    ordinal: int = 1,
    *,
    characters: int = 12,
    scope: str = "global",
) -> MemoryReferenceV1:
    memory_id = f"memory_{ordinal:032x}"
    return MemoryReferenceV1(
        memory_id=memory_id,
        revision=ordinal,
        scope=cast(MemoryScope, scope),
        characters=characters,
    )


def test_model_input_plan_has_versioned_ordered_identity_style_and_skill_blocks(
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

    identity_core = load_identity_core()
    plan = ModelInputPlanner().build_plan(
        config=run_config(
            "provider",
            "model-1",
            skills=(skill,),
            identity_core=identity_core,
        ),
        items=(ContextItem(kind="message", role="user", content="hello", data={}),),
        budget_snapshot=bounded_budget(),
    )

    assert [block.id for block in plan.instructions] == [
        "ikaros-identity",
        "output-style",
        "skill-catalog",
    ]
    assert (
        plan.instructions[0].version,
        plan.instructions[0].source,
        plan.instructions[0].authority,
        plan.instructions[0].scope,
        plan.instructions[0].lifetime,
    ) == (
        1,
        "ikaros-runtime:identity",
        "runtime_identity",
        "global",
        "release",
    )
    assert plan.instructions[0] == identity_core
    assert (
        plan.instructions[1].version,
        plan.instructions[1].source,
        plan.instructions[1].authority,
        plan.instructions[1].scope,
        plan.instructions[1].lifetime,
    ) == (
        1,
        "ikaros-runtime:output-style-v1",
        "runtime_instruction",
        "global",
        "release",
    )
    assert (
        plan.instructions[2].version,
        plan.instructions[2].source,
        plan.instructions[2].authority,
        plan.instructions[2].scope,
        plan.instructions[2].lifetime,
    ) == (1, "run:skill-descriptors", "runtime_instruction", "run", "run")
    assert "<name>research</name>" in plan.instructions[2].content
    assert "Research &lt;carefully&gt; &amp; cite sources." in plan.instructions[2].content
    assert str(skill_file.resolve()) in plan.instructions[2].content
    assert "BODY-MUST-STAY-LAZY" not in plan.instructions[2].content
    assert plan.context_data == ()
    assert [block.authority for block in plan.instructions].count("runtime_identity") == 1
    assert plan.generation_options.max_output_tokens == 4096
    assert plan.budget_snapshot.mode == "bounded"


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
        config=run_config("provider", "model-1", tools=tools, skills=skills),
        items=items,
        budget_snapshot=bounded_budget(),
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
    frame = run_config("provider", "model-1", tools=tools)

    first = planner.build_plan(config=frame, items=items, budget_snapshot=bounded_budget())
    second = planner.build_plan(config=frame, items=items, budget_snapshot=bounded_budget())

    assert isinstance(first, ModelInputPlan)
    assert first == second


def test_model_input_plan_constructor_defensively_copies_runtime_sequences() -> None:
    baseline = ModelInputPlanner().build_plan(
        config=run_config("provider", "model-1"),
        items=(),
        budget_snapshot=bounded_budget(),
    )
    instructions = list(baseline.instructions)
    context_data = list(baseline.context_data)
    messages: list[ContextItem] = []
    tools: list[ToolDefinition] = []

    plan = ModelInputPlan(
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


def test_bounded_context_revision_accounts_for_all_actual_input_parts() -> None:
    tool = ToolDefinition(
        name="process_run",
        description="Run a process.",
        input_schema={"type": "object", "properties": {"command": {"type": "string"}}},
    )
    frame = run_config("provider", "model-1", tools=(tool,))
    records = (
        ContextItemRecordV1(
            item_id="item_history",
            turn_id="turn_history",
            run_id="run_history",
            kind="message",
            role="assistant",
            content="past",
            data={},
        ),
        ContextItemRecordV1(
            item_id="item_current",
            turn_id=frame.turn_id,
            run_id=frame.run_id,
            kind="message",
            role="user",
            content="now",
            data={},
        ),
    )

    snapshot = build_context_revision(
        records,
        current_run_id=frame.run_id,
        config=frame,
        maximum_tokens=frame.maximum_input_tokens,
        reserved_current_run_tokens=frame.reserved_current_run_tokens,
        omissions=(),
    )
    budget = snapshot.budget
    expected_instructions, expected_tools = run_input_module.config_input_token_counts(frame)

    assert budget.mode == "bounded"
    assert budget.measurement_version == INPUT_BUDGET_MEASUREMENT_VERSION
    assert budget.maximum_tokens == frame.maximum_input_tokens
    assert budget.reserved_current_run_tokens == frame.reserved_current_run_tokens
    assert budget.instruction_tokens == expected_instructions
    assert budget.context_data_tokens == 0
    assert budget.tool_tokens == expected_tools
    assert budget.history_tokens == records[0].estimated_tokens
    assert budget.current_run_tokens == records[1].estimated_tokens
    assert budget.memory_tokens == 0
    assert budget.total_tokens == (
        expected_instructions
        + expected_tools
        + records[0].estimated_tokens
        + records[1].estimated_tokens
    )
    assert InputBudgetRecord.from_wire(budget.to_wire()) == budget


def test_context_item_character_measurement_matches_provider_facing_parts() -> None:
    message = ContextItemRecordV1(
        item_id="item_message",
        turn_id="turn_1",
        run_id="run_1",
        kind="message",
        role="user",
        content="hello",
        data={},
    )
    arguments = {"command": "Write-Output 你好"}
    tool_call = ContextItemRecordV1(
        item_id="item_call",
        turn_id="turn_1",
        run_id="run_1",
        kind="tool_call",
        role="assistant",
        content="",
        data={
            "stepId": "step_1",
            "callId": "call_1",
            "toolName": "process_run",
            "arguments": arguments,
            "reasoningContent": "reason",
            "outcome": "completed",
            "durationMs": 999,
        },
    )
    result = {
        "toolCallId": "call_1",
        "toolName": "process_run",
        "ok": True,
        "output": "done",
        "cancelled": False,
    }
    tool_result = ContextItemRecordV1(
        item_id="item_result",
        turn_id="turn_1",
        run_id="run_1",
        kind="tool_result",
        role="tool",
        content=canonical_json(result),
        data={
            "stepId": "step_1",
            "callId": "call_1",
            "toolCallItemId": "item_call",
            "toolName": "process_run",
            "result": result,
        },
    )

    assert message.estimated_tokens >= len(b"hello")
    assert tool_call.estimated_tokens >= len(canonical_json(arguments).encode("utf-8")) + len(
        "reason"
    )
    assert tool_result.estimated_tokens >= len(canonical_json(result).encode("utf-8"))


@pytest.mark.parametrize(
    "record",
    (
        ContextItemRecordV1(
            item_id="item_bad_message",
            turn_id="turn_1",
            run_id="run_1",
            kind="message",
            role="tool",
            content="bad",
            data={},
        ),
        ContextItemRecordV1(
            item_id="item_bad_call",
            turn_id="turn_1",
            run_id="run_1",
            kind="tool_call",
            role="assistant",
            content="",
            data={"arguments": "not-an-object"},
        ),
        ContextItemRecordV1(
            item_id="item_bad_result",
            turn_id="turn_1",
            run_id="run_1",
            kind="tool_result",
            role="tool",
            content="{}",
            data={"result": "not-an-object"},
        ),
    ),
)
def test_context_item_character_measurement_rejects_non_provider_shapes(
    record: ContextItemRecordV1,
) -> None:
    with pytest.raises(ValueError, match="context Item|context Item data"):
        _ = record.estimated_tokens


@pytest.mark.parametrize(
    ("maximum_tokens", "reserved_current_run_tokens"),
    ((None, 0), (100, 0), (1, 1), (1, 2)),
)
def test_bounded_input_budget_rejects_missing_or_invalid_limits(
    maximum_tokens: int | None,
    reserved_current_run_tokens: int,
) -> None:
    with pytest.raises(ValueError, match="bounded input budget"):
        InputBudgetRecord(
            mode="bounded",
            measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
            maximum_tokens=cast(Any, maximum_tokens),
            reserved_current_run_tokens=reserved_current_run_tokens,
            instruction_tokens=0,
            context_data_tokens=0,
            tool_tokens=0,
            history_tokens=0,
            current_run_tokens=0,
            memory_tokens=0,
            total_tokens=0,
        )


@pytest.mark.parametrize(
    ("kind", "role"),
    (
        ("unknown", "assistant"),
        ("message", None),
        ("message", "tool"),
        ("tool_call", "tool"),
        ("tool_result", "assistant"),
    ),
)
def test_history_item_reference_rejects_invalid_kind_role_pairs(
    kind: str,
    role: str | None,
) -> None:
    with pytest.raises(ValueError, match="kind and role"):
        HistoryItemReferenceV1(
            item_id="item_1",
            turn_id="turn_1",
            run_id="run_1",
            kind=kind,
            role=role,
            tokens=1,
        )


@pytest.mark.parametrize("field", ["itemId", "turnId", "runId"])
def test_history_item_reference_rejects_empty_ids_from_wire(field: str) -> None:
    value: dict[str, object] = {
        "itemId": "item_1",
        "turnId": "turn_1",
        "runId": "run_1",
        "kind": "message",
        "role": "user",
        "tokens": 1,
    }
    value[field] = ""

    with pytest.raises(ValueError):
        HistoryItemReferenceV1.from_wire(value)


@pytest.mark.parametrize(
    ("slot", "field", "replacement"),
    (
        ("outputStyle", "id", "wrong-output"),
        ("outputStyle", "source", "wrong-source"),
        ("outputStyle", "authority", "user_instruction"),
        ("outputStyle", "scope", "run"),
        ("outputStyle", "lifetime", "run"),
        ("outputStyle", "version", 2),
        ("outputStyle", "content", "replaced output style"),
        ("identityCore", "id", "wrong-identity"),
        ("identityCore", "source", "wrong-source"),
        ("identityCore", "authority", "runtime_instruction"),
        ("identityCore", "scope", "run"),
        ("identityCore", "lifetime", "run"),
        ("identityCore", "version", 2),
        ("skillCatalog", "id", "wrong-skill-catalog"),
        ("skillCatalog", "source", "wrong-source"),
        ("skillCatalog", "authority", "user_instruction"),
        ("skillCatalog", "scope", "global"),
        ("skillCatalog", "lifetime", "release"),
        ("skillCatalog", "version", 2),
        ("skillCatalog", "content", "replaced catalog"),
    ),
)
def test_run_config_rejects_instruction_slot_metadata_corruption(
    slot: str,
    field: str,
    replacement: object,
) -> None:
    skill = SkillDescriptor("demo", "Demo Skill", "C:/skills/demo/SKILL.md")
    wire = run_config("provider", "model-1", skills=(skill,)).to_wire()
    instructions = cast(dict[str, Any], wire["instructions"])
    if slot == "identityCore":
        instructions["identityCore"] = load_identity_core().to_wire()
    block = cast(dict[str, Any], instructions[slot])
    block[field] = replacement

    with pytest.raises(ValueError, match="instruction"):
        run_input_module.RunConfig.from_wire(wire)


def test_run_config_accepts_identity_but_keeps_memory_slot_empty() -> None:
    identity_core = load_identity_core()
    frame = run_config("provider", "model-1", identity_core=identity_core)
    restored = run_input_module.RunConfig.from_wire(frame.to_wire())
    assert restored.identity_core == identity_core
    assert restored.instructions[0] == identity_core

    assert "contextData" not in restored.to_wire()


def test_memory_reference_wire_is_exact_and_omits_content_fingerprints() -> None:
    reference = _memory_reference(characters=9)
    assert MemoryReferenceV1.from_wire(reference.to_wire()) == reference
    assert set(reference.to_wire()) == {
        "memoryId",
        "revision",
        "scope",
        "characters",
    }

    malformed = reference.to_wire()
    malformed["snapshotSha256"] = "a" * 64
    with pytest.raises(ValueError, match="structure"):
        MemoryReferenceV1.from_wire(malformed)


@pytest.mark.parametrize(
    ("field", "replacement"),
    (
        ("memoryId", "memory_1"),
        ("revision", 0),
        ("revision", True),
        ("scope", "thread"),
        ("characters", 0),
        ("characters", 2_049),
    ),
)
def test_memory_reference_rejects_noncanonical_values(
    field: str,
    replacement: object,
) -> None:
    wire = _memory_reference().to_wire()
    wire[field] = replacement

    with pytest.raises(ValueError):
        MemoryReferenceV1.from_wire(wire)


def test_frozen_memory_context_enforces_limits_and_disjoint_omissions() -> None:
    selected = _memory_reference(characters=11)
    overlapping = OmissionRecordV1(
        source_type="memory",
        source_id=selected.memory_id,
        revision=selected.revision,
        characters=selected.characters,
        reason="omitted_by_limit",
    )
    with pytest.raises(ValueError, match="overlap"):
        FrozenMemoryContextV1((selected,), (overlapping,), 11, 5)

    with pytest.raises(ValueError, match="item limit"):
        FrozenMemoryContextV1(
            tuple(_memory_reference(index, characters=1) for index in range(1, 10)),
            (),
            9,
            5,
        )

    with pytest.raises(ValueError, match="character count"):
        FrozenMemoryContextV1(
            tuple(_memory_reference(index, characters=2_048) for index in range(1, 4)),
            (),
            6_144,
            5,
        )

    with pytest.raises(ValueError, match="context-data"):
        FrozenMemoryContextV1((selected,), (), 11, 0)

    assert FrozenMemoryContextV1((), (), 0, 0) == EMPTY_FROZEN_MEMORY_CONTEXT_V1


def test_omission_wire_is_discriminated_and_history_precedes_memory() -> None:
    history = OmissionRecordV1("history", "turn_boundary", "omitted_by_budget")
    memory = OmissionRecordV1(
        "memory",
        f"memory_{3:032x}",
        "omitted_by_limit",
        revision=2,
        characters=101,
    )
    assert OmissionRecordV1.from_wire(history.to_wire()) == history
    assert OmissionRecordV1.from_wire(memory.to_wire()) == memory
    assert set(history.to_wire()) == {"sourceType", "sourceId", "reason"}
    assert set(memory.to_wire()) == {
        "sourceType",
        "sourceId",
        "revision",
        "characters",
        "reason",
    }

    with pytest.raises(ValueError, match="history omission"):
        FrozenMemoryContextV1((), (history,), 0, 0)


def _context_revision_fixture() -> tuple[
    run_input_module.RunConfig,
    tuple[ContextItemRecordV1, ...],
    ContextRevision,
]:
    frame = run_config("provider", "model-1")
    records = (
        ContextItemRecordV1(
            item_id="item_old",
            turn_id="turn_old",
            run_id="run_old",
            kind="message",
            role="assistant",
            content="old",
            data={},
        ),
        ContextItemRecordV1(
            item_id=frame.user_item_id,
            turn_id=frame.turn_id,
            run_id=frame.run_id,
            kind="message",
            role="user",
            content="current",
            data={},
        ),
    )
    snapshot = build_context_revision(
        records,
        current_run_id=frame.run_id,
        config=frame,
        maximum_tokens=frame.maximum_input_tokens,
        reserved_current_run_tokens=frame.reserved_current_run_tokens,
        omissions=(),
    )
    return frame, records, snapshot


def test_context_revision_rejects_non_bijective_or_misordered_history_groups() -> None:
    _frame, _records, snapshot = _context_revision_fixture()
    baseline = snapshot.to_wire()
    corruptions: list[dict[str, Any]] = []

    ghost = deepcopy(baseline)
    cast(list[dict[str, Any]], ghost["historyGroups"])[0]["itemIds"] = ["item_ghost"]
    corruptions.append(ghost)

    reordered = deepcopy(baseline)
    groups = cast(list[dict[str, Any]], reordered["historyGroups"])
    groups[0]["itemIds"], groups[1]["itemIds"] = groups[1]["itemIds"], groups[0]["itemIds"]
    corruptions.append(reordered)

    wrong_turn = deepcopy(baseline)
    cast(list[dict[str, Any]], wrong_turn["historyGroups"])[0]["turnId"] = "turn_wrong"
    corruptions.append(wrong_turn)

    omitted = deepcopy(baseline)
    cast(list[dict[str, Any]], omitted["historyGroups"]).pop()
    corruptions.append(omitted)

    duplicate_item = deepcopy(baseline)
    items = cast(list[dict[str, Any]], duplicate_item["historyItems"])
    items[1]["itemId"] = items[0]["itemId"]
    corruptions.append(duplicate_item)

    duplicate_turn_group = deepcopy(baseline)
    groups = cast(list[dict[str, Any]], duplicate_turn_group["historyGroups"])
    groups[1]["turnId"] = groups[0]["turnId"]
    corruptions.append(duplicate_turn_group)

    for corrupted in corruptions:
        with pytest.raises(ValueError, match="history"):
            ContextRevision.from_wire(corrupted)


def test_context_revision_rejects_empty_history() -> None:
    _frame, _records, snapshot = _context_revision_fixture()

    empty = snapshot.to_wire()
    empty["historyGroups"] = []
    empty["historyItems"] = []
    empty_budget = cast(dict[str, Any], empty["budget"])
    removed = cast(int, empty_budget["historyTokens"]) + cast(int, empty_budget["currentRunTokens"])
    empty_budget["historyTokens"] = 0
    empty_budget["currentRunTokens"] = 0
    empty_budget["totalTokens"] = cast(int, empty_budget["totalTokens"]) - removed
    with pytest.raises(ValueError, match="current User Item"):
        ContextRevision.from_wire(empty)


def test_context_revision_accepts_frozen_memory_and_body_free_omissions() -> None:
    frame, records, _snapshot = _context_revision_fixture()
    selected = _memory_reference(characters=17)
    omitted = OmissionRecordV1(
        source_type="memory",
        source_id=f"memory_{2:032x}",
        revision=2,
        characters=19,
        reason="omitted_by_budget",
    )
    frozen = FrozenMemoryContextV1(
        memory=(selected,),
        omissions=(omitted,),
        memory_characters=17,
        context_data_characters=41,
    )

    snapshot = build_context_revision(
        records,
        current_run_id=frame.run_id,
        config=frame,
        maximum_tokens=frame.maximum_input_tokens,
        reserved_current_run_tokens=frame.reserved_current_run_tokens,
        omissions=(),
        memory_context=frozen,
    )
    restored = ContextRevision.from_wire(snapshot.to_wire())

    assert restored.memory == (selected,)
    assert restored.omissions == (omitted,)
    assert restored.budget.memory_tokens == 17 * 4
    assert restored.budget.context_data_tokens == 41 * 4
    assert FrozenMemoryContextV1.from_revision(restored) == frozen


def test_context_revision_accepts_one_unselected_budget_boundary() -> None:
    _frame, _records, snapshot = _context_revision_fixture()
    omitted = snapshot.to_wire()
    omitted["omissions"] = [
        {
            "sourceType": "history",
            "sourceId": "turn_older_boundary",
            "reason": "omitted_by_budget",
        }
    ]

    parsed = ContextRevision.from_wire(omitted)

    assert parsed.omissions[0].source_id == "turn_older_boundary"


@pytest.mark.parametrize(
    "omissions",
    (
        [
            {
                "sourceType": "history",
                "sourceId": "turn_old",
                "reason": "omitted_by_budget",
            }
        ],
        [
            {
                "sourceType": "history",
                "sourceId": "turn_a",
                "reason": "omitted_by_budget",
            },
            {
                "sourceType": "history",
                "sourceId": "turn_b",
                "reason": "omitted_by_budget",
            },
        ],
        [
            {
                "sourceType": "history_suffix",
                "sourceId": "turn_boundary",
                "reason": "omitted_by_budget",
            }
        ],
    ),
)
def test_context_revision_rejects_invalid_budget_boundaries(
    omissions: list[dict[str, str]],
) -> None:
    _frame, _records, snapshot = _context_revision_fixture()
    wire = snapshot.to_wire()
    wire["omissions"] = omissions

    with pytest.raises(ValueError, match="omission"):
        ContextRevision.from_wire(wire)


@pytest.mark.parametrize("budget_field", ["historyTokens", "currentRunTokens"])
def test_context_revision_rejects_budget_counts_that_disagree_with_items(
    budget_field: str,
) -> None:
    _frame, _records, snapshot = _context_revision_fixture()
    wire = snapshot.to_wire()
    budget = cast(dict[str, Any], wire["budget"])
    budget[budget_field] = cast(int, budget[budget_field]) + 1
    budget["totalTokens"] = cast(int, budget["totalTokens"]) + 1

    with pytest.raises(ValueError, match="history counts"):
        ContextRevision.from_wire(wire)


@pytest.mark.parametrize(
    ("field", "replacement"),
    (("stepOrdinal", 0), ("contextRevision", 0)),
)
def test_step_input_rejects_invalid_identity_fields(
    field: str,
    replacement: int,
) -> None:
    frame, records, snapshot = _context_revision_fixture()
    manifest = build_step_input(
        1,
        records,
        snapshot,
        current_run_id=frame.run_id,
        config=frame,
    )
    wire = manifest.to_wire()
    wire[field] = replacement

    with pytest.raises(ValueError):
        StepInput.from_wire(wire)


def test_step_input_rejects_budget_counts_that_disagree_with_items() -> None:
    frame, records, snapshot = _context_revision_fixture()
    manifest = build_step_input(
        1,
        records,
        snapshot,
        current_run_id=frame.run_id,
        config=frame,
    )
    wire = manifest.to_wire()
    budget = cast(dict[str, Any], wire["budget"])
    budget["currentRunTokens"] = cast(int, budget["currentRunTokens"]) + 1
    budget["totalTokens"] = cast(int, budget["totalTokens"]) + 1

    with pytest.raises(ValueError, match="history counts"):
        StepInput.from_wire(wire)


def test_step_input_preserves_frozen_memory_and_omissions() -> None:
    frame, records, _snapshot = _context_revision_fixture()
    selected = _memory_reference(characters=23)
    omitted = OmissionRecordV1(
        source_type="memory",
        source_id=f"memory_{2:032x}",
        revision=1,
        characters=31,
        reason="omitted_by_budget",
    )
    snapshot = build_context_revision(
        records,
        current_run_id=frame.run_id,
        config=frame,
        maximum_tokens=frame.maximum_input_tokens,
        reserved_current_run_tokens=frame.reserved_current_run_tokens,
        omissions=(),
        memory_context=FrozenMemoryContextV1(
            memory=(selected,),
            omissions=(omitted,),
            memory_characters=23,
            context_data_characters=47,
        ),
    )
    manifest = build_step_input(
        1,
        records,
        snapshot,
        current_run_id=frame.run_id,
        config=frame,
    )

    restored = StepInput.from_wire(manifest.to_wire())

    assert restored.memory == snapshot.memory == (selected,)
    assert restored.omissions == snapshot.omissions == (omitted,)
    assert restored.budget.memory_tokens == 23 * 4
    assert restored.budget.context_data_tokens == 47 * 4


@pytest.mark.parametrize("window,output", [(32768, 4096), (131072, 8192)])
def test_run_config_freezes_model_capacity_and_run_budget(window: int, output: int) -> None:
    config = replace(
        run_config("provider", "model"),
        context_window=window,
        max_output_tokens=output,
        max_model_calls=73,
        max_duration_seconds=900,
    )
    restored = run_input_module.RunConfig.from_wire(config.to_wire())
    assert restored == config
    assert restored.maximum_input_tokens == window - output
    assert restored.reserved_current_run_tokens == (window - output) // 4
    assert restored.max_model_calls == 73
    assert restored.max_duration_seconds == 900
    plan = ModelInputPlanner().build_plan(
        config=restored, items=(), budget_snapshot=bounded_budget()
    )
    assert plan.generation_options.max_output_tokens == output


@pytest.mark.parametrize(
    "changes",
    [
        {"context_window": 0},
        {"max_output_tokens": 0},
        {"context_window": 4096, "max_output_tokens": 4096},
        {"max_model_calls": 0},
        {"max_model_calls": True},
        {"max_duration_seconds": 0},
    ],
)
def test_run_config_rejects_invalid_execution_limits(changes: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        replace(run_config("provider", "model"), **cast(Any, changes))


def test_new_context_revision_is_append_only_and_step_binds_its_revision() -> None:
    config, records, initial = _context_revision_fixture()
    revised = replace(initial, revision=2)
    step = build_step_input(3, records, revised, current_run_id=config.run_id, config=config)
    assert initial.revision == 1
    assert revised.revision == 2
    assert step.context_revision == 2
    assert ContextRevision.from_wire(revised.to_wire()) == revised
    assert StepInput.from_wire(step.to_wire()) == step


def test_current_input_formats_reject_obsolete_fields() -> None:
    config, records, revision = _context_revision_fixture()
    step = build_step_input(1, records, revision, current_run_id=config.run_id, config=config)
    for value, parser in (
        (config.to_wire(), run_input_module.RunConfig.from_wire),
        (revision.to_wire(), ContextRevision.from_wire),
        (step.to_wire(), StepInput.from_wire),
    ):
        value["schemaVersion"] = 1
        with pytest.raises(ValueError, match="structure"):
            parser(value)


@pytest.mark.parametrize("field", ["context_window", "max_output_tokens"])
def test_step_input_rejects_model_capacity_changed_after_revision(field: str) -> None:
    config, records, revision = _context_revision_fixture()
    changed = replace(config, **{field: getattr(config, field) + 1})
    with pytest.raises(ModelInputUnavailableError):
        build_step_input(1, records, revision, current_run_id=config.run_id, config=changed)


def test_memory_metadata_is_not_reconstructed_from_budget_estimates() -> None:
    config, records, _ = _context_revision_fixture()
    memory = FrozenMemoryContextV1((_memory_reference(characters=7),), (), 7, 13)
    revision = build_context_revision(
        records,
        current_run_id=config.run_id,
        config=config,
        maximum_tokens=config.maximum_input_tokens,
        reserved_current_run_tokens=config.reserved_current_run_tokens,
        omissions=(),
        memory_context=memory,
    )
    assert revision.memory_context_characters == 13
    assert revision.budget.memory_tokens == 28
    assert revision.budget.context_data_tokens == 52
    wire = revision.to_wire()
    wire["memoryContextCharacters"] = 14
    with pytest.raises(ValueError, match="context-data token estimate"):
        ContextRevision.from_wire(wire)
