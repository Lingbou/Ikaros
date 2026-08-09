import { beforeEach, describe, expect, it, vi } from "vitest";

const electron = vi.hoisted(() => {
  const window = {
    loadURL: vi.fn(),
    once: vi.fn(),
    setBackgroundColor: vi.fn(),
    setTitleBarOverlay: vi.fn(),
  };
  return {
    app: {
      getAppPath: vi.fn(() => "/opt/ikaros/resources/app.asar"),
      isPackaged: true,
    },
    browserWindow: vi.fn(function BrowserWindow(_options: Record<string, unknown>) {
      return window;
    }),
    nativeTheme: { shouldUseDarkColors: true },
    window,
  };
});

vi.mock("electron", () => ({
  app: electron.app,
  BrowserWindow: electron.browserWindow,
  nativeTheme: electron.nativeTheme,
}));

vi.mock("./security", () => ({
  installWindowSecurity: vi.fn(),
}));

import { DEFAULT_UI_PREFERENCES } from "../shared/platform";
import { createMainWindow } from "./window";

describe("Linux desktop window", () => {
  beforeEach(() => {
    vi.spyOn(process, "platform", "get").mockReturnValue("linux");
  });

  it("uses custom chrome without a native title-bar icon", () => {
    createMainWindow(
      "file:///renderer/index.html",
      { assertTrustedIpc: vi.fn(), isTrustedUrl: vi.fn(() => true) },
      DEFAULT_UI_PREFERENCES,
    );

    const options = electron.browserWindow.mock.calls[0]?.[0];
    expect(options).toEqual(
      expect.objectContaining({
        frame: false,
        titleBarStyle: "hidden",
      }),
    );
    expect(options).not.toHaveProperty("icon");
  });
});
