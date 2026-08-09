import { app, BrowserWindow, nativeTheme, type BrowserWindowConstructorOptions } from "electron";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import type { UiPreferences } from "../shared/platform";
import { activeThemePreferences, deriveThemeColors } from "../shared/theme";
import { installWindowSecurity, type RendererTrustPolicy } from "./security";

const WINDOWS_TITLE_BAR_HEIGHT = 36;

export function rendererEntryUrl(): string {
  return (
    process.env.ELECTRON_RENDERER_URL ??
    pathToFileURL(join(__dirname, "../renderer/index.html")).toString()
  );
}

export function updateWindowChrome(
  window: BrowserWindow,
  preferences: Readonly<UiPreferences>
): void {
  const theme = activeThemePreferences(preferences, nativeTheme.shouldUseDarkColors);
  const colors = deriveThemeColors(theme);
  window.setBackgroundColor(colors.background);

  if (process.platform === "win32") {
    window.setTitleBarOverlay({
      color: colors.titlebar,
      symbolColor: colors.foreground,
      height: WINDOWS_TITLE_BAR_HEIGHT
    });
  }
}

function platformTitleBarOptions(preferences: Readonly<UiPreferences>): Pick<
  BrowserWindowConstructorOptions,
  "frame" | "titleBarOverlay" | "titleBarStyle"
> {
  if (process.platform === "win32") {
    const theme = activeThemePreferences(preferences, nativeTheme.shouldUseDarkColors);
    const colors = deriveThemeColors(theme);
    return {
      titleBarStyle: "hidden",
      titleBarOverlay: {
        color: colors.titlebar,
        symbolColor: colors.foreground,
        height: WINDOWS_TITLE_BAR_HEIGHT
      }
    };
  }

  if (process.platform === "darwin") {
    return { titleBarStyle: "hiddenInset" };
  }

  return { frame: false, titleBarStyle: "hidden" };
}

function platformWindowIcon(): Pick<BrowserWindowConstructorOptions, "icon"> {
  if (process.platform === "linux") {
    return { icon: join(app.getAppPath(), "build/icon.png") };
  }

  if (process.platform === "win32" && !app.isPackaged) {
    return { icon: join(__dirname, "../../build/icon.ico") };
  }

  return {};
}

export function createMainWindow(
  rendererUrl: string,
  trustPolicy: RendererTrustPolicy,
  preferences: Readonly<UiPreferences>
): BrowserWindow {
  const theme = activeThemePreferences(preferences, nativeTheme.shouldUseDarkColors);
  const window = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 960,
    minHeight: 640,
    show: false,
    autoHideMenuBar: true,
    backgroundColor: theme.background,
    ...platformWindowIcon(),
    ...platformTitleBarOptions(preferences),
    webPreferences: {
      preload: join(__dirname, "../preload/index.js"),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      nodeIntegrationInSubFrames: false,
      webviewTag: false,
      spellcheck: true
    }
  });

  installWindowSecurity(window, trustPolicy);
  updateWindowChrome(window, preferences);

  window.once("ready-to-show", () => {
    window.show();
  });

  void window.loadURL(rendererUrl);
  return window;
}
