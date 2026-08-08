import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  cloneUiPreferences,
  DEFAULT_UI_PREFERENCES,
  mergeUiPreferences,
  type IkarosDesktopApi,
  type UiPreferences,
  type UiPreferencesPatch
} from "../../shared/platform";
import { themeTransitionCoordinator } from "../applyUiPreferences";
import { setUiLanguage } from "../i18n";
import { useAppStore } from "../store";
import { SettingsPage } from "./SettingsPage";

const initialState = useAppStore.getState();

function desktopApiWithPreferences(
  updateImplementation?: (patch: UiPreferencesPatch) => Promise<UiPreferences>
): { api: IkarosDesktopApi; update: ReturnType<typeof vi.fn> } {
  let current = cloneUiPreferences(DEFAULT_UI_PREFERENCES);
  const update = vi.fn(
    updateImplementation ??
      (async (patch: UiPreferencesPatch) => {
        current = mergeUiPreferences(current, patch);
        return cloneUiPreferences(current);
      })
  );

  return {
    update,
    api: {
      preferences: {
        get: async () => cloneUiPreferences(current),
        update,
        onChanged: () => () => undefined
      }
    }
  };
}

beforeEach(() => {
  setUiLanguage("en");
  useAppStore.setState({ settingsOpen: true });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn((query: string) => ({
      matches: query === "(prefers-color-scheme: dark)",
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn()
    }))
  });
});

afterEach(() => {
  cleanup();
  themeTransitionCoordinator.dispose();
  Reflect.deleteProperty(document, "startViewTransition");
  Reflect.deleteProperty(window, "ikarosDesktop");
  Reflect.deleteProperty(window, "matchMedia");
  document.documentElement.removeAttribute("style");
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("data-color-scheme");
  document.documentElement.removeAttribute("data-translucent-sidebar");
  setUiLanguage("en");
  useAppStore.setState(initialState, true);
});

