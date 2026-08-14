import type {
  IkarosRuntimeApi,
  RuntimeThreadSummary,
  RuntimeTurnHistory,
} from "../shared/runtime";

const TURN_HISTORY_PAGE_LIMIT = 100;
const MAX_TURN_HISTORY_PAGES = 10_000;

type RuntimeHistoryReader = Pick<IkarosRuntimeApi, "getThread" | "listTurns">;

export interface LoadedRuntimeThreadHistory {
  thread: RuntimeThreadSummary;
  turns: LoadedRuntimeTurnHistory[];
  metadataSnapshotSeq: number;
  historySnapshotSeq: number;
  latestSnapshotSeq: number;
}

export interface LoadedRuntimeTurnHistory {
  turn: RuntimeTurnHistory;
  snapshotSeq: number;
}

export async function loadRuntimeThreadHistory(
  reader: RuntimeHistoryReader,
  threadId: string,
): Promise<LoadedRuntimeThreadHistory> {
  const metadata = await reader.getThread(threadId);
  const turns: LoadedRuntimeTurnHistory[] = [];
  const turnIds = new Set<string>();
  const ordinals = new Set<number>();
  const cursors = new Set<string>();
  let cursor: string | undefined;
  let previousSnapshotSeq = metadata.snapshotSeq;
  let historySnapshotSeq: number | undefined;

  for (let pageNumber = 0; pageNumber < MAX_TURN_HISTORY_PAGES; pageNumber += 1) {
    const page = await reader.listTurns({
      threadId,
      branchId: metadata.thread.defaultBranchId,
      limit: TURN_HISTORY_PAGE_LIMIT,
      ...(cursor === undefined ? {} : { cursor }),
    });
    if (page.snapshotSeq < previousSnapshotSeq) {
      throw new Error("Runtime Thread history watermark moved backwards.");
    }
    historySnapshotSeq ??= page.snapshotSeq;
    previousSnapshotSeq = page.snapshotSeq;
    for (const turn of page.turns) {
      if (turn.threadId !== threadId || turn.branchId !== metadata.thread.defaultBranchId) {
        throw new Error("Runtime Thread history escaped its requested scope.");
      }
      if (turnIds.has(turn.id) || ordinals.has(turn.ordinal)) {
        throw new Error("Runtime Thread history returned a duplicate Turn.");
      }
      turnIds.add(turn.id);
      ordinals.add(turn.ordinal);
      turns.push({ turn, snapshotSeq: page.snapshotSeq });
    }
    if (!page.hasMore) {
      return {
        thread: metadata.thread,
        turns: turns.sort((left, right) => left.turn.ordinal - right.turn.ordinal),
        metadataSnapshotSeq: metadata.snapshotSeq,
        historySnapshotSeq: historySnapshotSeq ?? page.snapshotSeq,
        latestSnapshotSeq: Math.max(metadata.snapshotSeq, previousSnapshotSeq),
      };
    }
    const nextCursor = page.nextCursor;
    if (!nextCursor || nextCursor === cursor || cursors.has(nextCursor)) {
      throw new Error("Runtime Thread history cursor did not advance.");
    }
    cursors.add(nextCursor);
    cursor = nextCursor;
  }

  throw new Error("Runtime Thread history exceeded the page limit.");
}
