import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../store";
import { Composer } from "./Composer";

const initialState = useAppStore.getState();
let resizeCallback: ResizeObserverCallback | undefined;

beforeEach(() => {
  resizeCallback = undefined;
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: ResizeObserverCallback) {
        resizeCallback = callback;
      }

      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
  vi.unstubAllGlobals();
});

describe("Composer input and clearance", () => {
  it("uses one unified conversation surface without a mode selector", () => {
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    expect(screen.queryByRole("button", { name: "Agent" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Chat" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Research" })).toBeNull();
    expect(screen.getByRole("button", { name: "Full access" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Ikaros" })).toBeVisible();
  });

  it("does not submit while an IME composition is being confirmed", () => {
    const sendDraft = vi.fn(async () => undefined);
    useAppStore.setState({ draft: "你好", runStatus: "idle", sendDraft });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.keyDown(textarea, { key: "Enter", isComposing: true });
    expect(sendDraft).not.toHaveBeenCalled();

    fireEvent.keyDown(textarea, { key: "Enter", isComposing: false });
    expect(sendDraft).toHaveBeenCalledTimes(1);
  });

  it("opens the slash command palette and inserts the keyboard-selected command", () => {
    const sendDraft = vi.fn(async () => undefined);
    useAppStore.setState({ draft: "", runStatus: "idle", sendDraft });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.change(textarea, { target: { value: "/" } });

    expect(screen.getByRole("listbox", { name: "Slash commands" })).toBeVisible();
    expect(screen.getAllByRole("option")).toHaveLength(3);
    fireEvent.keyDown(textarea, { key: "ArrowDown" });
    expect(screen.getByRole("option", { name: /Mock feature 2/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(useAppStore.getState().draft).toBe("/mock-2 ");
    expect(screen.queryByRole("listbox", { name: "Slash commands" })).toBeNull();
    expect(sendDraft).not.toHaveBeenCalled();
  });

  it("filters slash commands and lets Escape dismiss the palette", () => {
    useAppStore.setState({ draft: "", runStatus: "idle" });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.change(textarea, { target: { value: "/3" } });
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(screen.getByRole("option", { name: /Mock feature 3/ })).toBeVisible();

    fireEvent.keyDown(textarea, { key: "Escape" });
    expect(screen.queryByRole("listbox", { name: "Slash commands" })).toBeNull();
    expect(useAppStore.getState().draft).toBe("/3");
  });

  it("does not open the slash palette during IME composition", () => {
    useAppStore.setState({ draft: "", runStatus: "idle" });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.compositionStart(textarea);
    fireEvent.change(textarea, { target: { value: "/" } });
    expect(screen.queryByRole("listbox", { name: "Slash commands" })).toBeNull();

    fireEvent.compositionEnd(textarea);
    expect(screen.getByRole("listbox", { name: "Slash commands" })).toBeVisible();
  });

  it("does not turn Enter into Stop while a run is active", () => {
    const sendDraft = vi.fn(async () => undefined);
    const stopRun = vi.fn();
    useAppStore.setState({
      draft: "Write the next message",
      runStatus: "running",
      sendDraft,
      stopRun,
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.keyDown(textarea, { key: "Enter" });
    expect(sendDraft).not.toHaveBeenCalled();
    expect(stopRun).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(stopRun).toHaveBeenCalledTimes(1);
  });

  it("reports clearance from the measured composer surface instead of a fixed height", () => {
    const onClearanceChange = vi.fn();
    const { container } = render(
      <Tooltip.Provider>
        <Composer onClearanceChange={onClearanceChange} />
      </Tooltip.Provider>,
    );
    const overlay = container.querySelector<HTMLElement>("[data-composer-overlay]");
    const surface = container.querySelector<HTMLElement>("[data-composer-surface]");
    if (!overlay || !surface || !resizeCallback) throw new Error("Composer measurement was not initialized");

    vi.spyOn(overlay, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 337, 960, 303));
    vi.spyOn(surface, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 389, 738, 247));
    act(() => resizeCallback?.([], {} as ResizeObserver));

    expect(onClearanceChange).toHaveBeenLastCalledWith(275);
  });
});
