import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";

import {
  DESKTOP_IPC_CHANNELS,
  type IkarosDesktopApi,
  type UiPreferences,
  type UiPreferencesPatch
} from "../shared/platform";

function subscribe<T>(channel: string, listener: (value: T) => void): () => void {
  const wrapped = (_event: IpcRendererEvent, value: T): void => listener(value);
  ipcRenderer.on(channel, wrapped);
  return () => ipcRenderer.removeListener(channel, wrapped);
}

const desktopApi: IkarosDesktopApi = Object.freeze({
  preferences: Object.freeze({
    get: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.preferences.get) as Promise<UiPreferences>,
    update: (patch: UiPreferencesPatch) =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.preferences.update, patch) as Promise<UiPreferences>,
    onChanged: (listener: (preferences: UiPreferences) => void) =>
      subscribe(DESKTOP_IPC_CHANNELS.preferences.changed, listener)
  }),
  windowControls: Object.freeze({
    usesCustomTitleBar: process.platform === "linux",
    close: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.window.close) as Promise<void>,
    minimize: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.window.minimize) as Promise<void>,
    toggleMaximize: () =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.window.toggleMaximize) as Promise<void>
  })
});

contextBridge.exposeInMainWorld("ikarosDesktop", desktopApi);
