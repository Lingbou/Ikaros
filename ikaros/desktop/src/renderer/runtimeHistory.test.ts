import { describe, expect, it, vi } from "vitest";

import type { IkarosRuntimeApi, RuntimeTurnHistory } from "../shared/runtime";
import { loadRuntimeThreadHistory } from "./runtimeHistory";

const thread = {
  id: "thread-1",
  title: "History",
  defaultBranchId: "branch-1",
  workspace: null,
  createdAt: "2026-08-14T00:00:00.000Z",
  updatedAt: "2026-08-14T00:00:00.000Z",
};

function turn(ordinal: number): RuntimeTurnHistory {
  return {
    id: `turn-${ordinal}`,
    threadId: thread.id,
    branchId: thread.defaultBranchId,
    ordinal,
    status: "completed",
    createdAt: `2026-08-14T00:00:0${ordinal}.000Z`,
    updatedAt: `2026-08-14T00:00:0${ordinal}.000Z`,
    runs: [],
  };
}

function reader(
  listTurns: IkarosRuntimeApi["listTurns"],
): Pick<IkarosRuntimeApi, "getThread" | "listTurns"> {
  return {
    getThread: vi.fn(async () => ({ thread, snapshotSeq: 10 })),
    listTurns,
  };
}

describe("Runtime Thread history loader", () => {
  it("loads every page and returns Turns in canonical ordinal order", async () => {
    const listTurns = vi
      .fn<IkarosRuntimeApi["listTurns"]>()
      .mockResolvedValueOnce({
        turns: [turn(3), turn(4)],
        nextCursor: "older_page",
        hasMore: true,
        snapshotSeq: 11,
      })
      .mockResolvedValueOnce({
        turns: [turn(1), turn(2)],
        nextCursor: null,
        hasMore: false,
        snapshotSeq: 12,
      });

    await expect(loadRuntimeThreadHistory(reader(listTurns), thread.id)).resolves.toEqual({
      thread,
      turns: [
        { turn: turn(1), snapshotSeq: 12 },
        { turn: turn(2), snapshotSeq: 12 },
        { turn: turn(3), snapshotSeq: 11 },
        { turn: turn(4), snapshotSeq: 11 },
      ],
      metadataSnapshotSeq: 10,
      historySnapshotSeq: 11,
      latestSnapshotSeq: 12,
    });
    expect(listTurns).toHaveBeenNthCalledWith(1, {
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      limit: 100,
    });
    expect(listTurns).toHaveBeenNthCalledWith(2, {
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      cursor: "older_page",
      limit: 100,
    });
  });

  it("rejects a page that escapes the Thread scope or repeats a Turn", async () => {
    const escaped = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [{ ...turn(1), branchId: "branch-other" }],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 10,
    });
    await expect(loadRuntimeThreadHistory(reader(escaped), thread.id)).rejects.toThrow(
      "escaped its requested scope",
    );

    const duplicated = vi
      .fn<IkarosRuntimeApi["listTurns"]>()
      .mockResolvedValueOnce({
        turns: [turn(2)],
        nextCursor: "older_page",
        hasMore: true,
        snapshotSeq: 10,
      })
      .mockResolvedValueOnce({
        turns: [turn(2)],
        nextCursor: null,
        hasMore: false,
        snapshotSeq: 10,
      });
    await expect(loadRuntimeThreadHistory(reader(duplicated), thread.id)).rejects.toThrow(
      "duplicate Turn",
    );
  });

  it("rejects backward watermarks and non-advancing cursors", async () => {
    const backward = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 9,
    });
    await expect(loadRuntimeThreadHistory(reader(backward), thread.id)).rejects.toThrow(
      "watermark moved backwards",
    );

    const stuck = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [],
      nextCursor: "same_cursor",
      hasMore: true,
      snapshotSeq: 10,
    });
    const stuckReader = reader(stuck);
    const pending = loadRuntimeThreadHistory(stuckReader, thread.id);
    await expect(pending).rejects.toThrow("cursor did not advance");
    expect(stuck).toHaveBeenCalledTimes(2);
  });
});
