import { describe, expect, it } from "vitest";

import type { RuntimeJournalEvent, RuntimeThreadSummary } from "../shared/runtime";
import { projectRuntimeThreads, replayRuntimeEvents } from "./runtimeProjection";

const summary: RuntimeThreadSummary = {
  id: "thread-1",
  title: "Runtime chat",
  defaultBranchId: "branch-1",
  createdAt: "2026-08-11T12:00:00.000Z",
  updatedAt: "2026-08-11T12:00:00.000Z"
};

function event(
  seq: number,
  type: string,
  turnId: string,
  runId: string,
  itemId: string | null,
  payload: Record<string, unknown>
): RuntimeJournalEvent {
  return {
    seq,
    type,
    threadId: summary.id,
    branchId: summary.defaultBranchId,
    turnId,
    runId,
    itemId,
    timestamp: `2026-08-11T12:00:${String(seq).padStart(2, "0")}.000Z`,
    payload
  };
}

function messageItem(
  id: string,
  turnId: string,
  runId: string,
  role: "user" | "assistant",
  content: string,
  status: string
) {
  return {
    id,
    turnId,
    runId,
    ordinal: role === "user" ? 1 : 2,
    kind: "message",
    role,
    status,
    content,
    createdAt: "2026-08-11T12:00:00.000Z",
    updatedAt: "2026-08-11T12:00:00.000Z"
  };
}

describe("Runtime event projection", () => {
  it("reduces streamed Items and settled Runs into the existing conversation UI model", () => {
    const events = [
      event(2, "item.completed", "turn-1", "run-1", "user-1", {
        item: messageItem("user-1", "turn-1", "run-1", "user", "hello", "completed")
      }),
      event(3, "run.state_changed", "turn-1", "run-1", null, { status: "queued" }),
      event(4, "run.state_changed", "turn-1", "run-1", null, { status: "running" }),
      event(5, "item.started", "turn-1", "run-1", "assistant-1", {
        item: messageItem("assistant-1", "turn-1", "run-1", "assistant", "", "streaming")
      }),
      event(6, "item.delta", "turn-1", "run-1", "assistant-1", { delta: "Hello " }),
      event(7, "item.delta", "turn-1", "run-1", "assistant-1", { delta: "world" }),
      event(8, "item.completed", "turn-1", "run-1", "assistant-1", {
        item: messageItem(
          "assistant-1",
          "turn-1",
          "run-1",
          "assistant",
          "Hello world",
          "completed"
        )
      }),
      event(9, "run.settled", "turn-1", "run-1", null, { status: "completed" }),
      event(10, "item.completed", "turn-2", "run-2", "user-2", {
        item: messageItem("user-2", "turn-2", "run-2", "user", "again", "completed")
      }),
      event(11, "run.settled", "turn-2", "run-2", null, { status: "completed" }),
      event(12, "item.started", "turn-3", "run-3", "assistant-3", {
        item: messageItem("assistant-3", "turn-3", "run-3", "assistant", "", "streaming")
      }),
      event(13, "item.completed", "turn-3", "run-3", "assistant-3", {
        item: messageItem("assistant-3", "turn-3", "run-3", "assistant", "partial", "failed")
      }),
      event(14, "run.settled", "turn-3", "run-3", null, { status: "failed" }),
      event(15, "item.started", "turn-4", "run-4", "assistant-4", {
        item: messageItem("assistant-4", "turn-4", "run-4", "assistant", "", "streaming")
      }),
      event(16, "item.delta", "turn-4", "run-4", "assistant-4", { delta: "partial" }),
      event(17, "item.completed", "turn-4", "run-4", "assistant-4", {
        item: messageItem("assistant-4", "turn-4", "run-4", "assistant", "partial", "cancelled")
      }),
      event(18, "run.settled", "turn-4", "run-4", null, { status: "cancelled" })
    ];

    const [thread] = replayRuntimeEvents(projectRuntimeThreads([summary]), events);
    const turns = thread.branches[0]?.turns;

    expect(turns).toHaveLength(4);
    expect(turns?.[0]).toMatchObject({
      id: "turn-1",
      runId: "run-1",
      status: "completed",
      events: [
        { id: "user-1", role: "user", content: "hello", status: "complete" },
        {
          id: "assistant-1",
          role: "assistant",
          content: "Hello world",
          status: "complete"
        }
      ]
    });
    expect(turns?.[1]).toMatchObject({
      id: "turn-2",
      runId: "run-2",
      status: "completed",
      events: [{ id: "user-2", role: "user", content: "again" }]
    });
    expect(turns?.[2]).toMatchObject({
      id: "turn-3",
      runId: "run-3",
      status: "failed",
      events: [{ id: "assistant-3", content: "partial", status: "failed" }]
    });
    expect(turns?.[3]).toMatchObject({
      id: "turn-4",
      runId: "run-4",
      status: "interrupted",
      events: [{ id: "assistant-4", content: "partial", status: "interrupted" }]
    });
  });
});
