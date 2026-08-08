import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AgentEvent,
  MessageEvent,
  RunStatus,
  Thread,
  Turn,
} from "./domain";
import {
  createBranchFromMessage,
  createInitialThreads,
  freshScenarioThread,
  MockAgentClient,
} from "./mockAgentClient";

type Emission = { event: AgentEvent; status?: RunStatus };

const collect = (emissions: Emission[]) =>
  (event: AgentEvent, status?: RunStatus) => emissions.push({ event, status });

async function settle<T>(promise: Promise<T>) {
  await vi.runAllTimersAsync();
  return promise;
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
});

describe("mock thread fixtures", () => {
  it("keeps scenario seed order and returns the requested fresh fixture", () => {
    expect(createInitialThreads().slice(0, 5).map((thread) => thread.scenarioId)).toEqual([
      "streaming",
      "tools",
      "artifact",
      "permission",
      "recovery",
    ]);
    expect(freshScenarioThread("permission")).toMatchObject({
      id: "thread-permission",
      scenarioId: "permission",
    });
  });
});

describe("MockAgentClient deterministic scenarios", () => {
  it("streams cumulative message revisions in order and completes", async () => {
    const client = new MockAgentClient();
    const emissions: Emission[] = [];

    const terminal = await settle(client.playScenario("streaming", collect(emissions)));

    expect(terminal).toBe("completed");
    expect(emissions.map(({ event }) => event.id)).toEqual([
      "thread-streaming-assistant",
      "thread-streaming-assistant",
      "thread-streaming-assistant",
    ]);
    expect(emissions.map(({ status }) => status)).toEqual([
      "running",
      "running",
      "completed",
    ]);

    const messages = emissions
      .map(({ event }) => event)
      .filter((event): event is MessageEvent => event.type === "message");
    expect(messages.map(({ status }) => status)).toEqual([
      "streaming",
      "streaming",
      "complete",
    ]);
    expect(messages[1].content.startsWith(messages[0].content)).toBe(true);
    expect(messages[2].content.startsWith(messages[1].content)).toBe(true);
  });

  it("emits linked success and failure tool terminal states before completing", async () => {
    const client = new MockAgentClient();
    const emissions: Emission[] = [];

    const terminal = await settle(client.playScenario("tools", collect(emissions)));

    expect(terminal).toBe("completed");
    expect(emissions.map(({ event }) => event.id)).toEqual([
      "thread-tools-search",
      "thread-tools-search",
      "thread-tools-search-result",
      "thread-tools-extract",
      "thread-tools-extract",
      "thread-tools-extract-result",
      "thread-tools-assistant",
    ]);
    expect(emissions.map(({ event }) => event.type)).toEqual([
      "tool_call",
      "tool_call",
      "tool_result",
      "tool_call",
      "tool_call",
      "tool_result",
      "message",
    ]);
    expect(emissions.map(({ status }) => status)).toEqual([
      "running",
      "running",
      "running",
      "running",
      "running",
      "running",
      "completed",
    ]);
    expect(emissions[0].event).toMatchObject({
      type: "tool_call",
      toolName: "archive.inspect",
      label: { source: "app", kind: "tool.inspectArchiveMetadata" },
      arguments: { path: "sources/field-notes.zip", verifyChecksums: true },
      status: "running",
    });
    expect(emissions[1].event).toMatchObject({ type: "tool_call", status: "success" });
    expect(emissions[2].event).toMatchObject({
      type: "tool_result",
      toolCallId: "thread-tools-search",
      status: "success",
      summary: { source: "app", kind: "result.archiveManifestVerified" },
      output: "42 entries · SHA-256 manifest valid · last updated 2026-08-04",
    });
    expect(emissions[3].event).toMatchObject({
      type: "tool_call",
      toolName: "workspace.extract",
      label: { source: "app", kind: "tool.testArchiveExtraction" },
      arguments: { destination: ".ikaros/tmp/source-preview", mode: "read-only" },
      status: "running",
    });
    expect(emissions[4].event).toMatchObject({ type: "tool_call", status: "error" });
    expect(emissions[5].event).toMatchObject({
      type: "tool_result",
      toolCallId: "thread-tools-extract",
      status: "error",
      summary: { source: "app", kind: "result.windowsReservedNames" },
      output:
        "EINVAL: reserved device name `con.txt`\nThe archive is intact; extraction stopped before writing files.",
    });
  });

  it("stops the permission scenario at a pending request", async () => {
    const client = new MockAgentClient();
    const emissions: Emission[] = [];

    const terminal = await settle(client.playScenario("permission", collect(emissions)));

    expect(terminal).toBe("waiting_permission");
    expect(emissions).toHaveLength(1);
    expect(emissions[0]).toMatchObject({
      status: "waiting_permission",
      event: {
        id: "thread-permission-permission",
        type: "permission_request",
        requestId: "permission-move-receipts",
        title: {
          source: "app",
          kind: "permission.moveReceiptsTitle",
          values: { count: 18 },
        },
        description: {
          source: "app",
          kind: "permission.moveReceiptsDescription",
        },
        resource: "C:\\Users\\demo\\Downloads\\receipts\\*.pdf",
        status: "pending",
      },
    });
  });

  it("ends the recovery scenario in an interrupted, recoverable state", async () => {
    const client = new MockAgentClient();
    const emissions: Emission[] = [];

    const terminal = await settle(client.playScenario("recovery", collect(emissions)));

    expect(terminal).toBe("interrupted");
    expect(emissions.map(({ event }) => event.type)).toEqual([
      "tool_call",
      "tool_call",
      "interrupt",
    ]);
    expect(emissions.map(({ status }) => status)).toEqual([
      "running",
      "interrupted",
      "interrupted",
    ]);
    expect(emissions[1].event).toMatchObject({
      id: "thread-recovery-export",
      type: "tool_call",
      label: { source: "app", kind: "tool.exportReportPackage" },
      status: "interrupted",
      durationMs: 540,
    });
    expect(emissions[2].event).toMatchObject({
      id: "thread-recovery-interrupt",
      type: "interrupt",
      copy: { source: "app", kind: "worker_disconnected" },
      recoverable: true,
      status: "interrupted",
    });
  });

  it("emits artifact and file records before its final assistant message", async () => {
    const client = new MockAgentClient();
    const emissions: Emission[] = [];

    const terminal = await settle(client.playScenario("artifact", collect(emissions)));

    expect(terminal).toBe("completed");
    expect(emissions.map(({ event }) => event.type)).toEqual([
      "status",
      "artifact",
      "file_change",
      "status",
      "message",
    ]);
    expect(emissions.map(({ status }) => status)).toEqual([
      "running",
      "running",
      "running",
      "running",
      "completed",
    ]);
    expect(emissions[1].event).toMatchObject({
      id: "thread-artifact-artifact",
      type: "artifact",
      artifactId: "artifact-weekly-brief",
      mediaType: "text/markdown",
      version: 1,
    });
    expect(emissions[2].event).toMatchObject({
      id: "thread-artifact-file",
      type: "file_change",
      operation: "created",
      deletions: 0,
    });
    expect(emissions[0].event).toMatchObject({
      id: "thread-artifact-status",
      type: "status",
      tone: "neutral",
      label: {
        source: "app",
        kind: "status.buildingBrief",
        values: { count: 12 },
      },
      detail: { source: "app", kind: "status.sourcesLinked" },
    });
    expect(emissions[3].event).toMatchObject({
      id: "thread-artifact-status",
      type: "status",
      tone: "success",
      label: {
        source: "app",
        kind: "status.briefBuilt",
        values: { count: 12 },
      },
    });
  });
});

