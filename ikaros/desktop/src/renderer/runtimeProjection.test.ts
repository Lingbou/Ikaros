import { describe, expect, it } from "vitest";

import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  type RuntimeItemHistory,
  type RuntimeJournalEvent,
  type RuntimeThreadSummary,
  type RuntimeTurnHistory,
} from "../shared/runtime";
import {
  applyRuntimeCatalogEvent,
  projectRuntimeProjects,
  projectRuntimeThreadHistory,
  projectRuntimeThreads,
  replayRuntimeEvents,
} from "./runtimeProjection";

const summary: RuntimeThreadSummary = {
  id: "thread-1",
  title: "Runtime chat",
  defaultBranchId: "branch-1",
  workspace: null,
  createdAt: "2026-08-11T12:00:00.000Z",
  updatedAt: "2026-08-11T12:00:00.000Z",
  archivedAt: null
};

function threadSnapshotEvent(
  seq: number,
  type: "thread.renamed" | "thread.archived" | "thread.unarchived",
  thread: RuntimeThreadSummary,
): RuntimeJournalEvent {
  return {
    seq,
    schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type,
    threadId: thread.id,
    branchId: thread.defaultBranchId,
    turnId: null,
    runId: null,
    itemId: null,
    timestamp: thread.updatedAt,
    payload: { thread },
  };
}

describe("Runtime Thread lifecycle projection", () => {
  it("renames in place, removes archived Threads, and restores unarchived Threads", () => {
    const initial = projectRuntimeThreads([summary]);
    initial[0]?.branches[0]?.turns.push({
      id: "turn-preserved",
      branchId: summary.defaultBranchId,
      runId: "run-preserved",
      status: "completed",
      events: [],
    });
    const renamed = {
      ...summary,
      title: "Renamed",
      updatedAt: "2026-08-11T12:01:00.000Z",
    };
    const afterRename = applyRuntimeCatalogEvent(
      initial,
      threadSnapshotEvent(2, "thread.renamed", renamed),
    );
    expect(afterRename[0]).toMatchObject({ title: "Renamed" });
    expect(afterRename[0]?.branches[0]?.turns).toHaveLength(1);

    const archived = {
      ...renamed,
      archivedAt: "2026-08-11T12:02:00.000Z",
      updatedAt: "2026-08-11T12:02:00.000Z",
    };
    expect(
      applyRuntimeCatalogEvent(
        afterRename,
        threadSnapshotEvent(3, "thread.archived", archived),
      ),
    ).toEqual([]);

    const restored = {
      ...archived,
      archivedAt: null,
      updatedAt: "2026-08-11T12:03:00.000Z",
    };
    expect(
      applyRuntimeCatalogEvent([], threadSnapshotEvent(4, "thread.unarchived", restored)),
    ).toMatchObject([{ id: summary.id, title: "Renamed" }]);
  });
});

describe("Runtime workspace projection", () => {
  it("groups workspace Threads once and leaves ordinary Threads standalone", () => {
    const workspace = {
      id: "workspace-ikaros",
      name: "Ikaros",
      rootUri: "C:\\Workspace\\github\\Ikaros",
    };
    const summaries = [
      { ...summary, id: "project-thread-1", workspace },
      { ...summary, id: "project-thread-2", workspace },
      { ...summary, id: "ordinary-thread", workspace: null },
    ];

    expect(projectRuntimeThreads(summaries).map((thread) => thread.projectId)).toEqual([
      workspace.id,
      workspace.id,
      null,
    ]);
    expect(projectRuntimeProjects(summaries)).toEqual([
      {
        id: workspace.id,
        name: workspace.name,
        color: "var(--muted-strong)",
        rootUri: workspace.rootUri,
      },
    ]);
  });

  it("projects a materialized Thread history without requiring Journal replay", () => {
    const turns: RuntimeTurnHistory[] = [
      {
        id: "turn-history",
        threadId: summary.id,
        branchId: summary.defaultBranchId,
        ordinal: 1,
        status: "completed",
        createdAt: summary.createdAt,
        updatedAt: summary.updatedAt,
        runs: [
          {
            id: "run-history",
            turnId: "turn-history",
            providerId: "scripted",
            modelId: "scripted-v1",
            executionPolicy: "full_access",
            startedAt: null, modelCalls: 0,
            status: "completed",
            reasonCode: null,
            createdAt: summary.createdAt,
            settledAt: summary.updatedAt,
            items: [
              {
                id: "user-history",
                turnId: "turn-history",
                runId: "run-history",
                ordinal: 1,
                kind: "message",
                role: "user",
                status: "completed",
                content: "hello",
                data: {},
                createdAt: summary.createdAt,
                updatedAt: summary.updatedAt,
              },
              {
                id: "assistant-history",
                turnId: "turn-history",
                runId: "run-history",
                ordinal: 2,
                kind: "message",
                role: "assistant",
                status: "completed",
                content: "world",
                data: {},
                createdAt: summary.createdAt,
                updatedAt: summary.updatedAt,
              },
            ],
          },
        ],
      },
    ];

    expect(projectRuntimeThreadHistory(summary, turns).branches[0]?.turns).toEqual([
      expect.objectContaining({
        id: "turn-history",
        runId: "run-history",
        status: "completed",
        events: [
          expect.objectContaining({ id: "user-history", content: "hello" }),
          expect.objectContaining({ id: "assistant-history", content: "world" }),
        ],
      }),
    ]);
  });
});

