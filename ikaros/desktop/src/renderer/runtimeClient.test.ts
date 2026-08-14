import { describe, expect, it, vi } from "vitest";

import type { IkarosRuntimeBridgeApi } from "../shared/runtime";
import { RuntimeClient, RuntimeRpcError } from "./runtimeClient";

function bridgeWithListThreads(
  listThreads: IkarosRuntimeBridgeApi["listThreads"]
): IkarosRuntimeBridgeApi {
  return {
    listThreads,
    getThread: vi.fn(),
    listTurns: vi.fn(),
    createThread: vi.fn(),
    renameThread: vi.fn(),
    archiveThread: vi.fn(),
    unarchiveThread: vi.fn(),
    startTurn: vi.fn(),
    cancelRun: vi.fn(),
    replayEvents: vi.fn(),
    listProviders: vi.fn(),
    configureProvider: vi.fn(),
    discoverProviderModels: vi.fn(),
    disconnectProvider: vi.fn(),
    removeProvider: vi.fn(),
    listModels: vi.fn(),
    setModelEnabled: vi.fn(),
    onEvent: vi.fn(() => () => undefined)
  };
}

describe("RuntimeClient", () => {
  it("unwraps successful Electron bridge results", async () => {
    const threads = { threads: [], snapshotSeq: 0 };
    const client = new RuntimeClient(
      bridgeWithListThreads(vi.fn(async () => ({ ok: true as const, value: threads })))
    );

    await expect(client.listThreads()).resolves.toEqual(threads);
  });

  it("forwards archived catalog and Thread lifecycle mutations", async () => {
    const listThreads = vi.fn(async () => ({
      ok: true as const,
      value: { threads: [], snapshotSeq: 4 },
    }));
    const bridge = bridgeWithListThreads(listThreads);
    const mutation = {
      thread: { id: "thread-1" },
      changed: true,
      event: { seq: 4 },
    };
    bridge.renameThread = vi.fn(async () => ({ ok: true as const, value: mutation })) as never;
    bridge.archiveThread = vi.fn(async () => ({ ok: true as const, value: mutation })) as never;
    bridge.unarchiveThread = vi.fn(async () => ({ ok: true as const, value: mutation })) as never;
    const client = new RuntimeClient(bridge);

    await expect(client.listThreads({ archived: true })).resolves.toEqual({
      threads: [],
      snapshotSeq: 4,
    });
    await expect(
      client.renameThread({ threadId: "thread-1", title: "Renamed" }),
    ).resolves.toEqual(mutation);
    await expect(client.archiveThread("thread-1")).resolves.toEqual(mutation);
    await expect(client.unarchiveThread("thread-1")).resolves.toEqual(mutation);
    expect(listThreads).toHaveBeenCalledWith({ archived: true });
    expect(bridge.renameThread).toHaveBeenCalledWith({
      threadId: "thread-1",
      title: "Renamed",
    });
    expect(bridge.archiveThread).toHaveBeenCalledWith("thread-1");
    expect(bridge.unarchiveThread).toHaveBeenCalledWith("thread-1");
  });

  it("forwards the Thread workspace snapshot without flattening it", async () => {
    const bridge = bridgeWithListThreads(
      vi.fn(async () => ({ ok: true as const, value: { threads: [], snapshotSeq: 0 } })),
    );
    const value = { thread: { id: "thread-1" }, event: { seq: 1 } };
    bridge.createThread = vi.fn(async () => ({ ok: true as const, value })) as never;
    const client = new RuntimeClient(bridge);
    const params = {
      title: "Workspace chat",
      workspace: { id: "workspace-1", name: "Ikaros", rootUri: "C:\\Ikaros" },
      clientRequestId: "create-1",
    };

    await expect(client.createThread(params)).resolves.toEqual(value);
    expect(bridge.createThread).toHaveBeenCalledWith(params);
  });

  it("forwards Thread metadata and paginated Turn history reads", async () => {
    const bridge = bridgeWithListThreads(
      vi.fn(async () => ({ ok: true as const, value: { threads: [], snapshotSeq: 5 } })),
    );
    const thread = {
      id: "thread-1",
      title: "History",
      defaultBranchId: "branch-1",
      workspace: null,
      createdAt: "2026-08-14T00:00:00.000Z",
      updatedAt: "2026-08-14T00:00:00.000Z",
      archivedAt: null,
    };
    const metadata = { thread, snapshotSeq: 5 };
    const history = { turns: [], nextCursor: null, hasMore: false, snapshotSeq: 5 };
    bridge.getThread = vi.fn(async () => ({ ok: true as const, value: metadata }));
    bridge.listTurns = vi.fn(async () => ({ ok: true as const, value: history }));
    const client = new RuntimeClient(bridge);
    const params = { threadId: thread.id, branchId: thread.defaultBranchId, limit: 50 };

    await expect(client.getThread(thread.id)).resolves.toEqual(metadata);
    await expect(client.listTurns(params)).resolves.toEqual(history);
    expect(bridge.getThread).toHaveBeenCalledWith(thread.id);
    expect(bridge.listTurns).toHaveBeenCalledWith(params);
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

  it("forwards write-only provider configuration and unwraps the redacted result", async () => {
    const bridge = bridgeWithListThreads(
      vi.fn(async () => ({ ok: true as const, value: { threads: [], snapshotSeq: 0 } }))
    );
    const provider = {
      id: "deepseek",
      displayName: "DeepSeek",
      origin: "builtin" as const,
      configured: true,
      credentialConfigured: true,
      health: "unknown" as const
    };
    bridge.configureProvider = vi.fn(async () => ({
      ok: true as const,
      value: { provider }
    }));
    const client = new RuntimeClient(bridge);
    const params = {
      kind: "deepseek" as const,
      apiKey: "write-only-secret",
      models: [{ id: "deepseek-chat", displayName: "DeepSeek Chat" }]
    };

    await expect(client.configureProvider(params)).resolves.toEqual({ provider });
    expect(bridge.configureProvider).toHaveBeenCalledWith(params);
    expect(JSON.stringify(await client.configureProvider(params))).not.toContain(
      "write-only-secret"
    );
  });

  it("forwards write-only discovery credentials and unwraps only discovered models", async () => {
    const bridge = bridgeWithListThreads(
      vi.fn(async () => ({ ok: true as const, value: { threads: [], snapshotSeq: 0 } }))
    );
    const models = [
      { id: "deepseek-v4-flash", displayName: "DeepSeek V4 Flash" },
      { id: "deepseek-v4-pro", displayName: "DeepSeek V4 Pro" }
    ];
    bridge.discoverProviderModels = vi.fn(async () => ({
      ok: true as const,
      value: { models }
    }));
    const client = new RuntimeClient(bridge);
    const params = { kind: "deepseek" as const, apiKey: "write-only-discovery-secret" };

    await expect(client.discoverProviderModels(params)).resolves.toEqual({ models });
    expect(bridge.discoverProviderModels).toHaveBeenCalledWith(params);
    expect(JSON.stringify(await client.discoverProviderModels(params))).not.toContain(
      "write-only-discovery-secret"
    );
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