describe("MockAgentClient control flows", () => {
  it.each(["allow", "deny"] as const)(
    "resolves a pending permission with the %s terminal path",
    async (decision) => {
      const client = new MockAgentClient();
      const pending: Emission[] = [];
      const pendingTerminal = await settle(
        client.playScenario("permission", collect(pending)),
      );
      expect(pendingTerminal).toBe("waiting_permission");

      const emissions: Emission[] = [];
      const terminal = await settle(
        client.resolvePermission(
          decision,
          "thread-permission-turn",
          collect(emissions),
        ),
      );

      expect(terminal).toBe("completed");
      expect(emissions).toHaveLength(2);
      expect(emissions[0].event).toMatchObject({
        id: "thread-permission-permission",
        type: "permission_request",
        title: {
          source: "app",
          kind: "permission.moveReceiptsTitle",
          values: { count: 18 },
        },
        description: {
          source: "app",
          kind: "permission.moveReceiptsDescription",
        },
        status: decision === "allow" ? "allowed" : "denied",
      });
      expect(emissions[0].status).toBe("running");
      expect(emissions[1]).toMatchObject({
        status: "completed",
        event: {
          id: `permission-${decision}-assistant`,
          type: "message",
          role: "assistant",
        },
      });
    },
  );

  it.each(["retry", "resume"] as const)(
    "recovers an interrupted run using %s",
    async (strategy) => {
      const client = new MockAgentClient();
      const interrupted: Emission[] = [];
      expect(
        await settle(client.playScenario("recovery", collect(interrupted))),
      ).toBe("interrupted");

      const emissions: Emission[] = [];
      const terminal = await settle(
        client.recover(strategy, "thread-recovery-turn", collect(emissions)),
      );

      expect(terminal).toBe("completed");
      expect(emissions.map(({ event }) => event.type)).toEqual([
        "interrupt",
        "tool_call",
        "status",
        "interrupt",
        "message",
      ]);
      expect(emissions.map(({ status }) => status)).toEqual([
        "running",
        "running",
        "running",
        "running",
        "completed",
      ]);
      expect(emissions[0].event).toMatchObject({
        id: "thread-recovery-interrupt",
        type: "interrupt",
        copy: {
          source: "app",
          kind: strategy === "resume" ? "recovering_checkpoint" : "retrying_export",
        },
        status: "recovering",
      });
      expect(emissions[1].event).toMatchObject({
        id: "thread-recovery-export",
        type: "tool_call",
        label: { source: "app", kind: "tool.exportReportPackage" },
        status: "success",
      });
      expect(emissions[2].event).toMatchObject({
        id: `recovery-${strategy}-status`,
        type: "status",
        label: {
          source: "app",
          kind:
            strategy === "resume"
              ? "status.checkpointRestored"
              : "status.exportStepRetried",
        },
        detail: { source: "app", kind: "status.outputsValidated" },
      });
      expect(emissions[3].event).toMatchObject({
        id: "thread-recovery-interrupt",
        type: "interrupt",
        copy: { source: "app", kind: "recovery_completed" },
        recoverable: false,
        status: "recovered",
      });
      expect(emissions[4].event).toMatchObject({
        id: `recovery-${strategy}-assistant`,
        type: "message",
        status: "complete",
      });
    },
  );

  it("cancels after an emitted streaming revision without leaking later revisions", async () => {
    const client = new MockAgentClient();
    const emissions: Emission[] = [];
    const run = client.playScenario("streaming", collect(emissions));

    await vi.advanceTimersByTimeAsync(180);
    expect(emissions).toHaveLength(1);
    const firstRevision = emissions[0].event;

    client.cancel();
    await vi.runAllTimersAsync();

    expect(await run).toBe("interrupted");
    expect(emissions).toEqual([{ event: firstRevision, status: "running" }]);
  });
});