function event(
  seq: number,
  type: RuntimeJournalEvent["type"],
  turnId: string,
  runId: string,
  itemId: string | null,
  payload: Record<string, unknown>
): RuntimeJournalEvent {
  return {
    seq,
    schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type,
    threadId: summary.id,
    branchId: summary.defaultBranchId,
    turnId,
    runId,
    itemId,
    timestamp: `2026-08-11T12:00:${String(seq).padStart(2, "0")}.000Z`,
    payload
  };
}

function messageItem(
  id: string,
  turnId: string,
  runId: string,
  role: "user" | "assistant",
  content: string,
  status: string
) {
  return {
    id,
    turnId,
    runId,
    ordinal: role === "user" ? 1 : 2,
    kind: "message",
    role,
    status,
    content,
    createdAt: "2026-08-11T12:00:00.000Z",
    updatedAt: "2026-08-11T12:00:00.000Z"
  };
}

function toolCallItem(
  id: string,
  turnId: string,
  runId: string,
  status: "running" | "completed" | "failed" | "cancelled",
  callId: string,
  durationMs?: number
) {
  return {
    id,
    turnId,
    runId,
    ordinal: 2,
    kind: "tool_call",
    role: "assistant",
    status,
    content: "",
    data: {
      stepId: "step-1",
      callId,
      toolName: "process_start",
      arguments: { command: "Write-Output original-command" },
      ...(durationMs === undefined ? {} : { durationMs })
    },
    createdAt: "2026-08-11T12:00:00.000Z",
    updatedAt: "2026-08-11T12:00:00.000Z"
  };
}

function toolResultItem(
  id: string,
  turnId: string,
  runId: string,
  status: "completed" | "failed" | "cancelled",
  toolCallItemId: string,
  output: string
) {
  return {
    id,
    turnId,
    runId,
    ordinal: 3,
    kind: "tool_result",
    role: "tool",
    status,
    content: "{}",
    data: {
      stepId: "step-1",
      callId: `provider-${toolCallItemId}`,
      toolCallItemId,
      toolName: "process_start",
      result: { output }
    },
    createdAt: "2026-08-11T12:00:00.000Z",
    updatedAt: "2026-08-11T12:00:00.000Z"
  };
}

function fileToolCallItem(
  id: string,
  turnId: string,
  runId: string,
  toolName: "read" | "write" | "edit",
  status: "running" | "completed" | "failed" | "cancelled",
  argumentsValue: Record<string, unknown>,
) {
  return {
    id,
    turnId,
    runId,
    ordinal: 2,
    kind: "tool_call",
    role: "assistant",
    status,
    content: "",
    data: {
      stepId: "step-files",
      callId: `provider-${id}`,
      toolName,
      arguments: argumentsValue,
    },
    createdAt: "2026-08-11T12:00:00.000Z",
    updatedAt: "2026-08-11T12:00:00.000Z",
  };
}

function fileToolResultItem(
  id: string,
  turnId: string,
  runId: string,
  toolName: "read" | "write" | "edit",
  status: "completed" | "failed" | "cancelled",
  toolCallItemId: string,
  result: Record<string, unknown>,
) {
  return {
    id,
    turnId,
    runId,
    ordinal: 3,
    kind: "tool_result",
    role: "tool",
    status,
    content: "{}",
    data: {
      stepId: "step-files",
      callId: `provider-${toolCallItemId}`,
      toolCallItemId,
      toolName,
      result,
    },
    createdAt: "2026-08-11T12:00:00.000Z",
    updatedAt: "2026-08-11T12:00:00.000Z",
  };
}

