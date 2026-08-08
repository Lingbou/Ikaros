import { app, nativeTheme } from "electron";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

import {
  cloneUiPreferences,
  DEFAULT_DARK_THEME,
  DEFAULT_LIGHT_THEME,
  DEFAULT_UI_PREFERENCES,
  mergeUiPreferences,
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

function sanitizeStoredTheme(
  value: unknown,
  fallback: Readonly<ThemePreferences>
): ThemePreferences {
  if (!isRecord(value)) {
    return { ...fallback };
  }

  return {
    accent: isHexColor(value.accent) ? normalizeHexColor(value.accent) : fallback.accent,
    background: isHexColor(value.background)
      ? normalizeHexColor(value.background)
      : fallback.background,
    foreground: isHexColor(value.foreground)
      ? normalizeHexColor(value.foreground)
      : fallback.foreground,
    uiFont: isUiFont(value.uiFont) ? value.uiFont : fallback.uiFont,
    codeFont: isCodeFont(value.codeFont) ? value.codeFont : fallback.codeFont,
    translucentSidebar:
      typeof value.translucentSidebar === "boolean"
        ? value.translucentSidebar
        : fallback.translucentSidebar,
    contrast: isContrast(value.contrast) ? value.contrast : fallback.contrast
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
  if (!isRecord(value)) {
    return cloneUiPreferences(DEFAULT_UI_PREFERENCES);
  }

  return {
    colorScheme: isColorScheme(value.colorScheme)
      ? value.colorScheme
      : DEFAULT_UI_PREFERENCES.colorScheme,
    language: isUiLanguage(value.language)
      ? value.language
      : DEFAULT_UI_PREFERENCES.language,
    sidebarCollapsed:
      typeof value.sidebarCollapsed === "boolean"
        ? value.sidebarCollapsed
        : DEFAULT_UI_PREFERENCES.sidebarCollapsed,
    reduceMotion:
      typeof value.reduceMotion === "boolean"
        ? value.reduceMotion
        : DEFAULT_UI_PREFERENCES.reduceMotion,
    darkTheme: sanitizeStoredTheme(value.darkTheme, DEFAULT_DARK_THEME),
    lightTheme: sanitizeStoredTheme(value.lightTheme, DEFAULT_LIGHT_THEME)
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

  if ("sidebarCollapsed" in value) {
    if (typeof value.sidebarCollapsed !== "boolean") {
      throw new TypeError("sidebarCollapsed must be a boolean.");
    }
    patch.sidebarCollapsed = value.sidebarCollapsed;
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
