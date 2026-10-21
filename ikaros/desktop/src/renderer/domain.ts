export type TurnStatus =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "interrupted";

export type RunStatus = "idle" | TurnStatus;

export function isRunActive(status: RunStatus) {
  return status === "queued" || status === "running";
}

interface EventBase {
  id: string;
  turnId: string;
  createdAt: string;
}

export type AppEventTextKind =
  | "tool.runProcess"
  | "tool.startProcess"
  | "tool.readProcess"
  | "tool.waitProcess"
  | "tool.stopProcess"
  | "tool.writeProcess"
  | "tool.resizeProcess"
  | "tool.readFile"
  | "tool.writeFile"
  | "tool.editFile"
  | "tool.globFiles"
  | "tool.searchFiles"
  | "tool.listDirectory"
  | "tool.listThreads"
  | "tool.readThread"
  | "tool.memory"
  | "tool.subagent"
  | "result.processCompleted"
  | "result.processRunning"
  | "result.processStopped"
  | "result.processUnknown"
  | "result.processFailed"
  | "result.processInterrupted"
  | "result.readCompleted"
  | "result.readFailed"
  | "result.readInterrupted"
  | "result.writeCompleted"
  | "result.writeFailed"
  | "result.writeInterrupted"
  | "result.editCompleted"
  | "result.editFailed"
  | "result.editInterrupted"
  | "result.globCompleted"
  | "result.globFailed"
  | "result.globInterrupted"
  | "result.searchCompleted"
  | "result.searchFailed"
  | "result.searchInterrupted"
  | "result.listCompleted"
  | "result.listFailed"
  | "result.listInterrupted"
  | "result.threadListCompleted"
  | "result.threadListFailed"
  | "result.threadListInterrupted"
  | "result.threadReadCompleted"
  | "result.threadReadFailed"
  | "result.threadReadInterrupted"
  | "result.entryCount"
  | "result.matchCount"
  | "result.memorySaved"
  | "result.memorySaveFailed"
  | "result.memorySaveInterrupted"
  | "result.memoryFound"
  | "result.memorySearchFailed"
  | "result.subagentCompleted"
  | "result.memorySearchInterrupted"
  | "result.memoryCount";

export type EventTextValues = Record<string, string | number>;

export type EventText =
  | {
      source: "app";
      kind: AppEventTextKind;
      values?: EventTextValues;
    }
  | {
      source: "external";
      value: string;
    };

export function appEventText(
  kind: AppEventTextKind,
  values?: EventTextValues,
): EventText {
  return values ? { source: "app", kind, values } : { source: "app", kind };
}

export function externalEventText(value: string): EventText {
  return { source: "external", value };
}

export interface MessageEvent extends EventBase {
  type: "message";
  role: "user" | "assistant";
  content: string;
  status?: "streaming" | "complete" | "failed" | "interrupted";
}

export interface ToolCallEvent extends EventBase {
  type: "tool_call";
  toolName: string;
  label: EventText;
  status: "running" | "success" | "error" | "interrupted";
  arguments: Record<string, unknown>;
  durationMs?: number;
}

export interface ToolResultEvent extends EventBase {
  type: "tool_result";
  toolCallId: string;
  toolName?: string;
  status: "success" | "error" | "interrupted";
  summary: EventText;
  output: string;
  path?: string;
  errorCode?: string;
  details?: {
    lineStart?: number;
    lineEnd?: number;
    totalLines?: number;
    nextOffset?: number;
    bytesRead?: number;
    bytesWritten?: number;
    replacements?: number;
    created?: boolean;
    truncated?: boolean;
    entryCount?: number;
    matchCount?: number;
    count?: number;
    processId?: string;
    background?: boolean;
    processState?: "running" | "exited" | "terminated" | "unknown";
    supervise?: "always" | "on-failure" | null;
    restartCount?: number;
    exitCode?: number | null;
    cursor?: number;
    nextCursor?: number;
    hasMore?: boolean;
  };
}

export interface FileChangeEvent extends EventBase {
  type: "file_change";
  path: string;
  operation: "created" | "modified" | "deleted" | "renamed";
  additions: number;
  deletions: number;
  diff: string;
}

export interface StatusEvent extends EventBase {
  type: "status";
  tone: "neutral" | "success" | "warning" | "danger";
  label: EventText;
  detail?: EventText;
}

export type AgentEvent =
  | MessageEvent
  | ToolCallEvent
  | ToolResultEvent
  | FileChangeEvent
  | StatusEvent;

export type RuntimeModelSelection = { providerId: string; modelId: string };

export interface RuntimeRunProgress {
  queuedAt: string;
  startedAt: string | null;
  settledAt: string | null;
  modelCalls: number;
  compactions?: number;
  compacting?: boolean;
}

export interface Turn {
  id: string;
  branchId: string;
  runId?: string;
  status: TurnStatus;
  reasonCode?: string | null;
  runProgress?: RuntimeRunProgress;
  modelSelection?: RuntimeModelSelection;
  events: AgentEvent[];
}

export interface Branch {
  id: string;
  threadId: string;
  turns: Turn[];
  createdAt: string;
}

export interface Thread {
  id: string;
  projectId: string | null;
  title: string;
  activeBranchId: string;
  branches: Branch[];
  updatedAt: string;
}

export interface Project {
  id: string;
  name: string;
  color: string;
  rootUri?: string;
  instructions?: string;
}

export function findThread(threads: Thread[], threadId: string | null) {
  if (!threadId) return undefined;
  return threads.find((thread) => thread.id === threadId);
}

export function findProjectForThread(projects: Project[], thread: Thread | undefined) {
  if (!thread?.projectId) return undefined;
  return projects.find((project) => project.id === thread.projectId);
}

export function threadsForProject(threads: Thread[], projectId: string) {
  return threads.filter((thread) => thread.projectId === projectId);
}

export function standaloneThreads(threads: Thread[]) {
  return threads
    .filter((thread) => thread.projectId === null)
    .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt));
}

export function activeBranch(thread: Thread | undefined) {
  return thread?.branches.find((branch) => branch.id === thread.activeBranchId);
}

export function flattenEvents(thread: Thread | undefined) {
  return activeBranch(thread)?.turns.flatMap((turn) => turn.events) ?? [];
}

export function updateThread(
  threads: Thread[],
  threadId: string,
  updater: (thread: Thread) => Thread,
) {
  return threads.map((thread) =>
    thread.id === threadId ? updater(thread) : thread,
  );
}