describe("Runtime event projection", () => {
  it.each([
    ["failed", "provider_timeout", "failed"],
    ["failed", "runtime_interrupted", "failed"],
    ["cancelled", "cancelled", "interrupted"],
    ["failed", null, "failed"],
  ] as const)("preserves %s / %s outcomes equally in live and message-free history", (status, reasonCode, projectedStatus) => {
    const settled = event(2, "run.settled", "turn-outcome", "run-outcome", null, {
      status,
      ...(reasonCode === null ? {} : { reasonCode }),

      createdAt: summary.createdAt, startedAt: null, modelCalls: 0,
      providerId: "scripted", modelId: "scripted-v1",
    });
    const live = replayRuntimeEvents(projectRuntimeThreads([summary]), [settled]);
    const history = projectRuntimeThreadHistory(summary, [{
      id: "turn-outcome",
      threadId: summary.id,
      branchId: summary.defaultBranchId,
      ordinal: 1,
      status,
      createdAt: summary.createdAt,
      updatedAt: settled.timestamp,
      runs: [{
        id: "run-outcome",
        turnId: "turn-outcome",
        providerId: "scripted",
        modelId: "scripted-v1",
        executionPolicy: "full_access",
        startedAt: null, modelCalls: 0,
        status,
        reasonCode,
        createdAt: summary.createdAt,
        settledAt: settled.timestamp,
        items: [],
      }],
    }]);
    expect(live[0]?.branches[0]?.turns).toEqual(history.branches[0]?.turns);
    expect(history.branches[0]?.turns[0]).toMatchObject({
      status: projectedStatus,
      reasonCode,
      events: [],
    });
  });

  it("preserves successful mutations and process interruption reasons when a Run fails", () => {
    const events = [
      event(2, "item.completed", "turn-outcome", "run-outcome", "call-success", {
        item: fileToolCallItem("call-success", "turn-outcome", "run-outcome", "write", "completed", { path: "/tmp/a.txt" }),
      }),
      event(3, "item.completed", "turn-outcome", "run-outcome", "result-success", {
        item: fileToolResultItem("result-success", "turn-outcome", "run-outcome", "write", "completed", "call-success", { path: "/tmp/a.txt", bytesWritten: 4 }),
      }),
      event(4, "item.completed", "turn-outcome", "run-outcome", "call-interrupted", {
        item: toolCallItem("call-interrupted", "turn-outcome", "run-outcome", "failed", "provider-interrupted"),
      }),
      event(5, "item.completed", "turn-outcome", "run-outcome", "result-interrupted", {
        item: {
          ...toolResultItem("result-interrupted", "turn-outcome", "run-outcome", "failed", "call-interrupted", ""),
          data: {
            toolCallItemId: "call-interrupted",
            toolName: "process_start",
            result: { output: "", errorCode: "runtime_interrupted" },
          },
        },
      }),
      event(6, "run.settled", "turn-outcome", "run-outcome", null, { status: "failed", reasonCode: "runtime_interrupted", createdAt: summary.createdAt, modelCalls: 0, providerId: "scripted", modelId: "scripted-v1" }),
    ];
    const [thread] = replayRuntimeEvents(projectRuntimeThreads([summary]), events);
    const turn = thread.branches[0]?.turns[0];
    expect(turn).toMatchObject({ status: "failed", reasonCode: "runtime_interrupted" });
    expect(turn?.events).toEqual([
      expect.objectContaining({ id: "call-success", status: "success" }),
      expect.objectContaining({ id: "result-success", status: "success", path: "/tmp/a.txt" }),
      expect.objectContaining({ id: "call-interrupted", status: "error" }),
      expect.objectContaining({ id: "result-interrupted", status: "error", errorCode: "runtime_interrupted" }),
    ]);
    const history = projectRuntimeThreadHistory(summary, [{
      id: "turn-outcome",
      threadId: summary.id,
      branchId: summary.defaultBranchId,
      ordinal: 1,
      status: "failed",
      createdAt: summary.createdAt,
      updatedAt: events[4]!.timestamp,
      runs: [{
        id: "run-outcome",
        turnId: "turn-outcome",
        providerId: "scripted",
        modelId: "scripted-v1",
        executionPolicy: "full_access",
        startedAt: null, modelCalls: 0,
        status: "failed",
        reasonCode: "runtime_interrupted",
        createdAt: summary.createdAt,
        settledAt: events[4]!.timestamp,
        items: events.slice(0, 4).map((itemEvent, index) => ({
          ...(itemEvent.payload.item as RuntimeItemHistory),
          ordinal: index + 1,
        })),
      }],
    }]);
    expect(history.branches[0]?.turns).toEqual(thread.branches[0]?.turns);
  });

  it("does not create conversation content from standalone model audit events", () => {
    const initial = projectRuntimeThreads([summary]);
    const auditEvents = [
      event(2, "model.input_prepared", "turn-audit", "run-audit", null, {
        stepOrdinal: 1,
      }),
      event(3, "model.response_finished", "turn-audit", "run-audit", null, {
        stepOrdinal: 1,
      }),
    ];

    expect(applyRuntimeCatalogEvent(initial, auditEvents[0] as RuntimeJournalEvent)).toBe(
      initial,
    );
    expect(applyRuntimeCatalogEvent(initial, auditEvents[1] as RuntimeJournalEvent)).toBe(
      initial,
    );
    expect(replayRuntimeEvents(initial, auditEvents)).toBe(initial);
  });

  it("reduces streamed Items and settled Runs into the existing conversation UI model", () => {
    const events = [
      event(2, "item.completed", "turn-1", "run-1", "user-1", {
        item: messageItem("user-1", "turn-1", "run-1", "user", "hello", "completed")
      }),
      event(3, "run.state_changed", "turn-1", "run-1", null, { status: "queued" }),
      event(4, "run.state_changed", "turn-1", "run-1", null, { status: "running" }),
      event(5, "item.started", "turn-1", "run-1", "assistant-1", {
        item: messageItem("assistant-1", "turn-1", "run-1", "assistant", "", "streaming")
      }),
      event(6, "item.delta", "turn-1", "run-1", "assistant-1", { delta: "Hello " }),
      event(7, "item.delta", "turn-1", "run-1", "assistant-1", { delta: "world" }),
      event(8, "item.completed", "turn-1", "run-1", "assistant-1", {
        item: messageItem(
          "assistant-1",
          "turn-1",
          "run-1",
          "assistant",
          "Hello world",
          "completed"
        )
      }),
      event(9, "run.settled", "turn-1", "run-1", null, { status: "completed" }),
      event(10, "item.completed", "turn-2", "run-2", "user-2", {
        item: messageItem("user-2", "turn-2", "run-2", "user", "again", "completed")
      }),
      event(11, "run.settled", "turn-2", "run-2", null, { status: "completed" }),
      event(12, "item.started", "turn-3", "run-3", "assistant-3", {
        item: messageItem("assistant-3", "turn-3", "run-3", "assistant", "", "streaming")
      }),
      event(13, "item.completed", "turn-3", "run-3", "assistant-3", {
        item: messageItem("assistant-3", "turn-3", "run-3", "assistant", "partial", "failed")
      }),
      event(14, "run.settled", "turn-3", "run-3", null, { status: "failed" }),
      event(15, "item.started", "turn-4", "run-4", "assistant-4", {
        item: messageItem("assistant-4", "turn-4", "run-4", "assistant", "", "streaming")
      }),
      event(16, "item.delta", "turn-4", "run-4", "assistant-4", { delta: "partial" }),
      event(17, "item.completed", "turn-4", "run-4", "assistant-4", {
        item: messageItem("assistant-4", "turn-4", "run-4", "assistant", "partial", "cancelled")
      }),
      event(18, "run.settled", "turn-4", "run-4", null, { status: "cancelled" })
    ];

    const [thread] = replayRuntimeEvents(projectRuntimeThreads([summary]), events);
    const turns = thread.branches[0]?.turns;

    expect(turns).toHaveLength(4);
    expect(turns?.[0]).toMatchObject({
      id: "turn-1",
      runId: "run-1",
      status: "completed",
      events: [
        { id: "user-1", role: "user", content: "hello", status: "complete" },
        {
          id: "assistant-1",
          role: "assistant",
          content: "Hello world",
          status: "complete"
        }
      ]
    });
    expect(turns?.[1]).toMatchObject({
      id: "turn-2",
      runId: "run-2",
      status: "completed",
      events: [{ id: "user-2", role: "user", content: "again" }]
    });
    expect(turns?.[2]).toMatchObject({
      id: "turn-3",
      runId: "run-3",
      status: "failed",
      events: [{ id: "assistant-3", content: "partial", status: "failed" }]
    });
    expect(turns?.[3]).toMatchObject({
      id: "turn-4",
      runId: "run-4",
      status: "interrupted",
      events: [{ id: "assistant-4", content: "partial", status: "interrupted" }]
    });
  });

  it("projects replay-safe process tool lifecycle items without translating command output", () => {
    const events = [
      event(2, "item.completed", "turn-tools", "run-tools", "user-tools", {
        item: messageItem(
          "user-tools",
          "turn-tools",
          "run-tools",
          "user",
          "/process_start Write-Output original-command",
          "completed"
        )
      }),
      event(3, "item.started", "turn-tools", "run-tools", "call-success", {
        item: toolCallItem(
          "call-success",
          "turn-tools",
          "run-tools",
          "running",
          "provider-success"
        )
      }),
      event(4, "item.completed", "turn-tools", "run-tools", "call-success", {
        item: toolCallItem(
          "call-success",
          "turn-tools",
          "run-tools",
          "completed",
          "provider-success",
          12
        )
      }),
      event(5, "item.completed", "turn-tools", "run-tools", "result-success", {
        item: toolResultItem(
          "result-success",
          "turn-tools",
          "run-tools",
          "completed",
          "call-success",
          "original-output\nsecond-line"
        )
      }),
      event(6, "item.started", "turn-tools", "run-tools", "call-failed", {
        item: toolCallItem(
          "call-failed",
          "turn-tools",
          "run-tools",
          "running",
          "provider-failed"
        )
      }),
      event(7, "item.completed", "turn-tools", "run-tools", "call-failed", {
        item: toolCallItem(
          "call-failed",
          "turn-tools",
          "run-tools",
          "failed",
          "provider-failed"
        )
      }),
      event(8, "item.completed", "turn-tools", "run-tools", "result-failed", {
        item: toolResultItem(
          "result-failed",
          "turn-tools",
          "run-tools",
          "failed",
          "call-failed",
          "failure-output"
        )
      }),
      event(9, "item.started", "turn-tools", "run-tools", "call-cancelled", {
        item: toolCallItem(
          "call-cancelled",
          "turn-tools",
          "run-tools",
          "running",
          "provider-cancelled"
        )
      }),
      event(10, "item.completed", "turn-tools", "run-tools", "call-cancelled", {
        item: toolCallItem(
          "call-cancelled",
          "turn-tools",
          "run-tools",
          "cancelled",
          "provider-cancelled"
        )
      }),
      event(11, "item.completed", "turn-tools", "run-tools", "result-cancelled", {
        item: toolResultItem(
          "result-cancelled",
          "turn-tools",
          "run-tools",
          "cancelled",
          "call-cancelled",
          "partial-output"
        )
      })
    ];

    const once = replayRuntimeEvents(projectRuntimeThreads([summary]), events);
    const twice = replayRuntimeEvents(once, events);
    const projected = twice[0]?.branches[0]?.turns[0]?.events;

    expect(projected).toHaveLength(7);
    expect(projected?.[1]).toMatchObject({
      id: "call-success",
      type: "tool_call",
      toolName: "process_start",
      label: { source: "app", kind: "tool.startProcess" },
      status: "success",
      arguments: { command: "Write-Output original-command" },
      durationMs: 12
    });
    expect(projected?.[2]).toMatchObject({
      id: "result-success",
      type: "tool_result",
      toolCallId: "call-success",
      status: "success",
      summary: { source: "app", kind: "result.processCompleted" },
      output: "original-output\nsecond-line"
    });
    expect(projected?.[3]).toMatchObject({ type: "tool_call", status: "error" });
    expect(projected?.[4]).toMatchObject({
      type: "tool_result",
      status: "error",
      summary: { source: "app", kind: "result.processFailed" }
    });
    expect(projected?.[5]).toMatchObject({ type: "tool_call", status: "interrupted" });
    expect(projected?.[6]).toMatchObject({
      type: "tool_result",
      status: "interrupted",
      summary: { source: "app", kind: "result.processInterrupted" },
      output: "partial-output"
    });
  });

  it("projects compact, replay-safe file tool events without retaining file or replacement content", () => {
    const secretContent = "SECRET-FILE-CONTENT";
    const oldString = "SECRET-OLD-TEXT";
    const newString = "SECRET-NEW-TEXT";
    const events = [
      event(2, "item.completed", "turn-files", "run-files", "call-read", {
        item: fileToolCallItem(
          "call-read",
          "turn-files",
          "run-files",
          "read",
          "completed",
          { filePath: "notes/readme.txt", offset: 3, limit: 40 },
        ),
      }),
      event(3, "item.completed", "turn-files", "run-files", "result-read", {
        item: fileToolResultItem(
          "result-read",
          "turn-files",
          "run-files",
          "read",
          "completed",
          "call-read",
          {
            output: secretContent,
            path: "C:\\work\\notes\\readme.txt",
            lineStart: 3,
            lineEnd: 18,
            bytesRead: 420,
            nextOffset: 19,
            truncated: true,
          },
        ),
      }),
      event(4, "item.completed", "turn-files", "run-files", "call-write", {
        item: fileToolCallItem(
          "call-write",
          "turn-files",
          "run-files",
          "write",
          "completed",
          { filePath: "notes/new.txt", content: secretContent },
        ),
      }),
      event(5, "item.completed", "turn-files", "run-files", "result-write", {
        item: fileToolResultItem(
          "result-write",
          "turn-files",
          "run-files",
          "write",
          "completed",
          "call-write",
          {
            output: "Created file successfully: C:\\work\\notes\\new.txt",
            path: "C:\\work\\notes\\new.txt",
            bytesWritten: 128,
            created: true,
            truncated: false,
          },
        ),
      }),
      event(6, "item.completed", "turn-files", "run-files", "call-edit", {
        item: fileToolCallItem(
          "call-edit",
          "turn-files",
          "run-files",
          "edit",
          "failed",
          {
            filePath: "notes/readme.txt",
            oldString,
            newString,
            replaceAll: false,
          },
        ),
      }),
      event(7, "item.completed", "turn-files", "run-files", "result-edit", {
        item: fileToolResultItem(
          "result-edit",
          "turn-files",
          "run-files",
          "edit",
          "failed",
          "call-edit",
          {
            output: "File changed since it was read; read it again before editing.",
            path: "C:\\work\\notes\\readme.txt",
            errorCode: "stale_content",
            truncated: false,
          },
        ),
      }),
    ];

    const once = replayRuntimeEvents(projectRuntimeThreads([summary]), events);
    const twice = replayRuntimeEvents(once, events);
    const projected = twice[0]?.branches[0]?.turns[0]?.events;

    expect(projected).toHaveLength(6);
    expect(projected?.[0]).toMatchObject({
      type: "tool_call",
      toolName: "read",
      label: { source: "app", kind: "tool.readFile" },
      arguments: { filePath: "notes/readme.txt", offset: 3, limit: 40 },
    });
    expect(projected?.[1]).toMatchObject({
      type: "tool_result",
      toolName: "read",
      summary: { source: "app", kind: "result.readCompleted" },
      output: "",
      path: "C:\\work\\notes\\readme.txt",
      details: {
        lineStart: 3,
        lineEnd: 18,
        nextOffset: 19,
        bytesRead: 420,
        truncated: true,
      },
    });
    expect(projected?.[2]).toMatchObject({
      type: "tool_call",
      toolName: "write",
      label: { source: "app", kind: "tool.writeFile" },
      arguments: { filePath: "notes/new.txt" },
    });
    expect(projected?.[3]).toMatchObject({
      type: "tool_result",
      toolName: "write",
      summary: { source: "app", kind: "result.writeCompleted" },
      output: "",
      details: { bytesWritten: 128, created: true },
    });
    expect(projected?.[4]).toMatchObject({
      type: "tool_call",
      toolName: "edit",
      label: { source: "app", kind: "tool.editFile" },
      arguments: { filePath: "notes/readme.txt", replaceAll: false },
    });
    expect(projected?.[5]).toMatchObject({
      type: "tool_result",
      toolName: "edit",
      status: "error",
      summary: { source: "app", kind: "result.editFailed" },
      errorCode: "stale_content",
      output: "File changed since it was read; read it again before editing.",
    });
    expect(JSON.stringify(projected)).not.toContain(secretContent);
    expect(JSON.stringify(projected)).not.toContain(oldString);
    expect(JSON.stringify(projected)).not.toContain(newString);
  });

  it("places each tool result directly after its matching call", () => {
    const callIds = ["call-read-one", "call-read-two", "call-read-three"];
    const calls = callIds.map((callId, index) =>
      event(index + 2, "item.started", "turn-paired", "run-paired", callId, {
        item: fileToolCallItem(
          callId,
          "turn-paired",
          "run-paired",
          "read",
          "running",
          { filePath: `notes/${index + 1}.txt` },
        ),
      }),
    );
    const completed = callIds.flatMap((callId, index) => [
      event(index * 2 + 5, "item.completed", "turn-paired", "run-paired", callId, {
        item: fileToolCallItem(
          callId,
          "turn-paired",
          "run-paired",
          "read",
          "completed",
          { filePath: `notes/${index + 1}.txt` },
        ),
      }),
      event(
        index * 2 + 6,
        "item.completed",
        "turn-paired",
        "run-paired",
        `result-read-${index + 1}`,
        {
          item: fileToolResultItem(
            `result-read-${index + 1}`,
            "turn-paired",
            "run-paired",
            "read",
            "completed",
            callId,
            {
              output: `file ${index + 1}`,
              path: `C:\\work\\notes\\${index + 1}.txt`,
              lineStart: 1,
              lineEnd: 1,
              totalLines: 1,
            },
          ),
        },
      ),
    ]);

    const afterFirstResult = replayRuntimeEvents(
      projectRuntimeThreads([summary]),
      [...calls, ...completed.slice(0, 2)],
    );
    expect(
      afterFirstResult[0]?.branches[0]?.turns[0]?.events.map((projected) => projected.id),
    ).toEqual([
      "call-read-one",
      "result-read-1",
      "call-read-two",
      "call-read-three",
    ]);

    const once = replayRuntimeEvents(projectRuntimeThreads([summary]), [...calls, ...completed]);
    const twice = replayRuntimeEvents(once, [...calls, ...completed]);
    const projected = twice[0]?.branches[0]?.turns[0]?.events;

    expect(projected?.map((item) => item.id)).toEqual([
      "call-read-one",
      "result-read-1",
      "call-read-two",
      "result-read-2",
      "call-read-three",
      "result-read-3",
    ]);
    expect(
      projected?.map((item) =>
        item.type === "tool_result" ? item.toolCallId : item.id
      ),
    ).toEqual([
      "call-read-one",
      "call-read-one",
      "call-read-two",
      "call-read-two",
      "call-read-three",
      "call-read-three",
    ]);
  });
});

