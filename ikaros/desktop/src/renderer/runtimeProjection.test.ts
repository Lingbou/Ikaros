import { describe, expect, it } from "vitest";

import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  type RuntimeJournalEvent,
  type RuntimeThreadSummary
} from "../shared/runtime";
import {
  projectRuntimeProjects,
  projectRuntimeThreads,
  replayRuntimeEvents,
} from "./runtimeProjection";

const summary: RuntimeThreadSummary = {
  id: "thread-1",
  title: "Runtime chat",
  defaultBranchId: "branch-1",
  workspace: null,
  createdAt: "2026-08-11T12:00:00.000Z",
  updatedAt: "2026-08-11T12:00:00.000Z"
};

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
});

function event(
  seq: number,
  type: string,
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
      toolName: "process_run",
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
      toolName: "process_run",
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
          "/process.run Write-Output original-command",
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
      toolName: "process.run",
      label: { source: "app", kind: "tool.runProcess" },
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
