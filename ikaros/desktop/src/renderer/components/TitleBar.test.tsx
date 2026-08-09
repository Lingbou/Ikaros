import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUiLanguage } from "../i18n";
import { useAppStore } from "../store";
import { TitleBar } from "./TitleBar";

const initialState = useAppStore.getState();

afterEach(() => {
  cleanup();
  Reflect.deleteProperty(window, "ikarosDesktop");
  setUiLanguage("en");
  useAppStore.setState(initialState, true);
});

describe("Linux title-bar controls", () => {
  it("minimizes, maximizes, and closes through the desktop interface", () => {
    const minimize = vi.fn();
    const toggleMaximize = vi.fn();
    const close = vi.fn();
    Object.defineProperty(window, "ikarosDesktop", {
      configurable: true,
      value: {
        windowControls: {
          usesCustomTitleBar: true,
          close,
          minimize,
          toggleMaximize,
        },
      },
    });

    render(
      <Tooltip.Provider>
        <TitleBar />
      </Tooltip.Provider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Minimize" }));
    fireEvent.click(screen.getByRole("button", { name: "Maximize" }));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));

    expect(minimize).toHaveBeenCalledOnce();
    expect(toggleMaximize).toHaveBeenCalledOnce();
    expect(close).toHaveBeenCalledOnce();
  });

  it("toggles maximize when the empty title bar is double-clicked", () => {
    const toggleMaximize = vi.fn();
    Object.defineProperty(window, "ikarosDesktop", {
      configurable: true,
      value: {
        windowControls: {
          usesCustomTitleBar: true,
          close: vi.fn(),
          minimize: vi.fn(),
          toggleMaximize,
        },
      },
    });

    render(
      <Tooltip.Provider>
        <TitleBar />
      </Tooltip.Provider>,
    );
    fireEvent.doubleClick(screen.getByRole("banner"));

    expect(toggleMaximize).toHaveBeenCalledOnce();
  });
});
