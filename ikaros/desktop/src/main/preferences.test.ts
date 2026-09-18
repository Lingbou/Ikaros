import { describe, expect, it, vi } from "vitest";

vi.mock("electron", () => ({
  app: { getPath: vi.fn(() => "C:\\Ikaros-test") },
  nativeTheme: { themeSource: "dark" },
}));

import {
  cloneUiPreferences,
  DEFAULT_DARK_THEME,
  DEFAULT_PROFILE_USERNAME,
  DEFAULT_SIDEBAR_WIDTH,
  DEFAULT_UI_PREFERENCES,
  MAX_PROFILE_USERNAME_LENGTH,
  MAX_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  mergeUiPreferences,
  type UiPreferences,
} from "../shared/platform";
import { sanitizePreferencesPatch, sanitizeStoredPreferences } from "./preferences";

function storedPreferences(overrides: Partial<UiPreferences> = {}): UiPreferences {
  const base = cloneUiPreferences(DEFAULT_UI_PREFERENCES);
  return {
    ...base,
    ...overrides,
    darkTheme: { ...(overrides.darkTheme ?? base.darkTheme) },
    lightTheme: { ...(overrides.lightTheme ?? base.lightTheme) },
  };
}

describe("stored UI preferences", () => {
  it("accepts the complete current preference shape", () => {
    const stored = storedPreferences({
      language: "zh-CN",
      username: "  Nova Lane  ",
      sidebarCollapsed: true,
      sidebarWidth: 412,
      darkTheme: { ...DEFAULT_DARK_THEME, accent: "#ABCDEF" },
    });

    expect(sanitizeStoredPreferences(stored)).toEqual({
      ...stored,
      username: "Nova Lane",
      darkTheme: { ...stored.darkTheme, accent: "#abcdef" },
    });
  });

  it("resets incomplete, unknown, or unsupported stored preferences to defaults", () => {
    const partialTheme = { ...DEFAULT_DARK_THEME, uiFont: "unknown" as never };
    for (const value of [
      {},
      { colorScheme: "dark", sidebarCollapsed: true, reduceMotion: true },
      { ...storedPreferences(), extra: true },
      storedPreferences({ colorScheme: "blue" as never }),
      storedPreferences({ language: "fr" as never }),
      storedPreferences({ username: "" }),
      storedPreferences({ sidebarWidth: 260.5 }),
      storedPreferences({ darkTheme: partialTheme }),
    ]) {
      expect(sanitizeStoredPreferences(value)).toEqual(DEFAULT_UI_PREFERENCES);
    }
  });

  it("rejects incomplete stored themes instead of filling their fields", () => {
    const { accent: _accent, ...partialDarkTheme } = DEFAULT_DARK_THEME;
    expect(
      sanitizeStoredPreferences(
        storedPreferences({ darkTheme: partialDarkTheme as never }),
      ),
    ).toEqual(DEFAULT_UI_PREFERENCES);
  });

  it("accepts supported stored UI languages", () => {
    expect(sanitizeStoredPreferences(storedPreferences({ language: "zh-CN" })).language).toBe(
      "zh-CN",
    );
    expect(sanitizeStoredPreferences(storedPreferences({ language: "en" })).language).toBe("en");
  });

  it("trims valid stored usernames", () => {
    expect(
      sanitizeStoredPreferences(storedPreferences({ username: "  Nova Lane  " })).username,
    ).toBe("Nova Lane");
  });

  it("accepts bounded stored sidebar widths", () => {
    for (const sidebarWidth of [MIN_SIDEBAR_WIDTH, DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH]) {
      expect(sanitizeStoredPreferences(storedPreferences({ sidebarWidth })).sidebarWidth).toBe(
        sidebarWidth,
      );
    }
  });
});

describe("UI preference patches", () => {
  it("accepts supported patches and rejects invalid language updates", () => {
    expect(sanitizePreferencesPatch({ language: "zh-CN" })).toEqual({ language: "zh-CN" });
    expect(() => sanitizePreferencesPatch({ language: "fr" })).toThrow(
      "Unsupported UI language preference.",
    );
  });

  it("normalizes valid username patches and rejects invalid updates", () => {
    expect(sanitizePreferencesPatch({ username: "  Nova Lane  " })).toEqual({
      username: "Nova Lane",
    });
    expect(
      sanitizePreferencesPatch({ username: "x".repeat(MAX_PROFILE_USERNAME_LENGTH) }),
    ).toEqual({ username: "x".repeat(MAX_PROFILE_USERNAME_LENGTH) });

    for (const username of [null, 42, "", "   ", "x".repeat(33)]) {
      expect(() => sanitizePreferencesPatch({ username })).toThrow(/username/);
    }
    expect(DEFAULT_PROFILE_USERNAME).toBe("User");
  });

  it("accepts bounded sidebar width patches and rejects invalid updates", () => {
    for (const sidebarWidth of [MIN_SIDEBAR_WIDTH, DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH]) {
      expect(sanitizePreferencesPatch({ sidebarWidth })).toEqual({ sidebarWidth });
    }

    for (const sidebarWidth of [
      MIN_SIDEBAR_WIDTH - 1,
      MAX_SIDEBAR_WIDTH + 1,
      260.5,
      Number.NaN,
      Number.NEGATIVE_INFINITY,
      "260",
    ]) {
      expect(() => sanitizePreferencesPatch({ sidebarWidth })).toThrow(
        `sidebarWidth must be an integer from ${MIN_SIDEBAR_WIDTH} to ${MAX_SIDEBAR_WIDTH}.`,
      );
    }
  });

  it("preserves sidebar width when cloning and merging preferences", () => {
    const resized = mergeUiPreferences(DEFAULT_UI_PREFERENCES, { sidebarWidth: 412 });
    const cloned = cloneUiPreferences(resized);

    expect(resized.sidebarWidth).toBe(412);
    expect(cloned.sidebarWidth).toBe(412);
    expect(cloned).not.toBe(resized);
  });

  it("validates theme patches and deeply merges them", () => {
    const patch = sanitizePreferencesPatch({
      darkTheme: { accent: "#FF5500", contrast: 75 },
    });
    expect(patch).toEqual({ darkTheme: { accent: "#ff5500", contrast: 75 } });
    expect(mergeUiPreferences(DEFAULT_UI_PREFERENCES, patch).darkTheme).toEqual({
      ...DEFAULT_DARK_THEME,
      accent: "#ff5500",
      contrast: 75,
    });
    expect(() => sanitizePreferencesPatch({ darkTheme: { accent: "purple" } })).toThrow(
      "six-digit hex color",
    );
    expect(() => sanitizePreferencesPatch({ darkTheme: { contrast: 101 } })).toThrow(
      "integer from 0 to 100",
    );
  });
});
