import { describe, expect, it } from "vitest";

import type { ThemePreferences, UiPreferences } from "./platform";
import {
  activeThemePreferences,
  deriveThemeColors,
  effectiveColorScheme,
  hexToRgb,
  hexWithAlpha,
  isHexColor,
  mixHexColors,
  readableTextColor
} from "./theme";

const lightTheme: ThemePreferences = {
  accent: "#087eea",
  background: "#f7f7f7",
  foreground: "#171717",
  uiFont: "inter",
  codeFont: "consolas",
  translucentSidebar: false,
  contrast: 60
};

const darkTheme: ThemePreferences = {
  accent: "#339cff",
  background: "#181818",
  foreground: "#ffffff",
  uiFont: "sego-ui",
  codeFont: "cascadia-code",
  translucentSidebar: true,
  contrast: 60
};

describe("effective color scheme", () => {
  it.each([
    [false, "light"],
    [true, "dark"]
  ] as const)("uses the system %s preference", (systemPrefersDark, expected) => {
    expect(effectiveColorScheme("system", systemPrefersDark)).toBe(expected);
  });

  it("keeps an explicit scheme independent of the system preference", () => {
    expect(effectiveColorScheme("light", true)).toBe("light");
    expect(effectiveColorScheme("dark", false)).toBe("dark");
  });

  it("selects the theme for the effective system scheme", () => {
    const preferences: UiPreferences = {
      colorScheme: "system",
      language: "en",
      sidebarCollapsed: false,
      reduceMotion: false,
      lightTheme,
      darkTheme
    };

    expect(activeThemePreferences(preferences, false)).toBe(lightTheme);
    expect(activeThemePreferences(preferences, true)).toBe(darkTheme);
  });
});

describe("derived theme colors", () => {
  const baseTheme: ThemePreferences = {
    accent: "#336699",
    background: "#000000",
    foreground: "#FFFFFF",
    uiFont: "system",
    codeFont: "system-mono",
    translucentSidebar: true,
    contrast: 0
  };

  it("derives the complete palette from the base colors and contrast", () => {
    expect(deriveThemeColors(baseTheme)).toEqual({
      accent: "#336699",
      background: "#000000",
      border: "#1a1a1a",
      borderSoft: "#121212",
      canvas: "#000000",
      foreground: "#ffffff",
      menu: "#161616",
      muted: "#666666",
      mutedStrong: "#8f8f8f",
      panel: "#090909",
      panelHover: "#121212",
      panelRaised: "#0e0e0e",
      panelSelected: "#161616",
      separator: "#0f0f0f",
      shadow: "rgba(0, 0, 0, 0.480)",
      sidebar: "#060606",
      sidebarBottom: "#030608",
      sidebarTranslucent: "rgba(6, 6, 6, 0.900)",
      surfaceHover: "rgba(255, 255, 255, 0.045)",
      titlebar: "#060606"
    });
  });

  it("clamps contrast before deriving the palette", () => {
    expect(deriveThemeColors({ ...baseTheme, contrast: -10 })).toEqual(
      deriveThemeColors({ ...baseTheme, contrast: 0 })
    );
    expect(deriveThemeColors({ ...baseTheme, contrast: 110 })).toEqual(
      deriveThemeColors({ ...baseTheme, contrast: 100 })
    );
  });
});

describe("theme color helper validation", () => {
  it.each([undefined, null, 42, "336699", "#369", "#gg6699", "#33669900"])(
    "rejects invalid six-digit colors (%s)",
    (value) => {
      expect(isHexColor(value)).toBe(false);
    }
  );

  it("accepts case-insensitive six-digit colors", () => {
    expect(isHexColor("#336699")).toBe(true);
    expect(isHexColor("#AABBCC")).toBe(true);
  });

  it.each([
    ["hexToRgb", () => hexToRgb("invalid")],
    ["mixHexColors from", () => mixHexColors("invalid", "#ffffff", 0.5)],
    ["mixHexColors to", () => mixHexColors("#000000", "invalid", 0.5)],
    ["hexWithAlpha", () => hexWithAlpha("invalid", 0.5)],
    ["readableTextColor", () => readableTextColor("invalid")]
  ] as const)("throws for an invalid color passed to %s", (_name, invoke) => {
    expect(invoke).toThrow(TypeError);
  });
});
