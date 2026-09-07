import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  type RuntimeJournalEvent,
  type RuntimeThreadSummary,
  type RuntimeTurnHistory,
  type RuntimeWorkspaceSummary,
} from "../shared/runtime";
import type {
  AgentEvent,
  AppEventTextKind,
  Project,
  RuntimeRunProgress,
  Thread,
  ToolResultEvent,
  Turn,
  TurnStatus,
} from "./domain";
import { appEventText, externalEventText } from "./domain";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

const THREAD_SNAPSHOT_EVENTS = new Set([
  "thread.created",
  "thread.renamed",
  "thread.archived",
  "thread.unarchived",
]);

export function runtimeThreadSummaryFromEvent(
  event: RuntimeJournalEvent,
): RuntimeThreadSummary | undefined {
  if (!THREAD_SNAPSHOT_EVENTS.has(event.type) || !isRecord(event.payload.thread)) {
    return undefined;
  }
  const value = event.payload.thread;
  const workspace = value.workspace;
  if (
    typeof value.id !== "string" ||
    (value.title !== null && typeof value.title !== "string") ||
    typeof value.defaultBranchId !== "string" ||
    (workspace !== null &&
      (!isRecord(workspace) ||
        typeof workspace.id !== "string" ||
        typeof workspace.name !== "string" ||
        (workspace.rootUri !== null && typeof workspace.rootUri !== "string"))) ||
    typeof value.createdAt !== "string" ||
    typeof value.updatedAt !== "string" ||
    (value.archivedAt !== null && typeof value.archivedAt !== "string")
  ) {
    return undefined;
  }
  return value as unknown as RuntimeThreadSummary;
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

function materializedItemEventType(status: string): "item.started" | "item.completed" {
  return status === "queued" || status === "running" || status === "streaming"
    ? "item.started"
    : "item.completed";
}

export function projectRuntimeThreadHistory(
  summary: RuntimeThreadSummary,
  turns: readonly RuntimeTurnHistory[],
): Thread {
  let projected = projectRuntimeThread(summary);
  for (const turn of [...turns].sort((left, right) => left.ordinal - right.ordinal)) {
    for (const run of turn.runs) {
      for (const item of run.items) {
        [projected] = applyRuntimeEvent([projected], {
          seq: 0,
          schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
          type: materializedItemEventType(item.status),
          threadId: turn.threadId,
          branchId: turn.branchId,
          turnId: turn.id,
          runId: run.id,
          itemId: item.id,
          timestamp: item.updatedAt,
          payload: { item },
        });
      }
      [projected] = applyRuntimeEvent([projected], {
        seq: 0,
        schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
        type:
          run.status === "completed" ||
          run.status === "failed" ||
          run.status === "cancelled"
            ? "run.settled"
            : "run.state_changed",
        threadId: turn.threadId,
        branchId: turn.branchId,
        turnId: turn.id,
        runId: run.id,
        itemId: null,
        timestamp: run.settledAt ?? turn.updatedAt,
        payload: {
          status: run.status, reasonCode: run.reasonCode,
          modelCalls: run.modelCalls,
          startedAt: run.startedAt, settledAt: run.settledAt, createdAt: run.createdAt,
        },
      });
    }
  }
  const updatedAt = turns.reduce(
    (latest, turn) =>
      turn.updatedAt.localeCompare(latest) > 0 ? turn.updatedAt : latest,
    summary.updatedAt,
  );
  return { ...projected, updatedAt };
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
  const thread = runtimeThreadSummaryFromEvent(event);
  if (!thread) return undefined;
  const workspace = thread.workspace;
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
    const events = turn?.events ?? [];
    const existing = events.some((candidate) => candidate.id === projected.id);
    let nextEvents: AgentEvent[];
    if (existing) {
      nextEvents = events.map((candidate) =>
        candidate.id === projected.id ? projected : candidate
      );
    } else if (projected.type === "tool_result") {
      const toolCallIndex = events.findIndex(
        (candidate) =>
          candidate.type === "tool_call" && candidate.id === projected.toolCallId
      );
      nextEvents = [...events];
      nextEvents.splice(toolCallIndex < 0 ? events.length : toolCallIndex + 1, 0, projected);
    } else {
      nextEvents = [...events, projected];
    }
    return {
      id: event.turnId as string,
      branchId: event.branchId as string,
      runId: event.runId ?? turn?.runId,
      status: turn?.status ?? "running",
      reasonCode: turn?.reasonCode ?? null,
      events: nextEvents
    };
  });
}

function toolStatus(status: unknown): "running" | "success" | "error" | "interrupted" {
  if (status === "completed") return "success";
  if (status === "failed") return "error";
  if (status === "cancelled") return "interrupted";
  return "running";
}

const PROCESS_TOOL_LABELS = {
  process_start: "tool.startProcess", process_read: "tool.readProcess",
  process_wait: "tool.waitProcess", process_stop: "tool.stopProcess",
} as const satisfies Record<string, AppEventTextKind>;

function isProcessTool(name: string): name is keyof typeof PROCESS_TOOL_LABELS {
  return Object.hasOwn(PROCESS_TOOL_LABELS, name);
}

