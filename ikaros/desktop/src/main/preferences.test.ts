import { describe, expect, it, vi } from "vitest";

vi.mock("electron", () => ({
  app: { getPath: vi.fn(() => "C:\\Ikaros-test") },
  nativeTheme: { themeSource: "dark" },
}));

import {
  DEFAULT_DARK_THEME,
  DEFAULT_UI_PREFERENCES,
  mergeUiPreferences
} from "../shared/platform";
import { sanitizePreferencesPatch, sanitizeStoredPreferences } from "./preferences";

describe("UI preference language validation", () => {
  it("migrates older preference files to the English default", () => {
    expect(
      sanitizeStoredPreferences({
        colorScheme: "dark",
        sidebarCollapsed: true,
        reduceMotion: true,
      }),
    ).toEqual({
      ...DEFAULT_UI_PREFERENCES,
      sidebarCollapsed: true,
      reduceMotion: true,
    });
  });

  it("accepts only supported stored UI languages", () => {
    expect(sanitizeStoredPreferences({ language: "zh-CN" }).language).toBe("zh-CN");
    expect(sanitizeStoredPreferences({ language: "fr" }).language).toBe("en");
  });

  it("accepts supported patches and rejects invalid language updates", () => {
    expect(sanitizePreferencesPatch({ language: "zh-CN" })).toEqual({ language: "zh-CN" });
    expect(() => sanitizePreferencesPatch({ language: "fr" })).toThrow(
      "Unsupported UI language preference.",
    );
  });

  it("migrates missing theme fields without discarding valid custom values", () => {
    expect(
      sanitizeStoredPreferences({
        darkTheme: { accent: "#ABCDEF", contrast: 72, uiFont: "unknown" }
      }).darkTheme
    ).toEqual({
      ...DEFAULT_DARK_THEME,
      accent: "#abcdef",
      contrast: 72
    });
  });

  it("validates theme patches and deeply merges them", () => {
    const patch = sanitizePreferencesPatch({
      darkTheme: { accent: "#FF5500", contrast: 75 }
    });
    expect(patch).toEqual({ darkTheme: { accent: "#ff5500", contrast: 75 } });
    expect(mergeUiPreferences(DEFAULT_UI_PREFERENCES, patch).darkTheme).toEqual({
      ...DEFAULT_DARK_THEME,
      accent: "#ff5500",
      contrast: 75
    });
    expect(() => sanitizePreferencesPatch({ darkTheme: { accent: "purple" } })).toThrow(
      "six-digit hex color"
    );
    expect(() => sanitizePreferencesPatch({ darkTheme: { contrast: 101 } })).toThrow(
      "integer from 0 to 100"
    );
  });
});
