import type {
  RuntimeJournalEvent,
  RuntimeThreadSummary,
  RuntimeWorkspaceSummary,
} from "../shared/runtime";
import type { AgentEvent, Project, Thread, Turn, TurnStatus } from "./domain";
import { appEventText, externalEventText } from "./domain";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function projectRuntimeThread(summary: RuntimeThreadSummary): Thread {
  const workspace = summary.workspace ?? null;
  return {
    id: summary.id,
    projectId: workspace?.id ?? null,
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

export function projectRuntimeProjects(
  summaries: readonly RuntimeThreadSummary[],
): Project[] {
  const workspaces = new Map<string, RuntimeWorkspaceSummary>();
  for (const summary of summaries) {
    const workspace = summary.workspace ?? null;
    if (workspace && !workspaces.has(workspace.id)) {
      workspaces.set(workspace.id, workspace);
    }
  }
  return [...workspaces.values()].map((workspace) => ({
    id: workspace.id,
    name: workspace.name,
    color: "var(--muted-strong)",
    ...(workspace.rootUri === null ? {} : { rootUri: workspace.rootUri }),
  }));
}

export function runtimeThreadWorkspaceFromEvent(
  event: RuntimeJournalEvent,
): RuntimeWorkspaceSummary | null | undefined {
  if (event.type !== "thread.created" || !isRecord(event.payload.thread)) {
    return undefined;
  }
  const workspace = event.payload.thread.workspace;
  if (workspace === null) return null;
  if (
    !isRecord(workspace) ||
    typeof workspace.id !== "string" ||
    typeof workspace.name !== "string" ||
    (workspace.rootUri !== null && typeof workspace.rootUri !== "string")
  ) {
    return undefined;
  }
  return workspace as unknown as RuntimeWorkspaceSummary;
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

function upsertAgentEvent(
  thread: Thread,
  event: RuntimeJournalEvent,
  projected: AgentEvent
): Thread {
  if (!event.branchId || !event.turnId) {
    return thread;
  }
  return updateTurn(thread, event.branchId, event.turnId, (turn) => {
    const existing = turn?.events.some((candidate) => candidate.id === projected.id);
    return {
      id: event.turnId as string,
      branchId: event.branchId as string,
      runId: event.runId ?? turn?.runId,
      status: turn?.status ?? "running",
      events: existing
        ? (turn?.events ?? []).map((candidate) =>
            candidate.id === projected.id ? projected : candidate
          )
        : [...(turn?.events ?? []), projected]
    };
  });
}

function toolStatus(status: unknown): "running" | "success" | "error" | "interrupted" {
  if (status === "completed") return "success";
  if (status === "failed") return "error";
  if (status === "cancelled") return "interrupted";
  return "running";
}

function displayToolName(name: string): string {
  return name === "process_run" ? "process.run" : name;
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
        next = upsertAgentEvent(thread, event, {
          id: item.id,
          turnId: event.turnId,
          type: "message",
          role: item.role,
          content: item.content,
          status:
            event.type === "item.started"
              ? "streaming"
              : item.status === "cancelled"
                ? "interrupted"
              : item.status === "failed"
                ? "failed"
                : "complete",
          createdAt: typeof item.createdAt === "string" ? item.createdAt : event.timestamp
        });
      }
    } else if (
      isRecord(item) &&
      item.kind === "tool_call" &&
      typeof item.id === "string" &&
      isRecord(item.data) &&
      typeof item.data.toolName === "string" &&
      isRecord(item.data.arguments)
    ) {
      const toolName = displayToolName(item.data.toolName);
      next = upsertAgentEvent(thread, event, {
        id: item.id,
        turnId: event.turnId,
        type: "tool_call",
        toolName,
        label:
          item.data.toolName === "process_run"
            ? appEventText("tool.runProcess")
            : externalEventText(toolName),
        status: toolStatus(item.status),
        arguments: item.data.arguments,
        durationMs:
          typeof item.data.durationMs === "number" ? item.data.durationMs : undefined,
        createdAt: typeof item.createdAt === "string" ? item.createdAt : event.timestamp
      });
    } else if (
      isRecord(item) &&
      item.kind === "tool_result" &&
      typeof item.id === "string" &&
      isRecord(item.data) &&
      typeof item.data.toolCallItemId === "string" &&
      isRecord(item.data.result)
    ) {
      const status = toolStatus(item.status);
      const resultStatus = status === "running" ? "error" : status;
      const toolName =
        typeof item.data.toolName === "string"
          ? displayToolName(item.data.toolName)
          : "tool";
      next = upsertAgentEvent(thread, event, {
        id: item.id,
        turnId: event.turnId,
        type: "tool_result",
        toolCallId: item.data.toolCallItemId,
        status: resultStatus,
        summary:
          item.data.toolName === "process_run"
            ? appEventText(
                resultStatus === "success"
                  ? "result.processCompleted"
                  : resultStatus === "interrupted"
                    ? "result.processInterrupted"
                    : "result.processFailed"
              )
            : externalEventText(toolName),
        output: typeof item.data.result.output === "string" ? item.data.result.output : "",
        createdAt: typeof item.createdAt === "string" ? item.createdAt : event.timestamp
      });
    }
  } else if (event.type === "item.delta" && event.itemId && typeof event.payload.delta === "string") {
    next = updateTurn(thread, event.branchId, event.turnId, (turn) => ({
      id: event.turnId as string,
      branchId: event.branchId as string,
      runId: event.runId ?? turn?.runId,
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
        runId: event.runId ?? turn?.runId,
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
