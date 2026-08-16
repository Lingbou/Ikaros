import { createHash } from "node:crypto";

import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  RUNTIME_JOURNAL_EVENT_TYPES,
  RUNTIME_PROTOCOL_MANIFEST,
  RUNTIME_PROTOCOL_VERSION,
  RUNTIME_PROVIDER_TOOL_IDS,
  RUNTIME_RPC_METHODS,
  RUNTIME_SERVER_NAME,
  type RuntimeCancelRunResult,
  type RuntimeHostStatus,
  type RuntimeInitializeResult,
  type RuntimeItemHistory,
  type RuntimeJournalEvent,
  type RuntimeJournalEventType,
  type RuntimeMemoryCreateResult,
  type RuntimeMemoryGetResult,
  type RuntimeMemoryKind,
  type RuntimeMemoryListPage,
  type RuntimeMemoryProvenance,
  type RuntimeMemoryRecord,
  type RuntimeMemoryScope,
  type RuntimeMemoryState,
  type RuntimeMemorySummary,
  type RuntimeModelSummary,
  type RuntimeProviderSummary,
  type RuntimeReplayResult,
  type RuntimeRpcMethod,
  type RuntimeRunHistory,
  type RuntimeSkillDiagnostic,
  type RuntimeSkillListResult,
  type RuntimeSkillSetEnabledResult,
  type RuntimeSkillSummary,
  type RuntimeThreadCreateResult,
  type RuntimeThreadGetResult,
  type RuntimeThreadListPage,
  type RuntimeThreadMutationResult,
  type RuntimeThreadSummary,
  type RuntimeTurnHistory,
  type RuntimeTurnListPage,
  type RuntimeTurnListParams,
  type RuntimeUsageReadResult
} from "../../shared/runtime";

const THREAD_CATALOG_PAGE_LIMIT = 100;
const TURN_HISTORY_PAGE_LIMIT = 100;
const TURN_HISTORY_CURSOR_MAX_LENGTH = 2048;
const MAX_WIRE_IDENTIFIER_LENGTH = 200;
const TURN_AND_RUN_STATUSES = new Set([
  "queued",
  "running",
  "completed",
  "failed",
  "cancelled"
]);
const ITEM_STATUSES = new Set([
  "streaming",
  "running",
  "completed",
  "failed",
  "cancelled"
]);
const ITEM_KINDS = new Set(["message", "tool_call", "tool_result"]);
const INPUT_BUDGET_MEASUREMENT_VERSION = "unicode-codepoints-canonical-json-v1";
const INPUT_BUDGET_MODES = new Set(["bounded"]);
const MAXIMUM_INPUT_CHARACTERS_V1 = 48_000;
const RESERVED_CURRENT_RUN_CHARACTERS_V1 = 12_000;
const OUTPUT_STYLE_CONTENT =
  "Use a restrained, professional response style. Do not use emoji or decorative " +
  "Unicode symbols unless the user explicitly asks for them. Never use them for " +
  "decoration, headings, or list markers. Use Markdown hyphen bullets (`- item`) " +
  "for ordinary unordered lists; the client will render them as simple round bullets.";
const TOOL_RESULT_REQUIRED_KEYS = [
  "toolCallId",
  "toolName",
  "ok",
  "output",
  "cancelled"
] as const;
const TOOL_RESULT_DETAIL_KEYS_BY_TOOL = {
  process_run: [
    "stdout",
    "stderr",
    "cwd",
    "exitCode",
    "durationMs",
    "timedOut",
    "truncated",
    "errorCode"
  ],
  read: [
    "durationMs",
    "truncated",
    "errorCode",
    "path",
    "lineStart",
    "lineEnd",
    "bytesRead",
    "bom",
    "lineTruncations",
    "totalLines",
    "nextOffset"
  ],
  write: [
    "durationMs",
    "truncated",
    "errorCode",
    "path",
    "created",
    "bytesWritten",
    "verified",
    "bom",
    "newline"
  ],
  edit: [
    "durationMs",
    "truncated",
    "errorCode",
    "path",
    "bytesWritten",
    "verified",
    "bom",
    "newline",
    "replacements"
  ]
} as const satisfies Record<string, readonly string[]>;
const RUNTIME_JOURNAL_EVENT_TYPE_SET = new Set<string>(RUNTIME_JOURNAL_EVENT_TYPES);
const RUNTIME_PROVIDER_TOOL_ID_SET = new Set<string>(RUNTIME_PROVIDER_TOOL_IDS);
const RUNTIME_RPC_METHOD_SET = new Set<string>(RUNTIME_RPC_METHODS);
const SKILL_NAME_PATTERN = /^[a-z0-9][a-z0-9-]{0,63}$/;
const MEMORY_ID_PATTERN = /^memory_[0-9a-f]{32}$/;
const MEMORY_KINDS = new Set<RuntimeMemoryKind>([
  "fact",
  "preference",
  "relationship",
  "project"
]);
const MEMORY_STATES = new Set<RuntimeMemoryState>(["active", "forgotten"]);
const MEMORY_CONTENT_MAX_CHARACTERS = 2_048;
const MEMORY_PREVIEW_MAX_CHARACTERS = 160;
const MEMORY_LIST_DEFAULT_LIMIT = 50;
const MEMORY_LIST_PAGE_LIMIT = 100;
const MEMORY_CURSOR_MAX_LENGTH = 1_024;

interface JsonRpcResultResponse {
  jsonrpc: "2.0";
  id: number;
  result: unknown;
}

interface JsonRpcErrorResponse {
  jsonrpc: "2.0";
  id: number;
  error: { code: number; message: string };
}

export type JsonRpcResponse = JsonRpcResultResponse | JsonRpcErrorResponse;

export class RuntimeRpcError extends Error {
  readonly kind = "json_rpc" as const;

  constructor(
    readonly code: number,
    message: string
  ) {
    super(message);
    this.name = "RuntimeRpcError";
  }
}

export interface RuntimeNotification {
  jsonrpc: "2.0";
  method: string;
  params: unknown;
}

function hasOwn(value: object, property: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(value, property);
}

export function isRuntimeNotification(value: unknown): value is RuntimeNotification {
  try {
    parseRuntimeEventNotification(value);
    return true;
  } catch {
    return false;
  }
}

export function parseRuntimeEventNotification(value: unknown): RuntimeNotification {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["jsonrpc", "method", "params"]) ||
    value.jsonrpc !== RUNTIME_PROTOCOL_MANIFEST.jsonrpcVersion ||
    value.method !== RUNTIME_PROTOCOL_MANIFEST.notificationMethods[0]
  ) {
    throw new Error("Runtime returned an unsupported notification envelope.");
  }
  return {
    jsonrpc: "2.0",
    method: RUNTIME_PROTOCOL_MANIFEST.notificationMethods[0],
    params: parseRuntimeJournalEvent(value.params)
  };
}

export function responseId(value: unknown): number | undefined {
  if (typeof value !== "object" || value === null) {
    return undefined;
  }
  const id = (value as { id?: unknown }).id;
  return typeof id === "number" && Number.isInteger(id) ? id : undefined;
}

export function parseRuntimeJournalEvent(value: unknown): RuntimeJournalEvent {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, [
      "seq",
      "schemaVersion",
      "type",
      "threadId",
      "branchId",
      "turnId",
      "runId",
      "itemId",
      "timestamp",
      "payload"
    ])
  ) {
    throw new Error("Runtime returned an invalid journal event.");
  }
  if (value.schemaVersion !== RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION) {
    throw new Error(
      `Runtime journal event schema ${String(value.schemaVersion)} is unsupported.`
    );
  }
  if (
    !isSafePositiveInteger(value.seq) ||
    !isRuntimeJournalEventType(value.type) ||
    !isNullableWireIdentifier(value.threadId) ||
    !isNullableWireIdentifier(value.branchId) ||
    !isNullableWireIdentifier(value.turnId) ||
    !isNullableWireIdentifier(value.runId) ||
    !isNullableWireIdentifier(value.itemId) ||
    !isNonEmptyString(value.timestamp) ||
    !isWireObject(value.payload)
  ) {
    throw new Error("Runtime returned an invalid journal event.");
  }
  const event: RuntimeJournalEvent = {
    seq: value.seq,
    schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type: value.type,
    threadId: value.threadId,
    branchId: value.branchId,
    turnId: value.turnId,
    runId: value.runId,
    itemId: value.itemId,
    timestamp: value.timestamp,
    payload: value.payload
  };
  parseRuntimeJournalEventPayload(event);
  return event;
}

function isRuntimeJournalEventType(value: unknown): value is RuntimeJournalEventType {
  return typeof value === "string" && RUNTIME_JOURNAL_EVENT_TYPE_SET.has(value);
}

function invalidJournalEventPayload(type: RuntimeJournalEventType): never {
  throw new Error(`Runtime returned an invalid ${type} journal event payload.`);
}

function hasThreadEventScope(event: RuntimeJournalEvent): boolean {
  return (
    event.threadId !== null &&
    event.branchId !== null &&
    event.turnId === null &&
    event.runId === null &&
    event.itemId === null
  );
}

function hasRunEventScope(event: RuntimeJournalEvent, item: boolean): boolean {
  return (
    event.threadId !== null &&
    event.branchId !== null &&
    event.turnId !== null &&
    event.runId !== null &&
    (item ? event.itemId !== null : event.itemId === null)
  );
}

function hasScopedPayloadKeys(
  event: RuntimeJournalEvent,
  required: readonly string[],
  optional: readonly string[] = []
): boolean {
  const scope: Array<["turnId" | "runId" | "itemId", string | null]> = [
    ["turnId", event.turnId],
    ["runId", event.runId],
    ["itemId", event.itemId]
  ];
  const presentScopeKeys = scope
    .filter((entry): entry is ["turnId" | "runId" | "itemId", string] => entry[1] !== null)
    .map(([key]) => key);
  if (!hasRequiredAndOptionalKeys(event.payload, [...required, ...presentScopeKeys], optional)) {
    return false;
  }
  return scope.every(([key, value]) =>
    value === null ? !hasOwn(event.payload, key) : event.payload[key] === value
  );
}

