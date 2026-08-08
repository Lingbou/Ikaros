import { useEffect } from "react";

import type { UiPreferences } from "../shared/platform";
import { effectiveColorScheme } from "../shared/theme";
import { applyDocumentPreferences, themeTransitionCoordinator } from "./applyUiPreferences";
import { useAppStore } from "./store";

export function usePlatformPreferences() {
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);

  useEffect(() => {
    const api = window.ikarosDesktop;
    if (!api) return;

    let disposed = false;
    let initialized = false;
    let applyingHostValue = false;
    let currentPreferences: UiPreferences | undefined;
    const colorSchemeQuery =
      typeof window.matchMedia === "function"
        ? window.matchMedia("(prefers-color-scheme: dark)")
        : undefined;

    const applyPreferences = (preferences: UiPreferences) => {
      if (disposed) return;
      currentPreferences = preferences;
      applyingHostValue = true;
      applyDocumentPreferences(preferences, colorSchemeQuery?.matches ?? true);
      setSidebarOpen(!preferences.sidebarCollapsed);
      applyingHostValue = false;
      initialized = true;
    };

    void api.preferences.get().then(applyPreferences).catch(() => {
      initialized = true;
    });

    const unsubscribeHost = api.preferences.onChanged(applyPreferences);
    const onSystemSchemeChanged = () => {
      if (currentPreferences?.colorScheme === "system") {
        const systemPrefersDark = colorSchemeQuery?.matches ?? true;
        themeTransitionCoordinator.apply(
          () => applyDocumentPreferences(currentPreferences!, systemPrefersDark),
          {
            nextScheme: effectiveColorScheme(currentPreferences.colorScheme, systemPrefersDark),
            reduceMotion: currentPreferences.reduceMotion
          }
        );
      }
    };
    colorSchemeQuery?.addEventListener("change", onSystemSchemeChanged);
    const unsubscribeStore = useAppStore.subscribe((state, previous) => {
      if (
        disposed ||
        !initialized ||
        applyingHostValue ||
        state.sidebarOpen === previous.sidebarOpen
      ) {
        return;
      }
      void api.preferences
        .update({ sidebarCollapsed: !state.sidebarOpen })
        .catch(() => undefined);
    });

    return () => {
      disposed = true;
      unsubscribeHost();
      unsubscribeStore();
      colorSchemeQuery?.removeEventListener("change", onSystemSchemeChanged);
    };
  }, [setSidebarOpen]);
}
