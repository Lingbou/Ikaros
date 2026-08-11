import { app, BrowserWindow, Menu, nativeTheme } from "electron";

import { registerDesktopIpc } from "./ipc";
import { applyNativeTheme, getUiPreferences } from "./preferences";
import { developmentRuntimeRoot, RuntimeHost } from "./runtimeHost";
import { createRendererTrustPolicy } from "./security";
import { createMainWindow, rendererEntryUrl, updateWindowChrome } from "./window";

let mainWindow: BrowserWindow | null = null;
let removeIpcHandlers: (() => void) | undefined;
let runtimeHost: RuntimeHost | undefined;
let quittingAfterRuntimeShutdown = false;

const singleInstanceLock = app.requestSingleInstanceLock();

function focusMainWindow(): void {
  if (!mainWindow || mainWindow.isDestroyed()) {
    return;
  }
  if (mainWindow.isMinimized()) {
    mainWindow.restore();
  }
  mainWindow.show();
  mainWindow.focus();
}

async function initializeDesktop(): Promise<void> {
  if (process.platform === "win32") {
    app.setAppUserModelId("io.github.lingbou.ikaros");
  }

  Menu.setApplicationMenu(null);

  runtimeHost = new RuntimeHost({ runtimeRoot: developmentRuntimeRoot(app.getAppPath()) });
  await runtimeHost.start();

  const preferences = await getUiPreferences();
  applyNativeTheme(preferences);

  const rendererUrl = rendererEntryUrl();
  const trustPolicy = createRendererTrustPolicy(rendererUrl);
  removeIpcHandlers = registerDesktopIpc(trustPolicy);

  const openWindow = async (): Promise<BrowserWindow> => {
    const currentPreferences = await getUiPreferences();
    const window = createMainWindow(rendererUrl, trustPolicy, currentPreferences);
    window.on("closed", () => {
      if (mainWindow === window) {
        mainWindow = null;
      }
    });
    mainWindow = window;
    return window;
  };

  await openWindow();

  nativeTheme.on("updated", () => {
    void getUiPreferences().then((currentPreferences) => {
      for (const window of BrowserWindow.getAllWindows()) {
        updateWindowChrome(window, currentPreferences);
      }
    });
  });

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      void openWindow();
    } else {
      focusMainWindow();
    }
  });
}

if (!singleInstanceLock) {
  app.quit();
} else {
  app.on("second-instance", focusMainWindow);
  app.whenReady().then(initializeDesktop).catch((error: unknown) => {
    console.error("Failed to initialize the Ikaros desktop app.", error);
    app.quit();
  });
}

app.on("before-quit", (event) => {
  removeIpcHandlers?.();
  removeIpcHandlers = undefined;
  if (runtimeHost?.isRunning && !quittingAfterRuntimeShutdown) {
    event.preventDefault();
    quittingAfterRuntimeShutdown = true;
    void runtimeHost.stop().finally(() => app.quit());
  }
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});
