"""Durable, provider-neutral input snapshots for one Agent Run.

The records in this module deliberately contain no credentials, user-message copy,
Memory body, Tool arguments/results, or provider request body.  They are shared by
the Agent and storage layers, so this module must not import either package.
"""

from __future__ import annotations

import hashlib
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any, Literal, cast

from .domain import ContextItem, JournalEvent, JsonObject, SkillDescriptor, WorkspaceSummary
from .errors import ContextBudgetExceededError, ModelInputUnavailableError
from .json_codec import dumps as json_dumps
from .json_codec import loads as json_loads
from .tools.core import ToolDefinition

type InstructionAuthority = Literal[
    "runtime_identity",
    "runtime_instruction",
    "user_instruction",
]
type InputAuthority = InstructionAuthority | Literal["contextual_data"]
type InputLifetime = Literal["release", "run"]
type ModelStepOutcome = Literal["completed", "failed", "cancelled"]
type MemoryScope = Literal["global", "workspace"]
type OmissionSourceType = Literal["history", "memory"]
type OmissionReason = Literal["omitted_by_budget", "omitted_by_limit"]

SUBMISSION_FRAME_SCHEMA_VERSION = 1
RUN_MANIFEST_SCHEMA_VERSION = 1
CONTEXT_SNAPSHOT_SCHEMA_VERSION = 1
STEP_MANIFEST_SCHEMA_VERSION = 1
CONTEXT_SELECTION_VERSION = "bounded-history-v1"
INPUT_BUDGET_MEASUREMENT_VERSION = "unicode-codepoints-canonical-json-v1"
MEMORY_CONTEXT_VERSION = 2
MAXIMUM_INPUT_CHARACTERS_V1 = 48_000
RESERVED_CURRENT_RUN_CHARACTERS_V1 = 12_000
MEMORY_CONTENT_MAX_CHARACTERS_V1 = 2_048
MEMORY_RETRIEVAL_MAX_CANDIDATES_V1 = 2_000
MEMORY_SELECTION_MAX_ITEMS_V1 = 8
MEMORY_SELECTION_MAX_CHARACTERS_V1 = 6_000
IKAROS_IDENTITY_ID = "ikaros-identity"
IKAROS_IDENTITY_VERSION = 1
IKAROS_IDENTITY_SOURCE = "ikaros-runtime:identity"
IDENTITY_CORE_MAX_CHARACTERS_V1 = 2_048

REGISTERED_CONTEXT_SELECTION_VERSIONS = frozenset({CONTEXT_SELECTION_VERSION})
EXECUTABLE_CONTEXT_SELECTION_VERSIONS = frozenset({CONTEXT_SELECTION_VERSION})
REGISTERED_INPUT_BUDGET_MODES = frozenset({"bounded"})

OUTPUT_STYLE_CONTENT = (
    "Use a restrained, professional response style. Do not use emoji or decorative "
    "Unicode symbols unless the user explicitly asks for them. Never use them for "
    "decoration, headings, or list markers. Use Markdown hyphen bullets (`- item`) "
    "for ordinary unordered lists; the client will render them as simple round bullets."
)


def canonical_json(value: object) -> str:
    """Return the one canonical JSON representation used by durable hashes."""

    return json_dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    )


def canonical_sha256(value: object) -> str:
    return hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


def deep_frozen_json_object(value: Mapping[str, Any]) -> JsonObject:
    """Detach nested JSON containers from caller-owned mutable objects."""

    decoded = json_loads(canonical_json(value))
    if not isinstance(decoded, dict):  # pragma: no cover - guaranteed by the input type
        raise ValueError("JSON object snapshot is invalid")
    return cast(JsonObject, decoded)


@dataclass(frozen=True, slots=True)
class InstructionBlockV1:
    id: str
    version: int
    source: str
    authority: InstructionAuthority
    scope: str
    lifetime: InputLifetime
    content: str

    def __post_init__(self) -> None:
        _nonempty("instruction ID", self.id)
        _positive("instruction version", self.version)
        _nonempty("instruction source", self.source)
        if self.authority not in {
            "runtime_identity",
            "runtime_instruction",
            "user_instruction",
        }:
            raise ValueError("instruction authority is invalid")
        _nonempty("instruction scope", self.scope)
        if self.lifetime not in {"release", "run"}:
            raise ValueError("instruction lifetime is invalid")
        _text("instruction content", self.content)

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "version": self.version,
            "source": self.source,
            "authority": self.authority,
            "scope": self.scope,
            "lifetime": self.lifetime,
            "content": self.content,
        }

    @classmethod
    def from_wire(cls, value: object) -> InstructionBlockV1:
        row = _object(value, "instruction", _INSTRUCTION_KEYS)
        return cls(
            id=_as_str(row["id"]),
            version=_as_int(row["version"]),
            source=_as_str(row["source"]),
            authority=cast(InstructionAuthority, row["authority"]),
            scope=_as_str(row["scope"]),
            lifetime=cast(InputLifetime, row["lifetime"]),
            content=_as_str(row["content"], allow_empty=True),
        )


@dataclass(frozen=True, slots=True)
class ContextDataBlockV1:
    id: str
    version: int
    source: str
    authority: Literal["contextual_data"]
    scope: str
    lifetime: InputLifetime
    content: str

    def __post_init__(self) -> None:
        _nonempty("context-data ID", self.id)
        _positive("context-data version", self.version)
        _nonempty("context-data source", self.source)
        if self.authority != "contextual_data":
            raise ValueError("context-data authority is invalid")
        _nonempty("context-data scope", self.scope)
        if self.lifetime not in {"release", "run"}:
            raise ValueError("context-data lifetime is invalid")
        _text("context-data content", self.content)


@dataclass(frozen=True, slots=True)
class ProviderExecutionSnapshotV1:
    provider_id: str
    origin: str
    base_url: str | None
    model_id: str
    supports_tools: bool

    def __post_init__(self) -> None:
        _nonempty("provider ID", self.provider_id)
        _nonempty("provider origin", self.origin)
        if self.base_url is not None:
            _nonempty("provider base URL", self.base_url)
        _nonempty("model ID", self.model_id)
        if not isinstance(self.supports_tools, bool):
            raise ValueError("model Tool capability is invalid")

    def public_wire(self) -> JsonObject:
        return {
            "providerId": self.provider_id,
            "origin": self.origin,
            "baseUrl": self.base_url,
            "modelId": self.model_id,
            "supportsTools": self.supports_tools,
        }

    @property
    def fingerprint(self) -> str:
        return canonical_sha256(self.public_wire())


@dataclass(frozen=True, slots=True)
class ToolDefinitionSnapshotV1:
    name: str
    description: str
    input_schema: JsonObject
    definition_sha256: str

    def __post_init__(self) -> None:
        _nonempty("Tool name", self.name)
        _text("Tool description", self.description)
        detached = deep_frozen_json_object(self.input_schema)
        object.__setattr__(self, "input_schema", detached)
        expected = canonical_sha256(
            {
                "name": self.name,
                "description": self.description,
                "inputSchema": detached,
            }
        )
        _sha256("Tool definition hash", self.definition_sha256)
        if self.definition_sha256 != expected:
            raise ValueError("Tool definition hash does not match its definition")

    @classmethod
    def from_definition(cls, definition: ToolDefinition) -> ToolDefinitionSnapshotV1:
        body = {
            "name": definition.name,
            "description": definition.description,
            "inputSchema": deep_frozen_json_object(definition.input_schema),
        }
        return cls(
            name=definition.name,
            description=definition.description,
            input_schema=cast(JsonObject, body["inputSchema"]),
            definition_sha256=canonical_sha256(body),
        )

    def to_definition(self) -> ToolDefinition:
        return ToolDefinition(
            name=self.name,
            description=self.description,
            input_schema=deep_frozen_json_object(self.input_schema),
        )

    def to_wire(self) -> JsonObject:
        return {
            "name": self.name,
            "description": self.description,
            "inputSchema": deep_frozen_json_object(self.input_schema),
            "definitionSha256": self.definition_sha256,
        }

    @classmethod
    def from_wire(cls, value: object) -> ToolDefinitionSnapshotV1:
        row = _object(value, "Tool definition snapshot", _TOOL_SNAPSHOT_KEYS)
        schema = row["inputSchema"]
        if not isinstance(schema, dict):
            raise ValueError("Tool input schema is invalid")
        return cls(
            name=_as_str(row["name"]),
            description=_as_str(row["description"], allow_empty=True),
            input_schema=cast(JsonObject, schema),
            definition_sha256=_as_str(row["definitionSha256"]),
        )


