import type { RuntimeJournalEvent, RuntimeThreadSummary } from "../shared/runtime";
import type { AgentEvent, Thread, Turn, TurnStatus } from "./domain";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function projectRuntimeThread(summary: RuntimeThreadSummary): Thread {
  return {
    id: summary.id,
    projectId: null,
    title: summary.title ?? "",
    activeBranchId: summary.defaultBranchId,
    updatedAt: summary.updatedAt,
    branches: [
      {
        id: summary.defaultBranchId,
        threadId: summary.id,
        label: { source: "app", kind: "main" },
        turns: [],
        createdAt: summary.createdAt
      }
    ]
  };
}

export function projectRuntimeThreads(summaries: RuntimeThreadSummary[]): Thread[] {
  return summaries.map(projectRuntimeThread);
}

function runtimeStatus(status: unknown): TurnStatus | undefined {
  if (status === "queued" || status === "running" || status === "completed" || status === "failed") {
    return status;
  }
  if (status === "cancelled") {
    return "interrupted";
  }
  return undefined;
}

function updateTurn(
  thread: Thread,
  branchId: string,
  turnId: string,
  update: (turn: Turn | undefined) => Turn
): Thread {
  return {
    ...thread,
    branches: thread.branches.map((branch) => {
      if (branch.id !== branchId) {
        return branch;
      }
      const existing = branch.turns.find((turn) => turn.id === turnId);
      const next = update(existing);
      return {
        ...branch,
        turns: existing
          ? branch.turns.map((turn) => (turn.id === turnId ? next : turn))
          : [...branch.turns, next]
      };
    })
  };
}

function upsertMessage(
  thread: Thread,
  event: RuntimeJournalEvent,
  message: AgentEvent
): Thread {
  if (!event.branchId || !event.turnId) {
    return thread;
  }
  return updateTurn(thread, event.branchId, event.turnId, (turn) => {
    const existing = turn?.events.some((candidate) => candidate.id === message.id);
    return {
      id: event.turnId as string,
      branchId: event.branchId as string,
      status: turn?.status ?? "running",
      events: existing
        ? (turn?.events ?? []).map((candidate) =>
            candidate.id === message.id ? message : candidate
          )
        : [...(turn?.events ?? []), message]
    };
  });
}

function projectThreadCreated(
  threads: Thread[],
  event: RuntimeJournalEvent
): Thread[] | undefined {
  if (!isRecord(event.payload.thread)) {
    return undefined;
  }
  const value = event.payload.thread;
  if (
    typeof value.id !== "string" ||
    typeof value.defaultBranchId !== "string" ||
    typeof value.createdAt !== "string" ||
    typeof value.updatedAt !== "string" ||
    (value.title !== null && typeof value.title !== "string")
  ) {
    return undefined;
  }
  const projected = projectRuntimeThread(value as unknown as RuntimeThreadSummary);
  const exists = threads.some((thread) => thread.id === projected.id);
  return exists
    ? threads.map((thread) => (thread.id === projected.id ? { ...projected, branches: thread.branches } : thread))
    : [projected, ...threads];
}

export function applyRuntimeEvent(threads: Thread[], event: RuntimeJournalEvent): Thread[] {
  if (event.type === "thread.created") {
    return projectThreadCreated(threads, event) ?? threads;
  }
  if (!event.threadId || !event.branchId || !event.turnId) {
    return threads;
  }
  const thread = threads.find((candidate) => candidate.id === event.threadId);
  if (!thread) {
    return threads;
  }

  let next = thread;
  if (event.type === "item.started" || event.type === "item.completed") {
    const item = event.payload.item;
    if (isRecord(item) && item.kind === "message" && typeof item.role === "string") {
      if (
        typeof item.id === "string" &&
        typeof item.content === "string" &&
        (item.role === "user" || item.role === "assistant")
      ) {
        next = upsertMessage(thread, event, {
          id: item.id,
          turnId: event.turnId,
          type: "message",
          role: item.role,
          content: item.content,
          status:
            event.type === "item.started"
              ? "streaming"
              : item.status === "failed"
                ? "failed"
                : "complete",
          createdAt: typeof item.createdAt === "string" ? item.createdAt : event.timestamp
        });
      }
    }
  } else if (event.type === "item.delta" && event.itemId && typeof event.payload.delta === "string") {
    next = updateTurn(thread, event.branchId, event.turnId, (turn) => ({
      id: event.turnId as string,
      branchId: event.branchId as string,
      status: turn?.status ?? "running",
      events: (turn?.events ?? []).map((candidate) =>
        candidate.id === event.itemId && candidate.type === "message"
          ? { ...candidate, content: `${candidate.content}${event.payload.delta}` }
          : candidate
      )
    }));
  } else if (event.type === "run.state_changed" || event.type === "run.settled") {
    const status = runtimeStatus(event.payload.status);
    if (status) {
      next = updateTurn(thread, event.branchId, event.turnId, (turn) => ({
        id: event.turnId as string,
        branchId: event.branchId as string,
        status,
        events: turn?.events ?? []
      }));
    }
  }

  next = { ...next, updatedAt: event.timestamp };
  return threads.map((candidate) => (candidate.id === next.id ? next : candidate));
}

export function replayRuntimeEvents(
  threads: Thread[],
  events: RuntimeJournalEvent[]
): Thread[] {
  return events.reduce(applyRuntimeEvent, threads);
}
