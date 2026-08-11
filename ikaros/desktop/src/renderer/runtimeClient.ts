import type {
  IkarosRuntimeApi,
  RuntimeJournalEvent,
  RuntimeReplayResult,
  RuntimeThreadCreateResult,
  RuntimeThreadSummary,
  RuntimeTurnStartParams,
  RuntimeTurnStartResult
} from "../shared/runtime";

export class RuntimeClient {
  constructor(private readonly api: IkarosRuntimeApi) {}

  listThreads(): Promise<{ threads: RuntimeThreadSummary[] }> {
    return this.api.listThreads();
  }

  createThread(title: string | null): Promise<RuntimeThreadCreateResult> {
    return this.api.createThread(title);
  }

  startTurn(params: RuntimeTurnStartParams): Promise<RuntimeTurnStartResult> {
    return this.api.startTurn(params);
  }

  replayEvents(afterSeq: number, limit = 500): Promise<RuntimeReplayResult> {
    return this.api.replayEvents(afterSeq, limit);
  }

  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void {
    return this.api.onEvent(listener);
  }
}

export function createRuntimeClient(): RuntimeClient | null {
  if (typeof window === "undefined") {
    return null;
  }
  const desktop = (
    window as Window & { ikarosDesktop?: { runtime?: IkarosRuntimeApi } }
  ).ikarosDesktop;
  return desktop?.runtime ? new RuntimeClient(desktop.runtime) : null;
}