describe("Runtime execution progress projection", () => {
  it("retains progress through streamed items and reconstructs the same usage from history", () => {
    const queuedAt = "2026-09-07T00:00:00Z";
    const startedAt = "2026-09-07T00:01:00Z";
    const settledAt = "2026-09-07T00:01:20Z";
    const user: RuntimeItemHistory = {
      id: "user-progress", turnId: "turn-progress", runId: "run-progress", ordinal: 1,
      kind: "message", role: "user", status: "completed", content: "Do the task", data: {},
      createdAt: queuedAt, updatedAt: queuedAt,
    };
    const assistant: RuntimeItemHistory = {
      ...user, id: "assistant-progress", ordinal: 2, role: "assistant", content: "Done",
      createdAt: startedAt, updatedAt: settledAt,
    };
    const events = [
      { ...event(1, "item.completed", user.turnId, user.runId, user.id, { item: user, run: { createdAt: queuedAt } }), timestamp: queuedAt },
      { ...event(2, "run.state_changed", user.turnId, user.runId, null, { status: "running" }), timestamp: startedAt },
      event(3, "model.input_prepared", user.turnId, user.runId, null, { stepOrdinal: 1 }),
      event(4, "item.started", user.turnId, user.runId, assistant.id, { item: { ...assistant, content: "", status: "streaming" } }),
      event(5, "item.delta", user.turnId, user.runId, assistant.id, { delta: "Done" }),
      event(6, "model.input_prepared", user.turnId, user.runId, null, { stepOrdinal: 2 }),
      event(7, "item.completed", user.turnId, user.runId, assistant.id, { item: assistant }),
      { ...event(8, "run.settled", user.turnId, user.runId, null, { status: "completed" }), timestamp: settledAt },
    ];
    const live = replayRuntimeEvents(projectRuntimeThreads([summary]), events)[0]!;
    const expected = { queuedAt, startedAt, settledAt, modelCalls: 2 };
    expect(live.branches[0]?.turns[0]?.runProgress).toEqual(expected);
    const history = projectRuntimeThreadHistory(summary, [{
      id: user.turnId, threadId: summary.id, branchId: summary.defaultBranchId, ordinal: 1,
      status: "completed", createdAt: queuedAt, updatedAt: settledAt,
      runs: [{
        id: user.runId, turnId: user.turnId, providerId: "test", modelId: "test",
        executionPolicy: "full_access", status: "completed", reasonCode: null,
        createdAt: queuedAt, startedAt, settledAt, modelCalls: 2,
        items: [user, assistant],
      }],
    }]);
    expect(history.branches[0]?.turns[0]?.runProgress).toEqual(expected);
  });

  it("shows context compaction while the next model input is being prepared", () => {
    const user = messageItem(
      "user-compaction",
      "turn-compaction",
      "run-compaction",
      "user",
      "Do the task",
      "completed",
    );
    let projected = replayRuntimeEvents(projectRuntimeThreads([summary]), [
      event(1, "item.completed", user.turnId, user.runId, user.id, {
        item: user,
        run: { createdAt: summary.createdAt },
      }),
      event(2, "run.state_changed", user.turnId, user.runId, null, { status: "running" }),
      event(3, "model.input_prepared", user.turnId, user.runId, null, { stepOrdinal: 1 }),
      event(4, "context.compacted", user.turnId, user.runId, null, {
        revision: 2,
        droppedTurns: ["turn-old"],
        contextRevision: {},
      }),
    ]);
    expect(projected[0]?.branches[0]?.turns[0]?.runProgress).toMatchObject({
      compactions: 1,
      compacting: true,
    });

    projected = replayRuntimeEvents(projected, [
      event(5, "model.input_prepared", user.turnId, user.runId, null, { stepOrdinal: 2 }),
    ]);
    expect(projected[0]?.branches[0]?.turns[0]?.runProgress).toMatchObject({
      compactions: 1,
      compacting: false,
    });
  });
});


