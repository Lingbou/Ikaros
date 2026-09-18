import { describe, expect, it } from "vitest";

import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  type RuntimeRunHistory,
  type RuntimeTurnListPage,
} from "../../shared/runtime";
import { parseRuntimeJournalEvent, parseRuntimeTurnListPage } from "./wire";

const timestamp = "2026-09-05T00:00:00.000Z";

function page(status: RuntimeRunHistory["status"], reasonCode: string | null): RuntimeTurnListPage {
  return {
    turns: [{
      id: "turn-1",
      threadId: "thread-1",
      branchId: "branch-1",
      ordinal: 1,
      status,
      createdAt: timestamp,
      updatedAt: timestamp,
      runs: [{
        id: "run-1",
        turnId: "turn-1",
        providerId: "provider-1",
        modelId: "model-1",
        executionPolicy: "full_access",
        startedAt: status === "queued" ? null : timestamp,
        modelCalls: 0,
        compactions: 0,
        status,
        reasonCode,
        createdAt: timestamp,
        settledAt: status === "queued" || status === "running" ? null : timestamp,
        items: [],
      }],
    }],
    nextCursor: null,
    hasMore: false,
    snapshotSeq: 1,
  };
}

describe("Run outcome wire contract", () => {
  it.each(["failed", "cancelled"] as const)("preserves %s reasons without enumerating codes", (status) => {
    const value = page(status, "future_provider.reason-2");
    expect(parseRuntimeTurnListPage(value)).toEqual(value);
    expect(parseRuntimeTurnListPage(page(status, null)).turns[0]?.runs[0]?.reasonCode).toBeNull();
  });

  it("rejects history missing its current reasonCode field", () => {
    const value = page("failed", null);
    const run = value.turns[0]!.runs[0]! as Partial<RuntimeRunHistory>;
    delete run.reasonCode;
    expect(() => parseRuntimeTurnListPage(value)).toThrow("invalid Turn history page");
  });

  it.each(["queued", "running", "completed"] as const)("requires null reason for %s", (status) => {
    expect(parseRuntimeTurnListPage(page(status, null)).turns[0]?.runs[0]?.reasonCode).toBeNull();
    expect(() => parseRuntimeTurnListPage(page(status, "provider_timeout"))).toThrow(
      "invalid Turn history page",
    );
  });

  it.each(["", "x".repeat(201), "bad\nreason", "provider error", "<private>", 42, false, {}])(
    "rejects invalid historical reason %j",
    (reason) => {
      const value = page("failed", null);
      Object.assign(value.turns[0]!.runs[0]!, { reasonCode: reason });
      expect(() => parseRuntimeTurnListPage(value)).toThrow("invalid Turn history page");
    },
  );

  it("accepts future live failure identifiers and rejects unrestricted diagnostic text", () => {
    const event = {
      seq: 1,
      schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
      type: "run.settled",
      threadId: "thread-1",
      branchId: "branch-1",
      turnId: "turn-1",
      runId: "run-1",
      itemId: null,
      timestamp,
      payload: {
        turnId: "turn-1",
        runId: "run-1",
        status: "failed",
        settledAt: timestamp,
        reasonCode: "provider_future-category",
      },
    };
    expect(parseRuntimeJournalEvent(event)).toEqual(event);
    expect(() => parseRuntimeJournalEvent({
      ...event,
      payload: { ...event.payload, reasonCode: "provider secret diagnostic" },
    })).toThrow("invalid run.settled journal event payload");
  });
});
