import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";

import {
  DESKTOP_IPC_CHANNELS,
  type IkarosDesktopApi,
  type UiPreferences,
  type UiPreferencesPatch
} from "../shared/platform";
import type {
  RuntimeFileChangeGetParams,
  RuntimeFileChangeResult,
  RuntimeFilePreviewParams,
  RuntimeFilePreviewResult,
  RuntimeInvocationResult,
  RuntimeJournalEvent,
  RuntimeHostStatus,
  RuntimeMemoryCorrectParams,
  RuntimeMemoryCreateParams,
  RuntimeMemoryCreateResult,
  RuntimeMemoryForgetParams,
  RuntimeMemoryGetResult,
  RuntimeMemoryListPage,
  RuntimeMemoryListParams,
  RuntimeMemoryMutationResult,
  RuntimeModelSetEnabledParams,
  RuntimeModelSetLimitsParams,
  RuntimeModelSetEnabledResult,
  RuntimeModelSummary,
  RuntimeProviderConfigureParams,
  RuntimeProviderConfigureResult,
  RuntimeProviderDiscoverModelsParams,
  RuntimeProviderDiscoverModelsResult,
  RuntimeProviderRemoveResult,
  RuntimeProviderSummary,
  RuntimeProcessReadParams,
  RuntimeProcessReadResult,
  RuntimeProcessStopParams,
  RuntimeProcessStopResult,
  RuntimeCancelRunResult,
  RuntimeSteerRunParams,
  RuntimeSteerRunResult,
  RuntimeReplayResult,
  RuntimeSkillListResult,
  RuntimeSkillSetEnabledParams,
  RuntimeSkillSetEnabledResult,
  RuntimeThreadCreateParams,
  RuntimeThreadCreateResult,
  RuntimeThreadCatalogParams,
  RuntimeThreadGetResult,
  RuntimeThreadListPage,
  RuntimeThreadMutationResult,
  RuntimeThreadRenameParams,
  RuntimeTurnListPage,
  RuntimeTurnListParams,
  RuntimeTurnStartParams,
  RuntimeTurnStartResult,
  RuntimeUsageReadResult
} from "../shared/runtime";

function subscribe<T>(channel: string, listener: (value: T) => void): () => void {
  const wrapped = (_event: IpcRendererEvent, value: T): void => listener(value);
  ipcRenderer.on(channel, wrapped);
  return () => ipcRenderer.removeListener(channel, wrapped);
}

const desktopApi: IkarosDesktopApi = Object.freeze({
  runtime: Object.freeze({
    listThreads: (params: RuntimeThreadCatalogParams = {}) =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.threadList, params) as Promise<
        RuntimeInvocationResult<RuntimeThreadListPage>
      >,
    getThread: (threadId: string) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.threadGet,
        threadId
      ) as Promise<RuntimeInvocationResult<RuntimeThreadGetResult>>,
    listTurns: (params: RuntimeTurnListParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.turnList,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeTurnListPage>>,
    createThread: (params: RuntimeThreadCreateParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.threadCreate,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeThreadCreateResult>>,
    renameThread: (params: RuntimeThreadRenameParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.threadRename,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeThreadMutationResult>>,
    archiveThread: (threadId: string) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.threadArchive,
        threadId
      ) as Promise<RuntimeInvocationResult<RuntimeThreadMutationResult>>,
    unarchiveThread: (threadId: string) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.threadUnarchive,
        threadId
      ) as Promise<RuntimeInvocationResult<RuntimeThreadMutationResult>>,
    startTurn: (params: RuntimeTurnStartParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.turnStart,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeTurnStartResult>>,
    cancelRun: (runId: string) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.runCancel,
        runId
      ) as Promise<RuntimeInvocationResult<RuntimeCancelRunResult>>,
    readProcess: (params: RuntimeProcessReadParams) =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.processRead, params) as Promise<RuntimeInvocationResult<RuntimeProcessReadResult>>,
    stopProcess: (params: RuntimeProcessStopParams) =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.processStop, params) as Promise<RuntimeInvocationResult<RuntimeProcessStopResult>>,
    steerRun: (params: RuntimeSteerRunParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.runSteer,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeSteerRunResult>>,
    replayEvents: (afterSeq: number, limit?: number) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.eventReplay,
        afterSeq,
        limit
      ) as Promise<RuntimeInvocationResult<RuntimeReplayResult>>,
    listProviders: () =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.providerList) as Promise<
        RuntimeInvocationResult<{ providers: RuntimeProviderSummary[] }>
      >,
    configureProvider: (params: RuntimeProviderConfigureParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.providerConfigure,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeProviderConfigureResult>>,
    discoverProviderModels: (params: RuntimeProviderDiscoverModelsParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.providerDiscoverModels,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeProviderDiscoverModelsResult>>,
    disconnectProvider: (providerId: "deepseek") =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.providerDisconnect,
        providerId
      ) as Promise<RuntimeInvocationResult<RuntimeProviderConfigureResult>>,
    removeProvider: (providerId: string) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.providerRemove,
        providerId
      ) as Promise<RuntimeInvocationResult<RuntimeProviderRemoveResult>>,
    listModels: () =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.modelList) as Promise<
        RuntimeInvocationResult<{ models: RuntimeModelSummary[] }>
      >,
    setModelEnabled: (params: RuntimeModelSetEnabledParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.modelSetEnabled,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeModelSetEnabledResult>>,
    setModelLimits: (params: RuntimeModelSetLimitsParams) =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.modelSetLimits, params) as Promise<RuntimeInvocationResult<RuntimeModelSetEnabledResult>>,
    listSkills: () =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.skillList) as Promise<
        RuntimeInvocationResult<RuntimeSkillListResult>
      >,
    setSkillEnabled: (params: RuntimeSkillSetEnabledParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.skillSetEnabled,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeSkillSetEnabledResult>>,
    createMemory: (params: RuntimeMemoryCreateParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.memoryCreate,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeMemoryCreateResult>>,
    correctMemory: (params: RuntimeMemoryCorrectParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.memoryCorrect,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeMemoryMutationResult>>,
    forgetMemory: (params: RuntimeMemoryForgetParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.memoryForget,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeMemoryMutationResult>>,
    listMemories: (params: RuntimeMemoryListParams = {}) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.memoryList,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeMemoryListPage>>,
    getMemory: (memoryId: string) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.memoryGet,
        memoryId
      ) as Promise<RuntimeInvocationResult<RuntimeMemoryGetResult>>,
    readUsage: () =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.runtime.usageRead) as Promise<
        RuntimeInvocationResult<RuntimeUsageReadResult>
      >,
    previewFile: (params: RuntimeFilePreviewParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.filePreview,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeFilePreviewResult>>,
    getFileChange: (params: RuntimeFileChangeGetParams) =>
      ipcRenderer.invoke(
        DESKTOP_IPC_CHANNELS.runtime.fileChangeGet,
        params
      ) as Promise<RuntimeInvocationResult<RuntimeFileChangeResult>>,
    onEvent: (listener: (event: RuntimeJournalEvent) => void) =>
      subscribe(DESKTOP_IPC_CHANNELS.runtime.event, listener),
    onStatus: (listener: (status: RuntimeHostStatus) => void) =>
      subscribe(DESKTOP_IPC_CHANNELS.runtime.status, listener)
  }),
  workspace: Object.freeze({
    chooseDirectory: () =>
      ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.workspace.chooseDirectory) as Promise<
        import("../shared/runtime").RuntimeWorkspaceSummary | null
      >
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
