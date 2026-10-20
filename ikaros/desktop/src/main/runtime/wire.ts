import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  RUNTIME_JOURNAL_EVENT_TYPES,
  RUNTIME_PROTOCOL_MANIFEST,
  RUNTIME_RPC_METHODS,
  type RuntimeJournalEventType,
  type RuntimeRpcMethod
} from "../../shared/generated/runtimeProtocol";
import type {
  RuntimeFileChangeResult,
  RuntimeFilePreviewResult,
  RuntimeInitializeResult,
  RuntimeJournalEvent,
  RuntimeMemoryCreateResult,
  RuntimeMemoryErrorReasonCode,
  RuntimeMemoryGetResult,
  RuntimeMemoryListPage,
  RuntimeMemoryMutationResult,
  RuntimeMemoryRpcMethod,
  RuntimeReplayResult,
  RuntimeThreadGetResult,
  RuntimeThreadListPage,
  RuntimeThreadMutationResult,
  RuntimeTurnListPage,
  RuntimeUsageReadResult
} from "../../shared/runtime";
import { RUNTIME_MEMORY_ERROR_REASON_CODES_BY_METHOD } from "../../shared/runtime";

const RUNTIME_RPC_METHOD_SET = new Set<string>(RUNTIME_RPC_METHODS);
const RUNTIME_JOURNAL_EVENT_TYPE_SET = new Set<string>(RUNTIME_JOURNAL_EVENT_TYPES);
const MEMORY_OPERATION_ERROR_CODE =
  RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.code;
const MEMORY_OPERATION_ERROR_MESSAGE =
  RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.message;

interface JsonRpcResultResponse {
  jsonrpc: "2.0";
  id: number;
  result: unknown;
}

interface JsonRpcErrorResponse {
  jsonrpc: "2.0";
  id: number;
  error: {
    code: number;
    message: string;
    reasonCode?: RuntimeMemoryErrorReasonCode;
  };
}

export type JsonRpcResponse = JsonRpcResultResponse | JsonRpcErrorResponse;

export class RuntimeRpcError extends Error {
  readonly kind = "json_rpc" as const;

  constructor(
    readonly code: number,
    message: string,
    readonly reasonCode?: RuntimeMemoryErrorReasonCode
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

function isWireObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(value: object, expectedKeys: readonly string[]): boolean {
  const keys = Object.keys(value);
  if (keys.length !== expectedKeys.length) {
    return false;
  }
  return expectedKeys.every((key) => hasOwn(value, key));
}

function isSafePositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

function isNullableString(value: unknown): value is string | null {
  return value === null || (typeof value === "string" && value.length > 0);
}

function isRunReasonCode(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 200 &&
    /^[A-Za-z0-9_.-]+$/.test(value)
  );
}

function validateTurnHistory(value: Record<string, unknown>): void {
  if (!Array.isArray(value.turns)) return;
  for (const turn of value.turns) {
    if (!isWireObject(turn) || !Array.isArray(turn.runs)) continue;
    for (const run of turn.runs) {
      if (!isWireObject(run)) continue;
      if (!hasOwn(run, "reasonCode")) {
        throw new Error("Runtime returned an invalid Turn history page.");
      }
      const status = run.status;
      const reasonCode = run.reasonCode;
      if (
        status === "queued" ||
        status === "running" ||
        status === "completed"
      ) {
        if (reasonCode !== null) {
          throw new Error("Runtime returned an invalid Turn history page.");
        }
      } else if (reasonCode !== null && !isRunReasonCode(reasonCode)) {
        throw new Error("Runtime returned an invalid Turn history page.");
      }
    }
  }
}

export function isRuntimeNotification(
  value: unknown
): value is RuntimeNotification {
  try {
    parseRuntimeEventNotification(value);
    return true;
  } catch {
    return false;
  }
}

export function parseRuntimeEventNotification(
  value: unknown
): RuntimeNotification {
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
    typeof value.type !== "string" ||
    value.type.length === 0 ||
    !isNullableString(value.threadId) ||
    !isNullableString(value.branchId) ||
    !isNullableString(value.turnId) ||
    !isNullableString(value.runId) ||
    !isNullableString(value.itemId) ||
    typeof value.timestamp !== "string" ||
    value.timestamp.length === 0 ||
    !isWireObject(value.payload)
  ) {
    throw new Error("Runtime returned an invalid journal event.");
  }
  if (
    value.type === "run.settled" &&
    isWireObject(value.payload) &&
    typeof value.payload.reasonCode === "string" &&
    !isRunReasonCode(value.payload.reasonCode)
  ) {
    throw new Error("Runtime returned an invalid run.settled journal event payload.");
  }
  return {
    seq: value.seq,
    schemaVersion: value.schemaVersion as typeof RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type: value.type as RuntimeJournalEventType,
    threadId: value.threadId,
    branchId: value.branchId,
    turnId: value.turnId,
    runId: value.runId,
    itemId: value.itemId,
    timestamp: value.timestamp,
    payload: value.payload
  };
}

