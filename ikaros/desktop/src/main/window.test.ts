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
    browserWindow: vi.fn(function BrowserWindow() {
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

  it("uses the packaged Ikaros icon", () => {
    createMainWindow(
      "file:///renderer/index.html",
      { assertTrustedIpc: vi.fn(), isTrustedUrl: vi.fn(() => true) },
      DEFAULT_UI_PREFERENCES,
    );

    expect(electron.browserWindow).toHaveBeenCalledWith(
      expect.objectContaining({
        icon: "/opt/ikaros/resources/app.asar/build/icon.png",
      }),
    );
  });
});
