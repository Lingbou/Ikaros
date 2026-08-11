import { BrowserWindow, ipcMain, type IpcMainInvokeEvent } from "electron";

import { DESKTOP_IPC_CHANNELS, type UiPreferences } from "../shared/platform";
import type {
  RuntimeCancelRunResult,
  RuntimeInvocationResult,
  RuntimeJournalEvent,
  RuntimeReplayResult,
  RuntimeThreadCreateResult,
  RuntimeThreadSummary,
  RuntimeTurnStartParams,
  RuntimeTurnStartResult
} from "../shared/runtime";
import { getUiPreferences, updateUiPreferences } from "./preferences";
import type { RendererTrustPolicy } from "./security";
import { RuntimeRpcError, type RuntimeHost } from "./runtimeHost";
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

export function registerDesktopIpc(
  trustPolicy: RendererTrustPolicy,
  runtimeHost: Pick<RuntimeHost, "request" | "onNotification">
): RemoveIpcHandlers {
  const handledChannels = [
    DESKTOP_IPC_CHANNELS.runtime.threadCreate,
    DESKTOP_IPC_CHANNELS.runtime.threadList,
    DESKTOP_IPC_CHANNELS.runtime.turnStart,
    DESKTOP_IPC_CHANNELS.runtime.runCancel,
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
      sendToLiveWindow(window, DESKTOP_IPC_CHANNELS.runtime.event, event);
    }
  });

  ipcMain.handle(DESKTOP_IPC_CHANNELS.runtime.threadList, async (event) => {
    trustPolicy.assertTrustedIpc(event);
    return invokeRuntime(() =>
      runtimeHost.request<{ threads: RuntimeThreadSummary[] }>("thread.list")
    );
  });

  ipcMain.handle(
    DESKTOP_IPC_CHANNELS.runtime.threadCreate,
    async (event, title: unknown, clientRequestId: unknown) => {
      trustPolicy.assertTrustedIpc(event);
      return invokeRuntime(() =>
        runtimeHost.request<RuntimeThreadCreateResult>("thread.create", {
          title,
          clientRequestId
        })
      );
    }
  );

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
