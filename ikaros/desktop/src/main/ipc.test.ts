import { tmpdir } from "node:os";
import { join } from "node:path";

import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  RUNTIME_HOST_STATUS_NOTIFICATION,
  RuntimeRpcError,
  type RuntimeNotification
} from "./runtimeHost";

const electron = vi.hoisted(() => {
  const handlers = new Map<string, (event: unknown, ...args: unknown[]) => unknown>();
  const windows: unknown[] = [];
  const runtimeNotification: { listener?: (notification: RuntimeNotification) => void } = {};
  const targetWindow = {
    close: vi.fn(),
    isMaximized: vi.fn(() => false),
    maximize: vi.fn(),
    minimize: vi.fn(),
    unmaximize: vi.fn(),
  };
  return {
    handlers,
    ipcMain: {
      handle: vi.fn((channel: string, handler: (event: unknown, ...args: unknown[]) => unknown) => {
        handlers.set(channel, handler);
      }),
      removeHandler: vi.fn((channel: string) => handlers.delete(channel)),
    },
    BrowserWindow: {
      fromWebContents: vi.fn(() => targetWindow),
      getAllWindows: vi.fn(() => windows),
    },
    dialog: {
      showOpenDialog: vi.fn(),
    },
    windows,
    targetWindow,
    runtimeNotification,
    runtimeHost: {
      request: vi.fn(),
      onNotification: vi.fn((listener: (notification: RuntimeNotification) => void) => {
        runtimeNotification.listener = listener;
        return vi.fn();
      }),
    },
  };
});

vi.mock("electron", () => ({
  BrowserWindow: electron.BrowserWindow,
  dialog: electron.dialog,
  ipcMain: electron.ipcMain,
}));

vi.mock("./preferences", () => ({
  getUiPreferences: vi.fn(),
  updateUiPreferences: vi.fn(),
}));

vi.mock("./window", () => ({
  updateWindowChrome: vi.fn(),
}));

import { registerDesktopIpc } from "./ipc";

