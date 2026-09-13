"""Durable, provider-neutral input snapshots for one Agent Run.

The records in this module deliberately contain no credentials, user-message copy,
Memory body, Tool arguments/results, or provider request body.  They are shared by
the Agent and storage layers, so this module must not import either package.
"""

from __future__ import annotations

import hashlib
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Literal, cast

from .domain import ContextItem, JournalEvent, JsonObject, SkillDescriptor, WorkspaceSummary
from .errors import ContextBudgetExceededError, ModelInputUnavailableError
from .history_status import EMPTY_HISTORY_STATUS_V1, FrozenHistoryStatusV1
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

INPUT_BUDGET_MEASUREMENT_VERSION = "conservative-utf8-upper-bound"
MEMORY_CONTENT_MAX_CHARACTERS_V1 = 2_048
MEMORY_RETRIEVAL_MAX_CANDIDATES_V1 = 2_000
MEMORY_SELECTION_MAX_ITEMS_V1 = 8
MEMORY_SELECTION_MAX_CHARACTERS_V1 = 6_000
IKAROS_IDENTITY_ID = "ikaros-identity"
IKAROS_IDENTITY_VERSION = 1
IKAROS_IDENTITY_SOURCE = "ikaros-runtime:identity"
IDENTITY_CORE_MAX_CHARACTERS_V1 = 2_048

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
class ProviderExecutionSnapshot:
    provider_id: str
    origin: str
    base_url: str | None
    model_id: str
    supports_tools: bool
    context_window: int = 32768
    max_output_tokens: int = 4096

    def __post_init__(self) -> None:
        _nonempty("provider ID", self.provider_id)
        _nonempty("provider origin", self.origin)
        if self.base_url is not None:
            _nonempty("provider base URL", self.base_url)
        _nonempty("model ID", self.model_id)
        if not isinstance(self.supports_tools, bool):
            raise ValueError("model Tool capability is invalid")
        _validate_model_window(self.context_window, self.max_output_tokens)

    def public_wire(self) -> JsonObject:
        return {
            "providerId": self.provider_id,
            "origin": self.origin,
            "baseUrl": self.base_url,
            "modelId": self.model_id,
            "supportsTools": self.supports_tools,
            "contextWindow": self.context_window,
            "maxOutputTokens": self.max_output_tokens,
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
class RunConfigTemplate:
    provider: ProviderExecutionSnapshot
    execution_policy: str
    skills: tuple[SkillDescriptor, ...]
    tools: tuple[ToolDefinitionSnapshotV1, ...]
    output_style: InstructionBlockV1
    identity_core: InstructionBlockV1 | None
    skill_catalog: InstructionBlockV1 | None

    def __post_init__(self) -> None:
        object.__setattr__(self, "skills", _skill_snapshot(self.skills))
        object.__setattr__(self, "tools", tuple(self.tools))
        _nonempty("execution policy", self.execution_policy)
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
        _validate_skill_catalog_content(self.skills, self.skill_catalog)

    @classmethod
    def create(
        cls,
        *,
        provider: ProviderExecutionSnapshot,
        execution_policy: str,
        skills: Sequence[SkillDescriptor],
        tools: Sequence[ToolDefinition],
        identity_core: InstructionBlockV1 | None,
    ) -> RunConfigTemplate:
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
        )


