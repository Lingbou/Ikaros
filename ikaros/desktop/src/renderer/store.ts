import { create, type StoreApi } from "zustand";
import {
  activeBranch,
  findThread,
  isRunActive,
  type AgentEvent,
  type PermissionEvent,
  type Project,
  type RunStatus,
  type RuntimeModelSelection,
  type ScenarioId,
  type Thread,
  type Turn,
  updateThread,
} from "./domain";
import {
  createBranchFromMessage,
  createInitialProjects,
  createInitialThreads,
  MockAgentClient,
} from "./mockAgentClient";
import { DEFAULT_SIDEBAR_WIDTH } from "../shared/platform";
import { LOCAL_PROFILE } from "./localProfile";
import { createRuntimeClient, isRuntimeRpcError } from "./runtimeClient";
import {
  applyRuntimeCatalogEvent,
  applyRuntimeEvent as projectRuntimeEvent,
  projectRuntimeProjects,
  projectRuntimeThread,
  projectRuntimeThreadHistory,
  projectRuntimeThreads,
  runtimeThreadSummaryFromEvent,
  runtimeThreadWorkspaceFromEvent,
} from "./runtimeProjection";
import {
  loadOlderRuntimeTurnPage,
  loadRuntimeThreadHistory,
  type LoadedRuntimeTurnHistory,
} from "./runtimeHistory";
import type {
  RuntimeDiscoveredModel,
  RuntimeModelSetLimitsParams,
  RuntimeHostStatus,
  RuntimeHostStatusState,
  RuntimeJournalEvent,
  RuntimeModelSetEnabledParams,
  RuntimeModelSummary,
  RuntimeProviderConfigureParams,
  RuntimeProviderSummary,
  RuntimeSkillDiagnostic,
  RuntimeSkillSetEnabledParams,
  RuntimeSkillSummary,
  RuntimeThreadCreateResult,
  RuntimeThreadMutationResult,
  RuntimeThreadSummary,
  RuntimeTurnStartResult,
  RuntimeWorkspaceSummary,
} from "../shared/runtime";

type EditingMessage = { eventId: string; content: string } | null;
type RunContext = {
  threadId: string;
  branchId: string;
  turnId: string;
  runId?: string;
  epoch: number;
};
type PendingRuntimeSubmission = {
  epoch: number;
  branchId: string | null;
  prompt: string;
  providerId: string;
  modelId: string;
  afterSeq: number;
  foregroundGeneration: number;
  workspace: RuntimeWorkspaceSummary | null;
  createRequestId?: string;
  turnRequestId: string;
  turnStartClaimed: boolean;
  createdEvent?: RuntimeJournalEvent;
  acknowledged?: boolean;
  runId?: string;
  turnId?: string;
};
type ProviderCatalogStatus = "idle" | "loading" | "ready" | "error";
type SkillCatalogStatus = "idle" | "loading" | "ready" | "error";
type RuntimeThreadDetailStatus = "idle" | "loading" | "ready" | "error";
type RuntimeArchivedCatalogStatus = "idle" | "loading" | "ready" | "error";
type RuntimeContinuationStatus = "idle" | "loading" | "error";
type RuntimeSearchCatalogStatus = "idle" | "loading" | "ready" | "error";
type RuntimeThreadDetailState = {
  status: RuntimeThreadDetailStatus;
  snapshotSeq: number;
  error: string | null;
  nextCursor: string | null;
  hasMore: boolean;
  olderStatus: RuntimeContinuationStatus;
  olderError: string | null;
  historySnapshotSeq: number;
  turnOrdinals: Record<string, number>;
};
type RuntimeRunActivityState = {
  seq: number;
  branchId: string;
  turnId: string;
  runId: string;
  status: RunStatus;
  turnOrdinal: number | null;
  runOrdinal: number | null;
};
type RuntimeThreadActivityState = Record<string, RuntimeRunActivityState>;
export type RuntimeIssue =
  | { kind: "connection" | "initialization"; message: string }
  | { kind: "send"; message: string; threadId: string | null; prompt: string }
  | { kind: "cancel"; message: string; runId: string }
  | { kind: "history" | "archive" | "unarchive"; message: string; threadId: string }
  | { kind: "rename"; message: string; threadId: string; title: string | null }
  | { kind: "archived_catalog"; message: string };

export interface RuntimeFileSelection {
  threadId: string;
  path: string;
  sourceToolCallItemId?: string;
  toolCallItemId?: string;
  view: "current" | "change";
}

interface AppState {
  runtimeMode: boolean;
  runtimeReady: boolean;
  runtimeConnectionStatus: RuntimeHostStatusState;
  runtimeError: string | null;
  runtimeIssue: RuntimeIssue | null;
  runtimeSeq: number;
  providerCatalogStatus: ProviderCatalogStatus;
  providers: RuntimeProviderSummary[];
  models: RuntimeModelSummary[];
  skillCatalogStatus: SkillCatalogStatus;
  skillCatalogError: string | null;
  skills: RuntimeSkillSummary[];
  skillDiagnostics: RuntimeSkillDiagnostic[];
  selectedModel: RuntimeModelSelection | null;
  projects: Project[];
  threads: Thread[];
  threadCatalogNextCursor: string | null;
  threadCatalogSnapshotSeq: number;
  threadCatalogHasMore: boolean;
  threadCatalogMoreStatus: RuntimeContinuationStatus;
  threadCatalogMoreError: string | null;
  searchCatalogStatus: RuntimeSearchCatalogStatus;
  searchCatalogError: string | null;
  archivedThreads: RuntimeThreadSummary[];
  archivedCatalogNextCursor: string | null;
  archivedCatalogSnapshotSeq: number;
  archivedCatalogStatus: RuntimeArchivedCatalogStatus;
  archivedCatalogHasMore: boolean;
  archivedCatalogMoreStatus: RuntimeContinuationStatus;
  archivedCatalogMoreError: string | null;
  runtimeThreadDetails: Record<string, RuntimeThreadDetailState>;
  runtimeThreadActivity: Record<string, RuntimeThreadActivityState>;
  selectedThreadId: string | null;
  runStatus: RunStatus;
  draft: string;
  fileSelection: RuntimeFileSelection | null;
  sidebarOpen: boolean;
  sidebarWidth: number;
  searchOpen: boolean;
  settingsOpen: boolean;
  settingsInitialSection: "general" | "providers" | "models";
  profileUsername: string;
  editingMessage: EditingMessage;
  expandedProjects: Record<string, boolean>;
  runEpoch: number;
  runtimeForegroundGeneration: number;
  localThreadCounter: number;
  branchCounter: number;
  activeRun: RunContext | null;
  pendingRuntimeSubmissions: Record<string, PendingRuntimeSubmission>;
  pendingRuntimeNewThread: PendingRuntimeSubmission | null;
  newThreadWorkspace: RuntimeWorkspaceSummary | null;

  initializeRuntime: () => Promise<void>;
  retryRuntimeConnection: () => Promise<void>;
  retryRuntimeIssue: () => Promise<void>;
  clearRuntimeError: () => void;
  applyRuntimeEvent: (event: RuntimeJournalEvent) => void;
  loadMoreThreads: () => Promise<void>;
  loadAllThreadsForSearch: () => Promise<void>;
  loadArchivedThreads: () => Promise<void>;
  loadMoreArchivedThreads: () => Promise<void>;
  renameThread: (threadId: string, title: string | null) => Promise<void>;
  archiveThread: (threadId: string) => Promise<void>;
  unarchiveThread: (threadId: string) => Promise<void>;
  loadProviderCatalog: () => Promise<void>;
  discoverDeepSeekModels: (apiKey: string) => Promise<RuntimeDiscoveredModel[]>;
  configureProvider: (params: RuntimeProviderConfigureParams) => Promise<void>;
  disconnectProvider: (providerId: "deepseek") => Promise<void>;
  removeProvider: (providerId: string) => Promise<void>;
  setModelEnabled: (params: RuntimeModelSetEnabledParams) => Promise<void>;
  loadSkillCatalog: () => Promise<void>;
  setSkillEnabled: (params: RuntimeSkillSetEnabledParams) => Promise<void>;
  selectModel: (selection: RuntimeModelSelection | null) => void;
  setDraft: (draft: string) => void;
  setModelLimits: (params: RuntimeModelSetLimitsParams) => Promise<void>;
  openFile: (selection: RuntimeFileSelection) => void;
  closeFile: () => void;
  setSidebarOpen: (open: boolean) => void;
  setSidebarWidth: (width: number) => void;
  setSearchOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean, section?: "providers" | "models") => void;
  setProfileUsername: (username: string) => void;
  toggleProject: (projectId: string) => void;
  beginEditMessage: (eventId: string, content: string) => void;
  cancelEditMessage: () => void;
  commitMessageEdit: (content: string) => void;
  switchBranch: (branchId: string) => void;
  newChat: () => void;
  newProjectChat: (projectId: string) => void;
  stageProjectWorkspace: (workspace: RuntimeWorkspaceSummary) => void;
  bindWorkspaceFromFolder: () => Promise<void>;
  selectThread: (threadId: string) => Promise<void>;
  retryRuntimeThread: (threadId: string) => Promise<void>;
  loadOlderRuntimeTurns: (threadId: string) => Promise<void>;
  sendDraft: () => Promise<void>;
  stopRun: () => void;
  resolvePermission: (decision: "allow" | "deny") => Promise<void>;
  recoverRun: (strategy: "retry" | "resume") => Promise<void>;
  appendAgentEvent: (
    threadId: string,
    branchId: string,
    event: AgentEvent,
    status?: RunStatus,
  ) => void;
}

const client = new MockAgentClient();
const runtimeClient = createRuntimeClient();
let runtimeInitialization: Promise<void> | undefined;
let providerCatalogRevision = 0;
let modelSelectionRevision = 0;
let skillCatalogRevision = 0;
let removeRuntimeSubscription: (() => void) | undefined;
let removeRuntimeStatusSubscription: (() => void) | undefined;
let bufferedRuntimeEvents: RuntimeJournalEvent[] = [];
const pendingRuntimeEvents = new Map<number, RuntimeJournalEvent>();
const runtimeThreadLoadEvents = new Map<string, Map<number, RuntimeJournalEvent>>();
const runtimeThreadLoads = new Map<string, Promise<void>>();
const runtimeOlderTurnLoadEvents = new Map<string, Map<number, RuntimeJournalEvent>>();
const runtimeOlderTurnLoads = new Map<string, Promise<void>>();
const runtimeCatalogRecoveryEvents = new Map<string, Map<number, RuntimeJournalEvent>>();
const runtimeCatalogRecoveries = new Map<string, Promise<void>>();
const cancellingRuntimeRuns = new Set<string>();
const runtimeTurnContinuations = new Map<string, Promise<void>>();
const canonicalRuntimeTurnStarts = new Map<string, RuntimeTurnStartResult>();
let runtimeArchivedCatalogLoadEvents: Map<number, RuntimeJournalEvent> | undefined;
let runtimeArchivedCatalogLoad: Promise<void> | undefined;
let runtimeArchivedCatalogContinuation: Promise<void> | undefined;
let runtimeActiveCatalogLoadEvents: Map<number, RuntimeJournalEvent> | undefined;
let runtimeActiveCatalogContinuation: Promise<void> | undefined;
let runtimeSearchCatalogLoad: Promise<void> | undefined;
let runtimeGapRecovery: Promise<void> | undefined;
const RUNTIME_GAP_RETRY_DELAYS_MS = [25, 75, 200] as const;
const RUNTIME_COMMAND_RETRY_DELAYS_MS = [50, 150, 400, 800] as const;
const RUNTIME_THREAD_CATALOG_PAGE_LIMIT = 25;
const MAX_THREAD_CATALOG_PAGES_PER_SEARCH = 10_000;
const MOCK_AT = "2026-08-05T06:00:00.000Z";

function sortRuntimeThreads(threads: readonly Thread[]): Thread[] {
  return [...threads].sort(
    (left, right) =>
      right.updatedAt.localeCompare(left.updatedAt) || left.id.localeCompare(right.id),
  );
}

function emptyRuntimeThreadDetail(
  snapshotSeq: number,
  current?: RuntimeThreadDetailState,
): RuntimeThreadDetailState {
  return {
    status: current?.status ?? "idle",
    snapshotSeq: Math.max(current?.snapshotSeq ?? 0, snapshotSeq),
    error: current?.error ?? null,
    nextCursor: current?.nextCursor ?? null,
    hasMore: current?.hasMore ?? false,
    olderStatus: current?.olderStatus ?? "idle",
    olderError: current?.olderError ?? null,
    historySnapshotSeq: current?.historySnapshotSeq ?? 0,
    turnOrdinals: current?.turnOrdinals ?? {},
  };
}

function runnableModels(
  providers: readonly RuntimeProviderSummary[],
  models: readonly RuntimeModelSummary[],
): RuntimeModelSummary[] {
  const configuredProviders = new Set(
    providers.filter((provider) => provider.configured).map((provider) => provider.id),
  );
  return models.filter(
    (model) => model.enabled && configuredProviders.has(model.providerId),
  );
}

function selectionIsRunnable(
  selection: RuntimeModelSelection | null,
  providers: readonly RuntimeProviderSummary[],
  models: readonly RuntimeModelSummary[],
): selection is RuntimeModelSelection {
  if (!selection) return false;
  return runnableModels(providers, models).some(
    (model) =>
      model.providerId === selection.providerId && model.id === selection.modelId,
  );
}

function reconcileModelSelection(
  selection: RuntimeModelSelection | null,
  providers: readonly RuntimeProviderSummary[],
  models: readonly RuntimeModelSummary[],
): RuntimeModelSelection | null {
  if (selectionIsRunnable(selection, providers, models)) {
    return selection;
  }
  const runnable = runnableModels(providers, models);
  return runnable[0]
    ? { providerId: runnable[0].providerId, modelId: runnable[0].id }
    : null;
}

function lastThreadModel(thread: Thread | undefined): RuntimeModelSelection | null {
  return [...(activeBranch(thread)?.turns ?? [])]
    .reverse().find((turn) => turn.modelSelection)?.modelSelection ?? null;
}

async function loadRuntimeThreadAndRestoreModel(
  set: StoreSet,
  get: StoreGet,
  threadId: string,
): Promise<void> {
  const selectionRevision = modelSelectionRevision;
  await loadRuntimeThreadIntoStore(set, get, threadId);
  set((state) => {
    if (
      state.selectedThreadId !== threadId ||
      selectionRevision !== modelSelectionRevision ||
      state.runtimeThreadDetails[threadId]?.status !== "ready"
    ) return {};
    return {
      selectedModel: reconcileModelSelection(
        lastThreadModel(findThread(state.threads, threadId)), state.providers, state.models,
      ),
    };
  });
}

function newRuntimeRequestId(kind: "thread" | "turn"): string {
  return `${kind}_${globalThis.crypto.randomUUID()}`;
}

