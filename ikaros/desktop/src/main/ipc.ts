import { BrowserWindow, ipcMain, type IpcMainInvokeEvent } from "electron";

import { DESKTOP_IPC_CHANNELS, type UiPreferences } from "../shared/platform";
import { getUiPreferences, updateUiPreferences } from "./preferences";
import type { RendererTrustPolicy } from "./security";
import { updateWindowChrome } from "./window";

type RemoveIpcHandlers = () => void;

function trustedRequestingWindow(
  event: IpcMainInvokeEvent,
  trustPolicy: RendererTrustPolicy
): BrowserWindow {
  trustPolicy.assertTrustedIpc(event);
  const window = BrowserWindow.fromWebContents(event.sender);
  if (!window) {
    throw new Error("The requesting renderer is not attached to a desktop window.");
  }
  return window;
}

function broadcastPreferences(preferences: UiPreferences): void {
  for (const window of BrowserWindow.getAllWindows()) {
    if (!window.isDestroyed()) {
      updateWindowChrome(window, preferences);
      window.webContents.send(DESKTOP_IPC_CHANNELS.preferences.changed, preferences);
    }
  }
}

export function registerDesktopIpc(trustPolicy: RendererTrustPolicy): RemoveIpcHandlers {
  const handledChannels = [
    DESKTOP_IPC_CHANNELS.preferences.get,
    DESKTOP_IPC_CHANNELS.preferences.update,
    DESKTOP_IPC_CHANNELS.window.close,
    DESKTOP_IPC_CHANNELS.window.minimize,
    DESKTOP_IPC_CHANNELS.window.toggleMaximize
  ];

  ipcMain.handle(DESKTOP_IPC_CHANNELS.preferences.get, async (event) => {
    trustPolicy.assertTrustedIpc(event);
    return getUiPreferences();
  });

  ipcMain.handle(DESKTOP_IPC_CHANNELS.preferences.update, async (event, patch: unknown) => {
    trustPolicy.assertTrustedIpc(event);
    const preferences = await updateUiPreferences(patch);
    broadcastPreferences(preferences);
    return preferences;
  });

  ipcMain.handle(DESKTOP_IPC_CHANNELS.window.minimize, (event) => {
    const window = trustedRequestingWindow(event, trustPolicy);
    window.minimize();
  });

  ipcMain.handle(DESKTOP_IPC_CHANNELS.window.toggleMaximize, (event) => {
    const window = trustedRequestingWindow(event, trustPolicy);
    if (window.isMaximized()) {
      window.unmaximize();
    } else {
      window.maximize();
    }
  });

  ipcMain.handle(DESKTOP_IPC_CHANNELS.window.close, (event) => {
    const window = trustedRequestingWindow(event, trustPolicy);
    window.close();
  });

  return () => {
    for (const channel of handledChannels) {
      ipcMain.removeHandler(channel);
    }
  };
}
