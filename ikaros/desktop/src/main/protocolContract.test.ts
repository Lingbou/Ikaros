import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  parseRuntimeEventNotification,
  parseRuntimeInitializeResult,
  parseRuntimeJsonRpcResponse,
  parseRuntimeMethodResult
} from "./runtime/wire";
import {
  RUNTIME_JOURNAL_EVENT_TYPES,
  RUNTIME_PROVIDER_TOOL_IDS,
  RUNTIME_RPC_METHODS
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
        expect(initialized.capabilities.tools).toEqual(RUNTIME_PROVIDER_TOOL_IDS);
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
    expect(RUNTIME_RPC_METHODS).toHaveLength(21);
    expect(RUNTIME_PROVIDER_TOOL_IDS).toContain("process_run");
    expect(RUNTIME_PROVIDER_TOOL_IDS).not.toContain("process.run");
  });

  it("rejects unknown event types and malformed discriminated payloads", () => {
    const source = trace.messages.find((message) => message.name === "assistant-item-delta");
    if (!source || typeof source.envelope !== "object" || source.envelope === null) {
      throw new Error("Golden delta notification is missing");
    }
    const unknown = structuredClone(source.envelope) as {
      params: { type: string; payload: Record<string, unknown> };
    };
    unknown.params.type = "item.unknown";
    expect(() => parseRuntimeEventNotification(unknown)).toThrow();

    const malformed = structuredClone(source.envelope) as {
      params: { type: string; payload: Record<string, unknown> };
    };
    malformed.params.payload = { status: "completed" };
    expect(() => parseRuntimeEventNotification(malformed)).toThrow();
    expect(() => parseRuntimeMethodResult("thread.unknown", {}, {})).toThrow(
      /does not support/
    );
  });
});