function turnStatus(status: RunStatus | undefined): Turn["status"] | undefined {
  if (!status || status === "idle") return undefined;
  return status;
}

function appendEventToThread(
  thread: Thread,
  branchId: string,
  event: AgentEvent,
  status?: RunStatus,
): Thread {
  const branch = thread.branches.find((candidate) => candidate.id === branchId);
  if (!branch) return thread;
  const existingTurn = branch.turns.find((turn) => turn.id === event.turnId);
  const revisesExistingEvent =
    existingTurn?.events.some((candidate) => candidate.id === event.id) ?? false;
  const nextTurnStatus = turnStatus(status);
  let turns: Turn[];

  if (existingTurn) {
    turns = branch.turns.map((turn) => {
      if (turn.id !== event.turnId) return turn;
      const exists = turn.events.some((candidate) => candidate.id === event.id);
      return {
        ...turn,
        status: nextTurnStatus ?? turn.status,
        events: exists
          ? turn.events.map((candidate) =>
              candidate.id === event.id ? event : candidate,
            )
          : [...turn.events, event],
      };
    });
  } else {
    turns = [
      ...branch.turns,
      {
        id: event.turnId,
        branchId: branch.id,
        status: nextTurnStatus ?? "running",
        events: [event],
      },
    ];
  }

  return {
    ...thread,
    updatedAt: revisesExistingEvent
      ? thread.updatedAt
      : event.createdAt.localeCompare(thread.updatedAt) > 0
        ? event.createdAt
        : thread.updatedAt,
    branches: thread.branches.map((candidate) =>
      candidate.id === branch.id ? { ...candidate, turns } : candidate,
    ),
  };
}

function lastTurn(thread: Thread | undefined) {
  const branch = activeBranch(thread);
  return branch?.turns.at(-1);
}

function storedRunStatus(thread: Thread | undefined): RunStatus {
  return lastTurn(thread)?.status ?? "idle";
}

function storedRunStatusForRun(thread: Thread | undefined, runId: string): RunStatus | undefined {
  for (const branch of thread?.branches ?? []) {
    const turn = branch.turns.find((candidate) => candidate.runId === runId);
    if (turn) {
      return turn.status;
    }
  }
  return undefined;
}

function runtimeActivityStatus(value: unknown): RunStatus | undefined {
  if (
    value === "queued" ||
    value === "running" ||
    value === "completed" ||
    value === "failed"
  ) {
    return value;
  }
  return value === "cancelled" ? "interrupted" : undefined;
}

function latestRuntimeActivity(
  activities: RuntimeThreadActivityState | undefined,
  activeOnly: boolean,
): RuntimeRunActivityState | undefined {
  let latest: RuntimeRunActivityState | undefined;
  for (const activity of Object.values(activities ?? {})) {
    if (activeOnly && !isRunActive(activity.status)) continue;
    if (!latest || compareRuntimeActivityOrder(activity, latest) >= 0) {
      latest = activity;
    }
  }
  return latest;
}

function compareRuntimeActivityOrder(
  left: RuntimeRunActivityState,
  right: RuntimeRunActivityState,
): number {
  if (left.turnOrdinal !== null && right.turnOrdinal !== null) {
    if (left.turnOrdinal !== right.turnOrdinal) {
      return left.turnOrdinal - right.turnOrdinal;
    }
    if (left.runOrdinal !== null && right.runOrdinal !== null) {
      if (left.runOrdinal !== right.runOrdinal) {
        return left.runOrdinal - right.runOrdinal;
      }
    }
  } else if (left.turnOrdinal === null && right.turnOrdinal !== null) {
    // A Run absent from the installed snapshot was created after that snapshot.
    return 1;
  } else if (left.turnOrdinal !== null && right.turnOrdinal === null) {
    return -1;
  }
  return left.seq - right.seq;
}

function runtimeActivityForThread(
  state: Pick<AppState, "runtimeThreadActivity">,
  threadId: string,
): RuntimeRunActivityState | undefined {
  const activities = state.runtimeThreadActivity[threadId];
  return latestRuntimeActivity(activities, true) ?? latestRuntimeActivity(activities, false);
}

function activeRuntimeActivityForThread(
  state: Pick<AppState, "runtimeThreadActivity">,
  threadId: string,
): RuntimeRunActivityState | undefined {
  return latestRuntimeActivity(state.runtimeThreadActivity[threadId], true);
}

function hasPendingRuntimeSubmission(state: AppState, threadId: string | null): boolean {
  return threadId === null
    ? state.pendingRuntimeNewThread !== null
    : state.pendingRuntimeSubmissions[threadId] !== undefined;
}

function workspaceForProject(
  projects: readonly Project[],
  projectId: string,
): RuntimeWorkspaceSummary | undefined {
  const project = projects.find((candidate) => candidate.id === projectId);
  if (!project) return undefined;
  return {
    id: project.id,
    name: project.name,
    rootUri: project.rootUri ?? null,
  };
}

function sameWorkspace(
  left: RuntimeWorkspaceSummary | null,
  right: RuntimeWorkspaceSummary | null,
): boolean {
  return left?.id === right?.id;
}

function mergeRuntimeProjects(
  projects: readonly Project[],
  summaries: readonly RuntimeThreadSummary[],
): Project[] {
  return mergeWorkspaceProjects(
    projects,
    projectRuntimeProjects(summaries).map((project) => ({
      id: project.id,
      name: project.name,
      rootUri: project.rootUri ?? null,
    })),
  );
}

function mergeWorkspaceProjects(
  projects: readonly Project[],
  workspaces: readonly RuntimeWorkspaceSummary[],
): Project[] {
  const additions = workspaces.map((workspace) => ({
    id: workspace.id,
    name: workspace.name,
    color: "var(--muted-strong)",
    ...(workspace.rootUri === null ? {} : { rootUri: workspace.rootUri }),
  }));
  if (!additions.length) return [...projects];
  const next = new Map(projects.map((project) => [project.id, project] as const));
  for (const project of additions) {
    if (!next.has(project.id)) {
      next.set(project.id, project);
    }
  }
  return [...next.values()];
}

function runtimeRunStatusForSelection(
  state: AppState,
  selectedThreadId: string | null,
  threads = state.threads,
): RunStatus {
  if (selectedThreadId === null) {
    return state.pendingRuntimeNewThread ? "queued" : "idle";
  }
  const activity = runtimeActivityForThread(state, selectedThreadId);
  const stored = activity?.status ?? storedRunStatus(findThread(threads, selectedThreadId));
  if (state.pendingRuntimeSubmissions[selectedThreadId] && !isRunActive(stored)) {
    return "queued";
  }
  return stored;
}

function finishRun(
  set: StoreSet,
  get: StoreGet,
  context: RunContext,
  terminal: RunStatus,
) {
  const state = get();
  if (state.runEpoch !== context.epoch) return;
  const visibleThread = findThread(state.threads, state.selectedThreadId);
  const visibleBranch = activeBranch(visibleThread);
  const runStatus =
    state.selectedThreadId === context.threadId && visibleBranch?.id === context.branchId
      ? terminal
      : storedRunStatus(visibleThread);
  set({ runStatus, activeRun: null });
}

function isRunCancelable(status: RunStatus) {
  return status === "queued" || status === "running";
}

function ownsRuntimeSendForeground(
  state: AppState,
  epoch: number,
  selectedThreadId: string | null,
  foregroundGeneration: number,
  workspace?: RuntimeWorkspaceSummary | null,
): boolean {
  return (
    state.runEpoch === epoch &&
    state.selectedThreadId === selectedThreadId &&
    state.runtimeForegroundGeneration === foregroundGeneration &&
    (selectedThreadId !== null ||
      workspace === undefined ||
      sameWorkspace(state.newThreadWorkspace, workspace))
  );
}

function terminalizeEvent(event: AgentEvent): AgentEvent {
  if (event.type === "message" && event.status === "streaming") {
    return { ...event, status: "complete" };
  }
  if (event.type === "tool_call" && event.status === "running") {
    return { ...event, status: "interrupted" };
  }
  if (event.type === "interrupt" && event.status === "recovering") {
    return { ...event, status: "interrupted" };
  }
  if (event.type === "status" && event.tone === "neutral") {
    return { ...event, tone: "warning" };
  }
  return event;
}

function terminalizeTurn(thread: Thread, branchId: string, turnId: string): Thread {
  return {
    ...thread,
    branches: thread.branches.map((branch) =>
      branch.id === branchId
        ? {
            ...branch,
            turns: branch.turns.map((turn) =>
              turn.id === turnId
                ? {
                    ...turn,
                    status: "interrupted",
                    events: turn.events.map(terminalizeEvent),
                  }
                : turn,
            ),
          }
        : branch,
    ),
  };
}

function disableRecoveryForTurn(thread: Thread, branchId: string, turnId: string): Thread {
  return {
    ...thread,
    branches: thread.branches.map((branch) =>
      branch.id === branchId
        ? {
            ...branch,
            turns: branch.turns.map((turn) =>
              turn.id === turnId
                ? {
                    ...turn,
                    events: turn.events.map((event) =>
                      event.type === "interrupt"
                        ? { ...event, recoverable: false }
                        : event,
                    ),
                  }
                : turn,
            ),
          }
        : branch,
    ),
  };
}

function currentRunContext(state: AppState): RunContext | null {
  const thread = findThread(state.threads, state.selectedThreadId);
  const branch = activeBranch(thread);
  if (state.runtimeMode) {
    const activity = state.selectedThreadId
      ? activeRuntimeActivityForThread(state, state.selectedThreadId)
      : undefined;
    if (thread && activity && isRunActive(activity.status)) {
      return {
        threadId: thread.id,
        branchId: activity.branchId,
        turnId: activity.turnId,
        runId: activity.runId,
        epoch: state.runEpoch,
      };
    }
    const turn = [...(branch?.turns ?? [])]
      .reverse()
      .find((candidate) => isRunActive(candidate.status) && candidate.runId);
    if (thread && branch && turn?.runId) {
      return {
        threadId: thread.id,
        branchId: branch.id,
        turnId: turn.id,
        runId: turn.runId,
        epoch: state.runEpoch,
      };
    }
    const pending = thread ? state.pendingRuntimeSubmissions[thread.id] : undefined;
    if (thread && branch && pending?.runId && pending.turnId) {
      return {
        threadId: thread.id,
        branchId: pending.branchId ?? branch.id,
        turnId: pending.turnId,
        runId: pending.runId,
        epoch: pending.epoch,
      };
    }
    if (state.activeRun?.threadId === state.selectedThreadId) {
      return state.activeRun;
    }
    return null;
  }
  if (state.activeRun) return state.activeRun;
  const turn = branch?.turns.at(-1);
  if (!thread || !branch || !turn) return null;
  return {
    threadId: thread.id,
    branchId: branch.id,
    turnId: turn.id,
    epoch: state.runEpoch,
  };
}

function terminalizeCurrentRun(state: AppState) {
  if (!isRunCancelable(state.runStatus)) return state.threads;
  const context = currentRunContext(state);
  if (!context) return state.threads;
  return updateThread(state.threads, context.threadId, (thread) =>
    terminalizeTurn(thread, context.branchId, context.turnId),
  );
}

type StoreSet = StoreApi<AppState>["setState"];
type StoreGet = StoreApi<AppState>["getState"];

async function refreshRuntimeProviderCatalog(set: StoreSet): Promise<void> {
  if (!runtimeClient) {
    return;
  }
  const revision = ++providerCatalogRevision;
  set({ providerCatalogStatus: "loading" });
  try {
    const [providerResult, modelResult] = await Promise.all([
      runtimeClient.listProviders(),
      runtimeClient.listModels(),
    ]);
    if (revision !== providerCatalogRevision) return;
    set((state) => ({
      providers: providerResult.providers,
      models: modelResult.models,
      selectedModel: reconcileModelSelection(
        state.selectedModel,
        providerResult.providers,
        modelResult.models,
      ),
      providerCatalogStatus: "ready",
      ...(state.runtimeIssue === null ? { runtimeError: null } : {}),
    }));
  } catch (error: unknown) {
    if (revision !== providerCatalogRevision) return;
    set({
      providerCatalogStatus: "error",
      runtimeError:
        error instanceof Error ? error.message : "Runtime provider catalog failed",
      runtimeIssue: null,
    });
    throw error;
  }
}

async function mutateRuntimeProviderCatalog(
  set: StoreSet,
  mutation: () => Promise<unknown>,
): Promise<void> {
  if (!runtimeClient) {
    const error = new Error("Runtime provider configuration is unavailable.");
    set({ runtimeError: error.message, runtimeIssue: null, providerCatalogStatus: "error" });
    throw error;
  }
  try {
    await mutation();
  } catch (error: unknown) {
    set({
      runtimeError:
        error instanceof Error ? error.message : "Runtime provider configuration failed",
      runtimeIssue: null,
    });
    throw error;
  }
  try {
    await refreshRuntimeProviderCatalog(set);
  } catch {
    // The mutation is already durable. Keep its success separate from a
    // follow-up catalog refresh failure so retrying cannot duplicate it.
  }
}

async function refreshRuntimeSkillCatalog(set: StoreSet): Promise<void> {
  if (!runtimeClient) {
    set({
      skillCatalogStatus: "ready",
      skillCatalogError: null,
      skills: [],
      skillDiagnostics: [],
    });
    return;
  }
  const revision = ++skillCatalogRevision;
  set({ skillCatalogStatus: "loading", skillCatalogError: null });
  try {
    const result = await runtimeClient.listSkills();
    if (revision !== skillCatalogRevision) return;
    set({
      skillCatalogStatus: "ready",
      skillCatalogError: null,
      skills: result.skills,
      skillDiagnostics: result.diagnostics,
    });
  } catch (error: unknown) {
    if (revision !== skillCatalogRevision) return;
    const message = error instanceof Error ? error.message : "Runtime Skill catalog failed";
    set({ skillCatalogStatus: "error", skillCatalogError: message });
    throw error;
  }
}

async function mutateRuntimeSkillEnabled(
  set: StoreSet,
  params: RuntimeSkillSetEnabledParams,
): Promise<void> {
  if (!runtimeClient) {
    const error = new Error("Runtime Skill configuration is unavailable.");
    set({ skillCatalogStatus: "error", skillCatalogError: error.message });
    throw error;
  }
  const revision = ++skillCatalogRevision;
  set({ skillCatalogStatus: "loading", skillCatalogError: null });
  try {
    const result = await runtimeClient.setSkillEnabled(params);
    if (revision !== skillCatalogRevision) return;
    set((state) => ({
      skillCatalogStatus: "ready",
      skillCatalogError: null,
      skills: state.skills.some((skill) => skill.name === result.skill.name)
        ? state.skills.map((skill) =>
            skill.name === result.skill.name ? result.skill : skill,
          )
        : [...state.skills, result.skill].sort((left, right) =>
            left.name.localeCompare(right.name),
          ),
    }));
  } catch (error: unknown) {
    if (revision !== skillCatalogRevision) return;
    const message = error instanceof Error ? error.message : "Runtime Skill update failed";
    set({ skillCatalogStatus: "error", skillCatalogError: message });
    throw error;
  }
}

