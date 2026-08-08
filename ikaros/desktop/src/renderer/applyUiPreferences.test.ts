import { afterEach, describe, expect, it, vi } from "vitest";

import type { ThemePreferences, UiPreferences } from "../shared/platform";
import { deriveThemeColors } from "../shared/theme";
import { setUiLanguage } from "./i18n";
import {
  applyDocumentPreferences,
  createThemeTransitionCoordinator,
  prefersDarkColorScheme,
  type ThemeTransitionDependencies
} from "./applyUiPreferences";

const darkTheme: ThemePreferences = {
  accent: "#4c9aff",
  background: "#101820",
  foreground: "#f0f4f8",
  uiFont: "sego-ui",
  codeFont: "cascadia-code",
  translucentSidebar: true,
  contrast: 25
};

const lightTheme: ThemePreferences = {
  accent: "#aa5500",
  background: "#fefefe",
  foreground: "#121212",
  uiFont: "inter",
  codeFont: "consolas",
  translucentSidebar: false,
  contrast: 75
};

const preferences: UiPreferences = {
  colorScheme: "system",
  language: "zh-CN",
  sidebarCollapsed: false,
  reduceMotion: true,
  darkTheme,
  lightTheme
};

afterEach(() => {
  Reflect.deleteProperty(window, "matchMedia");
  document.documentElement.removeAttribute("style");
  document.documentElement.removeAttribute("data-color-scheme");
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("data-reduce-motion");
  document.documentElement.removeAttribute("data-translucent-sidebar");
  setUiLanguage("en");
});

describe("prefersDarkColorScheme", () => {
  it("uses matchMedia when it is available", () => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({ matches: false }))
    });

    expect(prefersDarkColorScheme()).toBe(false);
    expect(window.matchMedia).toHaveBeenCalledWith("(prefers-color-scheme: dark)");
  });

  it("falls back to dark when matchMedia is unavailable", () => {
    Reflect.deleteProperty(window, "matchMedia");
    expect(prefersDarkColorScheme()).toBe(true);
  });
});

describe("applyDocumentPreferences", () => {
  it.each([
    {
      systemPrefersDark: true,
      scheme: "dark",
      theme: darkTheme,
      uiFont: '"Segoe UI", ui-sans-serif, sans-serif',
      codeFont: '"Cascadia Code", "Cascadia Mono", Consolas, monospace'
    },
    {
      systemPrefersDark: false,
      scheme: "light",
      theme: lightTheme,
      uiFont: 'Inter, "Segoe UI", sans-serif',
      codeFont: 'Consolas, "Courier New", monospace'
    }
  ] as const)(
    "applies the $scheme system theme, CSS variables, fonts, and translucency",
    ({ systemPrefersDark, scheme, theme, uiFont, codeFont }) => {
      applyDocumentPreferences(preferences, systemPrefersDark);

      const root = document.documentElement;
      const colors = deriveThemeColors(theme);
      const expectedVariables: Record<string, string> = {
        "--accent": colors.accent,
        "--border": colors.border,
        "--border-soft": colors.borderSoft,
        "--canvas": colors.canvas,
        "--code-font": codeFont,
        "--menu": colors.menu,
        "--muted": colors.muted,
        "--muted-strong": colors.mutedStrong,
        "--panel": colors.panel,
        "--panel-hover": colors.panelHover,
        "--panel-raised": colors.panelRaised,
        "--panel-selected": colors.panelSelected,
        "--separator": colors.separator,
        "--shadow-color": colors.shadow,
        "--sidebar": colors.sidebar,
        "--sidebar-bottom": colors.sidebarBottom,
        "--sidebar-translucent": colors.sidebarTranslucent,
        "--surface-hover": colors.surfaceHover,
        "--text": colors.foreground,
        "--ui-font": uiFont
      };

      expect(root.dataset).toMatchObject({
        colorScheme: "system",
        theme: scheme,
        reduceMotion: "true",
        translucentSidebar: String(theme.translucentSidebar)
      });
      expect(root.style.colorScheme).toBe(scheme);
      expect(root.lang).toBe("zh-CN");

      for (const [property, value] of Object.entries(expectedVariables)) {
        expect(root.style.getPropertyValue(property)).toBe(value);
      }
    }
  );
});

function transitionDependencies({
  startViewTransition,
  reducedMotion = false,
  visible = true
}: {
  startViewTransition?: ThemeTransitionDependencies["getStartViewTransition"] extends () => infer T
    ? T
    : never;
  reducedMotion?: boolean;
  visible?: boolean;
} = {}): ThemeTransitionDependencies {
  return {
    getRoot: () => document.documentElement,
    getViewport: () => ({ width: 1000, height: 800 }),
    getStartViewTransition: () => startViewTransition,
    prefersReducedMotion: () => reducedMotion,
    isDocumentVisible: () => visible
  };
}

