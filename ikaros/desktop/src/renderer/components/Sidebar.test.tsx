import * as Tooltip from "@radix-ui/react-tooltip";
import { Profiler, type ProfilerOnRenderCallback } from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { findThread, flattenEvents, type MessageEvent } from "../domain";
import { setUiLanguage } from "../i18n";
import { LOCAL_PROFILE } from "../localProfile";
import { createInitialProjects, createInitialThreads } from "../mockAgentClient";
import { useAppStore } from "../store";
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
  useAppStore.setState({
    projects: createInitialProjects(),
    threads: createInitialThreads(),
    selectedThreadId: null,
    sidebarOpen: true,
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
    const newChat = screen.getByRole("button", { name: "New chat" });

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
    expect(screen.getByText("Projects").className).toContain("text-[12px]");
    expect(screen.getByText("Projects").className).toContain("font-semibold");
    expect(screen.getByText("Recents").className).toContain("mt-1");
    expect(screen.getByText("Recents").className).toContain("text-[12px]");
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

  it("updates the profile name and derived initials from the in-memory store", () => {
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

    expect(screen.getByRole("button", { name: "新对话" })).toBeTruthy();
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
