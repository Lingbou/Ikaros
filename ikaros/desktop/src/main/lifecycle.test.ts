// @vitest-environment node

import { describe, expect, it, vi } from "vitest";

import { deferQuitForRuntimeShutdown } from "./lifecycle";

describe("desktop shutdown lifecycle", () => {
  it("defers quit for an existing RuntimeHost even when it has no child", async () => {
    const event = { preventDefault: vi.fn() };
    const runtimeHost = { stop: vi.fn(async () => undefined) };
    const quit = vi.fn();

    const state = deferQuitForRuntimeShutdown(event, runtimeHost, "idle", quit);

    expect(state).toBe("pending");
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(runtimeHost.stop).toHaveBeenCalledOnce();
    await vi.waitFor(() => expect(quit).toHaveBeenCalledOnce());
  });

  it("keeps repeated before-quit events deferred while shutdown is pending", async () => {
    let finishStop: (() => void) | undefined;
    const runtimeHost = {
      stop: vi.fn(
        () =>
          new Promise<void>((resolve) => {
            finishStop = resolve;
          })
      )
    };
    const quit = vi.fn();
    const firstEvent = { preventDefault: vi.fn() };
    const secondEvent = { preventDefault: vi.fn() };

    const pending = deferQuitForRuntimeShutdown(firstEvent, runtimeHost, "idle", quit);
    const stillPending = deferQuitForRuntimeShutdown(
      secondEvent,
      runtimeHost,
      pending,
      quit
    );

    expect(pending).toBe("pending");
    expect(stillPending).toBe("pending");
    expect(firstEvent.preventDefault).toHaveBeenCalledOnce();
    expect(secondEvent.preventDefault).toHaveBeenCalledOnce();
    expect(runtimeHost.stop).toHaveBeenCalledOnce();
    expect(quit).not.toHaveBeenCalled();

    finishStop?.();
    await vi.waitFor(() => expect(quit).toHaveBeenCalledOnce());
  });

  it("allows the final before-quit event after Runtime shutdown settles", () => {
    const event = { preventDefault: vi.fn() };
    const runtimeHost = { stop: vi.fn(async () => undefined) };
    const quit = vi.fn();

    const state = deferQuitForRuntimeShutdown(event, runtimeHost, "ready", quit);

    expect(state).toBe("ready");
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(runtimeHost.stop).not.toHaveBeenCalled();
    expect(quit).not.toHaveBeenCalled();
  });
});
