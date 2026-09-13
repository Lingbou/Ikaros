import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Turn } from "../domain";
import { RuntimeRunProgress } from "./RuntimeRunProgress";

afterEach(() => { cleanup(); vi.useRealTimers(); });

const turn: Turn = {
  id: "turn", branchId: "branch", runId: "run", status: "queued", events: [],
  runProgress: {
    queuedAt: "2026-09-07T00:00:00Z", startedAt: null, settledAt: null,
    modelCalls: 0,
  },
};

describe("RuntimeRunProgress", () => {
  it("counts queue time separately and freezes elapsed time at settlement", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-07T00:02:00Z"));
    const view = render(<RuntimeRunProgress turn={turn} />);
    expect(screen.getByLabelText("Task execution")).toHaveTextContent("Queued for 2:00");
    expect(screen.getByLabelText("Task execution")).toHaveTextContent("Model calls 0");
    view.rerender(<RuntimeRunProgress turn={{ ...turn, status: "running", runProgress: { ...turn.runProgress!, startedAt: "2026-09-07T00:02:00Z", modelCalls: 3 } }} />);
    act(() => { vi.advanceTimersByTime(12000); });
    expect(screen.getByLabelText("Task execution")).toHaveTextContent("Elapsed 0:12");
    expect(screen.getByLabelText("Task execution")).toHaveTextContent("Model calls 3");
    view.rerender(<RuntimeRunProgress turn={{ ...turn, status: "failed", runProgress: { ...turn.runProgress!, startedAt: "2026-09-07T00:02:00Z", settledAt: "2026-09-07T00:02:12Z", modelCalls: 3 } }} />);
    act(() => { vi.advanceTimersByTime(60000); });
    expect(screen.getByLabelText("Task execution")).toHaveTextContent("Elapsed 0:12");
  });

  it("surfaces context compaction state", () => {
    const view = render(
      <RuntimeRunProgress
        turn={{
          ...turn,
          status: "running",
          runProgress: { ...turn.runProgress!, compactions: 1, compacting: true },
        }}
      />,
    );
    expect(screen.getByLabelText("Task execution")).toHaveTextContent("Organizing context…");
    view.rerender(
      <RuntimeRunProgress
        turn={{
          ...turn,
          status: "running",
          runProgress: { ...turn.runProgress!, compactions: 1, compacting: false },
        }}
      />,
    );
    expect(screen.getByLabelText("Task execution")).toHaveTextContent("Context organized 1");
  });
});
