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
}

export interface RuntimeTurnStartResult {
  threadId: string;
  branchId: string;
  turnId: string;
  runId: string;
}

export interface RuntimeReplayResult {
  events: RuntimeJournalEvent[];
  latestSeq: number;
  nextAfterSeq: number;
  hasMore: boolean;
}

export interface IkarosRuntimeApi {
  listThreads(): Promise<{ threads: RuntimeThreadSummary[] }>;
  createThread(title: string | null): Promise<RuntimeThreadCreateResult>;
  startTurn(params: RuntimeTurnStartParams): Promise<RuntimeTurnStartResult>;
  replayEvents(afterSeq: number, limit?: number): Promise<RuntimeReplayResult>;
  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void;
}