async function playScenarioInStore(
  set: StoreSet,
  get: StoreGet,
  threadId: string,
  scenario: ScenarioId,
) {
  client.cancel();
  const state = get();
  const epoch = state.runEpoch + 1;
  let threads = terminalizeCurrentRun(state);

  const thread = findThread(threads, threadId);
  const branch = activeBranch(thread);
  const turn = branch?.turns.at(-1);
  if (!thread || !branch || !turn) return;

  const context: RunContext = {
    threadId: thread.id,
    branchId: branch.id,
    turnId: turn.id,
    epoch,
  };
  threads = updateThread(threads, thread.id, (candidate) => ({
    ...candidate,
    branches: candidate.branches.map((candidateBranch) =>
      candidateBranch.id === branch.id
        ? {
            ...candidateBranch,
            turns: candidateBranch.turns.map((candidateTurn) =>
              candidateTurn.id === turn.id
                ? { ...candidateTurn, status: "running" }
                : candidateTurn,
            ),
          }
        : candidateBranch,
    ),
  }));
  set({
    threads,
    selectedThreadId: thread.id,
    runStatus: "running",
    editingMessage: null,
    searchOpen: false,
    settingsOpen: false,
    runEpoch: epoch,
    activeRun: context,
  });

  const terminal = await client.playScenario(scenario, (event, status) => {
    if (get().runEpoch !== epoch) return;
    get().appendAgentEvent(thread.id, branch.id, event, status);
  });
  finishRun(set, get, context, terminal);
}

function runtimeThreadCreateResultFromEvent(
  event: RuntimeJournalEvent,
  clientRequestId: string,
): RuntimeThreadCreateResult | undefined {
  if (
    event.type !== "thread.created" ||
    event.payload.clientRequestId !== clientRequestId
  ) {
    return undefined;
  }
  const value = event.payload.thread;
  if (
    typeof value !== "object" ||
    value === null ||
    !("id" in value) ||
    typeof value.id !== "string" ||
    !("title" in value) ||
    (value.title !== null && typeof value.title !== "string") ||
    !("defaultBranchId" in value) ||
    typeof value.defaultBranchId !== "string" ||
    !("createdAt" in value) ||
    typeof value.createdAt !== "string" ||
    !("updatedAt" in value) ||
    typeof value.updatedAt !== "string" ||
    !("archivedAt" in value) ||
    value.archivedAt !== null
  ) {
    return undefined;
  }
  const workspace = "workspace" in value ? value.workspace : null;
  if (
    workspace !== null &&
    (typeof workspace !== "object" ||
      !("id" in workspace) ||
      typeof workspace.id !== "string" ||
      !("name" in workspace) ||
      typeof workspace.name !== "string" ||
      !("rootUri" in workspace) ||
      (workspace.rootUri !== null && typeof workspace.rootUri !== "string"))
  ) {
    return undefined;
  }
  return {
    thread: { ...value, workspace } as unknown as RuntimeThreadSummary,
    event,
  };
}

function reconcileArchivedThreadSummary(
  archivedThreads: readonly RuntimeThreadSummary[],
  summary: RuntimeThreadSummary,
): RuntimeThreadSummary[] {
  const withoutCurrent = archivedThreads.filter((thread) => thread.id !== summary.id);
  if (summary.archivedAt === null) {
    return withoutCurrent;
  }
  return [...withoutCurrent, summary].sort(
    (left, right) =>
      right.updatedAt.localeCompare(left.updatedAt) || left.id.localeCompare(right.id),
  );
}

function runtimeEventAddsTimelineEntry(
  thread: Thread | undefined,
  event: RuntimeJournalEvent,
): boolean {
  if (runtimeThreadSummaryFromEvent(event)) return true;
  if (!thread || !event.branchId || !event.turnId) return false;
  const branch = thread.branches.find((candidate) => candidate.id === event.branchId);
  const turn = branch?.turns.find((candidate) => candidate.id === event.turnId);
  if (!turn) return true;
  if (event.type !== "item.started" && event.type !== "item.completed") {
    return false;
  }
  const itemId = event.itemId;
  return Boolean(itemId && !turn.events.some((candidate) => candidate.id === itemId));
}

function runtimeStateForEvent(state: AppState, event: RuntimeJournalEvent): Partial<AppState> {
  const threadSummary = runtimeThreadSummaryFromEvent(event);
  const detail = event.threadId ? state.runtimeThreadDetails[event.threadId] : undefined;
  const coveredByInstalledSnapshot =
    detail?.status === "ready" && event.seq <= detail.snapshotSeq;
  const existingThread = event.threadId
    ? findThread(state.threads, event.threadId)
    : undefined;
  let projectedThreads = coveredByInstalledSnapshot
    ? state.threads
    : detail?.status === "ready"
      ? projectRuntimeEvent(state.threads, event)
      : applyRuntimeCatalogEvent(state.threads, event);
  if (
    existingThread &&
    event.threadId &&
    !runtimeEventAddsTimelineEntry(existingThread, event)
  ) {
    projectedThreads = projectedThreads.map((thread) =>
      thread.id === event.threadId
        ? { ...thread, updatedAt: existingThread.updatedAt }
        : thread,
    );
  }
  const threads = sortRuntimeThreads(projectedThreads);
  const runtimeThreadDetails =
    event.threadId && detail?.status === "ready" && event.seq > detail.snapshotSeq
      ? {
          ...state.runtimeThreadDetails,
          [event.threadId]: { ...detail, snapshotSeq: event.seq },
        }
      : state.runtimeThreadDetails;
  const activityStatus =
    event.type === "run.state_changed" || event.type === "run.settled"
      ? runtimeActivityStatus(event.payload.status)
      : undefined;
  const threadActivities = event.threadId
    ? state.runtimeThreadActivity[event.threadId]
    : undefined;
  const existingActivity = event.runId ? threadActivities?.[event.runId] : undefined;
  const runtimeThreadActivity =
    event.threadId &&
    event.branchId &&
    event.turnId &&
    event.runId &&
    activityStatus &&
    event.seq > (existingActivity?.seq ?? -1)
      ? {
          ...state.runtimeThreadActivity,
          [event.threadId]: {
            ...threadActivities,
            [event.runId]: {
              ...existingActivity,
              seq: event.seq,
              branchId: event.branchId,
              turnId: event.turnId,
              runId: event.runId,
              status: activityStatus,
              turnOrdinal: existingActivity?.turnOrdinal ?? null,
              runOrdinal: existingActivity?.runOrdinal ?? null,
            },
          },
        }
      : state.runtimeThreadActivity;
  const createdWorkspace = runtimeThreadWorkspaceFromEvent(event);
  const projects = createdWorkspace
    ? mergeWorkspaceProjects(state.projects, [createdWorkspace])
    : state.projects;
  const settledActiveRun =
    event.type === "run.settled" && event.runId === state.activeRun?.runId;
  let pendingRuntimeSubmissions = state.pendingRuntimeSubmissions;
  let pendingRuntimeNewThread = state.pendingRuntimeNewThread;
  const pendingWorkspace = pendingRuntimeNewThread?.workspace;
  let selectedThreadId =
    threadSummary?.archivedAt !== null && threadSummary?.id === state.selectedThreadId
      ? null
      : state.selectedThreadId;
  const archivedThreads = threadSummary
    ? reconcileArchivedThreadSummary(state.archivedThreads, threadSummary)
    : state.archivedThreads;
  const pendingCreateRequestId = pendingRuntimeNewThread?.createRequestId;
  if (
    pendingRuntimeNewThread &&
    pendingCreateRequestId &&
    event.seq > pendingRuntimeNewThread.afterSeq &&
    event.threadId &&
    event.branchId &&
    runtimeThreadCreateResultFromEvent(event, pendingCreateRequestId)
  ) {
    if (
      ownsRuntimeSendForeground(
        state,
        pendingRuntimeNewThread.epoch,
        null,
        pendingRuntimeNewThread.foregroundGeneration,
        pendingRuntimeNewThread.workspace,
      )
    ) {
      selectedThreadId = event.threadId;
    }
    pendingRuntimeSubmissions = {
      ...pendingRuntimeSubmissions,
      [event.threadId]: {
        ...pendingRuntimeNewThread,
        branchId: event.branchId,
        createdEvent: event,
      },
    };
    pendingRuntimeNewThread = null;
  }
  if (event.threadId && event.type === "item.completed" && event.runId && event.turnId) {
    const pending = pendingRuntimeSubmissions[event.threadId];
    const item = event.payload.item;
    const eventRequestId = event.payload.clientRequestId;
    const matchesSubmittedUserItem =
      typeof item === "object" &&
      item !== null &&
      "role" in item &&
      item.role === "user" &&
      "content" in item &&
      item.content === pending?.prompt &&
      event.seq > (pending?.afterSeq ?? event.seq) &&
      event.branchId === pending?.branchId &&
      eventRequestId === pending?.turnRequestId;
    if (pending && pending.runId === undefined && matchesSubmittedUserItem) {
      canonicalRuntimeTurnStarts.set(pending.turnRequestId, {
        threadId: event.threadId,
        branchId: event.branchId as string,
        turnId: event.turnId,
        runId: event.runId,
      });
      pendingRuntimeSubmissions = {
        ...pendingRuntimeSubmissions,
        [event.threadId]: {
          ...pending,
          acknowledged: true,
          runId: event.runId,
          turnId: event.turnId,
        },
      };
    }
  }
  if (event.threadId && (event.type === "run.state_changed" || event.type === "run.settled")) {
    const pending = pendingRuntimeSubmissions[event.threadId];
    if (pending?.acknowledged && pending.runId === event.runId) {
      pendingRuntimeSubmissions = { ...pendingRuntimeSubmissions };
      delete pendingRuntimeSubmissions[event.threadId];
    }
  }
  const projectedState = {
    ...state,
    threads,
    archivedThreads,
    runtimeThreadDetails,
    runtimeThreadActivity,
    pendingRuntimeSubmissions,
    pendingRuntimeNewThread,
    selectedThreadId,
  };
  return {
    threads,
    archivedThreads,
    projects,
    runtimeThreadDetails,
    runtimeThreadActivity,
    runtimeSeq: event.seq,
    runStatus: runtimeRunStatusForSelection(
      projectedState,
      projectedState.selectedThreadId,
      threads,
    ),
    activeRun: settledActiveRun ? null : state.activeRun,
    pendingRuntimeSubmissions,
    pendingRuntimeNewThread,
    ...(pendingRuntimeNewThread === null &&
    pendingWorkspace !== undefined &&
    sameWorkspace(state.newThreadWorkspace, pendingWorkspace)
      ? { newThreadWorkspace: null }
      : {}),
    selectedThreadId,
  };
}

function drainRuntimeEventQueue(set: StoreSet, get: StoreGet): void {
  let state = get();
  let changed = false;
  while (true) {
    const event = pendingRuntimeEvents.get(state.runtimeSeq + 1);
    if (!event) {
      break;
    }
    pendingRuntimeEvents.delete(event.seq);
    if (event.type === "run.settled" && event.runId) {
      cancellingRuntimeRuns.delete(event.runId);
    }
    state = { ...state, ...runtimeStateForEvent(state, event) };
    changed = true;
  }
  if (changed) {
    set({
      projects: state.projects,
      threads: state.threads,
      archivedThreads: state.archivedThreads,
      runtimeSeq: state.runtimeSeq,
      runtimeThreadDetails: state.runtimeThreadDetails,
      runtimeThreadActivity: state.runtimeThreadActivity,
      runStatus: state.runStatus,
      activeRun: state.activeRun,
      pendingRuntimeSubmissions: state.pendingRuntimeSubmissions,
      pendingRuntimeNewThread: state.pendingRuntimeNewThread,
      selectedThreadId: state.selectedThreadId,
    });
    for (const [threadId, pending] of Object.entries(
      state.pendingRuntimeSubmissions,
    )) {
      if (pending.branchId && !pending.turnStartClaimed) {
        void continueRuntimeSubmission(set, get, threadId, pending.epoch);
      }
    }
  }
}

function hasRuntimeEventGap(state: AppState): boolean {
  return pendingRuntimeEvents.size > 0 && !pendingRuntimeEvents.has(state.runtimeSeq + 1);
}

function waitForRuntimeRetry(delayMs: number): Promise<void> {
  return new Promise((resolve) => {
    globalThis.setTimeout(resolve, delayMs);
  });
}

function recordRuntimeThreadLoadEvent(event: RuntimeJournalEvent): void {
  if (!event.threadId) return;
  runtimeThreadLoadEvents.get(event.threadId)?.set(event.seq, event);
  runtimeOlderTurnLoadEvents.get(event.threadId)?.set(event.seq, event);
}

function mergeRecoveredRuntimeCatalogThread(existing: Thread, recovered: Thread): Thread {
  return {
    ...recovered,
    activeBranchId: existing.activeBranchId,
    branches: existing.branches,
    updatedAt:
      existing.updatedAt.localeCompare(recovered.updatedAt) > 0
        ? existing.updatedAt
        : recovered.updatedAt,
  };
}

function recordRuntimeCatalogRecoveryEvent(
  set: StoreSet,
  get: StoreGet,
  event: RuntimeJournalEvent,
): void {
  const threadId = event.threadId;
  if (!runtimeClient || !threadId) return;

  const observedEvents = runtimeCatalogRecoveryEvents.get(threadId);
  observedEvents?.set(event.seq, event);
  const threadSummary = runtimeThreadSummaryFromEvent(event);
  if (threadSummary?.archivedAt !== null && threadSummary?.archivedAt !== undefined) {
    return;
  }
  if (findThread(get().threads, threadId) || event.type === "thread.created") {
    return;
  }

  const recoveryEvents = observedEvents ?? new Map<number, RuntimeJournalEvent>();
  recoveryEvents.set(event.seq, event);
  runtimeCatalogRecoveryEvents.set(threadId, recoveryEvents);
  void recoverRuntimeCatalogThread(set, get, threadId, recoveryEvents);
}

