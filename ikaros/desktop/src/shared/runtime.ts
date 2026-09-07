export interface RuntimeWorkspaceSummary {
  id: string;
  name: string;
  rootUri: string | null;
}

export interface RuntimeThreadSummary {
  id: string;
  title: string | null;
  defaultBranchId: string;
  workspace: RuntimeWorkspaceSummary | null;
  createdAt: string;
  updatedAt: string;
  archivedAt: string | null;
}

export interface RuntimeJournalEvent {
  seq: number;
  schemaVersion: typeof RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION;
  type: RuntimeJournalEventType;
  threadId: string | null;
  branchId: string | null;
  turnId: string | null;
  runId: string | null;
  itemId: string | null;
  timestamp: string;
  payload: Record<string, unknown>;
}

export interface RuntimeSkillDescriptor {
  name: string;
  description: string;
  location: string;
}

export interface RuntimeSkillSummary extends RuntimeSkillDescriptor {
  enabled: boolean;
}

export interface RuntimeSkillDiagnostic {
  entry: string;
  code: string;
  message: string;
}

export interface RuntimeSkillListResult {
  skills: RuntimeSkillSummary[];
  diagnostics: RuntimeSkillDiagnostic[];
}

export interface RuntimeSkillSetEnabledResult {
  skill: RuntimeSkillSummary;
}

export interface RuntimeSkillSetEnabledParams {
  name: string;
  enabled: boolean;
}

export type RuntimeMemoryKind =
  | "fact"
  | "preference"
  | "relationship"
  | "project";

export type RuntimeMemoryState = "active" | "forgotten";

export type RuntimeMemoryScope =
  | { type: "global"; key: null }
  | { type: "workspace"; key: string };

export interface RuntimeMemoryProvenance {
  sourceKind: "user_explicit" | "session_item";
  threadId: string | null;
  turnId: string | null;
  itemId: string | null;
  status: "not_applicable" | "available" | "unavailable";
}

export interface RuntimeMemorySummary {
  id: string;
  kind: RuntimeMemoryKind;
  scope: RuntimeMemoryScope;
  revision: number;
  state: RuntimeMemoryState;
  preview: string | null;
  createdAt: string;
  updatedAt: string;
  forgottenAt: string | null;
}

export interface RuntimeMemoryRecord extends Omit<RuntimeMemorySummary, "preview"> {
  content: string | null;
  provenance: RuntimeMemoryProvenance;
}

export interface RuntimeMemoryCreateParams {
  kind: RuntimeMemoryKind;
  scope: RuntimeMemoryScope;
  content: string;
  clientRequestId: string;
  source?: RuntimeMemorySource;
}

export interface RuntimeMemorySource {
  type: "session_item";
  itemId: string;
}

export interface RuntimeMemoryCorrectParams {
  memoryId: string;
  expectedRevision: number;
  content: string;
  clientRequestId: string;
}

export interface RuntimeMemoryForgetParams {
  memoryId: string;
  expectedRevision: number;
  clientRequestId: string;
}

export interface RuntimeMemoryMutationResult {
  memoryId: string;
  resultingRevision: number;
  created: boolean;
}

export type RuntimeMemoryCreateResult = RuntimeMemoryMutationResult;

export type RuntimeMemoryErrorReasonCode =
  (typeof RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.reasonCodes)[number];

export type RuntimeMemoryRpcMethod =
  | "memory.create"
  | "memory.correct"
  | "memory.forget"
  | "memory.list"
  | "memory.get";

export const RUNTIME_MEMORY_ERROR_REASON_CODES_BY_METHOD = {
  "memory.create": [
    "memory_forgotten",
    "memory_idempotency_conflict",
    "memory_source_unavailable"
  ],
  "memory.correct": [
    "memory_not_found",
    "memory_revision_conflict",
    "memory_forgotten",
    "memory_idempotency_conflict"
  ],
  "memory.forget": [
    "memory_not_found",
    "memory_revision_conflict",
    "memory_forgotten",
    "memory_idempotency_conflict"
  ],
  "memory.list": [],
  "memory.get": ["memory_not_found"]
} as const satisfies Record<
  RuntimeMemoryRpcMethod,
  readonly RuntimeMemoryErrorReasonCode[]
>;

