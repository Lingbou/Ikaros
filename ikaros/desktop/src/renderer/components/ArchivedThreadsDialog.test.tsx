import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useAppStore } from "../store";
import { ArchivedThreadsDialog } from "./ArchivedThreadsDialog";

const initialState = useAppStore.getState();

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
});

describe("ArchivedThreadsDialog errors", () => {
  it("renders a retryable failure instead of an empty or stale catalog", () => {
    const loadArchivedThreads = vi.fn(async () => undefined);
    useAppStore.setState({
      archivedCatalogStatus: "error",
      archivedThreads: [
        {
          id: "stale-thread",
          title: "Stale archived chat",
          defaultBranchId: "stale-branch",
          workspace: null,
          createdAt: "2026-08-15T00:00:00.000Z",
          updatedAt: "2026-08-15T00:00:00.000Z",
          archivedAt: "2026-08-15T00:00:01.000Z",
        },
      ],
      runtimeError: "disk unavailable",
      runtimeIssue: { kind: "archived_catalog", message: "disk unavailable" },
      loadArchivedThreads,
    });

    render(<ArchivedThreadsDialog open onOpenChange={() => undefined} />);

    expect(screen.getByRole("alert")).toHaveTextContent("Could not load archived chats");
    expect(screen.getByRole("alert")).toHaveTextContent("disk unavailable");
    expect(screen.queryByText("Stale archived chat")).toBeNull();
    expect(screen.queryByText("No archived chats yet")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(loadArchivedThreads).toHaveBeenCalledTimes(2);
  });

  it("keeps the loaded archive page when continuation fails", () => {
    const loadArchivedThreads = vi.fn(async () => undefined);
    const loadMoreArchivedThreads = vi.fn(async () => undefined);
    useAppStore.setState({
      archivedCatalogStatus: "ready",
      archivedCatalogHasMore: true,
      archivedCatalogMoreStatus: "error",
      archivedCatalogMoreError: "next page unavailable",
      archivedThreads: [
        {
          id: "archived-thread",
          title: "Keep this archived chat",
          defaultBranchId: "archived-branch",
          workspace: null,
          createdAt: "2026-08-15T00:00:00.000Z",
          updatedAt: "2026-08-15T00:00:00.000Z",
          archivedAt: "2026-08-15T00:00:01.000Z",
        },
      ],
      loadArchivedThreads,
      loadMoreArchivedThreads,
    });

    render(<ArchivedThreadsDialog open onOpenChange={() => undefined} />);

    expect(screen.getByText("Keep this archived chat")).toBeInTheDocument();
    const retry = screen.getByRole("button", { name: "Retry" });
    expect(retry).toHaveAttribute("title", "next page unavailable");
    fireEvent.click(retry);
    expect(loadMoreArchivedThreads).toHaveBeenCalledOnce();
  });
});
