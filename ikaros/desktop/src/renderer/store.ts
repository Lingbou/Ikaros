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
import { LOCAL_PROFILE } from "./localProfile";
import { createRuntimeClient } from "./runtimeClient";
import {
  applyRuntimeEvent as projectRuntimeEvent,
  projectRuntimeThreads,
  replayRuntimeEvents,
} from "./runtimeProjection";
import type { RuntimeJournalEvent } from "../shared/runtime";

type EditingMessage = { eventId: string; content: string } | null;
type RunContext = {
  threadId: string;
  branchId: string;
  turnId: string;
  runId?: string;
  epoch: number;
};

interface AppState {
  runtimeMode: boolean;
  runtimeReady: boolean;
  runtimeError: string | null;
  runtimeSeq: number;
  projects: Project[];
  threads: Thread[];
  selectedThreadId: string | null;
  runStatus: RunStatus;
  draft: string;
  sidebarOpen: boolean;
  searchOpen: boolean;
  settingsOpen: boolean;
  profileUsername: string;
  editingMessage: EditingMessage;
  expandedProjects: Record<string, boolean>;
  runEpoch: number;
  localThreadCounter: number;
  branchCounter: number;
  activeRun: RunContext | null;

  initializeRuntime: () => Promise<void>;
  applyRuntimeEvent: (event: RuntimeJournalEvent) => void;
  setDraft: (draft: string) => void;
  setSidebarOpen: (open: boolean) => void;
  setSearchOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setProfileUsername: (username: string) => void;
  toggleProject: (projectId: string) => void;
  beginEditMessage: (eventId: string, content: string) => void;
  cancelEditMessage: () => void;
  commitMessageEdit: (content: string) => void;
  switchBranch: (branchId: string) => void;
  newChat: () => void;
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
let removeRuntimeSubscription: (() => void) | undefined;
let bufferedRuntimeEvents: RuntimeJournalEvent[] = [];
const MOCK_AT = "2026-08-05T06:00:00.000Z";

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
  if (state.activeRun) return state.activeRun;
  const thread = findThread(state.threads, state.selectedThreadId);
  const branch = activeBranch(thread);
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

function runtimeStateForEvent(state: AppState, event: RuntimeJournalEvent): Partial<AppState> {
  if (event.seq <= state.runtimeSeq) {
    return {};
  }
  const threads = projectRuntimeEvent(state.threads, event);
  const selected = findThread(threads, state.selectedThreadId);
  const settledActiveRun =
    event.type === "run.settled" && event.runId === state.activeRun?.runId;
  return {
    threads,
    runtimeSeq: event.seq,
    runStatus: selected ? storedRunStatus(selected) : "idle",
    activeRun: settledActiveRun ? null : state.activeRun,
  };
}

async function initializeRuntimeInStore(set: StoreSet, get: StoreGet): Promise<void> {
  if (!runtimeClient || get().runtimeReady) {
    return;
  }
  if (runtimeInitialization) {
    return runtimeInitialization;
  }

  runtimeInitialization = (async () => {
    bufferedRuntimeEvents = [];
    removeRuntimeSubscription ??= runtimeClient.onEvent((event) => {
      if (get().runtimeReady) {
        get().applyRuntimeEvent(event);
      } else {
        bufferedRuntimeEvents.push(event);
      }
    });

    const listed = await runtimeClient.listThreads();
    let threads = projectRuntimeThreads(listed.threads);
    let cursor = 0;
    while (true) {
      const replay = await runtimeClient.replayEvents(cursor);
      threads = replayRuntimeEvents(threads, replay.events);
      cursor = replay.nextAfterSeq;
      if (!replay.hasMore) {
        break;
      }
    }
    const liveEvents = bufferedRuntimeEvents
      .filter((event) => event.seq > cursor)
      .sort((left, right) => left.seq - right.seq);
    threads = replayRuntimeEvents(threads, liveEvents);
    cursor = liveEvents.at(-1)?.seq ?? cursor;
    bufferedRuntimeEvents = [];

    const selectedThreadId = threads.some((thread) => thread.id === get().selectedThreadId)
      ? get().selectedThreadId
      : null;
    const selected = findThread(threads, selectedThreadId);
    set({
      threads,
      selectedThreadId,
      runtimeReady: true,
      runtimeError: null,
      runtimeSeq: cursor,
      runStatus: storedRunStatus(selected),
    });
  })().catch((error: unknown) => {
    set({
      runtimeError: error instanceof Error ? error.message : "Runtime initialization failed",
    });
  });
  return runtimeInitialization;
}

async function sendRuntimeDraft(
  set: StoreSet,
  get: StoreGet,
  prompt: string,
): Promise<void> {
  if (!runtimeClient) {
    return;
  }
  await get().initializeRuntime();
  const initial = get();
  if (!initial.runtimeReady) {
    return;
  }

  const epoch = initial.runEpoch + 1;
  set({
    draft: "",
    runStatus: "queued",
    runtimeError: null,
    runEpoch: epoch,
  });

  try {
    let thread = findThread(get().threads, get().selectedThreadId);
    if (!thread) {
      const title = prompt.length > 42 ? `${prompt.slice(0, 42)}…` : prompt;
      const created = await runtimeClient.createThread(title);
      get().applyRuntimeEvent(created.event);
      thread = findThread(get().threads, created.thread.id);
      if (!thread) {
        throw new Error("Runtime thread projection was not created.");
      }
      set({ selectedThreadId: thread.id });
    }

    const started = await runtimeClient.startTurn({
      threadId: thread.id,
      branchId: thread.activeBranchId,
      content: prompt,
      providerId: "scripted",
      modelId: "scripted-v1",
    });
    const currentThread = findThread(get().threads, started.threadId);
    const projectedStatus = storedRunStatus(currentThread);
    const startedRun = {
      threadId: started.threadId,
      branchId: started.branchId,
      turnId: started.turnId,
      runId: started.runId,
      epoch,
    };
    set({
      selectedThreadId: started.threadId,
      runStatus: projectedStatus === "idle" ? "queued" : projectedStatus,
      activeRun:
        projectedStatus === "idle" || isRunActive(projectedStatus) ? startedRun : null,
    });
  } catch (error) {
    set({
      draft: prompt,
      runStatus: "failed",
      activeRun: null,
      runtimeError: error instanceof Error ? error.message : "Runtime request failed",
    });
  }
}

export const useAppStore = create<AppState>()((set, get) => ({
  runtimeMode: runtimeClient !== null,
  runtimeReady: runtimeClient === null,
  runtimeError: null,
  runtimeSeq: 0,
  projects: runtimeClient ? [] : createInitialProjects(),
  threads: runtimeClient ? [] : createInitialThreads(),
  selectedThreadId: null,
  runStatus: "idle",
  draft: "",
  sidebarOpen: true,
  searchOpen: false,
  settingsOpen: false,
  profileUsername: LOCAL_PROFILE.name,
  editingMessage: null,
  expandedProjects: {
    "project-research": true,
    "project-personal": true,
  },
  runEpoch: 0,
  localThreadCounter: 0,
  branchCounter: 0,
  activeRun: null,

  initializeRuntime: () => initializeRuntimeInStore(set, get),
  applyRuntimeEvent: (event) => set((state) => runtimeStateForEvent(state, event)),

  setDraft: (draft) => set({ draft }),
  setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }),
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
      set({
        selectedThreadId: null,
        runStatus: "idle",
        draft: "",
        editingMessage: null,
        settingsOpen: false,
      });
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

  selectThread: async (threadId) => {
    const state = get();
    const thread = findThread(state.threads, threadId);
    if (!thread) return;

    if (runtimeClient) {
      set({
        selectedThreadId: threadId,
        runStatus: storedRunStatus(thread),
        editingMessage: null,
        searchOpen: false,
        settingsOpen: false,
      });
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
      await sendRuntimeDraft(set, get, prompt);
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