async function recoverRuntimeCatalogThread(
  set: StoreSet,
  get: StoreGet,
  threadId: string,
  observedEvents: Map<number, RuntimeJournalEvent>,
): Promise<void> {
  if (!runtimeClient) return;
  const existing = runtimeCatalogRecoveries.get(threadId);
  if (existing) {
    return existing;
  }

  const recovery = (async () => {
    try {
      const metadata = await runtimeClient.getThread(threadId);
      if (metadata.thread.archivedAt !== null) {
        return;
      }
      let recoveredThreads = [projectRuntimeThread(metadata.thread)];
      const events = [...observedEvents.values()]
        .filter((event) => event.threadId === threadId && event.seq > metadata.snapshotSeq)
        .sort((left, right) => left.seq - right.seq);
      for (const event of events) {
        recoveredThreads = applyRuntimeCatalogEvent(recoveredThreads, event);
      }
      const recovered = findThread(recoveredThreads, threadId);
      if (!recovered) return;
      const recoveredSnapshotSeq = Math.max(
        metadata.snapshotSeq,
        events.at(-1)?.seq ?? 0,
      );

      set((state) => {
        const current = findThread(state.threads, threadId);
        const installed = current
          ? mergeRecoveredRuntimeCatalogThread(current, recovered)
          : recovered;
        const threads = sortRuntimeThreads(
          current
            ? state.threads.map((thread) => (thread.id === threadId ? installed : thread))
            : [installed, ...state.threads],
        );
        const currentDetail = state.runtimeThreadDetails[threadId];
        const runtimeThreadDetails =
          currentDetail?.status === "ready" || currentDetail?.status === "loading"
            ? state.runtimeThreadDetails
            : {
                ...state.runtimeThreadDetails,
                [threadId]: {
                  ...emptyRuntimeThreadDetail(recoveredSnapshotSeq, currentDetail),
                  status: "idle" as const,
                  error: null,
                },
              };
        return {
          projects: mergeRuntimeProjects(state.projects, [metadata.thread]),
          threads,
          runtimeThreadDetails,
          runStatus: runtimeRunStatusForSelection(
            { ...state, threads, runtimeThreadDetails },
            state.selectedThreadId,
            threads,
          ),
        };
      });
    } catch (error: unknown) {
      if (!findThread(get().threads, threadId)) {
        set({
          runtimeError:
            error instanceof Error ? error.message : "Runtime Thread catalog recovery failed",
          runtimeIssue: null,
        });
      }
    } finally {
      if (runtimeCatalogRecoveryEvents.get(threadId) === observedEvents) {
        runtimeCatalogRecoveryEvents.delete(threadId);
      }
      runtimeCatalogRecoveries.delete(threadId);
    }
  })();
  runtimeCatalogRecoveries.set(threadId, recovery);
  return recovery;
}

function projectLoadedRuntimeThread(
  loaded: Awaited<ReturnType<typeof loadRuntimeThreadHistory>>,
  events: readonly RuntimeJournalEvent[],
): Thread {
  const turnCoverage = new Map(
    loaded.turns.map(({ turn, snapshotSeq }) => [turn.id, snapshotSeq]),
  );
  let projected = projectRuntimeThreadHistory(
    loaded.thread,
    loaded.turns.map(({ turn }) => turn),
  );
  for (const event of events) {
    const detailCoverage = event.turnId
      ? (turnCoverage.get(event.turnId) ?? loaded.historySnapshotSeq)
      : loaded.historySnapshotSeq;
    const detailCovered = event.seq <= detailCoverage;
    const metadataCovered = event.seq <= loaded.metadataSnapshotSeq;
    if (!detailCovered) {
      projected = projectRuntimeEvent([projected], event)[0] ?? projected;
    } else if (!metadataCovered) {
      projected = applyRuntimeCatalogEvent([projected], event)[0] ?? projected;
    }
  }
  return projected;
}

function snapshotRuntimeThreadActivities(
  loaded: { turns: readonly LoadedRuntimeTurnHistory[] },
): RuntimeThreadActivityState {
  const activities: RuntimeThreadActivityState = {};
  for (const { turn, snapshotSeq } of loaded.turns) {
    for (const [runIndex, run] of turn.runs.entries()) {
      const status = runtimeActivityStatus(run.status);
      if (!status) continue;
      activities[run.id] = {
        seq: snapshotSeq,
        branchId: turn.branchId,
        turnId: turn.id,
        runId: run.id,
        status,
        turnOrdinal: turn.ordinal,
        runOrdinal: runIndex + 1,
      };
    }
  }
  return activities;
}

async function replayRuntimeEventsIntoQueue(
  set: StoreSet,
  get: StoreGet,
  afterSeq: number,
): Promise<void> {
  if (!runtimeClient) {
    return;
  }

  let cursor = afterSeq;
  while (true) {
    const replay = await runtimeClient.replayEvents(cursor);
    for (const event of replay.events) {
      recordRuntimeThreadLoadEvent(event);
      recordRuntimeCatalogRecoveryEvent(set, get, event);
      if (event.seq > get().runtimeSeq) {
        pendingRuntimeEvents.set(event.seq, event);
      }
    }
    drainRuntimeEventQueue(set, get);
    if (!replay.hasMore) {
      return;
    }

    const nextCursor = Math.max(replay.nextAfterSeq, get().runtimeSeq);
    if (nextCursor <= cursor) {
      throw new Error("Runtime event replay cursor did not advance.");
    }
    cursor = nextCursor;
  }
}

function recoverRuntimeEventGap(set: StoreSet, get: StoreGet): void {
  if (!runtimeClient || runtimeGapRecovery || !hasRuntimeEventGap(get())) {
    return;
  }
  const recovery = (async () => {
    let lastError: unknown;
    for (let attempt = 0; attempt <= RUNTIME_GAP_RETRY_DELAYS_MS.length; attempt += 1) {
      if (!hasRuntimeEventGap(get())) {
        return;
      }
      if (attempt > 0) {
        const retryDelayMs = RUNTIME_GAP_RETRY_DELAYS_MS[attempt - 1];
        if (retryDelayMs === undefined) {
          break;
        }
        await waitForRuntimeRetry(retryDelayMs);
        if (!hasRuntimeEventGap(get())) {
          return;
        }
      }
      try {
        await replayRuntimeEventsIntoQueue(set, get, get().runtimeSeq);
        lastError = undefined;
      } catch (error: unknown) {
        lastError = error;
      }
    }
    if (!hasRuntimeEventGap(get())) {
      return;
    }
    if (lastError) {
      throw lastError;
    }
    if (hasRuntimeEventGap(get())) {
      const firstPending = Math.min(...pendingRuntimeEvents.keys());
      set({
        runtimeError: `Runtime event sequence gap: expected ${get().runtimeSeq + 1}, received ${firstPending}.`,
        runtimeIssue: null,
      });
    }
  })()
    .catch((error: unknown) => {
      set({
        runtimeError:
          error instanceof Error ? error.message : "Runtime event replay failed",
        runtimeIssue: null,
      });
    })
    .finally(() => {
      if (runtimeGapRecovery === recovery) {
        runtimeGapRecovery = undefined;
      }
    });
  runtimeGapRecovery = recovery;
}

function enqueueRuntimeEvent(
  set: StoreSet,
  get: StoreGet,
  event: RuntimeJournalEvent,
): void {
  runtimeActiveCatalogLoadEvents?.set(event.seq, event);
  runtimeArchivedCatalogLoadEvents?.set(event.seq, event);
  recordRuntimeThreadLoadEvent(event);
  recordRuntimeCatalogRecoveryEvent(set, get, event);
  if (event.type === "run.settled" && event.runId) {
    cancellingRuntimeRuns.delete(event.runId);
  }
  if (event.seq <= get().runtimeSeq || pendingRuntimeEvents.has(event.seq)) {
    return;
  }
  pendingRuntimeEvents.set(event.seq, event);
  drainRuntimeEventQueue(set, get);
  recoverRuntimeEventGap(set, get);
}

function validateRuntimeThreadCatalogPage(
  page: Awaited<ReturnType<NonNullable<typeof runtimeClient>["listThreads"]>>,
  requestedCursor: string | null,
  previousSnapshotSeq: number,
  archived: boolean,
  previouslyLoadedIds: ReadonlySet<string> = new Set(),
): void {
  if (page.snapshotSeq < previousSnapshotSeq) {
    throw new Error("Runtime Thread catalog watermark moved backwards.");
  }
  if (page.hasMore && page.nextCursor === requestedCursor) {
    throw new Error("Runtime Thread catalog cursor did not advance.");
  }
  const ids = new Set<string>();
  for (const summary of page.threads) {
    if (ids.has(summary.id) || previouslyLoadedIds.has(summary.id)) {
      throw new Error("Runtime Thread catalog returned a duplicate Thread.");
    }
    if ((summary.archivedAt !== null) !== archived) {
      throw new Error("Runtime Thread catalog escaped its requested scope.");
    }
    ids.add(summary.id);
  }
}

function mergeActiveRuntimeCatalogPage(
  state: AppState,
  summaries: readonly RuntimeThreadSummary[],
  snapshotSeq: number,
  observedEvents: ReadonlyMap<number, RuntimeJournalEvent>,
): Pick<AppState, "projects" | "threads" | "runtimeThreadDetails"> {
  let threads = [...state.threads];
  const runtimeThreadDetails = { ...state.runtimeThreadDetails };
  for (const summary of summaries) {
    if (summary.archivedAt !== null) continue;
    const current = findThread(threads, summary.id);
    const projected = projectRuntimeThread(summary);
    if (!current) {
      threads.push(projected);
    } else if (summary.updatedAt.localeCompare(current.updatedAt) > 0) {
      threads = threads.map((thread) =>
        thread.id === summary.id
          ? {
              ...projected,
              activeBranchId: thread.activeBranchId,
              branches: thread.branches,
            }
          : thread,
      );
    }
    runtimeThreadDetails[summary.id] ??= emptyRuntimeThreadDetail(snapshotSeq);
  }
  for (const event of [...observedEvents.values()].sort(
    (left, right) => left.seq - right.seq,
  )) {
    if (event.seq > snapshotSeq) {
      threads = applyRuntimeCatalogEvent(threads, event);
    }
  }
  return {
    projects: mergeRuntimeProjects(state.projects, summaries),
    threads: sortRuntimeThreads(threads),
    runtimeThreadDetails,
  };
}

async function loadMoreRuntimeThreadsIntoStore(
  set: StoreSet,
  get: StoreGet,
): Promise<void> {
  if (!runtimeClient) return;
  if (runtimeActiveCatalogContinuation) return runtimeActiveCatalogContinuation;
  const initial = get();
  const cursor = initial.threadCatalogNextCursor;
  if (!initial.threadCatalogHasMore || !cursor) return;

  const observedEvents = new Map<number, RuntimeJournalEvent>();
  runtimeActiveCatalogLoadEvents = observedEvents;
  set({ threadCatalogMoreStatus: "loading", threadCatalogMoreError: null });
  const loading = (async () => {
    try {
      const page = await runtimeClient.listThreads({
        cursor,
        limit: RUNTIME_THREAD_CATALOG_PAGE_LIMIT,
      });
      validateRuntimeThreadCatalogPage(
        page,
        cursor,
        initial.threadCatalogSnapshotSeq,
        false,
        new Set(initial.threads.map((thread) => thread.id)),
      );
      set((state) => ({
        ...mergeActiveRuntimeCatalogPage(
          state,
          page.threads,
          page.snapshotSeq,
          observedEvents,
        ),
        threadCatalogNextCursor: page.nextCursor,
        threadCatalogHasMore: page.hasMore,
        threadCatalogSnapshotSeq: page.snapshotSeq,
        threadCatalogMoreStatus: "idle",
        threadCatalogMoreError: null,
      }));
    } catch (error: unknown) {
      set({
        threadCatalogMoreStatus: "error",
        threadCatalogMoreError:
          error instanceof Error ? error.message : "Thread catalog loading failed",
      });
    } finally {
      if (runtimeActiveCatalogLoadEvents === observedEvents) {
        runtimeActiveCatalogLoadEvents = undefined;
      }
      runtimeActiveCatalogContinuation = undefined;
    }
  })();
  runtimeActiveCatalogContinuation = loading;
  return loading;
}

async function loadAllRuntimeThreadsForSearch(
  set: StoreSet,
  get: StoreGet,
): Promise<void> {
  if (!runtimeClient) {
    set({ searchCatalogStatus: "ready", searchCatalogError: null });
    return;
  }
  if (runtimeSearchCatalogLoad) return runtimeSearchCatalogLoad;
  set({ searchCatalogStatus: "loading", searchCatalogError: null });
  const loading = (async () => {
    for (let pageNumber = 0; pageNumber < MAX_THREAD_CATALOG_PAGES_PER_SEARCH; pageNumber += 1) {
      if (!get().threadCatalogHasMore) {
        set({ searchCatalogStatus: "ready", searchCatalogError: null });
        return;
      }
      await loadMoreRuntimeThreadsIntoStore(set, get);
      const state = get();
      if (state.threadCatalogMoreStatus === "error") {
        set({
          searchCatalogStatus: "error",
          searchCatalogError: state.threadCatalogMoreError ?? "Thread catalog loading failed",
        });
        return;
      }
    }
    set({
      searchCatalogStatus: "error",
      searchCatalogError: "Runtime Thread catalog exceeded the page limit.",
    });
  })().finally(() => {
    runtimeSearchCatalogLoad = undefined;
  });
  runtimeSearchCatalogLoad = loading;
  return loading;
}

function mergeArchivedRuntimeCatalogPage(
  current: readonly RuntimeThreadSummary[],
  summaries: readonly RuntimeThreadSummary[],
  snapshotSeq: number,
  observedEvents: ReadonlyMap<number, RuntimeJournalEvent>,
): RuntimeThreadSummary[] {
  let archivedThreads = [...current];
  for (const summary of summaries) {
    const existing = archivedThreads.find((thread) => thread.id === summary.id);
    if (!existing || summary.updatedAt.localeCompare(existing.updatedAt) > 0) {
      archivedThreads = reconcileArchivedThreadSummary(archivedThreads, summary);
    }
  }
  for (const event of [...observedEvents.values()].sort(
    (left, right) => left.seq - right.seq,
  )) {
    if (event.seq <= snapshotSeq) continue;
    const summary = runtimeThreadSummaryFromEvent(event);
    if (summary) {
      archivedThreads = reconcileArchivedThreadSummary(archivedThreads, summary);
    }
  }
  return archivedThreads;
}

async function loadArchivedThreadsIntoStore(
  set: StoreSet,
  get: StoreGet,
): Promise<void> {
  if (!runtimeClient) {
    set({ archivedCatalogStatus: "ready" });
    return;
  }
  if (get().archivedCatalogStatus === "ready") return;
  if (runtimeArchivedCatalogLoad) return runtimeArchivedCatalogLoad;

  const observedEvents = new Map<number, RuntimeJournalEvent>();
  runtimeArchivedCatalogLoadEvents = observedEvents;
  set((state) => ({
    archivedCatalogStatus: "loading",
    ...(state.runtimeIssue?.kind === "archived_catalog"
      ? { runtimeError: null, runtimeIssue: null }
      : {}),
  }));
  const loading = (async () => {
    try {
      const listed = await runtimeClient.listThreads({
        archived: true,
        limit: RUNTIME_THREAD_CATALOG_PAGE_LIMIT,
      });
      validateRuntimeThreadCatalogPage(listed, null, 0, true);
      set((state) => ({
        archivedThreads: mergeArchivedRuntimeCatalogPage(
          [],
          listed.threads,
          listed.snapshotSeq,
          observedEvents,
        ),
        archivedCatalogStatus: "ready",
        archivedCatalogNextCursor: listed.nextCursor,
        archivedCatalogHasMore: listed.hasMore,
        archivedCatalogSnapshotSeq: listed.snapshotSeq,
        archivedCatalogMoreStatus: "idle",
        archivedCatalogMoreError: null,
        ...(state.runtimeIssue?.kind === "archived_catalog"
          ? { runtimeError: null, runtimeIssue: null }
          : {}),
      }));
    } catch (error: unknown) {
      const message =
        error instanceof Error ? error.message : "Archived Thread catalog loading failed";
      set({
        archivedCatalogStatus: "error",
        runtimeError: message,
        runtimeIssue: { kind: "archived_catalog", message },
      });
      throw error;
    } finally {
      if (runtimeArchivedCatalogLoadEvents === observedEvents) {
        runtimeArchivedCatalogLoadEvents = undefined;
      }
      runtimeArchivedCatalogLoad = undefined;
    }
  })();
  runtimeArchivedCatalogLoad = loading;
  return loading;
}

