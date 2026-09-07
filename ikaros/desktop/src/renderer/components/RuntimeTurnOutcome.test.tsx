import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { appEventText, type ToolCallEvent, type ToolResultEvent, type Turn } from "../domain";
import { setUiLanguage } from "../i18n";
import { RuntimeTurnOutcome } from "./RuntimeTurnOutcome";

afterEach(() => {
  cleanup();
  setUiLanguage("en");
});

function failedTurn(overrides: Partial<Turn> = {}): Turn {
  return {
    id: "turn-failed",
    branchId: "branch-1",
    runId: "run-failed",
    status: "failed",
    reasonCode: "provider_timeout",
    events: [],
    ...overrides,
  };
}

function call(toolName: string, status: ToolCallEvent["status"]): ToolCallEvent {
  return {
    id: "call-1",
    turnId: "turn-failed",
    createdAt: "2026-09-05T00:00:00Z",
    type: "tool_call",
    toolName,
    label: appEventText("tool.writeFile"),
    status,
    arguments: { path: "/workspace/report.md" },
  };
}

function result(status: ToolResultEvent["status"], errorCode?: string): ToolResultEvent {
  return {
    id: "result-1",
    turnId: "turn-failed",
    createdAt: "2026-09-05T00:00:01Z",
    type: "tool_result",
    toolCallId: "call-1",
    toolName: "write",
    summary: appEventText("result.writeCompleted"),
    status,
    output: "",
    ...(errorCode ? { errorCode } : {}),
  };
}

describe("RuntimeTurnOutcome", () => {
  it("shows a failed run and recorded reason even without an assistant message", () => {
    render(<RuntimeTurnOutcome turn={failedTurn()} />);
    expect(screen.getByRole("status", { name: "This run failed" })).toHaveTextContent(
      "Reason: provider_timeout",
    );
    expect(screen.getByRole("status")).toHaveTextContent("The provider request timed out.");
    expect(screen.getByRole("status")).toHaveTextContent("start a new turn");
    expect(screen.getByRole("status")).toHaveTextContent("will not automatically resume");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it.each(["queued", "running", "completed"] as const)(
    "does not show an outcome for %s turns",
    (status) => {
      render(<RuntimeTurnOutcome turn={failedTurn({ status })} />);
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    },
  );

  it("shows unknown historical reasons honestly, without rendering unsafe reason text", () => {
    const view = render(<RuntimeTurnOutcome turn={failedTurn({ reasonCode: null })} />);
    expect(screen.getByRole("status")).toHaveTextContent("No detailed failure reason is available.");
    view.rerender(<RuntimeTurnOutcome turn={failedTurn({ reasonCode: "bad\nreason" })} />);
    expect(screen.getByRole("status")).toHaveTextContent("No detailed failure reason is available.");
    expect(screen.getByRole("status")).not.toHaveTextContent("bad");
  });

  it("keeps successful results authoritative after a later run failure", () => {
    render(
      <RuntimeTurnOutcome
        turn={failedTurn({ events: [call("write", "running"), result("success")] })}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("reported completion");
    expect(screen.getByRole("status")).not.toHaveTextContent("may already have changed files");
  });

  it.each(["write", "edit", "process_start"])(
    "warns that an unsettled %s operation may already have changed files",
    (toolName) => {
      render(<RuntimeTurnOutcome turn={failedTurn({ events: [call(toolName, "running")] })} />);
      expect(screen.getByRole("status")).toHaveTextContent("may already have changed files");
      expect(screen.getByRole("status")).toHaveTextContent("not rolled back");
    },
  );

  it("recognizes Runtime interruption results whose tool status is failed", () => {
    render(
      <RuntimeTurnOutcome
        turn={failedTurn({
          reasonCode: "runtime_interrupted",
          events: [call("write", "error"), result("error", "runtime_interrupted")],
        })}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("may already have changed files");
  });

  it.each(["write", "edit", "process_start"])(
    "warns about possible partial effects when %s reports an ordinary failure",
    (toolName) => {
      render(
        <RuntimeTurnOutcome
          turn={failedTurn({
            events: [call(toolName, "error"), { ...result("error", "operation_failed"), toolName }],
          })}
        />,
      );
      expect(screen.getByRole("status")).toHaveTextContent("may already have changed files");
    },
  );

  it("shows cancellation with interrupted mutation results", () => {
    render(
      <RuntimeTurnOutcome
        turn={failedTurn({
          status: "interrupted",
          reasonCode: "cancelled",
          events: [call("write", "interrupted"), result("interrupted", "cancelled")],
        })}
      />,
    );
    expect(screen.getByRole("status", { name: "This run was cancelled" })).toHaveTextContent(
      "may already have changed files",
    );
  });

  it("does not label a read interruption as a possible file change", () => {
    render(<RuntimeTurnOutcome turn={failedTurn({ events: [call("read", "interrupted")] })} />);
    expect(screen.getByRole("status")).not.toHaveTextContent("may already have changed files");
  });

  it("retains an unknown machine reason and translates known explanations into Chinese", () => {
    const view = render(<RuntimeTurnOutcome turn={failedTurn({ reasonCode: "future_reason" })} />);
    expect(screen.getByRole("status")).toHaveTextContent("Reason: future_reason");
    act(() => setUiLanguage("zh-CN"));
    view.rerender(<RuntimeTurnOutcome turn={failedTurn({ reasonCode: "provider_authentication" })} />);
    expect(screen.getByRole("status", { name: "本次运行失败" })).toHaveTextContent(
      "提供商拒绝了凭据",
    );
    expect(screen.getByRole("status")).toHaveTextContent("provider_authentication");
  });

});

it("treats a started command as uncertain until a later successful exit is recorded", () => {
  const start: ToolResultEvent = {
    ...result("success"), toolName: "process_start",
    details: { processId: "process-1", processState: "running", exitCode: null },
  };
  const view = render(<RuntimeTurnOutcome turn={failedTurn({ events: [call("process_start", "success"), start] })} />);
  expect(screen.getByRole("status")).toHaveTextContent("may already have changed files");
  const finished: ToolResultEvent = {
    ...start, id: "wait-result", toolCallId: "wait-call", toolName: "process_wait",
    details: { processId: "process-1", processState: "exited", exitCode: 0 },
  };
  view.rerender(<RuntimeTurnOutcome turn={failedTurn({ events: [call("process_start", "success"), start, finished] })} />);
  expect(screen.getByRole("status")).not.toHaveTextContent("may already have changed files");
});