export interface RuntimeMemoryListParams {
  cursor?: string;
  limit?: number;
  scope?: RuntimeMemoryScope;
  kind?: RuntimeMemoryKind;
  state?: RuntimeMemoryState;
}

export interface RuntimeMemoryListPage {
  memories: RuntimeMemorySummary[];
  nextCursor: string | null;
  hasMore: boolean;
}

export interface RuntimeMemoryGetResult {
  memory: RuntimeMemoryRecord;
}

export interface RuntimeInitializeResult {
  protocolVersion: number;
  server: { name: string; version: string };
  capabilities: {
    threads: true;
    turns: true;
    eventReplay: true;
    streaming: true;
    scriptedProvider: true;
    runCancellation: true;
    providers: true;
    models: true;
    usage: true;
    skills: true;
    memory: true;
    tools: readonly string[];
    executionPolicy: "full_access";
  };
}

export type RuntimeHostStatusState =
  | "starting"
  | "connected"
  | "reconnecting"
  | "offline";

export interface RuntimeHostStatus {
  state: RuntimeHostStatusState;
  message: string | null;
}

export interface RuntimeThreadCreateResult {
  thread: RuntimeThreadSummary;
  event: RuntimeJournalEvent;
}

export interface RuntimeThreadCatalogParams {
  cursor?: string;
  limit?: number;
  archived?: boolean;
}

export interface RuntimeThreadListPage {
  threads: RuntimeThreadSummary[];
  nextCursor: string | null;
  hasMore: boolean;
  snapshotSeq: number;
}

export interface RuntimeThreadGetResult {
  thread: RuntimeThreadSummary;
  snapshotSeq: number;
}

export interface RuntimeTurnListParams {
  threadId: string;
  branchId: string;
  cursor?: string;
  limit?: number;
}

export interface RuntimeItemHistory {
  id: string;
  turnId: string;
  runId: string;
  ordinal: number;
  kind: "message" | "tool_call" | "tool_result";
  role: "user" | "assistant" | "tool" | null;
  status: "streaming" | "running" | "completed" | "failed" | "cancelled";
  content: string;
  data: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
}

export interface RuntimeRunHistory {
  id: string;
  turnId: string;
  providerId: string;
  modelId: string;
  executionPolicy: "full_access";
  status: "queued" | "running" | "completed" | "failed" | "cancelled";
  reasonCode: string | null;
  createdAt: string;
  startedAt: string | null;
  settledAt: string | null;
  modelCalls: number;
  items: RuntimeItemHistory[];
}

export interface RuntimeTurnHistory {
  id: string;
  threadId: string;
  branchId: string;
  ordinal: number;
  status: "queued" | "running" | "completed" | "failed" | "cancelled";
  createdAt: string;
  updatedAt: string;
  runs: RuntimeRunHistory[];
}

export interface RuntimeTurnListPage {
  turns: RuntimeTurnHistory[];
  nextCursor: string | null;
  hasMore: boolean;
  snapshotSeq: number;
}

export interface RuntimeThreadCreateParams {
  title: string | null;
  workspace: RuntimeWorkspaceSummary | null;
  clientRequestId?: string;
}

export interface RuntimeThreadRenameParams {
  threadId: string;
  title: string | null;
}

export interface RuntimeThreadMutationResult {
  thread: RuntimeThreadSummary;
  changed: boolean;
  event: RuntimeJournalEvent | null;
}

export interface RuntimeTurnStartParams {
  threadId: string;
  branchId: string;
  content: string;
  providerId: string;
  modelId: string;
  clientRequestId?: string;
}

export interface RuntimeTurnStartResult {
  threadId: string;
  branchId: string;
  turnId: string;
  runId: string;
}

export interface RuntimeCancelRunResult {
  accepted: boolean;
  runId: string;
  status: "queued" | "running" | "completed" | "failed" | "cancelled";
}

export interface RuntimeReplayResult {
  events: RuntimeJournalEvent[];
  latestSeq: number;
  nextAfterSeq: number;
  hasMore: boolean;
}

export interface RuntimeProviderSummary {
  id: string;
  displayName: string;
  origin: "builtin" | "custom";
  configured: boolean;
  credentialConfigured: boolean;
  health: "unknown" | "ready" | "error";
}