@dataclass(frozen=True, slots=True)
class MemoryReferenceV1:
    memory_id: str
    revision: int
    scope: MemoryScope
    characters: int

    def __post_init__(self) -> None:
        _runtime_id("Memory ID", self.memory_id, "memory_")
        _positive("Memory revision", self.revision)
        if self.scope not in {"global", "workspace"}:
            raise ValueError("Memory scope is invalid")
        if (
            not isinstance(self.characters, int)
            or isinstance(self.characters, bool)
            or not 1 <= self.characters <= MEMORY_CONTENT_MAX_CHARACTERS_V1
        ):
            raise ValueError("Memory character count is invalid")

    def to_wire(self) -> JsonObject:
        return {
            "memoryId": self.memory_id,
            "revision": self.revision,
            "scope": self.scope,
            "characters": self.characters,
        }

    @classmethod
    def from_wire(cls, value: object) -> MemoryReferenceV1:
        row = _object(value, "Memory reference", _MEMORY_REFERENCE_KEYS)
        return cls(
            memory_id=_as_str(row["memoryId"]),
            revision=_as_int(row["revision"]),
            scope=cast(MemoryScope, row["scope"]),
            characters=_as_int(row["characters"]),
        )


@dataclass(frozen=True, slots=True, kw_only=True)
class SubmissionFrameTemplateV1:
    provider: ProviderExecutionSnapshotV1
    execution_policy: str
    skills: tuple[SkillDescriptor, ...]
    tools: tuple[ToolDefinitionSnapshotV1, ...]
    output_style: InstructionBlockV1
    identity_core: InstructionBlockV1 | None
    skill_catalog: InstructionBlockV1 | None
    memory_context: tuple[MemoryReferenceV1, ...]
    max_steps: int

    def __post_init__(self) -> None:
        object.__setattr__(self, "skills", _skill_snapshot(self.skills))
        object.__setattr__(self, "tools", tuple(self.tools))
        object.__setattr__(self, "memory_context", tuple(self.memory_context))
        _nonempty("execution policy", self.execution_policy)
        _positive("maximum Agent Steps", self.max_steps)
        _unique_names("Tool", (tool.name for tool in self.tools))
        _unique_names("Skill", (skill.name for skill in self.skills))
        _validate_instruction_slots(
            output_style=self.output_style,
            identity_core=self.identity_core,
            skill_catalog=self.skill_catalog,
        )
        if self.skills and self.skill_catalog is None:
            raise ValueError("Skill descriptors require a Skill catalog instruction")
        if not self.skills and self.skill_catalog is not None:
            raise ValueError("Skill catalog instruction has no Skill descriptors")
        if self.memory_context:
            raise ValueError("Memory context is not available in this Runtime version")
        _validate_skill_catalog_content(self.skills, self.skill_catalog)

    @classmethod
    def create(
        cls,
        *,
        provider: ProviderExecutionSnapshotV1,
        execution_policy: str,
        skills: Sequence[SkillDescriptor],
        tools: Sequence[ToolDefinition],
        identity_core: InstructionBlockV1 | None,
        max_steps: int,
    ) -> SubmissionFrameTemplateV1:
        from .skills import build_skill_prompt

        frozen_skills = _skill_snapshot(skills)
        skill_prompt = build_skill_prompt(frozen_skills)
        return cls(
            provider=provider,
            execution_policy=execution_policy,
            skills=frozen_skills,
            tools=tuple(ToolDefinitionSnapshotV1.from_definition(tool) for tool in tools),
            output_style=InstructionBlockV1(
                id="output-style",
                version=1,
                source="ikaros-runtime:output-style-v1",
                authority="runtime_instruction",
                scope="global",
                lifetime="release",
                content=OUTPUT_STYLE_CONTENT,
            ),
            identity_core=identity_core,
            skill_catalog=(
                InstructionBlockV1(
                    id="skill-catalog",
                    version=1,
                    source="run:skill-descriptors",
                    authority="runtime_instruction",
                    scope="run",
                    lifetime="run",
                    content=skill_prompt,
                )
                if skill_prompt is not None
                else None
            ),
            memory_context=(),
            max_steps=max_steps,
        )


