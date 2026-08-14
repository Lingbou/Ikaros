/// <reference types="node" />

import { randomBytes } from "node:crypto";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import { listAllRuntimeThreads, RuntimeHost, RuntimeRpcError } from "../main/runtimeHost";
import type { IkarosDesktopApi } from "../shared/platform";
import type {
  IkarosRuntimeBridgeApi,
  RuntimeCancelRunResult,
  RuntimeInvocationResult,
  RuntimeJournalEvent,
  RuntimeModelSetEnabledResult,
  RuntimeProviderConfigureResult,
  RuntimeProviderDiscoverModelsResult,
  RuntimeProviderRemoveResult,
  RuntimeReplayResult,
  RuntimeThreadCreateResult,
  RuntimeThreadGetResult,
  RuntimeThreadMutationResult,
  RuntimeTurnListPage,
  RuntimeTurnStartResult,
  RuntimeUsageReadResult,
} from "../shared/runtime";
import { activeBranch, type AgentEvent, type Thread, type Turn } from "./domain";

const liveEnabled = process.env.IKAROS_LIVE_DEEPSEEK_SMOKE === "1";
const runtimeRoot = resolve(process.cwd(), "..", "..", "runtime");

async function bridgeInvocation<TResult>(
  operation: () => Promise<TResult>,
): Promise<RuntimeInvocationResult<TResult>> {
  try {
    return { ok: true, value: await operation() };
  } catch (error) {
    if (error instanceof RuntimeRpcError) {
      return {
        ok: false,
        error: { kind: "json_rpc", code: error.code, message: error.message },
      };
    }
    throw error;
  }
}

function runtimeBridge(
  host: RuntimeHost,
  cancellationResults: RuntimeCancelRunResult[],
): IkarosRuntimeBridgeApi {
  return {
    listThreads: () => bridgeInvocation(() => listAllRuntimeThreads(host)),
    getThread: (threadId) =>
      bridgeInvocation(() =>
        host.request<RuntimeThreadGetResult>("thread.get", { threadId }),
      ),
    listTurns: (params) =>
      bridgeInvocation(() =>
        host.request<RuntimeTurnListPage>("turn.list", { ...params }),
      ),
    createThread: (params) =>
      bridgeInvocation(() =>
        host.request<RuntimeThreadCreateResult>("thread.create", { ...params }),
      ),
    renameThread: (params) =>
      bridgeInvocation(() =>
        host.request<RuntimeThreadMutationResult>("thread.rename", { ...params }),
      ),
    archiveThread: (threadId) =>
      bridgeInvocation(() =>
        host.request<RuntimeThreadMutationResult>("thread.archive", { threadId }),
      ),
    unarchiveThread: (threadId) =>
      bridgeInvocation(() =>
        host.request<RuntimeThreadMutationResult>("thread.unarchive", { threadId }),
      ),
    startTurn: (params) =>
      bridgeInvocation(() => host.request<RuntimeTurnStartResult>("turn.start", { ...params })),
    cancelRun: (runId) =>
      bridgeInvocation(async () => {
        const result = await host.request<RuntimeCancelRunResult>("run.cancel", { runId });
        cancellationResults.push(result);
        return result;
      }),
    replayEvents: (afterSeq, limit) =>
      bridgeInvocation(() =>
        host.request<RuntimeReplayResult>("event.replay", {
          afterSeq,
          ...(limit === undefined ? {} : { limit }),
        }),
      ),
    listProviders: () =>
      bridgeInvocation(() =>
        host.request<Awaited<ReturnType<IkarosRuntimeBridgeApi["listProviders"]>> extends {
          ok: true;
          value: infer TValue;
        }
          ? TValue
          : never>("provider.list"),
      ),
    configureProvider: (params) =>
      bridgeInvocation(() =>
        host.request<RuntimeProviderConfigureResult>("provider.configure", { ...params }),
      ),
    discoverProviderModels: (params) =>
      bridgeInvocation(() =>
        host.request<RuntimeProviderDiscoverModelsResult>("provider.discover_models", {
          ...params,
        }),
      ),
    disconnectProvider: (providerId) =>
      bridgeInvocation(() =>
        host.request<RuntimeProviderConfigureResult>("provider.disconnect", { providerId }),
      ),
    removeProvider: (providerId) =>
      bridgeInvocation(() =>
        host.request<RuntimeProviderRemoveResult>("provider.remove", { providerId }),
      ),
    listModels: () =>
      bridgeInvocation(() =>
        host.request<Awaited<ReturnType<IkarosRuntimeBridgeApi["listModels"]>> extends {
          ok: true;
          value: infer TValue;
        }
          ? TValue
          : never>("model.list"),
      ),
    setModelEnabled: (params) =>
      bridgeInvocation(() =>
        host.request<RuntimeModelSetEnabledResult>("model.set_enabled", { ...params }),
      ),
    readUsage: () =>
      bridgeInvocation(() => host.request<RuntimeUsageReadResult>("usage.read")),
    onEvent: (listener) =>
      host.onNotification((notification) => {
        if (notification.method === "event") {
          listener(notification.params as RuntimeJournalEvent);
        }
      }),
  };
}

