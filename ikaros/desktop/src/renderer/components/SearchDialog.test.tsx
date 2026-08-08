import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createInitialProjects, createInitialThreads } from "../mockAgentClient";
import { useAppStore } from "../store";
import { SearchDialog } from "./SearchDialog";

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState({
    projects: createInitialProjects(),
    threads: createInitialThreads(),
    searchOpen: true,
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
});

describe("SearchDialog keyboard navigation", () => {
  it("unmounts while closed and starts with a fresh query when reopened", async () => {
    useAppStore.setState({ searchOpen: false });

    render(
      <Tooltip.Provider>
        <SearchDialog />
      </Tooltip.Provider>,
    );

    expect(screen.queryByRole("textbox", { name: "Search chats" })).not.toBeInTheDocument();

    act(() => useAppStore.getState().setSearchOpen(true));
    const input = await screen.findByRole("textbox", { name: "Search chats" });
    fireEvent.change(input, { target: { value: "archive" } });
    expect(input).toHaveValue("archive");

    fireEvent.click(screen.getByRole("button", { name: "Close search" }));
    await waitFor(() => {
      expect(screen.queryByRole("textbox", { name: "Search chats" })).not.toBeInTheDocument();
    });

    act(() => useAppStore.getState().setSearchOpen(true));
    const reopenedInput = await screen.findByRole("textbox", { name: "Search chats" });
    expect(reopenedInput).toHaveValue("");
    expect(screen.getByText("Check the source archive")).toBeInTheDocument();
  });

  it("focuses the search field and opens the first visible result on Enter", async () => {
    const selectThread = vi.fn(async () => undefined);
    useAppStore.setState({ selectThread });

    render(
      <Tooltip.Provider>
        <SearchDialog />
      </Tooltip.Provider>,
    );

    const input = screen.getByPlaceholderText("Search chats");
    await waitFor(() => expect(document.activeElement).toBe(input));

    fireEvent.change(input, { target: { value: "archive" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(selectThread).toHaveBeenCalledTimes(1);
    expect(selectThread).toHaveBeenCalledWith("thread-tools");
    expect(useAppStore.getState().searchOpen).toBe(false);
  });

  it("does nothing on Enter without a match and leaves Escape to the dialog", async () => {
    const selectThread = vi.fn(async () => undefined);
    useAppStore.setState({ selectThread });

    render(
      <Tooltip.Provider>
        <SearchDialog />
      </Tooltip.Provider>,
    );

    const input = screen.getByRole("textbox", { name: "Search chats" });
    fireEvent.change(input, { target: { value: "no matching conversation" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(selectThread).not.toHaveBeenCalled();
    expect(useAppStore.getState().searchOpen).toBe(true);

    fireEvent.keyDown(input, { key: "Escape" });
    await waitFor(() => expect(useAppStore.getState().searchOpen).toBe(false));
  });
});
