import "@testing-library/jest-dom/vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RuntimeFileChangeRecord, RuntimeFilePreviewText } from "../../shared/runtime";
import { setUiLanguage } from "../i18n";
import { useAppStore } from "../store";
import { FilePanel } from "./FilePanel";

const api = vi.hoisted(() => ({ previewFile: vi.fn(), getFileChange: vi.fn() }));
vi.mock("../runtimeClient", () => ({
  createRuntimeClient: () => api,
  isRuntimeRpcError: () => false,
}));
const initialState = useAppStore.getState();

function page(overrides: Partial<RuntimeFilePreviewText> = {}): RuntimeFilePreviewText {
  return {
    threadId: "thread-files", path: "/workspace/notes.txt", status: "text",
    revision: "a".repeat(64), encoding: "utf-8", bom: false,
    content: "alpha\r\nbeta\n", lineStart: 1, lineEnd: 2,
    nextOffset: null, truncated: false, truncationReason: null,
    ...overrides,
  };
}

function change(overrides: Partial<RuntimeFileChangeRecord> = {}): RuntimeFileChangeRecord {
  return {
    threadId: "thread-files", toolCallItemId: "call-edit", path: "/workspace/notes.txt",
    operation: "edit", recordedAt: "2026-09-05T01:00:00Z", status: "recorded",
    before: { exists: true, byteCount: 6, revision: "b".repeat(64), encoding: "utf-8", bom: false, newline: "lf", lineCount: 1 },
    after: { exists: true, byteCount: 6, revision: "c".repeat(64), encoding: "utf-8", bom: false, newline: "lf", lineCount: 1 },
    diff: "--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1 @@\n-alpha\n+omega\n", additions: 1, deletions: 1,
    ...overrides,
  };
}

function pending<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  setUiLanguage("en");
  api.previewFile.mockReset().mockResolvedValue(page());
  api.getFileChange.mockReset().mockResolvedValue(change());
  useAppStore.setState({
    runtimeMode: true, selectedThreadId: "thread-files", draft: "Keep this request",
    fileSelection: { threadId: "thread-files", path: "notes.txt", view: "current" },
  });
});
afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
  setUiLanguage("en");
});

