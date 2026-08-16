import { expect, it } from "vitest";

import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  type RuntimeJournalEvent,
} from "../shared/runtime";
import { toolCallPairingIsComplete } from "./runtimeStore.liveEvidence";

function toolEvidenceEvent(
  seq: number,
  runId: string,
  item: Record<string, unknown>,
): RuntimeJournalEvent {
  return {
    seq,
    schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type: "item.completed",
    threadId: null,
    branchId: null,
    turnId: null,
    runId,
    itemId: typeof item.id === "string" ? item.id : null,
    timestamp: "2026-08-17T00:00:00Z",
    payload: { item },
  };
}

it("pairs Tool Results by Item identity when call IDs repeat across Steps", () => {
  const runId = "run_pairing";
  const firstCall = toolEvidenceEvent(1, runId, {
    id: "item_call_first",
    kind: "tool_call",
    data: { callId: "call_reused", toolName: "read" },
  });
  const secondCall = toolEvidenceEvent(2, runId, {
    id: "item_call_second",
    kind: "tool_call",
    data: { callId: "call_reused", toolName: "read" },
  });
  const firstResult = toolEvidenceEvent(3, runId, {
    id: "item_result_first",
    kind: "tool_result",
    data: {
      toolCallItemId: "item_call_first",
      callId: "call_reused",
      toolName: "read",
    },
  });
  const secondResult = toolEvidenceEvent(4, runId, {
    id: "item_result_second",
    kind: "tool_result",
    data: {
      toolCallItemId: "item_call_second",
      callId: "call_reused",
      toolName: "read",
    },
  });

  expect(
    toolCallPairingIsComplete([firstCall, secondCall, firstResult, secondResult]),
  ).toBe(true);
  expect(
    toolCallPairingIsComplete([firstCall, secondCall, firstResult, firstResult]),
  ).toBe(false);
});
