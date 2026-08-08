import { BrowserWindow, ipcMain } from "electron";

import { DESKTOP_IPC_CHANNELS, type UiPreferences } from "../shared/platform";
import { getUiPreferences, updateUiPreferences } from "./preferences";
import type { RendererTrustPolicy } from "./security";
import { updateWindowChrome } from "./window";

type RemoveIpcHandlers = () => void;

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
    DESKTOP_IPC_CHANNELS.preferences.update
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

  return () => {
    for (const channel of handledChannels) {
      ipcMain.removeHandler(channel);
    }
  };
}
