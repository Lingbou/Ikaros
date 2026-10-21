import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { IkarosRuntimeBridgeApi, RuntimeChannelSummary } from "../../shared/runtime";
import { setUiLanguage } from "../i18n";
import { TelegramSettings } from "./TelegramSettings";

// The renderer store builds its Runtime client when the module loads, which is
// before a test can install `window.ikarosDesktop`. Route every bridge call
// through a stable proxy that resolves to the current test's mock, so the store
// and the component observe the same Runtime.
const bridge = vi.hoisted(() => {
  const state: { current: Record<string, unknown> } = { current: {} };
  const runtime = new Proxy(
    {},
    {
      get: (_target, property: string) =>
        (...args: unknown[]) => {
          const method = state.current[property];
          return typeof method === "function"
            ? (method as (...methodArgs: unknown[]) => unknown)(...args)
            : undefined;
        }
    }
  );
  Object.defineProperty(window, "ikarosDesktop", {
    configurable: true,
    value: { runtime }
  });
  return { state, runtime };
});

const disabledChannel: RuntimeChannelSummary = {
  id: "telegram",
  status: "disabled",
  enabled: false,
  configured: false,
  proxyConfigured: false,
  defaultProviderId: null,
  defaultModelId: null,
  bot: null,
  owner: null,
  lastErrorCode: null
};

function ok<T>(value: T) {
  return Promise.resolve({ ok: true as const, value });
}

function installRuntime(overrides: Partial<IkarosRuntimeBridgeApi> = {}) {
  const runtime = {
    listChannels: vi.fn(() => ok({ channels: [disabledChannel] })),
    configureChannel: vi.fn(() =>
      ok({
        channel: {
          ...disabledChannel,
          enabled: true,
          configured: true
        }
      })
    ),
    testChannel: vi.fn(() =>
      ok({
        ok: true,
        reasonCode: null,
        bot: { id: "1", username: "ikaros_bot", name: "Ikaros" }
      })
    ),
    resetChannelBinding: vi.fn(() => ok({ channel: disabledChannel })),
    listProviders: vi.fn(() => ok({ providers: [] })),
    listModels: vi.fn(() => ok({ models: [] })),
    ...overrides
  } as unknown as IkarosRuntimeBridgeApi;
  bridge.state.current = runtime as unknown as Record<string, unknown>;
  Object.defineProperty(window, "ikarosDesktop", {
    configurable: true,
    value: { runtime: bridge.runtime }
  });
  return runtime;
}

beforeEach(() => setUiLanguage("en"));

afterEach(() => {
  cleanup();
  Reflect.deleteProperty(window, "ikarosDesktop");
  setUiLanguage("en");
});

describe("TelegramSettings", () => {
  it("loads the channel once per mount instead of hammering the runtime", async () => {
    const runtime = installRuntime();
    render(<TelegramSettings />);

    await screen.findByText(
      "All Telegram messages use the fixed Telegram thread in Recents. /new archives the old thread and starts another one called Telegram."
    );
    await new Promise((resolve) => setTimeout(resolve, 250));

    expect(runtime.listChannels).toHaveBeenCalledTimes(1);
    expect(runtime.listProviders).toHaveBeenCalledTimes(1);
    expect(runtime.listModels).toHaveBeenCalledTimes(1);
  });
  it("loads the fixed Telegram conversation and saves a token", async () => {
    const runtime = installRuntime();
    render(<TelegramSettings />);

    expect(await screen.findByText("All Telegram messages use the fixed Telegram thread in Recents. /new archives the old thread and starts another one called Telegram.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.change(screen.getByLabelText("Bot token"), {
      target: { value: "123456:test-token" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(runtime.configureChannel).toHaveBeenCalledWith({
        channelId: "telegram",
        enabled: true,
        token: "123456:test-token",
        defaultProviderId: null,
        defaultModelId: null
      });
    });
    expect(await screen.findByText("Telegram settings saved.")).toBeInTheDocument();
  });

  it("tests the bot token and reports the bot username", async () => {
    const runtime = installRuntime();
    render(<TelegramSettings />);
    await screen.findByText("All Telegram messages use the fixed Telegram thread in Recents. /new archives the old thread and starts another one called Telegram.");

    fireEvent.change(screen.getByLabelText("Bot token"), {
      target: { value: "123456:test-token" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));

    await waitFor(() => {
      expect(runtime.testChannel).toHaveBeenCalledWith({
        channelId: "telegram",
        proxy: undefined,
        token: "123456:test-token"
      });
    });
    expect(await screen.findByText("Connected as @ikaros_bot.")).toBeInTheDocument();
  });
});