function parseRuntimeJournalEventPayload(event: RuntimeJournalEvent): void {
  const payload = event.payload;
  if (event.type === "thread.created") {
    if (
      !hasThreadEventScope(event) ||
      !hasScopedPayloadKeys(event, ["thread", "branch"], ["clientRequestId"])
    ) {
      invalidJournalEventPayload(event.type);
    }
    const thread = parseRuntimeThreadSummary(payload.thread, invalidEventMessage(event.type));
    if (
      thread.id !== event.threadId ||
      thread.defaultBranchId !== event.branchId ||
      thread.updatedAt !== event.timestamp ||
      !isWireObject(payload.branch) ||
      !hasExactKeys(payload.branch, ["id", "threadId", "createdAt", "isDefault"]) ||
      payload.branch.id !== event.branchId ||
      payload.branch.threadId !== event.threadId ||
      payload.branch.createdAt !== event.timestamp ||
      payload.branch.isDefault !== true ||
      !isOptionalWireIdentifier(payload.clientRequestId)
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  if (
    event.type === "thread.renamed" ||
    event.type === "thread.archived" ||
    event.type === "thread.unarchived"
  ) {
    if (!hasThreadEventScope(event) || !hasScopedPayloadKeys(event, ["thread"])) {
      invalidJournalEventPayload(event.type);
    }
    const thread = parseRuntimeThreadSummary(payload.thread, invalidEventMessage(event.type));
    const archivedStateMatches =
      event.type === "thread.archived"
        ? thread.archivedAt === event.timestamp
        : event.type !== "thread.unarchived" || thread.archivedAt === null;
    if (
      thread.id !== event.threadId ||
      thread.defaultBranchId !== event.branchId ||
      thread.updatedAt !== event.timestamp ||
      !archivedStateMatches
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  if (event.type === "run.state_changed") {
    if (
      !hasRunEventScope(event, false) ||
      !hasScopedPayloadKeys(event, ["status"]) ||
      (payload.status !== "queued" && payload.status !== "running")
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  if (event.type === "item.delta") {
    if (
      !hasRunEventScope(event, true) ||
      !hasScopedPayloadKeys(event, ["delta"]) ||
      typeof payload.delta !== "string" ||
      payload.delta.length === 0
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  if (event.type === "item.started") {
    if (
      !hasRunEventScope(event, true) ||
      !hasScopedPayloadKeys(event, ["item"]) ||
      !parseRuntimeEventItem(event, payload.item, false)
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  if (event.type === "item.completed") {
    if (!hasRunEventScope(event, true)) {
      invalidJournalEventPayload(event.type);
    }
    if (hasOwn(payload, "turn") || hasOwn(payload, "run")) {
      parseInitialTurnCompletedEvent(event);
      return;
    }
    if (
      !hasScopedPayloadKeys(event, ["item"]) ||
      !parseRuntimeEventItem(event, payload.item, true)
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  if (event.type === "model.input_prepared") {
    if (
      !hasRunEventScope(event, false) ||
      !hasScopedPayloadKeys(event, [
        "stepOrdinal",
        "preparedAt",
        "contextSnapshot",
        "stepManifest"
      ]) ||
      !isSafePositiveInteger(payload.stepOrdinal) ||
      payload.preparedAt !== event.timestamp ||
      !isContextSnapshot(payload.contextSnapshot, event.runId) ||
      !isStepManifest(
        payload.stepManifest,
        payload.stepOrdinal,
        payload.contextSnapshot,
        event.runId
      )
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  if (event.type === "model.response_finished") {
    const usageValid =
      payload.usage === null
        ? payload.activityDate === null
        : payload.outcome === "completed" &&
          isCanonicalCalendarDate(payload.activityDate) &&
          isModelUsage(payload.usage);
    const reasonValid =
      payload.outcome === "completed"
        ? payload.reasonCode === null
        : isWireIdentifier(payload.reasonCode);
    if (
      !hasRunEventScope(event, false) ||
      !hasScopedPayloadKeys(event, [
        "stepOrdinal",
        "providerId",
        "modelId",
        "outcome",
        "reasonCode",
        "responseModelId",
        "requestId",
        "usage",
        "activityDate",
        "finishedAt"
      ]) ||
      !isSafePositiveInteger(payload.stepOrdinal) ||
      !isNonEmptyString(payload.providerId) ||
      !isNonEmptyString(payload.modelId) ||
      (payload.outcome !== "completed" &&
        payload.outcome !== "failed" &&
        payload.outcome !== "cancelled") ||
      !reasonValid ||
      !isNullableWireIdentifier(payload.responseModelId) ||
      !isNullableWireIdentifier(payload.requestId) ||
      !usageValid ||
      payload.finishedAt !== event.timestamp
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  if (event.type === "run.settled") {
    if (
      !hasRunEventScope(event, false) ||
      !hasScopedPayloadKeys(event, ["status", "settledAt"], ["reasonCode"]) ||
      (payload.status !== "completed" &&
        payload.status !== "failed" &&
        payload.status !== "cancelled") ||
      payload.settledAt !== event.timestamp ||
      !isOptionalWireIdentifier(payload.reasonCode)
    ) {
      invalidJournalEventPayload(event.type);
    }
    return;
  }

  const exhaustive: never = event.type;
  throw new Error(`Runtime returned unsupported journal event ${String(exhaustive)}.`);
}

function invalidEventMessage(type: RuntimeJournalEventType): string {
  return `Runtime returned an invalid ${type} journal event payload.`;
}

function parseRuntimeEventItem(
  event: RuntimeJournalEvent,
  value: unknown,
  completed: boolean
): boolean {
  if (event.turnId === null || event.runId === null || event.itemId === null) return false;
  let item: RuntimeItemHistory;
  try {
    item = parseRuntimeItemHistory(value, event.turnId, event.runId);
  } catch {
    return false;
  }
  if (item.id !== event.itemId || item.updatedAt !== event.timestamp) return false;
  if (completed) {
    if (
      item.status !== "completed" &&
      item.status !== "failed" &&
      item.status !== "cancelled"
    ) {
      return false;
    }
  } else if (
    !(
      (item.kind === "message" && item.role === "assistant" && item.status === "streaming") ||
      (item.kind === "tool_call" && item.role === "assistant" && item.status === "running")
    )
  ) {
    return false;
  }
  return validRuntimeItemData(item, completed);
}

function validRuntimeItemData(item: RuntimeItemHistory, completed: boolean): boolean {
  const data = item.data;
  if (item.kind === "message") {
    if (item.role === "user") return completed && hasExactKeys(data, []);
    return hasRequiredAndOptionalKeys(data, [], ["stepId"]) &&
      isOptionalWireIdentifier(data.stepId);
  }
  if (item.kind === "tool_call") {
    if (
      !hasRequiredAndOptionalKeys(
        data,
        ["stepId", "callId", "toolName", "arguments"],
        ["reasoningContent", "outcome", "durationMs"]
      ) ||
      !isWireIdentifier(data.stepId) ||
      !isWireIdentifier(data.callId) ||
      !isProviderToolId(data.toolName) ||
      !isWireObject(data.arguments) ||
      (data.reasoningContent !== undefined && typeof data.reasoningContent !== "string") ||
      (data.durationMs !== undefined && !isSafeNonNegativeInteger(data.durationMs))
    ) {
      return false;
    }
    return completed
      ? data.outcome === item.status
      : data.outcome === undefined && data.durationMs === undefined;
  }
  return (
    completed &&
    hasExactKeys(data, ["stepId", "callId", "toolCallItemId", "toolName", "result"]) &&
    isWireIdentifier(data.stepId) &&
    isWireIdentifier(data.callId) &&
    isWireIdentifier(data.toolCallItemId) &&
    isProviderToolId(data.toolName) &&
    isCanonicalToolResult(data.result, data.callId, data.toolName, item.status) &&
    isCanonicalToolResultContent(item.content, data.result)
  );
}

function isCanonicalToolResult(
  value: unknown,
  callId: string,
  toolName: string,
  itemStatus: RuntimeItemHistory["status"]
): value is Record<string, unknown> {
  const allowedDetails = isProviderToolId(toolName)
    ? TOOL_RESULT_DETAIL_KEYS_BY_TOOL[toolName]
    : undefined;
  if (
    !isWireObject(value) ||
    allowedDetails === undefined ||
    !hasRequiredAndOptionalKeys(
      value,
      [...TOOL_RESULT_REQUIRED_KEYS],
      [...allowedDetails]
    ) ||
    value.toolCallId !== callId ||
    value.toolName !== toolName ||
    !isWireIdentifier(value.toolCallId) ||
    !isProviderToolId(value.toolName) ||
    typeof value.ok !== "boolean" ||
    typeof value.output !== "string" ||
    typeof value.cancelled !== "boolean" ||
    value.ok !== (itemStatus === "completed") ||
    value.cancelled !== (itemStatus === "cancelled")
  ) {
    return false;
  }
  return Object.entries(value).every(([key, detail]) =>
    TOOL_RESULT_REQUIRED_KEYS.includes(key as (typeof TOOL_RESULT_REQUIRED_KEYS)[number])
      ? true
      : isCanonicalToolResultDetail(key, detail)
  );
}

function isCanonicalToolResultDetail(key: string, value: unknown): boolean {
  if (key === "stdout" || key === "stderr" || key === "cwd") {
    return typeof value === "string";
  }
  if (key === "exitCode") return value === null || Number.isSafeInteger(value);
  if (
    key === "durationMs" ||
    key === "lineEnd" ||
    key === "bytesRead" ||
    key === "lineTruncations" ||
    key === "totalLines" ||
    key === "bytesWritten" ||
    key === "replacements"
  ) {
    return isSafeNonNegativeInteger(value);
  }
  if (key === "lineStart" || key === "nextOffset") return isSafePositiveInteger(value);
  if (
    key === "timedOut" ||
    key === "truncated" ||
    key === "bom" ||
    key === "created" ||
    key === "verified"
  ) {
    return typeof value === "boolean";
  }
  if (key === "errorCode") return isWireIdentifier(value);
  if (key === "path") return value === null || typeof value === "string";
  if (key === "newline") return value === null || value === "lf" || value === "crlf";
  return false;
}

function isCanonicalToolResultContent(content: string, result: Record<string, unknown>): boolean {
  let parsed: unknown;
  try {
    parsed = JSON.parse(content) as unknown;
  } catch {
    return false;
  }
  return sameWireValue(parsed, result);
}

function parseInitialTurnCompletedEvent(event: RuntimeJournalEvent): void {
  const payload = event.payload;
  if (
    !hasScopedPayloadKeys(
      event,
      ["turn", "run", "item", "submissionFrame", "runManifest"],
      ["clientRequestId"]
    ) ||
    !isWireObject(payload.turn) ||
    !hasExactKeys(payload.turn, [
      "id",
      "threadId",
      "branchId",
      "ordinal",
      "status",
      "createdAt",
      "updatedAt"
    ]) ||
    payload.turn.id !== event.turnId ||
    payload.turn.threadId !== event.threadId ||
    payload.turn.branchId !== event.branchId ||
    !isSafePositiveInteger(payload.turn.ordinal) ||
    payload.turn.status !== "queued" ||
    payload.turn.createdAt !== event.timestamp ||
    payload.turn.updatedAt !== event.timestamp ||
    !isWireObject(payload.run) ||
    !hasRequiredAndOptionalKeys(
      payload.run,
      [
        "id",
        "turnId",
        "providerId",
        "modelId",
        "executionPolicy",
        "status",
        "createdAt",
        "settledAt",
        "skills"
      ],
      ["clientRequestId"]
    ) ||
    payload.run.id !== event.runId ||
    payload.run.turnId !== event.turnId ||
    !isNonEmptyString(payload.run.providerId) ||
    !isNonEmptyString(payload.run.modelId) ||
    payload.run.executionPolicy !== "full_access" ||
    payload.run.status !== "queued" ||
    payload.run.createdAt !== event.timestamp ||
    payload.run.settledAt !== null ||
    !isRuntimeSkillSnapshot(payload.run.skills) ||
    !isOptionalWireIdentifier(payload.clientRequestId) ||
    !isOptionalWireIdentifier(payload.run.clientRequestId) ||
    payload.clientRequestId !== payload.run.clientRequestId ||
    !parseRuntimeEventItem(event, payload.item, true) ||
    !isSubmissionFrame(payload.submissionFrame, event, payload.run) ||
    !isRunManifest(payload.runManifest, payload.submissionFrame)
  ) {
    invalidJournalEventPayload(event.type);
  }
  const item = payload.item as RuntimeItemHistory;
  if (
    item.id !== event.itemId ||
    item.ordinal !== 1 ||
    item.kind !== "message" ||
    item.role !== "user" ||
    item.status !== "completed"
  ) {
    invalidJournalEventPayload(event.type);
  }
}

function isSubmissionFrame(
  value: unknown,
  event: RuntimeJournalEvent,
  runValue: unknown
): boolean {
  if (
    !isWireObject(value) ||
    !isWireObject(runValue) ||
    !hasExactKeys(value, [
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
      "maxSteps"
    ]) ||
    value.schemaVersion !== 1 ||
    value.userItemId !== event.itemId ||
    value.threadId !== event.threadId ||
    value.branchId !== event.branchId ||
    value.turnId !== event.turnId ||
    value.runId !== event.runId ||
    !isNonEmptyString(value.providerId) ||
    value.providerId !== runValue.providerId ||
    !isNonEmptyString(value.modelId) ||
    value.modelId !== runValue.modelId ||
    !isSha256(value.publicProviderConfigFingerprint) ||
    value.executionPolicy !== "full_access" ||
    value.executionPolicy !== runValue.executionPolicy ||
    !isSafePositiveInteger(value.maxSteps) ||
    !isRuntimeSkillSnapshot(value.skills) ||
    !sameWireValue(value.skills, runValue.skills) ||
    !Array.isArray(value.tools) ||
    !isWireObject(value.instructions) ||
    !hasExactKeys(value.instructions, ["outputStyle", "identityCore", "skillCatalog"]) ||
    !isInstructionBlock(value.instructions.outputStyle, {
      id: "output-style",
      source: "ikaros-runtime:output-style-v1",
      authority: "runtime_instruction",
      scope: "global",
      lifetime: "release",
      content: OUTPUT_STYLE_CONTENT
    }) ||
    !isIdentityCoreBlock(value.instructions.identityCore) ||
    !isWireObject(value.contextData) ||
    !hasExactKeys(value.contextData, ["memory"]) ||
    !Array.isArray(value.contextData.memory) ||
    value.contextData.memory.length !== 0 ||
    !isWorkspaceSnapshot(value.workspace)
  ) {
    return false;
  }
  const toolNames = new Set<string>();
  for (const tool of value.tools) {
    if (
      !isWireObject(tool) ||
      !hasExactKeys(tool, ["name", "description", "inputSchema", "definitionSha256"]) ||
      !isProviderToolId(tool.name) ||
      toolNames.has(tool.name) ||
      typeof tool.description !== "string" ||
      !isWireObject(tool.inputSchema) ||
      !isSha256(tool.definitionSha256) ||
      tool.definitionSha256 !==
        canonicalSha256({
          name: tool.name,
          description: tool.description,
          inputSchema: tool.inputSchema
        })
    ) {
      return false;
    }
    toolNames.add(tool.name);
  }
  const hasSkills = value.skills.length > 0;
  const expectedSkillCatalog = buildSkillCatalogContent(value.skills);
  return (
    hasSkills === (value.instructions.skillCatalog !== null) &&
    (value.instructions.skillCatalog === null ||
      isInstructionBlock(value.instructions.skillCatalog, {
        id: "skill-catalog",
        source: "run:skill-descriptors",
        authority: "runtime_instruction",
        scope: "run",
        lifetime: "run",
        content: expectedSkillCatalog
      }))
  );
}

function isIdentityCoreBlock(value: unknown): boolean {
  if (value === null) return true;
  return (
    isWireObject(value) &&
    hasExactKeys(value, [
      "id",
      "version",
      "source",
      "authority",
      "scope",
      "lifetime",
      "content"
    ]) &&
    value.id === "ikaros-identity" &&
    value.version === 1 &&
    value.source === "ikaros-runtime:identity" &&
    value.authority === "runtime_identity" &&
    value.scope === "global" &&
    value.lifetime === "release" &&
    typeof value.content === "string" &&
    value.content.trim().length > 0 &&
    [...value.content].length <= 2048
  );
}

function isRunManifest(value: unknown, frameValue: unknown): boolean {
  if (
    !isWireObject(value) ||
    !isWireObject(frameValue) ||
    !hasExactKeys(value, [
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
      "maxSteps"
    ]) ||
    value.schemaVersion !== 1 ||
    value.runId !== frameValue.runId ||
    value.modelInputPlanVersion !== 1 ||
    value.submissionFrameVersion !== 1 ||
    value.contextSelectionVersion !== "bounded-history-v1" ||
    value.memoryContextVersion !== 1 ||
    value.executionPolicy !== frameValue.executionPolicy ||
    value.maxSteps !== frameValue.maxSteps ||
    !Array.isArray(value.instructions) ||
    !Array.isArray(value.skills) ||
    !Array.isArray(value.tools) ||
    !isWireObject(value.provider) ||
    !hasExactKeys(value.provider, [
      "providerId",
      "modelId",
      "publicProviderConfigFingerprint"
    ]) ||
    value.provider.providerId !== frameValue.providerId ||
    value.provider.modelId !== frameValue.modelId ||
    value.provider.publicProviderConfigFingerprint !==
      frameValue.publicProviderConfigFingerprint
  ) {
    return false;
  }

  if (
    !isRuntimeSkillSnapshot(frameValue.skills) ||
    !Array.isArray(frameValue.tools) ||
    !isWireObject(frameValue.instructions)
  ) {
    return false;
  }
  const instructionBlocks = [
    frameValue.instructions.identityCore,
    frameValue.instructions.outputStyle,
    frameValue.instructions.skillCatalog
  ].filter((block) => block !== null);
  if (!instructionBlocks.every((block) => isWireObject(block))) {
    return false;
  }
  const expectedInstructions = instructionBlocks.map((block) => {
    const instruction = block as Record<string, unknown>;
    if (typeof instruction.content !== "string") return null;
    return {
      id: instruction.id,
      version: instruction.version,
      source: instruction.source,
      authority: instruction.authority,
      scope: instruction.scope,
      lifetime: instruction.lifetime,
      characters: [...instruction.content].length,
      contentSha256: canonicalSha256(instruction.content)
    };
  });
  if (expectedInstructions.some((instruction) => instruction === null)) {
    return false;
  }

  const expectedSkills = frameValue.skills.map((skill) => ({
    name: skill.name,
    descriptorSha256: canonicalSha256(skill)
  }));
  const expectedTools = frameValue.tools.map((tool) => {
    if (!isWireObject(tool)) return null;
    return { name: tool.name, definitionSha256: tool.definitionSha256 };
  });
  if (expectedTools.some((tool) => tool === null)) {
    return false;
  }

  return (
    sameWireValue(value.instructions, expectedInstructions) &&
    sameWireValue(value.skills, expectedSkills) &&
    sameWireValue(value.tools, expectedTools)
  );
}

function isInstructionBlock(
  value: unknown,
  expected: {
    id: string;
    source: string;
    authority: "runtime_identity" | "runtime_instruction";
    scope: string;
    lifetime: "release" | "run";
    content: string;
  }
): boolean {
  return (
    isWireObject(value) &&
    hasExactKeys(value, [
      "id",
      "version",
      "source",
      "authority",
      "scope",
      "lifetime",
      "content"
    ]) &&
    value.id === expected.id &&
    value.version === 1 &&
    value.source === expected.source &&
    value.authority === expected.authority &&
    value.scope === expected.scope &&
    value.lifetime === expected.lifetime &&
    value.content === expected.content
  );
}

function buildSkillCatalogContent(
  skills: Array<{ name: string; description: string; location: string }>
): string {
  if (skills.length === 0) return "";
  const rows = [
    "Skills provide optional instructions for specialized tasks. When a Skill is relevant, " +
      "use the read tool to load its SKILL.md from the listed location before following it.",
    "<available_skills>"
  ];
  for (const skill of skills) {
    rows.push(
      "  <skill>",
      `    <name>${escapeXmlText(skill.name)}</name>`,
      `    <description>${escapeXmlText(skill.description)}</description>`,
      `    <location>${escapeXmlText(skill.location)}</location>`,
      "  </skill>"
    );
  }
  rows.push("</available_skills>");
  return rows.join("\n");
}

function escapeXmlText(value: string): string {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

function isWorkspaceSnapshot(value: unknown): boolean {
  return (
    value === null ||
    (isWireObject(value) &&
      hasExactKeys(value, ["id", "name", "rootUri"]) &&
      isWireIdentifier(value.id) &&
      isNonEmptyString(value.name) &&
      (value.rootUri === null || isNonEmptyString(value.rootUri)))
  );
}

function isHistoryItemReference(value: unknown): boolean {
  const roleMatchesKind =
    isWireObject(value) &&
    ((value.kind === "message" &&
      (value.role === "user" || value.role === "assistant")) ||
      (value.kind === "tool_call" && value.role === "assistant") ||
      (value.kind === "tool_result" && value.role === "tool"));
  return (
    isWireObject(value) &&
    hasExactKeys(value, ["itemId", "turnId", "runId", "kind", "role", "characters"]) &&
    isWireIdentifier(value.itemId) &&
    isWireIdentifier(value.turnId) &&
    isWireIdentifier(value.runId) &&
    ITEM_KINDS.has(value.kind as string) &&
    roleMatchesKind &&
    isSafeNonNegativeInteger(value.characters)
  );
}

function isInputBudget(
  value: unknown
): value is Record<string, unknown> & {
  mode: string;
  measurementVersion: string;
  maximumCharacters: number;
  reservedCurrentRunCharacters: number;
  instructionCharacters: number;
  contextDataCharacters: number;
  toolCharacters: number;
  historyCharacters: number;
  currentRunCharacters: number;
  memoryCharacters: number;
  totalCharacters: number;
} {
  return (
    isWireObject(value) &&
    hasExactKeys(value, [
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
      "totalCharacters"
    ]) &&
    INPUT_BUDGET_MODES.has(value.mode as string) &&
    value.measurementVersion === INPUT_BUDGET_MEASUREMENT_VERSION &&
    isSafePositiveInteger(value.maximumCharacters) &&
    isSafePositiveInteger(value.reservedCurrentRunCharacters) &&
    value.reservedCurrentRunCharacters < value.maximumCharacters &&
    isSafeNonNegativeInteger(value.instructionCharacters) &&
    isSafeNonNegativeInteger(value.contextDataCharacters) &&
    isSafeNonNegativeInteger(value.toolCharacters) &&
    isSafeNonNegativeInteger(value.historyCharacters) &&
    isSafeNonNegativeInteger(value.currentRunCharacters) &&
    isSafeNonNegativeInteger(value.memoryCharacters) &&
    isSafeNonNegativeInteger(value.totalCharacters) &&
    value.totalCharacters ===
      value.instructionCharacters +
        value.contextDataCharacters +
        value.toolCharacters +
        value.historyCharacters +
        value.currentRunCharacters +
        value.memoryCharacters &&
    value.totalCharacters <= value.maximumCharacters
  );
}

function isContextSnapshot(value: unknown, currentRunId: unknown): boolean {
  if (
    !isWireObject(value) ||
    !isWireIdentifier(currentRunId) ||
    !hasExactKeys(value, [
      "schemaVersion",
      "selectionVersion",
      "historyGroups",
      "historyItems",
      "memory",
      "budget",
      "omissions"
    ]) ||
    value.schemaVersion !== 1 ||
    value.selectionVersion !== "bounded-history-v1" ||
    !Array.isArray(value.historyGroups) ||
    value.historyGroups.length === 0 ||
    !Array.isArray(value.historyItems) ||
    value.historyItems.length === 0 ||
    !value.historyItems.every(isHistoryItemReference) ||
    !hasUniqueHistoryItemIds(value.historyItems) ||
    !Array.isArray(value.memory) ||
    value.memory.length !== 0 ||
    !isInputBudget(value.budget) ||
    value.budget.maximumCharacters !== MAXIMUM_INPUT_CHARACTERS_V1 ||
    value.budget.reservedCurrentRunCharacters !== RESERVED_CURRENT_RUN_CHARACTERS_V1 ||
    !isHistoryBudgetForItems(value.budget, value.historyItems, currentRunId) ||
    value.budget.memoryCharacters !== 0 ||
    value.budget.contextDataCharacters !== 0 ||
    !Array.isArray(value.omissions) ||
    value.omissions.length > 1 ||
    !value.omissions.every(isHistoryOmission) ||
    value.budget.totalCharacters + value.budget.reservedCurrentRunCharacters >
      value.budget.maximumCharacters
  ) {
    return false;
  }
  if (!value.historyGroups.every(
    (group) =>
      isWireObject(group) &&
      hasExactKeys(group, ["turnId", "itemIds"]) &&
      isWireIdentifier(group.turnId) &&
      Array.isArray(group.itemIds) &&
      group.itemIds.length > 0 &&
      group.itemIds.every(isWireIdentifier) &&
      new Set(group.itemIds).size === group.itemIds.length
  )) {
    return false;
  }
  const groupTurnIds = value.historyGroups.map((group) =>
    isWireObject(group) ? group.turnId : undefined
  );
  if (
    !groupTurnIds.every(isWireIdentifier) ||
    new Set(groupTurnIds).size !== groupTurnIds.length
  ) {
    return false;
  }
  const historyItems = value.historyItems;
  const groupedItems = value.historyGroups.flatMap((group) => {
    const typedGroup = group as { turnId: string; itemIds: string[] };
    return typedGroup.itemIds.map((itemId) => ({ itemId, turnId: typedGroup.turnId }));
  });
  const selectedTurnIds = new Set(groupTurnIds as string[]);
  if (
    value.omissions.some(
      (omission) =>
        isWireObject(omission) && selectedTurnIds.has(omission.sourceId as string)
    )
  ) {
    return false;
  }
  return (
    groupedItems.length === historyItems.length &&
    groupedItems.every((grouped, index) => {
      const item = historyItems[index];
      return (
        isWireObject(item) &&
        grouped.itemId === item.itemId &&
        grouped.turnId === item.turnId
      );
    })
  );
}

function isStepManifest(
  value: unknown,
  stepOrdinal: unknown,
  contextSnapshot: unknown,
  currentRunId: unknown
): boolean {
  if (
    !isWireObject(value) ||
    !isWireObject(contextSnapshot) ||
    !isWireIdentifier(currentRunId) ||
    !hasExactKeys(value, [
      "schemaVersion",
      "stepOrdinal",
      "contextSnapshotVersion",
      "historyItems",
      "memory",
      "budget",
      "omissions"
    ]) ||
    value.schemaVersion !== 1 ||
    value.stepOrdinal !== stepOrdinal ||
    value.contextSnapshotVersion !== contextSnapshot.schemaVersion ||
    !Array.isArray(value.historyItems) ||
    value.historyItems.length === 0 ||
    !value.historyItems.every(isHistoryItemReference) ||
    !hasUniqueHistoryItemIds(value.historyItems) ||
    !Array.isArray(contextSnapshot.historyItems) ||
    !isHistoryPrefix(contextSnapshot.historyItems, value.historyItems, currentRunId) ||
    !Array.isArray(value.memory) ||
    value.memory.length !== 0 ||
    !sameWireValue(value.memory, contextSnapshot.memory) ||
    !isInputBudget(value.budget) ||
    !isHistoryBudgetForItems(value.budget, value.historyItems, currentRunId) ||
    value.budget.memoryCharacters !== 0 ||
    value.budget.contextDataCharacters !== 0 ||
    !isWireObject(contextSnapshot.budget) ||
    value.budget.mode !== contextSnapshot.budget.mode ||
    value.budget.measurementVersion !== contextSnapshot.budget.measurementVersion ||
    value.budget.maximumCharacters !== contextSnapshot.budget.maximumCharacters ||
    value.budget.reservedCurrentRunCharacters !==
      contextSnapshot.budget.reservedCurrentRunCharacters ||
    value.budget.instructionCharacters !== contextSnapshot.budget.instructionCharacters ||
    value.budget.contextDataCharacters !== contextSnapshot.budget.contextDataCharacters ||
    value.budget.toolCharacters !== contextSnapshot.budget.toolCharacters ||
    value.budget.historyCharacters !== contextSnapshot.budget.historyCharacters ||
    value.budget.memoryCharacters !== contextSnapshot.budget.memoryCharacters ||
    !Array.isArray(value.omissions) ||
    !value.omissions.every(isHistoryOmission) ||
    !sameWireValue(value.omissions, contextSnapshot.omissions)
  ) {
    return false;
  }
  const snapshotBudget = contextSnapshot.budget;
  return (
    isInputBudget(snapshotBudget) &&
    value.budget.currentRunCharacters >= snapshotBudget.currentRunCharacters
  );
}

function isHistoryOmission(value: unknown): boolean {
  return (
    isWireObject(value) &&
    hasExactKeys(value, ["sourceType", "sourceId", "reason"]) &&
    value.sourceType === "history" &&
    isWireIdentifier(value.sourceId) &&
    value.reason === "omitted_by_budget"
  );
}

function hasUniqueHistoryItemIds(items: unknown[]): boolean {
  const ids = items.map((item) => (isWireObject(item) ? item.itemId : undefined));
  return ids.every(isWireIdentifier) && new Set(ids).size === ids.length;
}

function isHistoryPrefix(
  frozenItems: unknown[],
  stepItems: unknown[],
  currentRunId: string
): boolean {
  if (frozenItems.length > stepItems.length) return false;
  if (!frozenItems.every((item, index) => sameWireValue(item, stepItems[index]))) {
    return false;
  }
  return stepItems.slice(frozenItems.length).every(
    (item) => isWireObject(item) && item.runId === currentRunId
  );
}

function isHistoryBudgetForItems(
  budget: Record<string, unknown>,
  items: unknown[],
  currentRunId: string
): boolean {
  let historyCharacters = 0;
  let currentRunCharacters = 0;
  let sawCurrentRun = false;
  for (const item of items) {
    if (!isWireObject(item) || !isSafeNonNegativeInteger(item.characters)) return false;
    if (item.runId === currentRunId) {
      sawCurrentRun = true;
      currentRunCharacters += item.characters;
    } else {
      if (sawCurrentRun) return false;
      historyCharacters += item.characters;
    }
  }
  return (
    sawCurrentRun &&
    budget.historyCharacters === historyCharacters &&
    budget.currentRunCharacters === currentRunCharacters
  );
}

function isModelUsage(value: unknown): boolean {
  return (
    isWireObject(value) &&
    hasExactKeys(value, [
      "inputTokens",
      "cachedInputTokens",
      "outputTokens",
      "reasoningOutputTokens",
      "totalTokens"
    ]) &&
    isSafeNonNegativeInteger(value.inputTokens) &&
    isNullableSafeNonNegativeInteger(value.cachedInputTokens) &&
    isSafeNonNegativeInteger(value.outputTokens) &&
    isNullableSafeNonNegativeInteger(value.reasoningOutputTokens) &&
    isSafeNonNegativeInteger(value.totalTokens) &&
    (value.cachedInputTokens === null || value.cachedInputTokens <= value.inputTokens) &&
    (value.reasoningOutputTokens === null ||
      value.reasoningOutputTokens <= value.outputTokens)
  );
}

function isSha256(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

function isRuntimeSkillSnapshot(
  value: unknown
): value is Array<{ name: string; description: string; location: string }> {
  if (!Array.isArray(value)) {
    return false;
  }
  let previousName: string | null = null;
  for (const candidate of value) {
    if (
      !isWireObject(candidate) ||
      !hasExactKeys(candidate, ["name", "description", "location"]) ||
      !isNonEmptyString(candidate.name) ||
      !isNonEmptyString(candidate.description) ||
      !isNonEmptyString(candidate.location) ||
      (previousName !== null && candidate.name <= previousName)
    ) {
      return false;
    }
    previousName = candidate.name;
  }
  return true;
}

function isProviderToolId(
  value: unknown
): value is keyof typeof TOOL_RESULT_DETAIL_KEYS_BY_TOOL {
  return (
    typeof value === "string" &&
    RUNTIME_PROVIDER_TOOL_ID_SET.has(value) &&
    hasOwn(TOOL_RESULT_DETAIL_KEYS_BY_TOOL, value)
  );
}

export function parseRuntimeReplayResult(value: unknown): RuntimeReplayResult {
  if (typeof value !== "object" || value === null) {
    throw new Error("Runtime returned an invalid event replay result.");
  }
  const replay = value as Partial<RuntimeReplayResult>;
  if (
    !Array.isArray(replay.events) ||
    !Number.isInteger(replay.latestSeq) ||
    !Number.isInteger(replay.nextAfterSeq) ||
    typeof replay.hasMore !== "boolean"
  ) {
    throw new Error("Runtime returned an invalid event replay result.");
  }
  return {
    events: replay.events.map(parseRuntimeJournalEvent),
    latestSeq: replay.latestSeq as number,
    nextAfterSeq: replay.nextAfterSeq as number,
    hasMore: replay.hasMore
  };
}

function isWireObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function isWireIdentifier(value: unknown): value is string {
  return isNonEmptyString(value) && value.length <= MAX_WIRE_IDENTIFIER_LENGTH;
}

function isNullableWireIdentifier(value: unknown): value is string | null {
  return value === null || isWireIdentifier(value);
}

function isOptionalWireIdentifier(value: unknown): value is string | undefined {
  return value === undefined || isWireIdentifier(value);
}

function isSafeNonNegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function isSafePositiveInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) > 0;
}

function hasExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actualKeys = Object.keys(value);
  return actualKeys.length === keys.length && keys.every((key) => hasOwn(value, key));
}

function hasRequiredAndOptionalKeys(
  value: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[]
): boolean {
  const allowed = new Set([...required, ...optional]);
  return (
    required.every((key) => hasOwn(value, key)) &&
    Object.keys(value).every((key) => allowed.has(key))
  );
}

function sameWireValue(left: unknown, right: unknown): boolean {
  if (Array.isArray(left) || Array.isArray(right)) {
    return (
      Array.isArray(left) &&
      Array.isArray(right) &&
      left.length === right.length &&
      left.every((value, index) => sameWireValue(value, right[index]))
    );
  }
  if (isWireObject(left) || isWireObject(right)) {
    if (!isWireObject(left) || !isWireObject(right)) return false;
    const keys = Object.keys(right);
    return hasExactKeys(left, keys) && keys.every((key) => sameWireValue(left[key], right[key]));
  }
  return left === right;
}

function canonicalJson(value: unknown): string {
  if (value === null || typeof value === "boolean" || typeof value === "number") {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) throw new Error("Runtime returned a non-JSON value.");
    return encoded;
  }
  if (typeof value === "string") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(",")}]`;
  }
  if (isWireObject(value)) {
    return `{${Object.keys(value)
      .sort(compareUnicodeCodePoints)
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  throw new Error("Runtime returned a non-JSON value.");
}

function compareUnicodeCodePoints(left: string, right: string): number {
  const leftPoints = Array.from(left, (character) => character.codePointAt(0) as number);
  const rightPoints = Array.from(right, (character) => character.codePointAt(0) as number);
  const length = Math.min(leftPoints.length, rightPoints.length);
  for (let index = 0; index < length; index += 1) {
    if (leftPoints[index] !== rightPoints[index]) {
      return leftPoints[index] - rightPoints[index];
    }
  }
  return leftPoints.length - rightPoints.length;
}

function canonicalSha256(value: unknown): string {
  return createHash("sha256").update(canonicalJson(value), "utf8").digest("hex");
}

export function parseRuntimeInitializeResult(value: unknown): RuntimeInitializeResult {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["protocolVersion", "server", "capabilities"]) ||
    value.protocolVersion !== RUNTIME_PROTOCOL_VERSION ||
    !isWireObject(value.server) ||
    !hasExactKeys(value.server, ["name", "version"]) ||
    value.server.name !== RUNTIME_SERVER_NAME ||
    !isNonEmptyString(value.server.version) ||
    !sameWireValue(value.capabilities, RUNTIME_PROTOCOL_MANIFEST.capabilities)
  ) {
    throw new Error("Runtime initialization result did not match the protocol contract.");
  }
  return value as unknown as RuntimeInitializeResult;
}

function isNullableSafeNonNegativeInteger(value: unknown): value is number | null {
  return value === null || isSafeNonNegativeInteger(value);
}

function isCanonicalCalendarDate(value: unknown): value is string {
  if (typeof value !== "string") return false;
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (year < 1) return false;
  const date = new Date(0);
  date.setUTCHours(0, 0, 0, 0);
  date.setUTCFullYear(year, month - 1, day);
  return (
    date.getUTCFullYear() === year &&
    date.getUTCMonth() === month - 1 &&
    date.getUTCDate() === day
  );
}

function isCanonicalTimestamp(value: unknown): value is string {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(value)) {
    return false;
  }
  const timestamp = new Date(value);
  return !Number.isNaN(timestamp.getTime()) && timestamp.toISOString() === value;
}

export function parseRuntimeUsageReadResult(value: unknown): RuntimeUsageReadResult {
  const invalidMessage = "Runtime returned an invalid usage result.";
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["summary", "dailyUsageBuckets"]) ||
    !isWireObject(value.summary) ||
    !hasExactKeys(value.summary, [
      "lifetimeTokens",
      "peakDailyTokens",
      "longestRunningTurnSec",
      "currentStreakDays",
      "longestStreakDays"
    ]) ||
    !isNullableSafeNonNegativeInteger(value.summary.lifetimeTokens) ||
    !isNullableSafeNonNegativeInteger(value.summary.peakDailyTokens) ||
    !isNullableSafeNonNegativeInteger(value.summary.longestRunningTurnSec) ||
    !isSafeNonNegativeInteger(value.summary.currentStreakDays) ||
    !isSafeNonNegativeInteger(value.summary.longestStreakDays) ||
    !Array.isArray(value.dailyUsageBuckets)
  ) {
    throw new Error(invalidMessage);
  }

  const dailyUsageBuckets = value.dailyUsageBuckets.map((bucket) => {
    if (
      !isWireObject(bucket) ||
      !hasExactKeys(bucket, ["startDate", "tokens"]) ||
      !isCanonicalCalendarDate(bucket.startDate) ||
      !isSafeNonNegativeInteger(bucket.tokens)
    ) {
      throw new Error(invalidMessage);
    }
    return { startDate: bucket.startDate, tokens: bucket.tokens };
  });

  return {
    summary: {
      lifetimeTokens: value.summary.lifetimeTokens,
      peakDailyTokens: value.summary.peakDailyTokens,
      longestRunningTurnSec: value.summary.longestRunningTurnSec,
      currentStreakDays: value.summary.currentStreakDays,
      longestStreakDays: value.summary.longestStreakDays
    },
    dailyUsageBuckets
  };
}

function parseRuntimeThreadSummary(
  value: unknown,
  invalidMessage = "Runtime returned an invalid thread catalog page."
): RuntimeThreadSummary {
  if (!isWireObject(value)) {
    throw new Error(invalidMessage);
  }
  const thread = value as Partial<RuntimeThreadSummary>;
  const workspace = thread.workspace;
  const validWorkspace =
    workspace === null ||
    (isWireObject(workspace) &&
      isWireIdentifier(workspace.id) &&
      isNonEmptyString(workspace.name) &&
      (workspace.rootUri === null || isNonEmptyString(workspace.rootUri)));
  if (
    !isWireIdentifier(thread.id) ||
    (thread.title !== null && typeof thread.title !== "string") ||
    !isWireIdentifier(thread.defaultBranchId) ||
    !validWorkspace ||
    !isNonEmptyString(thread.createdAt) ||
    !isNonEmptyString(thread.updatedAt) ||
    (thread.archivedAt !== null && !isNonEmptyString(thread.archivedAt))
  ) {
    throw new Error(invalidMessage);
  }
  return {
    id: thread.id,
    title: thread.title as string | null,
    defaultBranchId: thread.defaultBranchId,
    workspace: workspace as RuntimeThreadSummary["workspace"],
    createdAt: thread.createdAt,
    updatedAt: thread.updatedAt,
    archivedAt: thread.archivedAt as string | null
  };
}

export function parseRuntimeThreadMutationResult(
  value: unknown,
  expectedThreadId?: string,
  expectedEventType?: "thread.renamed" | "thread.archived" | "thread.unarchived"
): RuntimeThreadMutationResult {
  const invalidMessage = "Runtime returned an invalid Thread mutation result.";
  if (!isWireObject(value) || typeof value.changed !== "boolean") {
    throw new Error(invalidMessage);
  }
  const thread = parseRuntimeThreadSummary(value.thread, invalidMessage);
  if (expectedThreadId !== undefined && thread.id !== expectedThreadId) {
    throw new Error(invalidMessage);
  }
  if (
    (expectedEventType === "thread.archived" && thread.archivedAt === null) ||
    (expectedEventType === "thread.unarchived" && thread.archivedAt !== null)
  ) {
    throw new Error(invalidMessage);
  }
  const event = value.event;
  if (event === null) {
    if (value.changed) throw new Error(invalidMessage);
    return { thread, changed: false, event: null };
  }
  if (!value.changed) throw new Error(invalidMessage);
  let parsedEvent: RuntimeJournalEvent;
  try {
    parsedEvent = parseRuntimeJournalEvent(event);
  } catch {
    throw new Error(invalidMessage);
  }
  const eventThread = parseRuntimeThreadSummary(parsedEvent.payload.thread, invalidMessage);
  if (
    parsedEvent.threadId !== thread.id ||
    parsedEvent.branchId !== thread.defaultBranchId ||
    parsedEvent.timestamp !== thread.updatedAt ||
    !sameRuntimeThreadSummary(eventThread, thread) ||
    (expectedEventType !== undefined && parsedEvent.type !== expectedEventType) ||
    !["thread.renamed", "thread.archived", "thread.unarchived"].includes(
      parsedEvent.type
    )
  ) {
    throw new Error(invalidMessage);
  }
  return { thread, changed: true, event: parsedEvent };
}

function sameRuntimeThreadSummary(
  left: RuntimeThreadSummary,
  right: RuntimeThreadSummary
): boolean {
  const sameWorkspace =
    left.workspace === null
      ? right.workspace === null
      : right.workspace !== null &&
        left.workspace.id === right.workspace.id &&
        left.workspace.name === right.workspace.name &&
        left.workspace.rootUri === right.workspace.rootUri;
  return (
    left.id === right.id &&
    left.title === right.title &&
    left.defaultBranchId === right.defaultBranchId &&
    sameWorkspace &&
    left.createdAt === right.createdAt &&
    left.updatedAt === right.updatedAt &&
    left.archivedAt === right.archivedAt
  );
}

export function parseRuntimeThreadGetResult(
  value: unknown,
  expectedThreadId?: string
): RuntimeThreadGetResult {
  const invalidMessage = "Runtime returned invalid Thread metadata.";
  if (!isWireObject(value) || !isSafeNonNegativeInteger(value.snapshotSeq)) {
    throw new Error(invalidMessage);
  }
  const thread = parseRuntimeThreadSummary(value.thread, invalidMessage);
  if (expectedThreadId !== undefined && thread.id !== expectedThreadId) {
    throw new Error(invalidMessage);
  }
  return { thread, snapshotSeq: value.snapshotSeq };
}

function invalidTurnHistory(): Error {
  return new Error("Runtime returned an invalid Turn history page.");
}

function parseRuntimeItemHistory(
  value: unknown,
  turnId: string,
  runId: string
): RuntimeItemHistory {
  if (!isWireObject(value)) {
    throw invalidTurnHistory();
  }
  if (
    !hasExactKeys(value, [
      "id",
      "turnId",
      "runId",
      "ordinal",
      "kind",
      "role",
      "status",
      "content",
      "data",
      "createdAt",
      "updatedAt"
    ])
  ) {
    throw invalidTurnHistory();
  }
  const item = value as Partial<RuntimeItemHistory>;
  const roleMatchesKind =
    (item.kind === "message" && (item.role === "user" || item.role === "assistant")) ||
    (item.kind === "tool_call" && item.role === "assistant") ||
    (item.kind === "tool_result" && item.role === "tool");
  if (
    !isWireIdentifier(item.id) ||
    item.turnId !== turnId ||
    item.runId !== runId ||
    !isSafePositiveInteger(item.ordinal) ||
    !ITEM_KINDS.has(item.kind as string) ||
    !roleMatchesKind ||
    !ITEM_STATUSES.has(item.status as string) ||
    typeof item.content !== "string" ||
    !isWireObject(item.data) ||
    !isNonEmptyString(item.createdAt) ||
    !isNonEmptyString(item.updatedAt)
  ) {
    throw invalidTurnHistory();
  }
  return {
    id: item.id,
    turnId,
    runId,
    ordinal: item.ordinal,
    kind: item.kind as RuntimeItemHistory["kind"],
    role: item.role as RuntimeItemHistory["role"],
    status: item.status as RuntimeItemHistory["status"],
    content: item.content,
    data: item.data,
    createdAt: item.createdAt,
    updatedAt: item.updatedAt
  };
}

function parseRuntimeRunHistory(value: unknown, turnId: string): RuntimeRunHistory {
  if (!isWireObject(value)) {
    throw invalidTurnHistory();
  }
  const run = value as Partial<RuntimeRunHistory>;
  const terminal =
    run.status === "completed" || run.status === "failed" || run.status === "cancelled";
  if (
    !isWireIdentifier(run.id) ||
    run.turnId !== turnId ||
    !isNonEmptyString(run.providerId) ||
    !isNonEmptyString(run.modelId) ||
    run.executionPolicy !== "full_access" ||
    !TURN_AND_RUN_STATUSES.has(run.status as string) ||
    !isNonEmptyString(run.createdAt) ||
    (terminal ? !isNonEmptyString(run.settledAt) : run.settledAt !== null) ||
    !Array.isArray(run.items)
  ) {
    throw invalidTurnHistory();
  }

  const itemIds = new Set<string>();
  let previousOrdinal = 0;
  const items = run.items.map((candidate) => {
    const item = parseRuntimeItemHistory(candidate, turnId, run.id as string);
    if (itemIds.has(item.id) || item.ordinal <= previousOrdinal) {
      throw invalidTurnHistory();
    }
    itemIds.add(item.id);
    previousOrdinal = item.ordinal;
    return item;
  });
  return {
    id: run.id,
    turnId,
    providerId: run.providerId,
    modelId: run.modelId,
    executionPolicy: "full_access",
    status: run.status as RuntimeRunHistory["status"],
    createdAt: run.createdAt,
    settledAt: run.settledAt as string | null,
    items
  };
}

function parseRuntimeTurnHistory(
  value: unknown,
  expectedScope?: Pick<RuntimeTurnListParams, "threadId" | "branchId">
): RuntimeTurnHistory {
  if (!isWireObject(value)) {
    throw invalidTurnHistory();
  }
  const turn = value as Partial<RuntimeTurnHistory>;
  if (
    !isWireIdentifier(turn.id) ||
    !isWireIdentifier(turn.threadId) ||
    !isWireIdentifier(turn.branchId) ||
    (expectedScope !== undefined &&
      (turn.threadId !== expectedScope.threadId || turn.branchId !== expectedScope.branchId)) ||
    !isSafePositiveInteger(turn.ordinal) ||
    !TURN_AND_RUN_STATUSES.has(turn.status as string) ||
    !isNonEmptyString(turn.createdAt) ||
    !isNonEmptyString(turn.updatedAt) ||
    !Array.isArray(turn.runs)
  ) {
    throw invalidTurnHistory();
  }

  const runIds = new Set<string>();
  const runs = turn.runs.map((candidate) => {
    const run = parseRuntimeRunHistory(candidate, turn.id as string);
    if (runIds.has(run.id)) {
      throw invalidTurnHistory();
    }
    runIds.add(run.id);
    return run;
  });
  return {
    id: turn.id,
    threadId: turn.threadId,
    branchId: turn.branchId,
    ordinal: turn.ordinal,
    status: turn.status as RuntimeTurnHistory["status"],
    createdAt: turn.createdAt,
    updatedAt: turn.updatedAt,
    runs
  };
}

export function parseRuntimeTurnListPage(
  value: unknown,
  expectedScope?: Pick<RuntimeTurnListParams, "threadId" | "branchId">
): RuntimeTurnListPage {
  if (!isWireObject(value)) {
    throw invalidTurnHistory();
  }
  const page = value as Partial<RuntimeTurnListPage>;
  const validCursor =
    page.nextCursor === null ||
    (isNonEmptyString(page.nextCursor) &&
      page.nextCursor.length <= TURN_HISTORY_CURSOR_MAX_LENGTH &&
      /^[A-Za-z0-9_-]+$/.test(page.nextCursor));
  if (
    !Array.isArray(page.turns) ||
    page.turns.length > TURN_HISTORY_PAGE_LIMIT ||
    typeof page.hasMore !== "boolean" ||
    !isSafeNonNegativeInteger(page.snapshotSeq) ||
    !validCursor ||
    (page.hasMore && (page.nextCursor === null || page.turns.length === 0)) ||
    (!page.hasMore && page.nextCursor !== null)
  ) {
    throw invalidTurnHistory();
  }

  const turnIds = new Set<string>();
  let previousOrdinal = 0;
  const turns = page.turns.map((candidate) => {
    const turn = parseRuntimeTurnHistory(candidate, expectedScope);
    if (turnIds.has(turn.id) || turn.ordinal <= previousOrdinal) {
      throw invalidTurnHistory();
    }
    turnIds.add(turn.id);
    previousOrdinal = turn.ordinal;
    return turn;
  });
  return {
    turns,
    nextCursor: page.nextCursor as string | null,
    hasMore: page.hasMore,
    snapshotSeq: page.snapshotSeq
  };
}

export function parseRuntimeThreadListPage(value: unknown): RuntimeThreadListPage {
  if (typeof value !== "object" || value === null) {
    throw new Error("Runtime returned an invalid thread catalog page.");
  }
  const page = value as Partial<RuntimeThreadListPage>;
  const validCursor =
    page.nextCursor === null ||
    (typeof page.nextCursor === "string" &&
      page.nextCursor.length > 0 &&
      page.nextCursor.length <= 1024 &&
      /^[A-Za-z0-9_-]+$/.test(page.nextCursor));
  if (
    !Array.isArray(page.threads) ||
    page.threads.length > THREAD_CATALOG_PAGE_LIMIT ||
    typeof page.hasMore !== "boolean" ||
    !Number.isSafeInteger(page.snapshotSeq) ||
    (page.snapshotSeq as number) < 0 ||
    !validCursor ||
    (page.hasMore && (page.nextCursor === null || page.threads.length === 0)) ||
    (!page.hasMore && page.nextCursor !== null)
  ) {
    throw new Error("Runtime returned an invalid thread catalog page.");
  }
  return {
    threads: page.threads.map((thread) => parseRuntimeThreadSummary(thread)),
    nextCursor: page.nextCursor as string | null,
    hasMore: page.hasMore,
    snapshotSeq: page.snapshotSeq as number
  };
}

function invalidRuntimeMethodResult(method: RuntimeRpcMethod): Error {
  return new Error(`Runtime returned an invalid ${method} result.`);
}

function parseRuntimeShutdownResult(value: unknown): { accepted: true } {
  if (!isWireObject(value) || !hasExactKeys(value, ["accepted"]) || value.accepted !== true) {
    throw invalidRuntimeMethodResult("runtime.shutdown");
  }
  return { accepted: true };
}

function parseRuntimeThreadCreateResult(value: unknown): RuntimeThreadCreateResult {
  if (!isWireObject(value) || !hasExactKeys(value, ["thread", "event"])) {
    throw invalidRuntimeMethodResult("thread.create");
  }
  const thread = parseRuntimeThreadSummary(
    value.thread,
    invalidRuntimeMethodResult("thread.create").message
  );
  const event = parseRuntimeJournalEvent(value.event);
  if (
    event.type !== "thread.created" ||
    event.threadId !== thread.id ||
    event.branchId !== thread.defaultBranchId
  ) {
    throw invalidRuntimeMethodResult("thread.create");
  }
  return { thread, event };
}

function parseRuntimeProviderSummary(value: unknown): RuntimeProviderSummary {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, [
      "id",
      "displayName",
      "origin",
      "configured",
      "credentialConfigured",
      "health"
    ]) ||
    !isWireIdentifier(value.id) ||
    !isNonEmptyString(value.displayName) ||
    (value.origin !== "builtin" && value.origin !== "custom") ||
    typeof value.configured !== "boolean" ||
    typeof value.credentialConfigured !== "boolean" ||
    (value.health !== "unknown" && value.health !== "ready" && value.health !== "error")
  ) {
    throw new Error("Runtime returned an invalid Provider summary.");
  }
  return value as unknown as RuntimeProviderSummary;
}

function parseRuntimeModelSummary(value: unknown): RuntimeModelSummary {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["providerId", "id", "displayName", "enabled"]) ||
    !isWireIdentifier(value.providerId) ||
    !isWireIdentifier(value.id) ||
    !isNonEmptyString(value.displayName) ||
    typeof value.enabled !== "boolean"
  ) {
    throw new Error("Runtime returned an invalid Model summary.");
  }
  return value as unknown as RuntimeModelSummary;
}

function parseRuntimeProviderListResult(
  value: unknown
): { providers: RuntimeProviderSummary[] } {
  if (!isWireObject(value) || !hasExactKeys(value, ["providers"]) || !Array.isArray(value.providers)) {
    throw invalidRuntimeMethodResult("provider.list");
  }
  return { providers: value.providers.map(parseRuntimeProviderSummary) };
}

function parseRuntimeProviderResult(
  value: unknown,
  method: "provider.configure" | "provider.disconnect"
): { provider: RuntimeProviderSummary } {
  if (!isWireObject(value) || !hasExactKeys(value, ["provider"])) {
    throw invalidRuntimeMethodResult(method);
  }
  return { provider: parseRuntimeProviderSummary(value.provider) };
}

function parseRuntimeModelInput(value: unknown): { id: string; displayName: string } {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["id", "displayName"]) ||
    !isWireIdentifier(value.id) ||
    !isNonEmptyString(value.displayName)
  ) {
    throw invalidRuntimeMethodResult("provider.discover_models");
  }
  return { id: value.id, displayName: value.displayName };
}

function parseRuntimeProviderDiscoveryResult(
  value: unknown
): { models: Array<{ id: string; displayName: string }> } {
  if (!isWireObject(value) || !hasExactKeys(value, ["models"]) || !Array.isArray(value.models)) {
    throw invalidRuntimeMethodResult("provider.discover_models");
  }
  return { models: value.models.map(parseRuntimeModelInput) };
}

function parseRuntimeProviderRemoveResult(
  value: unknown
): { removed: boolean; providerId: string } {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["removed", "providerId"]) ||
    typeof value.removed !== "boolean" ||
    !isWireIdentifier(value.providerId)
  ) {
    throw invalidRuntimeMethodResult("provider.remove");
  }
  return { removed: value.removed, providerId: value.providerId };
}

function parseRuntimeModelListResult(value: unknown): { models: RuntimeModelSummary[] } {
  if (!isWireObject(value) || !hasExactKeys(value, ["models"]) || !Array.isArray(value.models)) {
    throw invalidRuntimeMethodResult("model.list");
  }
  return { models: value.models.map(parseRuntimeModelSummary) };
}

function parseRuntimeModelSetEnabledResult(
  value: unknown
): { model: RuntimeModelSummary } {
  if (!isWireObject(value) || !hasExactKeys(value, ["model"])) {
    throw invalidRuntimeMethodResult("model.set_enabled");
  }
  return { model: parseRuntimeModelSummary(value.model) };
}

function parseRuntimeSkillSummary(value: unknown): RuntimeSkillSummary {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["name", "description", "location", "enabled"]) ||
    typeof value.name !== "string" ||
    !SKILL_NAME_PATTERN.test(value.name) ||
    !isNonEmptyString(value.description) ||
    !isNonEmptyString(value.location) ||
    typeof value.enabled !== "boolean"
  ) {
    throw invalidRuntimeMethodResult("skill.list");
  }
  return value as unknown as RuntimeSkillSummary;
}