describe("SettingsPage", () => {
  it("keeps General and adds local Profile and Appearance sections", () => {
    render(<SettingsPage />);

    expect(screen.getByRole("heading", { level: 1, name: "General" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Profile" }));

    expect(screen.getByRole("heading", { level: 1, name: "Profile" })).toBeTruthy();
    expect(screen.getByText("HC")).toBeTruthy();
    expect(screen.getByText("hc")).toBeTruthy();
    expect(screen.queryByText(/@/)).toBeNull();
    expect(screen.queryByText("Free")).toBeNull();
    expect(screen.queryByText("Share")).toBeNull();
    expect(screen.queryByText("Private")).toBeNull();
    expect(screen.getByRole("button", { name: "Edit" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));

    expect(screen.getByRole("heading", { level: 1, name: "Appearance" })).toBeTruthy();
    expect(screen.getByRole("radiogroup", { name: "Theme" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "System" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "Light" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "Dark" }).getAttribute("aria-checked")).toBe(
      "true"
    );
    expect(screen.getByRole("slider", { name: "Contrast" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Translucent sidebar" })).toBeTruthy();
  });

  it("finds Profile through settings search without adding fake sections", () => {
    render(<SettingsPage />);

    fireEvent.change(screen.getByRole("searchbox", { name: "Search settings..." }), {
      target: { value: "profile" }
    });

    expect(screen.getByRole("button", { name: "Profile" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "General" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Appearance" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Account" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Usage & billing" })).toBeNull();
  });

  it("uses the normalized settings typography scale", () => {
    render(<SettingsPage />);

    const generalHeading = screen.getByRole("heading", { level: 1, name: "General" });
    expect(generalHeading.className).toContain("text-[20px]");
    expect(generalHeading.className).toContain("leading-[28px]");
    expect(screen.getByText("Language for the app UI").className).toContain("text-[11px]");
    expect(screen.getByRole("button", { name: "Choose UI language" }).className).toContain(
      "leading-[18px]"
    );

    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));

    const appearanceHeading = screen.getByRole("heading", { level: 1, name: "Appearance" });
    expect(appearanceHeading.className).toContain("text-[20px]");
    expect(appearanceHeading.className).toContain("leading-[28px]");
    expect(screen.getByText("Reduce interface animations and transitions").className).toContain(
      "text-[11px]"
    );

    const codeKeyword = screen.getAllByText("const")[0];
    const codePane = codeKeyword.parentElement?.parentElement?.parentElement;
    expect(codePane?.className).toContain("text-[11px]");
    expect(codePane?.className).toContain("leading-[20px]");

    const renderedClassNames = Array.from(document.querySelectorAll("[class]"), (node) =>
      node.getAttribute("class") ?? ""
    ).join(" ");
    expect(renderedClassNames).not.toMatch(/(?:text|leading)-\[(?:\d+\.\d+|23)px\]/);
  });

  it("applies Simplified Chinese immediately and persists it", async () => {
    const { api, update } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });

    render(<SettingsPage />);

    fireEvent.pointerDown(screen.getByRole("button", { name: "Choose UI language" }), {
      button: 0,
      ctrlKey: false
    });
    const chineseOption = await screen.findByRole("menuitemradio", { name: "简体中文" });
    fireEvent.click(chineseOption);

    expect(await screen.findByRole("heading", { level: 1, name: "常规" })).toBeTruthy();
    expect(screen.getByText("应用 UI 语言")).toBeTruthy();
    expect(document.documentElement.lang).toBe("zh-CN");
    await waitFor(() => expect(update).toHaveBeenCalledWith({ language: "zh-CN" }));
  });

  it("rolls back the UI language when persistence fails", async () => {
    const { api } = desktopApiWithPreferences(async () => {
      throw new Error("disk unavailable");
    });
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });

    render(<SettingsPage />);
    fireEvent.pointerDown(screen.getByRole("button", { name: "Choose UI language" }), {
      button: 0,
      ctrlKey: false
    });
    fireEvent.click(await screen.findByRole("menuitemradio", { name: "简体中文" }));

    expect(await screen.findByText("Could not save settings.")).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1, name: "General" })).toBeTruthy();
    expect(document.documentElement.lang).toBe("en");
  });

  it("persists theme selection and applies the light palette", async () => {
    const { api, update } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });
    render(<SettingsPage />);

    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));
    fireEvent.click(screen.getByRole("radio", { name: "Light" }));

    await waitFor(() => expect(update).toHaveBeenCalledWith({ colorScheme: "light" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.style.getPropertyValue("--canvas")).toBe("#f7f7f7");
  });

  it("starts the light-dark reveal from the clicked theme card", async () => {
    const { api } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });
    const startViewTransition = vi.fn((update: () => void) => {
      update();
      return {
        finished: new Promise(() => undefined),
        ready: Promise.resolve(),
        updateCallbackDone: Promise.resolve(),
        skipTransition: vi.fn()
      };
    });
    Object.defineProperty(document, "startViewTransition", {
      configurable: true,
      value: startViewTransition
    });
    render(<SettingsPage />);
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dark"));
    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));

    fireEvent.click(screen.getByRole("radio", { name: "Light" }), {
      clientX: 220,
      clientY: 300,
      detail: 1
    });

    expect(startViewTransition).toHaveBeenCalledOnce();
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.dataset.themeTransition).toBe("radial");
    expect(document.documentElement.style.getPropertyValue("--theme-transition-x")).toBe(
      "220px"
    );
    expect(document.documentElement.style.getPropertyValue("--theme-transition-y")).toBe(
      "300px"
    );
  });

  it("previews and persists an edited theme color", async () => {
    const { api, update } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));

    const accent = screen.getByLabelText("Choose Accent color");
    fireEvent.change(accent, { target: { value: "#ff0000" } });
    expect(document.documentElement.style.getPropertyValue("--accent")).toBe("#ff0000");
    fireEvent.blur(accent);

    await waitFor(() =>
      expect(update).toHaveBeenCalledWith(
        expect.objectContaining({ darkTheme: expect.objectContaining({ accent: "#ff0000" }) })
      )
    );
  });

  it("does not claim a preference was saved when the preload API is missing", async () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));
    fireEvent.click(screen.getByRole("radio", { name: "Light" }));

    expect(await screen.findByText("Could not save settings.")).toBeTruthy();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("returns to the current app state instead of opening a modal", () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Back to app" }));
    expect(useAppStore.getState().settingsOpen).toBe(false);
  });
});
