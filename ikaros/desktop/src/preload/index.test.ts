import { beforeEach, describe, expect, it, vi } from "vitest";

import type { IkarosDesktopApi } from "../shared/platform";

const electron = vi.hoisted(() => {
  let exposedApi: IkarosDesktopApi | undefined;
  return {
    contextBridge: {
      exposeInMainWorld: vi.fn((_key: string, api: IkarosDesktopApi) => {
        exposedApi = api;
      })
    },
    ipcRenderer: {
      invoke: vi.fn(),
      on: vi.fn(),
      removeListener: vi.fn()
    },
    exposedApi: () => exposedApi,
    reset: () => {
      exposedApi = undefined;
    }
  };
});

vi.mock("electron", () => ({
  contextBridge: electron.contextBridge,
  ipcRenderer: electron.ipcRenderer
}));

describe("preload Runtime bridge", () => {
  beforeEach(() => {
    vi.resetModules();
    electron.reset();
  });

  it("exposes aggregate token usage through its narrow IPC channel", async () => {
    const expected = {
      ok: true as const,
      value: {
        summary: {
          lifetimeTokens: 1_250,
          peakDailyTokens: 750,
          longestRunningTurnSec: 90,
          currentStreakDays: 2,
          longestStreakDays: 4
        },
        dailyUsageBuckets: [{ startDate: "2026-08-15", tokens: 750 }]
      }
    };
    electron.ipcRenderer.invoke.mockResolvedValueOnce(expected);
    await import("./index");

    await expect(electron.exposedApi()?.runtime.readUsage()).resolves.toEqual(expected);
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith("ikaros:runtime:usage-read");
  });

  it("exposes DeepSeek model discovery through its narrow IPC channel", async () => {
    const expected = {
      ok: true as const,
      value: {
        models: [{ id: "deepseek-v4-flash", displayName: "DeepSeek V4 Flash" }]
      }
    };
    electron.ipcRenderer.invoke.mockResolvedValueOnce(expected);
    await import("./index");

    const api = electron.exposedApi();
    expect(api).toBeDefined();
    const params = { kind: "deepseek" as const, apiKey: "write-only-discovery-secret" };
    await expect(api?.runtime.discoverProviderModels(params)).resolves.toEqual(expected);
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:runtime:provider-discover-models",
      params
    );
  });

  it("forwards the complete Thread workspace snapshot through one IPC argument", async () => {
    const expected = { ok: true as const, value: { thread: { id: "thread-1" } } };
    electron.ipcRenderer.invoke.mockResolvedValueOnce(expected);
    await import("./index");

    const params = {
      title: "Workspace chat",
      workspace: {
        id: "workspace-ikaros",
        name: "Ikaros",
        rootUri: "C:\\Workspace\\github\\Ikaros",
      },
      clientRequestId: "thread-request-1",
    };
    await expect(electron.exposedApi()?.runtime.createThread(params)).resolves.toEqual(expected);
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:runtime:thread-create",
      params,
    );
  });

  it("exposes archived catalog and Thread lifecycle mutations through narrow channels", async () => {
    const expected = { ok: true as const, value: { changed: true } };
    electron.ipcRenderer.invoke.mockResolvedValue(expected);
    await import("./index");

    const runtime = electron.exposedApi()?.runtime;
    await runtime?.listThreads({ archived: true });
    await runtime?.renameThread({ threadId: "thread-1", title: "Renamed" });
    await runtime?.archiveThread("thread-1");
    await runtime?.unarchiveThread("thread-1");

    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:runtime:thread-list",
      { archived: true },
    );
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:runtime:thread-rename",
      { threadId: "thread-1", title: "Renamed" },
    );
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:runtime:thread-archive",
      "thread-1",
    );
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:runtime:thread-unarchive",
      "thread-1",
    );
  });

  it("exposes Thread metadata and paginated Turn history through narrow channels", async () => {
    const metadata = {
      ok: true as const,
      value: { thread: { id: "thread-1" }, snapshotSeq: 10 },
    };
    const history = {
      ok: true as const,
      value: { turns: [], nextCursor: null, hasMore: false, snapshotSeq: 10 },
    };
    electron.ipcRenderer.invoke
      .mockResolvedValueOnce(metadata)
      .mockResolvedValueOnce(history);
    await import("./index");

    const runtime = electron.exposedApi()?.runtime;
    const params = { threadId: "thread-1", branchId: "branch-1", limit: 50 };
    await expect(runtime?.getThread("thread-1")).resolves.toEqual(metadata);
    await expect(runtime?.listTurns(params)).resolves.toEqual(history);
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:runtime:thread-get",
      "thread-1",
    );
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:runtime:turn-list",
      params,
    );
  });

  it("exposes the native workspace directory picker through one narrow channel", async () => {
    const workspace = {
      id: "workspace-ikaros",
      name: "Ikaros",
      rootUri: "C:\\Workspace\\github\\Ikaros",
    };
    electron.ipcRenderer.invoke.mockResolvedValueOnce(workspace);
    await import("./index");

    await expect(electron.exposedApi()?.workspace.chooseDirectory()).resolves.toEqual(workspace);
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(
      "ikaros:workspace:choose-directory",
    );
  });
});
