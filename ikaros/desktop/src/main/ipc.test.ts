import { beforeEach, describe, expect, it, vi } from "vitest";

const electron = vi.hoisted(() => {
  const handlers = new Map<string, (event: unknown, ...args: unknown[]) => unknown>();
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
      getAllWindows: vi.fn(() => []),
    },
    targetWindow,
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
  });

  it("lets a trusted renderer minimize its own window", async () => {
    const trustPolicy = {
      assertTrustedIpc: vi.fn(),
      isTrustedUrl: vi.fn(() => true),
    };
    registerDesktopIpc(trustPolicy);
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
    registerDesktopIpc(trustPolicy);
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
    registerDesktopIpc(trustPolicy);
    const event = { sender: {} };

    const handler = electron.handlers.get("ikaros:window:close");
    expect(handler).toBeDefined();
    await handler?.(event);

    expect(electron.targetWindow.close).toHaveBeenCalledOnce();
  });
});
