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
