import type { IkarosRuntimeBridgeApi, RuntimeWorkspaceSummary } from "./runtime";

export type ColorSchemePreference = "system" | "light" | "dark";
export type UiLanguagePreference = "en" | "zh-CN";
export type UiFontPreference = "system" | "inter" | "sego-ui";
export type CodeFontPreference = "system-mono" | "cascadia-code" | "consolas";

export const DEFAULT_SIDEBAR_WIDTH = 260;
export const MIN_SIDEBAR_WIDTH = 220;
export const MAX_SIDEBAR_WIDTH = 520;
export const DEFAULT_PROFILE_USERNAME = "User";
export const MAX_PROFILE_USERNAME_LENGTH = 32;

export interface ThemePreferences {
  accent: string;
  background: string;
  foreground: string;
  uiFont: UiFontPreference;
  codeFont: CodeFontPreference;
  translucentSidebar: boolean;
  contrast: number;
}

export type ThemePreferencesPatch = Partial<ThemePreferences>;

export interface UiPreferences {
  colorScheme: ColorSchemePreference;
  language: UiLanguagePreference;
  username: string;
  sidebarCollapsed: boolean;
  sidebarWidth: number;
  reduceMotion: boolean;
  darkTheme: ThemePreferences;
  lightTheme: ThemePreferences;
}

export type UiPreferencesPatch = Partial<
  Omit<UiPreferences, "darkTheme" | "lightTheme">
> & {
  darkTheme?: ThemePreferencesPatch;
  lightTheme?: ThemePreferencesPatch;
};

export interface IkarosDesktopApi {
  readonly runtime: IkarosRuntimeBridgeApi;
  readonly workspace: {
    chooseDirectory(): Promise<RuntimeWorkspaceSummary | null>;
  };
  readonly preferences: {
    get(): Promise<UiPreferences>;
    update(patch: UiPreferencesPatch): Promise<UiPreferences>;
    onChanged(listener: (preferences: UiPreferences) => void): () => void;
  };
  readonly windowControls: {
    readonly usesCustomTitleBar: boolean;
    close(): Promise<void>;
    minimize(): Promise<void>;
    toggleMaximize(): Promise<void>;
  };
}

export const DEFAULT_DARK_THEME: Readonly<ThemePreferences> = Object.freeze({
  accent: "#339cff",
  background: "#181818",
  foreground: "#ffffff",
  uiFont: "system",
  codeFont: "system-mono",
  translucentSidebar: true,
  contrast: 60
});

export const DEFAULT_LIGHT_THEME: Readonly<ThemePreferences> = Object.freeze({
  accent: "#087eea",
  background: "#f7f7f7",
  foreground: "#171717",
  uiFont: "system",
  codeFont: "system-mono",
  translucentSidebar: false,
  contrast: 60
});

export const DEFAULT_UI_PREFERENCES: Readonly<UiPreferences> = Object.freeze({
  colorScheme: "dark",
  language: "en",
  username: DEFAULT_PROFILE_USERNAME,
  sidebarCollapsed: false,
  sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
  reduceMotion: false,
  darkTheme: DEFAULT_DARK_THEME,
  lightTheme: DEFAULT_LIGHT_THEME
});

export function cloneUiPreferences(preferences: Readonly<UiPreferences>): UiPreferences {
  return {
    ...preferences,
    darkTheme: { ...preferences.darkTheme },
    lightTheme: { ...preferences.lightTheme }
  };
}

export function mergeUiPreferences(
  preferences: Readonly<UiPreferences>,
  patch: UiPreferencesPatch
): UiPreferences {
  return {
    ...preferences,
    ...patch,
    darkTheme: { ...preferences.darkTheme, ...patch.darkTheme },
    lightTheme: { ...preferences.lightTheme, ...patch.lightTheme }
  };
}

export const DESKTOP_IPC_CHANNELS = Object.freeze({
  runtime: {
    event: "ikaros:runtime:event",
    threadCreate: "ikaros:runtime:thread-create",
    threadList: "ikaros:runtime:thread-list",
    turnStart: "ikaros:runtime:turn-start",
    runCancel: "ikaros:runtime:run-cancel",
    eventReplay: "ikaros:runtime:event-replay",
    providerList: "ikaros:runtime:provider-list",
    providerConfigure: "ikaros:runtime:provider-configure",
    providerDiscoverModels: "ikaros:runtime:provider-discover-models",
    providerDisconnect: "ikaros:runtime:provider-disconnect",
    providerRemove: "ikaros:runtime:provider-remove",
    modelList: "ikaros:runtime:model-list",
    modelSetEnabled: "ikaros:runtime:model-set-enabled"
  },
  workspace: {
    chooseDirectory: "ikaros:workspace:choose-directory"
  },
  preferences: {
    changed: "ikaros:preferences:changed",
    get: "ikaros:preferences:get",
    update: "ikaros:preferences:update"
  },
  window: {
    close: "ikaros:window:close",
    minimize: "ikaros:window:minimize",
    toggleMaximize: "ikaros:window:toggle-maximize"
  }
} as const);
