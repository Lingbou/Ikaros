import type {
  IkarosRuntimeApi,
  RuntimeThreadSummary,
  RuntimeTurnHistory,
} from "../shared/runtime";

export const TURN_HISTORY_PAGE_LIMIT = 25;

type RuntimeHistoryReader = Pick<IkarosRuntimeApi, "getThread" | "listTurns">;

export interface LoadedRuntimeThreadHistory {
  thread: RuntimeThreadSummary;
  turns: LoadedRuntimeTurnHistory[];
  nextCursor: string | null;
  hasMore: boolean;
  metadataSnapshotSeq: number;
  historySnapshotSeq: number;
  latestSnapshotSeq: number;
}

export interface LoadedRuntimeTurnHistory {
  turn: RuntimeTurnHistory;
  snapshotSeq: number;
}

export interface OlderRuntimeTurnPageRequest {
  threadId: string;
  branchId: string;
  cursor: string;
  previousSnapshotSeq: number;
  existingTurnIds: ReadonlySet<string>;
  existingOrdinals: ReadonlySet<number>;
}

export interface LoadedOlderRuntimeTurnPage {
  turns: LoadedRuntimeTurnHistory[];
  nextCursor: string | null;
  hasMore: boolean;
  snapshotSeq: number;
}

function validateTurnPageScope(
  turns: readonly RuntimeTurnHistory[],
  threadId: string,
  branchId: string,
  existingTurnIds: ReadonlySet<string> = new Set(),
  existingOrdinals: ReadonlySet<number> = new Set(),
): void {
  const pageTurnIds = new Set<string>();
  const pageOrdinals = new Set<number>();
  const oldestLoadedOrdinal =
    existingOrdinals.size === 0 ? null : Math.min(...existingOrdinals);
  for (const turn of turns) {
    if (turn.threadId !== threadId || turn.branchId !== branchId) {
      throw new Error("Runtime Thread history escaped its requested scope.");
    }
    if (
      pageTurnIds.has(turn.id) ||
      pageOrdinals.has(turn.ordinal) ||
      existingTurnIds.has(turn.id) ||
      existingOrdinals.has(turn.ordinal)
    ) {
      throw new Error("Runtime Thread history returned a duplicate Turn.");
    }
    if (oldestLoadedOrdinal !== null && turn.ordinal >= oldestLoadedOrdinal) {
      throw new Error("Runtime Thread history returned an overlapping Turn page.");
    }
    pageTurnIds.add(turn.id);
    pageOrdinals.add(turn.ordinal);
  }
}

export async function loadRuntimeThreadHistory(
  reader: RuntimeHistoryReader,
  threadId: string,
): Promise<LoadedRuntimeThreadHistory> {
  const metadata = await reader.getThread(threadId);
  const page = await reader.listTurns({
    threadId,
    branchId: metadata.thread.defaultBranchId,
    limit: TURN_HISTORY_PAGE_LIMIT,
  });
  if (page.snapshotSeq < metadata.snapshotSeq) {
    throw new Error("Runtime Thread history watermark moved backwards.");
  }
  validateTurnPageScope(page.turns, threadId, metadata.thread.defaultBranchId);
  return {
    thread: metadata.thread,
    turns: page.turns.map((turn) => ({ turn, snapshotSeq: page.snapshotSeq })),
    nextCursor: page.nextCursor,
    hasMore: page.hasMore,
    metadataSnapshotSeq: metadata.snapshotSeq,
    historySnapshotSeq: page.snapshotSeq,
    latestSnapshotSeq: Math.max(metadata.snapshotSeq, page.snapshotSeq),
  };
}

export async function loadOlderRuntimeTurnPage(
  reader: Pick<IkarosRuntimeApi, "listTurns">,
  request: OlderRuntimeTurnPageRequest,
): Promise<LoadedOlderRuntimeTurnPage> {
  const page = await reader.listTurns({
    threadId: request.threadId,
    branchId: request.branchId,
    cursor: request.cursor,
    limit: TURN_HISTORY_PAGE_LIMIT,
  });
  if (page.snapshotSeq < request.previousSnapshotSeq) {
    throw new Error("Runtime Thread history watermark moved backwards.");
  }
  validateTurnPageScope(
    page.turns,
    request.threadId,
    request.branchId,
    request.existingTurnIds,
    request.existingOrdinals,
  );
  if (page.hasMore && page.nextCursor === request.cursor) {
    throw new Error("Runtime Thread history cursor did not advance.");
  }
  return {
    turns: page.turns.map((turn) => ({ turn, snapshotSeq: page.snapshotSeq })),
    nextCursor: page.nextCursor,
    hasMore: page.hasMore,
    snapshotSeq: page.snapshotSeq,
  };
}
