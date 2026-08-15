import { createHash } from "node:crypto";
import { basename, resolve } from "node:path";

import { BrowserWindow, dialog, ipcMain, type IpcMainInvokeEvent } from "electron";

import { DESKTOP_IPC_CHANNELS, type UiPreferences } from "../shared/platform";
import type {
  RuntimeCancelRunResult,
  RuntimeHostStatus,
  RuntimeInvocationResult,
  RuntimeJournalEvent,
  RuntimeModelSetEnabledParams,
  RuntimeModelSetEnabledResult,
  RuntimeModelSummary,
  RuntimeProviderConfigureParams,
  RuntimeProviderConfigureResult,
  RuntimeProviderDiscoverModelsParams,
  RuntimeProviderDiscoverModelsResult,
  RuntimeProviderRemoveResult,
  RuntimeProviderSummary,
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
  RuntimeUsageReadResult,
  RuntimeWorkspaceSummary
} from "../shared/runtime";
import { getUiPreferences, updateUiPreferences } from "./preferences";
import type { RendererTrustPolicy } from "./security";
import {
  RUNTIME_HOST_STATUS_NOTIFICATION,
  RuntimeRpcError,
  type RuntimeHost
} from "./runtimeHost";
import { updateWindowChrome } from "./window";

type RemoveIpcHandlers = () => void;

async function invokeRuntime<TResult>(
  operation: () => Promise<TResult>
): Promise<RuntimeInvocationResult<TResult>> {
  try {
    return { ok: true, value: await operation() };
  } catch (error) {
    if (error instanceof RuntimeRpcError) {
      return {
        ok: false,
        error: { kind: error.kind, code: error.code, message: error.message }
      };
    }
    throw error;
  }
}

function reportWindowDeliveryFailure(context: string, error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  try {
    process.stderr.write(`[ikaros-desktop] ${context}: ${message}\n`);
  } catch {
    // Window fan-out must continue even when the diagnostic stream is unavailable.
  }
}

function sendToLiveWindow(window: BrowserWindow, channel: string, payload: unknown): void {
  let webContents: BrowserWindow["webContents"] | undefined;
  try {
    if (window.isDestroyed()) {
      return;
    }
    webContents = window.webContents;
    if (webContents.isDestroyed()) {
      return;
    }
    webContents.send(channel, payload);
  } catch (error) {
    if (!window.isDestroyed() && webContents && !webContents.isDestroyed()) {
      reportWindowDeliveryFailure(`failed to send ${channel}`, error);
    }
  }
}

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
    try {
      if (window.isDestroyed()) {
        continue;
      }
      updateWindowChrome(window, preferences);
    } catch (error) {
      if (!window.isDestroyed()) {
        reportWindowDeliveryFailure("failed to update window chrome", error);
      }
    }
    sendToLiveWindow(window, DESKTOP_IPC_CHANNELS.preferences.changed, preferences);
  }
}

function workspaceFromDirectory(directory: string): RuntimeWorkspaceSummary {
  const rootUri = resolve(directory);
  const identity = process.platform === "win32" ? rootUri.toLocaleLowerCase("en-US") : rootUri;
  return {
    id: `workspace-${createHash("sha256").update(identity).digest("hex").slice(0, 24)}`,
    name: basename(rootUri) || rootUri,
    rootUri
  };
}