export interface RuntimeModelSummary {
  contextWindow: number;
  maxOutputTokens: number;
  providerId: string;
  id: string;
  displayName: string;
  enabled: boolean;
}

export interface RuntimeModelInput {
  contextWindow: number;
  maxOutputTokens: number;
  id: string;
  displayName: string;
}

export type RuntimeDiscoveredModel = RuntimeModelInput;

export interface RuntimeProviderDiscoverModelsParams {
  kind: "deepseek";
  apiKey: string;
}

export interface RuntimeProviderDiscoverModelsResult {
  models: RuntimeDiscoveredModel[];
}

export type RuntimeProviderConfigureParams =
  | {
      kind: "deepseek";
      apiKey: string;
      models: RuntimeModelInput[];
    }
  | {
      kind: "custom";
      providerId: string;
      displayName: string;
      baseUrl: string;
      apiKey?: string;
      headers?: Record<string, string>;
      models: RuntimeModelInput[];
    };

export interface RuntimeProviderConfigureResult {
  provider: RuntimeProviderSummary;
}

export interface RuntimeProviderRemoveResult {
  removed: boolean;
  providerId: string;
}

export interface RuntimeModelSetEnabledParams {
  providerId: string;
  modelId: string;
  enabled: boolean;
}

export interface RuntimeModelSetLimitsParams {
  providerId: string;
  modelId: string;
  contextWindow: number;
  maxOutputTokens: number;
}

export interface RuntimeModelSetEnabledResult {
  model: RuntimeModelSummary;
}

export interface RuntimeUsageSummary {
  lifetimeTokens: number | null;
  peakDailyTokens: number | null;
  longestRunningTurnSec: number | null;
  currentStreakDays: number;
  longestStreakDays: number;
}

export interface RuntimeUsageDailyBucket {
  startDate: string;
  tokens: number;
}

export interface RuntimeUsageReadResult {
  summary: RuntimeUsageSummary;
  dailyUsageBuckets: RuntimeUsageDailyBucket[];
}

export interface RuntimeFilePreviewParams {
  threadId: string;
  path: string;
  sourceToolCallItemId?: string;
  /** First logical line, 1-based; defaults to 1. */
  offset?: number;
  /** Required for later pages; use the first page's opaque revision. */
  expectedRevision?: string;
}

export type RuntimeFilePreviewUnavailableReason =
  | "file_not_found"
  | "not_a_file"
  | "binary_file"
  | "unsupported_encoding"
  | "too_large"
  | "scan_limit"
  | "revision_changed"
  | "read_failed"
  | "protected_content";

export interface RuntimeFilePreviewText {
  threadId: string;
  path: string;
  status: "text";
  /** Opaque file metadata fingerprint, not a digest of the complete content. */
  revision: string;
  encoding: "utf-8";
  bom: boolean;
  content: string;
  lineStart: number;
  lineEnd: number;
  nextOffset: number | null;
  truncated: boolean;
  truncationReason: "byte_limit" | "line_limit" | "scan_limit" | null;
}

export interface RuntimeFilePreviewUnavailable {
  threadId: string;
  path: string | null;
  status: "unavailable";
  reason: RuntimeFilePreviewUnavailableReason;
}

export type RuntimeFilePreviewResult = RuntimeFilePreviewText | RuntimeFilePreviewUnavailable;

export interface RuntimeFileChangeGetParams {
  threadId: string;
  toolCallItemId: string;
}

export interface RuntimeFileRevisionMetadata {
  exists: boolean;
  byteCount: number | null;
  /** SHA256 of captured raw bytes when available. */
  revision: string | null;
  encoding: "utf-8" | null;
  bom: boolean | null;
  newline: "lf" | "crlf" | "mixed" | null;
  lineCount: number | null;
}

export type RuntimeFileChangeUnavailableReason =
  | "not_recorded"
  | "too_large"
  | "binary_file"
  | "unsupported_encoding"
  | "read_failed"
  | "protected_content"
  | "result_unknown";

export interface RuntimeFileChangeRecord {
  threadId: string;
  toolCallItemId: string;
  path: string;
  operation: "write" | "edit";
  recordedAt: string;
  before: RuntimeFileRevisionMetadata;
  after: RuntimeFileRevisionMetadata;
  status: "recorded";
  diff: string;
  additions: number;
  deletions: number;
}