describe("desktop window controls", () => {
  beforeEach(() => {
    electron.handlers.clear();
    electron.windows.length = 0;
    electron.runtimeNotification.listener = undefined;
    electron.runtimeHost.request.mockReset();
    electron.dialog.showOpenDialog.mockReset();
  });

  it("routes file preview and change queries with their exact binding and pagination", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    const removeHandlers = registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };
    const previewParams = {
      threadId: "thread-1",
      path: "report.md",
      sourceToolCallItemId: "call-1",
      offset: 2001,
      expectedRevision: "a".repeat(64),
    };
    const preview = {
      threadId: "thread-1",
      path: "/workspace/report.md",
      status: "unavailable",
      reason: "revision_changed",
    };
    electron.runtimeHost.request.mockResolvedValueOnce(preview);
    await expect(electron.handlers.get("ikaros:runtime:file-preview")?.(event, previewParams))
      .resolves.toEqual({ ok: true, value: preview });
    expect(trustPolicy.assertTrustedIpc).toHaveBeenCalledWith(event);
    expect(electron.runtimeHost.request).toHaveBeenCalledWith("file.preview", previewParams);

    const changeParams = { threadId: "thread-1", toolCallItemId: "call-1" };
    const change = {
      threadId: "thread-1",
      toolCallItemId: "call-1",
      path: null,
      operation: "write",
      recordedAt: null,
      before: null,
      after: null,
      status: "unavailable",
      reason: "not_recorded",
    };
    electron.runtimeHost.request.mockResolvedValueOnce(change);
    await expect(electron.handlers.get("ikaros:runtime:file-change-get")?.(event, changeParams))
      .resolves.toEqual({ ok: true, value: change });
    expect(electron.runtimeHost.request).toHaveBeenCalledWith("file.change.get", changeParams);
    removeHandlers();
    expect(electron.handlers.has("ikaros:runtime:file-preview")).toBe(false);
    expect(electron.handlers.has("ikaros:runtime:file-change-get")).toBe(false);
  });

  it.each(["ikaros:runtime:file-preview", "ikaros:runtime:file-change-get"])(
    "rejects untrusted file queries before reaching Runtime on %s",
    async (channel) => {
      const trustPolicy = {
        assertTrustedIpc: vi.fn(() => { throw new Error("untrusted renderer"); }),
        isTrustedUrl: vi.fn(() => false),
      };
      registerDesktopIpc(trustPolicy, electron.runtimeHost);
      await expect(electron.handlers.get(channel)?.({ sender: {} }, { threadId: "thread-1" }))
        .rejects.toThrow("untrusted renderer");
      expect(electron.runtimeHost.request).not.toHaveBeenCalled();
    },
  );

  it("preserves file binding RPC rejection as a structured error", async () => {
    const trustPolicy = { assertTrustedIpc: vi.fn(), isTrustedUrl: vi.fn(() => true) };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    electron.runtimeHost.request.mockRejectedValueOnce(new RuntimeRpcError(-32602, "invalid file binding"));
    await expect(electron.handlers.get("ikaros:runtime:file-change-get")?.(
      { sender: {} }, { threadId: "thread-1", toolCallItemId: "foreign-call" },
    )).resolves.toEqual({
      ok: false,
      error: { kind: "json_rpc", code: -32602, message: "invalid file binding" },
    });
  });

  it("lets a trusted renderer minimize its own window", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };

    const handler = electron.handlers.get("ikaros:window:minimize");
    expect(handler).toBeDefined();
    await handler?.(event);

    expect(trustPolicy.assertTrustedIpc).toHaveBeenCalledWith(event);
    expect(electron.BrowserWindow.fromWebContents).toHaveBeenCalledWith(event.sender);
    expect(electron.targetWindow.minimize).toHaveBeenCalledOnce();
  });

  it("lets a trusted renderer toggle maximize on its own window", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };
    electron.targetWindow.isMaximized.mockReturnValueOnce(false).mockReturnValueOnce(true);

    const handler = electron.handlers.get("ikaros:window:toggle-maximize");
    expect(handler).toBeDefined();
    await handler?.(event);
    await handler?.(event);

    expect(electron.targetWindow.maximize).toHaveBeenCalledOnce();
    expect(electron.targetWindow.unmaximize).toHaveBeenCalledOnce();
  });

  it("lets a trusted renderer close its own window", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };

    const handler = electron.handlers.get("ikaros:window:close");
    expect(handler).toBeDefined();
    await handler?.(event);

    expect(electron.targetWindow.close).toHaveBeenCalledOnce();
  });

  it("returns one stable workspace snapshot for a selected directory", async () => {
    const directory = join(tmpdir(), "Workspace", "github", "Ikaros");
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    electron.dialog.showOpenDialog.mockResolvedValue({
      canceled: false,
      filePaths: [directory],
    });
    const event = { sender: {} };
    const handler = electron.handlers.get("ikaros:workspace:choose-directory");

    const first = await handler?.(event);
    const second = await handler?.(event);

    expect(first).toEqual(second);
    expect(first).toMatchObject({
      id: expect.stringMatching(/^workspace-[0-9a-f]{24}$/),
      name: "Ikaros",
      rootUri: directory,
    });
    expect(electron.dialog.showOpenDialog).toHaveBeenCalledWith(electron.targetWindow, {
      properties: ["openDirectory"],
    });
  });

  it("leaves the workspace unchanged when directory selection is cancelled", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    electron.dialog.showOpenDialog.mockResolvedValue({ canceled: true, filePaths: [] });
    const handler = electron.handlers.get("ikaros:workspace:choose-directory");

    await expect(handler?.({ sender: {} })).resolves.toBeNull();
  });

  it("forwards a trusted Run cancellation to the Runtime", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };

    const handler = electron.handlers.get("ikaros:runtime:run-cancel");
    expect(handler).toBeDefined();
    await handler?.(event, "run-123");

    expect(trustPolicy.assertTrustedIpc).toHaveBeenCalledWith(event);
    expect(electron.runtimeHost.request).toHaveBeenCalledWith("run.cancel", {
      runId: "run-123",
    });
  });

  it("forwards Skill catalog and enablement calls without reshaping them", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    const catalog = { skills: [], diagnostics: [] };
    const toggled = {
      skill: {
        name: "demo",
        description: "Demo Skill",
        location: "C:/Users/demo/.ikaros/skills/demo/SKILL.md",
        enabled: false,
      },
    };
    electron.runtimeHost.request
      .mockResolvedValueOnce(catalog)
      .mockResolvedValueOnce(toggled);
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };

    await expect(
      electron.handlers.get("ikaros:runtime:skill-list")?.(event)
    ).resolves.toEqual({ ok: true, value: catalog });
    await expect(
      electron.handlers
        .get("ikaros:runtime:skill-set-enabled")
        ?.(event, { name: "demo", enabled: false })
    ).resolves.toEqual({ ok: true, value: toggled });
    expect(electron.runtimeHost.request).toHaveBeenNthCalledWith(1, "skill.list");
    expect(electron.runtimeHost.request).toHaveBeenNthCalledWith(
      2,
      "skill.set_enabled",
      { name: "demo", enabled: false }
    );
  });

  it("forwards exactly one typed Runtime Thread catalog page", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    const first = {
      id: "thread-1",
      title: "First",
      defaultBranchId: "branch-1",
      workspace: null,
      createdAt: "2026-08-14T00:00:00.000Z",
      updatedAt: "2026-08-14T00:00:00.000Z",
      archivedAt: null,
    };
    const page = {
      threads: [first],
      nextCursor: "cursor_one",
      hasMore: true,
      snapshotSeq: 1,
    };
    electron.runtimeHost.request.mockResolvedValueOnce(page);
    registerDesktopIpc(trustPolicy, electron.runtimeHost);

    const handler = electron.handlers.get("ikaros:runtime:thread-list");
    const params = { cursor: "cursor_zero", limit: 25, archived: true };
    const result = await handler?.({ sender: {} }, params);

    expect(result).toEqual({
      ok: true,
      value: page,
    });
    expect(electron.runtimeHost.request).toHaveBeenCalledOnce();
    expect(electron.runtimeHost.request).toHaveBeenCalledWith("thread.list", params);
  });

  it("forwards the archived Thread catalog filter", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    electron.runtimeHost.request.mockResolvedValueOnce({
      threads: [],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 7,
    });
    registerDesktopIpc(trustPolicy, electron.runtimeHost);

    const handler = electron.handlers.get("ikaros:runtime:thread-list");
    await handler?.({ sender: {} }, { archived: true });

    expect(electron.runtimeHost.request).toHaveBeenCalledWith("thread.list", {
      archived: true,
    });
  });

  it.each([
    ["ikaros:runtime:thread-get", ["thread-1"], "thread.get", { threadId: "thread-1" }],
    [
      "ikaros:runtime:thread-rename",
      [{ threadId: "thread-1", title: "Renamed" }],
      "thread.rename",
      { threadId: "thread-1", title: "Renamed" },
    ],
    ["ikaros:runtime:thread-archive", ["thread-1"], "thread.archive", { threadId: "thread-1" }],
    [
      "ikaros:runtime:thread-unarchive",
      ["thread-1"],
      "thread.unarchive",
      { threadId: "thread-1" },
    ],
    [
      "ikaros:runtime:turn-list",
      [{ threadId: "thread-1", branchId: "branch-1", cursor: "older", limit: 25 }],
      "turn.list",
      { threadId: "thread-1", branchId: "branch-1", cursor: "older", limit: 25 },
    ],
    ["ikaros:runtime:provider-list", [], "provider.list", undefined],
    [
      "ikaros:runtime:provider-configure",
      [
        {
          kind: "deepseek",
          apiKey: "write-only-secret",
          models: [{ id: "deepseek-chat", displayName: "DeepSeek Chat" }]
        }
      ],
      "provider.configure",
      {
        kind: "deepseek",
        apiKey: "write-only-secret",
        models: [{ id: "deepseek-chat", displayName: "DeepSeek Chat" }]
      }
    ],
    [
      "ikaros:runtime:provider-discover-models",
      [{ kind: "deepseek", apiKey: "write-only-discovery-secret" }],
      "provider.discover_models",
      { kind: "deepseek", apiKey: "write-only-discovery-secret" }
    ],
    ["ikaros:runtime:provider-disconnect", ["deepseek"], "provider.disconnect", {
      providerId: "deepseek"
    }],
    ["ikaros:runtime:provider-remove", ["local"], "provider.remove", {
      providerId: "local"
    }],
    ["ikaros:runtime:model-list", [], "model.list", undefined],
    [
      "ikaros:runtime:memory-create",
      [
        {
          kind: "preference",
          scope: { type: "global", key: null },
          content: "Concise answers",
          clientRequestId: "memory-create-1",
          source: { type: "session_item", itemId: `item_${"2".repeat(32)}` }
        }
      ],
      "memory.create",
      {
        kind: "preference",
        scope: { type: "global", key: null },
        content: "Concise answers",
        clientRequestId: "memory-create-1",
        source: { type: "session_item", itemId: `item_${"2".repeat(32)}` }
      }
    ],
    [
      "ikaros:runtime:memory-correct",
      [
        {
          memoryId: `memory_${"1".repeat(32)}`,
          expectedRevision: 1,
          content: "Prefer concise answers",
          clientRequestId: "memory-correct-1"
        }
      ],
      "memory.correct",
      {
        memoryId: `memory_${"1".repeat(32)}`,
        expectedRevision: 1,
        content: "Prefer concise answers",
        clientRequestId: "memory-correct-1"
      }
    ],
    [
      "ikaros:runtime:memory-forget",
      [
        {
          memoryId: `memory_${"1".repeat(32)}`,
          expectedRevision: 2,
          clientRequestId: "memory-forget-1"
        }
      ],
      "memory.forget",
      {
        memoryId: `memory_${"1".repeat(32)}`,
        expectedRevision: 2,
        clientRequestId: "memory-forget-1"
      }
    ],
    [
      "ikaros:runtime:memory-list",
      [{ scope: { type: "global", key: null }, limit: 25 }],
      "memory.list",
      { scope: { type: "global", key: null }, limit: 25 }
    ],
    [
      "ikaros:runtime:memory-get",
      [`memory_${"1".repeat(32)}`],
      "memory.get",
      { memoryId: `memory_${"1".repeat(32)}` }
    ],
    ["ikaros:runtime:usage-read", [], "usage.read", undefined],
    [
      "ikaros:runtime:model-set-enabled",
      [{ providerId: "local", modelId: "model", enabled: false }],
      "model.set_enabled",
      { providerId: "local", modelId: "model", enabled: false }
    ]
  ])("forwards %s through the narrow Runtime RPC bridge", async (channel, args, method, params) => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true)
    };
    electron.runtimeHost.request.mockResolvedValueOnce({});
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };

    const handler = electron.handlers.get(channel as string);
    expect(handler).toBeDefined();
    await handler?.(event, ...(args as unknown[]));

    expect(trustPolicy.assertTrustedIpc).toHaveBeenCalledWith(event);
    if (params === undefined) {
      expect(electron.runtimeHost.request).toHaveBeenLastCalledWith(method);
    } else {
      expect(electron.runtimeHost.request).toHaveBeenLastCalledWith(method, params);
    }
  });

  it("forwards the stable thread.create request ID to the Runtime", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };

    const created = { thread: { id: "thread-123" } };
    electron.runtimeHost.request.mockResolvedValueOnce(created);
    const handler = electron.handlers.get("ikaros:runtime:thread-create");
    expect(handler).toBeDefined();
    const params = {
      title: "Exactly once",
      workspace: {
        id: "workspace-ikaros",
        name: "Ikaros",
        rootUri: "C:\\Workspace\\github\\Ikaros",
      },
      clientRequestId: "thread-request-123",
    };
    const result = await handler?.(event, params);

    expect(trustPolicy.assertTrustedIpc).toHaveBeenCalledWith(event);
    expect(electron.runtimeHost.request).toHaveBeenCalledWith("thread.create", params);
    expect(result).toEqual({ ok: true, value: created });
  });

  it("serializes only definitive JSON-RPC errors across IPC", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = { sender: {} };
    const handler = electron.handlers.get("ikaros:runtime:thread-create");
    expect(handler).toBeDefined();

    electron.runtimeHost.request.mockRejectedValueOnce(
      new RuntimeRpcError(-32602, "thread.create rejected")
    );
    await expect(
      handler?.(event, {
        title: "Rejected",
        workspace: null,
        clientRequestId: "request-rejected",
      }),
    ).resolves.toEqual({
      ok: false,
      error: {
        kind: "json_rpc",
        code: -32602,
        message: "thread.create rejected",
      },
    });

    const transportError = new Error("Runtime WebSocket closed.");
    electron.runtimeHost.request.mockRejectedValueOnce(transportError);
    await expect(
      handler?.(event, {
        title: "Ambiguous",
        workspace: null,
        clientRequestId: "request-ambiguous",
      }),
    ).rejects.toBe(transportError);

    const memoryHandler = electron.handlers.get("ikaros:runtime:memory-correct");
    expect(memoryHandler).toBeDefined();
    const memoryError = new RuntimeRpcError(
      -32020,
      "memory operation failed",
      "memory_revision_conflict"
    ) as RuntimeRpcError & { data?: unknown };
    memoryError.data = { content: "must not cross IPC" };
    electron.runtimeHost.request.mockRejectedValueOnce(memoryError);
    await expect(
      memoryHandler?.(event, {
        memoryId: `memory_${"1".repeat(32)}`,
        expectedRevision: 1,
        content: "Updated",
        clientRequestId: "memory-correct-conflict"
      })
    ).resolves.toEqual({
      ok: false,
      error: {
        kind: "json_rpc",
        code: -32020,
        message: "memory operation failed",
        reasonCode: "memory_revision_conflict"
      }
    });
  });

  it("survives a webContents destruction race and continues broadcasting events", () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    const destroyedContents = {
      isDestroyed: vi.fn().mockReturnValueOnce(false).mockReturnValue(true),
      send: vi.fn(() => {
        throw new Error("Object has been destroyed");
      }),
    };
    const liveContents = {
      isDestroyed: vi.fn(() => false),
      send: vi.fn(),
    };
    electron.windows.push(
      { isDestroyed: vi.fn(() => false), webContents: destroyedContents },
      { isDestroyed: vi.fn(() => false), webContents: liveContents },
    );
    registerDesktopIpc(trustPolicy, electron.runtimeHost);
    const event = {
      seq: 1,
      type: "thread.created",
      threadId: "thread-1",
      branchId: "branch-1",
      turnId: null,
      runId: null,
      itemId: null,
      timestamp: "2026-08-11T00:00:00Z",
      payload: {},
    };

    expect(() =>
      electron.runtimeNotification.listener?.({
        jsonrpc: "2.0",
        method: "event",
        params: event,
      }),
    ).not.toThrow();

    expect(destroyedContents.send).toHaveBeenCalledOnce();
    expect(liveContents.send).toHaveBeenCalledWith("ikaros:runtime:event", event);
  });

  it("broadcasts validated Runtime host lifecycle status", () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    const liveContents = {
      isDestroyed: vi.fn(() => false),
      send: vi.fn(),
    };
    electron.windows.push({ isDestroyed: vi.fn(() => false), webContents: liveContents });
    registerDesktopIpc(trustPolicy, electron.runtimeHost);

    electron.runtimeNotification.listener?.({
      jsonrpc: "2.0",
      method: RUNTIME_HOST_STATUS_NOTIFICATION,
      params: { state: "reconnecting", message: "socket closed" },
    });

    expect(liveContents.send).toHaveBeenCalledWith("ikaros:runtime:status", {
      state: "reconnecting",
      message: "socket closed",
    });
  });
});