@dataclass(frozen=True, slots=True, kw_only=True)
class SubmissionFrameV1:
    schema_version: Literal[1] = field(default=1, init=False)
    user_item_id: str
    thread_id: str
    branch_id: str
    turn_id: str
    run_id: str
    workspace: WorkspaceSummary | None
    provider_id: str
    model_id: str
    public_provider_config_fingerprint: str
    execution_policy: str
    skills: tuple[SkillDescriptor, ...]
    tools: tuple[ToolDefinitionSnapshotV1, ...]
    output_style: InstructionBlockV1
    identity_core: InstructionBlockV1 | None
    skill_catalog: InstructionBlockV1 | None
    memory_context: tuple[MemoryReferenceV1, ...]
    max_steps: int

    def __post_init__(self) -> None:
        for label, value in (
            ("User Item ID", self.user_item_id),
            ("Thread ID", self.thread_id),
            ("Branch ID", self.branch_id),
            ("Turn ID", self.turn_id),
            ("Run ID", self.run_id),
            ("Provider ID", self.provider_id),
            ("Model ID", self.model_id),
            ("execution policy", self.execution_policy),
        ):
            _nonempty(label, value)
        _sha256(
            "public Provider configuration fingerprint",
            self.public_provider_config_fingerprint,
        )
        object.__setattr__(self, "skills", _skill_snapshot(self.skills))
        object.__setattr__(self, "tools", tuple(self.tools))
        object.__setattr__(self, "memory_context", tuple(self.memory_context))
        _positive("maximum Agent Steps", self.max_steps)
        _unique_names("Tool", (tool.name for tool in self.tools))
        _unique_names("Skill", (skill.name for skill in self.skills))
        if self.skills != tuple(sorted(self.skills, key=lambda skill: skill.name)):
            raise ValueError("Skill snapshot must be sorted")
        if (self.skill_catalog is None) != (not self.skills):
            raise ValueError("Skill catalog does not match the Skill snapshot")
        _validate_instruction_slots(
            output_style=self.output_style,
            identity_core=self.identity_core,
            skill_catalog=self.skill_catalog,
        )
        if self.memory_context:
            raise ValueError("Memory context is not available in this Runtime version")
        _validate_skill_catalog_content(self.skills, self.skill_catalog)

    @classmethod
    def from_template(
        cls,
        template: SubmissionFrameTemplateV1,
        *,
        user_item_id: str,
        thread_id: str,
        branch_id: str,
        turn_id: str,
        run_id: str,
        workspace: WorkspaceSummary | None,
    ) -> SubmissionFrameV1:
        return cls(
            user_item_id=user_item_id,
            thread_id=thread_id,
            branch_id=branch_id,
            turn_id=turn_id,
            run_id=run_id,
            workspace=workspace,
            provider_id=template.provider.provider_id,
            model_id=template.provider.model_id,
            public_provider_config_fingerprint=template.provider.fingerprint,
            execution_policy=template.execution_policy,
            skills=template.skills,
            tools=template.tools,
            output_style=template.output_style,
            identity_core=template.identity_core,
            skill_catalog=template.skill_catalog,
            memory_context=template.memory_context,
            max_steps=template.max_steps,
        )

    @property
    def instructions(self) -> tuple[InstructionBlockV1, ...]:
        return tuple(
            block
            for block in (self.identity_core, self.output_style, self.skill_catalog)
            if block is not None
        )

    @property
    def tool_definitions(self) -> tuple[ToolDefinition, ...]:
        return tuple(tool.to_definition() for tool in self.tools)

    def to_wire(self) -> JsonObject:
        return {
            "schemaVersion": self.schema_version,
            "userItemId": self.user_item_id,
            "threadId": self.thread_id,
            "branchId": self.branch_id,
            "turnId": self.turn_id,
            "runId": self.run_id,
            "workspace": self.workspace.to_wire() if self.workspace is not None else None,
            "providerId": self.provider_id,
            "modelId": self.model_id,
            "publicProviderConfigFingerprint": self.public_provider_config_fingerprint,
            "executionPolicy": self.execution_policy,
            "skills": [skill.to_wire() for skill in self.skills],
            "tools": [tool.to_wire() for tool in self.tools],
            "instructions": {
                "outputStyle": self.output_style.to_wire(),
                "identityCore": (
                    self.identity_core.to_wire() if self.identity_core is not None else None
                ),
                "skillCatalog": (
                    self.skill_catalog.to_wire() if self.skill_catalog is not None else None
                ),
            },
            "contextData": {
                "memory": [reference.to_wire() for reference in self.memory_context],
            },
            "maxSteps": self.max_steps,
        }

    @classmethod
    def from_wire(cls, value: object) -> SubmissionFrameV1:
        row = _object(value, "Submission Frame", _SUBMISSION_FRAME_KEYS)
        if row["schemaVersion"] != SUBMISSION_FRAME_SCHEMA_VERSION:
            raise ValueError("Submission Frame schema version is unsupported")
        workspace_value = row["workspace"]
        workspace = _workspace_from_wire(workspace_value)
        skills_value = row["skills"]
        tools_value = row["tools"]
        instructions = _object(row["instructions"], "instruction slots", _INSTRUCTION_SLOT_KEYS)
        context_data = _object(row["contextData"], "context-data slots", {"memory"})
        if not isinstance(skills_value, list) or not isinstance(tools_value, list):
            raise ValueError("Submission Frame snapshots are invalid")
        memory_value = context_data["memory"]
        if not isinstance(memory_value, list):
            raise ValueError("Submission Frame Memory references are invalid")
        return cls(
            user_item_id=_as_str(row["userItemId"]),
            thread_id=_as_str(row["threadId"]),
            branch_id=_as_str(row["branchId"]),
            turn_id=_as_str(row["turnId"]),
            run_id=_as_str(row["runId"]),
            workspace=workspace,
            provider_id=_as_str(row["providerId"]),
            model_id=_as_str(row["modelId"]),
            public_provider_config_fingerprint=_as_str(
                row["publicProviderConfigFingerprint"]
            ),
            execution_policy=_as_str(row["executionPolicy"]),
            skills=tuple(SkillDescriptor.from_wire(skill) for skill in skills_value),
            tools=tuple(ToolDefinitionSnapshotV1.from_wire(tool) for tool in tools_value),
            output_style=InstructionBlockV1.from_wire(instructions["outputStyle"]),
            identity_core=(
                InstructionBlockV1.from_wire(instructions["identityCore"])
                if instructions["identityCore"] is not None
                else None
            ),
            skill_catalog=(
                InstructionBlockV1.from_wire(instructions["skillCatalog"])
                if instructions["skillCatalog"] is not None
                else None
            ),
            memory_context=tuple(
                MemoryReferenceV1.from_wire(reference) for reference in memory_value
            ),
            max_steps=_as_int(row["maxSteps"]),
        )


@dataclass(frozen=True, slots=True)
class ManifestInstructionV1:
    id: str
    version: int
    source: str
    authority: InstructionAuthority
    scope: str
    lifetime: InputLifetime
    characters: int
    content_sha256: str

    @classmethod
    def from_block(cls, block: InstructionBlockV1) -> ManifestInstructionV1:
        return cls(
            id=block.id,
            version=block.version,
            source=block.source,
            authority=block.authority,
            scope=block.scope,
            lifetime=block.lifetime,
            characters=len(block.content),
            content_sha256=canonical_sha256(block.content),
        )

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "version": self.version,
            "source": self.source,
            "authority": self.authority,
            "scope": self.scope,
            "lifetime": self.lifetime,
            "characters": self.characters,
            "contentSha256": self.content_sha256,
        }


@dataclass(frozen=True, slots=True, kw_only=True)
class RunManifestV1:
    schema_version: Literal[1] = field(default=1, init=False)
    run_id: str
    model_input_plan_version: int
    submission_frame_version: int
    context_selection_version: str
    memory_context_version: int
    instructions: tuple[ManifestInstructionV1, ...]
    skills: tuple[JsonObject, ...]
    tools: tuple[JsonObject, ...]
    provider_id: str
    model_id: str
    public_provider_config_fingerprint: str
    execution_policy: str
    max_steps: int

    @classmethod
    def from_frame(
        cls,
        frame: SubmissionFrameV1,
        *,
        context_selection_version: str,
    ) -> RunManifestV1:
        if context_selection_version not in REGISTERED_CONTEXT_SELECTION_VERSIONS:
            raise ValueError("Context selection version is unsupported")
        return cls(
            run_id=frame.run_id,
            model_input_plan_version=1,
            submission_frame_version=frame.schema_version,
            context_selection_version=context_selection_version,
            memory_context_version=MEMORY_CONTEXT_VERSION,
            instructions=tuple(
                ManifestInstructionV1.from_block(block) for block in frame.instructions
            ),
            skills=tuple(
                {
                    "name": skill.name,
                    "descriptorSha256": canonical_sha256(skill.to_wire()),
                }
                for skill in frame.skills
            ),
            tools=tuple(
                {"name": tool.name, "definitionSha256": tool.definition_sha256}
                for tool in frame.tools
            ),
            provider_id=frame.provider_id,
            model_id=frame.model_id,
            public_provider_config_fingerprint=frame.public_provider_config_fingerprint,
            execution_policy=frame.execution_policy,
            max_steps=frame.max_steps,
        )

    def to_wire(self) -> JsonObject:
        return {
            "schemaVersion": self.schema_version,
            "runId": self.run_id,
            "modelInputPlanVersion": self.model_input_plan_version,
            "submissionFrameVersion": self.submission_frame_version,
            "contextSelectionVersion": self.context_selection_version,
            "memoryContextVersion": self.memory_context_version,
            "instructions": [instruction.to_wire() for instruction in self.instructions],
            "skills": [deep_frozen_json_object(skill) for skill in self.skills],
            "tools": [deep_frozen_json_object(tool) for tool in self.tools],
            "provider": {
                "providerId": self.provider_id,
                "modelId": self.model_id,
                "publicProviderConfigFingerprint": self.public_provider_config_fingerprint,
            },
            "executionPolicy": self.execution_policy,
            "maxSteps": self.max_steps,
        }

