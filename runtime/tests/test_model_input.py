from __future__ import annotations

from copy import deepcopy
from dataclasses import FrozenInstanceError
from pathlib import Path
from typing import Any, cast

import pytest

import ikaros_runtime.run_input as run_input_module
from ikaros_runtime.agent import ModelInputPlanner, ModelInputPlanV1
from ikaros_runtime.domain import ContextItem, SkillDescriptor
from ikaros_runtime.identity import load_identity_core
from ikaros_runtime.run_input import (
    CONTEXT_SELECTION_VERSION,
    INPUT_BUDGET_MEASUREMENT_VERSION,
    ContextItemRecordV1,
    ContextSnapshotV1,
    HistoryItemReferenceV1,
    InputBudgetRecordV1,
    RunManifestV1,
    StepManifestV1,
    build_context_snapshot,
    build_step_manifest,
    canonical_json,
    validate_run_manifest,
)
from ikaros_runtime.tools.core import ToolDefinition

from .helpers import bounded_budget, submission_frame


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
        frame=submission_frame(
            "provider",
            "model-1",
            skills=(skill,),
            identity_core=identity_core,
        ),
        items=(ContextItem(kind="message", role="user", content="hello", data={}),),
        budget_snapshot=bounded_budget(),
    )

    assert plan.version == 1
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
    assert plan.generation_options.mode == "provider_defaults"
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
        frame=submission_frame("provider", "model-1", tools=tools, skills=skills),
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
    frame = submission_frame("provider", "model-1", tools=tools)

    first = planner.build_plan(
        frame=frame, items=items, budget_snapshot=bounded_budget()
    )
    second = planner.build_plan(
        frame=frame, items=items, budget_snapshot=bounded_budget()
    )

    assert isinstance(first, ModelInputPlanV1)
    assert first == second


def test_model_input_plan_constructor_defensively_copies_runtime_sequences() -> None:
    baseline = ModelInputPlanner().build_plan(
        frame=submission_frame("provider", "model-1"),
        items=(),
        budget_snapshot=bounded_budget(),
    )
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


def test_bounded_context_snapshot_accounts_for_all_actual_input_parts() -> None:
    tool = ToolDefinition(
        name="process_run",
        description="Run a process.",
        input_schema={"type": "object", "properties": {"command": {"type": "string"}}},
    )
    frame = submission_frame("provider", "model-1", tools=(tool,))
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

    snapshot = build_context_snapshot(
        records,
        current_run_id=frame.run_id,
        frame=frame,
        selection_version=CONTEXT_SELECTION_VERSION,
        maximum_characters=48_000,
        reserved_current_run_characters=12_000,
        omissions=(),
    )
    budget = snapshot.budget
    expected_instructions = sum(len(block.content) for block in frame.instructions)
    expected_tools = len(
        canonical_json(
            {
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            }
        )
    )

    assert budget.mode == "bounded"
    assert budget.measurement_version == INPUT_BUDGET_MEASUREMENT_VERSION
    assert budget.maximum_characters == 48_000
    assert budget.reserved_current_run_characters == 12_000
    assert budget.instruction_characters == expected_instructions
    assert budget.context_data_characters == 0
    assert budget.tool_characters == expected_tools
    assert budget.history_characters == records[0].characters
    assert budget.current_run_characters == records[1].characters
    assert budget.memory_characters == 0
    assert budget.total_characters == (
        expected_instructions
        + expected_tools
        + records[0].characters
        + records[1].characters
    )
    assert InputBudgetRecordV1.from_wire(budget.to_wire()) == budget


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

    assert message.characters == len("hello")
    assert tool_call.characters == len(canonical_json(arguments)) + len("reason")
    assert tool_result.characters == len(canonical_json(result))


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
        _ = record.characters


@pytest.mark.parametrize(
    ("maximum_characters", "reserved_current_run_characters"),
    ((None, 0), (100, 0), (1, 1), (1, 2)),
)
def test_bounded_input_budget_rejects_missing_or_invalid_limits(
    maximum_characters: int | None,
    reserved_current_run_characters: int,
) -> None:
    with pytest.raises(ValueError, match="bounded input budget"):
        InputBudgetRecordV1(
            mode="bounded",
            measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
            maximum_characters=maximum_characters,
            reserved_current_run_characters=reserved_current_run_characters,
            instruction_characters=0,
            context_data_characters=0,
            tool_characters=0,
            history_characters=0,
            current_run_characters=0,
            memory_characters=0,
            total_characters=0,
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
            characters=1,
        )


@pytest.mark.parametrize("field", ["itemId", "turnId", "runId"])
def test_history_item_reference_rejects_empty_ids_from_wire(field: str) -> None:
    value: dict[str, object] = {
        "itemId": "item_1",
        "turnId": "turn_1",
        "runId": "run_1",
        "kind": "message",
        "role": "user",
        "characters": 1,
    }
    value[field] = ""

    with pytest.raises(ValueError):
        HistoryItemReferenceV1.from_wire(value)


