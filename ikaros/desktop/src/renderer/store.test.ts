import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { standaloneThreads, threadsForProject, type Thread } from "./domain";
import { LOCAL_PROFILE, profileInitials } from "./localProfile";
import { useAppStore } from "./store";

const initialState = useAppStore.getInitialState();

function resetStore() {
  useAppStore.setState(
    {
      ...initialState,
      expandedProjects: { ...initialState.expandedProjects },
    },
    true,
  );
}

function thread(
  id: string,
  projectId: string | null,
  updatedAt = "2026-08-05T06:00:00.000Z",
): Thread {
  return {
    id,
    projectId,
    title: id,
    activeBranchId: `${id}-main`,
    updatedAt,
    branches: [
      {
        id: `${id}-main`,
        threadId: id,
        turns: [],
        createdAt: updatedAt,
      },
    ],
  };
}

beforeEach(resetStore);

afterEach(() => {
  useAppStore.setState(initialState, true);
});

describe("Runtime-only store baseline", () => {
  it("starts without mock projects or threads", () => {
    const state = useAppStore.getState();
    expect(state.runtimeMode).toBe(true);
    expect(state.runtimeConnectionStatus).toBe("offline");
    expect(state.projects).toEqual([]);
    expect(state.threads).toEqual([]);
    expect(state.selectedThreadId).toBeNull();
  });
});

describe("conversation ownership", () => {
  it("partitions every thread into exactly one project or the standalone chat list", () => {
    const projects = [
      { id: "project-a", name: "Project A", color: "var(--muted-strong)" },
      { id: "project-b", name: "Project B", color: "var(--muted-strong)" },
    ];
    const threads = [
      thread("thread-a", "project-a"),
      thread("thread-b", "project-b"),
      thread("thread-standalone-1", null),
      thread("thread-standalone-2", null),
    ];

    const projectThreadIds = projects.flatMap((project) =>
      threadsForProject(threads, project.id).map((candidate) => candidate.id),
    );
    const standaloneThreadIds = standaloneThreads(threads).map((candidate) => candidate.id);

    expect(projectThreadIds).toEqual(["thread-a", "thread-b"]);
    expect(standaloneThreadIds).toHaveLength(2);
    expect(new Set(projectThreadIds).size).toBe(projectThreadIds.length);
    expect(projectThreadIds.filter((id) => standaloneThreadIds.includes(id))).toEqual([]);
    expect(new Set([...projectThreadIds, ...standaloneThreadIds])).toEqual(
      new Set(threads.map((candidate) => candidate.id)),
    );
  });
});

describe("core store invariants", () => {
  it("treats settings as a page and returns to the app on conversation navigation", () => {
    useAppStore.setState({ searchOpen: true, settingsOpen: false });

    useAppStore.getState().setSettingsOpen(true);

    expect(useAppStore.getState()).toMatchObject({
      searchOpen: false,
      settingsOpen: true,
    });

    useAppStore.getState().newChat();
    expect(useAppStore.getState().settingsOpen).toBe(false);
  });

  it("keeps the projected profile username in Zustand state", () => {
    expect(useAppStore.getState().profileUsername).toBe(LOCAL_PROFILE.name);
    expect(useAppStore.getInitialState().profileUsername).toBe(LOCAL_PROFILE.name);

    useAppStore.getState().setProfileUsername("March Seven");

    expect(useAppStore.getState().profileUsername).toBe("March Seven");
    expect(useAppStore.getInitialState().profileUsername).toBe(LOCAL_PROFILE.name);

    resetStore();
    expect(useAppStore.getState().profileUsername).toBe(LOCAL_PROFILE.name);
  });

  it("keeps sidebar width as renderer state for drag previews", () => {
    expect(useAppStore.getState().sidebarWidth).toBe(260);

    useAppStore.getState().setSidebarWidth(404);

    expect(useAppStore.getState().sidebarWidth).toBe(404);
    expect(useAppStore.getInitialState().sidebarWidth).toBe(260);
  });

  it("stages a new project workspace without fabricating a Runtime Thread", () => {
    const workspace = {
      id: "workspace-new",
      name: "Named project",
      rootUri: "C:\\Workspace\\named-project",
    };
    const beforeThreadIds = useAppStore.getState().threads.map((candidate) => candidate.id);

    useAppStore.getState().stageProjectWorkspace(workspace);

    const state = useAppStore.getState();
    expect(state.projects).toContainEqual({
      ...workspace,
      color: "var(--muted-strong)",
    });
    expect(state.threads.map((candidate) => candidate.id)).toEqual(beforeThreadIds);
    expect(state.newThreadWorkspace).toEqual(workspace);
    expect(state.selectedThreadId).toBeNull();
    expect(state.expandedProjects[workspace.id]).toBe(true);
  });

  it("reuses the canonical project snapshot when a staged workspace id already exists", () => {
    const existing = {
      id: "workspace-existing",
      name: "Existing project",
      color: "var(--muted-strong)",
      rootUri: "C:\\Workspace\\existing",
    };
    useAppStore.setState({ projects: [existing] });
    const beforeProjects = useAppStore.getState().projects;

    useAppStore.getState().stageProjectWorkspace({
      id: existing.id,
      name: "Ignored rename",
      rootUri: "C:\\Different\\root",
    });

    const state = useAppStore.getState();
    expect(state.projects).toBe(beforeProjects);
    expect(state.newThreadWorkspace).toEqual({
      id: existing.id,
      name: existing.name,
      rootUri: existing.rootUri,
    });
    expect(state.projects.filter((project) => project.id === existing.id)).toHaveLength(1);
  });

  it.each([
    ["hc", "HC"],
    ["  ikaros  ", "IK"],
    ["March Seven", "MS"],
    ["  March   Seventh  ", "MS"],
    ["伊卡洛斯", "伊卡"],
    ["   ", "US"],
])("derives profile initials from %j", (username, expected) => {
    expect(profileInitials(username)).toBe(expected);
  });
});
