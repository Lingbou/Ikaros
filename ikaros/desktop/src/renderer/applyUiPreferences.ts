import type { CodeFontPreference, UiFontPreference, UiPreferences } from "../shared/platform";
import {
  activeThemePreferences,
  deriveThemeColors,
  effectiveColorScheme
} from "../shared/theme";
import { setUiLanguage } from "./i18n";

const UI_FONT_STACKS: Record<UiFontPreference, string> = {
  system: 'Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
  inter: 'Inter, "Segoe UI", sans-serif',
  "sego-ui": '"Segoe UI", ui-sans-serif, sans-serif'
};

const CODE_FONT_STACKS: Record<CodeFontPreference, string> = {
  "system-mono": 'ui-monospace, "SFMono-Regular", Consolas, "Liberation Mono", monospace',
  "cascadia-code": '"Cascadia Code", "Cascadia Mono", Consolas, monospace',
  consolas: 'Consolas, "Courier New", monospace'
};

export interface ThemeTransitionOrigin {
  x: number;
  y: number;
}

export type ThemeTransitionPath =
  | "initial"
  | "same-scheme"
  | "reduced-motion"
  | "hidden"
  | "unsupported"
  | "transition"
  | "transition-error";

interface ThemeViewTransition {
  finished: Promise<unknown>;
  skipTransition?(): void;
}

type StartViewTransition = (update: () => void) => ThemeViewTransition;

export interface ThemeTransitionDependencies {
  getRoot(): HTMLElement;
  getViewport(): { width: number; height: number };
  getStartViewTransition(): StartViewTransition | undefined;
  prefersReducedMotion(): boolean;
  isDocumentVisible(): boolean;
}

export interface ThemeTransitionOptions {
  nextScheme: "light" | "dark";
  reduceMotion: boolean;
  origin?: ThemeTransitionOrigin;
}

function clearThemeTransitionStyles(root: HTMLElement): void {
  delete root.dataset.themeTransition;
  root.style.removeProperty("--theme-transition-x");
  root.style.removeProperty("--theme-transition-y");
  root.style.removeProperty("--theme-transition-radius");
}

export function createThemeTransitionCoordinator(dependencies: ThemeTransitionDependencies) {
  let generation = 0;
  let active:
    | { generation: number; root: HTMLElement; transition: ThemeViewTransition }
    | undefined;

  const stopActiveTransition = () => {
    if (!active) return;
    generation += 1;
    try {
      active.transition.skipTransition?.();
    } catch {
      // A skipped Chromium transition may already be finishing. Cleanup below is enough.
    }
    clearThemeTransitionStyles(active.root);
    active = undefined;
  };

  const apply = (update: () => void, options: ThemeTransitionOptions): ThemeTransitionPath => {
    const root = dependencies.getRoot();
    const currentScheme = root.dataset.theme;

    if (!active && currentScheme === options.nextScheme) {
      update();
      return "same-scheme";
    }

    stopActiveTransition();
    const requestGeneration = ++generation;

    if (currentScheme === undefined) {
      update();
      return "initial";
    }
    if (currentScheme === options.nextScheme) {
      update();
      return "same-scheme";
    }
    if (options.reduceMotion || dependencies.prefersReducedMotion()) {
      update();
      return "reduced-motion";
    }
    if (!dependencies.isDocumentVisible()) {
      update();
      return "hidden";
    }

    const startViewTransition = dependencies.getStartViewTransition();
    if (!startViewTransition) {
      update();
      return "unsupported";
    }

    const viewport = dependencies.getViewport();
    const origin = options.origin ?? {
      x: viewport.width / 2,
      y: viewport.height / 2
    };
    const x = Math.min(viewport.width, Math.max(0, origin.x));
    const y = Math.min(viewport.height, Math.max(0, origin.y));
    const radius =
      Math.hypot(
        Math.max(x, viewport.width - x),
        Math.max(y, viewport.height - y)
      ) + 96;

    root.dataset.themeTransition = "radial";
    root.style.setProperty("--theme-transition-x", `${x}px`);
    root.style.setProperty("--theme-transition-y", `${y}px`);
    root.style.setProperty("--theme-transition-radius", `${Math.ceil(radius)}px`);

    let applied = false;
    try {
      const transition = startViewTransition(() => {
        if (requestGeneration !== generation) return;
        applied = true;
        update();
      });
      active = { generation: requestGeneration, root, transition };
      void transition.finished
        .catch(() => {
          if (!applied && requestGeneration === generation) {
            applied = true;
            update();
          }
        })
        .finally(() => {
          if (requestGeneration !== generation) return;
          clearThemeTransitionStyles(root);
          active = undefined;
        });
      return "transition";
    } catch {
      if (!applied && requestGeneration === generation) update();
      clearThemeTransitionStyles(root);
      active = undefined;
      return "transition-error";
    }
  };

  return {
    apply,
    dispose: stopActiveTransition
  };
}

export const themeTransitionCoordinator = createThemeTransitionCoordinator({
  getRoot: () => document.documentElement,
  getViewport: () => ({ width: window.innerWidth, height: window.innerHeight }),
  getStartViewTransition: () => {
    if (typeof document.startViewTransition !== "function") return undefined;
    return (update) => document.startViewTransition(update);
  },
  prefersReducedMotion: () =>
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  isDocumentVisible: () => document.visibilityState !== "hidden"
});

export function prefersDarkColorScheme(): boolean {
  return typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)").matches
    : true;
}

export function applyDocumentPreferences(
  preferences: Readonly<UiPreferences>,
  systemPrefersDark = prefersDarkColorScheme()
): void {
  const root = document.documentElement;
  const scheme = effectiveColorScheme(preferences.colorScheme, systemPrefersDark);
  const theme = activeThemePreferences(preferences, systemPrefersDark);
  const colors = deriveThemeColors(theme);

  root.dataset.colorScheme = preferences.colorScheme;
  root.dataset.theme = scheme;
  root.dataset.reduceMotion = String(preferences.reduceMotion);
  root.dataset.translucentSidebar = String(theme.translucentSidebar);
  root.style.colorScheme = scheme;
  root.style.setProperty("--accent", colors.accent);
  root.style.setProperty("--border", colors.border);
  root.style.setProperty("--border-soft", colors.borderSoft);
  root.style.setProperty("--canvas", colors.canvas);
  root.style.setProperty("--code-font", CODE_FONT_STACKS[theme.codeFont]);
  root.style.setProperty("--menu", colors.menu);
  root.style.setProperty("--muted", colors.muted);
  root.style.setProperty("--muted-strong", colors.mutedStrong);
  root.style.setProperty("--panel", colors.panel);
  root.style.setProperty("--panel-hover", colors.panelHover);
  root.style.setProperty("--panel-raised", colors.panelRaised);
  root.style.setProperty("--panel-selected", colors.panelSelected);
  root.style.setProperty("--separator", colors.separator);
  root.style.setProperty("--shadow-color", colors.shadow);
  root.style.setProperty("--sidebar", colors.sidebar);
  root.style.setProperty("--sidebar-bottom", colors.sidebarBottom);
  root.style.setProperty("--sidebar-translucent", colors.sidebarTranslucent);
  root.style.setProperty("--surface-hover", colors.surfaceHover);
  root.style.setProperty("--text", colors.foreground);
  root.style.setProperty("--ui-font", UI_FONT_STACKS[theme.uiFont]);

  setUiLanguage(preferences.language);
}