export function parseRuntimeInitializeResult(
  value: unknown
): RuntimeInitializeResult {
  if (
    !isWireObject(value) ||
    typeof value.protocolVersion !== "number" ||
    !Number.isInteger(value.protocolVersion) ||
    !isWireObject(value.server) ||
    typeof value.server.name !== "string" ||
    typeof value.server.version !== "string"
  ) {
    throw new Error("Runtime returned an invalid initialize payload.");
  }
  return {
    protocolVersion: value.protocolVersion,
    server: {
      name: value.server.name,
      version: value.server.version
    }
  };
}

export function parseRuntimeReplayResult(value: unknown): RuntimeReplayResult {
  if (
    !isWireObject(value) ||
    !Array.isArray(value.events) ||
    typeof value.latestSeq !== "number" ||
    !Number.isInteger(value.latestSeq) ||
    typeof value.nextAfterSeq !== "number" ||
    !Number.isInteger(value.nextAfterSeq) ||
    typeof value.hasMore !== "boolean"
  ) {
    throw new Error("Runtime returned an invalid event replay result.");
  }
  return {
    events: value.events.map(parseRuntimeJournalEvent),
    latestSeq: value.latestSeq,
    nextAfterSeq: value.nextAfterSeq,
    hasMore: value.hasMore
  };
}

export function parseRuntimeThreadListPage(
  value: unknown
): RuntimeThreadListPage {
  if (
    !isWireObject(value) ||
    !Array.isArray(value.threads) ||
    typeof value.hasMore !== "boolean" ||
    typeof value.snapshotSeq !== "number" ||
    !Number.isInteger(value.snapshotSeq) ||
    value.snapshotSeq < 0
  ) {
    throw new Error("Runtime returned an invalid thread catalog page.");
  }
  return value as unknown as RuntimeThreadListPage;
}

export function parseRuntimeThreadGetResult(
  value: unknown,
  expectedThreadId?: string
): RuntimeThreadGetResult {
  if (
    !isWireObject(value) ||
    !isWireObject(value.thread) ||
    typeof value.thread.id !== "string" ||
    (expectedThreadId !== undefined && value.thread.id !== expectedThreadId)
  ) {
    throw new Error("Runtime returned invalid Thread metadata.");
  }
  return value as unknown as RuntimeThreadGetResult;
}

export function parseRuntimeThreadMutationResult(
  value: unknown,
  expectedThreadId?: string,
  eventType?: string
): RuntimeThreadMutationResult {
  if (
    !isWireObject(value) ||
    !isWireObject(value.thread) ||
    typeof value.changed !== "boolean" ||
    (value.event !== null && !isWireObject(value.event)) ||
    (value.changed ? value.event === null : value.event !== null) ||
    (expectedThreadId !== undefined && value.thread.id !== expectedThreadId) ||
    (eventType !== undefined &&
      value.event !== null &&
      (value.event as { type?: unknown }).type !== eventType)
  ) {
    throw new Error("Runtime returned an invalid Thread mutation result.");
  }
  const event =
    value.event === null ? null : parseRuntimeJournalEvent(value.event);
  return {
    thread: value.thread as unknown as RuntimeThreadMutationResult["thread"],
    changed: value.changed,
    event
  };
}

export function parseRuntimeTurnListPage(
  value: unknown,
  expectedScope?: { threadId: string; branchId: string }
): RuntimeTurnListPage {
  if (
    !isWireObject(value) ||
    !Array.isArray(value.turns) ||
    typeof value.hasMore !== "boolean" ||
    typeof value.snapshotSeq !== "number" ||
    !Number.isInteger(value.snapshotSeq) ||
    value.snapshotSeq < 0
  ) {
    throw new Error("Runtime returned an invalid Turn history page.");
  }
  validateTurnHistory(value);
  return value as unknown as RuntimeTurnListPage;
}