async function loadMoreArchivedThreadsIntoStore(
  set: StoreSet,
  get: StoreGet,
): Promise<void> {
  if (!runtimeClient) return;
  if (runtimeArchivedCatalogContinuation) return runtimeArchivedCatalogContinuation;
  const initial = get();
  const cursor = initial.archivedCatalogNextCursor;
  if (!initial.archivedCatalogHasMore || !cursor) return;

  const observedEvents = new Map<number, RuntimeJournalEvent>();
  runtimeArchivedCatalogLoadEvents = observedEvents;
  set({ archivedCatalogMoreStatus: "loading", archivedCatalogMoreError: null });
  const loading = (async () => {
    try {
      const page = await runtimeClient.listThreads({
        archived: true,
        cursor,
        limit: RUNTIME_THREAD_CATALOG_PAGE_LIMIT,
      });
      validateRuntimeThreadCatalogPage(
        page,
        cursor,
        initial.archivedCatalogSnapshotSeq,
        true,
        new Set(initial.archivedThreads.map((thread) => thread.id)),
      );
      set((state) => ({
        archivedThreads: mergeArchivedRuntimeCatalogPage(
          state.archivedThreads,
          page.threads,
          page.snapshotSeq,
          observedEvents,
        ),
        archivedCatalogNextCursor: page.nextCursor,
        archivedCatalogHasMore: page.hasMore,
        archivedCatalogSnapshotSeq: page.snapshotSeq,
        archivedCatalogMoreStatus: "idle",
        archivedCatalogMoreError: null,
      }));
    } catch (error: unknown) {
      set({
        archivedCatalogMoreStatus: "error",
        archivedCatalogMoreError:
          error instanceof Error ? error.message : "Archived Thread catalog loading failed",
      });
    } finally {
      if (runtimeArchivedCatalogLoadEvents === observedEvents) {
        runtimeArchivedCatalogLoadEvents = undefined;
      }
      runtimeArchivedCatalogContinuation = undefined;
    }
  })();
  runtimeArchivedCatalogContinuation = loading;
  return loading;
}

function reconcileRuntimeThreadMutation(
  state: AppState,
  summary: RuntimeThreadSummary,
): Partial<AppState> {
  const existing = findThread(state.threads, summary.id);
  const threads =
    summary.archivedAt !== null
      ? state.threads.filter((thread) => thread.id !== summary.id)
      : existing
        ? state.threads.map((thread) =>
            thread.id === summary.id
              ? {
                  ...thread,
                  projectId: summary.workspace?.id ?? null,
                  title: summary.title ?? "",
                  updatedAt: summary.updatedAt,
                }
              : thread,
          )
        : [projectRuntimeThread(summary), ...state.threads];
  const sortedThreads = sortRuntimeThreads(threads);
  const selectedThreadId =
    summary.archivedAt !== null && state.selectedThreadId === summary.id
      ? null
      : state.selectedThreadId;
  return {
    threads: sortedThreads,
    archivedThreads: reconcileArchivedThreadSummary(state.archivedThreads, summary),
    projects: mergeRuntimeProjects(state.projects, [summary]),
    selectedThreadId,
    runStatus: runtimeRunStatusForSelection(state, selectedThreadId, sortedThreads),
  };
}

function isMatchingThreadMutationIssue(
  current: RuntimeIssue | null,
  requested:
    | { kind: "archive" | "unarchive"; threadId: string }
    | { kind: "rename"; threadId: string; title: string | null },
): boolean {
  if (requested.kind === "rename") {
    return (
      current?.kind === "rename" &&
      current.threadId === requested.threadId &&
      current.title === requested.title
    );
  }
  return (
    current?.kind === requested.kind &&
    current.threadId === requested.threadId
  );
}

function localThreadSummary(
  state: AppState,
  thread: Thread,
  archivedAt: string | null,
): RuntimeThreadSummary {
  const project = thread.projectId
    ? state.projects.find((candidate) => candidate.id === thread.projectId)
    : undefined;
  const timestamp = new Date().toISOString();
  return {
    id: thread.id,
    title: thread.title || null,
    defaultBranchId: thread.activeBranchId,
    workspace: project
      ? {
          id: project.id,
          name: project.name,
          rootUri: project.rootUri ?? null,
        }
      : null,
    createdAt: thread.branches[0]?.createdAt ?? timestamp,
    updatedAt: timestamp,
    archivedAt,
  };
}

async function mutateRuntimeThread(
  set: StoreSet,
  get: StoreGet,
  invoke: () => Promise<RuntimeThreadMutationResult>,
  issue:
    | { kind: "archive" | "unarchive"; threadId: string }
    | { kind: "rename"; threadId: string; title: string | null },
): Promise<void> {
  if (!runtimeClient) return;
  set((state) =>
    isMatchingThreadMutationIssue(state.runtimeIssue, issue)
      ? { runtimeError: null, runtimeIssue: null }
      : {},
  );
  try {
    const result = await invoke();
    set((state) => ({
      ...reconcileRuntimeThreadMutation(state, result.thread),
      ...(isMatchingThreadMutationIssue(state.runtimeIssue, issue)
        ? { runtimeError: null, runtimeIssue: null }
        : {}),
    }));
    if (result.event) {
      get().applyRuntimeEvent(result.event);
    }
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : "Thread update failed";
    set({ runtimeError: message, runtimeIssue: { ...issue, message } });
    throw error;
  }
}

async function loadRuntimeThreadIntoStore(
  set: StoreSet,
  get: StoreGet,
  threadId: string,
): Promise<void> {
  if (!runtimeClient || get().runtimeThreadDetails[threadId]?.status === "ready") {
    return;
  }
  const existing = runtimeThreadLoads.get(threadId);
  if (existing) {
    return existing;
  }

  const observedEvents = new Map<number, RuntimeJournalEvent>();
  runtimeThreadLoadEvents.set(threadId, observedEvents);
  set((state) => ({
    ...(state.selectedThreadId === threadId &&
    state.runtimeIssue?.kind === "history" &&
    state.runtimeIssue.threadId === threadId
      ? { runtimeError: null, runtimeIssue: null }
      : {}),
    runtimeThreadDetails: {
      ...state.runtimeThreadDetails,
      [threadId]: {
        ...emptyRuntimeThreadDetail(
          state.runtimeThreadDetails[threadId]?.snapshotSeq ?? 0,
          state.runtimeThreadDetails[threadId],
        ),
        status: "loading",
        error: null,
      },
    },
  }));

  const loading = (async () => {
    try {
      const loaded = await loadRuntimeThreadHistory(runtimeClient, threadId);
      const replayAfterSeq = Math.min(
        loaded.metadataSnapshotSeq,
        loaded.historySnapshotSeq,
      );
      if (replayAfterSeq > 0) {
        await replayRuntimeEventsIntoQueue(set, get, replayAfterSeq);
      }
      const events = [...observedEvents.values()]
        .filter((event) => event.threadId === threadId)
        .sort((left, right) => left.seq - right.seq);
      const projected = projectLoadedRuntimeThread(loaded, events);
      const latestSnapshotSeq = Math.max(
        loaded.latestSnapshotSeq,
        events.at(-1)?.seq ?? 0,
      );
      const snapshotActivities = snapshotRuntimeThreadActivities(loaded);

      set((state) => {
        const existingIndex = state.threads.findIndex((thread) => thread.id === threadId);
        const threads = sortRuntimeThreads(
          existingIndex === -1
            ? [projected, ...state.threads]
            : state.threads.map((thread) => (thread.id === threadId ? projected : thread)),
        );
        const currentActivities = state.runtimeThreadActivity[threadId] ?? {};
        const mergedActivities = { ...snapshotActivities };
        for (const [runId, activity] of Object.entries(currentActivities)) {
          const snapshotActivity = mergedActivities[runId];
          if (!snapshotActivity) {
            mergedActivities[runId] = activity;
          } else if (activity.seq > snapshotActivity.seq) {
            mergedActivities[runId] = {
              ...activity,
              turnOrdinal: snapshotActivity.turnOrdinal,
              runOrdinal: snapshotActivity.runOrdinal,
            };
          }
        }
        const runtimeThreadActivity = {
          ...state.runtimeThreadActivity,
          [threadId]: mergedActivities,
        };
        return {
          projects: mergeRuntimeProjects(state.projects, [loaded.thread]),
          threads,
          runtimeThreadDetails: {
            ...state.runtimeThreadDetails,
            [threadId]: {
              status: "ready",
              snapshotSeq: latestSnapshotSeq,
              error: null,
              nextCursor: loaded.nextCursor,
              hasMore: loaded.hasMore,
              olderStatus: "idle",
              olderError: null,
              historySnapshotSeq: loaded.historySnapshotSeq,
              turnOrdinals: Object.fromEntries(
                loaded.turns.map(({ turn }) => [turn.id, turn.ordinal]),
              ),
            },
          },
          runtimeThreadActivity,
          ...(state.selectedThreadId === threadId &&
          state.runtimeIssue?.kind === "history" &&
          state.runtimeIssue.threadId === threadId
            ? { runtimeError: null, runtimeIssue: null }
            : {}),
          runStatus: runtimeRunStatusForSelection(
            { ...state, runtimeThreadActivity },
            state.selectedThreadId,
            threads,
          ),
        };
      });
    } catch (error: unknown) {
      const message =
        error instanceof Error ? error.message : "Runtime Thread history loading failed";
      set((state) => ({
        runtimeError: state.selectedThreadId === threadId ? message : state.runtimeError,
        runtimeIssue:
          state.selectedThreadId === threadId
            ? { kind: "history" as const, message, threadId }
            : state.runtimeIssue,
        runtimeThreadDetails: {
          ...state.runtimeThreadDetails,
          [threadId]: {
            ...emptyRuntimeThreadDetail(
              state.runtimeThreadDetails[threadId]?.snapshotSeq ?? 0,
              state.runtimeThreadDetails[threadId],
            ),
            status: "error",
            error: message,
          },
        },
      }));
    } finally {
      if (runtimeThreadLoadEvents.get(threadId) === observedEvents) {
        runtimeThreadLoadEvents.delete(threadId);
      }
      runtimeThreadLoads.delete(threadId);
    }
  })();
  runtimeThreadLoads.set(threadId, loading);
  return loading;
}

function runtimeSummaryForProjectedThread(
  state: AppState,
  thread: Thread,
): RuntimeThreadSummary {
  const project = thread.projectId
    ? state.projects.find((candidate) => candidate.id === thread.projectId)
    : undefined;
  const branch =
    thread.branches.find((candidate) => candidate.id === thread.activeBranchId) ??
    thread.branches[0];
  return {
    id: thread.id,
    title: thread.title || null,
    defaultBranchId: thread.activeBranchId,
    workspace: project
      ? { id: project.id, name: project.name, rootUri: project.rootUri ?? null }
      : null,
    createdAt: branch?.createdAt ?? thread.updatedAt,
    updatedAt: thread.updatedAt,
    archivedAt: null,
  };
}

async function loadOlderRuntimeTurnsIntoStore(
  set: StoreSet,
  get: StoreGet,
  threadId: string,
): Promise<void> {
  if (!runtimeClient) return;
  const existing = runtimeOlderTurnLoads.get(threadId);
  if (existing) return existing;

  const initial = get();
  const thread = findThread(initial.threads, threadId);
  const detail = initial.runtimeThreadDetails[threadId];
  const cursor = detail?.nextCursor;
  if (!thread || detail?.status !== "ready" || !detail.hasMore || !cursor) return;
  const branch =
    thread.branches.find((candidate) => candidate.id === thread.activeBranchId) ??
    thread.branches[0];
  if (!branch) return;

  const observedEvents = new Map<number, RuntimeJournalEvent>();
  runtimeOlderTurnLoadEvents.set(threadId, observedEvents);
  const existingTurnIds = new Set(branch.turns.map((turn) => turn.id));
  const existingOrdinals = new Set(Object.values(detail.turnOrdinals));
  set((state) => ({
    runtimeThreadDetails: {
      ...state.runtimeThreadDetails,
      [threadId]: {
        ...(state.runtimeThreadDetails[threadId] ?? detail),
        olderStatus: "loading",
        olderError: null,
      },
    },
  }));

  const loading = (async () => {
    try {
      const page = await loadOlderRuntimeTurnPage(runtimeClient, {
        threadId,
        branchId: branch.id,
        cursor,
        previousSnapshotSeq: detail.historySnapshotSeq,
        existingTurnIds,
        existingOrdinals,
      });
      let projectedOlder = projectRuntimeThreadHistory(
        runtimeSummaryForProjectedThread(initial, thread),
        page.turns.map(({ turn }) => turn),
      );
      for (const event of [...observedEvents.values()].sort(
        (left, right) => left.seq - right.seq,
      )) {
        if (event.threadId === threadId && event.seq > page.snapshotSeq) {
          projectedOlder = projectRuntimeEvent([projectedOlder], event)[0] ?? projectedOlder;
        }
      }
      const projectedBranch = projectedOlder.branches.find(
        (candidate) => candidate.id === branch.id,
      );
      const olderTurns = projectedBranch?.turns ?? [];
      const olderTurnIds = new Set(olderTurns.map((turn) => turn.id));
      const snapshotActivities = snapshotRuntimeThreadActivities(page);

      set((state) => {
        const current = findThread(state.threads, threadId);
        const currentDetail = state.runtimeThreadDetails[threadId];
        if (!current || !currentDetail) return {};
        const threads = state.threads.map((candidate) => {
          if (candidate.id !== threadId) return candidate;
          return {
            ...candidate,
            branches: candidate.branches.map((candidateBranch) =>
              candidateBranch.id === branch.id
                ? {
                    ...candidateBranch,
                    turns: [
                      ...olderTurns,
                      ...candidateBranch.turns.filter(
                        (turn) => !olderTurnIds.has(turn.id),
                      ),
                    ],
                  }
                : candidateBranch,
            ),
          };
        });
        const currentActivities = state.runtimeThreadActivity[threadId] ?? {};
        const mergedActivities = { ...snapshotActivities };
        for (const [runId, activity] of Object.entries(currentActivities)) {
          if (!mergedActivities[runId] || activity.seq > mergedActivities[runId].seq) {
            mergedActivities[runId] = activity;
          }
        }
        const runtimeThreadActivity = {
          ...state.runtimeThreadActivity,
          [threadId]: mergedActivities,
        };
        return {
          threads,
          runtimeThreadActivity,
          runtimeThreadDetails: {
            ...state.runtimeThreadDetails,
            [threadId]: {
              ...currentDetail,
              nextCursor: page.nextCursor,
              hasMore: page.hasMore,
              olderStatus: "idle" as const,
              olderError: null,
              historySnapshotSeq: page.snapshotSeq,
              turnOrdinals: {
                ...currentDetail.turnOrdinals,
                ...Object.fromEntries(
                  page.turns.map(({ turn }) => [turn.id, turn.ordinal]),
                ),
              },
            },
          },
          runStatus: runtimeRunStatusForSelection(
            { ...state, runtimeThreadActivity },
            state.selectedThreadId,
            threads,
          ),
        };
      });
    } catch (error: unknown) {
      const message =
        error instanceof Error ? error.message : "Older Runtime Turns loading failed";
      set((state) => {
        const currentDetail = state.runtimeThreadDetails[threadId];
        if (!currentDetail) return {};
        return {
          runtimeThreadDetails: {
            ...state.runtimeThreadDetails,
            [threadId]: {
              ...currentDetail,
              olderStatus: "error" as const,
              olderError: message,
            },
          },
        };
      });
    } finally {
      if (runtimeOlderTurnLoadEvents.get(threadId) === observedEvents) {
        runtimeOlderTurnLoadEvents.delete(threadId);
      }
      runtimeOlderTurnLoads.delete(threadId);
    }
  })();
  runtimeOlderTurnLoads.set(threadId, loading);
  return loading;
}