function installDesktopBridge(runtime: IkarosRuntimeBridgeApi): void {
  Object.defineProperty(window, "ikarosDesktop", {
    configurable: true,
    value: {
      runtime,
      preferences: {},
      windowControls: {},
    } as IkarosDesktopApi,
  });
}

async function waitFor<TResult>(
  description: string,
  probe: () => TResult | null | undefined | false | Promise<TResult | null | undefined | false>,
  timeoutMs = 120_000,
): Promise<TResult> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await probe();
    if (value !== null && value !== undefined && value !== false) {
      return value;
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`Timed out waiting for ${description}.`);
}

function selectedThread(state: { threads: Thread[]; selectedThreadId: string | null }): Thread {
  const thread = state.threads.find((candidate) => candidate.id === state.selectedThreadId);
  if (!thread) {
    throw new Error("The live Runtime did not project a selected Thread.");
  }
  return thread;
}

function latestTurn(thread: Thread): Turn {
  const turn = activeBranch(thread)?.turns.at(-1);
  if (!turn) {
    throw new Error("The live Runtime did not project a Turn.");
  }
  return turn;
}

function assistantMessage(turn: Turn): Extract<AgentEvent, { type: "message" }> {
  const message = [...turn.events]
    .reverse()
    .find((event) => event.type === "message" && event.role === "assistant");
  if (!message || message.type !== "message") {
    throw new Error("The live Runtime did not project an assistant message.");
  }
  return message;
}

function completedItem(event: RuntimeJournalEvent): Record<string, unknown> | undefined {
  if (event.type !== "item.completed") {
    return undefined;
  }
  const item = event.payload.item;
  return typeof item === "object" && item !== null
    ? (item as Record<string, unknown>)
    : undefined;
}

async function replayAll(host: RuntimeHost): Promise<RuntimeJournalEvent[]> {
  let afterSeq = 0;
  const events: RuntimeJournalEvent[] = [];
  while (true) {
    const page = await host.request<RuntimeReplayResult>("event.replay", {
      afterSeq,
      limit: 1000,
    });
    events.push(...page.events);
    if (!page.hasMore) {
      return events;
    }
    afterSeq = page.nextAfterSeq;
  }
}

function hiddenWindowsChildCommand(pidFile: string): string {
  const path = pidFile.replaceAll("'", "''");
  return [
    "$psi = [Diagnostics.ProcessStartInfo]::new()",
    "$psi.FileName = 'powershell.exe'",
    "$psi.Arguments = '-NoLogo -NoProfile -NonInteractive -Command \"Start-Sleep -Seconds 120\"'",
    "$psi.UseShellExecute = $false",
    "$psi.CreateNoWindow = $true",
    "$child = [Diagnostics.Process]::Start($psi)",
    `[IO.File]::WriteAllText('${path}', [string]$child.Id)`,
    "$child.WaitForExit()",
  ].join("; ");
}

