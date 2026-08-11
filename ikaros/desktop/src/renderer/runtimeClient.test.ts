import { describe, expect, it, vi } from "vitest";

import type { IkarosRuntimeBridgeApi } from "../shared/runtime";
import { RuntimeClient, RuntimeRpcError } from "./runtimeClient";

function bridgeWithListThreads(
  listThreads: IkarosRuntimeBridgeApi["listThreads"]
): IkarosRuntimeBridgeApi {
  return {
    listThreads,
    createThread: vi.fn(),
    startTurn: vi.fn(),
    cancelRun: vi.fn(),
    replayEvents: vi.fn(),
    onEvent: vi.fn(() => () => undefined)
  };
}

describe("RuntimeClient", () => {
  it("unwraps successful Electron bridge results", async () => {
    const threads = { threads: [] };
    const client = new RuntimeClient(
      bridgeWithListThreads(vi.fn(async () => ({ ok: true as const, value: threads })))
    );

    await expect(client.listThreads()).resolves.toEqual(threads);
  });

  it("reconstructs definitive RPC errors without reclassifying transport errors", async () => {
    const rpcClient = new RuntimeClient(
      bridgeWithListThreads(
        vi.fn(async () => ({
          ok: false as const,
          error: { kind: "json_rpc" as const, code: -32602, message: "invalid params" }
        }))
      )
    );
    let rejected: unknown;
    try {
      await rpcClient.listThreads();
    } catch (error) {
      rejected = error;
    }
    expect(rejected).toBeInstanceOf(RuntimeRpcError);
    expect(rejected).toMatchObject({
      kind: "json_rpc",
      code: -32602,
      message: "invalid params"
    });

    const transportError = new Error("Runtime WebSocket closed.");
    const transportClient = new RuntimeClient(
      bridgeWithListThreads(vi.fn(async () => Promise.reject(transportError)))
    );
    await expect(transportClient.listThreads()).rejects.toBe(transportError);
  });

  it.each([
    {},
    { ok: true },
    { ok: false },
    { ok: true, value: { threads: [] }, error: { kind: "json_rpc", code: -1, message: "invalid" } },
    { ok: false, value: { threads: [] }, error: { kind: "json_rpc", code: -1, message: "invalid" } },
    { ok: false, error: { kind: "json_rpc", code: Number.NaN, message: "invalid" } },
  ])("keeps malformed bridge envelopes ambiguous: %o", async (envelope) => {
    const listThreads = vi.fn(async () => envelope) as unknown as
      IkarosRuntimeBridgeApi["listThreads"];
    const client = new RuntimeClient(bridgeWithListThreads(listThreads));

    let rejected: unknown;
    try {
      await client.listThreads();
    } catch (error) {
      rejected = error;
    }
    expect(rejected).toEqual(
      new Error("Runtime bridge returned an invalid invocation result.")
    );
    expect(rejected).not.toBeInstanceOf(RuntimeRpcError);
  });
});