function parseRuntimeSkillDiagnostic(value: unknown): RuntimeSkillDiagnostic {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["entry", "code", "message"]) ||
    !isNonEmptyString(value.entry) ||
    !isNonEmptyString(value.code) ||
    !isNonEmptyString(value.message)
  ) {
    throw invalidRuntimeMethodResult("skill.list");
  }
  return value as unknown as RuntimeSkillDiagnostic;
}

function parseRuntimeSkillListResult(value: unknown): RuntimeSkillListResult {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["skills", "diagnostics"]) ||
    !Array.isArray(value.skills) ||
    !Array.isArray(value.diagnostics)
  ) {
    throw invalidRuntimeMethodResult("skill.list");
  }
  const skills = value.skills.map(parseRuntimeSkillSummary);
  for (let index = 1; index < skills.length; index += 1) {
    if (skills[index - 1].name >= skills[index].name) {
      throw invalidRuntimeMethodResult("skill.list");
    }
  }
  return {
    skills,
    diagnostics: value.diagnostics.map(parseRuntimeSkillDiagnostic)
  };
}

function parseRuntimeSkillSetEnabledResult(value: unknown): RuntimeSkillSetEnabledResult {
  if (!isWireObject(value) || !hasExactKeys(value, ["skill"])) {
    throw invalidRuntimeMethodResult("skill.set_enabled");
  }
  return { skill: parseRuntimeSkillSummary(value.skill) };
}