function processSummary(result: Record<string, unknown>, status: ToolResultEvent["status"]): AppEventTextKind {
  if (result.state === "running") return "result.processRunning";
  if (result.state === "terminated") return "result.processStopped";
  if (result.state === "unknown") return "result.processUnknown";
  return status === "success" ? "result.processCompleted" : status === "interrupted" ? "result.processInterrupted" : "result.processFailed";
}

function processDetails(result: Record<string, unknown>): ToolResultEvent["details"] {
  return {
    ...(typeof result.processId === "string" ? { processId: result.processId } : {}),
    ...(result.exitCode === null || Number.isSafeInteger(result.exitCode) ? { exitCode: result.exitCode as number | null } : {}),
    ...(["running", "exited", "terminated", "unknown"].includes(String(result.state)) ? { processState: result.state as "running" | "exited" | "terminated" | "unknown" } : {}),
    ...(typeof result.truncated === "boolean" ? { truncated: result.truncated } : {}),
  };
}

type FileToolName = "read" | "write" | "edit";

function isFileToolName(name: string): name is FileToolName {
  return name === "read" || name === "write" || name === "edit";
}

const FILE_TOOL_LABELS: Record<FileToolName, AppEventTextKind> = {
  read: "tool.readFile",
  write: "tool.writeFile",
  edit: "tool.editFile",
};

const FILE_TOOL_RESULTS: Record<
  FileToolName,
  Record<ToolResultEvent["status"], AppEventTextKind>
> = {
  read: {
    success: "result.readCompleted",
    error: "result.readFailed",
    interrupted: "result.readInterrupted",
  },
  write: {
    success: "result.writeCompleted",
    error: "result.writeFailed",
    interrupted: "result.writeInterrupted",
  },
  edit: {
    success: "result.editCompleted",
    error: "result.editFailed",
    interrupted: "result.editInterrupted",
  },
};

function projectToolArguments(
  toolName: string,
  argumentsValue: Record<string, unknown>,
): Record<string, unknown> {
  if (!isFileToolName(toolName)) return argumentsValue;

  const projected: Record<string, unknown> = {};
  if (typeof argumentsValue.filePath === "string") {
    projected.filePath = argumentsValue.filePath;
  }
  if (toolName === "read") {
    if (Number.isInteger(argumentsValue.offset) && Number(argumentsValue.offset) > 0) {
      projected.offset = argumentsValue.offset;
    }
    if (Number.isInteger(argumentsValue.limit) && Number(argumentsValue.limit) > 0) {
      projected.limit = argumentsValue.limit;
    }
  } else if (toolName === "edit" && typeof argumentsValue.replaceAll === "boolean") {
    projected.replaceAll = argumentsValue.replaceAll;
  }
  return projected;
}

function projectFileResultDetails(
  result: Record<string, unknown>,
): ToolResultEvent["details"] | undefined {
  const details: NonNullable<ToolResultEvent["details"]> = {};
  const integerKeys = [
    "lineStart",
    "lineEnd",
    "totalLines",
    "nextOffset",
    "bytesRead",
    "bytesWritten",
    "replacements",
  ] as const;
  for (const key of integerKeys) {
    const value = result[key];
    if (Number.isInteger(value) && Number(value) >= 0) {
      details[key] = Number(value);
    }
  }
  if (typeof result.created === "boolean") details.created = result.created;
  if (typeof result.truncated === "boolean") details.truncated = result.truncated;
  return Object.keys(details).length > 0 ? details : undefined;
}

function safeFileResultOutput(value: unknown, status: ToolResultEvent["status"]): string {
  if (status === "success" || typeof value !== "string") return "";
  return value.slice(0, 2_000);
}

function projectThreadSnapshot(
  threads: Thread[],
  event: RuntimeJournalEvent
): Thread[] | undefined {
  const summary = runtimeThreadSummaryFromEvent(event);
  if (!summary) return undefined;
  if (summary.archivedAt !== null) {
    return threads.filter((thread) => thread.id !== summary.id);
  }
  const projected = projectRuntimeThread(summary);
  const exists = threads.some((thread) => thread.id === projected.id);
  return exists
    ? threads.map((thread) => (thread.id === projected.id ? { ...projected, branches: thread.branches } : thread))
    : [projected, ...threads];
}

export function applyRuntimeCatalogEvent(
  threads: Thread[],
  event: RuntimeJournalEvent,
): Thread[] {
  if (event.type === "model.input_prepared" || event.type === "model.response_finished") {
    return threads;
  }
  if (THREAD_SNAPSHOT_EVENTS.has(event.type)) {
    return projectThreadSnapshot(threads, event) ?? threads;
  }
  if (!event.threadId) {
    return threads;
  }
  return threads.map((thread) =>
    thread.id === event.threadId
      ? {
          ...thread,
          updatedAt:
            event.timestamp.localeCompare(thread.updatedAt) > 0
              ? event.timestamp
              : thread.updatedAt,
        }
      : thread,
  );
}

