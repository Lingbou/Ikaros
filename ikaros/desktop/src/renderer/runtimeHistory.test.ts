import { describe, expect, it, vi } from "vitest";

import type { IkarosRuntimeApi, RuntimeTurnHistory } from "../shared/runtime";
import {
  loadOlderRuntimeTurnPage,
  loadRuntimeThreadHistory,
} from "./runtimeHistory";

const thread = {
  id: "thread-1",
  title: "History",
  defaultBranchId: "branch-1",
  workspace: null,
  createdAt: "2026-08-14T00:00:00.000Z",
  updatedAt: "2026-08-14T00:00:00.000Z",
  archivedAt: null,
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
  it("loads only the newest Turn page and exposes its continuation", async () => {
    const listTurns = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [turn(3), turn(4)],
      nextCursor: "older_page",
      hasMore: true,
      snapshotSeq: 11,
    });

    await expect(loadRuntimeThreadHistory(reader(listTurns), thread.id)).resolves.toEqual({
      thread,
      turns: [
        { turn: turn(3), snapshotSeq: 11 },
        { turn: turn(4), snapshotSeq: 11 },
      ],
      nextCursor: "older_page",
      hasMore: true,
      metadataSnapshotSeq: 10,
      historySnapshotSeq: 11,
      latestSnapshotSeq: 11,
    });
    expect(listTurns).toHaveBeenCalledOnce();
    expect(listTurns).toHaveBeenCalledWith({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      limit: 25,
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

    const duplicated = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [turn(2), { ...turn(2), id: "turn-other" }],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 10,
    });
    await expect(loadRuntimeThreadHistory(reader(duplicated), thread.id)).rejects.toThrow(
      "duplicate Turn",
    );
  });

  it("rejects a backward initial history watermark", async () => {
    const backward = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 9,
    });
    await expect(loadRuntimeThreadHistory(reader(backward), thread.id)).rejects.toThrow(
      "watermark moved backwards",
    );

  });

  it("loads one older page and rejects cursor, watermark, or ordinal overlap", async () => {
    const existingTurnIds = new Set(["turn-3", "turn-4"]);
    const existingOrdinals = new Set([3, 4]);
    const request = {
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      cursor: "older_page",
      previousSnapshotSeq: 11,
      existingTurnIds,
      existingOrdinals,
    };
    const listTurns = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [turn(1), turn(2)],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 12,
    });
    await expect(loadOlderRuntimeTurnPage({ listTurns }, request)).resolves.toEqual({
      turns: [
        { turn: turn(1), snapshotSeq: 12 },
        { turn: turn(2), snapshotSeq: 12 },
      ],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 12,
    });
    expect(listTurns).toHaveBeenCalledWith({
      threadId: thread.id,
      branchId: thread.defaultBranchId,
      cursor: "older_page",
      limit: 25,
    });

    const backward = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [turn(2)],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 10,
    });
    await expect(loadOlderRuntimeTurnPage({ listTurns: backward }, request)).rejects.toThrow(
      "watermark moved backwards",
    );

    const overlap = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [turn(3)],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 12,
    });
    await expect(loadOlderRuntimeTurnPage({ listTurns: overlap }, request)).rejects.toThrow(
      "duplicate Turn",
    );

    const stuck = vi.fn<IkarosRuntimeApi["listTurns"]>().mockResolvedValue({
      turns: [turn(2)],
      nextCursor: "older_page",
      hasMore: true,
      snapshotSeq: 12,
    });
    await expect(loadOlderRuntimeTurnPage({ listTurns: stuck }, request)).rejects.toThrow(
      "cursor did not advance",
    );
  });
});