describe("createBranchFromMessage", () => {
  it("preserves the source history and truncates only the new branch after the edit", () => {
    const fixture = freshScenarioThread("streaming");
    const main = fixture.branches[0];
    const firstTurn = main.turns[0];
    const sourceEvent = firstTurn.events[0];
    const assistantAfterEdit: AgentEvent = {
      id: "assistant-after-source",
      turnId: firstTurn.id,
      type: "message",
      role: "assistant",
      content: "This belongs only to the original history.",
      status: "complete",
      createdAt: fixture.updatedAt,
    };
    const laterTurn: Turn = {
      id: "later-turn",
      branchId: main.id,
      status: "completed",
      events: [
        {
          id: "later-user",
          turnId: "later-turn",
          type: "message",
          role: "user",
          content: "A later message",
          status: "complete",
          createdAt: fixture.updatedAt,
        },
      ],
    };
    const source: Thread = {
      ...fixture,
      branches: [
        {
          ...main,
          turns: [
            { ...firstTurn, status: "completed", events: [sourceEvent, assistantAfterEdit] },
            laterTurn,
          ],
        },
      ],
    };

    const branched = createBranchFromMessage(
      source,
      sourceEvent.id,
      "A replacement message",
      "thread-streaming-branch-1",
    );

    expect(source.branches).toHaveLength(1);
    expect(source.branches[0].turns).toHaveLength(2);
    expect(source.branches[0].turns[0].events).toEqual([
      sourceEvent,
      assistantAfterEdit,
    ]);

    expect(branched.branches).toHaveLength(2);
    expect(branched.activeBranchId).toBe("thread-streaming-branch-1");
    const branch = branched.branches[1];
    expect(branch).toMatchObject({
      id: "thread-streaming-branch-1",
      parentBranchId: main.id,
      forkedFromEventId: sourceEvent.id,
      label: { source: "app", kind: "number", number: 1 },
    });
    expect(branch.turns).toHaveLength(1);
    expect(branch.turns[0].events).toHaveLength(2);
    expect(branch.turns[0].events[0]).toMatchObject({
      id: sourceEvent.id,
      type: "message",
      content: "A replacement message",
    });
    expect(branch.turns[0].events[1]).toMatchObject({
      id: "thread-streaming-branch-1-created",
      type: "branch_created",
      sourceEventId: sourceEvent.id,
      copy: { source: "app", kind: "edited_message" },
    });
  });

  it("returns the original thread when the source message does not exist", () => {
    const thread = freshScenarioThread("streaming");

    expect(
      createBranchFromMessage(thread, "missing-event", "replacement", "branch-missing"),
    ).toBe(thread);
  });
});
