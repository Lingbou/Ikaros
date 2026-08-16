import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  RUNTIME_PROTOCOL_MANIFEST,
  type IkarosRuntimeBridgeApi,
  type RuntimeMemoryRecord,
  type RuntimeMemorySummary
} from "../../shared/runtime";
import { setUiLanguage } from "../i18n";
import { MemorySettings } from "./MemorySettings";

const MEMORY_ID = `memory_${"1".repeat(32)}`;
const SECOND_MEMORY_ID = `memory_${"2".repeat(32)}`;

const summary: RuntimeMemorySummary = {
  id: MEMORY_ID,
  kind: "preference",
  scope: { type: "global", key: null },
  revision: 3,
  state: "active",
  preview: "Use concise technical explanations.",
  createdAt: "2026-08-15T00:00:00.000Z",
  updatedAt: "2026-08-16T00:00:00.000Z",
  forgottenAt: null
};

const record: RuntimeMemoryRecord = {
  ...summary,
  content: "Use concise technical explanations.",
  provenance: {
    sourceKind: "user_explicit",
    threadId: null,
    turnId: null,
    itemId: null,
    status: "not_applicable"
  }
};

function ok<T>(value: T) {
  return Promise.resolve({ ok: true as const, value });
}

function installRuntime(overrides: Partial<IkarosRuntimeBridgeApi> = {}) {
  const runtime = {
    listMemories: vi.fn(() =>
      ok({ memories: [summary], nextCursor: null, hasMore: false })
    ),
    getMemory: vi.fn(() => ok({ memory: record })),
    createMemory: vi.fn(() =>
      ok({ memoryId: SECOND_MEMORY_ID, resultingRevision: 1, created: true })
    ),
    correctMemory: vi.fn(() =>
      ok({ memoryId: MEMORY_ID, resultingRevision: 4, created: false })
    ),
    forgetMemory: vi.fn(() =>
      ok({ memoryId: MEMORY_ID, resultingRevision: 4, created: false })
    ),
    ...overrides
  } as unknown as IkarosRuntimeBridgeApi;
  Object.defineProperty(window, "ikarosDesktop", {
    configurable: true,
    value: { runtime }
  });
  return runtime;
}

beforeEach(() => setUiLanguage("en"));

afterEach(() => {
  cleanup();
  Reflect.deleteProperty(window, "ikarosDesktop");
  setUiLanguage("en");
});

