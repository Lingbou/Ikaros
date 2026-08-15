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
const RUNTIME_JOURNAL_EVENT_TYPE_SET = new Set<string>(RUNTIME_JOURNAL_EVENT_TYPES);
const RUNTIME_PROVIDER_TOOL_ID_SET = new Set<string>(RUNTIME_PROVIDER_TOOL_IDS);
const RUNTIME_RPC_METHOD_SET = new Set<string>(RUNTIME_RPC_METHODS);
const SKILL_NAME_PATTERN = /^[a-z0-9][a-z0-9-]{0,63}$/;

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

  if (event.type === "model.usage_recorded") {
    if (
      !hasRunEventScope(event, false) ||
      !hasScopedPayloadKeys(event, [
        "stepOrdinal",
        "providerId",
        "modelId",
        "activityDate",
        "completedAt",
        "usage"
      ]) ||
      !isSafePositiveInteger(payload.stepOrdinal) ||
      !isNonEmptyString(payload.providerId) ||
      !isNonEmptyString(payload.modelId) ||
      !isCanonicalCalendarDate(payload.activityDate) ||
      payload.completedAt !== event.timestamp ||
      !isWireObject(payload.usage) ||
      !hasExactKeys(payload.usage, [
        "inputTokens",
        "cachedInputTokens",
        "outputTokens",
        "reasoningOutputTokens",
        "totalTokens"
      ]) ||
      !isSafeNonNegativeInteger(payload.usage.inputTokens) ||
      !isNullableSafeNonNegativeInteger(payload.usage.cachedInputTokens) ||
      !isSafeNonNegativeInteger(payload.usage.outputTokens) ||
      !isNullableSafeNonNegativeInteger(payload.usage.reasoningOutputTokens) ||
      !isSafeNonNegativeInteger(payload.usage.totalTokens)
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
    isWireObject(data.result)
  );
}

function parseInitialTurnCompletedEvent(event: RuntimeJournalEvent): void {
  const payload = event.payload;
  if (
    !hasScopedPayloadKeys(event, ["turn", "run", "item"], ["clientRequestId"]) ||
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
    !parseRuntimeEventItem(event, payload.item, true)
  ) {
    invalidJournalEventPayload(event.type);
  }
  const item = payload.item as RuntimeItemHistory;
  if (item.kind !== "message" || item.role !== "user" || item.status !== "completed") {
    invalidJournalEventPayload(event.type);
  }
}

function isRuntimeSkillSnapshot(value: unknown): boolean {
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

function isProviderToolId(value: unknown): boolean {
  return typeof value === "string" && RUNTIME_PROVIDER_TOOL_ID_SET.has(value);
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