function isMemoryId(value: unknown): value is string {
  return typeof value === "string" && MEMORY_ID_PATTERN.test(value);
}

function isValidMemoryText(
  value: unknown,
  maximumCharacters: number,
  requireNonBlank: boolean
): value is string {
  if (typeof value !== "string") return false;
  const characters = Array.from(value);
  if (
    characters.length > maximumCharacters ||
    (requireNonBlank && characters.every(isPythonWhitespace))
  ) {
    return false;
  }
  return characters.every((character) => {
    const codePoint = character.codePointAt(0) as number;
    return (
      !((codePoint < 0x20 && codePoint !== 0x09 && codePoint !== 0x0a && codePoint !== 0x0d) ||
        codePoint === 0x7f) &&
      !(codePoint >= 0xd800 && codePoint <= 0xdfff)
    );
  });
}

function isPythonWhitespace(character: string): boolean {
  const codePoint = character.codePointAt(0) as number;
  return (
    (codePoint >= 0x09 && codePoint <= 0x0d) ||
    (codePoint >= 0x1c && codePoint <= 0x20) ||
    codePoint === 0x85 ||
    codePoint === 0xa0 ||
    codePoint === 0x1680 ||
    (codePoint >= 0x2000 && codePoint <= 0x200a) ||
    codePoint === 0x2028 ||
    codePoint === 0x2029 ||
    codePoint === 0x202f ||
    codePoint === 0x205f ||
    codePoint === 0x3000
  );
}