@dataclass(frozen=True, slots=True)
class ContextItemRecordV1:
    item_id: str
    turn_id: str
    run_id: str
    kind: str
    role: str | None
    content: str
    data: JsonObject

    def __post_init__(self) -> None:
        object.__setattr__(self, "data", deep_frozen_json_object(self.data))

    @property
    def characters(self) -> int:
        if self.kind == "message":
            if self.role not in {"user", "assistant"}:
                raise ValueError("message context Item has an invalid role")
            return len(self.content)
        if self.kind == "tool_call":
            if self.role != "assistant":
                raise ValueError("Tool Call context Item has an invalid role")
            arguments = self.data.get("arguments")
            reasoning_content = self.data.get("reasoningContent")
            if not isinstance(arguments, dict) or (
                reasoning_content is not None and not isinstance(reasoning_content, str)
            ):
                raise ValueError("Tool Call context Item data is invalid")
            return len(canonical_json(arguments)) + (
                len(reasoning_content) if reasoning_content is not None else 0
            )
        if self.kind == "tool_result":
            if self.role != "tool":
                raise ValueError("Tool Result context Item has an invalid role")
            result = self.data.get("result")
            if not isinstance(result, dict):
                raise ValueError("Tool Result context Item data is invalid")
            return len(canonical_json(result))
        raise ValueError("context Item kind is invalid")

    def to_context_item(self) -> ContextItem:
        return ContextItem(
            kind=self.kind,
            role=self.role,
            content=self.content,
            data=deep_frozen_json_object(self.data),
        )


@dataclass(frozen=True, slots=True)
class HistoryItemReferenceV1:
    item_id: str
    turn_id: str
    run_id: str
    kind: str
    role: str | None
    characters: int

    def __post_init__(self) -> None:
        for label, value in (
            ("history Item ID", self.item_id),
            ("history Turn ID", self.turn_id),
            ("history Run ID", self.run_id),
        ):
            _nonempty(label, value)
        expected_roles = {
            "message": frozenset({"user", "assistant"}),
            "tool_call": frozenset({"assistant"}),
            "tool_result": frozenset({"tool"}),
        }
        if self.kind not in expected_roles or self.role not in expected_roles[self.kind]:
            raise ValueError("history Item kind and role are invalid")
        if (
            not isinstance(self.characters, int)
            or isinstance(self.characters, bool)
            or self.characters < 0
        ):
            raise ValueError("history Item character count is invalid")

    @classmethod
    def from_record(cls, record: ContextItemRecordV1) -> HistoryItemReferenceV1:
        return cls(
            item_id=record.item_id,
            turn_id=record.turn_id,
            run_id=record.run_id,
            kind=record.kind,
            role=record.role,
            characters=record.characters,
        )

    def to_wire(self) -> JsonObject:
        return {
            "itemId": self.item_id,
            "turnId": self.turn_id,
            "runId": self.run_id,
            "kind": self.kind,
            "role": self.role,
            "characters": self.characters,
        }

    @classmethod
    def from_wire(cls, value: object) -> HistoryItemReferenceV1:
        row = _object(value, "history Item reference", _HISTORY_ITEM_KEYS)
        role = row["role"]
        if not isinstance(role, str):
            raise ValueError("history Item role is invalid")
        return cls(
            item_id=_as_str(row["itemId"]),
            turn_id=_as_str(row["turnId"]),
            run_id=_as_str(row["runId"]),
            kind=_as_str(row["kind"]),
            role=role,
            characters=_as_int(row["characters"]),
        )


@dataclass(frozen=True, slots=True)
class HistoryGroupReferenceV1:
    turn_id: str
    item_ids: tuple[str, ...]

    def __post_init__(self) -> None:
        _nonempty("history group Turn ID", self.turn_id)
        object.__setattr__(self, "item_ids", tuple(self.item_ids))
        if not self.item_ids:
            raise ValueError("history group must contain at least one Item")
        for item_id in self.item_ids:
            _nonempty("history group Item ID", item_id)
        if len(self.item_ids) != len(set(self.item_ids)):
            raise ValueError("history group Item IDs must be unique")

    def to_wire(self) -> JsonObject:
        return {"turnId": self.turn_id, "itemIds": list(self.item_ids)}

    @classmethod
    def from_wire(cls, value: object) -> HistoryGroupReferenceV1:
        row = _object(value, "history group", {"turnId", "itemIds"})
        item_ids = row["itemIds"]
        if not isinstance(item_ids, list) or any(not isinstance(item, str) for item in item_ids):
            raise ValueError("history group Item IDs are invalid")
        return cls(turn_id=_as_str(row["turnId"]), item_ids=tuple(item_ids))


@dataclass(frozen=True, slots=True)
class InputBudgetRecordV1:
    mode: str
    measurement_version: str
    maximum_characters: int | None
    reserved_current_run_characters: int
    instruction_characters: int
    context_data_characters: int
    tool_characters: int
    history_characters: int
    current_run_characters: int
    memory_characters: int
    total_characters: int

    def __post_init__(self) -> None:
        if self.mode not in REGISTERED_INPUT_BUDGET_MODES:
            raise ValueError("input budget mode is unsupported")
        if self.measurement_version != INPUT_BUDGET_MEASUREMENT_VERSION:
            raise ValueError("input budget measurement version is unsupported")
        if self.maximum_characters is not None and (
            not isinstance(self.maximum_characters, int)
            or isinstance(self.maximum_characters, bool)
            or self.maximum_characters < 1
        ):
            raise ValueError("input budget maximum is invalid")
        if self.mode == "bounded" and (
            self.maximum_characters is None
            or self.reserved_current_run_characters == 0
            or self.reserved_current_run_characters >= self.maximum_characters
        ):
            raise ValueError("bounded input budget limits are invalid")
        counts = (
            self.reserved_current_run_characters,
            self.instruction_characters,
            self.context_data_characters,
            self.tool_characters,
            self.history_characters,
            self.current_run_characters,
            self.memory_characters,
            self.total_characters,
        )
        if any(
            not isinstance(count, int) or isinstance(count, bool) or count < 0
            for count in counts
        ):
            raise ValueError("input budget counts are invalid")
        actual_parts = counts[1:7]
        if self.total_characters != sum(actual_parts):
            raise ValueError("input budget total is invalid")
        if (
            self.maximum_characters is not None
            and self.total_characters > self.maximum_characters
        ):
            raise ValueError("input budget exceeds its maximum")

    def to_wire(self) -> JsonObject:
        return {
            "mode": self.mode,
            "measurementVersion": self.measurement_version,
            "maximumCharacters": self.maximum_characters,
            "reservedCurrentRunCharacters": self.reserved_current_run_characters,
            "instructionCharacters": self.instruction_characters,
            "contextDataCharacters": self.context_data_characters,
            "toolCharacters": self.tool_characters,
            "historyCharacters": self.history_characters,
            "currentRunCharacters": self.current_run_characters,
            "memoryCharacters": self.memory_characters,
            "totalCharacters": self.total_characters,
        }

    @classmethod
    def from_wire(cls, value: object) -> InputBudgetRecordV1:
        row = _object(value, "input budget", _INPUT_BUDGET_KEYS)
        result = cls(
            mode=_as_str(row["mode"]),
            measurement_version=_as_str(row["measurementVersion"]),
            maximum_characters=_optional_positive_int(
                row["maximumCharacters"],
                label="input budget maximum",
            ),
            reserved_current_run_characters=_as_int(
                row["reservedCurrentRunCharacters"]
            ),
            instruction_characters=_as_int(row["instructionCharacters"]),
            context_data_characters=_as_int(row["contextDataCharacters"]),
            tool_characters=_as_int(row["toolCharacters"]),
            history_characters=_as_int(row["historyCharacters"]),
            current_run_characters=_as_int(row["currentRunCharacters"]),
            memory_characters=_as_int(row["memoryCharacters"]),
            total_characters=_as_int(row["totalCharacters"]),
        )
        return result