function processExists(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ESRCH") {
      return false;
    }
    throw error;
  }
}

async function filesUnder(root: string): Promise<string[]> {
  let entries;
  try {
    entries = await readdir(root, { withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      return [];
    }
    throw error;
  }
  const files: string[] = [];
  for (const entry of entries) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await filesUnder(path)));
    } else if (entry.isFile()) {
      files.push(path);
    }
  }
  return files;
}

async function assertCredentialIsolated(
  apiKey: string,
  providerConfigured: boolean,
  runtimeHome: string,
): Promise<void> {
  const credential = Buffer.from(apiKey, "utf8");
  const configPath = join(runtimeHome, "config.yaml");
  if (providerConfigured) {
    const config = await readFile(configPath);
    if (config.indexOf(credential) < 0) {
      throw new Error("The live credential was not persisted in config.yaml.");
    }
  }
  for (const path of await filesUnder(runtimeHome)) {
    if (path === configPath) {
      continue;
    }
    if ((await readFile(path)).indexOf(credential) >= 0) {
      throw new Error(`The live credential leaked into ${path}.`);
    }
  }
}

afterEach(() => {
  vi.useRealTimers();
  vi.resetModules();
  Reflect.deleteProperty(window, "ikarosDesktop");
});

describe("Scripted Runtime store history vertical slice", () => {
  it(
    "loads only the catalog at startup and hydrates persisted history on selection",
    { timeout: 60_000 },
    async () => {
      const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-scripted-history-"));
      const host = new RuntimeHost({ runtimeRoot, runtimeHome });
      try {
        await host.start();
        const created = await host.request<RuntimeThreadCreateResult>("thread.create", {
          title: "Persisted Scripted history",
        });
        const started = await host.request<RuntimeTurnStartResult>("turn.start", {
          threadId: created.thread.id,
          branchId: created.thread.defaultBranchId,
          content: "history hydration proof",
          providerId: "scripted",
          modelId: "scripted-v1",
        });
        await waitFor("the Scripted Run to settle", async () => {
          const events = await replayAll(host);
          return events.find(
            (event) => event.type === "run.settled" && event.runId === started.runId,
          );
        });

        installDesktopBridge(runtimeBridge(host, []));
        vi.resetModules();
        const { useAppStore } = await import("./store");
        await useAppStore.getState().initializeRuntime();

        const catalogThread = useAppStore
          .getState()
          .threads.find((thread) => thread.id === created.thread.id);
        expect(catalogThread).toBeDefined();
        expect(activeBranch(catalogThread)?.turns).toEqual([]);

        await useAppStore.getState().selectThread(created.thread.id);
        const hydrated = selectedThread(useAppStore.getState());
        const turn = latestTurn(hydrated);
        expect(turn.runId).toBe(started.runId);
        expect(assistantMessage(turn)).toMatchObject({
          content: "Scripted response to: history hydration proof",
          status: "complete",
        });
      } finally {
        await host.stop();
        await rm(runtimeHome, { recursive: true, force: true });
      }
    },
  );
});

