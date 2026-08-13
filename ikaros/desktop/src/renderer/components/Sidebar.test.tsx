import * as Tooltip from "@radix-ui/react-tooltip";
import { Profiler, type ProfilerOnRenderCallback } from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { findThread, flattenEvents, type MessageEvent } from "../domain";
import { setUiLanguage } from "../i18n";
import { LOCAL_PROFILE } from "../localProfile";
import { createInitialProjects, createInitialThreads } from "../mockAgentClient";
import { useAppStore } from "../store";
import { DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH } from "../../shared/platform";
import { Sidebar } from "./Sidebar";

const initialState = useAppStore.getState();

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(() => ({
      matches: true,
      media: "(min-width: 900px)",
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
  Object.defineProperty(window, "ikarosDesktop", {
    configurable: true,
    value: {
      workspace: {
        chooseDirectory: vi.fn(async () => null),
      },
      preferences: {
        update: vi.fn(async () => undefined),
      },
    },
  });
  useAppStore.setState({
    projects: createInitialProjects(),
    threads: createInitialThreads(),
    selectedThreadId: null,
    sidebarOpen: true,
    sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
    settingsOpen: false,
    profileUsername: LOCAL_PROFILE.name,
  });
  setUiLanguage("en");
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useAppStore.setState(initialState, true);
});

describe("Sidebar conversation ownership", () => {
  it("shows the product name without a leading brand icon", () => {
    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const productName = screen.getByLabelText("Current workspace: Ikaros");
    expect(productName.textContent).toBe("Ikaros");
    expect(productName.previousElementSibling).toBeNull();
    const brand = screen.getByText("Ikaros");
    expect(brand.className).toContain("text-[13px]");
    expect(brand.className).toContain("leading-5");
    expect(brand.className).toContain("font-semibold");
  });

  it("previews pointer resizing in memory and persists only when released", () => {
    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const separator = screen.getByRole("separator", { name: "Resize sidebar" });
    Object.defineProperty(separator, "setPointerCapture", { value: vi.fn() });
    Object.defineProperty(separator, "hasPointerCapture", { value: vi.fn(() => true) });
    Object.defineProperty(separator, "releasePointerCapture", { value: vi.fn() });

    fireEvent.pointerDown(separator, { button: 0, pointerId: 7, clientX: 260 });
    fireEvent.pointerMove(separator, { pointerId: 7, clientX: 360 });

    expect(useAppStore.getState().sidebarWidth).toBe(360);
    expect(window.ikarosDesktop?.preferences.update).not.toHaveBeenCalled();
    expect(separator.getAttribute("aria-valuenow")).toBe("360");

    fireEvent.pointerUp(separator, { pointerId: 7, clientX: 360 });

    expect(window.ikarosDesktop?.preferences.update).toHaveBeenCalledTimes(1);
    expect(window.ikarosDesktop?.preferences.update).toHaveBeenCalledWith({
      sidebarWidth: 360,
    });
  });

  it("rolls an unfinished drag back when the sidebar unmounts", () => {
    const { unmount } = render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );
    const separator = screen.getByRole("separator", { name: "Resize sidebar" });
    Object.defineProperty(separator, "setPointerCapture", { value: vi.fn() });

    fireEvent.pointerDown(separator, { button: 0, pointerId: 8, clientX: 260 });
    fireEvent.pointerMove(separator, { pointerId: 8, clientX: 390 });
    expect(useAppStore.getState().sidebarWidth).toBe(390);

    unmount();

    expect(useAppStore.getState().sidebarWidth).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(document.documentElement.dataset.resizingSidebar).toBeUndefined();
    expect(window.ikarosDesktop?.preferences.update).not.toHaveBeenCalled();
  });

  it("clamps pointer resizing, supports keyboard resizing, and resets on double-click", () => {
    useAppStore.setState({ sidebarWidth: MAX_SIDEBAR_WIDTH - 5 });
    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const separator = screen.getByRole("separator", { name: "Resize sidebar" });
    fireEvent.keyDown(separator, { key: "ArrowRight" });
    expect(useAppStore.getState().sidebarWidth).toBe(MAX_SIDEBAR_WIDTH);

    fireEvent.keyDown(separator, { key: "Home" });
    expect(useAppStore.getState().sidebarWidth).toBe(MIN_SIDEBAR_WIDTH);

    fireEvent.keyDown(separator, { key: "ArrowRight", shiftKey: true });
    expect(useAppStore.getState().sidebarWidth).toBe(MIN_SIDEBAR_WIDTH + 40);

    fireEvent.doubleClick(separator);
    expect(useAppStore.getState().sidebarWidth).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(window.ikarosDesktop?.preferences.update).toHaveBeenLastCalledWith({
      sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
    });
  });

  it("keeps the overlay sidebar non-resizable below the desktop breakpoint", () => {
    vi.mocked(window.matchMedia).mockReturnValue({
      matches: false,
      media: "(min-width: 900px)",
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    });
    const { container } = render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const separator = container.querySelector<HTMLElement>(".sidebar-resizer");
    expect(separator).not.toBeNull();
    if (!separator) throw new Error("Sidebar resizer was not rendered");
    expect(separator.getAttribute("aria-hidden")).toBe("true");
    expect(separator.getAttribute("tabindex")).toBe("-1");
    fireEvent.keyDown(separator, { key: "End" });
    fireEvent.doubleClick(separator);

    expect(useAppStore.getState().sidebarWidth).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(window.ikarosDesktop?.preferences.update).not.toHaveBeenCalled();
  });

  it("keeps project chats and standalone recent chats in separate groups", () => {
    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const research = screen.getByRole("region", { name: /Research/ });
    const recents = screen.getByRole("region", { name: "Recents" });

    expect(within(research).getByText("Summarize the field notes")).toBeTruthy();
    expect(within(recents).queryByText("Summarize the field notes")).toBeNull();
    expect(within(recents).getByText("Plan tomorrow's priorities")).toBeTruthy();
    expect(within(research).queryByText("Plan tomorrow's priorities")).toBeNull();
    expect(screen.getAllByText("Summarize the field notes")).toHaveLength(1);
  });

  it("opens both groups through the same thread action", () => {
    const selectThread = vi.fn(async () => undefined);
    useAppStore.setState({ selectThread });

    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Summarize the field notes" }));
    fireEvent.click(screen.getByRole("button", { name: "Plan tomorrow's priorities" }));

    expect(selectThread).toHaveBeenNthCalledWith(1, "thread-streaming");
    expect(selectThread).toHaveBeenNthCalledWith(2, "thread-recent-priorities");
  });

  it("keeps group actions hidden and opens project creation without choosing or staging", () => {
    const stageProjectWorkspace = vi.fn();
    const newChat = vi.fn();
    useAppStore.setState({ stageProjectWorkspace, newChat } as Partial<typeof initialState>);

    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const projectAction = screen.getByRole("button", { name: "New project chat" });
    const newChatActions = screen.getAllByRole("button", { name: "New chat" });
    const recentAction = newChatActions.at(-1);
    const projectsToggle = screen.getByRole("button", { name: "Projects" });
    const recentsToggle = screen.getByRole("button", { name: "Recents" });
    const projectsChevron = projectsToggle.querySelector("span:last-child");
    const recentsChevron = recentsToggle.querySelector("span:last-child");
    expect(projectAction.className).toContain("opacity-0");
    expect(recentAction?.className).toContain("opacity-0");
    expect(projectsChevron?.className).toContain("opacity-0");
    expect(recentsChevron?.className).toContain("opacity-0");
    expect(projectAction.className).not.toContain("focus-within");
    expect(recentAction?.className).not.toContain("focus-within");
    expect(projectsChevron?.className).not.toContain("focus-within");
    expect(recentsChevron?.className).not.toContain("focus-within");

    if (!recentAction) throw new Error("Recent chat action was not rendered");
    fireEvent.click(recentAction);
    fireEvent.click(projectAction);

    expect(newChat).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("dialog", { name: "Create project" })).toBeTruthy();
    expect(window.ikarosDesktop.workspace.chooseDirectory).not.toHaveBeenCalled();
    expect(stageProjectWorkspace).not.toHaveBeenCalled();
  });

  it("collapses project and recent groups independently", () => {
    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const projects = screen.getByRole("button", { name: "Projects" });
    const recents = screen.getByRole("button", { name: "Recents" });
    expect(projects.getAttribute("aria-expanded")).toBe("true");
    expect(recents.getAttribute("aria-expanded")).toBe("true");

    fireEvent.click(projects);
    expect(projects.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("button", { name: "Research" })).toBeNull();
    expect(screen.getByRole("button", { name: "Plan tomorrow's priorities" })).toBeTruthy();

    fireEvent.click(recents);
    expect(recents.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("button", { name: "Plan tomorrow's priorities" })).toBeNull();
  });

  it("uses one project icon, keeps child selection full-width, and leaves recents icon-free", () => {
    useAppStore.setState({ selectedThreadId: "thread-streaming" });

    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const researchButton = screen.getByRole("button", { name: "Research" });
    const projectThread = screen.getByRole("button", { name: "Summarize the field notes" });
    const recentThread = screen.getByRole("button", { name: "Plan tomorrow's priorities" });
    const newChat = screen.getAllByRole("button", { name: "New chat" })[0];

    expect(researchButton.querySelectorAll("svg")).toHaveLength(1);
    expect(researchButton.className).toContain("text-[13px]");
    expect(researchButton.className).toContain("font-medium");
    expect(projectThread.className).toContain("pl-[31px]");
    expect(projectThread.className).toContain("text-[13px]");
    expect(projectThread.className).toContain("font-medium");
    expect(recentThread.querySelector("svg")).toBeNull();
    expect(recentThread.className).toContain("text-[13px]");
    expect(recentThread.className).not.toContain("font-medium");
    expect(newChat.className).toContain("text-[13px]");
    expect(newChat.className).not.toContain("font-medium");
    expect(screen.getByText("Projects").closest("div")?.className).toContain("text-[12px]");
    expect(screen.getByText("Projects").closest("div")?.className).toContain("font-semibold");
    expect(screen.getByText("Recents").closest("section")?.className).toContain("mt-1");
    expect(screen.getByText("Recents").closest("div")?.className).toContain("text-[12px]");
    expect(screen.getByText(LOCAL_PROFILE.initials).className).toContain("text-[10px]");
    expect(screen.getByText(LOCAL_PROFILE.name).className).toContain("text-[12px]");
    expect(screen.getByText(LOCAL_PROFILE.name).className).toContain("leading-[18px]");

    fireEvent.click(researchButton);
    expect(screen.queryByRole("button", { name: "Summarize the field notes" })).toBeNull();
    fireEvent.click(researchButton);
    expect(screen.getByRole("button", { name: "Summarize the field notes" })).toBeTruthy();
  });

  it("renders the shared local profile identity", () => {
    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    const profileTrigger = screen.getByRole("button", { name: "Open profile menu" });
    expect(within(profileTrigger).getByText(LOCAL_PROFILE.initials)).toBeTruthy();
    expect(within(profileTrigger).getByText(LOCAL_PROFILE.name)).toBeTruthy();
  });

  it("updates the projected profile name and derived initials", () => {
    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    act(() => useAppStore.getState().setProfileUsername("March Seven"));

    const profileTrigger = screen.getByRole("button", { name: "Open profile menu" });
    expect(within(profileTrigger).getByText("March Seven")).toBeTruthy();
    expect(within(profileTrigger).getByText("MS")).toBeTruthy();
    expect(within(profileTrigger).queryByText(LOCAL_PROFILE.name)).toBeNull();
  });

  it("does not commit again for event-only thread revisions", () => {
    useAppStore.setState({
      selectedThreadId: "thread-streaming",
      runStatus: "running",
    });
    const onRender = vi.fn<ProfilerOnRenderCallback>();

    render(
      <Profiler id="sidebar" onRender={onRender}>
        <Tooltip.Provider>
          <Sidebar />
        </Tooltip.Provider>
      </Profiler>,
    );

    const commitsAfterMount = onRender.mock.calls.length;
    const beforeThreads = useAppStore.getState().threads;
    const beforeThread = findThread(beforeThreads, "thread-streaming");
    const sourceEvent = flattenEvents(beforeThread)[0];
    expect(sourceEvent?.type).toBe("message");

    const revision: MessageEvent = {
      ...(sourceEvent as MessageEvent),
      content: "Revised event content",
      createdAt: "2026-08-05T06:00:01.000Z",
    };
    act(() => {
      useAppStore
        .getState()
        .appendAgentEvent("thread-streaming", "thread-streaming-main", revision, "running");
    });

    const revisedState = useAppStore.getState();
    const revisedThread = findThread(revisedState.threads, "thread-streaming");
    expect(revisedState.threads).not.toBe(beforeThreads);
    expect(revisedThread).not.toBe(beforeThread);
    expect(flattenEvents(revisedThread)[0]).toMatchObject({
      id: sourceEvent?.id,
      content: "Revised event content",
    });
    expect(onRender).toHaveBeenCalledTimes(commitsAfterMount);

    act(() => {
      useAppStore.setState({
        threads: revisedState.threads.map((thread) =>
          thread.id === "thread-streaming"
            ? { ...thread, title: "Renamed field notes" }
            : thread,
        ),
      });
    });
    expect(screen.getByRole("button", { name: "Renamed field notes" })).toBeTruthy();
    expect(onRender.mock.calls.length).toBeGreaterThan(commitsAfterMount);
  });

  it("translates product navigation without rewriting mock conversation data", () => {
    setUiLanguage("zh-CN");

    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    expect(screen.getAllByRole("button", { name: "新对话" })).not.toHaveLength(0);
    expect(screen.getByRole("button", { name: "新建项目对话" })).toBeTruthy();
    expect(screen.getByText("项目")).toBeTruthy();
    expect(screen.getByText("最近")).toBeTruthy();
    expect(screen.getByText("Research")).toBeTruthy();
    expect(screen.getByText("Summarize the field notes")).toBeTruthy();
    expect(screen.getByText(LOCAL_PROFILE.name)).toBeTruthy();
    expect(screen.queryByText("此设备上")).toBeNull();
  });

  it("keeps secondary features inside a profile menu with one working settings entry", async () => {
    render(
      <Tooltip.Provider>
        <Sidebar />
      </Tooltip.Provider>,
    );

    expect(screen.queryByText("Providers")).toBeNull();
    expect(screen.queryByText("Skills")).toBeNull();
    expect(screen.getByText(LOCAL_PROFILE.name)).toBeTruthy();
    expect(screen.queryByText("On this device")).toBeNull();
    fireEvent.pointerDown(screen.getByRole("button", { name: "Open profile menu" }), {
      button: 0,
      ctrlKey: false,
    });

    const settings = await screen.findByRole("menuitem", { name: "Settings" });
    expect(settings.className).toContain("text-[12px]");
    expect(settings.className).toContain("leading-[18px]");
    expect(settings.className).not.toContain("12.5px");
    expect(screen.getAllByRole("menuitem")).toHaveLength(1);
    fireEvent.click(settings);

    expect(useAppStore.getState().settingsOpen).toBe(true);
    expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();
    expect(screen.queryByRole("switch", { name: "Reduce motion" })).toBeNull();
  });
});