describe("FilePanel", () => {
  it("loads current contents with line numbers and appends a reference without sending", async () => {
    const sendDraft = vi.fn();
    useAppStore.setState({ sendDraft });
    render(<FilePanel />);

    const region = await screen.findByRole("region", { name: "File contents" });
    expect(within(region).getByText("alpha")).toBeVisible();
    expect(region.querySelectorAll("[data-line-number]")).toHaveLength(2);
    expect(region.querySelector('[data-line-number="2"]')).toHaveTextContent("beta");
    expect(api.previewFile).toHaveBeenCalledWith({ threadId: "thread-files", path: "notes.txt", offset: 1 });
    fireEvent.click(screen.getByRole("button", { name: "Reference in message" }));
    expect(useAppStore.getState().draft).toBe("Keep this request\n\nFile: /workspace/notes.txt (lines 1–2)");
    expect(sendDraft).not.toHaveBeenCalled();
    expect(screen.queryByRole("textbox", { name: "File contents" })).toBeNull();
  });

  it("uses server offsets and the original revision for next and previous pages", async () => {
    api.previewFile
      .mockResolvedValueOnce(page({ nextOffset: 3, truncated: true, truncationReason: "line_limit" }))
      .mockResolvedValueOnce(page({ content: "gamma\rdelta", lineStart: 3, lineEnd: 4 }))
      .mockResolvedValueOnce(page({ nextOffset: 3, truncated: true, truncationReason: "line_limit" }));
    render(<FilePanel />);
    await screen.findByText("alpha");
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    await screen.findByText("gamma");
    expect(screen.queryByText("alpha")).toBeNull();
    expect(api.previewFile).toHaveBeenLastCalledWith({ threadId: "thread-files", path: "notes.txt", offset: 3, expectedRevision: "a".repeat(64) });
    expect(screen.getByText("delta").parentElement).toHaveAttribute("data-line-number", "4");
    fireEvent.click(screen.getByRole("button", { name: "Previous page" }));
    await screen.findByText("alpha");
    expect(api.previewFile).toHaveBeenLastCalledWith({ threadId: "thread-files", path: "notes.txt", offset: 1, expectedRevision: "a".repeat(64) });
  });

  it("keeps an earlier page separate when the file changes and refreshes from the beginning", async () => {
    api.previewFile
      .mockResolvedValueOnce(page({ nextOffset: 3, truncated: true, truncationReason: "line_limit" }))
      .mockResolvedValueOnce({ threadId: "thread-files", path: "/workspace/notes.txt", status: "unavailable", reason: "revision_changed" })
      .mockResolvedValueOnce(page({ content: "new version\n", lineEnd: 1, revision: "d".repeat(64) }));
    render(<FilePanel />);
    await screen.findByText("alpha");
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("file changed while paging");
    expect(screen.getByText("alpha")).toBeVisible();
    expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Reference in message" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await screen.findByText("new version");
    expect(screen.queryByText("alpha")).toBeNull();
    expect(api.previewFile).toHaveBeenLastCalledWith({ threadId: "thread-files", path: "notes.txt", offset: 1 });
  });

  it("does not promise continuation beyond the scan ceiling", async () => {
    api.previewFile
      .mockResolvedValueOnce(page({ nextOffset: 3, truncated: true, truncationReason: "scan_limit" }))
      .mockResolvedValueOnce({ threadId: "thread-files", path: "/workspace/notes.txt", status: "unavailable", reason: "scan_limit" });
    render(<FilePanel />);
    await screen.findByText(/Further pages may be unavailable/);
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("scan limit");
    expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();
    expect(screen.getByText("alpha")).toBeVisible();
  });

  it.each(["binary_file", "unsupported_encoding", "too_large", "protected_content"])("renders %s without pretending an empty preview succeeded", async (reason) => {
    api.previewFile.mockResolvedValue({ threadId: "thread-files", path: null, status: "unavailable", reason });
    render(<FilePanel />);
    expect(await screen.findByRole("alert")).toBeVisible();
    expect(screen.queryByRole("region", { name: "File contents" })).toBeNull();
    expect(screen.queryByText("This file is empty.")).toBeNull();
  });

  it("distinguishes a real empty UTF-8 file", async () => {
    api.previewFile.mockResolvedValue(page({ content: "", lineEnd: 0, bom: true }));
    render(<FilePanel />);
    expect(await screen.findByText("This file is empty.")).toBeVisible();
    expect(screen.getByText("UTF-8 BOM")).toBeVisible();
  });

  it("removes old contents when a refresh withholds protected file text", async () => {
    api.previewFile.mockResolvedValueOnce(page()).mockResolvedValueOnce({ threadId: "thread-files", path: null, status: "unavailable", reason: "protected_content" });
    render(<FilePanel />);
    await screen.findByText("alpha");
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await screen.findByRole("alert");
    expect(screen.queryByText("alpha")).toBeNull();
    expect(screen.queryByRole("button", { name: "Reference in message" })).toBeNull();
  });

  it("opens a command-created file by path in the current conversation folder", async () => {
    useAppStore.setState({ fileSelection: { threadId: "thread-files", path: "", view: "current" } });
    render(<FilePanel />);
    expect(api.previewFile).not.toHaveBeenCalled();
    fireEvent.change(screen.getByRole("textbox", { name: "File path" }), { target: { value: "generated/report.txt" } });
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    await screen.findByText("alpha");
    expect(api.previewFile).toHaveBeenCalledWith({ threadId: "thread-files", path: "generated/report.txt", offset: 1 });
  });

  it("shows the immutable operation diff independently of the current file", async () => {
    useAppStore.setState({ fileSelection: { threadId: "thread-files", path: "notes.txt", toolCallItemId: "call-edit", sourceToolCallItemId: "call-edit", view: "change" } });
    render(<FilePanel />);
    const diff = await screen.findByRole("region", { name: "Recorded file diff" });
    expect(diff).toHaveTextContent("-alpha");
    expect(diff).toHaveTextContent("+omega");
    expect(api.getFileChange).toHaveBeenCalledWith({ threadId: "thread-files", toolCallItemId: "call-edit" });
    expect(api.previewFile).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Reference in message" }));
    expect(useAppStore.getState().draft).toContain("File change: /workspace/notes.txt (recorded");
    expect(useAppStore.getState().draft).not.toContain("call-edit");
    fireEvent.click(screen.getByRole("tab", { name: "Current file" }));
    await screen.findByRole("region", { name: "File contents" });
    expect(api.previewFile).toHaveBeenCalledWith({ threadId: "thread-files", path: "notes.txt", sourceToolCallItemId: "call-edit", offset: 1 });
    expect(screen.queryByRole("region", { name: "Recorded file diff" })).toBeNull();
  });

  it("shows format-only changes through metadata even with an empty patch", async () => {
    const result = change({ diff: "", additions: 0, deletions: 0 });
    result.after = { ...result.after, byteCount: 10, bom: true, newline: "crlf" };
    api.getFileChange.mockResolvedValue(result);
    useAppStore.setState({ fileSelection: { threadId: "thread-files", path: "notes.txt", toolCallItemId: "call-edit", view: "change" } });
    render(<FilePanel />);
    expect(await screen.findByText(/File bytes or format changed/)).toBeVisible();
    expect(screen.getByText("6 bytes · 1 lines · UTF-8 · LF")).toBeVisible();
    expect(screen.getByText("10 bytes · 1 lines · UTF-8 BOM · CRLF")).toBeVisible();
    expect(screen.queryByText(/did not change/)).toBeNull();
  });

  it("does not reconstruct an unrecorded historical diff from current contents", async () => {
    api.getFileChange.mockResolvedValue({ ...change(), status: "unavailable", reason: "not_recorded", path: null, recordedAt: null, before: null, after: null });
    useAppStore.setState({ fileSelection: { threadId: "thread-files", path: "notes.txt", toolCallItemId: "call-edit", view: "change" } });
    render(<FilePanel />);
    expect(await screen.findByRole("alert")).toHaveTextContent("No diff was recorded");
    expect(api.previewFile).not.toHaveBeenCalled();
    expect(screen.queryByRole("region", { name: "Recorded file diff" })).toBeNull();
  });

  it("ignores a late response after opening a different file", async () => {
    const first = pending<RuntimeFilePreviewText>();
    api.previewFile.mockReturnValueOnce(first.promise).mockResolvedValueOnce(page({ path: "/workspace/other.txt", content: "other\n", lineEnd: 1 }));
    render(<FilePanel />);
    act(() => useAppStore.getState().openFile({ threadId: "thread-files", path: "other.txt", view: "current" }));
    await screen.findByText("other");
    await act(async () => first.resolve(page()));
    expect(screen.queryByText("alpha")).toBeNull();
    expect(screen.getByText("other")).toBeVisible();
  });

  it.each(["close", "thread"])("ignores a late response after %s", async (action) => {
    const first = pending<RuntimeFilePreviewText>();
    api.previewFile.mockReturnValue(first.promise);
    render(<FilePanel />);
    if (action === "close") fireEvent.click(screen.getByRole("button", { name: "Close file viewer" }));
    else act(() => useAppStore.setState({ selectedThreadId: "another-thread" }));
    await act(async () => first.resolve(page()));
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(useAppStore.getState().fileSelection).toBeNull();
  });

  it("retries transport failures without touching the draft", async () => {
    api.previewFile.mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(page());
    render(<FilePanel />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load");
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(api.previewFile).toHaveBeenCalledTimes(2));
    await screen.findByText("alpha");
    expect(useAppStore.getState().draft).toBe("Keep this request");
  });
});