function applyRuntimeHostStatus(set: StoreSet, status: RuntimeHostStatus): void {
  set((state) => {
    if (status.state === "connected") {
      const clearsConnectionError =
        state.runtimeIssue?.kind === "connection" ||
        state.runtimeIssue?.kind === "initialization";
      return {
        runtimeConnectionStatus: "connected",
        ...(clearsConnectionError ? { runtimeError: null, runtimeIssue: null } : {}),
      };
    }
    if (status.state === "offline") {
      const message = status.message ?? "Runtime connection is unavailable.";
      return {
        runtimeConnectionStatus: "offline",
        runtimeError: message,
        runtimeIssue: { kind: "connection", message },
      };
    }
    return { runtimeConnectionStatus: status.state };
  });
}

async function retryRuntimeConnectionInStore(
  set: StoreSet,
  get: StoreGet,
): Promise<void> {
  if (!runtimeClient) return;
  const initialized = get().runtimeReady;
  set((state) => ({
    runtimeConnectionStatus: initialized ? "reconnecting" : "starting",
    ...(state.runtimeIssue?.kind === "connection" ||
    state.runtimeIssue?.kind === "initialization"
      ? { runtimeError: null, runtimeIssue: null }
      : {}),
  }));
  if (!initialized) {
    await initializeRuntimeInStore(set, get);
    return;
  }
  const catalogRevision = ++providerCatalogRevision;
  set({ providerCatalogStatus: "loading" });
  try {
    const [providerResult, modelResult] = await Promise.all([
      runtimeClient.listProviders(),
      runtimeClient.listModels(),
    ]);
    set((state) => ({
      runtimeConnectionStatus: "connected",
      ...(catalogRevision === providerCatalogRevision
        ? {
            providers: providerResult.providers,
            models: modelResult.models,
            selectedModel: reconcileModelSelection(
              state.selectedModel,
              providerResult.providers,
              modelResult.models,
            ),
            providerCatalogStatus: "ready" as const,
          }
        : {}),
      ...(state.runtimeIssue?.kind === "connection" ||
      state.runtimeIssue?.kind === "initialization"
        ? { runtimeError: null, runtimeIssue: null }
        : {}),
    }));
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : "Runtime connection failed";
    set((state) => ({
      runtimeConnectionStatus: "offline",
      runtimeError: message,
      runtimeIssue: { kind: "connection" as const, message },
      ...(catalogRevision === providerCatalogRevision
        ? { providerCatalogStatus: "error" as const }
        : { providerCatalogStatus: state.providerCatalogStatus }),
    }));
  }
}

async function retryRuntimeIssueInStore(set: StoreSet, get: StoreGet): Promise<void> {
  const issue = get().runtimeIssue;
  if (!issue || issue.message !== get().runtimeError) return;
  const clearIssue = (): void => {
    set((state) =>
      state.runtimeIssue === issue && state.runtimeError === issue.message
        ? { runtimeError: null, runtimeIssue: null }
        : {},
    );
  };
  switch (issue.kind) {
    case "connection":
    case "initialization":
      await retryRuntimeConnectionInStore(set, get);
      return;
    case "send": {
      const state = get();
      if (state.selectedThreadId !== issue.threadId || state.draft !== issue.prompt) {
        return;
      }
      clearIssue();
      await get().sendDraft();
      return;
    }
    case "cancel": {
      if (currentRunContext(get())?.runId !== issue.runId) {
        return;
      }
      clearIssue();
      get().stopRun();
      return;
    }
    case "history":
      clearIssue();
      await loadRuntimeThreadAndRestoreModel(set, get, issue.threadId);
      return;
    case "archived_catalog":
      clearIssue();
      await loadArchivedThreadsIntoStore(set, get);
      return;
    case "archive":
      clearIssue();
      await get().archiveThread(issue.threadId);
      return;
    case "unarchive":
      clearIssue();
      await get().unarchiveThread(issue.threadId);
      return;
    case "rename":
      clearIssue();
      await get().renameThread(issue.threadId, issue.title);
  }
}

async function initializeRuntimeInStore(set: StoreSet, get: StoreGet): Promise<void> {
  if (!runtimeClient || get().runtimeReady) {
    return;
  }
  if (runtimeInitialization) {
    return runtimeInitialization;
  }

  const initialization = (async () => {
    const initialModelSelectionRevision = modelSelectionRevision;
    set({ runtimeConnectionStatus: "starting", runtimeError: null, runtimeIssue: null });
    const catalogRevision = ++providerCatalogRevision;
    bufferedRuntimeEvents = [];
    removeRuntimeSubscription ??= runtimeClient.onEvent((event) => {
      if (get().runtimeReady) {
        get().applyRuntimeEvent(event);
      } else {
        bufferedRuntimeEvents.push(event);
      }
    });
    removeRuntimeStatusSubscription ??= runtimeClient.onStatus((status) => {
      applyRuntimeHostStatus(set, status);
    });

    const [listed, providerResult, modelResult] = await Promise.all([
      runtimeClient.listThreads({ limit: RUNTIME_THREAD_CATALOG_PAGE_LIMIT }),
      runtimeClient.listProviders(),
      runtimeClient.listModels(),
    ]);
    validateRuntimeThreadCatalogPage(listed, null, 0, false);
    pendingRuntimeEvents.clear();
    set((state) => ({
      projects: mergeRuntimeProjects(state.projects, listed.threads),
      threads: sortRuntimeThreads(projectRuntimeThreads(listed.threads)),
      threadCatalogNextCursor: listed.nextCursor,
      threadCatalogHasMore: listed.hasMore,
      threadCatalogSnapshotSeq: listed.snapshotSeq,
      threadCatalogMoreStatus: "idle",
      threadCatalogMoreError: null,
      searchCatalogStatus: "idle",
      searchCatalogError: null,
      runtimeThreadDetails: Object.fromEntries(
        listed.threads.map((thread) => [
          thread.id,
          emptyRuntimeThreadDetail(listed.snapshotSeq),
        ]),
      ),
      runtimeThreadActivity: {},
      runtimeSeq: listed.snapshotSeq,
      runStatus: "idle",
      activeRun: null,
    }));

    if (listed.snapshotSeq > 0) {
      await replayRuntimeEventsIntoQueue(set, get, listed.snapshotSeq);
    }
    for (const event of bufferedRuntimeEvents) {
      recordRuntimeCatalogRecoveryEvent(set, get, event);
      if (event.seq > get().runtimeSeq) {
        pendingRuntimeEvents.set(event.seq, event);
      }
    }
    bufferedRuntimeEvents = [];
    drainRuntimeEventQueue(set, get);

    const threads = get().threads;
    const selectedThreadId = threads.some((thread) => thread.id === get().selectedThreadId)
      ? get().selectedThreadId
      : null;
    set((state) => {
      const catalogIsCurrent = catalogRevision === providerCatalogRevision;
      const clearsInitializationIssue =
        state.runtimeIssue?.kind === "connection" ||
        state.runtimeIssue?.kind === "initialization";
      return {
        threads,
        selectedThreadId,
        runtimeReady: true,
        runtimeConnectionStatus: "connected",
        ...(catalogIsCurrent && clearsInitializationIssue
          ? { runtimeError: null, runtimeIssue: null }
          : {}),
        ...(catalogIsCurrent
          ? {
              providers: providerResult.providers,
              models: modelResult.models,
              selectedModel: reconcileModelSelection(
                modelSelectionRevision === initialModelSelectionRevision && selectedThreadId &&
                  state.runtimeThreadDetails[selectedThreadId]?.status === "ready"
                  ? lastThreadModel(findThread(state.threads, selectedThreadId))
                  : state.selectedModel,
                providerResult.providers,
                modelResult.models,
              ),
              providerCatalogStatus: "ready" as const,
            }
          : {}),
        runStatus: runtimeRunStatusForSelection(get(), selectedThreadId, threads),
      };
    });
    recoverRuntimeEventGap(set, get);
  })();
  runtimeInitialization = initialization;
  try {
    await initialization;
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : "Runtime initialization failed";
    set({
      runtimeConnectionStatus:
        get().runtimeConnectionStatus === "connected" ? "connected" : "offline",
      runtimeError: message,
      runtimeIssue: { kind: "initialization", message },
    });
  } finally {
    if (runtimeInitialization === initialization) {
      runtimeInitialization = undefined;
    }
  }
}

function recoveredThreadCreateResult(
  state: AppState,
  submission: PendingRuntimeSubmission,
): RuntimeThreadCreateResult | undefined {
  if (!submission.createRequestId) {
    return undefined;
  }
  const candidates = [
    state.pendingRuntimeNewThread,
    ...Object.values(state.pendingRuntimeSubmissions),
  ];
  const matched = candidates.find(
    (candidate) =>
      candidate?.epoch === submission.epoch &&
      candidate.createRequestId === submission.createRequestId &&
      candidate.createdEvent,
  );
  return matched?.createdEvent
    ? runtimeThreadCreateResultFromEvent(
        matched.createdEvent,
        submission.createRequestId,
      )
    : undefined;
}

function recoveredTurnStartResult(
  state: AppState,
  threadId: string,
  submission: PendingRuntimeSubmission,
): RuntimeTurnStartResult | undefined {
  const canonical = canonicalRuntimeTurnStarts.get(submission.turnRequestId);
  if (canonical) {
    return canonical;
  }
  const pending = state.pendingRuntimeSubmissions[threadId];
  if (
    pending?.epoch !== submission.epoch ||
    pending.turnRequestId !== submission.turnRequestId ||
    !pending.branchId ||
    !pending.turnId ||
    !pending.runId
  ) {
    return undefined;
  }
  return {
    threadId,
    branchId: pending.branchId,
    turnId: pending.turnId,
    runId: pending.runId,
  };
}

async function invokeIdempotentRuntimeCommand<TResult>(
  set: StoreSet,
  get: StoreGet,
  afterSeq: number,
  invoke: () => Promise<TResult>,
  recover: () => TResult | undefined,
): Promise<TResult> {
  let attempt = 0;
  while (true) {
    const alreadyRecovered = recover();
    if (alreadyRecovered) {
      return alreadyRecovered;
    }
    try {
      return await invoke();
    } catch (error: unknown) {
      const recoveredAfterInvoke = recover();
      if (recoveredAfterInvoke) {
        return recoveredAfterInvoke;
      }
      if (isRuntimeRpcError(error)) {
        throw error;
      }
    }

    const recoveredBeforeReplay = recover();
    if (recoveredBeforeReplay) {
      return recoveredBeforeReplay;
    }
    try {
      await replayRuntimeEventsIntoQueue(set, get, afterSeq);
    } catch {
      // A transport failure leaves acceptance ambiguous; the stable request ID makes retry safe.
    }
    const recoveredAfterReplay = recover();
    if (recoveredAfterReplay) {
      return recoveredAfterReplay;
    }
    const retryDelay =
      RUNTIME_COMMAND_RETRY_DELAYS_MS[
        Math.min(attempt, RUNTIME_COMMAND_RETRY_DELAYS_MS.length - 1)
      ] ?? 800;
    attempt += 1;
    await waitForRuntimeRetry(retryDelay);
  }
}

function adoptRuntimeCreatedThread(
  set: StoreSet,
  get: StoreGet,
  created: RuntimeThreadCreateResult,
  submission: PendingRuntimeSubmission,
): void {
  set((state) => {
    const pendingNew = state.pendingRuntimeNewThread;
    if (
      pendingNew?.epoch !== submission.epoch ||
      pendingNew.createRequestId !== submission.createRequestId
    ) {
      return state;
    }
    const threadId = created.thread.id;
    const threads = sortRuntimeThreads(
      state.threads.some((thread) => thread.id === threadId)
        ? state.threads
        : [projectRuntimeThread(created.thread), ...state.threads],
    );
    const ownsForeground = ownsRuntimeSendForeground(
      state,
      submission.epoch,
      null,
      submission.foregroundGeneration,
      submission.workspace,
    );
    return {
      projects: mergeRuntimeProjects(state.projects, [created.thread]),
      threads,
      runtimeThreadDetails: {
        ...state.runtimeThreadDetails,
        [threadId]: {
          status: "ready",
          snapshotSeq: created.event.seq,
          error: null,
          nextCursor: null,
          hasMore: false,
          olderStatus: "idle",
          olderError: null,
          historySnapshotSeq: created.event.seq,
          turnOrdinals: {},
        },
      },
      pendingRuntimeSubmissions: {
        ...state.pendingRuntimeSubmissions,
        [threadId]: {
          ...pendingNew,
          branchId: created.thread.defaultBranchId,
          createdEvent: created.event,
        },
      },
      pendingRuntimeNewThread: null,
      newThreadWorkspace: ownsForeground ? null : state.newThreadWorkspace,
      ...(ownsForeground
        ? { selectedThreadId: threadId, runStatus: "queued" as const }
        : {}),
    };
  });
  get().applyRuntimeEvent(created.event);
}