describe("Runtime Turn model selection", () => {
  it("restores each Turn's final Run model while ordering reversed history pages", () => {
    function historyTurn(ordinal: number, modelId: string): RuntimeTurnHistory {
      const turnId = `turn-${ordinal}`;
      return {
        id: turnId, threadId: summary.id, branchId: summary.defaultBranchId, ordinal,
        status: "completed", createdAt: summary.createdAt, updatedAt: summary.updatedAt,
        runs: [{
          id: `run-${ordinal}`, turnId, providerId: "provider", modelId,
          executionPolicy: "full_access", status: "completed", reasonCode: null,
          createdAt: summary.createdAt, startedAt: summary.createdAt,
          settledAt: summary.updatedAt, modelCalls: 1, items: [],
        }],
      };
    }
    const earlier = historyTurn(1, "older-model");
    const later = historyTurn(2, "superseded-model");
    later.runs.push({ ...later.runs[0]!, id: "latest-run", modelId: "latest-model" });
    const projected = projectRuntimeThreadHistory(summary, [later, earlier]);
    expect(projected.branches[0]?.turns.map(({ id, runId, modelSelection }) => ({
      id, runId, modelSelection,
    }))).toEqual([
      { id: "turn-1", runId: "run-1", modelSelection: { providerId: "provider", modelId: "older-model" } },
      { id: "turn-2", runId: "latest-run", modelSelection: { providerId: "provider", modelId: "latest-model" } },
    ]);
  });

  it("keeps the live initial Run model through later events and older Turn updates", () => {
    const olderSelection = { providerId: "provider-a", modelId: "older-model" };
    const latestSelection = { providerId: "provider-b", modelId: "latest-model" };
    const initial = replayRuntimeEvents(projectRuntimeThreads([summary]), [
      event(1, "item.completed", "older-turn", "older-run", "older-user", {
        item: messageItem("older-user", "older-turn", "older-run", "user", "Earlier", "completed"),
        run: { ...olderSelection, createdAt: summary.createdAt },
      }),
      event(2, "item.completed", "latest-turn", "latest-run", "latest-user", {
        item: messageItem("latest-user", "latest-turn", "latest-run", "user", "Latest", "completed"),
        run: { ...latestSelection, createdAt: summary.createdAt },
      }),
    ]);
    const laterEvents = [
      event(3, "run.state_changed", "latest-turn", "latest-run", null, { status: "running" }),
      event(4, "item.started", "latest-turn", "latest-run", "assistant", {
        item: messageItem("assistant", "latest-turn", "latest-run", "assistant", "", "streaming"),
      }),
      event(5, "item.delta", "latest-turn", "latest-run", "assistant", { delta: "Working" }),
      event(6, "model.input_prepared", "latest-turn", "latest-run", null, { stepOrdinal: 2 }),
      event(7, "item.completed", "latest-turn", "latest-run", "assistant", {
        item: messageItem("assistant", "latest-turn", "latest-run", "assistant", "Done", "completed"),
      }),
      event(8, "run.settled", "latest-turn", "latest-run", null, { status: "completed" }),
      event(9, "run.settled", "older-turn", "older-run", null, { status: "completed" }),
    ];
    let projected = initial;
    for (const update of laterEvents) {
      projected = replayRuntimeEvents(projected, [update]);
      expect(projected[0]?.branches[0]?.turns.map((turn) => turn.modelSelection)).toEqual([
        olderSelection, latestSelection,
      ]);
    }
    expect(projected[0]?.branches[0]?.turns[1]?.events).toContainEqual(
      expect.objectContaining({ id: "assistant", content: "Done" }),
    );
  });
});
