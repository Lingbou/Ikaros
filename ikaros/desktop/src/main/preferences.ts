import { app, nativeTheme } from "electron";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

import {
  cloneUiPreferences,
  DEFAULT_UI_PREFERENCES,
  MAX_SIDEBAR_WIDTH,
  MAX_PROFILE_USERNAME_LENGTH,
  mergeUiPreferences,
  MIN_SIDEBAR_WIDTH,
  type CodeFontPreference,
  type ColorSchemePreference,
  type ThemePreferences,
  type ThemePreferencesPatch,
  type UiFontPreference,
  type UiLanguagePreference,
  type UiPreferences,
  type UiPreferencesPatch
} from "../shared/platform";
import { isHexColor, normalizeHexColor } from "../shared/theme";

let cachedPreferences: UiPreferences | undefined;
let loadingPreferences: Promise<UiPreferences> | undefined;
let updateQueue: Promise<void> = Promise.resolve();

function preferencesPath(): string {
  return join(app.getPath("userData"), "ui-preferences.json");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isColorScheme(value: unknown): value is ColorSchemePreference {
  return value === "system" || value === "light" || value === "dark";
}

function isUiLanguage(value: unknown): value is UiLanguagePreference {
  return value === "en" || value === "zh-CN";
}

function isUiFont(value: unknown): value is UiFontPreference {
  return value === "system" || value === "inter" || value === "sego-ui";
}

function isCodeFont(value: unknown): value is CodeFontPreference {
  return value === "system-mono" || value === "cascadia-code" || value === "consolas";
}

function isContrast(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 100;
}

function isSidebarWidth(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    Number.isInteger(value) &&
    value >= MIN_SIDEBAR_WIDTH &&
    value <= MAX_SIDEBAR_WIDTH
  );
}

function storedUsername(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const username = value.trim();
  return username.length > 0 && username.length <= MAX_PROFILE_USERNAME_LENGTH
    ? username
    : null;
}

function usernamePatch(value: unknown): string {
  if (typeof value !== "string") {
    throw new TypeError("username must be a string.");
  }
  const username = value.trim();
  if (!username || username.length > MAX_PROFILE_USERNAME_LENGTH) {
    throw new TypeError(
      `username must contain 1 to ${MAX_PROFILE_USERNAME_LENGTH} characters after trimming.`
    );
  }
  return username;
}

const STORED_THEME_KEYS = [
  "accent",
  "background",
  "foreground",
  "uiFont",
  "codeFont",
  "translucentSidebar",
  "contrast"
] as const;

const STORED_PREFERENCE_KEYS = [
  "colorScheme",
  "language",
  "username",
  "sidebarCollapsed",
  "sidebarWidth",
  "reduceMotion",
  "darkTheme",
  "lightTheme"
] as const;

function hasExactKeys(
  value: Record<string, unknown>,
  keys: readonly string[]
): boolean {
  const actual = Object.keys(value);
  return actual.length === keys.length && keys.every((key) => key in value);
}

function isStoredTheme(value: unknown): value is ThemePreferences {
  return (
    isRecord(value) &&
    hasExactKeys(value, STORED_THEME_KEYS) &&
    isHexColor(value.accent) &&
    isHexColor(value.background) &&
    isHexColor(value.foreground) &&
    isUiFont(value.uiFont) &&
    isCodeFont(value.codeFont) &&
    typeof value.translucentSidebar === "boolean" &&
    isContrast(value.contrast)
  );
}

function normalizedStoredTheme(theme: ThemePreferences): ThemePreferences {
  return {
    ...theme,
    accent: normalizeHexColor(theme.accent),
    background: normalizeHexColor(theme.background),
    foreground: normalizeHexColor(theme.foreground)
  };
}

function sanitizeThemePatch(value: unknown, property: string): ThemePreferencesPatch {
  if (!isRecord(value)) {
    throw new TypeError(`${property} must be an object.`);
  }

  const patch: ThemePreferencesPatch = {};

  for (const color of ["accent", "background", "foreground"] as const) {
    if (color in value) {
      if (!isHexColor(value[color])) {
        throw new TypeError(`${property}.${color} must be a six-digit hex color.`);
      }
      patch[color] = normalizeHexColor(value[color]);
    }
  }

  if ("uiFont" in value) {
    if (!isUiFont(value.uiFont)) {
      throw new TypeError(`${property}.uiFont is unsupported.`);
    }
    patch.uiFont = value.uiFont;
  }

  if ("codeFont" in value) {
    if (!isCodeFont(value.codeFont)) {
      throw new TypeError(`${property}.codeFont is unsupported.`);
    }
    patch.codeFont = value.codeFont;
  }

  if ("translucentSidebar" in value) {
    if (typeof value.translucentSidebar !== "boolean") {
      throw new TypeError(`${property}.translucentSidebar must be a boolean.`);
    }
    patch.translucentSidebar = value.translucentSidebar;
  }

  if ("contrast" in value) {
    if (!isContrast(value.contrast)) {
      throw new TypeError(`${property}.contrast must be an integer from 0 to 100.`);
    }
    patch.contrast = value.contrast;
  }

  return patch;
}