function isPythonTrimmed(value: string): boolean {
  const characters = Array.from(value);
  return (
    characters.length > 0 &&
    !isPythonWhitespace(characters[0]) &&
    !isPythonWhitespace(characters[characters.length - 1])
  );
}

function parseRuntimeMemoryScope(
  value: unknown,
  method: "memory.list" | "memory.get"
): RuntimeMemoryScope {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["type", "key"]) ||
    (value.type === "global"
      ? value.key !== null
      : value.type !== "workspace" ||
        !isValidMemoryText(value.key, MAX_WIRE_IDENTIFIER_LENGTH, true) ||
        !isPythonTrimmed(value.key))
  ) {
    throw invalidRuntimeMethodResult(method);
  }
  return value as unknown as RuntimeMemoryScope;
}

function parseRuntimeMemoryProvenance(value: unknown): RuntimeMemoryProvenance {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["sourceKind", "threadId", "turnId", "itemId", "status"])
  ) {
    throw invalidRuntimeMethodResult("memory.get");
  }
  const references = [value.threadId, value.turnId, value.itemId];
  const valid =
    (value.sourceKind === "user_explicit" &&
      value.status === "not_applicable" &&
      references.every((reference) => reference === null)) ||
    (value.sourceKind === "session_item" &&
      (value.status === "available" || value.status === "unavailable") &&
      references.every(isWireIdentifier));
  if (!valid) {
    throw invalidRuntimeMethodResult("memory.get");
  }
  return value as unknown as RuntimeMemoryProvenance;
}