@dataclass(frozen=True, slots=True)
class OmissionRecordV1:
    source_type: OmissionSourceType
    source_id: str
    reason: OmissionReason
    revision: int | None = None
    characters: int | None = None

    def __post_init__(self) -> None:
        if self.source_type == "history":
            if (
                self.reason != "omitted_by_budget"
                or self.revision is not None
                or self.characters is not None
            ):
                raise ValueError("history omission record is invalid")
            _nonempty("history omission boundary Turn ID", self.source_id)
            return
        if self.source_type != "memory" or self.reason not in {
            "omitted_by_budget",
            "omitted_by_limit",
        }:
            raise ValueError("Memory omission record is invalid")
        _runtime_id("Memory omission ID", self.source_id, "memory_")
        _positive("Memory omission revision", self.revision)
        if (
            not isinstance(self.characters, int)
            or isinstance(self.characters, bool)
            or not 1 <= self.characters <= MEMORY_CONTENT_MAX_CHARACTERS_V1
        ):
            raise ValueError("Memory omission character count is invalid")

    def to_wire(self) -> JsonObject:
        if self.source_type == "history":
            return {
                "sourceType": self.source_type,
                "sourceId": self.source_id,
                "reason": self.reason,
            }
        return {
            "sourceType": self.source_type,
            "sourceId": self.source_id,
            "revision": cast(int, self.revision),
            "characters": cast(int, self.characters),
            "reason": self.reason,
        }

    @classmethod
    def from_wire(cls, value: object) -> OmissionRecordV1:
        if not isinstance(value, dict):
            raise ValueError("omission record has an invalid structure")
        source_type = value.get("sourceType")
        if source_type == "history":
            row = _object(value, "history omission record", _HISTORY_OMISSION_KEYS)
            return cls(
                source_type="history",
                source_id=_as_str(row["sourceId"]),
                reason=cast(OmissionReason, row["reason"]),
            )
        if source_type != "memory":
            raise ValueError("omission record source type is invalid")
        row = _object(value, "Memory omission record", _MEMORY_OMISSION_KEYS)
        return cls(
            source_type="memory",
            source_id=_as_str(row["sourceId"]),
            revision=_as_int(row["revision"]),
            characters=_as_int(row["characters"]),
            reason=cast(OmissionReason, row["reason"]),
        )


@dataclass(frozen=True, slots=True)
class FrozenMemoryContextV1:
    """Body-free Memory selection frozen for every Provider Step in one Run."""

    memory: tuple[MemoryReferenceV1, ...]
    omissions: tuple[OmissionRecordV1, ...]
    memory_characters: int
    context_data_characters: int

    def __post_init__(self) -> None:
        object.__setattr__(self, "memory", tuple(self.memory))
        object.__setattr__(self, "omissions", tuple(self.omissions))
        if len(self.memory) > MEMORY_SELECTION_MAX_ITEMS_V1:
            raise ValueError("Memory selection exceeds the item limit")
        selected_ids = tuple(reference.memory_id for reference in self.memory)
        if len(selected_ids) != len(set(selected_ids)):
            raise ValueError("Memory selection IDs must be unique")
        if any(omission.source_type != "memory" for omission in self.omissions):
            raise ValueError("Frozen Memory context contains a history omission")
        omitted_ids = tuple(omission.source_id for omission in self.omissions)
        if len(omitted_ids) != len(set(omitted_ids)):
            raise ValueError("Memory omission IDs must be unique")
        if set(selected_ids).intersection(omitted_ids):
            raise ValueError("selected and omitted Memory IDs overlap")
        if len(self.memory) + len(self.omissions) > MEMORY_RETRIEVAL_MAX_CANDIDATES_V1:
            raise ValueError("Memory selection exceeds the candidate limit")
        if any(
            omission.reason == "omitted_by_limit" for omission in self.omissions
        ) and len(self.memory) != MEMORY_SELECTION_MAX_ITEMS_V1:
            raise ValueError("Memory limit omission requires a full selection")
        expected_characters = sum(reference.characters for reference in self.memory)
        if (
            not isinstance(self.memory_characters, int)
            or isinstance(self.memory_characters, bool)
            or self.memory_characters != expected_characters
            or self.memory_characters > MEMORY_SELECTION_MAX_CHARACTERS_V1
        ):
            raise ValueError("Memory selection character count is invalid")
        if (
            not isinstance(self.context_data_characters, int)
            or isinstance(self.context_data_characters, bool)
            or self.context_data_characters < 0
            or bool(self.memory) != (self.context_data_characters > 0)
        ):
            raise ValueError("Memory context-data character count is invalid")

    @classmethod
    def from_snapshot(cls, snapshot: ContextSnapshotV1) -> FrozenMemoryContextV1:
        """Extract only Memory metadata from a validated persisted Snapshot."""

        memory_omissions: list[OmissionRecordV1] = []
        seen_memory_omission = False
        for omission in snapshot.omissions:
            if omission.source_type == "memory":
                seen_memory_omission = True
                memory_omissions.append(omission)
            elif seen_memory_omission:
                raise ValueError("history omission must precede Memory omissions")
        return cls(
            memory=snapshot.memory,
            omissions=tuple(memory_omissions),
            memory_characters=snapshot.budget.memory_characters,
            context_data_characters=snapshot.budget.context_data_characters,
        )


EMPTY_FROZEN_MEMORY_CONTEXT_V1 = FrozenMemoryContextV1((), (), 0, 0)


@dataclass(frozen=True, slots=True, kw_only=True)
class ContextSnapshotV1:
    schema_version: Literal[1] = field(default=1, init=False)
    selection_version: str
    history_groups: tuple[HistoryGroupReferenceV1, ...]
    history_items: tuple[HistoryItemReferenceV1, ...]
    memory: tuple[MemoryReferenceV1, ...]
    budget: InputBudgetRecordV1
    omissions: tuple[OmissionRecordV1, ...]

    def __post_init__(self) -> None:
        if self.selection_version not in REGISTERED_CONTEXT_SELECTION_VERSIONS:
            raise ValueError("Context selection version is unsupported")
        object.__setattr__(self, "history_groups", tuple(self.history_groups))
        object.__setattr__(self, "history_items", tuple(self.history_items))
        object.__setattr__(self, "memory", tuple(self.memory))
        object.__setattr__(self, "omissions", tuple(self.omissions))
        _validate_history_groups(self.history_groups, self.history_items)
        _validate_budget_history_counts(self.history_items, self.budget)
        _validate_context_snapshot_slots(
            memory=self.memory,
            omissions=self.omissions,
            budget=self.budget,
            selected_turn_ids=tuple(group.turn_id for group in self.history_groups),
        )
        maximum = self.budget.maximum_characters
        if (
            maximum != MAXIMUM_INPUT_CHARACTERS_V1
            or self.budget.reserved_current_run_characters
            != RESERVED_CURRENT_RUN_CHARACTERS_V1
        ):
            raise ValueError("Context Snapshot bounded-history-v1 limits are invalid")
        if (
            self.budget.total_characters
            + self.budget.reserved_current_run_characters
            > maximum
        ):
            raise ValueError("Context Snapshot does not preserve current Run capacity")

    def to_wire(self) -> JsonObject:
        return {
            "schemaVersion": self.schema_version,
            "selectionVersion": self.selection_version,
            "historyGroups": [group.to_wire() for group in self.history_groups],
            "historyItems": [item.to_wire() for item in self.history_items],
            "memory": [memory.to_wire() for memory in self.memory],
            "budget": self.budget.to_wire(),
            "omissions": [omission.to_wire() for omission in self.omissions],
        }

    @classmethod
    def from_wire(cls, value: object) -> ContextSnapshotV1:
        row = _object(value, "Context Snapshot", _CONTEXT_SNAPSHOT_KEYS)
        if row["schemaVersion"] != CONTEXT_SNAPSHOT_SCHEMA_VERSION:
            raise ValueError("Context Snapshot schema version is unsupported")
        selection_version = _as_str(row["selectionVersion"])
        if selection_version not in REGISTERED_CONTEXT_SELECTION_VERSIONS:
            raise ValueError("Context selection version is unsupported")
        groups = _array(row["historyGroups"], "history groups")
        items = _array(row["historyItems"], "history Items")
        memory = _array(row["memory"], "Memory references")
        omissions = _array(row["omissions"], "omissions")
        return cls(
            selection_version=selection_version,
            history_groups=tuple(HistoryGroupReferenceV1.from_wire(group) for group in groups),
            history_items=tuple(HistoryItemReferenceV1.from_wire(item) for item in items),
            memory=tuple(MemoryReferenceV1.from_wire(reference) for reference in memory),
            budget=InputBudgetRecordV1.from_wire(row["budget"]),
            omissions=tuple(OmissionRecordV1.from_wire(item) for item in omissions),
        )