describe("MemorySettings", () => {
  it("shows a real load failure when the Runtime bridge is unavailable", async () => {
    render(<MemorySettings workspaces={[]} preferredWorkspaceId={null} />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load memories.");
    expect(screen.queryByText("No memories here")).toBeNull();
  });

  it("loads active Runtime memories and verifies provenance without Mock data", async () => {
    const runtime = installRuntime();

    render(<MemorySettings workspaces={[]} preferredWorkspaceId={null} />);

    expect(await screen.findByText("Use concise technical explanations.")).toBeTruthy();
    expect(await screen.findByText("Added manually")).toBeTruthy();
    expect(runtime.listMemories).toHaveBeenCalledWith({
      limit: 25,
      state: "active"
    });
    expect(runtime.getMemory).toHaveBeenCalledWith(MEMORY_ID);
    expect(screen.queryByText(/mock/i)).toBeNull();
  });

  it("uses exact filters and appends cursor pages without duplicates", async () => {
    const secondSummary: RuntimeMemorySummary = {
      ...summary,
      id: SECOND_MEMORY_ID,
      kind: "project",
      scope: { type: "workspace", key: "workspace-stable-id" },
      preview: "Release checklist",
      revision: 1
    };
    const listMemories = vi
      .fn()
      .mockImplementationOnce(() =>
        ok({ memories: [summary], nextCursor: "cursor-1", hasMore: true })
      )
      .mockImplementationOnce(() =>
        ok({ memories: [summary, secondSummary], nextCursor: null, hasMore: false })
      )
      .mockImplementation(() =>
        ok({ memories: [secondSummary], nextCursor: null, hasMore: false })
      );
    const runtime = installRuntime({ listMemories } as Partial<IkarosRuntimeBridgeApi>);

    render(
      <MemorySettings
        workspaces={[{ id: "workspace-stable-id", name: "Ikaros" }]}
        preferredWorkspaceId="workspace-stable-id"
      />
    );

    await screen.findByText("Use concise technical explanations.");
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("Release checklist")).toBeTruthy();
    expect(screen.getAllByText("Use concise technical explanations.")).toHaveLength(1);
    expect(listMemories).toHaveBeenNthCalledWith(2, {
      limit: 25,
      state: "active",
      cursor: "cursor-1"
    });

    fireEvent.change(screen.getByLabelText("Filter by scope"), {
      target: { value: "workspace:workspace-stable-id" }
    });
    fireEvent.change(screen.getByLabelText("Filter by kind"), {
      target: { value: "project" }
    });
    fireEvent.change(screen.getByLabelText("Filter by state"), {
      target: { value: "forgotten" }
    });

    await waitFor(() =>
      expect(listMemories).toHaveBeenLastCalledWith({
        limit: 25,
        scope: { type: "workspace", key: "workspace-stable-id" },
        kind: "project",
        state: "forgotten"
      })
    );
    expect(runtime).toBeTruthy();
  });

  it("creates explicit workspace Memory and refreshes the authoritative first page", async () => {
    const runtime = installRuntime({
      listMemories: vi.fn(() =>
        ok({ memories: [], nextCursor: null, hasMore: false })
      )
    } as Partial<IkarosRuntimeBridgeApi>);

    render(
      <MemorySettings
        workspaces={[{ id: "workspace-stable-id", name: "Ikaros" }]}
        preferredWorkspaceId="workspace-stable-id"
      />
    );
    await screen.findByText("No memories here");
    fireEvent.click(screen.getByRole("button", { name: "Add memory" }));
    const dialog = screen.getByRole("dialog", { name: "Add memory" });
    expect((within(dialog).getByLabelText("Scope") as HTMLSelectElement).value).toBe(
      "workspace:workspace-stable-id"
    );
    fireEvent.change(within(dialog).getByLabelText("Kind"), {
      target: { value: "project" }
    });
    fireEvent.change(within(dialog).getByLabelText("Memory content"), {
      target: { value: "Keep the Runtime architecture provider-neutral.\n" }
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Add memory" }));

    await waitFor(() =>
      expect(runtime.createMemory).toHaveBeenCalledWith({
        kind: "project",
        scope: { type: "workspace", key: "workspace-stable-id" },
        content: "Keep the Runtime architecture provider-neutral.\n",
        clientRequestId: expect.stringMatching(/^memory_create_/)
      })
    );
    const sent = vi.mocked(runtime.createMemory).mock.calls[0]?.[0] as unknown as Record<
      string,
      unknown
    >;
    expect(sent).not.toHaveProperty("source");
    await waitFor(() => expect(runtime.listMemories).toHaveBeenCalledTimes(2));
  });

  it("corrects the full record and locks a conflicting draft instead of overwriting", async () => {
    const correctMemory = vi.fn(() =>
      Promise.resolve({
        ok: false as const,
        error: {
          kind: "json_rpc" as const,
          code: RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.code,
          message: RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.message,
          reasonCode: "memory_revision_conflict" as const
        }
      })
    );
    const runtime = installRuntime({ correctMemory } as Partial<IkarosRuntimeBridgeApi>);

    render(<MemorySettings workspaces={[]} preferredWorkspaceId={null} />);
    await screen.findByText("Use concise technical explanations.");
    fireEvent.click(screen.getByRole("button", { name: "Correct memory" }));
    const dialog = await screen.findByRole("dialog", { name: "Correct memory" });
    const textarea = await within(dialog).findByLabelText("Memory content");
    expect((textarea as HTMLTextAreaElement).value).toBe(record.content);
    fireEvent.change(textarea, { target: { value: "Use short, evidence-backed answers." } });
    const save = within(dialog).getByRole("button", { name: "Save correction" });
    fireEvent.click(save);

    await waitFor(() =>
      expect(correctMemory).toHaveBeenCalledWith({
        memoryId: MEMORY_ID,
        expectedRevision: 3,
        content: "Use short, evidence-backed answers.",
        clientRequestId: expect.stringMatching(/^memory_correct_/)
      })
    );
    expect(
      await within(dialog).findByText(/changed elsewhere/i)
    ).toBeTruthy();
    expect((save as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(save);
    expect(correctMemory).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(runtime.listMemories).toHaveBeenCalledTimes(2));
  });

  it("invalidates Correct when the authoritative record was forgotten elsewhere", async () => {
    const externallyForgotten: RuntimeMemoryRecord = {
      ...record,
      revision: 4,
      state: "forgotten",
      content: null,
      forgottenAt: "2026-08-17T00:00:00.000Z"
    };
    const getMemory = vi
      .fn()
      .mockImplementationOnce(() => ok({ memory: record }))
      .mockImplementation(() => ok({ memory: externallyForgotten }));
    const listMemories = vi
      .fn()
      .mockImplementationOnce(() =>
        ok({ memories: [summary], nextCursor: null, hasMore: false })
      )
      .mockImplementation(() =>
        ok({ memories: [], nextCursor: null, hasMore: false })
      );
    const runtime = installRuntime({
      getMemory,
      listMemories
    } as Partial<IkarosRuntimeBridgeApi>);

    render(<MemorySettings workspaces={[]} preferredWorkspaceId={null} />);
    await screen.findByText("Added manually");
    fireEvent.click(screen.getByRole("button", { name: "Correct memory" }));
    const dialog = await screen.findByRole("dialog", { name: "Correct memory" });

    expect(await within(dialog).findByText(/already been forgotten/i)).toBeTruthy();
    expect(
      (within(dialog).getByRole("button", { name: "Save correction" }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
    expect(runtime.correctMemory).not.toHaveBeenCalled();
    await waitFor(() => expect(listMemories).toHaveBeenCalledTimes(2));
  });

  it("re-reads the current revision before a confirmed Forget", async () => {
    const latest = { ...record, revision: 4 };
    const getMemory = vi.fn(() => ok({ memory: latest }));
    const runtime = installRuntime({ getMemory } as Partial<IkarosRuntimeBridgeApi>);

    render(<MemorySettings workspaces={[]} preferredWorkspaceId={null} />);
    await screen.findByText("Use concise technical explanations.");
    fireEvent.click(screen.getByRole("button", { name: "Forget memory" }));
    const dialog = screen.getByRole("dialog", { name: "Forget this memory?" });
    const confirm = within(dialog).getByRole("button", { name: "Forget memory" });
    await waitFor(() => expect((confirm as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(confirm);

    await waitFor(() =>
      expect(runtime.forgetMemory).toHaveBeenCalledWith({
        memoryId: MEMORY_ID,
        expectedRevision: 4,
        clientRequestId: expect.stringMatching(/^memory_forget_/)
      })
    );
    await waitFor(() => expect(runtime.listMemories).toHaveBeenCalledTimes(2));
  });

  it("invalidates Forget when the authoritative record was forgotten elsewhere", async () => {
    const externallyForgotten: RuntimeMemoryRecord = {
      ...record,
      revision: 4,
      state: "forgotten",
      content: null,
      forgottenAt: "2026-08-17T00:00:00.000Z"
    };
    const getMemory = vi
      .fn()
      .mockImplementationOnce(() => ok({ memory: record }))
      .mockImplementation(() => ok({ memory: externallyForgotten }));
    const listMemories = vi
      .fn()
      .mockImplementationOnce(() =>
        ok({ memories: [summary], nextCursor: null, hasMore: false })
      )
      .mockImplementation(() =>
        ok({ memories: [], nextCursor: null, hasMore: false })
      );
    const runtime = installRuntime({
      getMemory,
      listMemories
    } as Partial<IkarosRuntimeBridgeApi>);

    render(<MemorySettings workspaces={[]} preferredWorkspaceId={null} />);
    await screen.findByText("Added manually");
    fireEvent.click(screen.getByRole("button", { name: "Forget memory" }));
    const dialog = screen.getByRole("dialog", { name: "Forget this memory?" });

    expect(await within(dialog).findByText(/already been forgotten/i)).toBeTruthy();
    expect(
      (within(dialog).getByRole("button", { name: "Forget memory" }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
    expect(runtime.forgetMemory).not.toHaveBeenCalled();
    await waitFor(() => expect(listMemories).toHaveBeenCalledTimes(2));
  });

  it("shows forgotten tombstones without correction or restore actions", async () => {
    const tombstone: RuntimeMemorySummary = {
      ...summary,
      state: "forgotten",
      preview: null,
      revision: 4,
      forgottenAt: "2026-08-17T00:00:00.000Z"
    };
    installRuntime({
      listMemories: vi.fn(() =>
        ok({ memories: [tombstone], nextCursor: null, hasMore: false })
      ),
      getMemory: vi.fn(() =>
        ok({
          memory: {
            ...record,
            state: "forgotten",
            revision: 4,
            content: null,
            forgottenAt: tombstone.forgottenAt
          }
        })
      )
    } as Partial<IkarosRuntimeBridgeApi>);

    render(<MemorySettings workspaces={[]} preferredWorkspaceId={null} />);
    fireEvent.change(screen.getByLabelText("Filter by state"), {
      target: { value: "forgotten" }
    });

    expect(await screen.findByText("Memory content was forgotten.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Correct memory" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Forget memory" })).toBeNull();
    expect(screen.queryByRole("button", { name: /restore/i })).toBeNull();
    expect(screen.getByRole("button", { name: "View memory" })).toBeTruthy();
  });

  it("translates fixed UI labels without changing Memory content", async () => {
    setUiLanguage("zh-CN");
    installRuntime();

    render(<MemorySettings workspaces={[]} preferredWorkspaceId={null} />);

    expect(await screen.findByRole("heading", { level: 1, name: "记忆" })).toBeTruthy();
    expect(screen.getByText("Use concise technical explanations.")).toBeTruthy();
    expect(await screen.findByText("手动添加")).toBeTruthy();
  });
});