function handleRuntimeSubmissionFailure(
  set: StoreSet,
  get: StoreGet,
  submission: PendingRuntimeSubmission,
  sendingThreadId: string | null,
  error: unknown,
): void {
  const runtimeError = error instanceof Error ? error.message : "Runtime request failed";
  set((state) => {
    let pendingRuntimeSubmissions = state.pendingRuntimeSubmissions;
    let matchedThreadId = sendingThreadId;
    for (const [threadId, pending] of Object.entries(pendingRuntimeSubmissions)) {
      if (
        pending.epoch === submission.epoch &&
        pending.turnRequestId === submission.turnRequestId
      ) {
        pendingRuntimeSubmissions = { ...pendingRuntimeSubmissions };
        delete pendingRuntimeSubmissions[threadId];
        matchedThreadId = threadId;
        break;
      }
    }
    let pendingRuntimeNewThread = state.pendingRuntimeNewThread;
    if (
      pendingRuntimeNewThread?.epoch === submission.epoch &&
      pendingRuntimeNewThread.turnRequestId === submission.turnRequestId
    ) {
      pendingRuntimeNewThread = null;
    }
    const ownsForeground = ownsRuntimeSendForeground(
      state,
      submission.epoch,
      matchedThreadId,
      submission.foregroundGeneration,
    );
    const projectedState = {
      ...state,
      pendingRuntimeSubmissions,
      pendingRuntimeNewThread,
    };
    return {
      ...(ownsForeground || matchedThreadId !== null
        ? {
            runtimeError,
            runtimeIssue: {
              kind: "send" as const,
              message: runtimeError,
              threadId: matchedThreadId,
              prompt: submission.prompt,
            },
          }
        : {}),
      ...(ownsForeground
        ? {
            draft: submission.prompt,
            runStatus: "failed" as const,
            activeRun: null,
          }
        : {
            runStatus: runtimeRunStatusForSelection(
              projectedState,
              state.selectedThreadId,
            ),
          }),
      pendingRuntimeSubmissions,
      pendingRuntimeNewThread,
    };
  });
}

async function startRuntimeTurnForSubmission(
  set: StoreSet,
  get: StoreGet,
  threadId: string,
  submission: PendingRuntimeSubmission,
): Promise<void> {
  if (!runtimeClient || !submission.branchId) {
    return;
  }
  try {
    const started = await invokeIdempotentRuntimeCommand(
      set,
      get,
      submission.afterSeq,
      () =>
        runtimeClient.startTurn({
          threadId,
          branchId: submission.branchId as string,
          content: submission.prompt,
          providerId: submission.providerId,
          modelId: submission.modelId,
          clientRequestId: submission.turnRequestId,
        }),
      () => recoveredTurnStartResult(get(), threadId, submission),
    );
    const startedRun: RunContext = {
      threadId: started.threadId,
      branchId: started.branchId,
      turnId: started.turnId,
      runId: started.runId,
      epoch: submission.epoch,
    };
    canonicalRuntimeTurnStarts.delete(submission.turnRequestId);
    set((state) => {
      const projectedStatus = storedRunStatusForRun(
        findThread(state.threads, started.threadId),
        started.runId,
      );
      const pending = state.pendingRuntimeSubmissions[started.threadId];
      let pendingRuntimeSubmissions = state.pendingRuntimeSubmissions;
      if (
        pending?.epoch === submission.epoch &&
        pending.turnRequestId === submission.turnRequestId
      ) {
        pendingRuntimeSubmissions = { ...pendingRuntimeSubmissions };
        if (projectedStatus === undefined) {
          pendingRuntimeSubmissions[started.threadId] = {
            ...pending,
            acknowledged: true,
            runId: started.runId,
            turnId: started.turnId,
          };
        } else {
          delete pendingRuntimeSubmissions[started.threadId];
        }
      }
      const ownsForeground = ownsRuntimeSendForeground(
        state,
        submission.epoch,
        threadId,
        submission.foregroundGeneration,
      );
      const projectedState = { ...state, pendingRuntimeSubmissions };
      return {
        pendingRuntimeSubmissions,
        runStatus: ownsForeground
          ? projectedStatus ?? "queued"
          : runtimeRunStatusForSelection(projectedState, state.selectedThreadId),
        activeRun: ownsForeground
          ? projectedStatus === undefined || isRunActive(projectedStatus)
            ? startedRun
            : null
          : state.activeRun,
      };
    });
  } catch (error: unknown) {
    handleRuntimeSubmissionFailure(set, get, submission, threadId, error);
  }
}

function continueRuntimeSubmission(
  set: StoreSet,
  get: StoreGet,
  threadId: string,
  epoch: number,
): Promise<void> {
  const initial = get().pendingRuntimeSubmissions[threadId];
  if (!initial || initial.epoch !== epoch || !initial.branchId) {
    return Promise.resolve();
  }
  const existing = runtimeTurnContinuations.get(initial.turnRequestId);
  if (existing) {
    return existing;
  }

  let claimed: PendingRuntimeSubmission | undefined;
  set((state) => {
    const pending = state.pendingRuntimeSubmissions[threadId];
    if (
      !pending ||
      pending.epoch !== epoch ||
      pending.turnStartClaimed ||
      !pending.branchId
    ) {
      return state;
    }
    claimed = { ...pending, turnStartClaimed: true };
    return {
      pendingRuntimeSubmissions: {
        ...state.pendingRuntimeSubmissions,
        [threadId]: claimed,
      },
    };
  });
  if (!claimed) {
    return runtimeTurnContinuations.get(initial.turnRequestId) ?? Promise.resolve();
  }

  let continuation: Promise<void>;
  continuation = startRuntimeTurnForSubmission(set, get, threadId, claimed).finally(
    () => {
      if (runtimeTurnContinuations.get(claimed?.turnRequestId ?? "") === continuation) {
        runtimeTurnContinuations.delete(claimed?.turnRequestId ?? "");
      }
    },
  );
  runtimeTurnContinuations.set(claimed.turnRequestId, continuation);
  return continuation;
}

async function sendRuntimeDraft(
  set: StoreSet,
  get: StoreGet,
  prompt: string,
  selectedThreadIdAtSend: string | null,
  selectedModelAtSend: RuntimeModelSelection | null,
): Promise<void> {
  if (!runtimeClient) {
    return;
  }
  await get().initializeRuntime();
  const initial = get();
  if (!initial.runtimeReady) {
    return;
  }

  if (!selectionIsRunnable(selectedModelAtSend, initial.providers, initial.models)) {
    const message = "No configured model is selected.";
    set({
      runtimeError: message,
      runtimeIssue: {
        kind: "send",
        message,
        threadId: selectedThreadIdAtSend,
        prompt,
      },
    });
    return;
  }

  const selectionAtSend = selectedThreadIdAtSend;
  if (hasPendingRuntimeSubmission(initial, selectionAtSend)) {
    return;
  }

  const epoch = initial.runEpoch + 1;
  const thread = findThread(initial.threads, selectionAtSend);
  const submission: PendingRuntimeSubmission = {
    epoch,
    branchId: thread?.activeBranchId ?? null,
    prompt,
    providerId: selectedModelAtSend.providerId,
    modelId: selectedModelAtSend.modelId,
    afterSeq: initial.runtimeSeq,
    foregroundGeneration: initial.runtimeForegroundGeneration,
    workspace: thread ? null : initial.newThreadWorkspace,
    createRequestId: thread ? undefined : newRuntimeRequestId("thread"),
    turnRequestId: newRuntimeRequestId("turn"),
    turnStartClaimed: false,
  };
  set((state) => ({
    draft: "",
    runStatus: "queued",
    runtimeError: null,
    runtimeIssue: null,
    runEpoch: epoch,
    pendingRuntimeSubmissions:
      selectionAtSend === null
        ? state.pendingRuntimeSubmissions
        : {
            ...state.pendingRuntimeSubmissions,
            [selectionAtSend]: submission,
          },
    pendingRuntimeNewThread:
      selectionAtSend === null ? submission : state.pendingRuntimeNewThread,
  }));

  if (thread) {
    await continueRuntimeSubmission(set, get, thread.id, epoch);
    return;
  }

  const title = prompt.length > 42 ? `${prompt.slice(0, 42)}…` : prompt;
  try {
    const created = await invokeIdempotentRuntimeCommand(
      set,
      get,
      submission.afterSeq,
      () =>
        runtimeClient.createThread({
          title,
          workspace: submission.workspace,
          clientRequestId: submission.createRequestId,
        }),
      () => recoveredThreadCreateResult(get(), submission),
    );
    adoptRuntimeCreatedThread(set, get, created, submission);
    await continueRuntimeSubmission(set, get, created.thread.id, epoch);
  } catch (error: unknown) {
    handleRuntimeSubmissionFailure(set, get, submission, null, error);
  }
}

