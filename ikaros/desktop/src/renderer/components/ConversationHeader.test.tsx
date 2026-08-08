import "@testing-library/jest-dom/vitest";
import { Profiler, type ProfilerOnRenderCallback } from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  findThread,
  flattenEvents,
  type Branch,
  type MessageEvent,
  type Thread,
} from "../domain";
import { setUiLanguage } from "../i18n";
import { createInitialProjects, createInitialThreads } from "../mockAgentClient";
import { useAppStore } from "../store";
import { ConversationHeader } from "./ConversationHeader";

const initialState = useAppStore.getState();

function branch(id: string, label: Branch["label"]): Branch {
  return {
    id,
    threadId: "thread-localization",
    label,
    turns: [],
    createdAt: "2026-08-05T06:00:00.000Z",
  };
}

const branches = [
  branch("branch-main", { source: "app", kind: "main" }),
  branch("branch-1", { source: "app", kind: "number", number: 1 }),
  branch("branch-external", { source: "external", value: "feature/login" }),
];

function threadWithActiveBranch(activeBranchId: string): Thread {
  return {
    id: "thread-localization",
    projectId: null,
    title: "Localization boundary",
    activeBranchId,
    branches,
    updatedAt: "2026-08-05T06:00:00.000Z",
  };
}

afterEach(() => {
  cleanup();
  setUiLanguage("en");
  useAppStore.setState(initialState, true);
});

describe("ConversationHeader branch localization", () => {
  it("retranslates app branch labels and preserves external branch names", () => {
    useAppStore.setState({
      projects: [],
      threads: [threadWithActiveBranch("branch-main")],
      selectedThreadId: "thread-localization",
      runStatus: "idle",
    });

    render(<ConversationHeader />);

    const heading = screen.getByRole("heading", { name: "Localization boundary" });
    expect(heading.className).toContain("text-[13px]");
    expect(heading.className).toContain("leading-5");
    expect(heading.className).not.toContain("font-medium");
    expect(heading.parentElement?.className).toContain("h-10");
    expect(heading.parentElement?.className).toContain("select-none");
    expect(screen.queryByText("Chat")).toBeNull();
    expect(screen.queryByText("Ready")).toBeNull();

    const branchButton = screen.getByRole("button", { name: /Main/ });
    expect(branchButton.className).toContain("text-[12px]");
    expect(branchButton.className).toContain("leading-[18px]");

    expect(screen.getByText("Main")).toBeInTheDocument();

    act(() => setUiLanguage("zh-CN"));
    expect(screen.getByText("主分支")).toBeInTheDocument();

    act(() => {
      useAppStore.setState({ threads: [threadWithActiveBranch("branch-1")] });
    });
    expect(screen.getByText("分支 1")).toBeInTheDocument();

    act(() => {
      useAppStore.setState({ threads: [threadWithActiveBranch("branch-external")] });
    });
    expect(screen.getByText("feature/login")).toBeInTheDocument();

    act(() => setUiLanguage("en"));
    expect(screen.getByText("feature/login")).toBeInTheDocument();
  });
});

describe("ConversationHeader store subscriptions", () => {
  it("skips event-only and run-status commits", () => {
    useAppStore.setState({
      projects: createInitialProjects(),
      threads: createInitialThreads(),
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
      useAppStore.setState({ runStatus: "waiting_permission" });
    });
    expect(screen.queryByText("Needs permission")).toBeNull();
    expect(onRender).toHaveBeenCalledTimes(commitsAfterMount);
  });
});
