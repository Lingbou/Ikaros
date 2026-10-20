import "@testing-library/jest-dom/vitest";
import { Profiler, type ProfilerOnRenderCallback } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  findThread,
  flattenEvents,
  type MessageEvent,
  type Thread,
} from "../domain";
import { setUiLanguage } from "../i18n";
import { TEST_EXPANDED_PROJECTS, TEST_PROJECTS, createFixtureThreads } from "../__fixtures__/testFixtures";
import { useAppStore } from "../store";
import { ConversationHeader } from "./ConversationHeader";

const initialState = useAppStore.getState();

function threadFixture(projectId: string | null = null): Thread {
  return {
    id: "thread-localization",
    projectId,
    title: "Localization boundary",
    activeBranchId: "branch-main",
    branches: [
      {
        id: "branch-main",
        threadId: "thread-localization",
        turns: [],
        createdAt: "2026-08-05T06:00:00.000Z",
      },
    ],
    updatedAt: "2026-08-05T06:00:00.000Z",
  };
}

afterEach(() => {
  cleanup();
  setUiLanguage("en");
  useAppStore.setState(initialState, true);
});

describe("ConversationHeader actions", () => {
  it("opens a path entry for a regular conversation folder without a Git requirement", () => {
    useAppStore.setState({
            projects: [{ id: "workspace-files", name: "Reports", color: "#ffffff", rootUri: "/tmp/plain-reports" }],
      threads: [threadFixture("workspace-files")],
      selectedThreadId: "thread-localization",
      draft: "Keep my draft",
    });
    render(<ConversationHeader />);
    fireEvent.click(screen.getByRole("button", { name: "Open file" }));
    expect(useAppStore.getState()).toMatchObject({
      fileSelection: { threadId: "thread-localization", path: "", view: "current" },
      draft: "Keep my draft",
    });
  });

  it("renames and archives from the flat title menu without an ellipsis control", async () => {
    const renameThread = vi.fn(async () => undefined);
    const archiveThread = vi.fn(async () => undefined);
    useAppStore.setState({
      projects: [],
      threads: [threadFixture()],
      selectedThreadId: "thread-localization",
      runStatus: "idle",
      renameThread,
      archiveThread,
    });

    render(<ConversationHeader />);
    const titleButton = screen.getByRole("button", { name: "Localization boundary" });
    expect(titleButton.querySelector("svg")).toBeNull();

    fireEvent.pointerDown(titleButton);
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename" }));
    const input = screen.getByRole("textbox", { name: "Conversation title" });
    fireEvent.change(input, { target: { value: "Renamed conversation" } });
    fireEvent.submit(input.closest("form") as HTMLFormElement);
    await waitFor(() => {
      expect(renameThread).toHaveBeenCalledWith(
        "thread-localization",
        "Renamed conversation",
      );
    });

    fireEvent.pointerDown(
      screen.getByRole("button", { name: "Localization boundary" }),
    );
    fireEvent.click(await screen.findByRole("menuitem", { name: "Archive" }));
    await waitFor(() => expect(archiveThread).toHaveBeenCalledWith("thread-localization"));
  });
});

describe("ConversationHeader store subscriptions", () => {
  it("skips event-only and run-status commits", () => {
    useAppStore.setState({
      projects: TEST_PROJECTS,
      threads: createFixtureThreads(),
      expandedProjects: TEST_EXPANDED_PROJECTS,
      selectedThreadId: "thread-streaming",
      runStatus: "running",
    });
    const onRender = vi.fn<ProfilerOnRenderCallback>();

    render(
      <Profiler id="conversation-header" onRender={onRender}>
        <ConversationHeader />
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
      useAppStore.setState({ runStatus: "interrupted" });
    });
    expect(onRender).toHaveBeenCalledTimes(commitsAfterMount);
  });
});