export function registerDesktopIpc(
  trustPolicy: RendererTrustPolicy,
  runtimeHost: Pick<RuntimeHost, "request" | "onNotification">
): RemoveIpcHandlers {
  const handledChannels = [
    DESKTOP_IPC_CHANNELS.runtime.threadCreate,
    DESKTOP_IPC_CHANNELS.runtime.threadRename,
    DESKTOP_IPC_CHANNELS.runtime.threadArchive,
    DESKTOP_IPC_CHANNELS.runtime.threadUnarchive,
    DESKTOP_IPC_CHANNELS.runtime.threadGet,
    DESKTOP_IPC_CHANNELS.runtime.threadList,
    DESKTOP_IPC_CHANNELS.runtime.turnList,
    DESKTOP_IPC_CHANNELS.runtime.turnStart,
    DESKTOP_IPC_CHANNELS.runtime.runCancel,
    DESKTOP_IPC_CHANNELS.runtime.eventReplay,
    DESKTOP_IPC_CHANNELS.runtime.providerList,
    DESKTOP_IPC_CHANNELS.runtime.providerConfigure,
    DESKTOP_IPC_CHANNELS.runtime.providerDiscoverModels,
    DESKTOP_IPC_CHANNELS.runtime.providerDisconnect,
    DESKTOP_IPC_CHANNELS.runtime.providerRemove,
    DESKTOP_IPC_CHANNELS.runtime.modelList,
    DESKTOP_IPC_CHANNELS.runtime.modelSetEnabled,
    DESKTOP_IPC_CHANNELS.runtime.skillList,
    DESKTOP_IPC_CHANNELS.runtime.skillSetEnabled,
    DESKTOP_IPC_CHANNELS.runtime.usageRead,
    DESKTOP_IPC_CHANNELS.workspace.chooseDirectory,
    DESKTOP_IPC_CHANNELS.preferences.get,
    DESKTOP_IPC_CHANNELS.preferences.update,
    DESKTOP_IPC_CHANNELS.window.close,
    DESKTOP_IPC_CHANNELS.window.minimize,
    DESKTOP_IPC_CHANNELS.window.toggleMaximize
  ];

  const removeRuntimeNotification = runtimeHost.onNotification((notification) => {
    if (notification.method === RUNTIME_HOST_STATUS_NOTIFICATION) {
      const status = notification.params as Partial<RuntimeHostStatus>;
      if (
        (status.state === "starting" ||
          status.state === "connected" ||
          status.state === "reconnecting" ||
          status.state === "offline") &&
        (status.message === null || typeof status.message === "string")
      ) {
        for (const window of BrowserWindow.getAllWindows()) {
          sendToLiveWindow(window, DESKTOP_IPC_CHANNELS.runtime.status, status);
        }
      }
      return;
    }
    if (notification.method !== "event") {
      return;
    }
    const event = notification.params as RuntimeJournalEvent;
    for (const window of BrowserWindow.getAllWindows()) {
      sendToLiveWindow(window, DESKTOP_IPC_CHANNELS.runtime.event, event);
    }
  });

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.threadList,
    async (event, params: RuntimeThreadCatalogParams = {}) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeThreadListPage>("thread.list", { ...params })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.threadGet,
    async (event, threadId: string) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeThreadGetResult>("thread.get", { threadId })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.turnList,
    async (event, params: RuntimeTurnListParams) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeTurnListPage>("turn.list", { ...params })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.threadCreate,
    async (event, params: RuntimeThreadCreateParams) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeThreadCreateResult>("thread.create", { ...params })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.threadRename,
    async (event, params: RuntimeThreadRenameParams) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeThreadMutationResult>("thread.rename", { ...params })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.threadArchive,
    async (event, threadId: string) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeThreadMutationResult>("thread.archive", { threadId })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.threadUnarchive,
    async (event, threadId: string) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeThreadMutationResult>("thread.unarchive", { threadId })
      );
    }
  );

  ipcMain.handle(DESKTOP_IPC_CHANNELS.runtime.providerList, async (event) => {
    trustPolicy.assertTrustedIpc(event);
    return invokeRuntime(() =>
      runtimeHost.request<{ providers: RuntimeProviderSummary[] }>("provider.list")
    );
  });

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.providerConfigure,
    async (event, params: RuntimeProviderConfigureParams) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeProviderConfigureResult>("provider.configure", {
          ...params
        })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.providerDiscoverModels,
    async (event, params: RuntimeProviderDiscoverModelsParams) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeProviderDiscoverModelsResult>("provider.discover_models", {
          ...params
        })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.providerDisconnect,
    async (event, providerId: unknown) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeProviderConfigureResult>("provider.disconnect", {
          providerId
        })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.providerRemove,
    async (event, providerId: unknown) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeProviderRemoveResult>("provider.remove", { providerId })
      );
    }
  );

  ipcMain.handle(DESKTOP_IPC_CHANNELS.runtime.modelList, async (event) => {
    trustPolicy.assertTrustedIpc(event);
    return invokeRuntime(() =>
      runtimeHost.request<{ models: RuntimeModelSummary[] }>("model.list")
    );
  });

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.modelSetEnabled,
    async (event, params: RuntimeModelSetEnabledParams) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeModelSetEnabledResult>("model.set_enabled", {
          ...params
        })
      );
    }
  );

  ipcMain.handle(DESKTOP_IPC_CHANNELS.runtime.skillList, async (event) => {
    trustPolicy.assertTrustedIpc(event);
    return invokeRuntime(() => runtimeHost.request<RuntimeSkillListResult>("skill.list"));
  });

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.skillSetEnabled,
    async (event, params: RuntimeSkillSetEnabledParams) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeSkillSetEnabledResult>("skill.set_enabled", {
          ...params
        })
      );
    }
  );

  ipcMain.handle(DESKTOP_IPC_CHANNELS.runtime.usageRead, async (event) => {
    trustPolicy.assertTrustedIpc(event);
    return invokeRuntime(() => runtimeHost.request<RuntimeUsageReadResult>("usage.read"));
  });

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.turnStart,
    async (event, params: RuntimeTurnStartParams) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeTurnStartResult>("turn.start", { ...params })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.runCancel,
    async (event, runId: unknown) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeCancelRunResult>("run.cancel", { runId })
      );
    }
  );

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.eventReplay,
    async (event, afterSeq: unknown, limit: unknown) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeReplayResult>("event.replay", {
          afterSeq,
          ...(limit === undefined ? {} : { limit })
        })
      );
    }
  );

  ipcMain.handle(DESKTOP_IPC_CHANNELS.preferences.get, async (event) => {
    trustPolicy.assertTrustedIpc(event);
    return getUiPreferences();
  });

  ipcMain.handle(DESKTOP_IPC_CHANNELS.workspace.chooseDirectory, async (event) => {
    const window = trustedRequestingWindow(event, trustPolicy);
    const result = await dialog.showOpenDialog(window, { properties: ["openDirectory"] });
    const directory = result.filePaths[0];
    return result.canceled || !directory ? null : workspaceFromDirectory(directory);
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
