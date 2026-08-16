import type { RuntimeJournalEvent } from "../shared/runtime";

function completedItem(event: RuntimeJournalEvent): Record<string, unknown> | undefined {
  if (event.type !== "item.completed") {
    return undefined;
  }
  const item = event.payload.item;
  return typeof item === "object" && item !== null
    ? (item as Record<string, unknown>)
    : undefined;
}

export function toolCallPairingIsComplete(events: RuntimeJournalEvent[]): boolean {
  const calls = new Map<
    string,
    { runId: string; callId: string; toolName: string }
  >();
  const results: Array<{
    runId: string;
    toolCallItemId: string;
    callId: string;
    toolName: string;
  }> = [];
  const resultToolCallItemIds = new Set<string>();
  let structurallyValid = true;
  for (const event of events) {
    const item = completedItem(event);
    if (!item || typeof item.data !== "object" || item.data === null) {
      continue;
    }
    const record = item.data as Record<string, unknown>;
    if (
      item.kind === "tool_call" &&
      typeof event.runId === "string" &&
      typeof item.id === "string" &&
      typeof record.callId === "string" &&
      typeof record.toolName === "string"
    ) {
      if (calls.has(item.id)) {
        structurallyValid = false;
      }
      calls.set(item.id, {
        runId: event.runId,
        callId: record.callId,
        toolName: record.toolName,
      });
    } else if (
      item.kind === "tool_result" &&
      typeof event.runId === "string" &&
      typeof record.toolCallItemId === "string" &&
      typeof record.callId === "string" &&
      typeof record.toolName === "string"
    ) {
      if (resultToolCallItemIds.has(record.toolCallItemId)) {
        structurallyValid = false;
      }
      resultToolCallItemIds.add(record.toolCallItemId);
      results.push({
        runId: event.runId,
        toolCallItemId: record.toolCallItemId,
        callId: record.callId,
        toolName: record.toolName,
      });
    }
  }
  return (
    structurallyValid &&
    calls.size === results.length &&
    results.every((result) => {
      const call = calls.get(result.toolCallItemId);
      return (
        call?.runId === result.runId &&
        call.callId === result.callId &&
        call.toolName === result.toolName
      );
    })
  );
}