describe.skipIf(!liveEnabled)("live DeepSeek Runtime store vertical slice", () => {
  it(
    "streams contextual Turns, executes file tools, and stops a process tree",
    { timeout: 300_000 },
    async () => {
      if (process.platform !== "win32") {
        throw new Error("This live process-tree proof currently requires Windows.");
      }
      const keyFile = process.env.IKAROS_LIVE_DEEPSEEK_KEY_FILE;
      if (!keyFile) {
        throw new Error("IKAROS_LIVE_DEEPSEEK_KEY_FILE is required for the live smoke.");
      }
      const apiKey = (await readFile(keyFile, "utf8")).replace(/^\uFEFF/, "").trim();
      if (!apiKey || /[\u0000-\u001f\u007f]/u.test(apiKey)) {
        throw new Error("The DeepSeek credential file has an invalid format.");
      }

      const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-live-"));
      const runtimeStderr: string[] = [];
      const host = new RuntimeHost({
        runtimeRoot,
        runtimeHome,
        startTimeoutMs: 30_000,
        stopTimeoutMs: 15_000,
        stderrSink: (message) => runtimeStderr.push(message),
      });
      const rawEvents: RuntimeJournalEvent[] = [];
      const cancellationResults: RuntimeCancelRunResult[] = [];
      const removeRawListener = host.onNotification((notification) => {
        if (notification.method === "event") {
          rawEvents.push(notification.params as RuntimeJournalEvent);
        }
      });
      const pidFile = join(runtimeHome, "nested-process.pid");
      let providerConfigured = false;
      let spawnedChildPid: number | undefined;
      let testFailure: unknown;
      let fileWorkspace: string | undefined;

      try {
        await host.start();
        installDesktopBridge(runtimeBridge(host, cancellationResults));
        vi.resetModules();
        const { useAppStore } = await import("./store");

        await useAppStore.getState().initializeRuntime();
        await useAppStore.getState().configureProvider({
          kind: "deepseek",
          apiKey,
          models: [{ id: "deepseek-chat", displayName: "DeepSeek Chat" }],
        });
        providerConfigured = true;
        expect(useAppStore.getState().providers).toContainEqual(
          expect.objectContaining({
            id: "deepseek",
            configured: true,
            credentialConfigured: true,
          }),
        );
        expect(useAppStore.getState().selectedModel).toEqual({
          providerId: "deepseek",
          modelId: "deepseek-chat",
        });

        useAppStore.getState().newChat();
        const contextMarker = `IKAROS_CONTEXT_${randomBytes(8).toString("hex").toUpperCase()}`;
        useAppStore
          .getState()
          .setDraft(
            `Remember this exact marker for my next message: ${contextMarker}. ` +
              "Reply with the marker verbatim and do not call a tool on this turn.",
          );
        await useAppStore.getState().sendDraft();
        const firstTurn = await waitFor("the first DeepSeek Turn to complete", () => {
          const state = useAppStore.getState();
          const thread = state.selectedThreadId
            ? state.threads.find((candidate) => candidate.id === state.selectedThreadId)
            : undefined;
          const turn = thread ? activeBranch(thread)?.turns.at(-1) : undefined;
          return turn?.status === "completed" ? turn : undefined;
        });
        expect(assistantMessage(firstTurn).content).toContain(contextMarker);
        expect(
          rawEvents.some(
            (event) =>
              event.runId === firstTurn.runId &&
              event.type === "item.delta" &&
              typeof event.payload.delta === "string" &&
              event.payload.delta.length > 0,
          ),
        ).toBe(true);

        fileWorkspace = await mkdtemp(join(tmpdir(), "ikaros-live-files-"));
        const filePath = join(fileWorkspace, "tool-proof.txt");
        const initialFileToken = `IKAROS_FILE_INITIAL_${randomBytes(8).toString("hex").toUpperCase()}`;
        const editedFileToken = `IKAROS_FILE_EDITED_${randomBytes(8).toString("hex").toUpperCase()}`;
        useAppStore
          .getState()
          .setDraft(
            "This is a file-tool execution validation. Use the available tools in this exact order: " +
              `(1) call write once with filePath ${JSON.stringify(filePath)} and content ` +
              `${JSON.stringify(initialFileToken)}, (2) call read once for that same file, ` +
              `(3) call edit once with oldString ${JSON.stringify(initialFileToken)} and ` +
              `newString ${JSON.stringify(editedFileToken)}, and (4) call read once more. ` +
              "Do not use process_run and do not merely describe the calls. After all four tool " +
              "results, reply in one short line containing the marker from my previous message " +
              "and the final edited token.",
          );
        await useAppStore.getState().sendDraft();
        const secondTurn = await waitFor("the second DeepSeek Turn to complete", () => {
          const thread = selectedThread(useAppStore.getState());
          const turn = latestTurn(thread);
          return turn.id !== firstTurn.id && turn.status === "completed" ? turn : undefined;
        });
        const secondRaw = rawEvents.filter((event) => event.runId === secondTurn.runId);
        const toolCalls = secondRaw
          .map((event) => completedItem(event))
          .filter((item): item is NonNullable<typeof item> => item?.kind === "tool_call");
        const toolResults = secondRaw
          .map((event) => completedItem(event))
          .filter((item): item is NonNullable<typeof item> => item?.kind === "tool_result");
        expect(
          toolCalls.map((item) =>
            typeof item.data === "object" && item.data !== null
              ? (item.data as Record<string, unknown>).toolName
              : undefined,
          ),
        ).toEqual([
          "write",
          "read",
          "edit",
          "read",
        ]);
        expect(toolResults).toHaveLength(4);
        expect(toolResults.every((item) => item.status === "completed")).toBe(true);
        const persistedFile = await readFile(filePath, "utf8");
        expect(persistedFile).toBe(editedFileToken);
        expect(assistantMessage(secondTurn).content).toContain(contextMarker);
        expect(assistantMessage(secondTurn).content).toContain(editedFileToken);
        expect(secondTurn.events).toEqual(
          expect.arrayContaining([
            expect.objectContaining({
              type: "tool_call",
              toolName: "write",
              status: "success",
            }),
            expect.objectContaining({
              type: "tool_call",
              toolName: "read",
              status: "success",
            }),
            expect.objectContaining({
              type: "tool_call",
              toolName: "edit",
              status: "success",
            }),
          ]),
        );
        const lastToolResultEvent = [...secondRaw]
          .reverse()
          .find((event) => completedItem(event)?.kind === "tool_result");
        const finalAssistantEvent = [...secondRaw].reverse().find((event) => {
          const item = completedItem(event);
          return item?.kind === "message" && item.role === "assistant";
        });
        expect(lastToolResultEvent?.seq).toBeLessThan(finalAssistantEvent?.seq ?? 0);
        expect(secondRaw.filter((event) => event.type === "run.settled")).toEqual([
          expect.objectContaining({ payload: expect.objectContaining({ status: "completed" }) }),
        ]);

        const projectedThread = selectedThread(useAppStore.getState());
        const projectedTurns = activeBranch(projectedThread)?.turns ?? [];
        expect(projectedTurns.slice(-2).map((turn) => turn.runId)).toEqual([
          firstTurn.runId,
          secondTurn.runId,
        ]);

        const usage = await host.request<RuntimeUsageReadResult>("usage.read");
        expect(usage.summary.lifetimeTokens).not.toBeNull();
        expect(usage.summary.lifetimeTokens ?? 0).toBeGreaterThan(0);
        expect(usage.summary.peakDailyTokens).not.toBeNull();
        expect(usage.summary.peakDailyTokens ?? 0).toBeGreaterThan(0);
        expect(usage.summary.longestRunningTurnSec).not.toBeNull();
        expect(usage.summary.currentStreakDays).toBeGreaterThanOrEqual(1);
        expect(usage.summary.longestStreakDays).toBeGreaterThanOrEqual(1);
        expect(
          usage.dailyUsageBuckets.reduce((total, bucket) => total + bucket.tokens, 0),
        ).toBe(usage.summary.lifetimeTokens);

        const stopStarted = await host.request<RuntimeTurnStartResult>("turn.start", {
          threadId: projectedThread.id,
          branchId: projectedThread.activeBranchId,
          content: `/process.run ${hiddenWindowsChildCommand(pidFile)}`,
          providerId: "scripted",
          modelId: "scripted-v1",
          clientRequestId: `live-stop-${randomBytes(8).toString("hex")}`,
        });
        const childPid = await waitFor("the nested process PID marker", async () => {
          try {
            const source = (await readFile(pidFile, "utf8")).trim();
            const pid = Number(source);
            return Number.isSafeInteger(pid) && pid > 0 ? pid : undefined;
          } catch (error) {
            if ((error as NodeJS.ErrnoException).code === "ENOENT") {
              return undefined;
            }
            throw error;
          }
        });
        spawnedChildPid = childPid;
        expect(processExists(childPid)).toBe(true);
        await waitFor("the cancellable Turn to reach the renderer store", () => {
          const turn = latestTurn(selectedThread(useAppStore.getState()));
          return turn.runId === stopStarted.runId && turn.status === "running" ? turn : undefined;
        });
        useAppStore.getState().stopRun();
        await waitFor("the UI cancellation request to be accepted", () =>
          cancellationResults.find(
            (candidate) => candidate.runId === stopStarted.runId && candidate.accepted,
          ),
        );
        await waitFor("the cancelled Run to settle", () =>
          rawEvents.find(
            (event) =>
              event.runId === stopStarted.runId &&
              event.type === "run.settled" &&
              event.payload.status === "cancelled",
          ),
        );
        const stoppedTurn = await waitFor("the cancelled Turn projection", () => {
          const turn = latestTurn(selectedThread(useAppStore.getState()));
          return turn.runId === stopStarted.runId && turn.status === "interrupted"
            ? turn
            : undefined;
        });
        expect(stoppedTurn.events).toEqual(
          expect.arrayContaining([
            expect.objectContaining({ type: "tool_call", status: "interrupted" }),
            expect.objectContaining({ type: "tool_result", status: "interrupted" }),
          ]),
        );
        await waitFor("the nested process tree to terminate", () => !processExists(childPid));

        const replay = await replayAll(host);
        const publicState = {
          replay,
          providers: await host.request("provider.list"),
          models: await host.request("model.list"),
          renderer: useAppStore.getState(),
        };
        if (JSON.stringify(publicState).includes(apiKey)) {
          throw new Error("The live credential appeared in a public Runtime or renderer value.");
        }
      } catch (error) {
        testFailure = error;
      } finally {
        const failures: unknown[] = testFailure === undefined ? [] : [testFailure];
        removeRawListener();
        try {
          await host.stop();
        } catch (error) {
          failures.push(error);
        }
        if (runtimeStderr.some((message) => message.includes(apiKey))) {
          failures.push(new Error("The live credential appeared in Runtime stderr."));
        }
        if (spawnedChildPid !== undefined && processExists(spawnedChildPid)) {
          try {
            process.kill(spawnedChildPid);
            await waitFor(
              "the failed-smoke child process to terminate",
              () => !processExists(spawnedChildPid as number),
              10_000,
            );
          } catch (error) {
            failures.push(error);
          }
        }
        try {
          await rm(pidFile, { force: true });
        } catch (error) {
          failures.push(error);
        }
        if (fileWorkspace !== undefined) {
          try {
            await rm(fileWorkspace, { force: true, recursive: true });
          } catch (error) {
            failures.push(error);
          }
        }
        try {
          await assertCredentialIsolated(apiKey, providerConfigured, runtimeHome);
        } catch (error) {
          failures.push(error);
        }
        try {
          await rm(runtimeHome, { force: true, recursive: true });
        } catch (error) {
          failures.push(error);
        }
        if (failures.length === 1) {
          throw failures[0];
        }
        if (failures.length > 1) {
          throw new AggregateError(failures, "The live smoke and its cleanup reported failures.");
        }
      }
    },
  );
});
