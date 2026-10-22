import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setUiLanguage } from "../i18n";
import { useAppStore } from "../store";
import { RuntimeConnectionDialog } from "./RuntimeConnectionDialog";

const initialState = useAppStore.getState();

beforeEach(() => {
  setUiLanguage("en");
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
  vi.restoreAllMocks();
});

describe("RuntimeConnectionDialog", () => {
  it("blocks the app while the runtime is offline and retries on demand", () => {
    const retry = vi.fn(async () => undefined);
    useAppStore.setState({
      runtimeConnectionStatus: "offline",
      runtimeError: "Runtime exited before readiness (exit code 1).",
      retryRuntimeConnection: retry,
    } as never);

    render(<RuntimeConnectionDialog />);

    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(screen.getByText("Local runtime is not running")).toBeInTheDocument();
    // The raw runtime error stays behind the details disclosure.
    expect(
      screen.queryByText("Runtime exited before readiness (exit code 1)."),
    ).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Show details" }));
    expect(
      screen.getByText("Runtime exited before readiness (exit code 1)."),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(retry).toHaveBeenCalledTimes(1);
  });

  it("stays out of the way while the runtime is connected", () => {
    useAppStore.setState({ runtimeConnectionStatus: "connected" } as never);
    render(<RuntimeConnectionDialog />);
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("offers a one-click runtime install", () => {
    const installRuntime = vi.fn(async () => undefined);
    useAppStore.setState({
      runtimeConnectionStatus: "offline",
      installRuntime
    } as never);

    render(<RuntimeConnectionDialog />);

    fireEvent.click(screen.getByRole("button", { name: "Install runtime" }));
    expect(installRuntime).toHaveBeenCalledTimes(1);
  });
});