@dataclass(frozen=True, slots=True, kw_only=True)
class StepManifestV1:
    schema_version: Literal[1] = field(default=1, init=False)
    step_ordinal: int
    context_snapshot_version: int
    history_items: tuple[HistoryItemReferenceV1, ...]
    memory: tuple[MemoryReferenceV1, ...]
    budget: InputBudgetRecordV1
    omissions: tuple[OmissionRecordV1, ...]

    def __post_init__(self) -> None:
        _positive("Step Manifest ordinal", self.step_ordinal)
        if self.context_snapshot_version != CONTEXT_SNAPSHOT_SCHEMA_VERSION:
            raise ValueError("Step Manifest Context Snapshot version is unsupported")
        object.__setattr__(self, "history_items", tuple(self.history_items))
        object.__setattr__(self, "memory", tuple(self.memory))
        object.__setattr__(self, "omissions", tuple(self.omissions))
        _validate_unique_history_items(self.history_items)
        _validate_budget_history_counts(self.history_items, self.budget)
        _validate_context_snapshot_slots(
            memory=self.memory,
            omissions=self.omissions,
            budget=self.budget,
            selected_turn_ids=tuple(item.turn_id for item in self.history_items),
        )
        if (
            self.budget.maximum_characters != MAXIMUM_INPUT_CHARACTERS_V1
            or self.budget.reserved_current_run_characters
            != RESERVED_CURRENT_RUN_CHARACTERS_V1
        ):
            raise ValueError("Step Manifest bounded-history-v1 limits are invalid")

    def to_wire(self) -> JsonObject:
        return {
            "schemaVersion": self.schema_version,
            "stepOrdinal": self.step_ordinal,
            "contextSnapshotVersion": self.context_snapshot_version,
            "historyItems": [item.to_wire() for item in self.history_items],
            "memory": [memory.to_wire() for memory in self.memory],
            "budget": self.budget.to_wire(),
            "omissions": [omission.to_wire() for omission in self.omissions],
        }

    @classmethod
    def from_wire(cls, value: object) -> StepManifestV1:
        row = _object(value, "Step Manifest", _STEP_MANIFEST_KEYS)
        if row["schemaVersion"] != STEP_MANIFEST_SCHEMA_VERSION:
            raise ValueError("Step Manifest schema version is unsupported")
        items = _array(row["historyItems"], "history Items")
        memory = _array(row["memory"], "Memory references")
        omissions = _array(row["omissions"], "omissions")
        return cls(
            step_ordinal=_as_int(row["stepOrdinal"]),
            context_snapshot_version=_as_int(row["contextSnapshotVersion"]),
            history_items=tuple(HistoryItemReferenceV1.from_wire(item) for item in items),
            memory=tuple(MemoryReferenceV1.from_wire(item) for item in memory),
            budget=InputBudgetRecordV1.from_wire(row["budget"]),
            omissions=tuple(OmissionRecordV1.from_wire(item) for item in omissions),
        )


@dataclass(frozen=True, slots=True)
class PreparedModelStepV1:
    context_snapshot: ContextSnapshotV1
    step_manifest: StepManifestV1
    items: tuple[ContextItem, ...]
    event: JournalEvent


@dataclass(frozen=True, slots=True)
class CompletedProviderStepV1:
    step_id: str | None
    tool_call_item_ids: tuple[str, ...]
    events: tuple[JournalEvent, ...]


def build_context_snapshot(
    records: Sequence[ContextItemRecordV1],
    *,
    current_run_id: str,
    frame: SubmissionFrameV1,
    selection_version: str,
    maximum_characters: int,
    reserved_current_run_characters: int,
    omissions: Sequence[OmissionRecordV1],
    memory_context: FrozenMemoryContextV1 = EMPTY_FROZEN_MEMORY_CONTEXT_V1,
) -> ContextSnapshotV1:
    if selection_version not in REGISTERED_CONTEXT_SELECTION_VERSIONS:
        raise ValueError("Context selection version is unsupported")
    if any(omission.source_type != "history" for omission in omissions):
        raise ValueError("history selection contains a Memory omission")
    _validate_memory_scope_for_frame(memory_context.memory, frame)
    references = tuple(HistoryItemReferenceV1.from_record(record) for record in records)
    groups: list[HistoryGroupReferenceV1] = []
    for reference in references:
        if groups and groups[-1].turn_id == reference.turn_id:
            previous = groups[-1]
            groups[-1] = HistoryGroupReferenceV1(
                turn_id=previous.turn_id,
                item_ids=(*previous.item_ids, reference.item_id),
            )
        else:
            groups.append(
                HistoryGroupReferenceV1(
                    turn_id=reference.turn_id,
                    item_ids=(reference.item_id,),
                )
            )
    history_characters = sum(
        reference.characters for reference in references if reference.run_id != current_run_id
    )
    current_run_characters = sum(
        reference.characters for reference in references if reference.run_id == current_run_id
    )
    return ContextSnapshotV1(
        selection_version=selection_version,
        history_groups=tuple(groups),
        history_items=references,
        memory=memory_context.memory,
        budget=InputBudgetRecordV1(
            mode="bounded",
            measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
            maximum_characters=maximum_characters,
            reserved_current_run_characters=reserved_current_run_characters,
            instruction_characters=_instruction_characters(frame),
            context_data_characters=memory_context.context_data_characters,
            tool_characters=_tool_definition_characters(frame),
            history_characters=history_characters,
            current_run_characters=current_run_characters,
            memory_characters=memory_context.memory_characters,
            total_characters=(
                _instruction_characters(frame)
                + memory_context.context_data_characters
                + _tool_definition_characters(frame)
                + history_characters
                + current_run_characters
                + memory_context.memory_characters
            ),
        ),
        omissions=(*tuple(omissions), *memory_context.omissions),
    )


