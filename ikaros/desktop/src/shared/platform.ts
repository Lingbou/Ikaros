import type { IkarosRuntimeApi } from "./runtime";

export type ColorSchemePreference = "system" | "light" | "dark";
export type UiLanguagePreference = "en" | "zh-CN";
export type UiFontPreference = "system" | "inter" | "sego-ui";
export type CodeFontPreference = "system-mono" | "cascadia-code" | "consolas";

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
  sidebarCollapsed: boolean;
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
  readonly runtime: IkarosRuntimeApi;
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
  sidebarCollapsed: false,
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
    eventReplay: "ikaros:runtime:event-replay"
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
