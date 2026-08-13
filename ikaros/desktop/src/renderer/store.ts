import { create, type StoreApi } from "zustand";
import {
  activeBranch,
  findThread,
  isRunActive,
  type AgentEvent,
  type PermissionEvent,
  type Project,
  type RunStatus,
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
  applyRuntimeEvent as projectRuntimeEvent,
  projectRuntimeProjects,
  projectRuntimeThread,
  projectRuntimeThreads,
  runtimeThreadWorkspaceFromEvent,
} from "./runtimeProjection";
import type {
  RuntimeDiscoveredModel,
  RuntimeJournalEvent,
  RuntimeModelSetEnabledParams,
  RuntimeModelSummary,
  RuntimeProviderConfigureParams,
  RuntimeProviderSummary,
  RuntimeThreadCreateResult,
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
type RuntimeModelSelection = { providerId: string; modelId: string };
type ProviderCatalogStatus = "idle" | "loading" | "ready" | "error";

interface AppState {
  runtimeMode: boolean;
  runtimeReady: boolean;
  runtimeError: string | null;
  runtimeSeq: number;
  providerCatalogStatus: ProviderCatalogStatus;
  providers: RuntimeProviderSummary[];
  models: RuntimeModelSummary[];
  selectedModel: RuntimeModelSelection | null;
  projects: Project[];
  threads: Thread[];
  selectedThreadId: string | null;
  runStatus: RunStatus;
  draft: string;
  sidebarOpen: boolean;
  sidebarWidth: number;
  searchOpen: boolean;
  settingsOpen: boolean;
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
  applyRuntimeEvent: (event: RuntimeJournalEvent) => void;
  loadProviderCatalog: () => Promise<void>;
  discoverDeepSeekModels: (apiKey: string) => Promise<RuntimeDiscoveredModel[]>;
  configureProvider: (params: RuntimeProviderConfigureParams) => Promise<void>;
  disconnectProvider: (providerId: "deepseek") => Promise<void>;
  removeProvider: (providerId: string) => Promise<void>;
  setModelEnabled: (params: RuntimeModelSetEnabledParams) => Promise<void>;
  selectModel: (selection: RuntimeModelSelection | null) => void;
  setDraft: (draft: string) => void;
  setSidebarOpen: (open: boolean) => void;
  setSidebarWidth: (width: number) => void;
  setSearchOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
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
let removeRuntimeSubscription: (() => void) | undefined;
let bufferedRuntimeEvents: RuntimeJournalEvent[] = [];
const pendingRuntimeEvents = new Map<number, RuntimeJournalEvent>();
const cancellingRuntimeRuns = new Set<string>();
const runtimeTurnContinuations = new Map<string, Promise<void>>();
const canonicalRuntimeTurnStarts = new Map<string, RuntimeTurnStartResult>();
let runtimeGapRecovery: Promise<void> | undefined;
const RUNTIME_GAP_RETRY_DELAYS_MS = [25, 75, 200] as const;
const RUNTIME_COMMAND_RETRY_DELAYS_MS = [50, 150, 400, 800] as const;
const MOCK_AT = "2026-08-05T06:00:00.000Z";

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
  return runnable.length === 1 && runnable[0]
    ? { providerId: runnable[0].providerId, modelId: runnable[0].id }
    : null;
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
    updatedAt: event.createdAt,
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
  const stored = storedRunStatus(findThread(threads, selectedThreadId));
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
      runtimeError: null,
    }));
  } catch (error: unknown) {
    if (revision !== providerCatalogRevision) return;
    set({
      providerCatalogStatus: "error",
      runtimeError:
        error instanceof Error ? error.message : "Runtime provider catalog failed",
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
    set({ runtimeError: error.message, providerCatalogStatus: "error" });
    throw error;
  }
  try {
    await mutation();
  } catch (error: unknown) {
    set({
      runtimeError:
        error instanceof Error ? error.message : "Runtime provider configuration failed",
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
    typeof value.updatedAt !== "string"
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

function runtimeStateForEvent(state: AppState, event: RuntimeJournalEvent): Partial<AppState> {
  const threads = projectRuntimeEvent(state.threads, event);
  const createdWorkspace = runtimeThreadWorkspaceFromEvent(event);
  const projects = createdWorkspace
    ? mergeWorkspaceProjects(state.projects, [createdWorkspace])
    : state.projects;
  const settledActiveRun =
    event.type === "run.settled" && event.runId === state.activeRun?.runId;
  let pendingRuntimeSubmissions = state.pendingRuntimeSubmissions;
  let pendingRuntimeNewThread = state.pendingRuntimeNewThread;
  const pendingWorkspace = pendingRuntimeNewThread?.workspace;
  let selectedThreadId = state.selectedThreadId;
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
    pendingRuntimeSubmissions,
    pendingRuntimeNewThread,
    selectedThreadId,
  };
  return {
    threads,
    projects,
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
      threads: state.threads,
      runtimeSeq: state.runtimeSeq,
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
      });
    }
  })()
    .catch((error: unknown) => {
      set({
        runtimeError:
          error instanceof Error ? error.message : "Runtime event replay failed",
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

async function initializeRuntimeInStore(set: StoreSet, get: StoreGet): Promise<void> {
  if (!runtimeClient || get().runtimeReady) {
    return;
  }
  if (runtimeInitialization) {
    return runtimeInitialization;
  }

  const initialization = (async () => {
    const catalogRevision = ++providerCatalogRevision;
    bufferedRuntimeEvents = [];
    removeRuntimeSubscription ??= runtimeClient.onEvent((event) => {
      if (get().runtimeReady) {
        get().applyRuntimeEvent(event);
      } else {
        bufferedRuntimeEvents.push(event);
      }
    });

    const [listed, providerResult, modelResult] = await Promise.all([
      runtimeClient.listThreads(),
      runtimeClient.listProviders(),
      runtimeClient.listModels(),
    ]);
    const replayedEvents = new Map<number, RuntimeJournalEvent>();
    let cursor = 0;
    while (true) {
      const replay = await runtimeClient.replayEvents(cursor);
      for (const event of replay.events) {
        replayedEvents.set(event.seq, event);
      }
      cursor = replay.nextAfterSeq;
      if (!replay.hasMore) {
        break;
      }
    }
    for (const event of bufferedRuntimeEvents) {
      replayedEvents.set(event.seq, event);
    }
    bufferedRuntimeEvents = [];

    // SQLite AUTOINCREMENT sequences do not restart when an older journal prefix is
    // removed. Bootstrap from the first retained event while keeping strict
    // contiguous-sequence checks for every event received after initialization.
    const replayBaselineSeq =
      replayedEvents.size > 0 ? Math.min(...replayedEvents.keys()) - 1 : 0;

    pendingRuntimeEvents.clear();
    set((state) => ({
      projects: mergeRuntimeProjects(state.projects, listed.threads),
      threads: projectRuntimeThreads(listed.threads),
      runtimeSeq: replayBaselineSeq,
      runStatus: "idle",
      activeRun: null,
    }));
    for (const event of replayedEvents.values()) {
      pendingRuntimeEvents.set(event.seq, event);
    }
    drainRuntimeEventQueue(set, get);

    const threads = get().threads;
    const selectedThreadId = threads.some((thread) => thread.id === get().selectedThreadId)
      ? get().selectedThreadId
      : null;
    set((state) => {
      const catalogIsCurrent = catalogRevision === providerCatalogRevision;
      return {
        threads,
        selectedThreadId,
        runtimeReady: true,
        runtimeError: catalogIsCurrent ? null : state.runtimeError,
        ...(catalogIsCurrent
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
        runStatus: runtimeRunStatusForSelection(get(), selectedThreadId, threads),
      };
    });
    recoverRuntimeEventGap(set, get);
  })();
  runtimeInitialization = initialization;
  try {
    await initialization;
  } catch (error: unknown) {
    set({
      runtimeError: error instanceof Error ? error.message : "Runtime initialization failed",
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
    const threads = state.threads.some((thread) => thread.id === threadId)
      ? state.threads
      : [projectRuntimeThread(created.thread), ...state.threads];
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
      ...(ownsForeground
        ? { draft: submission.prompt, runStatus: "failed" as const, activeRun: null }
        : {
            runStatus: runtimeRunStatusForSelection(
              projectedState,
              state.selectedThreadId,
            ),
          }),
      pendingRuntimeSubmissions,
      pendingRuntimeNewThread,
      runtimeError,
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
    set({ runtimeError: "No configured model is selected." });
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
  runtimeError: null,
  runtimeSeq: 0,
  providerCatalogStatus: runtimeClient === null ? "ready" : "idle",
  providers: [],
  models: [],
  selectedModel: null,
  projects: runtimeClient ? [] : createInitialProjects(),
  threads: runtimeClient ? [] : createInitialThreads(),
  selectedThreadId: null,
  runStatus: "idle",
  draft: "",
  sidebarOpen: true,
  sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
  searchOpen: false,
  settingsOpen: false,
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
  applyRuntimeEvent: (event) => enqueueRuntimeEvent(set, get, event),
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
  selectModel: (selection) =>
    set((state) => ({
      selectedModel: selectionIsRunnable(selection, state.providers, state.models)
        ? selection
        : null,
    })),

  setDraft: (draft) => set({ draft }),
  setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }),
  setSidebarWidth: (sidebarWidth) => set({ sidebarWidth }),
  setSearchOpen: (searchOpen) => set({ searchOpen }),
  setProfileUsername: (profileUsername) => set({ profileUsername }),
  setSettingsOpen: (settingsOpen) =>
    set((state) => ({
      settingsOpen,
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
      set((current) => ({
        selectedThreadId: threadId,
        runStatus: runtimeRunStatusForSelection(current, threadId),
        editingMessage: null,
        searchOpen: false,
        settingsOpen: false,
        runtimeForegroundGeneration: current.runtimeForegroundGeneration + 1,
      }));
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

  sendDraft: async () => {
    const state = get();
    const prompt = state.draft.trim();
    if (!prompt || isRunActive(state.runStatus)) return;
    if (runtimeClient) {
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
          set({
            runtimeError: error instanceof Error ? error.message : "Runtime cancellation failed",
          });
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
