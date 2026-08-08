import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageEvent, Thread } from "../domain";
import { useAppStore } from "../store";
import { EventFeed } from "./EventFeed";

const initialState = useAppStore.getState();
const rowHeights = new Map<number, number>();
const originalScrollTo = Object.getOwnPropertyDescriptor(
  HTMLElement.prototype,
  "scrollTo",
);

function message(
  id: string,
  turnId: string,
  role: MessageEvent["role"],
  content: string,
  status: MessageEvent["status"] = "complete",
): MessageEvent {
  return {
    id,
    turnId,
    createdAt: "2026-08-06T06:00:00.000Z",
    type: "message",
    role,
    content,
    status,
  };
}

function threadWith(events: MessageEvent[]): Thread {
  return {
    id: "thread-feed-measurement",
    projectId: null,
    title: "Feed measurement",
    activeBranchId: "branch-feed-measurement",
    branches: [
      {
        id: "branch-feed-measurement",
        threadId: "thread-feed-measurement",
        label: { source: "app", kind: "main" },
        turns: [
          {
            id: "turn-feed-measurement",
            branchId: "branch-feed-measurement",
            status: "running",
            events,
          },
        ],
        createdAt: "2026-08-06T06:00:00.000Z",
      },
    ],
    updatedAt: "2026-08-06T06:00:00.000Z",
  };
}

function translateY(element: HTMLElement) {
  const match = element.style.transform.match(/translateY\(([-\d.]+)px\)/);
  if (!match) throw new Error(`Missing translateY transform: ${element.style.transform}`);
  return Number(match[1]);
}

beforeEach(() => {
  rowHeights.clear();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(_callback: ResizeObserverCallback) {}
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
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(
    function (this: HTMLElement) {
      const index = this.dataset.index;
      if (index !== undefined) return rowHeights.get(Number(index)) ?? 0;
      if (this.getAttribute("role") === "log") return 800;
      return 0;
    },
  );
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(900);
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

describe("EventFeed dynamic row measurement", () => {
  it("renders the empty conversation without a decorative icon", () => {
    useAppStore.setState({ selectedThreadId: null });

    const { container } = render(
      <Tooltip.Provider>
        <EventFeed bottomClearance={0} />
      </Tooltip.Provider>,
    );

    expect(container.querySelector("h1")).toHaveClass(
      "text-[24px]",
      "leading-[30px]",
    );
    expect(container.querySelector("p")).toHaveClass(
      "text-[12px]",
      "leading-[18px]",
    );
    expect(container.querySelector("svg")).toBeNull();
  });

  it("keeps the next turn below a measured long assistant response", async () => {
    rowHeights.set(0, 72);
    rowHeights.set(1, 280);
    rowHeights.set(2, 72);
    const thread = threadWith([
      message("user-1", "turn-feed-measurement", "user", "Summarize this."),
      message(
        "assistant-1",
        "turn-feed-measurement",
        "assistant",
        "A long response\n\n- First point\n- Second point\n- Third point",
        "streaming",
      ),
    ]);
    useAppStore.setState({
      threads: [thread],
      selectedThreadId: thread.id,
      runStatus: "running",
    });

    const { container } = render(
      <Tooltip.Provider>
        <EventFeed bottomClearance={0} />
      </Tooltip.Provider>,
    );
    await waitFor(() => {
      expect(container.querySelectorAll<HTMLElement>("[data-index]")).toHaveLength(2);
    });

    act(() => {
      useAppStore.getState().appendAgentEvent(
        thread.id,
        thread.activeBranchId,
        message("user-2", "turn-follow-up", "user", "What does that mean?"),
        "running",
      );
    });

    await waitFor(() => {
      expect(container.querySelectorAll<HTMLElement>("[data-index]")).toHaveLength(3);
    });
    const assistantRow = container.querySelector<HTMLElement>('[data-index="1"]');
    const nextUserRow = container.querySelector<HTMLElement>('[data-index="2"]');
    if (!assistantRow || !nextUserRow) throw new Error("Expected all feed rows");

    expect(translateY(nextUserRow)).toBeGreaterThanOrEqual(
      translateY(assistantRow) + 280,
    );
  });

  it("follows a same-length streaming update without resetting measured rows", async () => {
    rowHeights.set(0, 72);
    rowHeights.set(1, 128);
    const thread = threadWith([
      message("user-1", "turn-feed-measurement", "user", "Keep following."),
      message(
        "assistant-1",
        "turn-feed-measurement",
        "assistant",
        "AAAA",
        "streaming",
      ),
    ]);
    useAppStore.setState({
      threads: [thread],
      selectedThreadId: thread.id,
      runStatus: "running",
    });

    const { container } = render(
      <Tooltip.Provider>
        <EventFeed bottomClearance={0} />
      </Tooltip.Provider>,
    );
    const scrollTo = vi.mocked(HTMLElement.prototype.scrollTo);
    await waitFor(() => expect(scrollTo).toHaveBeenCalled());
    const callsBeforeUpdate = scrollTo.mock.calls.length;

    act(() => {
      useAppStore.getState().appendAgentEvent(
        thread.id,
        thread.activeBranchId,
        message(
          "assistant-1",
          "turn-feed-measurement",
          "assistant",
          "BBBB",
          "streaming",
        ),
        "running",
      );
    });

    await waitFor(() => {
      expect(container).toHaveTextContent("BBBB");
      expect(scrollTo.mock.calls.length).toBeGreaterThan(callsBeforeUpdate);
    });
  });
});