export const useAppStore = create<AppState>()((set, get) => ({
  runtimeMode: runtimeClient !== null,
  runtimeReady: runtimeClient === null,
  runtimeConnectionStatus: runtimeClient === null ? "connected" : "starting",
  runtimeError: null,
  runtimeIssue: null,
  runtimeSeq: 0,
  providerCatalogStatus: runtimeClient === null ? "ready" : "idle",
  providers: [],
  models: [],
  skillCatalogStatus: runtimeClient === null ? "ready" : "idle",
  skillCatalogError: null,
  skills: [],
  skillDiagnostics: [],
  selectedModel: null,
  projects: runtimeClient ? [] : createInitialProjects(),
  threads: runtimeClient ? [] : createInitialThreads(),
  threadCatalogNextCursor: null,
  threadCatalogSnapshotSeq: 0,
  threadCatalogHasMore: false,
  threadCatalogMoreStatus: "idle",
  threadCatalogMoreError: null,
  searchCatalogStatus: runtimeClient ? "idle" : "ready",
  searchCatalogError: null,
  archivedThreads: [],
  archivedCatalogNextCursor: null,
  archivedCatalogSnapshotSeq: 0,
  archivedCatalogStatus: "idle",
  archivedCatalogHasMore: false,
  archivedCatalogMoreStatus: "idle",
  archivedCatalogMoreError: null,
  runtimeThreadDetails: {},
  runtimeThreadActivity: {},
  selectedThreadId: null,
  runStatus: "idle",
  draft: "",
  fileSelection: null,
  sidebarOpen: true,
  sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
  searchOpen: false,
  settingsOpen: false,
  settingsInitialSection: "general",
  profileUsername: LOCAL_PROFILE.name,
  editingMessage: null,
  expandedProjects: {
    "project-research": true,
    "project-personal": true,
  },
  runEpoch: 0,
  runtimeForegroundGeneration: 0,
  localThreadCounter: 0,
  branchCounter: 0,
  activeRun: null,
  pendingRuntimeSubmissions: {},
  pendingRuntimeNewThread: null,
  newThreadWorkspace: null,

  initializeRuntime: () => initializeRuntimeInStore(set, get),
  retryRuntimeConnection: () => retryRuntimeConnectionInStore(set, get),
  retryRuntimeIssue: () => retryRuntimeIssueInStore(set, get),
  clearRuntimeError: () => set({ runtimeError: null, runtimeIssue: null }),
  applyRuntimeEvent: (event) => enqueueRuntimeEvent(set, get, event),
  loadMoreThreads: () => loadMoreRuntimeThreadsIntoStore(set, get),
  loadAllThreadsForSearch: () => loadAllRuntimeThreadsForSearch(set, get),
  loadArchivedThreads: () => loadArchivedThreadsIntoStore(set, get),
  loadMoreArchivedThreads: () => loadMoreArchivedThreadsIntoStore(set, get),
  renameThread: async (threadId, title) => {
    if (runtimeClient) {
      await mutateRuntimeThread(set, get, () =>
        runtimeClient.renameThread({ threadId, title }),
        { kind: "rename", threadId, title },
      );
      return;
    }
    set((state) => ({
      threads: state.threads.map((thread) =>
        thread.id === threadId ? { ...thread, title: title ?? "" } : thread,
      ),
      archivedThreads: state.archivedThreads.map((thread) =>
        thread.id === threadId ? { ...thread, title } : thread,
      ),
    }));
  },
  archiveThread: async (threadId) => {
    if (runtimeClient) {
      await mutateRuntimeThread(
        set,
        get,
        () => runtimeClient.archiveThread(threadId),
        { kind: "archive", threadId },
      );
      return;
    }
    set((state) => {
      const thread = findThread(state.threads, threadId);
      if (!thread) return state;
      const summary = localThreadSummary(state, thread, new Date().toISOString());
      const threads = state.threads.filter((candidate) => candidate.id !== threadId);
      const selectedThreadId = state.selectedThreadId === threadId ? null : state.selectedThreadId;
      return {
        threads,
        archivedThreads: reconcileArchivedThreadSummary(state.archivedThreads, summary),
        selectedThreadId,
        runStatus: runtimeRunStatusForSelection(state, selectedThreadId, threads),
      };
    });
  },
  unarchiveThread: async (threadId) => {
    if (runtimeClient) {
      await mutateRuntimeThread(
        set,
        get,
        () => runtimeClient.unarchiveThread(threadId),
        { kind: "unarchive", threadId },
      );
      return;
    }
    set((state) => {
      const summary = state.archivedThreads.find((thread) => thread.id === threadId);
      if (!summary) return state;
      const restored = { ...summary, archivedAt: null, updatedAt: new Date().toISOString() };
      return {
        threads: [projectRuntimeThread(restored), ...state.threads],
        archivedThreads: state.archivedThreads.filter((thread) => thread.id !== threadId),
      };
    });
  },
  loadProviderCatalog: () => refreshRuntimeProviderCatalog(set),
  discoverDeepSeekModels: async (apiKey) => {
    if (!runtimeClient) {
      throw new Error("Runtime provider model discovery is unavailable.");
    }
    const result = await runtimeClient.discoverProviderModels({
      kind: "deepseek",
      apiKey,
    });
    return result.models;
  },
  configureProvider: (params) =>
    mutateRuntimeProviderCatalog(set, () =>
      runtimeClient?.configureProvider(params) ?? Promise.resolve(),
    ),
  disconnectProvider: (providerId) =>
    mutateRuntimeProviderCatalog(set, () =>
      runtimeClient?.disconnectProvider(providerId) ?? Promise.resolve(),
    ),
  removeProvider: (providerId) =>
    mutateRuntimeProviderCatalog(set, () =>
      runtimeClient?.removeProvider(providerId) ?? Promise.resolve(),
    ),
  setModelEnabled: (params) =>
    mutateRuntimeProviderCatalog(set, () =>
      runtimeClient?.setModelEnabled(params) ?? Promise.resolve(),
    ),
  loadSkillCatalog: () => refreshRuntimeSkillCatalog(set),
  setSkillEnabled: (params) => mutateRuntimeSkillEnabled(set, params),
  selectModel: (selection) => {
    modelSelectionRevision += 1;
    set((state) => ({
      selectedModel: reconcileModelSelection(selection, state.providers, state.models),
    }));
  },

  setDraft: (draft) => set({ draft }),
  setModelLimits: async (params) => {
    if (!runtimeClient) return;
    const result = await runtimeClient.setModelLimits(params);
    set((state) => ({ models: state.models.map((model) =>
      model.providerId === result.model.providerId && model.id === result.model.id ? result.model : model) }));
  },
  openFile: (fileSelection) => set({ fileSelection }),
  closeFile: () => set({ fileSelection: null }),
  setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }),
  setSidebarWidth: (sidebarWidth) => set({ sidebarWidth }),
  setSearchOpen: (searchOpen) => set({ searchOpen }),
  setProfileUsername: (profileUsername) => set({ profileUsername }),
  setSettingsOpen: (settingsOpen, section) =>
    set((state) => ({
      settingsOpen,
      settingsInitialSection: settingsOpen ? section ?? "general" : state.settingsInitialSection,
      searchOpen: settingsOpen ? false : state.searchOpen,
      editingMessage: settingsOpen ? null : state.editingMessage,
    })),
  toggleProject: (projectId) =>
    set((state) => ({
      expandedProjects: {
        ...state.expandedProjects,
        [projectId]: !state.expandedProjects[projectId],
      },
    })),
  beginEditMessage: (eventId, content) => {
    if (isRunActive(get().runStatus)) return;
    set({ editingMessage: { eventId, content } });
  },
  cancelEditMessage: () => set({ editingMessage: null }),

  commitMessageEdit: (content) => {
    const state = get();
    const threadId = state.selectedThreadId;
    const editing = state.editingMessage;
    if (isRunActive(state.runStatus) || !threadId || !editing || !content.trim()) return;
    if (runtimeClient) {
      set({ editingMessage: null });
      return;
    }
    const branchCounter = state.branchCounter + 1;
    const branchId = `${threadId}-branch-${branchCounter}`;
    set({
      threads: updateThread(state.threads, threadId, (thread) =>
        createBranchFromMessage(
          thread,
          editing.eventId,
          content.trim(),
          branchId,
        ),
      ),
      editingMessage: null,
      runStatus: "completed",
      branchCounter,
    });
  },

  switchBranch: (branchId) => {
    const state = get();
    const { threads, selectedThreadId } = state;
    if (!selectedThreadId) return;
    const thread = findThread(threads, selectedThreadId);
    const branch = thread?.branches.find((candidate) => candidate.id === branchId);
    if (!thread || !branch) return;
    set({
      threads: updateThread(threads, selectedThreadId, (thread) => ({
        ...thread,
        activeBranchId: branchId,
      })),
      runStatus: state.activeRun ? state.runStatus : branch.turns.at(-1)?.status ?? "idle",
      editingMessage: null,
    });
  },

  newChat: () => {
    if (runtimeClient) {
      set((state) => ({
        selectedThreadId: null,
        selectedModel: reconcileModelSelection(null, state.providers, state.models),
        runStatus: state.pendingRuntimeNewThread ? "queued" : "idle",
        draft: "",
        editingMessage: null,
        settingsOpen: false,
        newThreadWorkspace: null,
        runtimeForegroundGeneration: state.runtimeForegroundGeneration + 1,
      }));
      return;
    }
    client.cancel();
    set((state) => ({
      threads: terminalizeCurrentRun(state),
      selectedThreadId: null,
      runStatus: "idle",
      draft: "",
      editingMessage: null,
      settingsOpen: false,
      runEpoch: state.runEpoch + 1,
      activeRun: null,
    }));
  },

  newProjectChat: (projectId) => {
    const state = get();
    const workspace = workspaceForProject(state.projects, projectId);
    if (!workspace) return;
    if (runtimeClient) {
      set((current) => ({
        selectedThreadId: null,
        selectedModel: reconcileModelSelection(null, current.providers, current.models),
        runStatus: current.pendingRuntimeNewThread ? "queued" : "idle",
        draft: "",
        editingMessage: null,
        settingsOpen: false,
        newThreadWorkspace: workspace,
        expandedProjects: { ...current.expandedProjects, [projectId]: true },
        runtimeForegroundGeneration: current.runtimeForegroundGeneration + 1,
      }));
      return;
    }
    get().newChat();
  },

  stageProjectWorkspace: (workspace) => {
    set((state) => {
      const existingWorkspace = workspaceForProject(state.projects, workspace.id);
      const canonicalWorkspace = existingWorkspace ?? workspace;
      return {
        projects: existingWorkspace
          ? state.projects
          : mergeWorkspaceProjects(state.projects, [canonicalWorkspace]),
        selectedThreadId: null,
        selectedModel: reconcileModelSelection(null, state.providers, state.models),
        runStatus: state.pendingRuntimeNewThread ? "queued" : "idle",
        draft: "",
        editingMessage: null,
        settingsOpen: false,
        newThreadWorkspace: canonicalWorkspace,
        expandedProjects: {
          ...state.expandedProjects,
          [canonicalWorkspace.id]: true,
        },
        runtimeForegroundGeneration: state.runtimeForegroundGeneration + 1,
      };
    });
  },

  bindWorkspaceFromFolder: async () => {
    const desktop = (
      window as unknown as Window & {
        ikarosDesktop?: import("../shared/platform").IkarosDesktopApi;
      }
    ).ikarosDesktop;
    if (!desktop) {
      throw new Error("Desktop workspace picker is unavailable.");
    }
    const workspace = await desktop.workspace.chooseDirectory();
    if (!workspace) return;
    get().stageProjectWorkspace(workspace);
  },

  selectThread: async (threadId) => {
    const state = get();
    const thread = findThread(state.threads, threadId);
    if (!thread) return;

    if (runtimeClient) {
      const detailStatus = state.runtimeThreadDetails[threadId]?.status;
      const alreadySelected = state.selectedThreadId === threadId &&
        (detailStatus === "ready" || detailStatus === "loading");
      set((current) => ({
        selectedThreadId: threadId,
        selectedModel: alreadySelected ? current.selectedModel
          : current.runtimeThreadDetails[threadId]?.status === "ready"
            ? reconcileModelSelection(lastThreadModel(thread), current.providers, current.models)
            : null,
        runStatus: runtimeRunStatusForSelection(current, threadId),
        editingMessage: null,
        searchOpen: false,
        settingsOpen: false,
        runtimeForegroundGeneration: current.runtimeForegroundGeneration + 1,
      }));
      if (alreadySelected) await loadRuntimeThreadIntoStore(set, get, threadId);
      else await loadRuntimeThreadAndRestoreModel(set, get, threadId);
      return;
    }

    if (state.selectedThreadId === threadId) {
      set({ searchOpen: false, settingsOpen: false });
      if (thread.scenarioId && storedRunStatus(thread) === "queued") {
        await playScenarioInStore(set, get, thread.id, thread.scenarioId);
      }
      return;
    }

    if (thread.scenarioId && storedRunStatus(thread) === "queued") {
      await playScenarioInStore(set, get, thread.id, thread.scenarioId);
      return;
    }

    client.cancel();
    const epoch = state.runEpoch + 1;
    const threads = terminalizeCurrentRun(state);
    const storedThread = findThread(threads, threadId);
    set({
      threads,
      selectedThreadId: threadId,
      runStatus: storedRunStatus(storedThread),
      editingMessage: null,
      searchOpen: false,
      settingsOpen: false,
      runEpoch: epoch,
      activeRun: null,
    });
  },

  retryRuntimeThread: (threadId) => loadRuntimeThreadAndRestoreModel(set, get, threadId),
  loadOlderRuntimeTurns: (threadId) =>
    loadOlderRuntimeTurnsIntoStore(set, get, threadId),

  sendDraft: async () => {
    const state = get();
    const prompt = state.draft.trim();
    if (!prompt || isRunActive(state.runStatus)) return;
    if (runtimeClient) {
      if (state.selectedThreadId &&
        state.runtimeThreadDetails[state.selectedThreadId]?.status !== "ready") return;
      if (hasPendingRuntimeSubmission(state, state.selectedThreadId)) {
        return;
      }
      await sendRuntimeDraft(
        set,
        get,
        prompt,
        state.selectedThreadId,
        state.selectedModel,
      );
      return;
    }
    client.cancel();
    const epoch = state.runEpoch + 1;
    let threadId = state.selectedThreadId;
    let branchId: string;
    let turnId: string;
    let threads = state.threads;
    let localThreadCounter = state.localThreadCounter;

    if (!threadId) {
      localThreadCounter += 1;
      threadId = `thread-local-${localThreadCounter}`;
      branchId = `${threadId}-main`;
      turnId = `${threadId}-turn-1`;
      const newThread: Thread = {
        id: threadId,
        projectId: null,
        title: prompt.length > 42 ? `${prompt.slice(0, 42)}…` : prompt,
        activeBranchId: branchId,
        updatedAt: MOCK_AT,
        branches: [
          {
            id: branchId,
            threadId,
            label: { source: "app", kind: "main" },
            createdAt: MOCK_AT,
            turns: [
              {
                id: turnId,
                branchId,
                status: "running",
                events: [
                  {
                    id: `${turnId}-user`,
                    turnId,
                    type: "message",
                    role: "user",
                    content: prompt,
                    status: "complete",
                    createdAt: MOCK_AT,
                  },
                ],
              },
            ],
          },
        ],
      };
      threads = [newThread, ...threads];
    } else {
      const thread = findThread(threads, threadId);
      const branch = activeBranch(thread);
      if (!thread || !branch) return;
      branchId = branch.id;
      turnId = `${thread.id}-turn-${branch.turns.length + 1}`;
      const userEvent: AgentEvent = {
        id: `${turnId}-user`,
        turnId,
        type: "message",
        role: "user",
        content: prompt,
        status: "complete",
        createdAt: MOCK_AT,
      };
      threads = updateThread(threads, thread.id, (candidate) =>
        appendEventToThread(candidate, branch.id, userEvent, "running"),
      );
    }

    const context: RunContext = { threadId, branchId, turnId, epoch };

    set({
      threads,
      selectedThreadId: threadId,
      draft: "",
      runStatus: "running",
      runEpoch: epoch,
      localThreadCounter,
      activeRun: context,
    });

    const terminal = await client.submitPrompt(prompt, turnId, (event, status) => {
      if (get().runEpoch !== epoch) return;
      get().appendAgentEvent(threadId, branchId, event, status);
    });
    finishRun(set, get, context, terminal);
  },

  stopRun: () => {
    if (runtimeClient) {
      const state = get();
      const context = currentRunContext(state);
      if (
        !context?.runId ||
        !isRunCancelable(state.runStatus) ||
        cancellingRuntimeRuns.has(context.runId)
      ) {
        return;
      }
      cancellingRuntimeRuns.add(context.runId);
      set((state) =>
        state.runtimeIssue?.kind === "cancel" &&
        state.runtimeIssue.runId === context.runId
          ? { runtimeError: null, runtimeIssue: null }
          : {},
      );
      void runtimeClient
        .cancelRun(context.runId)
        .then(async (result) => {
          if (result.accepted) {
            return;
          }
          await replayRuntimeEventsIntoQueue(set, get, get().runtimeSeq);
          cancellingRuntimeRuns.delete(context.runId as string);
          recoverRuntimeEventGap(set, get);
        })
        .catch((error: unknown) => {
          cancellingRuntimeRuns.delete(context.runId as string);
          const message =
            error instanceof Error ? error.message : "Runtime cancellation failed";
          set((state) =>
            currentRunContext(state)?.runId === context.runId
              ? {
                  runtimeError: message,
                  runtimeIssue: { kind: "cancel", message, runId: context.runId as string },
                }
              : {},
          );
        });
      return;
    }
    const state = get();
    const context = currentRunContext(state);
    if (!context || !isRunCancelable(state.runStatus)) return;
    client.cancel();
    const event: AgentEvent = {
      id: `${context.turnId}-manual-interrupt`,
      turnId: context.turnId,
      type: "interrupt",
      copy: { source: "app", kind: "run_stopped" },
      recoverable: false,
      status: "interrupted",
      createdAt: MOCK_AT,
    };
    let threads = terminalizeCurrentRun(state);
    threads = updateThread(threads, context.threadId, (thread) =>
      appendEventToThread(
        disableRecoveryForTurn(thread, context.branchId, context.turnId),
        context.branchId,
        event,
        "interrupted",
      ),
    );
    set({
      threads,
      runEpoch: state.runEpoch + 1,
      runStatus: "interrupted",
      activeRun: null,
    });
  },

  resolvePermission: async (decision) => {
    if (runtimeClient) {
      return;
    }
    const state = get();
    const thread = findThread(state.threads, state.selectedThreadId);
    const branch = activeBranch(thread);
    const turn = branch?.turns.at(-1);
    const pending = turn?.events.find(
      (event): event is PermissionEvent =>
        event.type === "permission_request" && event.status === "pending",
    );
    if (!thread || !branch || !turn || !pending || state.runStatus !== "waiting_permission") {
      return;
    }
    client.cancel();
    const epoch = state.runEpoch + 1;
    const context: RunContext = {
      threadId: thread.id,
      branchId: branch.id,
      turnId: turn.id,
      epoch,
    };
    const permissionEvent: AgentEvent = {
      ...pending,
      status: decision === "allow" ? "allowed" : "denied",
    };
    const threads = updateThread(state.threads, thread.id, (candidate) =>
      appendEventToThread(candidate, branch.id, permissionEvent, "running"),
    );
    set({ threads, runEpoch: epoch, runStatus: "running", activeRun: context });
    const terminal = await client.resolvePermission(decision, turn.id, (event, status) => {
      if (get().runEpoch !== epoch) return;
      get().appendAgentEvent(thread.id, branch.id, event, status);
    });
    finishRun(set, get, context, terminal);
  },

  recoverRun: async (strategy) => {
    if (runtimeClient) {
      return;
    }
    const state = get();
    const thread = findThread(state.threads, state.selectedThreadId);
    const branch = activeBranch(thread);
    const turn = branch?.turns.at(-1);
    const recoverable = turn?.events.some(
      (event) =>
        event.type === "interrupt" &&
        event.status === "interrupted" &&
        event.recoverable,
    );
    if (!thread || !branch || !turn || !recoverable || state.runStatus !== "interrupted") {
      return;
    }
    client.cancel();
    const epoch = state.runEpoch + 1;
    const context: RunContext = {
      threadId: thread.id,
      branchId: branch.id,
      turnId: turn.id,
      epoch,
    };
    set({ runEpoch: epoch, runStatus: "running", activeRun: context });
    const terminal = await client.recover(strategy, turn.id, (event, status) => {
      if (get().runEpoch !== epoch) return;
      get().appendAgentEvent(thread.id, branch.id, event, status);
    });
    finishRun(set, get, context, terminal);
  },

  appendAgentEvent: (threadId, branchId, event, status) =>
    set((state) => ({
      threads: updateThread(state.threads, threadId, (thread) =>
        appendEventToThread(thread, branchId, event, status),
      ),
      runStatus: status ?? state.runStatus,
    })),
}));

export const selectCurrentThread = (state: AppState) =>
  findThread(state.threads, state.selectedThreadId);