def test_run_manifest_validation_uses_persisted_registered_selector(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    frame = submission_frame("provider", "model-1")
    persisted = RunManifestV1.from_frame(
        frame,
        context_selection_version=CONTEXT_SELECTION_VERSION,
    ).to_wire()
    monkeypatch.setattr(run_input_module, "CONTEXT_SELECTION_VERSION", "future-selector-v2")

    manifest = validate_run_manifest(persisted, frame)

    assert manifest.context_selection_version == "bounded-history-v1"


def test_run_manifest_validation_rejects_unregistered_persisted_selector() -> None:
    frame = submission_frame("provider", "model-1")
    persisted = RunManifestV1.from_frame(
        frame,
        context_selection_version=CONTEXT_SELECTION_VERSION,
    ).to_wire()
    persisted["contextSelectionVersion"] = "unknown-selector"

    with pytest.raises(ValueError, match="unsupported"):
        validate_run_manifest(persisted, frame)


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
def test_submission_frame_rejects_instruction_slot_metadata_corruption(
    slot: str,
    field: str,
    replacement: object,
) -> None:
    skill = SkillDescriptor("demo", "Demo Skill", "C:/skills/demo/SKILL.md")
    wire = submission_frame("provider", "model-1", skills=(skill,)).to_wire()
    instructions = cast(dict[str, Any], wire["instructions"])
    if slot == "identityCore":
        instructions["identityCore"] = load_identity_core().to_wire()
    block = cast(dict[str, Any], instructions[slot])
    block[field] = replacement

    with pytest.raises(ValueError, match="instruction"):
        run_input_module.SubmissionFrameV1.from_wire(wire)


def test_submission_frame_accepts_identity_but_rejects_future_memory_slot() -> None:
    identity_core = load_identity_core()
    frame = submission_frame("provider", "model-1", identity_core=identity_core)
    restored = run_input_module.SubmissionFrameV1.from_wire(frame.to_wire())
    assert restored.identity_core == identity_core
    assert restored.instructions[0] == identity_core

    wire = submission_frame("provider", "model-1").to_wire()
    context_data = cast(dict[str, Any], wire["contextData"])
    context_data["memory"] = [
        {
            "memoryId": "memory_1",
            "revision": 1,
            "contentSha256": "a" * 64,
            "scope": "global",
        }
    ]
    with pytest.raises(ValueError, match="Memory context is not available"):
        run_input_module.SubmissionFrameV1.from_wire(wire)


def _context_snapshot_fixture() -> tuple[
    run_input_module.SubmissionFrameV1,
    tuple[ContextItemRecordV1, ...],
    ContextSnapshotV1,
]:
    frame = submission_frame("provider", "model-1")
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
    snapshot = build_context_snapshot(
        records,
        current_run_id=frame.run_id,
        frame=frame,
        selection_version=CONTEXT_SELECTION_VERSION,
        maximum_characters=48_000,
        reserved_current_run_characters=12_000,
        omissions=(),
    )
    return frame, records, snapshot


def test_context_snapshot_rejects_non_bijective_or_misordered_history_groups() -> None:
    _frame, _records, snapshot = _context_snapshot_fixture()
    baseline = snapshot.to_wire()
    corruptions: list[dict[str, Any]] = []

    ghost = deepcopy(baseline)
    cast(list[dict[str, Any]], ghost["historyGroups"])[0]["itemIds"] = ["item_ghost"]
    corruptions.append(ghost)

    reordered = deepcopy(baseline)
    groups = cast(list[dict[str, Any]], reordered["historyGroups"])
    groups[0]["itemIds"], groups[1]["itemIds"] = groups[1]["itemIds"], groups[0][
        "itemIds"
    ]
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
            ContextSnapshotV1.from_wire(corrupted)


def test_context_snapshot_rejects_empty_history_and_unavailable_memory() -> None:
    _frame, _records, snapshot = _context_snapshot_fixture()

    empty = snapshot.to_wire()
    empty["historyGroups"] = []
    empty["historyItems"] = []
    empty_budget = cast(dict[str, Any], empty["budget"])
    removed = cast(int, empty_budget["historyCharacters"]) + cast(
        int, empty_budget["currentRunCharacters"]
    )
    empty_budget["historyCharacters"] = 0
    empty_budget["currentRunCharacters"] = 0
    empty_budget["totalCharacters"] = cast(int, empty_budget["totalCharacters"]) - removed
    with pytest.raises(ValueError, match="current User Item"):
        ContextSnapshotV1.from_wire(empty)

    memory = snapshot.to_wire()
    memory["memory"] = [
        {
            "memoryId": "memory_1",
            "revision": 1,
            "contentSha256": "a" * 64,
            "scope": "global",
        }
    ]
    memory_budget = cast(dict[str, Any], memory["budget"])
    memory_budget["memoryCharacters"] = 1
    memory_budget["totalCharacters"] = cast(int, memory_budget["totalCharacters"]) + 1
    with pytest.raises(ValueError, match="Memory context is not available"):
        ContextSnapshotV1.from_wire(memory)



def test_context_snapshot_accepts_one_unselected_budget_boundary() -> None:
    _frame, _records, snapshot = _context_snapshot_fixture()
    omitted = snapshot.to_wire()
    omitted["omissions"] = [
        {
            "sourceType": "history",
            "sourceId": "turn_older_boundary",
            "reason": "omitted_by_budget",
        }
    ]

    parsed = ContextSnapshotV1.from_wire(omitted)

    assert parsed.omissions[0].source_id == "turn_older_boundary"


def test_context_snapshot_rejects_noncanonical_bounded_history_limits() -> None:
    _frame, _records, snapshot = _context_snapshot_fixture()
    wire = snapshot.to_wire()
    budget = cast(dict[str, Any], wire["budget"])
    budget["maximumCharacters"] = 47_000
    budget["reservedCurrentRunCharacters"] = 11_000

    with pytest.raises(ValueError, match="bounded-history-v1 limits"):
        ContextSnapshotV1.from_wire(wire)


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
def test_context_snapshot_rejects_invalid_budget_boundaries(
    omissions: list[dict[str, str]],
) -> None:
    _frame, _records, snapshot = _context_snapshot_fixture()
    wire = snapshot.to_wire()
    wire["omissions"] = omissions

    with pytest.raises(ValueError, match="omission"):
        ContextSnapshotV1.from_wire(wire)


@pytest.mark.parametrize("budget_field", ["historyCharacters", "currentRunCharacters"])
def test_context_snapshot_rejects_budget_counts_that_disagree_with_items(
    budget_field: str,
) -> None:
    _frame, _records, snapshot = _context_snapshot_fixture()
    wire = snapshot.to_wire()
    budget = cast(dict[str, Any], wire["budget"])
    budget[budget_field] = cast(int, budget[budget_field]) + 1
    budget["totalCharacters"] = cast(int, budget["totalCharacters"]) + 1

    with pytest.raises(ValueError, match="history counts"):
        ContextSnapshotV1.from_wire(wire)


@pytest.mark.parametrize(
    ("field", "replacement"),
    (("stepOrdinal", 0), ("contextSnapshotVersion", 2)),
)
def test_step_manifest_rejects_invalid_identity_fields(
    field: str,
    replacement: int,
) -> None:
    frame, records, snapshot = _context_snapshot_fixture()
    manifest = build_step_manifest(
        1,
        records,
        snapshot,
        current_run_id=frame.run_id,
        frame=frame,
    )
    wire = manifest.to_wire()
    wire[field] = replacement

    with pytest.raises(ValueError):
        StepManifestV1.from_wire(wire)


def test_step_manifest_rejects_budget_counts_that_disagree_with_items() -> None:
    frame, records, snapshot = _context_snapshot_fixture()
    manifest = build_step_manifest(
        1,
        records,
        snapshot,
        current_run_id=frame.run_id,
        frame=frame,
    )
    wire = manifest.to_wire()
    budget = cast(dict[str, Any], wire["budget"])
    budget["currentRunCharacters"] = cast(int, budget["currentRunCharacters"]) + 1
    budget["totalCharacters"] = cast(int, budget["totalCharacters"]) + 1

    with pytest.raises(ValueError, match="history counts"):
        StepManifestV1.from_wire(wire)


def test_step_manifest_rejects_noncanonical_bounded_history_limits() -> None:
    frame, records, snapshot = _context_snapshot_fixture()
    manifest = build_step_manifest(
        1,
        records,
        snapshot,
        current_run_id=frame.run_id,
        frame=frame,
    )
    wire = manifest.to_wire()
    budget = cast(dict[str, Any], wire["budget"])
    budget["maximumCharacters"] = 47_000
    budget["reservedCurrentRunCharacters"] = 11_000

    with pytest.raises(ValueError, match="bounded-history-v1 limits"):
        StepManifestV1.from_wire(wire)


def test_step_manifest_rejects_unavailable_memory() -> None:
    frame, records, snapshot = _context_snapshot_fixture()
    manifest = build_step_manifest(
        1,
        records,
        snapshot,
        current_run_id=frame.run_id,
        frame=frame,
    )

    memory = manifest.to_wire()
    memory["memory"] = [
        {
            "memoryId": "memory_1",
            "revision": 1,
            "contentSha256": "a" * 64,
            "scope": "global",
        }
    ]
    memory_budget = cast(dict[str, Any], memory["budget"])
    memory_budget["memoryCharacters"] = 1
    memory_budget["totalCharacters"] = cast(int, memory_budget["totalCharacters"]) + 1
    with pytest.raises(ValueError, match="Memory context is not available"):
        StepManifestV1.from_wire(memory)