function progressFromEvent(previous: RuntimeRunProgress | undefined, event: RuntimeJournalEvent): RuntimeRunProgress | undefined {
  const snapshot = isRecord(event.payload.run) ? event.payload.run : event.payload;
  if (
    !previous && typeof snapshot.createdAt !== "string" &&
    event.type !== "run.state_changed" && event.type !== "run.settled"
  ) return undefined;
  const calls = event.type === "model.input_prepared" ? event.payload.stepOrdinal : snapshot.modelCalls;
  return {
    modelCalls: Number.isSafeInteger(calls) && Number(calls) >= 0
      ? Math.max(previous?.modelCalls ?? 0, Number(calls)) : previous?.modelCalls ?? 0,
    queuedAt: typeof snapshot.createdAt === "string" ? snapshot.createdAt : previous?.queuedAt ?? event.timestamp,
    startedAt: typeof snapshot.startedAt === "string" ? snapshot.startedAt : previous?.startedAt ??
      (event.payload.status === "running" ? event.timestamp : null),
    settledAt: typeof snapshot.settledAt === "string" ? snapshot.settledAt : previous?.settledAt ??
      (event.type === "run.settled" ? event.timestamp : null),
  };
}

export function applyRuntimeEvent(threads: Thread[], event: RuntimeJournalEvent): Thread[] {
  if (event.type === "model.response_finished") {
    return threads;
  }
  if (THREAD_SNAPSHOT_EVENTS.has(event.type)) {
    return projectThreadSnapshot(threads, event) ?? threads;
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
      const toolName = item.data.toolName;
      next = upsertAgentEvent(thread, event, {
        id: item.id,
        turnId: event.turnId,
        type: "tool_call",
        toolName,
        label: isFileToolName(toolName)
          ? appEventText(FILE_TOOL_LABELS[toolName])
          : isProcessTool(toolName)
            ? appEventText(PROCESS_TOOL_LABELS[toolName])
            : externalEventText(toolName),
        status: toolStatus(item.status),
        arguments: projectToolArguments(toolName, item.data.arguments),
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
          ? item.data.toolName
          : "tool";
      const fileTool = isFileToolName(toolName);
      const fileDetails = fileTool
        ? projectFileResultDetails(item.data.result)
        : isProcessTool(toolName) ? processDetails(item.data.result) : undefined;
      const path =
        fileTool && typeof item.data.result.path === "string"
          ? item.data.result.path
          : undefined;
      const errorCode =
        typeof item.data.result.errorCode === "string" &&
        /^[A-Za-z0-9_.-]{1,80}$/.test(item.data.result.errorCode)
          ? item.data.result.errorCode
          : undefined;
      next = upsertAgentEvent(thread, event, {
        id: item.id,
        turnId: event.turnId,
        type: "tool_result",
        toolCallId: item.data.toolCallItemId,
        toolName,
        status: resultStatus,
        summary: fileTool
          ? appEventText(FILE_TOOL_RESULTS[toolName][resultStatus])
          : isProcessTool(toolName)
            ? appEventText(processSummary(item.data.result, resultStatus))
            : externalEventText(toolName),
        output: fileTool
          ? safeFileResultOutput(item.data.result.output, resultStatus)
          : typeof item.data.result.output === "string"
            ? item.data.result.output
            : "",
        ...(path === undefined ? {} : { path }),
        ...(errorCode === undefined ? {} : { errorCode }),
        ...(fileDetails === undefined ? {} : { details: fileDetails }),
        createdAt: typeof item.createdAt === "string" ? item.createdAt : event.timestamp
      });
    }
  } else if (event.type === "item.delta" && event.itemId && typeof event.payload.delta === "string") {
    next = updateTurn(thread, event.branchId, event.turnId, (turn) => ({
      id: event.turnId as string,
      branchId: event.branchId as string,
      runId: event.runId ?? turn?.runId,
      status: turn?.status ?? "running",
      reasonCode: turn?.reasonCode ?? null,
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
        reasonCode:
          (status === "failed" || status === "interrupted") &&
          typeof event.payload.reasonCode === "string" &&
          /^[A-Za-z0-9_.-]{1,200}$/.test(event.payload.reasonCode)
            ? event.payload.reasonCode
            : null,
        events: turn?.events ?? []
      }));
    }
  }

  const priorTurn = thread.branches.find((branch) => branch.id === event.branchId)?.turns.find((turn) => turn.id === event.turnId);
  const runProgress = progressFromEvent(priorTurn?.runId === event.runId ? priorTurn.runProgress : undefined, event);
  if (runProgress) {
    next = updateTurn(next, event.branchId, event.turnId, (turn) => ({
      ...turn, id: event.turnId as string, branchId: event.branchId as string,
      runId: event.runId ?? turn?.runId, status: turn?.status ?? "queued",
      events: turn?.events ?? [], runProgress,
    }));
  }
  if (next === thread) return threads;
  next = { ...next, updatedAt: event.timestamp };
  return threads.map((candidate) => (candidate.id === next.id ? next : candidate));
}

export function replayRuntimeEvents(
  threads: Thread[],
  events: RuntimeJournalEvent[]
): Thread[] {
  return events.reduce(applyRuntimeEvent, threads);
}
