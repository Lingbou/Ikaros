import { beforeEach, describe, expect, it, vi } from "vitest";

import { RuntimeRpcError, type RuntimeNotification } from "./runtimeHost";

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

  it.each([
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
    ["ikaros:runtime:provider-disconnect", ["deepseek"], "provider.disconnect", {
      providerId: "deepseek"
    }],
    ["ikaros:runtime:provider-remove", ["local"], "provider.remove", {
      providerId: "local"
    }],
    ["ikaros:runtime:model-list", [], "model.list", undefined],
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
    const result = await handler?.(event, "Exactly once", "thread-request-123");

    expect(trustPolicy.assertTrustedIpc).toHaveBeenCalledWith(event);
    expect(electron.runtimeHost.request).toHaveBeenCalledWith("thread.create", {
      title: "Exactly once",
      clientRequestId: "thread-request-123",
    });
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
    await expect(handler?.(event, "Rejected", "request-rejected")).resolves.toEqual({
      ok: false,
      error: {
        kind: "json_rpc",
        code: -32602,
        message: "thread.create rejected",
      },
    });

    const transportError = new Error("Runtime WebSocket closed.");
    electron.runtimeHost.request.mockRejectedValueOnce(transportError);
    await expect(handler?.(event, "Ambiguous", "request-ambiguous")).rejects.toBe(
      transportError
    );
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
});