describe("theme transition coordinator", () => {
  it("reveals a new effective scheme from the supplied origin and cleans up", async () => {
    const root = document.documentElement;
    root.dataset.theme = "dark";
    let finish!: () => void;
    const finished = new Promise<void>((resolve) => {
      finish = resolve;
    });
    const startViewTransition = vi.fn((update: () => void) => {
      update();
      return { finished, skipTransition: vi.fn() };
    });
    const coordinator = createThemeTransitionCoordinator(
      transitionDependencies({ startViewTransition })
    );
    const update = vi.fn(() => {
      root.dataset.theme = "light";
    });

    expect(
      coordinator.apply(update, {
        nextScheme: "light",
        reduceMotion: false,
        origin: { x: 100, y: 200 }
      })
    ).toBe("transition");
    expect(startViewTransition).toHaveBeenCalledOnce();
    expect(update).toHaveBeenCalledOnce();
    expect(root.dataset.themeTransition).toBe("radial");
    expect(root.style.getPropertyValue("--theme-transition-x")).toBe("100px");
    expect(root.style.getPropertyValue("--theme-transition-y")).toBe("200px");
    expect(root.style.getPropertyValue("--theme-transition-radius")).toBe("1178px");

    finish();
    await finished;
    await Promise.resolve();
    expect(root.dataset.themeTransition).toBeUndefined();
  });

  it.each([
    { name: "the theme is hydrating", current: undefined, path: "initial" },
    { name: "the effective scheme is unchanged", current: "light", path: "same-scheme" },
    { name: "app motion is reduced", current: "dark", path: "reduced-motion", appReduced: true },
    { name: "OS motion is reduced", current: "dark", path: "reduced-motion", osReduced: true },
    { name: "the page is hidden", current: "dark", path: "hidden", visible: false },
    { name: "the API is unavailable", current: "dark", path: "unsupported" }
  ] as const)("applies immediately when $name", (scenario) => {
    const root = document.documentElement;
    if (scenario.current) root.dataset.theme = scenario.current;
    else delete root.dataset.theme;
    const startViewTransition =
      scenario.path === "unsupported"
        ? undefined
        : vi.fn((update: () => void) => {
            update();
            return { finished: Promise.resolve() };
          });
    const coordinator = createThemeTransitionCoordinator(
      transitionDependencies({
        startViewTransition,
        reducedMotion: Boolean("osReduced" in scenario && scenario.osReduced),
        visible: !("visible" in scenario) || scenario.visible
      })
    );
    const update = vi.fn(() => {
      root.dataset.theme = "light";
    });

    expect(
      coordinator.apply(update, {
        nextScheme: "light",
        reduceMotion: Boolean("appReduced" in scenario && scenario.appReduced)
      })
    ).toBe(scenario.path);
    expect(update).toHaveBeenCalledOnce();
    if (startViewTransition) expect(startViewTransition).not.toHaveBeenCalled();
  });

  it("falls back exactly once when starting a transition throws", () => {
    const root = document.documentElement;
    root.dataset.theme = "dark";
    const coordinator = createThemeTransitionCoordinator(
      transitionDependencies({
        startViewTransition: () => {
          throw new Error("snapshot failed");
        }
      })
    );
    const update = vi.fn(() => {
      root.dataset.theme = "light";
    });

    expect(
      coordinator.apply(update, { nextScheme: "light", reduceMotion: false })
    ).toBe("transition-error");
    expect(update).toHaveBeenCalledOnce();
    expect(root.dataset.themeTransition).toBeUndefined();
  });

  it("cancels a pending reveal when the user quickly returns to the current scheme", () => {
    const root = document.documentElement;
    root.dataset.theme = "dark";
    let staleUpdate!: () => void;
    const skipTransition = vi.fn();
    const coordinator = createThemeTransitionCoordinator(
      transitionDependencies({
        startViewTransition: (update) => {
          staleUpdate = update;
          return { finished: new Promise(() => undefined), skipTransition };
        }
      })
    );
    const applyLight = vi.fn(() => {
      root.dataset.theme = "light";
    });
    const applyDark = vi.fn(() => {
      root.dataset.theme = "dark";
    });

    expect(
      coordinator.apply(applyLight, { nextScheme: "light", reduceMotion: false })
    ).toBe("transition");
    expect(
      coordinator.apply(applyDark, { nextScheme: "dark", reduceMotion: false })
    ).toBe("same-scheme");
    staleUpdate();

    expect(skipTransition).toHaveBeenCalledOnce();
    expect(applyLight).not.toHaveBeenCalled();
    expect(applyDark).toHaveBeenCalledOnce();
    expect(root.dataset.theme).toBe("dark");
  });
});
