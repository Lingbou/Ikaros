export type ScenarioId =
  | "streaming"
  | "tools"
  | "artifact"
  | "permission"
  | "recovery";
export type TurnStatus =
  | "queued"
  | "running"
  | "waiting_permission"
  | "completed"
  | "failed"
  | "interrupted";

export type RunStatus = "idle" | TurnStatus;

export function isRunActive(status: RunStatus) {
  return status === "queued" || status === "running" || status === "waiting_permission";
}

interface EventBase {
  id: string;
  turnId: string;
  createdAt: string;
}

export type AppEventTextKind =
  | "tool.inspectArchiveMetadata"
  | "tool.testArchiveExtraction"
  | "tool.exportReportPackage"
  | "tool.runProcess"
  | "tool.startProcess"
  | "tool.readProcess"
  | "tool.waitProcess"
  | "tool.stopProcess"
  | "tool.readFile"
  | "tool.writeFile"
  | "tool.editFile"
  | "result.archiveManifestVerified"
  | "result.windowsReservedNames"
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
  | "permission.moveReceiptsTitle"
  | "permission.moveReceiptsDescription"
  | "status.buildingBrief"
  | "status.briefBuilt"
  | "status.sourcesLinked"
  | "status.checkpointRestored"
  | "status.exportStepRetried"
  | "status.outputsValidated";

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
    processId?: string;
    processState?: "running" | "exited" | "terminated" | "unknown";
    exitCode?: number | null;
  };
}

export interface PermissionEvent extends EventBase {
  type: "permission_request";
  requestId: string;
  title: EventText;
  description: EventText;
  resource: string;
  risk: "low" | "medium" | "high";
  status: "pending" | "allowed" | "denied";
}

export type InterruptCopy =
  | {
      source: "app";
      kind:
        | "run_stopped"
        | "worker_disconnected"
        | "recovering_checkpoint"
        | "retrying_export"
        | "recovery_completed";
    }
  | { source: "external"; title: string; description: string };

export interface InterruptEvent extends EventBase {
  type: "interrupt";
  copy: InterruptCopy;
  recoverable: boolean;
  status: "interrupted" | "recovering" | "recovered";
}

export interface ArtifactEvent extends EventBase {
  type: "artifact";
  artifactId: string;
  title: string;
  mediaType: "text/markdown" | "text/plain" | "application/json";
  content: string;
  version: number;
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

export type BranchCreatedCopy =
  | { source: "app"; kind: "edited_message" }
  | { source: "external"; value: string };

export interface BranchEvent extends EventBase {
  type: "branch_created";
  branchId: string;
  sourceEventId: string;
  copy: BranchCreatedCopy;
}

export type AgentEvent =
  | MessageEvent
  | ToolCallEvent
  | ToolResultEvent
  | PermissionEvent
  | InterruptEvent
  | ArtifactEvent
  | FileChangeEvent
  | StatusEvent
  | BranchEvent;

export type RuntimeModelSelection = { providerId: string; modelId: string };

export interface RuntimeRunProgress {
  queuedAt: string;
  startedAt: string | null;
  settledAt: string | null;
  modelCalls: number;
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

export type BranchLabel =
  | { source: "app"; kind: "main" }
  | { source: "app"; kind: "number"; number: number }
  | { source: "external"; value: string };

export interface Branch {
  id: string;
  threadId: string;
  label: BranchLabel;
  parentBranchId?: string;
  forkedFromEventId?: string;
  turns: Turn[];
  createdAt: string;
}

export interface Thread {
  id: string;
  projectId: string | null;
  title: string;
  activeBranchId: string;
  branches: Branch[];
  scenarioId?: ScenarioId;
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