export function sanitizeStoredPreferences(value: unknown): UiPreferences {
  if (!isRecord(value) || !hasExactKeys(value, STORED_PREFERENCE_KEYS)) {
    return cloneUiPreferences(DEFAULT_UI_PREFERENCES);
  }

  const username = storedUsername(value.username);
  if (
    !isColorScheme(value.colorScheme) ||
    !isUiLanguage(value.language) ||
    username === null ||
    typeof value.sidebarCollapsed !== "boolean" ||
    !isSidebarWidth(value.sidebarWidth) ||
    typeof value.reduceMotion !== "boolean" ||
    !isStoredTheme(value.darkTheme) ||
    !isStoredTheme(value.lightTheme)
  ) {
    return cloneUiPreferences(DEFAULT_UI_PREFERENCES);
  }

  return {
    colorScheme: value.colorScheme,
    language: value.language,
    username,
    sidebarCollapsed: value.sidebarCollapsed,
    sidebarWidth: value.sidebarWidth,
    reduceMotion: value.reduceMotion,
    darkTheme: normalizedStoredTheme(value.darkTheme),
    lightTheme: normalizedStoredTheme(value.lightTheme)
  };
}

export function sanitizePreferencesPatch(value: unknown): UiPreferencesPatch {
  if (!isRecord(value)) {
    throw new TypeError("UI preference update must be an object.");
  }

  const patch: UiPreferencesPatch = {};

  if ("colorScheme" in value) {
    if (!isColorScheme(value.colorScheme)) {
      throw new TypeError("Unsupported color scheme preference.");
    }
    patch.colorScheme = value.colorScheme;
  }

  if ("language" in value) {
    if (!isUiLanguage(value.language)) {
      throw new TypeError("Unsupported UI language preference.");
    }
    patch.language = value.language;
  }

  if ("username" in value) {
    patch.username = usernamePatch(value.username);
  }

  if ("sidebarCollapsed" in value) {
    if (typeof value.sidebarCollapsed !== "boolean") {
      throw new TypeError("sidebarCollapsed must be a boolean.");
    }
    patch.sidebarCollapsed = value.sidebarCollapsed;
  }

  if ("sidebarWidth" in value) {
    if (!isSidebarWidth(value.sidebarWidth)) {
      throw new TypeError(
        `sidebarWidth must be an integer from ${MIN_SIDEBAR_WIDTH} to ${MAX_SIDEBAR_WIDTH}.`
      );
    }
    patch.sidebarWidth = value.sidebarWidth;
  }

  if ("reduceMotion" in value) {
    if (typeof value.reduceMotion !== "boolean") {
      throw new TypeError("reduceMotion must be a boolean.");
    }
    patch.reduceMotion = value.reduceMotion;
  }

  if ("darkTheme" in value) {
    patch.darkTheme = sanitizeThemePatch(value.darkTheme, "darkTheme");
  }

  if ("lightTheme" in value) {
    patch.lightTheme = sanitizeThemePatch(value.lightTheme, "lightTheme");
  }

  return patch;
}

async function readPreferences(): Promise<UiPreferences> {
  try {
    const serialized = await readFile(preferencesPath(), "utf8");
    return sanitizeStoredPreferences(JSON.parse(serialized) as unknown);
  } catch (error) {
    const code = isRecord(error) && typeof error.code === "string" ? error.code : undefined;
    if (code !== "ENOENT") {
      console.warn("Unable to read UI preferences; using defaults.", error);
    }
    return cloneUiPreferences(DEFAULT_UI_PREFERENCES);
  }
}

export async function getUiPreferences(): Promise<UiPreferences> {
  if (cachedPreferences) {
    return cloneUiPreferences(cachedPreferences);
  }

  loadingPreferences ??= readPreferences().then((preferences) => {
    cachedPreferences = preferences;
    return preferences;
  });

  return cloneUiPreferences(await loadingPreferences);
}

export function applyNativeTheme(preferences: UiPreferences): void {
  nativeTheme.themeSource = preferences.colorScheme;
}

export function updateUiPreferences(value: unknown): Promise<UiPreferences> {
  const patch = sanitizePreferencesPatch(value);
  const operation = updateQueue.then(async () => {
    const current = await getUiPreferences();
    const next = mergeUiPreferences(current, patch);
    const filePath = preferencesPath();
    const temporaryPath = `${filePath}.tmp`;

    await mkdir(dirname(filePath), { recursive: true });
    await writeFile(temporaryPath, `${JSON.stringify(next, null, 2)}\n`, "utf8");
    await rename(temporaryPath, filePath);

    cachedPreferences = next;
    applyNativeTheme(next);
    return cloneUiPreferences(next);
  });

  updateQueue = operation.then(
    () => undefined,
    () => undefined
  );

  return operation;
}
