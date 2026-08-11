import { afterEach, describe, expect, it, vi } from "vitest";

import { initializeRuntimeWithRetry } from "./App";

afterEach(() => {
  vi.useRealTimers();
});

describe("Runtime initialization", () => {
  it("retries transient startup failures with bounded backoff", async () => {
    vi.useFakeTimers();
    let ready = false;
    const initialize = vi.fn(async () => {
      if (initialize.mock.calls.length === 3) {
        ready = true;
      }
    });
    const controller = new AbortController();

    const initialization = initializeRuntimeWithRetry(
      initialize,
      () => ready,
      controller.signal,
    );
    await vi.advanceTimersByTimeAsync(350);
    await initialization;

    expect(initialize).toHaveBeenCalledTimes(3);
    expect(ready).toBe(true);
  });

  it("stops after the bounded retry budget is exhausted", async () => {
    vi.useFakeTimers();
    const initialize = vi.fn(async () => undefined);
    const controller = new AbortController();

    const initialization = initializeRuntimeWithRetry(
      initialize,
      () => false,
      controller.signal,
    );
    await vi.advanceTimersByTimeAsync(1_000);
    await initialization;

    expect(initialize).toHaveBeenCalledTimes(4);
  });
});