def build_step_manifest(
    step_ordinal: int,
    records: Sequence[ContextItemRecordV1],
    snapshot: ContextSnapshotV1,
    *,
    current_run_id: str,
    frame: SubmissionFrameV1,
) -> StepManifestV1:
    try:
        references = tuple(HistoryItemReferenceV1.from_record(record) for record in records)
        _validate_memory_scope_for_frame(snapshot.memory, frame)
    except (TypeError, ValueError):
        raise ModelInputUnavailableError("model_input_unavailable") from None
    frozen = snapshot.history_items
    if len(references) < len(frozen) or references[: len(frozen)] != frozen:
        raise ModelInputUnavailableError("model_input_unavailable")
    if any(reference.run_id != current_run_id for reference in references[len(frozen) :]):
        raise ModelInputUnavailableError("model_input_unavailable")
    history_characters = sum(
        reference.characters for reference in references if reference.run_id != current_run_id
    )
    current_run_characters = sum(
        reference.characters for reference in references if reference.run_id == current_run_id
    )
    if history_characters != snapshot.budget.history_characters:
        raise ModelInputUnavailableError("model_input_unavailable")
    if (
        _instruction_characters(frame) != snapshot.budget.instruction_characters
        or _tool_definition_characters(frame) != snapshot.budget.tool_characters
    ):
        raise ModelInputUnavailableError("model_input_unavailable")
    maximum_characters = snapshot.budget.maximum_characters
    total_characters = (
        snapshot.budget.instruction_characters
        + snapshot.budget.context_data_characters
        + snapshot.budget.tool_characters
        + history_characters
        + current_run_characters
        + snapshot.budget.memory_characters
    )
    if maximum_characters is None or total_characters > maximum_characters:
        raise ContextBudgetExceededError("context_budget_exceeded")
    return StepManifestV1(
        step_ordinal=step_ordinal,
        context_snapshot_version=snapshot.schema_version,
        history_items=references,
        memory=snapshot.memory,
        budget=InputBudgetRecordV1(
            mode=snapshot.budget.mode,
            measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
            maximum_characters=maximum_characters,
            reserved_current_run_characters=snapshot.budget.reserved_current_run_characters,
            instruction_characters=snapshot.budget.instruction_characters,
            context_data_characters=snapshot.budget.context_data_characters,
            tool_characters=snapshot.budget.tool_characters,
            history_characters=history_characters,
            current_run_characters=current_run_characters,
            memory_characters=snapshot.budget.memory_characters,
            total_characters=total_characters,
        ),
        omissions=snapshot.omissions,
    )


def validate_tool_environment(
    frozen: Sequence[ToolDefinitionSnapshotV1],
    current: Sequence[ToolDefinition],
) -> bool:
    frozen_by_name = {tool.name: tool.definition_sha256 for tool in frozen}
    current_by_name = {
        tool.name: ToolDefinitionSnapshotV1.from_definition(tool).definition_sha256
        for tool in current
    }
    return frozen_by_name == current_by_name


def validate_run_manifest(value: object, frame: SubmissionFrameV1) -> RunManifestV1:
    row = _object(value, "Run Manifest", _RUN_MANIFEST_KEYS)
    context_selection_version = _as_str(row["contextSelectionVersion"])
    if context_selection_version not in REGISTERED_CONTEXT_SELECTION_VERSIONS:
        raise ValueError("Context selection version is unsupported")
    expected = RunManifestV1.from_frame(
        frame,
        context_selection_version=context_selection_version,
    )
    if value != expected.to_wire():
        raise ValueError("Run Manifest does not match its Submission Frame")
    return expected


def _validate_instruction_slots(
    *,
    output_style: InstructionBlockV1,
    identity_core: InstructionBlockV1 | None,
    skill_catalog: InstructionBlockV1 | None,
) -> None:
    _validate_instruction_slot(
        output_style,
        label="output style",
        expected_id="output-style",
        expected_source="ikaros-runtime:output-style-v1",
        expected_authority="runtime_instruction",
        expected_scope="global",
        expected_lifetime="release",
    )
    if output_style.content != OUTPUT_STYLE_CONTENT:
        raise ValueError("output style instruction slot content is invalid")
    if identity_core is not None:
        _validate_instruction_slot(
            identity_core,
            label="identity core",
            expected_id=IKAROS_IDENTITY_ID,
            expected_source=IKAROS_IDENTITY_SOURCE,
            expected_authority="runtime_identity",
            expected_scope="global",
            expected_lifetime="release",
        )
        if (
            not identity_core.content.strip()
            or len(identity_core.content) > IDENTITY_CORE_MAX_CHARACTERS_V1
        ):
            raise ValueError("identity core instruction content is invalid")
    if skill_catalog is not None:
        _validate_instruction_slot(
            skill_catalog,
            label="Skill catalog",
            expected_id="skill-catalog",
            expected_source="run:skill-descriptors",
            expected_authority="runtime_instruction",
            expected_scope="run",
            expected_lifetime="run",
        )


def _validate_instruction_slot(
    block: InstructionBlockV1,
    *,
    label: str,
    expected_id: str,
    expected_source: str | None,
    expected_authority: InstructionAuthority,
    expected_scope: str,
    expected_lifetime: InputLifetime,
) -> None:
    if (
        block.id != expected_id
        or block.version != 1
        or block.source != expected_source
        or block.authority != expected_authority
        or block.scope != expected_scope
        or block.lifetime != expected_lifetime
    ):
        raise ValueError(f"{label} instruction slot is invalid")


def _validate_history_groups(
    groups: Sequence[HistoryGroupReferenceV1],
    items: Sequence[HistoryItemReferenceV1],
) -> None:
    if not groups or not items:
        raise ValueError("history selection must contain the current User Item")
    _validate_unique_history_items(items)
    group_turn_ids = tuple(group.turn_id for group in groups)
    if len(group_turn_ids) != len(set(group_turn_ids)):
        raise ValueError("history group Turn IDs must be unique")
    item_index = 0
    for group in groups:
        for item_id in group.item_ids:
            if item_index >= len(items):
                raise ValueError("history groups contain a ghost Item")
            item = items[item_index]
            if item.item_id != item_id or item.turn_id != group.turn_id:
                raise ValueError("history groups do not match history Items")
            item_index += 1
    if item_index != len(items):
        raise ValueError("history groups omit history Items")


def _validate_unique_history_items(items: Sequence[HistoryItemReferenceV1]) -> None:
    if not items:
        raise ValueError("history selection must contain the current User Item")
    item_ids = tuple(item.item_id for item in items)
    if len(item_ids) != len(set(item_ids)):
        raise ValueError("history Item IDs must be unique")


def _validate_budget_history_counts(
    items: Sequence[HistoryItemReferenceV1],
    budget: InputBudgetRecordV1,
) -> None:
    current_run_id = items[-1].run_id if items else None
    history_characters = sum(
        item.characters for item in items if item.run_id != current_run_id
    )
    current_run_characters = sum(
        item.characters for item in items if item.run_id == current_run_id
    )
    if (
        budget.history_characters != history_characters
        or budget.current_run_characters != current_run_characters
    ):
        raise ValueError("input budget history counts do not match history Items")


def _validate_skill_catalog_content(
    skills: Sequence[SkillDescriptor],
    skill_catalog: InstructionBlockV1 | None,
) -> None:
    from .skills import build_skill_prompt

    expected = build_skill_prompt(skills)
    actual = skill_catalog.content if skill_catalog is not None else None
    if actual != expected:
        raise ValueError("Skill catalog instruction content is invalid")


def _validate_context_snapshot_slots(
    *,
    memory: Sequence[MemoryReferenceV1],
    omissions: Sequence[OmissionRecordV1],
    budget: InputBudgetRecordV1,
    selected_turn_ids: Sequence[str],
) -> None:
    if budget.mode != "bounded":
        raise ValueError("history selection budget mode is invalid")
    history_omissions: list[OmissionRecordV1] = []
    memory_omissions: list[OmissionRecordV1] = []
    seen_memory_omission = False
    for omission in omissions:
        if omission.source_type == "history":
            if seen_memory_omission:
                raise ValueError("history omission must precede Memory omissions")
            history_omissions.append(omission)
        else:
            seen_memory_omission = True
            memory_omissions.append(omission)
    if len(history_omissions) > 1:
        raise ValueError("history selection has multiple omission boundaries")
    selected = set(selected_turn_ids)
    if history_omissions and history_omissions[0].source_id in selected:
        raise ValueError("history omission boundary is selected")
    FrozenMemoryContextV1(
        memory=tuple(memory),
        omissions=tuple(memory_omissions),
        memory_characters=budget.memory_characters,
        context_data_characters=budget.context_data_characters,
    )