export interface RuntimeFileChangeUnavailable {
  threadId: string;
  toolCallItemId: string;
  path: string | null;
  operation: "write" | "edit";
  recordedAt: string | null;
  before: RuntimeFileRevisionMetadata | null;
  after: RuntimeFileRevisionMetadata | null;
  status: "unavailable";
  reason: RuntimeFileChangeUnavailableReason;
}

export type RuntimeFileChangeResult = RuntimeFileChangeRecord | RuntimeFileChangeUnavailable;

export interface RuntimeRpcFailure {
  kind: "json_rpc";
  code: number;
  message: string;
  reasonCode?: RuntimeMemoryErrorReasonCode;
}

export type RuntimeInvocationResult<TResult> =
  | { ok: true; value: TResult }
  | { ok: false; error: RuntimeRpcFailure };

export interface IkarosRuntimeApi {
  listThreads(
    params?: RuntimeThreadCatalogParams
  ): Promise<RuntimeThreadListPage>;
  getThread(threadId: string): Promise<RuntimeThreadGetResult>;
  listTurns(params: RuntimeTurnListParams): Promise<RuntimeTurnListPage>;
  createThread(params: RuntimeThreadCreateParams): Promise<RuntimeThreadCreateResult>;
  renameThread(params: RuntimeThreadRenameParams): Promise<RuntimeThreadMutationResult>;
  archiveThread(threadId: string): Promise<RuntimeThreadMutationResult>;
  unarchiveThread(threadId: string): Promise<RuntimeThreadMutationResult>;
  startTurn(params: RuntimeTurnStartParams): Promise<RuntimeTurnStartResult>;
  cancelRun(runId: string): Promise<RuntimeCancelRunResult>;
  replayEvents(afterSeq: number, limit?: number): Promise<RuntimeReplayResult>;
  listProviders(): Promise<{ providers: RuntimeProviderSummary[] }>;
  configureProvider(
    params: RuntimeProviderConfigureParams
  ): Promise<RuntimeProviderConfigureResult>;
  discoverProviderModels(
    params: RuntimeProviderDiscoverModelsParams
  ): Promise<RuntimeProviderDiscoverModelsResult>;
  disconnectProvider(providerId: "deepseek"): Promise<RuntimeProviderConfigureResult>;
  removeProvider(providerId: string): Promise<RuntimeProviderRemoveResult>;
  listModels(): Promise<{ models: RuntimeModelSummary[] }>;
  setModelEnabled(
    params: RuntimeModelSetEnabledParams
  ): Promise<RuntimeModelSetEnabledResult>;
  setModelLimits(params: RuntimeModelSetLimitsParams): Promise<RuntimeModelSetEnabledResult>;
  listSkills(): Promise<RuntimeSkillListResult>;
  setSkillEnabled(
    params: RuntimeSkillSetEnabledParams
  ): Promise<RuntimeSkillSetEnabledResult>;
  createMemory(params: RuntimeMemoryCreateParams): Promise<RuntimeMemoryCreateResult>;
  correctMemory(params: RuntimeMemoryCorrectParams): Promise<RuntimeMemoryMutationResult>;
  forgetMemory(params: RuntimeMemoryForgetParams): Promise<RuntimeMemoryMutationResult>;
  listMemories(params?: RuntimeMemoryListParams): Promise<RuntimeMemoryListPage>;
  getMemory(memoryId: string): Promise<RuntimeMemoryGetResult>;
  readUsage(): Promise<RuntimeUsageReadResult>;
  previewFile(params: RuntimeFilePreviewParams): Promise<RuntimeFilePreviewResult>;
  getFileChange(params: RuntimeFileChangeGetParams): Promise<RuntimeFileChangeResult>;
  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void;
  onStatus(listener: (status: RuntimeHostStatus) => void): () => void;
}

