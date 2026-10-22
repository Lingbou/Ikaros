import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentEvent, MessageEvent, Thread } from "../domain";
import { setUiLanguage } from "../i18n";
import { useAppStore } from "../store";
import { ThreadPanel } from "./ThreadPanel";

const initialState = useAppStore.getState();
const originalScrollTo = Object.getOwnPropertyDescriptor(
  HTMLElement.prototype,
  "scrollTo",
);

function message(id: string, content: string): MessageEvent {
  return {
    id,
    turnId: "turn_child",
    createdAt: "2026-10-21T00:00:00.000Z",
    type: "message",
    role: "assistant",
    content,
    status: "complete",
  };
}

function thread(id: string, title: string, events: AgentEvent[]): Thread {
  return {
    id,
    projectId: null,
    title,
    activeBranchId: `branch_${id}`,
    branches: [
      {
        id: `branch_${id}`,
        threadId: id,
        turns: [
          {
            id: "turn_child",
            branchId: `branch_${id}`,
            status: "completed",
            events,
          },
        ],
        createdAt: "2026-10-21T00:00:00.000Z",
      },
    ],
    updatedAt: "2026-10-21T00:00:00.000Z",
  };
}

function readyDetail() {
  return {
    status: "ready" as const,
    snapshotSeq: 1,
    error: null,
    nextCursor: null,
    hasMore: false,
    olderStatus: "idle" as const,
    olderError: null,
    historySnapshotSeq: 7,
    turnOrdinals: {},
  };
}

beforeEach(() => {
  setUiLanguage("en");
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  });
  vi.stubGlobal("cancelAnimationFrame", () => undefined);
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(800);
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(460);
  Object.defineProperty(HTMLElement.prototype, "scrollTo", {
    configurable: true,
    value: vi.fn(),
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  if (originalScrollTo) {
    Object.defineProperty(HTMLElement.prototype, "scrollTo", originalScrollTo);
  } else {
    delete (HTMLElement.prototype as { scrollTo?: unknown }).scrollTo;
  }
});

describe("ThreadPanel", () => {
  it("renders the panel thread like a conversation and closes on demand", () => {
    const child = thread("thread_child", "Subagent: stats", [
      message("child-message", "Child answer"),
    ]);
    const main = thread("thread_main", "Main", []);
    useAppStore.setState({
      threads: [main, child],
      selectedThreadId: main.id,
      panelThreadId: child.id,
      runtimeThreadDetails: { [child.id]: readyDetail() },
    });

    render(
      <Tooltip.Provider>
        <ThreadPanel />
      </Tooltip.Provider>,
    );

    expect(screen.getByText("Subagent: stats")).toBeInTheDocument();
    expect(screen.getByText("Child answer")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(useAppStore.getState().panelThreadId).toBeNull();
  });

  it("can promote the panel thread into the main view", () => {
    const selectThread = vi.fn(async () => undefined);
    const child = thread("thread_child", "Subagent: stats", [
      message("child-message", "Child answer"),
    ]);
    const main = thread("thread_main", "Main", []);
    useAppStore.setState({
      threads: [main, child],
      selectedThreadId: main.id,
      panelThreadId: child.id,
      runtimeThreadDetails: { [child.id]: readyDetail() },
      selectThread,
    });

    render(
      <Tooltip.Provider>
        <ThreadPanel />
      </Tooltip.Provider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Open in main view" }));
    expect(selectThread).toHaveBeenCalledWith("thread_child");
    expect(useAppStore.getState().panelThreadId).toBeNull();
  });
});

describe("ThreadPanel resizer", () => {
  const CHILD = "thread_child";
  const resizeName = "Resize conversation panel";

  function mountPanel(width: number) {
    const child = thread(CHILD, "Subagent: stats", []);
    useAppStore.setState({
      threads: [child],
      selectedThreadId: child.id,
      panelThreadId: child.id,
      runtimeThreadDetails: { [child.id]: readyDetail() },
      threadPanelWidth: width,
    });
    render(
      <Tooltip.Provider>
        <ThreadPanel />
      </Tooltip.Provider>,
    );
    const separator = screen.getByRole("separator", { name: resizeName });
    Object.defineProperty(separator, "setPointerCapture", { value: vi.fn() });
    Object.defineProperty(separator, "hasPointerCapture", {
      value: vi.fn(() => true),
    });
    Object.defineProperty(separator, "releasePointerCapture", { value: vi.fn() });
    return separator;
  }

  it("previews and clamps the width while dragging the left edge", () => {
    const separator = mountPanel(460);

    fireEvent.pointerDown(separator, { button: 0, pointerId: 3, clientX: 700 });
    expect(document.documentElement.dataset.resizingThreadPanel).toBe("true");
    fireEvent.pointerMove(separator, { pointerId: 3, clientX: 680 });
    expect(useAppStore.getState().threadPanelWidth).toBe(480);
    expect(separator.getAttribute("aria-valuenow")).toBe("480");
    fireEvent.pointerUp(separator, { pointerId: 3, clientX: 680 });
    expect(document.documentElement.dataset.resizingThreadPanel).toBeUndefined();

    // Narrower than the minimum clamps to 320.
    fireEvent.pointerDown(separator, { button: 0, pointerId: 4, clientX: 700 });
    fireEvent.pointerMove(separator, { pointerId: 4, clientX: 3000 });
    fireEvent.pointerUp(separator, { pointerId: 4, clientX: 3000 });
    expect(useAppStore.getState().threadPanelWidth).toBe(320);

    // Widening never squeezes the main column below its minimum: jsdom reports
    // a 1024px viewport, so the panel caps at 1024 - 520.
    fireEvent.pointerDown(separator, { button: 0, pointerId: 5, clientX: 700 });
    fireEvent.pointerMove(separator, { pointerId: 5, clientX: -300 });
    fireEvent.pointerUp(separator, { pointerId: 5, clientX: -300 });
    expect(useAppStore.getState().threadPanelWidth).toBe(504);
  });

  it("resizes with the keyboard and resets on double-click", () => {
    const separator = mountPanel(460);

    fireEvent.keyDown(separator, { key: "ArrowLeft" });
    expect(useAppStore.getState().threadPanelWidth).toBe(476);
    fireEvent.keyDown(separator, { key: "Home" });
    expect(useAppStore.getState().threadPanelWidth).toBe(320);
    fireEvent.keyDown(separator, { key: "End" });
    expect(useAppStore.getState().threadPanelWidth).toBe(504);

    fireEvent.doubleClick(separator);
    expect(useAppStore.getState().threadPanelWidth).toBe(460);
  });
});