function parseRuntimeMemorySummary(value: unknown): RuntimeMemorySummary {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, [
      "id",
      "kind",
      "scope",
      "revision",
      "state",
      "preview",
      "createdAt",
      "updatedAt",
      "forgottenAt"
    ]) ||
    !isMemoryId(value.id) ||
    !MEMORY_KINDS.has(value.kind as RuntimeMemoryKind) ||
    !isSafePositiveInteger(value.revision) ||
    !MEMORY_STATES.has(value.state as RuntimeMemoryState) ||
    !isCanonicalTimestamp(value.createdAt) ||
    !isCanonicalTimestamp(value.updatedAt)
  ) {
    throw invalidRuntimeMethodResult("memory.list");
  }
  const scope = parseRuntimeMemoryScope(value.scope, "memory.list");
  if (
    (value.state === "active" &&
      (!isValidMemoryText(value.preview, MEMORY_PREVIEW_MAX_CHARACTERS, false) ||
        value.preview.length === 0 ||
        value.forgottenAt !== null)) ||
    (value.state === "forgotten" &&
      (value.preview !== null || !isCanonicalTimestamp(value.forgottenAt)))
  ) {
    throw invalidRuntimeMethodResult("memory.list");
  }
  return {
    id: value.id,
    kind: value.kind as RuntimeMemoryKind,
    scope,
    revision: value.revision,
    state: value.state as RuntimeMemoryState,
    preview: value.preview as string | null,
    createdAt: value.createdAt,
    updatedAt: value.updatedAt,
    forgottenAt: value.forgottenAt as string | null
  };
}