export interface IkarosRuntimeBridgeApi {
  listThreads(params?: RuntimeThreadCatalogParams): Promise<
    RuntimeInvocationResult<RuntimeThreadListPage>
  >;
  getThread(threadId: string): Promise<RuntimeInvocationResult<RuntimeThreadGetResult>>;
  listTurns(
    params: RuntimeTurnListParams
  ): Promise<RuntimeInvocationResult<RuntimeTurnListPage>>;
  createThread(
    params: RuntimeThreadCreateParams
  ): Promise<RuntimeInvocationResult<RuntimeThreadCreateResult>>;
  renameThread(
    params: RuntimeThreadRenameParams
  ): Promise<RuntimeInvocationResult<RuntimeThreadMutationResult>>;
  archiveThread(
    threadId: string
  ): Promise<RuntimeInvocationResult<RuntimeThreadMutationResult>>;
  unarchiveThread(
    threadId: string
  ): Promise<RuntimeInvocationResult<RuntimeThreadMutationResult>>;
  startTurn(
    params: RuntimeTurnStartParams
  ): Promise<RuntimeInvocationResult<RuntimeTurnStartResult>>;
  cancelRun(runId: string): Promise<RuntimeInvocationResult<RuntimeCancelRunResult>>;
  replayEvents(
    afterSeq: number,
    limit?: number
  ): Promise<RuntimeInvocationResult<RuntimeReplayResult>>;
  listProviders(): Promise<
    RuntimeInvocationResult<{ providers: RuntimeProviderSummary[] }>
  >;
  configureProvider(
    params: RuntimeProviderConfigureParams
  ): Promise<RuntimeInvocationResult<RuntimeProviderConfigureResult>>;
  discoverProviderModels(
    params: RuntimeProviderDiscoverModelsParams
  ): Promise<RuntimeInvocationResult<RuntimeProviderDiscoverModelsResult>>;
  disconnectProvider(
    providerId: "deepseek"
  ): Promise<RuntimeInvocationResult<RuntimeProviderConfigureResult>>;
  removeProvider(
    providerId: string
  ): Promise<RuntimeInvocationResult<RuntimeProviderRemoveResult>>;
  listModels(): Promise<RuntimeInvocationResult<{ models: RuntimeModelSummary[] }>>;
  setModelEnabled(
    params: RuntimeModelSetEnabledParams
  ): Promise<RuntimeInvocationResult<RuntimeModelSetEnabledResult>>;
  setModelLimits(params: RuntimeModelSetLimitsParams): Promise<RuntimeInvocationResult<RuntimeModelSetEnabledResult>>;
  listSkills(): Promise<RuntimeInvocationResult<RuntimeSkillListResult>>;
  setSkillEnabled(
    params: RuntimeSkillSetEnabledParams
  ): Promise<RuntimeInvocationResult<RuntimeSkillSetEnabledResult>>;
  createMemory(
    params: RuntimeMemoryCreateParams
  ): Promise<RuntimeInvocationResult<RuntimeMemoryCreateResult>>;
  correctMemory(
    params: RuntimeMemoryCorrectParams
  ): Promise<RuntimeInvocationResult<RuntimeMemoryMutationResult>>;
  forgetMemory(
    params: RuntimeMemoryForgetParams
  ): Promise<RuntimeInvocationResult<RuntimeMemoryMutationResult>>;
  listMemories(
    params?: RuntimeMemoryListParams
  ): Promise<RuntimeInvocationResult<RuntimeMemoryListPage>>;
  getMemory(
    memoryId: string
  ): Promise<RuntimeInvocationResult<RuntimeMemoryGetResult>>;
  readUsage(): Promise<RuntimeInvocationResult<RuntimeUsageReadResult>>;
  previewFile(
    params: RuntimeFilePreviewParams
  ): Promise<RuntimeInvocationResult<RuntimeFilePreviewResult>>;
  getFileChange(
    params: RuntimeFileChangeGetParams
  ): Promise<RuntimeInvocationResult<RuntimeFileChangeResult>>;
  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void;
  onStatus?(listener: (status: RuntimeHostStatus) => void): () => void;
}
import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  RUNTIME_PROTOCOL_MANIFEST,
  type RuntimeJournalEventType
} from "./generated/runtimeProtocol";

export {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  RUNTIME_JOURNAL_EVENT_TYPES,
  RUNTIME_PROTOCOL_MANIFEST,
  RUNTIME_PROTOCOL_VERSION,
  RUNTIME_PROVIDER_TOOL_IDS,
  RUNTIME_RPC_METHODS,
  RUNTIME_SERVER_NAME,
  type RuntimeJournalEventType,
  type RuntimeProviderToolId,
  type RuntimeRpcMethod
} from "./generated/runtimeProtocol";
