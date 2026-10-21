import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  parseRuntimeEventNotification,
  parseRuntimeInitializeResult,
  parseRuntimeJournalEvent,
  parseRuntimeJsonRpcResponse,
  parseRuntimeMethodResult
} from "./runtime/wire";
import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  RUNTIME_JOURNAL_EVENT_TYPES,
  RUNTIME_PROVIDER_TOOL_IDS,
  RUNTIME_RPC_METHODS,
  RUNTIME_SERVER_NAME
} from "../shared/runtime";

interface GoldenMessage {
  name: string;
  kind: "response" | "notification";
  method?: string;
  requestParams?: Record<string, unknown>;
  envelope: unknown;
}

interface GoldenTrace {
  fixtureVersion: number;
  messages: GoldenMessage[];
}

const trace = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "../../runtime/protocol/golden-trace.json"),
    "utf8"
  )
) as GoldenTrace;

function asWireObject(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} is missing`);
  }
  return value as Record<string, unknown>;
}

function cloneGoldenNotification(name: string): {
  envelope: Record<string, unknown>;
  params: Record<string, unknown>;
  payload: Record<string, unknown>;
} {
  const source = trace.messages.find((message) => message.name === name);
  if (!source || source.kind !== "notification") {
    throw new Error(`Golden notification ${name} is missing`);
  }
  const envelope = asWireObject(structuredClone(source.envelope), `${name} envelope`);
  const params = asWireObject(envelope.params, `${name} params`);
  return {
    envelope,
    params,
    payload: asWireObject(params.payload, `${name} payload`)
  };
}

describe("Runtime protocol Golden Trace", () => {
  it("parses the shared Python/Desktop envelopes with production parsers", () => {
    const observedEvents = new Set<string>();
    for (const message of trace.messages) {
      if (message.kind === "notification") {
        const notification = parseRuntimeEventNotification(message.envelope);
        const event = notification.params as { type: string };
        observedEvents.add(event.type);
        continue;
      }

      const response = parseRuntimeJsonRpcResponse(message.envelope);
      if (!("result" in response)) throw new Error(`${message.name} unexpectedly failed`);
      if (message.method === "initialize") {
        const initialized = parseRuntimeInitializeResult(response.result);
        expect(initialized.server.name).toBe(RUNTIME_SERVER_NAME);
      } else {
        if (typeof message.method !== "string") {
          throw new Error("Golden response method is missing");
        }
        expect(
          parseRuntimeMethodResult(
            message.method,
            response.result,
            message.requestParams ?? {}
          )
        ).toBeDefined();
      }
    }

    expect(trace.fixtureVersion).toBe(1);
    expect([...observedEvents].sort()).toEqual([...RUNTIME_JOURNAL_EVENT_TYPES].sort());
    expect(RUNTIME_RPC_METHODS).toHaveLength(32);
    expect(RUNTIME_PROVIDER_TOOL_IDS).toEqual([
      "process_start",
      "process_read",
      "process_wait",
      "process_stop",
      "process_write",
      "process_resize",
      "history_read",
      "thread_list",
      "thread_read",
      "read",
      "write",
      "edit",
      "glob",
      "grep",
      "list",
    ]);
  });

  it("rejects invalid JSON-RPC and notification envelopes", () => {
    for (const invalid of [
      null,
      "invalid",
      123,
      [],
      {},
      { jsonrpc: "1.0", id: 1, result: {} },
      { jsonrpc: "2.0", id: 1.5, result: {} },
      { jsonrpc: "2.0", id: "not-a-number", result: {} },
      { jsonrpc: "2.0", id: 1 },
      { jsonrpc: "2.0", id: 1, result: {}, error: { code: -1, message: "err" } },
      { jsonrpc: "2.0", id: 1, error: null },
      { jsonrpc: "2.0", id: 1, error: { code: "string", message: "err" } },
      { jsonrpc: "2.0", id: 1, error: { code: -1, message: 123 } },
      { jsonrpc: "2.0", id: 1, error: { code: -1, message: "err", extra: true } }
    ]) {
      expect(() => parseRuntimeJsonRpcResponse(invalid)).toThrow(
        "Runtime returned an invalid JSON-RPC response."
      );
    }

    for (const invalid of [
      null,
      {},
      { jsonrpc: "2.0", method: "other", params: {} },
      { jsonrpc: "1.0", method: "event", params: {} },
      { jsonrpc: "2.0", method: "event" },
      { jsonrpc: "2.0", method: "event", params: {}, extra: true }
    ]) {
      expect(() => parseRuntimeEventNotification(invalid)).toThrow(
        "Runtime returned an unsupported notification envelope."
      );
    }

    expect(() => parseRuntimeMethodResult("thread.unknown", {}, {})).toThrow(
      "Desktop does not support Runtime RPC method thread.unknown."
    );
    expect(() => parseRuntimeMethodResult("thread.list", null)).toThrow(
      "Runtime returned an invalid thread.list result."
    );
  });

  it("requires the current journal schema and rejects malformed journal event envelopes", () => {
    const valid = cloneGoldenNotification("model-input-prepared");

    for (const version of [1, 5, 6, 7, 8, 9, RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION + 1, undefined]) {
      const event = structuredClone(valid);
      (event.params as Record<string, unknown>).schemaVersion = version;
      expect(() => parseRuntimeEventNotification(event.envelope)).toThrow(
        `Runtime journal event schema ${String(version)} is unsupported.`
      );
    }

    for (const invalid of [
      null,
      {},
      { ...valid.params, seq: 0 },
      { ...valid.params, seq: -1 },
      { ...valid.params, seq: 1.5 },
      { ...valid.params, type: "" },
      { ...valid.params, type: 123 },
      { ...valid.params, timestamp: "" },
      { ...valid.params, timestamp: 123 },
      { ...valid.params, payload: null },
      { ...valid.params, payload: [] },
      { ...valid.params, payload: "string" },
      { ...valid.params, extraKey: true }
    ]) {
      expect(() => parseRuntimeJournalEvent(invalid)).toThrow(
        "Runtime returned an invalid journal event."
      );
    }
  });

  it("accepts the background process fact field and rejects a non-boolean value", () => {
    const source = cloneGoldenNotification("process-recorded-1");
    const process = asWireObject(source.payload.process, "process fact");
    process.background = true;
    expect(() => parseRuntimeEventNotification(source.envelope)).not.toThrow();

    process.background = "yes";
    expect(() => parseRuntimeEventNotification(source.envelope)).toThrow(
      "Runtime returned an invalid process.recorded journal event payload."
    );
  });

  it("passes through unknown event types and arbitrary payload shapes", () => {
    const source = cloneGoldenNotification("assistant-item-delta");

    const unknownEvent = structuredClone(source);
    unknownEvent.params.type = "future.custom_event";
    (unknownEvent.payload as Record<string, unknown>).customField = "arbitrary_data";
    (unknownEvent.payload as Record<string, unknown>).nested = { version: 99, flags: [1, 2] };

    const parsedNotification = parseRuntimeEventNotification(unknownEvent.envelope);
    expect(parsedNotification.method).toBe("event");
    const event = parsedNotification.params as {
      type: string;
      payload: Record<string, unknown>;
    };
    expect(event.type).toBe("future.custom_event");
    expect(event.payload.customField).toBe("arbitrary_data");
    expect(event.payload.nested).toEqual({ version: 99, flags: [1, 2] });

    const arbitraryResult = {
      threads: [],
      unknownFutureProperty: { deep: "value" },
      items: [1, 2, 3]
    };
    const parsedMethodResult = parseRuntimeMethodResult("thread.list", arbitraryResult);
    expect(parsedMethodResult).toEqual(arbitraryResult);
  });

  it("accepts only the registered Memory application-error envelope", () => {
    const envelope = {
      jsonrpc: "2.0",
      id: 20,
      error: {
        code: -32020,
        message: "memory operation failed",
        data: { reasonCode: "memory_forgotten" }
      }
    };

    expect(parseRuntimeJsonRpcResponse(envelope, "memory.forget")).toEqual({
      jsonrpc: "2.0",
      id: 20,
      error: {
        code: -32020,
        message: "memory operation failed",
        reasonCode: "memory_forgotten"
      }
    });
    expect(() => parseRuntimeJsonRpcResponse(envelope, "thread.get")).toThrow();
    expect(() => parseRuntimeJsonRpcResponse(envelope, "memory.list")).toThrow();
    expect(() =>
      parseRuntimeJsonRpcResponse(
        {
          ...envelope,
          error: {
            ...envelope.error,
            data: { reasonCode: "memory_source_unavailable" }
          }
        },
        "memory.forget"
      )
    ).toThrow();
    expect(() =>
      parseRuntimeJsonRpcResponse(
        {
          ...envelope,
          error: {
            ...envelope.error,
            data: { reasonCode: "memory_unknown" }
          }
        },
        "memory.forget"
      )
    ).toThrow();
    expect(() =>
      parseRuntimeJsonRpcResponse(
        {
          ...envelope,
          error: {
            ...envelope.error,
            data: { reasonCode: "memory_forgotten", memoryId: "leak" }
          }
        },
        "memory.forget"
      )
    ).toThrow();
  });
});
