import { describe, expect, it, vi } from "vitest";

import type {
  IkarosRuntimeBridgeApi,
  RuntimeFileChangeResult,
  RuntimeFilePreviewResult,
} from "../shared/runtime";
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
    steerRun: vi.fn(),
    readProcess: vi.fn(),
    stopProcess: vi.fn(),
    replayEvents: vi.fn(),
    listProviders: vi.fn(),
    configureProvider: vi.fn(),
    discoverProviderModels: vi.fn(),
    disconnectProvider: vi.fn(),
    removeProvider: vi.fn(),
    listModels: vi.fn(),
    setModelLimits: vi.fn(),
    setModelEnabled: vi.fn(),
    listSkills: vi.fn(),
    setSkillEnabled: vi.fn(),
    createMemory: vi.fn(),
    correctMemory: vi.fn(),
    forgetMemory: vi.fn(),
    listMemories: vi.fn(),
    getMemory: vi.fn(),
    readUsage: vi.fn(),
    previewFile: vi.fn(),
    getFileChange: vi.fn(),
    onEvent: vi.fn(() => () => undefined),
    onStatus: vi.fn(() => () => undefined)
  };
}

describe("RuntimeClient", () => {
  it("forwards current-file pages and historical change queries without caching", async () => {
    const bridge = bridgeWithListThreads(vi.fn());
    const firstPage: RuntimeFilePreviewResult = {
      threadId: "thread-1",
      path: "/workspace/report.md",
      status: "text",
      revision: "a".repeat(64),
      encoding: "utf-8",
      bom: false,
      content: "First line\r\n",
      lineStart: 1,
      lineEnd: 1,
      nextOffset: 2,
      truncated: true,
      truncationReason: "byte_limit",
    };
    const changed: RuntimeFilePreviewResult = {
      threadId: "thread-1",
      path: "/workspace/report.md",
      status: "unavailable",
      reason: "revision_changed",
    };
    bridge.previewFile = vi.fn<IkarosRuntimeBridgeApi["previewFile"]>()
      .mockResolvedValueOnce({ ok: true, value: firstPage })
      .mockResolvedValueOnce({ ok: true, value: changed });
    const change: RuntimeFileChangeResult = {
      threadId: "thread-1",
      toolCallItemId: "call-1",
      path: "/workspace/report.md",
      operation: "write",
      recordedAt: "2026-09-05T00:00:00Z",
      before: {
        exists: true, byteCount: 6, revision: "b".repeat(64),
        encoding: "utf-8", bom: false, newline: "lf", lineCount: 1,
      },
      after: {
        exists: true, byteCount: 6, revision: "c".repeat(64),
        encoding: "utf-8", bom: false, newline: "lf", lineCount: 1,
      },
      status: "recorded",
      diff: "--- before\n+++ after\n@@ -1 +1 @@\n-alpha\n+omega\n",
      additions: 1,
      deletions: 1,
    };
    bridge.getFileChange = vi.fn(async () => ({ ok: true as const, value: change }));
    const client = new RuntimeClient(bridge);
    const params = { threadId: "thread-1", path: "report.md", sourceToolCallItemId: "call-1" };
    await expect(client.previewFile(params)).resolves.toEqual(firstPage);
    const laterParams = { ...params, offset: 2, expectedRevision: firstPage.revision };
    await expect(client.previewFile(laterParams)).resolves.toEqual(changed);
    expect(bridge.previewFile).toHaveBeenNthCalledWith(1, params);
    expect(bridge.previewFile).toHaveBeenNthCalledWith(2, laterParams);

    const changeParams = { threadId: "thread-1", toolCallItemId: "call-1" };
    await expect(client.getFileChange(changeParams)).resolves.toEqual(change);
    expect(bridge.getFileChange).toHaveBeenCalledWith(changeParams);
  });

  it("keeps a protected preview unavailable and rejects file binding RPC errors", async () => {
    const bridge = bridgeWithListThreads(vi.fn());
    const unavailable: RuntimeFilePreviewResult = {
      threadId: "thread-1", path: null, status: "unavailable", reason: "protected_content",
    };
    bridge.previewFile = vi.fn(async () => ({ ok: true as const, value: unavailable }));
    bridge.getFileChange = vi.fn(async () => ({
      ok: false as const,
      error: { kind: "json_rpc" as const, code: -32602, message: "invalid file binding" },
    }));
    const client = new RuntimeClient(bridge);
    await expect(client.previewFile({ threadId: "thread-1", path: "report.md" })).resolves.toEqual(unavailable);
    await expect(client.getFileChange({ threadId: "thread-1", toolCallItemId: "foreign-call" }))
      .rejects.toMatchObject({ code: -32602, message: "invalid file binding" });
  });

  it("unwraps successful Electron bridge results", async () => {
    const threads = {
      threads: [],
      nextCursor: null,
      hasMore: false,
      snapshotSeq: 0,
    };
    const client = new RuntimeClient(
      bridgeWithListThreads(vi.fn(async () => ({ ok: true as const, value: threads })))
    );

    await expect(client.listThreads()).resolves.toEqual(threads);
  });

  it("forwards archived catalog and Thread lifecycle mutations", async () => {
    const listThreads = vi.fn(async () => ({
      ok: true as const,
      value: { threads: [], nextCursor: null, hasMore: false, snapshotSeq: 4 },
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
      nextCursor: null,
      hasMore: false,
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
      vi.fn(async () => ({
        ok: true as const,
        value: { threads: [], nextCursor: null, hasMore: false, snapshotSeq: 0 },
      })),
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
      vi.fn(async () => ({
        ok: true as const,
        value: { threads: [], nextCursor: null, hasMore: false, snapshotSeq: 5 },
      })),
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
      vi.fn(async () => ({
        ok: true as const,
        value: { threads: [], nextCursor: null, hasMore: false, snapshotSeq: 0 },
      }))
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
      models: [{ id: "deepseek-chat", displayName: "DeepSeek Chat", contextWindow: 32768 }]
    };

    await expect(client.configureProvider(params)).resolves.toEqual({ provider });
    expect(bridge.configureProvider).toHaveBeenCalledWith(params);
    expect(JSON.stringify(await client.configureProvider(params))).not.toContain(
      "write-only-secret"
    );
  });

  it("forwards write-only discovery credentials and unwraps only discovered models", async () => {
    const bridge = bridgeWithListThreads(
      vi.fn(async () => ({
        ok: true as const,
        value: { threads: [], nextCursor: null, hasMore: false, snapshotSeq: 0 },
      }))
    );
    const models = [
      { id: "deepseek-v4-flash", displayName: "DeepSeek V4 Flash", contextWindow: 32768 },
      { id: "deepseek-v4-pro", displayName: "DeepSeek V4 Pro", contextWindow: 32768 }
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

  it("unwraps aggregate token usage without scanning loaded Thread history", async () => {
    const bridge = bridgeWithListThreads(
      vi.fn(async () => ({
        ok: true as const,
        value: { threads: [], nextCursor: null, hasMore: false, snapshotSeq: 0 },
      }))
    );
    const usage = {
      summary: {
        lifetimeTokens: 2_000,
        peakDailyTokens: 1_500,
        longestRunningTurnSec: 125,
        currentStreakDays: 1,
        longestStreakDays: 3
      },
      dailyUsageBuckets: [{ startDate: "2026-08-15", tokens: 1_500 }]
    };
    bridge.readUsage = vi.fn(async () => ({ ok: true as const, value: usage }));
    const client = new RuntimeClient(bridge);

    await expect(client.readUsage()).resolves.toEqual(usage);
    expect(bridge.readUsage).toHaveBeenCalledOnce();
  });

  it("forwards the Skills catalog and global enablement toggle", async () => {
    const bridge = bridgeWithListThreads(
      vi.fn(async () => ({
        ok: true as const,
        value: { threads: [], nextCursor: null, hasMore: false, snapshotSeq: 0 }
      }))
    );
    const skill = {
      name: "demo",
      description: "Demo Skill",
      location: "C:/Users/demo/.ikaros/skills/demo/SKILL.md",
      enabled: true
    };
    bridge.listSkills = vi.fn(async () => ({
      ok: true as const,
      value: { skills: [skill], diagnostics: [] }
    }));
    bridge.setSkillEnabled = vi.fn(async () => ({
      ok: true as const,
      value: { skill: { ...skill, enabled: false } }
    }));
    const client = new RuntimeClient(bridge);

    await expect(client.listSkills()).resolves.toEqual({ skills: [skill], diagnostics: [] });
    await expect(
      client.setSkillEnabled({ name: "demo", enabled: false })
    ).resolves.toEqual({ skill: { ...skill, enabled: false } });
    expect(bridge.setSkillEnabled).toHaveBeenCalledWith({ name: "demo", enabled: false });
  });

  it("forwards explicit Memory lifecycle operations without caching", async () => {
    const bridge = bridgeWithListThreads(
      vi.fn(async () => ({
        ok: true as const,
        value: { threads: [], nextCursor: null, hasMore: false, snapshotSeq: 0 }
      }))
    );
    const memoryId = `memory_${"1".repeat(32)}`;
    const scope = { type: "global" as const, key: null };
    const createParams = {
      kind: "preference" as const,
      scope,
      content: "The user prefers concise answers.",
      clientRequestId: "desktop-memory-create",
      source: { type: "session_item" as const, itemId: `item_${"2".repeat(32)}` }
    };
    const created = { memoryId, resultingRevision: 1, created: true };
    const correctParams = {
      memoryId,
      expectedRevision: 1,
      content: "The user prefers very concise answers.",
      clientRequestId: "desktop-memory-correct"
    };
    const corrected = { memoryId, resultingRevision: 2, created: true };
    const forgetParams = {
      memoryId,
      expectedRevision: 2,
      clientRequestId: "desktop-memory-forget"
    };
    const forgotten = { memoryId, resultingRevision: 3, created: true };
    const page = { memories: [], nextCursor: null, hasMore: false };
    const record = { memory: { id: memoryId, content: createParams.content } };
    bridge.createMemory = vi.fn(async () => ({ ok: true as const, value: created }));
    bridge.correctMemory = vi.fn(async () => ({ ok: true as const, value: corrected }));
    bridge.forgetMemory = vi.fn(async () => ({ ok: true as const, value: forgotten }));
    bridge.listMemories = vi.fn(async () => ({ ok: true as const, value: page }));
    bridge.getMemory = vi.fn(async () => ({ ok: true as const, value: record })) as never;
    const client = new RuntimeClient(bridge);

    await expect(client.createMemory(createParams)).resolves.toEqual(created);
    await expect(client.correctMemory(correctParams)).resolves.toEqual(corrected);
    await expect(client.forgetMemory(forgetParams)).resolves.toEqual(forgotten);
    await expect(client.listMemories({ scope, limit: 25 })).resolves.toEqual(page);
    await expect(client.getMemory(memoryId)).resolves.toEqual(record);
    expect(bridge.createMemory).toHaveBeenCalledWith(createParams);
    expect(bridge.correctMemory).toHaveBeenCalledWith(correctParams);
    expect(bridge.forgetMemory).toHaveBeenCalledWith(forgetParams);
    expect(bridge.listMemories).toHaveBeenCalledWith({ scope, limit: 25 });
    expect(bridge.getMemory).toHaveBeenCalledWith(memoryId);
  });

  it("preserves a validated Memory failure reason without exposing arbitrary data", async () => {
    const bridge = bridgeWithListThreads(vi.fn());
    bridge.correctMemory = vi.fn(async () => ({
      ok: false as const,
      error: {
        kind: "json_rpc" as const,
        code: -32020,
        message: "memory operation failed",
        reasonCode: "memory_revision_conflict" as const
      }
    }));
    const client = new RuntimeClient(bridge);
    const params = {
      memoryId: `memory_${"1".repeat(32)}`,
      expectedRevision: 1,
      content: "Updated",
      clientRequestId: "memory-correct-conflict"
    };

    await expect(client.correctMemory(params)).rejects.toMatchObject({
      kind: "json_rpc",
      code: -32020,
      message: "memory operation failed",
      reasonCode: "memory_revision_conflict"
    });
  });

  it.each([
    { code: -32020, message: "memory operation failed" },
    { code: -32020, message: "memory operation failed", reasonCode: "unknown" },
    {
      code: -32020,
      message: "memory operation failed",
      reasonCode: "memory_source_unavailable"
    },
    { code: -32602, message: "invalid params", reasonCode: "memory_not_found" },
    {
      code: -32020,
      message: "memory operation failed",
      reasonCode: "memory_not_found",
      data: { secret: "must not cross IPC" }
    }
  ])("rejects a malformed Memory bridge failure as ambiguous: %o", async (error) => {
    const bridge = bridgeWithListThreads(vi.fn());
    bridge.getMemory = vi.fn(async () => ({
      ok: false,
      error: { kind: "json_rpc", ...error }
    })) as IkarosRuntimeBridgeApi["getMemory"];
    const client = new RuntimeClient(bridge);

    await expect(client.getMemory(`memory_${"1".repeat(32)}`)).rejects.toEqual(
      new Error("Runtime bridge returned an invalid invocation result.")
    );
  });

  it.each([
    {},
    { ok: true },
    { ok: false },
    { ok: true, value: { threads: [] }, error: { kind: "json_rpc", code: -1, message: "invalid" } },
    { ok: false, value: { threads: [] }, error: { kind: "json_rpc", code: -1, message: "invalid" } },
    { ok: false, error: { kind: "json_rpc", code: Number.NaN, message: "invalid" } },
    {
      ok: false,
      error: {
        kind: "json_rpc",
        code: -32020,
        message: "memory operation failed",
        reasonCode: "memory_not_found"
      }
    }
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
