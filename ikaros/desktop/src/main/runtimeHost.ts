import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { createInterface } from "node:readline";

import WebSocket, { type RawData } from "ws";

import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  type RuntimeItemHistory,
  type RuntimeJournalEvent,
  type RuntimeReplayResult,
  type RuntimeRunHistory,
  type RuntimeThreadGetResult,
  type RuntimeThreadCatalogParams,
  type RuntimeThreadListPage,
  type RuntimeThreadMutationResult,
  type RuntimeThreadSummary,
  type RuntimeTurnHistory,
  type RuntimeTurnListPage,
  type RuntimeTurnListParams
} from "../shared/runtime";

const PROTOCOL_VERSION = 1;
const SOCKET_RECONNECT_DELAYS_MS = [50, 100, 200, 400, 800] as const;
const RUNTIME_RESTART_DELAYS_MS = [100, 200, 400] as const;
const THREAD_CATALOG_PAGE_LIMIT = 100;
const MAX_THREAD_CATALOG_PAGES = 10_000;
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

function wait(delayMs: number): Promise<void> {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, delayMs));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function reportRuntimeHostFailure(message: string): void {
  try {
    process.stderr.write(`[ikaros-runtime] ${message}\n`);
  } catch {
    // Diagnostics must never break Runtime recovery or event delivery.
  }
}

interface RuntimeReadyRecord {
  type: "ikaros_runtime.ready";
  protocolVersion: number;
  host: string;
  port: number;
  pid: number;
}

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

type JsonRpcResponse = JsonRpcResultResponse | JsonRpcErrorResponse;

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

function isRuntimeNotification(value: unknown): value is RuntimeNotification {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as Partial<RuntimeNotification>).jsonrpc === "2.0" &&
    typeof (value as Partial<RuntimeNotification>).method === "string" &&
    !hasOwn(value, "id") &&
    hasOwn(value, "params")
  );
}

function responseId(value: unknown): number | undefined {
  if (typeof value !== "object" || value === null) {
    return undefined;
  }
  const id = (value as { id?: unknown }).id;
  return typeof id === "number" && Number.isInteger(id) ? id : undefined;
}

export function parseRuntimeJournalEvent(value: unknown): RuntimeJournalEvent {
  if (typeof value !== "object" || value === null) {
    throw new Error("Runtime returned an invalid journal event.");
  }
  const event = value as Partial<RuntimeJournalEvent>;
  if (!Number.isInteger(event.seq) || typeof event.type !== "string") {
    throw new Error("Runtime returned an invalid journal event.");
  }
  if (event.schemaVersion !== RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION) {
    throw new Error(
      `Runtime journal event schema ${String(event.schemaVersion)} is unsupported.`
    );
  }
  return event as RuntimeJournalEvent;
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

function isSafeNonNegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function isSafePositiveInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) > 0;
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
  const parsedEvent = parseRuntimeJournalEvent(event);
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

interface RuntimeThreadCatalogRequester {
  request(method: string, params?: Record<string, unknown>): Promise<unknown>;
}