function parseRuntimeMemoryRecord(value: unknown): RuntimeMemoryRecord {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, [
      "id",
      "kind",
      "scope",
      "revision",
      "state",
      "content",
      "provenance",
      "createdAt",
      "updatedAt",
      "forgottenAt"
    ]) ||
    !isMemoryId(value.id) ||
    !MEMORY_KINDS.has(value.kind as RuntimeMemoryKind) ||
    !isSafePositiveInteger(value.revision) ||
    !MEMORY_STATES.has(value.state as RuntimeMemoryState) ||
    !isCanonicalTimestamp(value.createdAt) ||
    !isCanonicalTimestamp(value.updatedAt)
  ) {
    throw invalidRuntimeMethodResult("memory.get");
  }
  const scope = parseRuntimeMemoryScope(value.scope, "memory.get");
  const provenance = parseRuntimeMemoryProvenance(value.provenance);
  if (
    (value.state === "active" &&
      (!isValidMemoryText(value.content, MEMORY_CONTENT_MAX_CHARACTERS, true) ||
        value.forgottenAt !== null)) ||
    (value.state === "forgotten" &&
      (value.content !== null || !isCanonicalTimestamp(value.forgottenAt)))
  ) {
    throw invalidRuntimeMethodResult("memory.get");
  }
  return {
    id: value.id,
    kind: value.kind as RuntimeMemoryKind,
    scope,
    revision: value.revision,
    state: value.state as RuntimeMemoryState,
    content: value.content as string | null,
    provenance,
    createdAt: value.createdAt,
    updatedAt: value.updatedAt,
    forgottenAt: value.forgottenAt as string | null
  };
}