@dataclass(frozen=True, slots=True, kw_only=True)
class RunConfig:
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

    context_window: int = 32768
    max_output_tokens: int = 4096

    @property
    def maximum_input_tokens(self) -> int:
        return self.context_window - self.max_output_tokens

    @property
    def reserved_current_run_tokens(self) -> int:
        return max(1, self.maximum_input_tokens // 4)

    def __post_init__(self) -> None:
        _validate_model_window(self.context_window, self.max_output_tokens)
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
        _validate_skill_catalog_content(self.skills, self.skill_catalog)

    @classmethod
    def from_template(
        cls,
        template: RunConfigTemplate,
        *,
        user_item_id: str,
        thread_id: str,
        branch_id: str,
        turn_id: str,
        run_id: str,
        workspace: WorkspaceSummary | None,
    ) -> RunConfig:
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
            context_window=template.provider.context_window,
            max_output_tokens=template.provider.max_output_tokens,
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
            "contextWindow": self.context_window,
            "maxOutputTokens": self.max_output_tokens,
        }

    @classmethod
    def from_wire(cls, value: object) -> RunConfig:
        row = _object(value, "Run configuration", _RUN_CONFIG_KEYS)
        workspace_value = row["workspace"]
        workspace = _workspace_from_wire(workspace_value)
        skills_value = row["skills"]
        tools_value = row["tools"]
        instructions = _object(row["instructions"], "instruction slots", _INSTRUCTION_SLOT_KEYS)
        if not isinstance(skills_value, list) or not isinstance(tools_value, list):
            raise ValueError("Run configuration snapshots are invalid")
        return cls(
            user_item_id=_as_str(row["userItemId"]),
            thread_id=_as_str(row["threadId"]),
            branch_id=_as_str(row["branchId"]),
            turn_id=_as_str(row["turnId"]),
            run_id=_as_str(row["runId"]),
            workspace=workspace,
            provider_id=_as_str(row["providerId"]),
            model_id=_as_str(row["modelId"]),
            public_provider_config_fingerprint=_as_str(row["publicProviderConfigFingerprint"]),
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
            context_window=_as_int(row["contextWindow"]),
            max_output_tokens=_as_int(row["maxOutputTokens"]),
        )


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
    def estimated_tokens(self) -> int:
        if self.kind == "message":
            if self.role not in {"user", "assistant"}:
                raise ValueError("message context Item has an invalid role")
            return estimate_tokens(self.content) + 64
        if self.kind == "tool_call":
            if self.role != "assistant" or not isinstance(self.data.get("arguments"), dict):
                raise ValueError("Tool Call context Item data is invalid")
            reasoning = self.data.get("reasoningContent")
            if reasoning is not None and not isinstance(reasoning, str):
                raise ValueError("Tool Call reasoning content is invalid")
            envelope = {
                "id": self.data.get("callId"),
                "name": self.data.get("toolName"),
                "arguments": canonical_json(self.data["arguments"]),
            }
            return estimate_tokens(canonical_json(envelope)) + estimate_tokens(reasoning or "") + 64
        if self.kind == "tool_result":
            if self.role != "tool" or not isinstance(self.data.get("result"), dict):
                raise ValueError("Tool Result context Item data is invalid")
            return estimate_tokens(self.content) + 64
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
    tokens: int

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
        if not isinstance(self.tokens, int) or isinstance(self.tokens, bool) or self.tokens < 0:
            raise ValueError("history Item character count is invalid")

    @classmethod
    def from_record(cls, record: ContextItemRecordV1) -> HistoryItemReferenceV1:
        return cls(
            item_id=record.item_id,
            turn_id=record.turn_id,
            run_id=record.run_id,
            kind=record.kind,
            role=record.role,
            tokens=record.estimated_tokens,
        )

    def to_wire(self) -> JsonObject:
        return {
            "itemId": self.item_id,
            "turnId": self.turn_id,
            "runId": self.run_id,
            "kind": self.kind,
            "role": self.role,
            "tokens": self.tokens,
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
            tokens=_as_int(row["tokens"]),
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
class InputBudgetRecord:
    mode: str
    measurement_version: str
    maximum_tokens: int
    reserved_current_run_tokens: int
    instruction_tokens: int
    context_data_tokens: int
    tool_tokens: int
    history_tokens: int
    current_run_tokens: int
    memory_tokens: int
    total_tokens: int

    def __post_init__(self) -> None:
        if self.mode != "bounded":
            raise ValueError("input budget mode is unsupported")
        if self.measurement_version != INPUT_BUDGET_MEASUREMENT_VERSION:
            raise ValueError("input budget measurement version is unsupported")
        if self.maximum_tokens is not None and (
            not isinstance(self.maximum_tokens, int)
            or isinstance(self.maximum_tokens, bool)
            or self.maximum_tokens < 1
        ):
            raise ValueError("input budget maximum is invalid")
        if self.mode == "bounded" and (
            self.maximum_tokens is None
            or self.reserved_current_run_tokens == 0
            or self.reserved_current_run_tokens >= self.maximum_tokens
        ):
            raise ValueError("bounded input budget limits are invalid")
        counts = (
            self.reserved_current_run_tokens,
            self.instruction_tokens,
            self.context_data_tokens,
            self.tool_tokens,
            self.history_tokens,
            self.current_run_tokens,
            self.memory_tokens,
            self.total_tokens,
        )
        if any(
            not isinstance(count, int) or isinstance(count, bool) or count < 0 for count in counts
        ):
            raise ValueError("input budget counts are invalid")
        actual_parts = counts[1:7]
        if self.total_tokens != sum(actual_parts):
            raise ValueError("input budget total is invalid")
        if self.maximum_tokens is not None and self.total_tokens > self.maximum_tokens:
            raise ValueError("input budget exceeds its maximum")

    def to_wire(self) -> JsonObject:
        return {
            "mode": self.mode,
            "measurementVersion": self.measurement_version,
            "maximumTokens": self.maximum_tokens,
            "reservedCurrentRunTokens": self.reserved_current_run_tokens,
            "instructionTokens": self.instruction_tokens,
            "contextDataTokens": self.context_data_tokens,
            "toolTokens": self.tool_tokens,
            "historyTokens": self.history_tokens,
            "currentRunTokens": self.current_run_tokens,
            "memoryTokens": self.memory_tokens,
            "totalTokens": self.total_tokens,
        }

    @classmethod
    def from_wire(cls, value: object) -> InputBudgetRecord:
        row = _object(value, "input budget", _INPUT_BUDGET_KEYS)
        result = cls(
            mode=_as_str(row["mode"]),
            measurement_version=_as_str(row["measurementVersion"]),
            maximum_tokens=_as_int(row["maximumTokens"]),
            reserved_current_run_tokens=_as_int(row["reservedCurrentRunTokens"]),
            instruction_tokens=_as_int(row["instructionTokens"]),
            context_data_tokens=_as_int(row["contextDataTokens"]),
            tool_tokens=_as_int(row["toolTokens"]),
            history_tokens=_as_int(row["historyTokens"]),
            current_run_tokens=_as_int(row["currentRunTokens"]),
            memory_tokens=_as_int(row["memoryTokens"]),
            total_tokens=_as_int(row["totalTokens"]),
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
    """Body-free Memory selection materialized independently for model input."""

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
        if (
            any(omission.reason == "omitted_by_limit" for omission in self.omissions)
            and len(self.memory) != MEMORY_SELECTION_MAX_ITEMS_V1
        ):
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
    def from_revision(cls, snapshot: ContextRevision) -> FrozenMemoryContextV1:
        """Extract independently stored Memory metadata from a context revision."""

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
            memory_characters=sum(reference.characters for reference in snapshot.memory),
            context_data_characters=snapshot.memory_context_characters,
        )


EMPTY_FROZEN_MEMORY_CONTEXT_V1 = FrozenMemoryContextV1((), (), 0, 0)


@dataclass(frozen=True, slots=True, kw_only=True)
class ContextRevision:
    """Append-only selected context; later compaction creates another revision."""

    revision: int = 1
    history_groups: tuple[HistoryGroupReferenceV1, ...]
    history_items: tuple[HistoryItemReferenceV1, ...]
    memory: tuple[MemoryReferenceV1, ...]
    budget: InputBudgetRecord
    omissions: tuple[OmissionRecordV1, ...]
    history_status: FrozenHistoryStatusV1 = EMPTY_HISTORY_STATUS_V1
    memory_context_characters: int = 0
    compaction_summary: str = ""
    current_run_omitted_through_item_id: str = ""

    def __post_init__(self) -> None:
        _positive("Context revision", self.revision)
        for name in ("history_groups", "history_items", "memory", "omissions"):
            object.__setattr__(self, name, tuple(getattr(self, name)))
        if not isinstance(self.compaction_summary, str) or len(self.compaction_summary) > 6000:
            raise ValueError("Context compaction summary is invalid")
        if not isinstance(self.current_run_omitted_through_item_id, str):
            raise ValueError("Context current Run omission boundary is invalid")
        if self.current_run_omitted_through_item_id:
            _runtime_id(
                "Context current Run omission boundary Item ID",
                self.current_run_omitted_through_item_id,
                "item_",
            )
        _validate_history_groups(self.history_groups, self.history_items)
        _validate_budget_history_counts(self.history_items, self.budget)
        _validate_context_snapshot_slots(
            memory=self.memory,
            omissions=self.omissions,
            budget=self.budget,
            history_status=self.history_status,
            history_items=self.history_items,
            memory_context_characters=self.memory_context_characters,
            compaction_summary=self.compaction_summary,
            selected_turn_ids=tuple(group.turn_id for group in self.history_groups),
        )
        if (
            self.budget.total_tokens + self.budget.reserved_current_run_tokens
            > self.budget.maximum_tokens
        ):
            raise ValueError("Context revision does not preserve current Run capacity")

    def to_wire(self) -> JsonObject:
        payload: JsonObject = {
            "revision": self.revision,
            "historyGroups": [group.to_wire() for group in self.history_groups],
            "historyItems": [item.to_wire() for item in self.history_items],
            "memory": [memory.to_wire() for memory in self.memory],
            "budget": self.budget.to_wire(),
            "omissions": [omission.to_wire() for omission in self.omissions],
            "historyStatus": self.history_status.to_wire(),
            "memoryContextCharacters": self.memory_context_characters,
        }
        if self.compaction_summary:
            payload["compactionSummary"] = self.compaction_summary
        if self.current_run_omitted_through_item_id:
            payload["currentRunOmittedThroughItemId"] = self.current_run_omitted_through_item_id
        return payload

    @classmethod
    def from_wire(cls, value: object) -> ContextRevision:
        boundary_present = (
            isinstance(value, dict) and "currentRunOmittedThroughItemId" in value
        )
        if isinstance(value, dict):
            value = {
                **value,
                "compactionSummary": value.get("compactionSummary", ""),
                "currentRunOmittedThroughItemId": value.get(
                    "currentRunOmittedThroughItemId", ""
                ),
            }
        row = _object(value, "Context revision", _CONTEXT_REVISION_KEYS)
        return cls(
            revision=_as_int(row["revision"]),
            history_groups=tuple(
                HistoryGroupReferenceV1.from_wire(v)
                for v in _array(row["historyGroups"], "history groups")
            ),
            history_items=tuple(
                HistoryItemReferenceV1.from_wire(v)
                for v in _array(row["historyItems"], "history Items")
            ),
            memory=tuple(
                MemoryReferenceV1.from_wire(v) for v in _array(row["memory"], "Memory references")
            ),
            budget=InputBudgetRecord.from_wire(row["budget"]),
            omissions=tuple(
                OmissionRecordV1.from_wire(v) for v in _array(row["omissions"], "omissions")
            ),
            history_status=FrozenHistoryStatusV1.from_wire(row["historyStatus"]),
            memory_context_characters=_as_int(row["memoryContextCharacters"]),
            compaction_summary=_as_str(row.get("compactionSummary", ""), allow_empty=True),
            current_run_omitted_through_item_id=_as_str(
                row["currentRunOmittedThroughItemId"], allow_empty=not boundary_present
            ),
        )


@dataclass(frozen=True, slots=True, kw_only=True)
class StepInput:
    """Frozen input boundary and accounting for one model call."""

    step_ordinal: int
    context_revision: int
    history_items: tuple[HistoryItemReferenceV1, ...]
    memory: tuple[MemoryReferenceV1, ...]
    budget: InputBudgetRecord
    omissions: tuple[OmissionRecordV1, ...]
    history_status: FrozenHistoryStatusV1 = EMPTY_HISTORY_STATUS_V1
    memory_context_characters: int = 0
    compaction_summary: str = ""

    def __post_init__(self) -> None:
        _positive("Step ordinal", self.step_ordinal)
        _positive("Context revision", self.context_revision)
        if not isinstance(self.compaction_summary, str) or len(self.compaction_summary) > 6000:
            raise ValueError("Step input compaction summary is invalid")
        for name in ("history_items", "memory", "omissions"):
            object.__setattr__(self, name, tuple(getattr(self, name)))
        _validate_unique_history_items(self.history_items)
        _validate_budget_history_counts(self.history_items, self.budget)
        _validate_context_snapshot_slots(
            memory=self.memory,
            omissions=self.omissions,
            budget=self.budget,
            history_status=self.history_status,
            history_items=self.history_items,
            memory_context_characters=self.memory_context_characters,
            compaction_summary=self.compaction_summary,
            selected_turn_ids=tuple(item.turn_id for item in self.history_items),
        )

    def to_wire(self) -> JsonObject:
        payload: JsonObject = {
            "stepOrdinal": self.step_ordinal,
            "contextRevision": self.context_revision,
            "historyItems": [item.to_wire() for item in self.history_items],
            "memory": [memory.to_wire() for memory in self.memory],
            "budget": self.budget.to_wire(),
            "omissions": [omission.to_wire() for omission in self.omissions],
            "historyStatus": self.history_status.to_wire(),
            "memoryContextCharacters": self.memory_context_characters,
        }
        if self.compaction_summary:
            payload["compactionSummary"] = self.compaction_summary
        return payload

    @classmethod
    def from_wire(cls, value: object) -> StepInput:
        if isinstance(value, dict) and "compactionSummary" not in value:
            value = {**value, "compactionSummary": ""}
        row = _object(value, "Step input", _STEP_INPUT_KEYS)
        return cls(
            step_ordinal=_as_int(row["stepOrdinal"]),
            context_revision=_as_int(row["contextRevision"]),
            history_items=tuple(
                HistoryItemReferenceV1.from_wire(v)
                for v in _array(row["historyItems"], "history Items")
            ),
            memory=tuple(
                MemoryReferenceV1.from_wire(v) for v in _array(row["memory"], "Memory references")
            ),
            budget=InputBudgetRecord.from_wire(row["budget"]),
            omissions=tuple(
                OmissionRecordV1.from_wire(v) for v in _array(row["omissions"], "omissions")
            ),
            history_status=FrozenHistoryStatusV1.from_wire(row["historyStatus"]),
            memory_context_characters=_as_int(row["memoryContextCharacters"]),
            compaction_summary=_as_str(row.get("compactionSummary", ""), allow_empty=True),
        )


@dataclass(frozen=True, slots=True)
class PreparedModelStep:
    context_revision: ContextRevision
    step_input: StepInput
    items: tuple[ContextItem, ...]
    event: JournalEvent
    pre_events: tuple[JournalEvent, ...] = ()


@dataclass(frozen=True, slots=True)
class CompletedProviderStepV1:
    step_id: str | None
    tool_call_item_ids: tuple[str, ...]
    events: tuple[JournalEvent, ...]


def build_context_revision(
    records: Sequence[ContextItemRecordV1],
    *,
    current_run_id: str,
    config: RunConfig,
    maximum_tokens: int,
    reserved_current_run_tokens: int,
    omissions: Sequence[OmissionRecordV1],
    memory_context: FrozenMemoryContextV1 = EMPTY_FROZEN_MEMORY_CONTEXT_V1,
    history_status: FrozenHistoryStatusV1 = EMPTY_HISTORY_STATUS_V1,
    compaction_summary: str = "",
    current_run_omitted_through_item_id: str = "",
) -> ContextRevision:
    if (
        maximum_tokens != config.maximum_input_tokens
        or reserved_current_run_tokens != config.reserved_current_run_tokens
    ):
        raise ValueError("context budget does not match the frozen model configuration")
    if any(omission.source_type != "history" for omission in omissions):
        raise ValueError("history selection contains a Memory omission")
    _validate_memory_scope_for_config(memory_context.memory, config)
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
    history_tokens = sum(
        reference.tokens for reference in references if reference.run_id != current_run_id
    )
    current_run_tokens = sum(
        reference.tokens for reference in references if reference.run_id == current_run_id
    )
    return ContextRevision(
        history_status=history_status,
        memory_context_characters=memory_context.context_data_characters,
        compaction_summary=compaction_summary,
        current_run_omitted_through_item_id=current_run_omitted_through_item_id,
        history_groups=tuple(groups),
        history_items=references,
        memory=memory_context.memory,
        budget=InputBudgetRecord(
            mode="bounded",
            measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
            maximum_tokens=maximum_tokens,
            reserved_current_run_tokens=reserved_current_run_tokens,
            instruction_tokens=_instruction_tokens(config),
            context_data_tokens=memory_context.context_data_characters * 4
            + history_status_tokens(history_status)
            + len(compaction_summary) * 4,
            tool_tokens=_tool_definition_tokens(config),
            history_tokens=history_tokens,
            current_run_tokens=current_run_tokens,
            memory_tokens=memory_context.memory_characters * 4,
            total_tokens=(
                _instruction_tokens(config)
                + memory_context.context_data_characters * 4
                + history_status_tokens(history_status)
                + len(compaction_summary) * 4
                + _tool_definition_tokens(config)
                + history_tokens
                + current_run_tokens
                + memory_context.memory_characters * 4
            ),
        ),
        omissions=(*tuple(omissions), *memory_context.omissions),
    )


def build_step_input(
    step_ordinal: int,
    records: Sequence[ContextItemRecordV1],
    snapshot: ContextRevision,
    *,
    current_run_id: str,
    config: RunConfig,
) -> StepInput:
    try:
        references = tuple(HistoryItemReferenceV1.from_record(record) for record in records)
        _validate_memory_scope_for_config(snapshot.memory, config)
    except (TypeError, ValueError):
        raise ModelInputUnavailableError("model_input_unavailable") from None
    frozen = snapshot.history_items
    if len(references) < len(frozen) or references[: len(frozen)] != frozen:
        raise ModelInputUnavailableError("model_input_unavailable")
    if any(reference.run_id != current_run_id for reference in references[len(frozen) :]):
        raise ModelInputUnavailableError("model_input_unavailable")
    history_tokens = sum(
        reference.tokens for reference in references if reference.run_id != current_run_id
    )
    current_run_tokens = sum(
        reference.tokens for reference in references if reference.run_id == current_run_id
    )
    if history_tokens != snapshot.budget.history_tokens:
        raise ModelInputUnavailableError("model_input_unavailable")
    if (
        _instruction_tokens(config) != snapshot.budget.instruction_tokens
        or _tool_definition_tokens(config) != snapshot.budget.tool_tokens
        or snapshot.budget.maximum_tokens != config.maximum_input_tokens
        or snapshot.budget.reserved_current_run_tokens != config.reserved_current_run_tokens
    ):
        raise ModelInputUnavailableError("model_input_unavailable")
    maximum_tokens = snapshot.budget.maximum_tokens
    total_tokens = (
        snapshot.budget.instruction_tokens
        + snapshot.budget.context_data_tokens
        + snapshot.budget.tool_tokens
        + history_tokens
        + current_run_tokens
        + snapshot.budget.memory_tokens
    )
    if maximum_tokens is None or total_tokens > maximum_tokens:
        raise ContextBudgetExceededError("context_budget_exceeded")
    return StepInput(
        history_status=snapshot.history_status,
        memory_context_characters=snapshot.memory_context_characters,
        compaction_summary=snapshot.compaction_summary,
        step_ordinal=step_ordinal,
        context_revision=snapshot.revision,
        history_items=references,
        memory=snapshot.memory,
        budget=InputBudgetRecord(
            mode=snapshot.budget.mode,
            measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
            maximum_tokens=maximum_tokens,
            reserved_current_run_tokens=snapshot.budget.reserved_current_run_tokens,
            instruction_tokens=snapshot.budget.instruction_tokens,
            context_data_tokens=snapshot.budget.context_data_tokens,
            tool_tokens=snapshot.budget.tool_tokens,
            history_tokens=history_tokens,
            current_run_tokens=current_run_tokens,
            memory_tokens=snapshot.budget.memory_tokens,
            total_tokens=total_tokens,
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
    budget: InputBudgetRecord,
) -> None:
    current_run_id = items[-1].run_id if items else None
    history_tokens = sum(item.tokens for item in items if item.run_id != current_run_id)
    current_run_tokens = sum(item.tokens for item in items if item.run_id == current_run_id)
    if budget.history_tokens != history_tokens or budget.current_run_tokens != current_run_tokens:
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
    budget: InputBudgetRecord,
    selected_turn_ids: Sequence[str],
    history_status: FrozenHistoryStatusV1 = EMPTY_HISTORY_STATUS_V1,
    history_items: Sequence[HistoryItemReferenceV1] = (),
    memory_context_characters: int = 0,
    compaction_summary: str = "",
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
        memory_characters=sum(reference.characters for reference in memory),
        context_data_characters=memory_context_characters,
    )
    if budget.memory_tokens != sum(reference.characters for reference in memory) * 4:
        raise ValueError("Memory token estimate does not match its frozen references")
    if budget.context_data_tokens != (
        memory_context_characters * 4
        + history_status_tokens(history_status)
        + len(compaction_summary) * 4
    ):
        raise ValueError("context-data token estimate does not match its frozen metadata")
    included_runs = {(item.turn_id, item.run_id) for item in history_items}
    ordered_runs = list(dict.fromkeys(item.run_id for item in history_items))
    current_run_id = history_items[-1].run_id if history_items else None
    last_run_index = -1
    omitted_turns = {omission.source_id for omission in history_omissions}
    for run in history_status.runs:
        if run.run_id == current_run_id:
            raise ValueError("history status contains the current Run")
        if run.details == "included":
            if (run.turn_id, run.run_id) not in included_runs:
                raise ValueError("history status has no selected Run")
            run_index = ordered_runs.index(run.run_id)
            if run_index <= last_run_index:
                raise ValueError("history status Runs are not chronological")
            last_run_index = run_index
        elif (
            run.turn_id not in omitted_turns
            or run.turn_id in selected
            or any(item.run_id != current_run_id for item in history_items)
            or any(status.details == "included" for status in history_status.runs)
        ):
            raise ValueError("history status omission does not match history boundary")


def _validate_memory_scope_for_config(
    memory: Sequence[MemoryReferenceV1],
    config: RunConfig,
) -> None:
    if config.workspace is None and any(reference.scope == "workspace" for reference in memory):
        raise ValueError("Workspace Memory requires a Run workspace")


def estimate_tokens(content: str) -> int:
    """UTF-8 bytes form a conservative token bound, never an exact tokenizer count."""

    return len(content.encode("utf-8"))


def history_status_tokens(status: FrozenHistoryStatusV1) -> int:
    return estimate_tokens(status.content) + 64 if status.runs else 0


def _instruction_tokens(config: RunConfig) -> int:
    # Reserve request/message framing in addition to visible instruction content.
    return 512 + sum(estimate_tokens(block.content) + 64 for block in config.instructions)


def _tool_definition_tokens(config: RunConfig) -> int:
    return sum(
        estimate_tokens(
            canonical_json(
                {
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                }
            )
        )
        + 64
        for tool in config.tools
    )


def config_input_token_counts(config: RunConfig) -> tuple[int, int]:
    return _instruction_tokens(config), _tool_definition_tokens(config)


def _validate_model_window(context_window: int, max_output_tokens: int) -> None:
    _positive("model context window", context_window)
    _positive("model output reserve", max_output_tokens)
    if max_output_tokens >= context_window:
        raise ValueError("model output reserve must be smaller than its context window")


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
_RUN_CONFIG_KEYS = {
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
    "contextWindow",
    "maxOutputTokens",
}
_HISTORY_ITEM_KEYS = {"itemId", "turnId", "runId", "kind", "role", "tokens"}
_INPUT_BUDGET_KEYS = {
    "mode",
    "measurementVersion",
    "maximumTokens",
    "reservedCurrentRunTokens",
    "instructionTokens",
    "contextDataTokens",
    "toolTokens",
    "historyTokens",
    "currentRunTokens",
    "memoryTokens",
    "totalTokens",
}
_CONTEXT_REVISION_KEYS = {
    "revision",
    "historyGroups",
    "historyItems",
    "memory",
    "budget",
    "omissions",
    "historyStatus",
    "memoryContextCharacters",
    "compactionSummary",
    "currentRunOmittedThroughItemId",
}
_STEP_INPUT_KEYS = {
    "stepOrdinal",
    "contextRevision",
    "historyItems",
    "memory",
    "budget",
    "omissions",
    "historyStatus",
    "memoryContextCharacters",
    "compactionSummary",
}
