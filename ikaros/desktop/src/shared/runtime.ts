export interface RuntimeThreadSummary {
  id: string;
  title: string | null;
  defaultBranchId: string;
  createdAt: string;
  updatedAt: string;
}

export interface RuntimeJournalEvent {
  seq: number;
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

export interface RuntimeRpcFailure {
  kind: "json_rpc";
  code: number;
  message: string;
}

export type RuntimeInvocationResult<TResult> =
  | { ok: true; value: TResult }
  | { ok: false; error: RuntimeRpcFailure };

export interface IkarosRuntimeApi {
  listThreads(): Promise<{ threads: RuntimeThreadSummary[] }>;
  createThread(
    title: string | null,
    clientRequestId?: string
  ): Promise<RuntimeThreadCreateResult>;
  startTurn(params: RuntimeTurnStartParams): Promise<RuntimeTurnStartResult>;
  cancelRun(runId: string): Promise<RuntimeCancelRunResult>;
  replayEvents(afterSeq: number, limit?: number): Promise<RuntimeReplayResult>;
  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void;
}

export interface IkarosRuntimeBridgeApi {
  listThreads(): Promise<RuntimeInvocationResult<{ threads: RuntimeThreadSummary[] }>>;
  createThread(
    title: string | null,
    clientRequestId?: string
  ): Promise<RuntimeInvocationResult<RuntimeThreadCreateResult>>;
  startTurn(
    params: RuntimeTurnStartParams
  ): Promise<RuntimeInvocationResult<RuntimeTurnStartResult>>;
  cancelRun(runId: string): Promise<RuntimeInvocationResult<RuntimeCancelRunResult>>;
  replayEvents(
    afterSeq: number,
    limit?: number
  ): Promise<RuntimeInvocationResult<RuntimeReplayResult>>;
  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void;
}
