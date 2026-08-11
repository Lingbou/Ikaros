import { BrowserWindow, ipcMain, type IpcMainInvokeEvent } from "electron";

import { DESKTOP_IPC_CHANNELS, type UiPreferences } from "../shared/platform";
import type { RuntimeJournalEvent, RuntimeTurnStartParams } from "../shared/runtime";
import { getUiPreferences, updateUiPreferences } from "./preferences";
import type { RendererTrustPolicy } from "./security";
import type { RuntimeHost } from "./runtimeHost";
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

export function registerDesktopIpc(
  trustPolicy: RendererTrustPolicy,
  runtimeHost: Pick<RuntimeHost, "request" | "onNotification">
): RemoveIpcHandlers {
  const handledChannels = [
    DESKTOP_IPC_CHANNELS.runtime.threadCreate,
    DESKTOP_IPC_CHANNELS.runtime.threadList,
    DESKTOP_IPC_CHANNELS.runtime.turnStart,
    DESKTOP_IPC_CHANNELS.runtime.eventReplay,
    DESKTOP_IPC_CHANNELS.preferences.get,
    DESKTOP_IPC_CHANNELS.preferences.update,
    DESKTOP_IPC_CHANNELS.window.close,
    DESKTOP_IPC_CHANNELS.window.minimize,
    DESKTOP_IPC_CHANNELS.window.toggleMaximize
  ];

  const removeRuntimeNotification = runtimeHost.onNotification((notification) => {
    if (notification.method !== "event") {
      return;
    }
    const event = notification.params as RuntimeJournalEvent;
    for (const window of BrowserWindow.getAllWindows()) {
      if (!window.isDestroyed()) {
        window.webContents.send(DESKTOP_IPC_CHANNELS.runtime.event, event);
      }
    }
  });

  ipcMain.handle(DESKTOP_IPC_CHANNELS.runtime.threadList, async (event) => {
    trustPolicy.assertTrustedIpc(event);
    return runtimeHost.request("thread.list");
  });

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.threadCreate,
    async (event, title: unknown) => {
      trustPolicy.assertTrustedIpc(event);
      return runtimeHost.request("thread.create", { title });
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.turnStart,
    async (event, params: RuntimeTurnStartParams) => {
      trustPolicy.assertTrustedIpc(event);
      return runtimeHost.request("turn.start", { ...params });
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.eventReplay,
    async (event, afterSeq: unknown, limit: unknown) => {
      trustPolicy.assertTrustedIpc(event);
      return runtimeHost.request("event.replay", {
        afterSeq,
        ...(limit === undefined ? {} : { limit })
      });
    }
  );

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
    removeRuntimeNotification();
    for (const channel of handledChannels) {
      ipcMain.removeHandler(channel);
    }
  };
}