export function parseRuntimeMemoryCreateResult(value: unknown): RuntimeMemoryCreateResult {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["memoryId", "resultingRevision", "created"]) ||
    !isMemoryId(value.memoryId) ||
    value.resultingRevision !== 1 ||
    typeof value.created !== "boolean"
  ) {
    throw invalidRuntimeMethodResult("memory.create");
  }
  return value as unknown as RuntimeMemoryCreateResult;
}

export function parseRuntimeMemoryListPage(
  value: unknown,
  params: Readonly<Record<string, unknown>> = {}
): RuntimeMemoryListPage {
  const requestedLimit = params.limit ?? MEMORY_LIST_DEFAULT_LIMIT;
  if (
    typeof requestedLimit !== "number" ||
    !Number.isInteger(requestedLimit) ||
    requestedLimit < 1 ||
    requestedLimit > MEMORY_LIST_PAGE_LIMIT ||
    !isWireObject(value) ||
    !hasExactKeys(value, ["memories", "nextCursor", "hasMore"]) ||
    !Array.isArray(value.memories) ||
    value.memories.length > requestedLimit ||
    typeof value.hasMore !== "boolean" ||
    !(
      value.nextCursor === null ||
      (isNonEmptyString(value.nextCursor) &&
        value.nextCursor.length <= MEMORY_CURSOR_MAX_LENGTH &&
        /^[A-Za-z0-9_-]+$/.test(value.nextCursor))
    ) ||
    (value.hasMore && (value.nextCursor === null || value.memories.length === 0)) ||
    (value.hasMore && params.cursor === value.nextCursor) ||
    (!value.hasMore && value.nextCursor !== null)
  ) {
    throw invalidRuntimeMethodResult("memory.list");
  }
  const expectedScope =
    params.scope === undefined
      ? undefined
      : parseRuntimeMemoryScope(params.scope, "memory.list");
  const expectedKind = params.kind as RuntimeMemoryKind | undefined;
  const expectedState = (params.state ?? "active") as RuntimeMemoryState;
  const memories = value.memories.map(parseRuntimeMemorySummary);
  const ids = new Set<string>();
  for (let index = 0; index < memories.length; index += 1) {
    const memory = memories[index];
    const previous = memories[index - 1];
    if (
      ids.has(memory.id) ||
      (expectedScope !== undefined &&
        (memory.scope.type !== expectedScope.type || memory.scope.key !== expectedScope.key)) ||
      (expectedKind !== undefined && memory.kind !== expectedKind) ||
      memory.state !== expectedState ||
      (previous !== undefined &&
        (previous.updatedAt < memory.updatedAt ||
          (previous.updatedAt === memory.updatedAt && previous.id >= memory.id)))
    ) {
      throw invalidRuntimeMethodResult("memory.list");
    }
    ids.add(memory.id);
  }
  return {
    memories,
    nextCursor: value.nextCursor as string | null,
    hasMore: value.hasMore
  };
}

export function parseRuntimeMemoryGetResult(
  value: unknown,
  expectedMemoryId?: string
): RuntimeMemoryGetResult {
  if (!isWireObject(value) || !hasExactKeys(value, ["memory"])) {
    throw invalidRuntimeMethodResult("memory.get");
  }
  const memory = parseRuntimeMemoryRecord(value.memory);
  if (expectedMemoryId !== undefined && memory.id !== expectedMemoryId) {
    throw invalidRuntimeMethodResult("memory.get");
  }
  return { memory };
}

function parseRuntimeTurnStartResult(
  value: unknown
): { threadId: string; branchId: string; turnId: string; runId: string } {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["turnId", "runId", "threadId", "branchId"]) ||
    !isWireIdentifier(value.threadId) ||
    !isWireIdentifier(value.branchId) ||
    !isWireIdentifier(value.turnId) ||
    !isWireIdentifier(value.runId)
  ) {
    throw invalidRuntimeMethodResult("turn.start");
  }
  return {
    threadId: value.threadId,
    branchId: value.branchId,
    turnId: value.turnId,
    runId: value.runId
  };
}

function parseRuntimeCancelRunResult(value: unknown): RuntimeCancelRunResult {
  if (
    !isWireObject(value) ||
    !hasExactKeys(value, ["accepted", "runId", "status"]) ||
    typeof value.accepted !== "boolean" ||
    !isWireIdentifier(value.runId) ||
    !TURN_AND_RUN_STATUSES.has(value.status as string)
  ) {
    throw invalidRuntimeMethodResult("run.cancel");
  }
  return value as unknown as RuntimeCancelRunResult;
}

type RuntimeResultParser = (
  value: unknown,
  params: Readonly<Record<string, unknown>>
) => unknown;

const RUNTIME_RESULT_PARSERS = {
  "runtime.shutdown": (value) => parseRuntimeShutdownResult(value),
  "thread.create": (value) => parseRuntimeThreadCreateResult(value),
  "thread.rename": (value, params) =>
    parseRuntimeThreadMutationResult(
      value,
      typeof params.threadId === "string" ? params.threadId : undefined,
      "thread.renamed"
    ),
  "thread.archive": (value, params) =>
    parseRuntimeThreadMutationResult(
      value,
      typeof params.threadId === "string" ? params.threadId : undefined,
      "thread.archived"
    ),
  "thread.unarchive": (value, params) =>
    parseRuntimeThreadMutationResult(
      value,
      typeof params.threadId === "string" ? params.threadId : undefined,
      "thread.unarchived"
    ),
  "thread.get": (value, params) =>
    parseRuntimeThreadGetResult(
      value,
      typeof params.threadId === "string" ? params.threadId : undefined
    ),
  "thread.list": (value) => parseRuntimeThreadListPage(value),
  "provider.list": (value) => parseRuntimeProviderListResult(value),
  "provider.configure": (value) => parseRuntimeProviderResult(value, "provider.configure"),
  "provider.discover_models": (value) => parseRuntimeProviderDiscoveryResult(value),
  "provider.disconnect": (value) => parseRuntimeProviderResult(value, "provider.disconnect"),
  "provider.remove": (value) => parseRuntimeProviderRemoveResult(value),
  "model.list": (value) => parseRuntimeModelListResult(value),
  "model.set_enabled": (value) => parseRuntimeModelSetEnabledResult(value),
  "skill.list": (value) => parseRuntimeSkillListResult(value),
  "skill.set_enabled": (value) => parseRuntimeSkillSetEnabledResult(value),
  "memory.create": (value) => parseRuntimeMemoryCreateResult(value),
  "memory.list": (value, params) => parseRuntimeMemoryListPage(value, params),
  "memory.get": (value, params) =>
    parseRuntimeMemoryGetResult(
      value,
      typeof params.memoryId === "string" ? params.memoryId : undefined
    ),
  "turn.start": (value) => parseRuntimeTurnStartResult(value),
  "turn.list": (value, params) => {
    const expectedScope =
      typeof params.threadId === "string" && typeof params.branchId === "string"
        ? { threadId: params.threadId, branchId: params.branchId }
        : undefined;
    return parseRuntimeTurnListPage(value, expectedScope);
  },
  "run.cancel": (value) => parseRuntimeCancelRunResult(value),
  "event.replay": (value) => parseRuntimeReplayResult(value),
  "usage.read": (value) => parseRuntimeUsageReadResult(value)
} satisfies Record<RuntimeRpcMethod, RuntimeResultParser>;

function isRuntimeRpcMethod(method: string): method is RuntimeRpcMethod {
  return RUNTIME_RPC_METHOD_SET.has(method);
}

export function parseRuntimeMethodResult(
  method: string,
  value: unknown,
  params: Readonly<Record<string, unknown>> = {}
): unknown {
  if (!isRuntimeRpcMethod(method)) {
    throw new Error(`Desktop does not support Runtime RPC method ${method}.`);
  }
  return RUNTIME_RESULT_PARSERS[method](value, params);
}

export function parseRuntimeJsonRpcResponse(value: unknown): JsonRpcResponse {
  if (typeof value !== "object" || value === null) {
    throw new Error("Runtime returned an invalid JSON-RPC response.");
  }
  const candidate = value as {
    jsonrpc?: unknown;
    id?: unknown;
    result?: unknown;
    error?: unknown;
  };
  const hasResult = hasOwn(candidate, "result");
  const hasError = hasOwn(candidate, "error");
  if (
    candidate.jsonrpc !== "2.0" ||
    typeof candidate.id !== "number" ||
    !Number.isInteger(candidate.id) ||
    hasResult === hasError
  ) {
    throw new Error("Runtime returned an invalid JSON-RPC response.");
  }
  if (hasResult) {
    return { jsonrpc: "2.0", id: candidate.id, result: candidate.result };
  }
  const error = candidate.error;
  if (
    typeof error !== "object" ||
    error === null ||
    typeof (error as { code?: unknown }).code !== "number" ||
    !Number.isInteger((error as { code: number }).code) ||
    typeof (error as { message?: unknown }).message !== "string"
  ) {
    throw new Error("Runtime returned an invalid JSON-RPC response.");
  }
  return {
    jsonrpc: "2.0",
    id: candidate.id,
    error: {
      code: (error as { code: number }).code,
      message: (error as { message: string }).message
    }
  };
}