export function parseRuntimeUsageReadResult(
  value: unknown
): RuntimeUsageReadResult {
  if (!isWireObject(value)) {
    throw new Error("Runtime returned invalid usage metrics.");
  }
  return value as unknown as RuntimeUsageReadResult;
}

export function parseRuntimeMemoryCreateResult(
  value: unknown
): RuntimeMemoryCreateResult {
  if (!isWireObject(value)) {
    throw new Error("Runtime returned an invalid memory creation result.");
  }
  return value as unknown as RuntimeMemoryCreateResult;
}

export function parseRuntimeMemoryMutationResult(
  value: unknown,
  params?: unknown,
  method?: string
): RuntimeMemoryMutationResult {
  if (!isWireObject(value)) {
    throw new Error("Runtime returned an invalid memory mutation result.");
  }
  return value as unknown as RuntimeMemoryMutationResult;
}

export function parseRuntimeMemoryListPage(
  value: unknown,
  params?: unknown
): RuntimeMemoryListPage {
  if (!isWireObject(value) || !Array.isArray(value.memories)) {
    throw new Error("Runtime returned an invalid memory list page.");
  }
  return value as unknown as RuntimeMemoryListPage;
}

export function parseRuntimeMemoryGetResult(
  value: unknown,
  expectedMemoryId?: string
): RuntimeMemoryGetResult {
  if (
    !isWireObject(value) ||
    !isWireObject(value.memory) ||
    typeof value.memory.id !== "string" ||
    (expectedMemoryId !== undefined && value.memory.id !== expectedMemoryId)
  ) {
    throw new Error("Runtime returned an invalid memory record.");
  }
  return value as unknown as RuntimeMemoryGetResult;
}

export function parseRuntimeFilePreviewResult(
  value: unknown,
  params?: unknown
): RuntimeFilePreviewResult {
  if (!isWireObject(value)) {
    throw new Error("Runtime returned an invalid file preview result.");
  }
  return value as unknown as RuntimeFilePreviewResult;
}

export function parseRuntimeFileChangeResult(
  value: unknown,
  params?: unknown
): RuntimeFileChangeResult {
  if (!isWireObject(value)) {
    throw new Error("Runtime returned an invalid file change result.");
  }
  return value as unknown as RuntimeFileChangeResult;
}

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
  if (!isWireObject(value)) {
    throw new Error(`Runtime returned an invalid ${method} result.`);
  }
  return value;
}

export function parseRuntimeJsonRpcResponse(
  value: unknown,
  pendingMethod?: string
): JsonRpcResponse {
  if (!isWireObject(value)) {
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
    !isWireObject(error) ||
    typeof error.code !== "number" ||
    !Number.isInteger(error.code) ||
    typeof error.message !== "string"
  ) {
    throw new Error("Runtime returned an invalid JSON-RPC response.");
  }
  let reasonCode: RuntimeMemoryErrorReasonCode | undefined;
  if (error.code === MEMORY_OPERATION_ERROR_CODE) {
    if (
      !hasExactKeys(error, ["code", "message", "data"]) ||
      error.message !== MEMORY_OPERATION_ERROR_MESSAGE ||
      !isWireObject(error.data) ||
      !hasExactKeys(error.data, ["reasonCode"]) ||
      typeof error.data.reasonCode !== "string" ||
      !isMemoryErrorReasonAllowed(pendingMethod, error.data.reasonCode)
    ) {
      throw new Error("Runtime returned an invalid JSON-RPC response.");
    }
    reasonCode = error.data.reasonCode as RuntimeMemoryErrorReasonCode;
  } else if (!hasExactKeys(error, ["code", "message"])) {
    throw new Error("Runtime returned an invalid JSON-RPC response.");
  }
  return {
    jsonrpc: "2.0",
    id: candidate.id,
    error: {
      code: error.code,
      message: error.message,
      ...(reasonCode === undefined ? {} : { reasonCode })
    }
  };
}

function isMemoryErrorReasonAllowed(
  method: string | undefined,
  reasonCode: string
): reasonCode is RuntimeMemoryErrorReasonCode {
  if (
    method === undefined ||
    !hasOwn(RUNTIME_MEMORY_ERROR_REASON_CODES_BY_METHOD, method)
  ) {
    return false;
  }
  return (
    RUNTIME_MEMORY_ERROR_REASON_CODES_BY_METHOD[
      method as RuntimeMemoryRpcMethod
    ] as readonly string[]
  ).includes(reasonCode);
}
