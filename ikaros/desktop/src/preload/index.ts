import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";

import {
  DESKTOP_IPC_CHANNELS,
  type IkarosDesktopApi,
  type UiPreferences,
  type UiPreferencesPatch
} from "../shared/platform";
import type {
  RuntimeJournalEvent,
  RuntimeReplayResult,
  RuntimeThreadCreateResult,
  RuntimeThreadSummary,
  RuntimeTurnStartParams,
  RuntimeTurnStartResult
} from "../shared/runtime";

function subscribe<T>(channel: string, listener: (value: T) => void): () => void {
  const wrapped = (_event: IpcRendererEvent, value: T): void => listener(value);
  ipcRenderer.on(channel, wrapped);
  return () => ipcRenderer.removeListener(channel, wrapped);
}

const desktopApi: IkarosDesktopApi = Object.freeze({
  runtime: Object.freeze({
    listThreads: () =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.threadList) as Promise<{
        threads: RuntimeThreadSummary[];
      }>,
    createThread: (title: string | null) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.threadCreate,
        title
      ) as Promise<RuntimeThreadCreateResult>,
    startTurn: (params: RuntimeTurnStartParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.turnStart,
        params
      ) as Promise<RuntimeTurnStartResult>,
    replayEvents: (afterSeq: number, limit?: number) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.eventReplay,
        afterSeq,
        limit
      ) as Promise<RuntimeReplayResult>,
    onEvent: (listener: (event: RuntimeJournalEvent) => void) =>
      subscribe(DESKTOP_IPC_CHANNELS.runtime.event, listener)
  }),
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
