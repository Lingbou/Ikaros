import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  cloneUiPreferences,
  DEFAULT_UI_PREFERENCES,
  mergeUiPreferences,
  type UiPreferences,
} from "../shared/platform";
import { usePlatformPreferences } from "./platformPreferences";
import { useAppStore } from "./store";

const initialState = useAppStore.getState();

function Harness() {
  usePlatformPreferences();
  return null;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

beforeEach(() => {
  useAppStore.setState(
    {
      ...initialState,
      sidebarOpen: true,
      sidebarWidth: DEFAULT_UI_PREFERENCES.sidebarWidth,
      profileUsername: DEFAULT_UI_PREFERENCES.username,
    },
    true,
  );
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(() => ({
      matches: true,
      media: "(prefers-color-scheme: dark)",
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
});

afterEach(() => {
  cleanup();
  Reflect.deleteProperty(window, "ikarosDesktop");
  Reflect.deleteProperty(window, "matchMedia");
  useAppStore.setState(initialState, true);
});

describe("platform UI preference projection", () => {
  it("loads sidebar and username preferences without writing them back", async () => {
    const loaded = deferred<UiPreferences>();
    const update = vi.fn(async () => cloneUiPreferences(DEFAULT_UI_PREFERENCES));
    Object.defineProperty(window, "ikarosDesktop", {
      configurable: true,
      value: {
        preferences: {
          get: vi.fn(() => loaded.promise),
          update,
          onChanged: vi.fn(() => vi.fn()),
        },
      },
    });
    render(<Harness />);

    await act(async () => {
      loaded.resolve(
        mergeUiPreferences(DEFAULT_UI_PREFERENCES, {
          sidebarCollapsed: true,
          sidebarWidth: 412,
          username: "Nova Lane",
        }),
      );
      await loaded.promise;
    });

    expect(useAppStore.getState()).toMatchObject({
      sidebarOpen: false,
      sidebarWidth: 412,
      profileUsername: "Nova Lane",
    });
    expect(update).not.toHaveBeenCalled();
  });

  it("applies host preference changes to the sidebar width and username", async () => {
    let listener: ((preferences: UiPreferences) => void) | undefined;
    Object.defineProperty(window, "ikarosDesktop", {
      configurable: true,
      value: {
        preferences: {
          get: vi.fn(async () => cloneUiPreferences(DEFAULT_UI_PREFERENCES)),
          update: vi.fn(async () => cloneUiPreferences(DEFAULT_UI_PREFERENCES)),
          onChanged: vi.fn((nextListener: (preferences: UiPreferences) => void) => {
            listener = nextListener;
            return vi.fn();
          }),
        },
      },
    });
    render(<Harness />);
    await act(async () => Promise.resolve());

    act(() => {
      listener?.(
        mergeUiPreferences(DEFAULT_UI_PREFERENCES, {
          sidebarWidth: 480,
          username: "March Seven"
        })
      );
    });

    expect(useAppStore.getState().sidebarWidth).toBe(480);
    expect(useAppStore.getState().profileUsername).toBe("March Seven");
  });
});
