import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useAppStore } from "../store";
import { RuntimeStatusBanner } from "./RuntimeStatusBanner";

const initialState = useAppStore.getState();

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
});

function renderBanner() {
  return render(
    <Tooltip.Provider>
      <RuntimeStatusBanner />
    </Tooltip.Provider>,
  );
}

describe("RuntimeStatusBanner", () => {
  it("shows startup and reconnect lifecycle without presenting an empty app", () => {
    useAppStore.setState({ runtimeConnectionStatus: "starting" });
    const view = renderBanner();
    expect(screen.getByRole("status")).toHaveTextContent("Starting local runtime");

    useAppStore.setState({ runtimeConnectionStatus: "reconnecting" });
    view.rerender(
      <Tooltip.Provider>
        <RuntimeStatusBanner />
      </Tooltip.Provider>,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Reconnecting to local runtime");
  });

  it("offers the scoped send retry for a restored draft", () => {
    const retryRuntimeIssue = vi.fn(async () => undefined);
    useAppStore.setState({
            runtimeConnectionStatus: "connected",
      runtimeError: "request rejected",
      runtimeIssue: {
        kind: "send",
        message: "request rejected",
        threadId: null,
        prompt: "restored prompt",
      },
      draft: "restored prompt",
      retryRuntimeIssue,
    });
    renderBanner();

    expect(screen.getByRole("alert")).toHaveTextContent("Message was not sent");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retryRuntimeIssue).toHaveBeenCalledOnce();
    expect(useAppStore.getState().draft).toBe("restored prompt");
  });

  it("does not offer a stale send retry after navigation changes its target", () => {
    useAppStore.setState({
            runtimeConnectionStatus: "connected",
      runtimeError: "request rejected",
      runtimeIssue: {
        kind: "send",
        message: "request rejected",
        threadId: "thread-old",
        prompt: "restored prompt",
      },
      selectedThreadId: "thread-new",
      draft: "restored prompt",
    });
    renderBanner();

    expect(screen.getByRole("alert")).toHaveTextContent("Message was not sent");
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(screen.getByRole("button", { name: "Dismiss" })).toBeInTheDocument();
  });

  it("leaves an offline Runtime to the blocking dialog", () => {
    const retryRuntimeConnection = vi.fn(async () => undefined);
    useAppStore.setState({
      runtimeConnectionStatus: "offline",
      runtimeError: "socket closed",
      runtimeIssue: { kind: "connection", message: "socket closed" },
      retryRuntimeConnection,
    });
    renderBanner();

    expect(screen.queryByRole("alert")).toBeNull();
    expect(retryRuntimeConnection).not.toHaveBeenCalled();
  });
});