def _validate_memory_scope_for_frame(
    memory: Sequence[MemoryReferenceV1],
    frame: SubmissionFrameV1,
) -> None:
    if frame.workspace is None and any(reference.scope == "workspace" for reference in memory):
        raise ValueError("Workspace Memory requires a Run workspace")


def _instruction_characters(frame: SubmissionFrameV1) -> int:
    return sum(len(block.content) for block in frame.instructions)


def _tool_definition_characters(frame: SubmissionFrameV1) -> int:
    return sum(
        len(
            canonical_json(
                {
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                }
            )
        )
        for tool in frame.tools
    )


def frame_input_character_counts(frame: SubmissionFrameV1) -> tuple[int, int]:
    """Return canonical Instruction and Tool-definition character counts."""

    return _instruction_characters(frame), _tool_definition_characters(frame)


def _skill_snapshot(skills: Sequence[SkillDescriptor]) -> tuple[SkillDescriptor, ...]:
    copied = tuple(SkillDescriptor.from_wire(skill.to_wire()) for skill in skills)
    return tuple(sorted(copied, key=lambda skill: skill.name))


def _workspace_from_wire(value: object) -> WorkspaceSummary | None:
    if value is None:
        return None
    row = _object(value, "workspace", {"id", "name", "rootUri"})
    root_uri = row["rootUri"]
    if root_uri is not None and not isinstance(root_uri, str):
        raise ValueError("workspace root URI is invalid")
    return WorkspaceSummary(
        id=_as_str(row["id"]),
        name=_as_str(row["name"]),
        root_uri=root_uri,
    )


def _object(value: object, label: str, keys: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != keys:
        raise ValueError(f"{label} has an invalid structure")
    return cast(dict[str, Any], value)


def _array(value: object, label: str) -> list[Any]:
    if not isinstance(value, list):
        raise ValueError(f"{label} must be an array")
    return value


def _as_str(value: object, *, allow_empty: bool = False) -> str:
    if not isinstance(value, str) or (not allow_empty and not value):
        raise ValueError("expected a string")
    return value


def _as_int(value: object) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError("expected an integer")
    return value


def _optional_positive_int(value: object, *, label: str) -> int | None:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise ValueError(f"{label} is invalid")
    return value


def _nonempty(label: str, value: object) -> None:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} is invalid")


def _text(label: str, value: object) -> None:
    if not isinstance(value, str):
        raise ValueError(f"{label} is invalid")


def _positive(label: str, value: object) -> None:
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise ValueError(f"{label} is invalid")


def _sha256(label: str, value: object) -> None:
    if (
        not isinstance(value, str)
        or len(value) != 64
        or any(character not in "0123456789abcdef" for character in value)
    ):
        raise ValueError(f"{label} is invalid")


def _runtime_id(label: str, value: object, prefix: str) -> None:
    if not isinstance(value, str) or not value.startswith(prefix):
        raise ValueError(f"{label} is invalid")
    suffix = value.removeprefix(prefix)
    if len(suffix) != 32 or any(character not in "0123456789abcdef" for character in suffix):
        raise ValueError(f"{label} is invalid")


def _unique_names(label: str, names: Iterable[str]) -> None:
    copied = tuple(names)
    if len(copied) != len(set(copied)):
        raise ValueError(f"{label} names must be unique")


_INSTRUCTION_KEYS = {"id", "version", "source", "authority", "scope", "lifetime", "content"}
_TOOL_SNAPSHOT_KEYS = {"name", "description", "inputSchema", "definitionSha256"}
_MEMORY_REFERENCE_KEYS = {
    "memoryId",
    "revision",
    "scope",
    "characters",
}
_HISTORY_OMISSION_KEYS = {"sourceType", "sourceId", "reason"}
_MEMORY_OMISSION_KEYS = {
    "sourceType",
    "sourceId",
    "revision",
    "characters",
    "reason",
}
_INSTRUCTION_SLOT_KEYS = {"outputStyle", "identityCore", "skillCatalog"}
_SUBMISSION_FRAME_KEYS = {
    "schemaVersion",
    "userItemId",
    "threadId",
    "branchId",
    "turnId",
    "runId",
    "workspace",
    "providerId",
    "modelId",
    "publicProviderConfigFingerprint",
    "executionPolicy",
    "skills",
    "tools",
    "instructions",
    "contextData",
    "maxSteps",
}
_HISTORY_ITEM_KEYS = {"itemId", "turnId", "runId", "kind", "role", "characters"}
_INPUT_BUDGET_KEYS = {
    "mode",
    "measurementVersion",
    "maximumCharacters",
    "reservedCurrentRunCharacters",
    "instructionCharacters",
    "contextDataCharacters",
    "toolCharacters",
    "historyCharacters",
    "currentRunCharacters",
    "memoryCharacters",
    "totalCharacters",
}
_RUN_MANIFEST_KEYS = {
    "schemaVersion",
    "runId",
    "modelInputPlanVersion",
    "submissionFrameVersion",
    "contextSelectionVersion",
    "memoryContextVersion",
    "instructions",
    "skills",
    "tools",
    "provider",
    "executionPolicy",
    "maxSteps",
}
_CONTEXT_SNAPSHOT_KEYS = {
    "schemaVersion",
    "selectionVersion",
    "historyGroups",
    "historyItems",
    "memory",
    "budget",
    "omissions",
}
_STEP_MANIFEST_KEYS = {
    "schemaVersion",
    "stepOrdinal",
    "contextSnapshotVersion",
    "historyItems",
    "memory",
    "budget",
    "omissions",
}


__all__ = [
    "CONTEXT_SELECTION_VERSION",
    "EXECUTABLE_CONTEXT_SELECTION_VERSIONS",
    "IDENTITY_CORE_MAX_CHARACTERS_V1",
    "IKAROS_IDENTITY_ID",
    "IKAROS_IDENTITY_SOURCE",
    "IKAROS_IDENTITY_VERSION",
    "INPUT_BUDGET_MEASUREMENT_VERSION",
    "MAXIMUM_INPUT_CHARACTERS_V1",
    "MEMORY_CONTENT_MAX_CHARACTERS_V1",
    "MEMORY_RETRIEVAL_MAX_CANDIDATES_V1",
    "MEMORY_SELECTION_MAX_CHARACTERS_V1",
    "MEMORY_SELECTION_MAX_ITEMS_V1",
    "REGISTERED_CONTEXT_SELECTION_VERSIONS",
    "REGISTERED_INPUT_BUDGET_MODES",
    "RESERVED_CURRENT_RUN_CHARACTERS_V1",
    "CompletedProviderStepV1",
    "ContextDataBlockV1",
    "ContextItemRecordV1",
    "ContextSnapshotV1",
    "EMPTY_FROZEN_MEMORY_CONTEXT_V1",
    "FrozenMemoryContextV1",
    "HistoryItemReferenceV1",
    "InputAuthority",
    "InputBudgetRecordV1",
    "InputLifetime",
    "InstructionAuthority",
    "InstructionBlockV1",
    "MemoryReferenceV1",
    "MemoryScope",
    "ModelStepOutcome",
    "OmissionRecordV1",
    "PreparedModelStepV1",
    "ProviderExecutionSnapshotV1",
    "RunManifestV1",
    "StepManifestV1",
    "SubmissionFrameTemplateV1",
    "SubmissionFrameV1",
    "ToolDefinitionSnapshotV1",
    "build_context_snapshot",
    "build_step_manifest",
    "canonical_json",
    "canonical_sha256",
    "deep_frozen_json_object",
    "frame_input_character_counts",
    "validate_run_manifest",
    "validate_tool_environment",
]