export async function listAllRuntimeThreads(
  requester: RuntimeThreadCatalogRequester,
  options: RuntimeThreadCatalogParams = {}
): Promise<{ threads: RuntimeThreadSummary[]; snapshotSeq: number }> {
  const threads: RuntimeThreadSummary[] = [];
  const threadIds = new Set<string>();
  const cursors = new Set<string>();
  let cursor: string | undefined;
  let firstSnapshotSeq: number | undefined;
  let previousSnapshotSeq = -1;
  for (let pageNumber = 0; pageNumber < MAX_THREAD_CATALOG_PAGES; pageNumber += 1) {
    const params: Record<string, unknown> = { limit: THREAD_CATALOG_PAGE_LIMIT };
    if (options.archived === true) {
      params.archived = true;
    }
    if (cursor !== undefined) {
      params.cursor = cursor;
    }
    const page = parseRuntimeThreadListPage(
      await requester.request("thread.list", params)
    );
    firstSnapshotSeq ??= page.snapshotSeq;
    if (page.snapshotSeq < previousSnapshotSeq) {
      throw new Error("Runtime thread catalog watermark moved backwards.");
    }
    previousSnapshotSeq = page.snapshotSeq;
    for (const thread of page.threads) {
      if (threadIds.has(thread.id)) {
        throw new Error("Runtime thread catalog returned a duplicate thread.");
      }
      threadIds.add(thread.id);
      threads.push(thread);
    }
    if (!page.hasMore) {
      return { threads, snapshotSeq: firstSnapshotSeq };
    }
    const nextCursor = page.nextCursor as string;
    if (nextCursor === cursor || cursors.has(nextCursor)) {
      throw new Error("Runtime thread catalog cursor did not advance.");
    }
    cursors.add(nextCursor);
    cursor = nextCursor;
  }
  throw new Error("Runtime thread catalog exceeded the page limit.");
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

interface PendingRequest {
  resolve(value: unknown): void;
  reject(error: Error): void;
  timeout: ReturnType<typeof setTimeout>;
}

export interface RuntimeConnectionInfo {
  protocolVersion: number;
  host: string;
  port: number;
  pid: number;
  server: { name: string; version: string };
}

export interface RuntimeHostOptions {
  runtimeRoot: string;
  runtimeHome?: string;
  pythonExecutable?: string;
  stderrSink?: (message: string) => void;
  parentPid?: number;
  startTimeoutMs?: number;
  stopTimeoutMs?: number;
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, message: string): Promise<T> {
  return new Promise<T>((resolvePromise, reject) => {
    const timeout = setTimeout(() => reject(new Error(message)), timeoutMs);
    promise.then(
      (value) => {
        clearTimeout(timeout);
        resolvePromise(value);
      },
      (error: unknown) => {
        clearTimeout(timeout);
        reject(error);
      }
    );
  });
}

function remainingTimeoutMs(deadline: number, message: string): number {
  const remaining = deadline - Date.now();
  if (remaining <= 0) {
    throw new Error(message);
  }
  return remaining;
}

function parseReadyRecord(line: string): RuntimeReadyRecord {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch (error) {
    throw new Error("Runtime readiness output was not valid JSON.", { cause: error });
  }
  if (
    typeof value !== "object" ||
    value === null ||
    (value as Partial<RuntimeReadyRecord>).type !== "ikaros_runtime.ready" ||
    typeof (value as Partial<RuntimeReadyRecord>).protocolVersion !== "number" ||
    typeof (value as Partial<RuntimeReadyRecord>).host !== "string" ||
    typeof (value as Partial<RuntimeReadyRecord>).port !== "number" ||
    typeof (value as Partial<RuntimeReadyRecord>).pid !== "number"
  ) {
    throw new Error("Runtime readiness output did not match the expected schema.");
  }
  return value as RuntimeReadyRecord;
}

function waitForReadiness(child: ChildProcessWithoutNullStreams): Promise<RuntimeReadyRecord> {
  return new Promise((resolvePromise, reject) => {
    const reader = createInterface({ input: child.stdout });
    const cleanup = (): void => {
      reader.removeAllListeners();
      child.off("error", onError);
      child.off("exit", onExit);
      reader.close();
    };
    const onError = (error: Error): void => {
      cleanup();
      reject(error);
    };
    const onExit = (code: number | null): void => {
      cleanup();
      reject(new Error(`Runtime exited before readiness (exit code ${String(code)}).`));
    };

    child.once("error", onError);
    child.once("exit", onExit);
    reader.once("line", (line) => {
      try {
        const ready = parseReadyRecord(line);
        cleanup();
        resolvePromise(ready);
      } catch (error) {
        cleanup();
        reject(error);
      }
    });
  });
}

function openAuthenticatedSocket(
  ready: RuntimeReadyRecord,
  token: string,
  timeoutMs: number,
  timeoutMessage: string
): Promise<WebSocket> {
  return new Promise((resolvePromise, reject) => {
    const socket = new WebSocket(`ws://${ready.host}:${ready.port}`, {
      headers: { Authorization: `Bearer ${token}` }
    });
    const timeout = setTimeout(() => {
      fail(new Error(timeoutMessage));
    }, timeoutMs);
    const cleanup = (): void => {
      clearTimeout(timeout);
      socket.off("open", onOpen);
      socket.off("error", onError);
      socket.off("unexpected-response", onUnexpectedResponse);
    };
    const abortSocket = (): void => {
      const absorbAbortError = (): void => undefined;
      socket.once("error", absorbAbortError);
      socket.once("close", () => socket.off("error", absorbAbortError));
      socket.terminate();
    };
    const fail = (error: Error): void => {
      cleanup();
      abortSocket();
      reject(error);
    };
    const onOpen = (): void => {
      cleanup();
      resolvePromise(socket);
    };
    const onError = (error: Error): void => {
      fail(error);
    };
    const onUnexpectedResponse = (): void => {
      fail(new Error("Runtime rejected the authenticated WebSocket connection."));
    };

    socket.once("open", onOpen);
    socket.once("error", onError);
    socket.once("unexpected-response", onUnexpectedResponse);
  });
}

class JsonRpcConnection {
  private readonly pending = new Map<number, PendingRequest>();
  private nextRequestId = 1;
  private disconnected = false;

  constructor(
    private readonly socket: WebSocket,
    private readonly onNotification: (notification: RuntimeNotification) => void,
    private readonly onDisconnected: (error: Error) => void
  ) {
    socket.on("message", this.handleMessage);
    socket.on("error", this.handleError);
    socket.on("close", this.handleClose);
  }

  get isOpen(): boolean {
    return this.socket.readyState === WebSocket.OPEN;
  }

  request<TResult>(
    method: string,
    params: Record<string, unknown>,
    timeoutMs: number
  ): Promise<TResult> {
    if (!this.isOpen) {
      return Promise.reject(new Error("Runtime WebSocket is not open."));
    }
    const id = this.nextRequestId++;
    return new Promise<TResult>((resolvePromise, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Runtime request ${method} timed out.`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (value) => resolvePromise(value as TResult),
        reject,
        timeout
      });
      this.socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }), (error) => {
        if (!error) {
          return;
        }
        const pending = this.pending.get(id);
        if (pending) {
          clearTimeout(pending.timeout);
          this.pending.delete(id);
          pending.reject(error);
        }
      });
    });
  }

  close(): void {
    if (this.disconnected) {
      return;
    }
    this.disconnected = true;
    this.rejectAll(new Error("Runtime connection closed."));
    this.removeSocketListeners();
    this.socket.close();
  }

  private readonly handleMessage = (raw: RawData): void => {
    let message: unknown;
    try {
      message = JSON.parse(raw.toString()) as unknown;
    } catch {
      this.disconnect(new Error("Runtime returned invalid JSON."));
      this.socket.terminate();
      return;
    }
    if (isRuntimeNotification(message)) {
      try {
        this.onNotification(message);
      } catch (error) {
        this.disconnect(error instanceof Error ? error : new Error(String(error)));
        this.socket.terminate();
      }
      return;
    }
    const id = responseId(message);
    if (id === undefined) {
      return;
    }
    const pending = this.pending.get(id);
    if (!pending) {
      return;
    }
    let response: JsonRpcResponse;
    try {
      response = parseRuntimeJsonRpcResponse(message);
    } catch (error) {
      this.disconnect(error instanceof Error ? error : new Error(String(error)));
      this.socket.terminate();
      return;
    }
    clearTimeout(pending.timeout);
    this.pending.delete(id);
    if ("error" in response) {
      pending.reject(new RuntimeRpcError(response.error.code, response.error.message));
    } else {
      pending.resolve(response.result);
    }
  };

  private readonly handleError = (error: Error): void => {
    this.disconnect(error);
  };

  private readonly handleClose = (): void => {
    this.disconnect(new Error("Runtime WebSocket closed."));
  };

  private disconnect(error: Error): void {
    if (this.disconnected) {
      return;
    }
    this.disconnected = true;
    this.rejectAll(error);
    this.removeSocketListeners();
    this.onDisconnected(error);
  }

  private removeSocketListeners(): void {
    this.socket.off("message", this.handleMessage);
    this.socket.off("error", this.handleError);
    this.socket.off("close", this.handleClose);
  }

  private rejectAll(error: Error): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(error);
    }
    this.pending.clear();
  }
}

function runtimePython(runtimeRoot: string): string {
  const relative = process.platform === "win32" ? [".venv", "Scripts", "python.exe"] : [
    ".venv",
    "bin",
    "python"
  ];
  return resolve(runtimeRoot, ...relative);
}

export function developmentRuntimeRoot(desktopAppPath: string): string {
  return resolve(desktopAppPath, "..", "..", "runtime");
}

function terminateRuntimeProcess(
  child: ChildProcessWithoutNullStreams,
  runtimePid: number | undefined
): void {
  if (runtimePid && runtimePid !== child.pid) {
    try {
      process.kill(runtimePid);
    } catch {
      // The Runtime may already have exited between the readiness check and cleanup.
    }
  }
  if (child.exitCode === null) {
    child.kill();
  }
}

export class RuntimeHost {
  private readonly options: Required<
    Pick<RuntimeHostOptions, "parentPid" | "startTimeoutMs" | "stopTimeoutMs">
  > &
    Omit<RuntimeHostOptions, "parentPid" | "startTimeoutMs" | "stopTimeoutMs">;
  private child: ChildProcessWithoutNullStreams | undefined;
  private connection: JsonRpcConnection | undefined;
  private connectionInfo: RuntimeConnectionInfo | undefined;
  private readyRecord: RuntimeReadyRecord | undefined;
  private launchToken: string | undefined;
  private starting: Promise<RuntimeConnectionInfo> | undefined;
  private generation = 0;
  private supervisionEpoch = 0;
  private stopping = false;
  private hasConnected = false;
  private automaticRecoveryEnabled = false;
  private reconnecting: Promise<void> | undefined;
  private restarting: Promise<void> | undefined;
  private stoppingPromise: Promise<void> | undefined;
  private restartFailureCount = 0;
  private restartCircuitOpen = false;
  private synchronizingEvents = false;
  private lastEventSeq = 0;
  private readonly bufferedEventNotifications = new Map<number, RuntimeJournalEvent>();
  private readonly pendingEventNotifications = new Map<number, RuntimeJournalEvent>();
  private readonly notificationListeners = new Set<(notification: RuntimeNotification) => void>();

  constructor(options: RuntimeHostOptions) {
    this.options = {
      ...options,
      parentPid: options.parentPid ?? process.pid,
      startTimeoutMs: options.startTimeoutMs ?? 15_000,
      stopTimeoutMs: options.stopTimeoutMs ?? 5_000
    };
  }

  get isRunning(): boolean {
    return this.child?.exitCode === null;
  }

  get pid(): number | undefined {
    return this.child?.pid;
  }

  onNotification(listener: (notification: RuntimeNotification) => void): () => void {
    this.notificationListeners.add(listener);
    return () => this.notificationListeners.delete(listener);
  }

  start(): Promise<RuntimeConnectionInfo> {
    if (this.stopping) {
      return Promise.reject(new Error("Ikaros Runtime is stopping."));
    }
    if (this.restartCircuitOpen) {
      this.restartCircuitOpen = false;
      this.restartFailureCount = 0;
      this.supervisionEpoch += 1;
    }
    return this.beginStartAttempt();
  }

  private beginStartAttempt(): Promise<RuntimeConnectionInfo> {
    if (this.connection?.isOpen && this.connectionInfo) {
      return Promise.resolve(this.connectionInfo);
    }
    if (!this.starting) {
      const starting = this.ensureConnectedOnce().finally(() => {
        if (this.starting === starting) {
          this.starting = undefined;
        }
      });
      this.starting = starting;
    }
    return this.starting;
  }

  async request<TResult>(
    method: string,
    params: Record<string, unknown> = {}
  ): Promise<TResult> {
    await this.start();
    const connection = this.connection;
    if (!connection?.isOpen) {
      throw new Error("Runtime connection is unavailable.");
    }
    const result = await connection.request<unknown>(method, params, this.options.startTimeoutMs);
    if (method === "event.replay") {
      return parseRuntimeReplayResult(result) as TResult;
    }
    if (method === "thread.list") {
      return parseRuntimeThreadListPage(result) as TResult;
    }
    if (method === "thread.get") {
      return parseRuntimeThreadGetResult(
        result,
        typeof params.threadId === "string" ? params.threadId : undefined
      ) as TResult;
    }
    if (
      method === "thread.rename" ||
      method === "thread.archive" ||
      method === "thread.unarchive"
    ) {
      const expectedEventType =
        method === "thread.rename"
          ? "thread.renamed"
          : method === "thread.archive"
            ? "thread.archived"
            : "thread.unarchived";
      return parseRuntimeThreadMutationResult(
        result,
        typeof params.threadId === "string" ? params.threadId : undefined,
        expectedEventType
      ) as TResult;
    }
    if (method === "turn.list") {
      const expectedScope =
        typeof params.threadId === "string" && typeof params.branchId === "string"
          ? { threadId: params.threadId, branchId: params.branchId }
          : undefined;
      return parseRuntimeTurnListPage(result, expectedScope) as TResult;
    }
    return result as TResult;
  }

  private async ensureConnectedOnce(): Promise<RuntimeConnectionInfo> {
    if (
      this.child?.exitCode === null &&
      this.readyRecord &&
      this.launchToken
    ) {
      return this.connectToRuntime(
        this.readyRecord,
        this.launchToken,
        this.generation
      );
    }
    return this.launchRuntime();
  }

  private async launchRuntime(): Promise<RuntimeConnectionInfo> {
    const pythonExecutable =
      this.options.pythonExecutable ??
      process.env.IKAROS_RUNTIME_PYTHON ??
      runtimePython(this.options.runtimeRoot);
    if (!existsSync(pythonExecutable)) {
      throw new Error(
        `Ikaros Runtime Python was not found at ${pythonExecutable}. Run uv sync --project runtime --locked.`
      );
    }

    const token = randomBytes(32).toString("base64url");
    const generation = this.generation + 1;
    this.generation = generation;
    const childEnvironment: NodeJS.ProcessEnv = {
      ...process.env,
      PYTHONUNBUFFERED: "1",
      PYTHONUTF8: "1"
    };
    if (this.options.runtimeHome) {
      childEnvironment.IKAROS_HOME = this.options.runtimeHome;
    }
    const child = spawn(
      pythonExecutable,
      [
        "-m",
        "ikaros_runtime",
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        `--token=${token}`,
        "--parent-pid",
        String(this.options.parentPid)
      ],
      {
        cwd: this.options.runtimeRoot,
        env: childEnvironment,
        stdio: ["pipe", "pipe", "pipe"],
        windowsHide: true
      }
    );
    this.child = child;
    child.once("exit", () => this.handleChildExit(child, generation));
    child.stderr.on("data", (chunk: Buffer) => {
      const message = `[ikaros-runtime] ${chunk.toString()}`;
      if (this.options.stderrSink) {
        this.options.stderrSink(message);
      } else {
        process.stderr.write(message);
      }
    });

    try {
      const ready = await withTimeout(
        waitForReadiness(child),
        this.options.startTimeoutMs,
        "Timed out waiting for Ikaros Runtime readiness."
      );
      if (ready.protocolVersion !== PROTOCOL_VERSION) {
        throw new Error(
          `Runtime readiness protocol ${ready.protocolVersion} is incompatible with Desktop protocol ${PROTOCOL_VERSION}.`
        );
      }
      if (this.stopping || generation !== this.generation || this.child !== child) {
        throw new Error("Runtime launch was superseded.");
      }
      this.readyRecord = ready;
      this.launchToken = token;
      return await this.connectToRuntime(ready, token, generation);
    } catch (error) {
      if (this.child === child) {
        const runtimePid = this.readyRecord?.pid;
        this.child = undefined;
        this.readyRecord = undefined;
        this.launchToken = undefined;
        this.connectionInfo = undefined;
        this.connection?.close();
        this.connection = undefined;
        terminateRuntimeProcess(child, runtimePid);
      }
      throw error;
    }
  }

  private async connectToRuntime(
    ready: RuntimeReadyRecord,
    token: string,
    generation: number
  ): Promise<RuntimeConnectionInfo> {
    this.synchronizingEvents = true;
    this.bufferedEventNotifications.clear();
    let socket: WebSocket;
    try {
      socket = await openAuthenticatedSocket(
        ready,
        token,
        this.options.startTimeoutMs,
        "Timed out connecting to Ikaros Runtime."
      );
    } catch (error) {
      this.synchronizingEvents = false;
      this.bufferedEventNotifications.clear();
      throw error;
    }
    if (
      this.stopping ||
      generation !== this.generation ||
      this.child?.exitCode !== null
    ) {
      this.synchronizingEvents = false;
      this.bufferedEventNotifications.clear();
      socket.close();
      throw new Error("Runtime connection was superseded.");
    }
    let established = false;
    let connection: JsonRpcConnection;
    connection = new JsonRpcConnection(
      socket,
      (notification) => this.handleRuntimeNotification(notification),
      (error) => {
        if (established) {
          this.handleConnectionLoss(connection, generation, error);
        }
      }
    );
    this.connection = connection;
    try {
      const result = await connection.request<{
        protocolVersion?: number;
        server?: { name?: string; version?: string };
      }>(
        "initialize",
        {
          protocolVersion: PROTOCOL_VERSION,
          client: { name: "ikaros-desktop", version: "0.1.0" }
        },
        this.options.startTimeoutMs
      );
      if (
        result.protocolVersion !== PROTOCOL_VERSION ||
        typeof result.server?.name !== "string" ||
        typeof result.server.version !== "string"
      ) {
        throw new Error("Runtime initialization result did not match the expected schema.");
      }
      await this.synchronizeEventStream(connection, !this.hasConnected);
      if (
        this.stopping ||
        generation !== this.generation ||
        this.child?.exitCode !== null
      ) {
        throw new Error("Runtime connection was superseded.");
      }
      const connectionInfo: RuntimeConnectionInfo = {
        protocolVersion: result.protocolVersion,
        host: ready.host,
        port: ready.port,
        pid: ready.pid,
        server: { name: result.server.name, version: result.server.version }
      };
      this.connectionInfo = connectionInfo;
      this.hasConnected = true;
      this.automaticRecoveryEnabled = true;
      this.restartFailureCount = 0;
      this.restartCircuitOpen = false;
      established = true;
      return connectionInfo;
    } catch (error) {
      if (this.connection === connection) {
        this.connection = undefined;
        this.connectionInfo = undefined;
      }
      this.synchronizingEvents = false;
      this.bufferedEventNotifications.clear();
      connection.close();
      throw error;
    }
  }

  private async synchronizeEventStream(
    connection: JsonRpcConnection,
    initialConnection: boolean
  ): Promise<void> {
    if (initialConnection) {
      const snapshot = parseRuntimeThreadListPage(
        await connection.request<unknown>(
          "thread.list",
          { limit: 1 },
          this.options.startTimeoutMs
        )
      );
      this.lastEventSeq = snapshot.snapshotSeq;
      for (const seq of this.pendingEventNotifications.keys()) {
        if (seq <= this.lastEventSeq) {
          this.pendingEventNotifications.delete(seq);
        }
      }
    } else {
      let cursor = this.lastEventSeq;
      while (true) {
        const replay = parseRuntimeReplayResult(
          await connection.request<unknown>(
            "event.replay",
            { afterSeq: cursor, limit: 1000 },
            this.options.startTimeoutMs
          )
        );
        for (const event of replay.events) {
          if (event.seq > this.lastEventSeq) {
            this.pendingEventNotifications.set(event.seq, event);
          }
        }
        cursor = replay.nextAfterSeq;
        if (!replay.hasMore) {
          break;
        }
      }
    }

    for (const event of this.bufferedEventNotifications.values()) {
      if (event.seq > this.lastEventSeq) {
        this.pendingEventNotifications.set(event.seq, event);
      }
    }
    this.bufferedEventNotifications.clear();
    this.synchronizingEvents = false;
    this.flushPendingEvents();
  }

  private handleRuntimeNotification(notification: RuntimeNotification): void {
    if (notification.method !== "event") {
      this.emitNotification(notification);
      return;
    }
    const journalEvent = parseRuntimeJournalEvent(notification.params);
    if (this.synchronizingEvents) {
      this.bufferedEventNotifications.set(journalEvent.seq, journalEvent);
      return;
    }
    if (journalEvent.seq <= this.lastEventSeq) {
      return;
    }
    this.pendingEventNotifications.set(journalEvent.seq, journalEvent);
    this.flushPendingEvents();
  }

  private flushPendingEvents(): void {
    while (true) {
      const event = this.pendingEventNotifications.get(this.lastEventSeq + 1);
      if (!event) {
        return;
      }
      this.pendingEventNotifications.delete(event.seq);
      this.lastEventSeq = event.seq;
      this.emitNotification({ jsonrpc: "2.0", method: "event", params: event });
    }
  }

  private emitNotification(notification: RuntimeNotification): void {
    for (const listener of [...this.notificationListeners]) {
      try {
        listener(notification);
      } catch (error) {
        reportRuntimeHostFailure(
          `notification listener failed: ${errorMessage(error)}`
        );
      }
    }
  }

  private handleConnectionLoss(
    connection: JsonRpcConnection,
    generation: number,
    error: Error
  ): void {
    const child = this.child;
    if (
      this.stopping ||
      generation !== this.generation ||
      this.connection !== connection ||
      !child ||
      child.exitCode !== null
    ) {
      return;
    }
    this.connection = undefined;
    this.connectionInfo = undefined;
    this.scheduleSocketReconnect(child, generation, error);
  }

  private scheduleSocketReconnect(
    child: ChildProcessWithoutNullStreams,
    generation: number,
    cause: Error
  ): void {
    if (this.reconnecting || !this.canReconnectSocket(child, generation)) {
      return;
    }
    const supervisionEpoch = this.supervisionEpoch;
    let reconnecting: Promise<void>;
    reconnecting = this.reconnectSocketWithBackoff(
      child,
      generation,
      supervisionEpoch,
      cause
    ).finally(() => {
      if (this.reconnecting === reconnecting) {
        this.reconnecting = undefined;
      }
    });
    this.reconnecting = reconnecting;
  }

  private canReconnectSocket(
    child: ChildProcessWithoutNullStreams,
    generation: number,
    supervisionEpoch = this.supervisionEpoch
  ): boolean {
    return (
      this.automaticRecoveryEnabled &&
      !this.stopping &&
      supervisionEpoch === this.supervisionEpoch &&
      generation === this.generation &&
      this.child === child &&
      child.exitCode === null &&
      !this.connection?.isOpen &&
      this.readyRecord !== undefined &&
      this.launchToken !== undefined
    );
  }

  private async reconnectSocketWithBackoff(
    child: ChildProcessWithoutNullStreams,
    generation: number,
    supervisionEpoch: number,
    cause: Error
  ): Promise<void> {
    let lastError: unknown = cause;
    for (const delayMs of SOCKET_RECONNECT_DELAYS_MS) {
      await wait(delayMs);
      if (!this.canReconnectSocket(child, generation, supervisionEpoch)) {
        return;
      }
      try {
        await this.beginStartAttempt();
        return;
      } catch (error) {
        lastError = error;
        if (!this.canReconnectSocket(child, generation, supervisionEpoch)) {
          return;
        }
      }
    }
    if (this.canReconnectSocket(child, generation, supervisionEpoch)) {
      reportRuntimeHostFailure(
        `WebSocket reconnect attempts exhausted: ${errorMessage(lastError)}`
      );
    }
  }

  private handleChildExit(
    child: ChildProcessWithoutNullStreams,
    generation: number
  ): void {
    if (this.child !== child || generation !== this.generation) {
      return;
    }
    this.child = undefined;
    this.readyRecord = undefined;
    this.launchToken = undefined;
    this.connectionInfo = undefined;
    const connection = this.connection;
    this.connection = undefined;
    this.synchronizingEvents = false;
    this.bufferedEventNotifications.clear();
    this.generation += 1;
    connection?.close();
    if (!this.stopping && this.hasConnected && this.automaticRecoveryEnabled) {
      this.scheduleRuntimeRestart();
    }
  }

  private scheduleRuntimeRestart(): void {
    if (
      this.restarting ||
      this.restartCircuitOpen ||
      !this.automaticRecoveryEnabled ||
      this.stopping ||
      this.connection?.isOpen
    ) {
      return;
    }
    const supervisionEpoch = this.supervisionEpoch;
    let restarting: Promise<void>;
    restarting = this.restartRuntimeWithBackoff(supervisionEpoch).finally(() => {
      if (this.restarting === restarting) {
        this.restarting = undefined;
      }
      if (
        this.automaticRecoveryEnabled &&
        !this.stopping &&
        !this.restartCircuitOpen &&
        !this.connection?.isOpen &&
        !this.child
      ) {
        this.scheduleRuntimeRestart();
      }
    });
    this.restarting = restarting;
  }

  private canRestartRuntime(supervisionEpoch: number): boolean {
    return (
      this.automaticRecoveryEnabled &&
      !this.stopping &&
      supervisionEpoch === this.supervisionEpoch &&
      !this.restartCircuitOpen &&
      !this.connection?.isOpen
    );
  }

  private async restartRuntimeWithBackoff(supervisionEpoch: number): Promise<void> {
    while (this.restartFailureCount < RUNTIME_RESTART_DELAYS_MS.length) {
      if (!this.canRestartRuntime(supervisionEpoch)) {
        return;
      }
      await wait(RUNTIME_RESTART_DELAYS_MS[this.restartFailureCount] ?? 400);
      if (!this.canRestartRuntime(supervisionEpoch)) {
        return;
      }
      try {
        await this.beginStartAttempt();
        return;
      } catch (error) {
        if (!this.canRestartRuntime(supervisionEpoch)) {
          return;
        }
        this.restartFailureCount += 1;
        reportRuntimeHostFailure(
          `restart attempt ${this.restartFailureCount} failed: ${errorMessage(error)}`
        );
      }
    }
    if (this.canRestartRuntime(supervisionEpoch)) {
      this.restartCircuitOpen = true;
      reportRuntimeHostFailure(
        "automatic restart circuit opened after repeated readiness failures."
      );
    }
  }

  private async openShutdownConnection(
    ready: RuntimeReadyRecord,
    token: string,
    deadline: number
  ): Promise<JsonRpcConnection> {
    const socket = await openAuthenticatedSocket(
      ready,
      token,
      remainingTimeoutMs(deadline, "Timed out reconnecting for Runtime shutdown."),
      "Timed out reconnecting for Runtime shutdown."
    );
    const connection = new JsonRpcConnection(socket, () => undefined, () => undefined);
    try {
      const result = await connection.request<{
        protocolVersion?: number;
        server?: { name?: string; version?: string };
      }>(
        "initialize",
        {
          protocolVersion: PROTOCOL_VERSION,
          client: { name: "ikaros-desktop-shutdown", version: "0.1.0" }
        },
        remainingTimeoutMs(deadline, "Timed out initializing Runtime shutdown connection.")
      );
      if (
        result.protocolVersion !== PROTOCOL_VERSION ||
        typeof result.server?.name !== "string" ||
        typeof result.server.version !== "string"
      ) {
        throw new Error("Runtime shutdown initialization did not match the expected schema.");
      }
      return connection;
    } catch (error) {
      connection.close();
      throw error;
    }
  }

  private async requestRuntimeShutdown(
    connection: JsonRpcConnection,
    deadline: number
  ): Promise<void> {
    const result = await connection.request<{ accepted?: boolean }>(
      "runtime.shutdown",
      {},
      remainingTimeoutMs(deadline, "Timed out requesting Runtime shutdown.")
    );
    if (result.accepted !== true) {
      throw new Error("Runtime did not accept the shutdown request.");
    }
  }

  private async settleStoppedTasks(
    tasks: Array<Promise<unknown> | undefined>,
    deadline: number
  ): Promise<void> {
    const pending = tasks.filter((task): task is Promise<unknown> => task !== undefined);
    if (pending.length === 0) {
      return;
    }
    let timeoutMs: number;
    try {
      timeoutMs = remainingTimeoutMs(deadline, "Timed out settling Runtime supervision tasks.");
    } catch {
      return;
    }
    await withTimeout(
      Promise.allSettled(pending).then(() => undefined),
      timeoutMs,
      "Timed out settling Runtime supervision tasks."
    ).catch(() => undefined);
  }

  stop(): Promise<void> {
    if (this.stoppingPromise) {
      return this.stoppingPromise;
    }
    this.stopping = true;
    let stoppingPromise: Promise<void>;
    stoppingPromise = this.stopOnce().finally(() => {
      if (this.stoppingPromise === stoppingPromise) {
        this.stoppingPromise = undefined;
      }
      this.stopping = false;
    });
    this.stoppingPromise = stoppingPromise;
    return stoppingPromise;
  }

  private async stopOnce(): Promise<void> {
    const deadline = Date.now() + this.options.stopTimeoutMs;
    this.automaticRecoveryEnabled = false;
    this.supervisionEpoch += 1;
    this.generation += 1;
    this.restartFailureCount = 0;
    this.restartCircuitOpen = false;
    const pendingTasks = [this.starting, this.reconnecting, this.restarting];
    this.starting = undefined;
    this.reconnecting = undefined;
    this.restarting = undefined;
    const child = this.child;
    const connection = this.connection;
    const ready = this.readyRecord;
    const token = this.launchToken;
    const runtimePid = ready?.pid;
    this.connectionInfo = undefined;
    this.connection = undefined;
    this.child = undefined;
    this.readyRecord = undefined;
    this.launchToken = undefined;
    this.synchronizingEvents = false;
    this.bufferedEventNotifications.clear();
    this.pendingEventNotifications.clear();
    let shutdownConnection: JsonRpcConnection | undefined;
    try {
      if (!child) {
        connection?.close();
        await this.settleStoppedTasks(pendingTasks, deadline);
        return;
      }

      try {
        if (connection?.isOpen) {
          shutdownConnection = connection;
          try {
            await this.requestRuntimeShutdown(shutdownConnection, deadline);
          } catch (error) {
            shutdownConnection.close();
            shutdownConnection = undefined;
            if (!ready || !token || child.exitCode !== null) {
              throw error;
            }
            shutdownConnection = await this.openShutdownConnection(ready, token, deadline);
            await this.requestRuntimeShutdown(shutdownConnection, deadline);
          }
        } else {
          if (!ready || !token || child.exitCode !== null) {
            throw new Error("Runtime shutdown connection details are unavailable.");
          }
          shutdownConnection = await this.openShutdownConnection(ready, token, deadline);
          await this.requestRuntimeShutdown(shutdownConnection, deadline);
        }
      } catch {
        terminateRuntimeProcess(child, runtimePid);
      } finally {
        connection?.close();
        if (shutdownConnection !== connection) {
          shutdownConnection?.close();
        }
      }

      if (child.exitCode === null) {
        try {
          const timeoutMs = remainingTimeoutMs(
            deadline,
            "Timed out waiting for Runtime process exit."
          );
          await withTimeout(
            new Promise<void>((resolvePromise) => child.once("exit", () => resolvePromise())),
            timeoutMs,
            "Timed out waiting for Runtime process exit."
          );
        } catch {
          terminateRuntimeProcess(child, runtimePid);
        }
      }
      await this.settleStoppedTasks(pendingTasks, deadline);
    } finally {
      if (child?.exitCode === null && Date.now() >= deadline) {
        terminateRuntimeProcess(child, runtimePid);
      }
    }
  }
}
