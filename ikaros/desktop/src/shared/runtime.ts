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

export const RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION = 1 as const;

export interface RuntimeJournalEvent {
  seq: number;
  schemaVersion: typeof RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION;
  type: string;
  threadId: string | null;
  branchId: string | null;
  turnId: string | null;
  runId: string | null;
  itemId: string | null;
  timestamp: string;
  payload: Record<string, unknown>;
}

export interface RuntimeThreadCreateResult {
  thread: RuntimeThreadSummary;
  event: RuntimeJournalEvent;
}

export interface RuntimeThreadListParams {
  cursor?: string;
  limit?: number;
  archived?: boolean;
}

export interface RuntimeThreadCatalogParams {
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
  createdAt: string;
  settledAt: string | null;
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
  providerId: string;
  id: string;
  displayName: string;
  enabled: boolean;
}

export interface RuntimeModelInput {
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

export interface RuntimeRpcFailure {
  kind: "json_rpc";
  code: number;
  message: string;
}

export type RuntimeInvocationResult<TResult> =
  | { ok: true; value: TResult }
  | { ok: false; error: RuntimeRpcFailure };

export interface IkarosRuntimeApi {
  listThreads(
    params?: RuntimeThreadCatalogParams
  ): Promise<{ threads: RuntimeThreadSummary[]; snapshotSeq: number }>;
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
  readUsage(): Promise<RuntimeUsageReadResult>;
  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void;
}

export interface IkarosRuntimeBridgeApi {
  listThreads(params?: RuntimeThreadCatalogParams): Promise<
    RuntimeInvocationResult<{ threads: RuntimeThreadSummary[]; snapshotSeq: number }>
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
  readUsage(): Promise<RuntimeInvocationResult<RuntimeUsageReadResult>>;
  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void;
}
