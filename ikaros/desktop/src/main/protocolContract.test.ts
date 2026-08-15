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

function asWireObject(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} is missing`);
  }
  return value as Record<string, unknown>;
}

function asWireArray(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`${label} is missing`);
  return value;
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

function cloneGoldenResponse(name: string): {
  envelope: Record<string, unknown>;
  result: Record<string, unknown>;
  method: string;
  requestParams: Record<string, unknown>;
} {
  const source = trace.messages.find((message) => message.name === name);
  if (!source || source.kind !== "response" || typeof source.method !== "string") {
    throw new Error(`Golden response ${name} is missing`);
  }
  const envelope = asWireObject(structuredClone(source.envelope), `${name} envelope`);
  return {
    envelope,
    result: asWireObject(envelope.result, `${name} result`),
    method: source.method,
    requestParams: structuredClone(source.requestParams ?? {})
  };
}

function expectGoldenMutationRejected(
  name: string,
  mutate: (event: ReturnType<typeof cloneGoldenNotification>) => void
): void {
  const event = cloneGoldenNotification(name);
  mutate(event);
  expect(() => parseRuntimeEventNotification(event.envelope)).toThrow();
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

  it("rejects initial Turn fields that disagree with the frozen Submission Frame", () => {
    const mutations: Array<
      (event: ReturnType<typeof cloneGoldenNotification>) => void
    > = [
      ({ payload }) => {
        asWireObject(payload.item, "initial Item").ordinal = 2;
      },
      ({ payload }) => {
        asWireObject(payload.item, "initial Item").id = "item_other";
      },
      ({ payload }) => {
        asWireObject(payload.run, "initial Run").providerId = "other-provider";
      },
      ({ payload }) => {
        asWireObject(payload.run, "initial Run").modelId = "other-model";
      },
      ({ payload }) => {
        asWireObject(payload.run, "initial Run").executionPolicy = "ask";
      },
      ({ payload }) => {
        asWireObject(payload.run, "initial Run").skills = [
          {
            name: "sample-skill",
            description: "A sample skill.",
            location: "C:/skills/sample-skill"
          }
        ];
      },
      ({ payload }) => {
        asWireObject(payload.submissionFrame, "Submission Frame").maxSteps = 17;
      }
    ];

    for (const mutate of mutations) {
      expectGoldenMutationRejected("initial-user-item-completed", mutate);
    }
  });

  it("rejects Gate 2 future input slots and an empty current-Run history", () => {
    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      const frame = asWireObject(payload.submissionFrame, "Submission Frame");
      const instructions = asWireObject(frame.instructions, "Frame instructions");
      instructions.identityCore = {
        id: "identity-core",
        version: 1,
        source: "ikaros-runtime:identity-core-v1",
        authority: "runtime_identity",
        scope: "global",
        lifetime: "release",
        content: "Ikaros identity"
      };
      const manifest = asWireObject(payload.runManifest, "Run Manifest");
      asWireArray(manifest.instructions, "manifest instructions").push({
        id: "identity-core",
        version: 1,
        source: "ikaros-runtime:identity-core-v1",
        authority: "runtime_identity",
        scope: "global",
        lifetime: "release",
        characters: 15,
        contentSha256: "c987864c48fd568bd0fa866ef1879f7db19ad7b36b1424be101125d1d6a75601"
      });
    });

    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      const frame = asWireObject(payload.submissionFrame, "Submission Frame");
      const contextData = asWireObject(frame.contextData, "Frame Context Data");
      contextData.memory = [
        {
          memoryId: "memory_1",
          revision: 1,
          contentSha256: "a".repeat(64),
          scope: "global"
        }
      ];
    });

    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const snapshot = asWireObject(payload.contextSnapshot, "Context Snapshot");
      snapshot.historyGroups = [];
      snapshot.historyItems = [];
      const snapshotBudget = asWireObject(snapshot.budget, "Context budget");
      const snapshotCurrent = snapshotBudget.currentRunCharacters as number;
      snapshotBudget.currentRunCharacters = 0;
      snapshotBudget.totalCharacters =
        (snapshotBudget.totalCharacters as number) - snapshotCurrent;

      const manifest = asWireObject(payload.stepManifest, "Step Manifest");
      manifest.historyItems = [];
      const manifestBudget = asWireObject(manifest.budget, "Step budget");
      const manifestCurrent = manifestBudget.currentRunCharacters as number;
      manifestBudget.currentRunCharacters = 0;
      manifestBudget.totalCharacters =
        (manifestBudget.totalCharacters as number) - manifestCurrent;
    });
  });

  it("requires the Run Manifest to be the exact Python from_frame projection", () => {
    const instructionFields: Array<[string, unknown]> = [
      ["id", "other-instruction"],
      ["version", 2],
      ["source", "other-source"],
      ["authority", "runtime_identity"],
      ["scope", "run"],
      ["lifetime", "run"],
      ["characters", 999],
      ["contentSha256", "b".repeat(64)]
    ];
    for (const [field, replacement] of instructionFields) {
      expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
        const manifest = asWireObject(payload.runManifest, "Run Manifest");
        const instructions = asWireArray(manifest.instructions, "manifest instructions");
        asWireObject(instructions[0], "output-style manifest")[field] = replacement;
      });
    }

    const unicodeInstruction = cloneGoldenNotification("initial-user-item-completed");
    const unicodeFrame = asWireObject(
      unicodeInstruction.payload.submissionFrame,
      "Submission Frame"
    );
    const unicodeFrameInstructions = asWireObject(
      unicodeFrame.instructions,
      "Frame instructions"
    );
    asWireObject(unicodeFrameInstructions.outputStyle, "output-style instruction").content =
      "A\u{20000}";
    const unicodeManifest = asWireObject(
      unicodeInstruction.payload.runManifest,
      "Run Manifest"
    );
    const unicodeManifestInstructions = asWireArray(
      unicodeManifest.instructions,
      "manifest instructions"
    );
    const unicodeOutputStyle = asWireObject(
      unicodeManifestInstructions[0],
      "output-style manifest"
    );
    unicodeOutputStyle.characters = 2;
    unicodeOutputStyle.contentSha256 =
      "0d8905c5a520971b729f4656d07ae5d78f59d52197d8c91ab9a49202552f31ac";
    expect(() => parseRuntimeEventNotification(unicodeInstruction.envelope)).toThrow();

    const event = cloneGoldenNotification("initial-user-item-completed");
    const run = asWireObject(event.payload.run, "initial Run");
    const frame = asWireObject(event.payload.submissionFrame, "Submission Frame");
    const frameInstructions = asWireObject(frame.instructions, "Frame instructions");
    const manifest = asWireObject(event.payload.runManifest, "Run Manifest");
    const skill = {
      name: "sample-skill",
      description: "A sample skill.",
      location: "C:/skills/sample-skill"
    };
    const tool = {
      name: "process_run",
      description: "Run one process.",
      inputSchema: { type: "object", properties: {} },
      definitionSha256: "3e85e37230961a0f86ee6597987f408ae3063a8ff691301926facd1706461ca9"
    };
    run.skills = [skill];
    frame.skills = [structuredClone(skill)];
    frame.tools = [tool];
    frameInstructions.skillCatalog = {
      id: "skill-catalog",
      version: 1,
      source: "run:skill-descriptors",
      authority: "runtime_instruction",
      scope: "run",
      lifetime: "run",
      content:
        "Skills provide optional instructions for specialized tasks. When a Skill is relevant, " +
        "use the read tool to load its SKILL.md from the listed location before following it.\n" +
        "<available_skills>\n" +
        "  <skill>\n" +
        "    <name>sample-skill</name>\n" +
        "    <description>A sample skill.</description>\n" +
        "    <location>C:/skills/sample-skill</location>\n" +
        "  </skill>\n" +
        "</available_skills>"
    };
    asWireArray(manifest.instructions, "manifest instructions").push({
      id: "skill-catalog",
      version: 1,
      source: "run:skill-descriptors",
      authority: "runtime_instruction",
      scope: "run",
      lifetime: "run",
      characters: 355,
      contentSha256: "06db0276ac3b356767e3f2e3a6e43061d28b53517460d0719f7e9cb4627b2911"
    });
    manifest.skills = [
      {
        name: "sample-skill",
        descriptorSha256: "8fda94079f8c2a01bc7d6bb8fac72a42fa66e89a5d2073b503dc0e85c6b4e04a"
      }
    ];
    manifest.tools = [
      {
        name: "process_run",
        definitionSha256: tool.definitionSha256
      }
    ];
    expect(() => parseRuntimeEventNotification(event.envelope)).not.toThrow();

    const corruptedCatalog = structuredClone(event);
    const corruptedFrame = asWireObject(
      corruptedCatalog.payload.submissionFrame,
      "Submission Frame"
    );
    const corruptedInstructions = asWireObject(
      corruptedFrame.instructions,
      "Frame instructions"
    );
    asWireObject(corruptedInstructions.skillCatalog, "Skill catalog").content =
      "injected catalog";
    expect(() => parseRuntimeEventNotification(corruptedCatalog.envelope)).toThrow();

    for (const mutate of [
      (copy: ReturnType<typeof cloneGoldenNotification>) => {
        const copiedFrame = asWireObject(copy.payload.submissionFrame, "Submission Frame");
        const copiedTools = asWireArray(copiedFrame.tools, "Frame Tools");
        asWireObject(copiedTools[0], "Frame Tool").definitionSha256 = "b".repeat(64);
      },
      (copy: ReturnType<typeof cloneGoldenNotification>) => {
        const copiedManifest = asWireObject(copy.payload.runManifest, "Run Manifest");
        const copiedSkills = asWireArray(copiedManifest.skills, "manifest Skills");
        asWireObject(copiedSkills[0], "manifest Skill").descriptorSha256 = "b".repeat(64);
      },
      (copy: ReturnType<typeof cloneGoldenNotification>) => {
        const copiedManifest = asWireObject(copy.payload.runManifest, "Run Manifest");
        const copiedTools = asWireArray(copiedManifest.tools, "manifest Tools");
        asWireObject(copiedTools[0], "manifest Tool").definitionSha256 = "b".repeat(64);
      }
    ]) {
      const copy = structuredClone(event) as ReturnType<typeof cloneGoldenNotification>;
      mutate(copy);
      expect(() => parseRuntimeEventNotification(copy.envelope)).toThrow();
    }
  });

  it("accepts a real second Tool Step but rejects non-prefix or inconsistent manifests", () => {
    const secondStep = cloneGoldenNotification("model-input-prepared-step-2");
    const manifest = asWireObject(secondStep.payload.stepManifest, "Step Manifest");
    const historyItems = asWireArray(manifest.historyItems, "Step history Items");
    const budget = asWireObject(manifest.budget, "Step budget");
    const originalCurrentRunCharacters = budget.currentRunCharacters as number;
    const originalTotalCharacters = budget.totalCharacters as number;
    expect(() => parseRuntimeEventNotification(secondStep.envelope)).not.toThrow();

    const nonPrefix = structuredClone(secondStep);
    const nonPrefixManifest = asWireObject(nonPrefix.payload.stepManifest, "Step Manifest");
    const nonPrefixItems = asWireArray(nonPrefixManifest.historyItems, "Step history Items");
    [nonPrefixItems[0], nonPrefixItems[1]] = [nonPrefixItems[1], nonPrefixItems[0]];
    expect(() => parseRuntimeEventNotification(nonPrefix.envelope)).toThrow();

    const inconsistentBudget = structuredClone(secondStep);
    const inconsistentManifest = asWireObject(
      inconsistentBudget.payload.stepManifest,
      "Step Manifest"
    );
    const changedBudget = asWireObject(inconsistentManifest.budget, "Step budget");
    changedBudget.currentRunCharacters = originalCurrentRunCharacters - 1;
    changedBudget.totalCharacters = originalTotalCharacters - 1;
    expect(() => parseRuntimeEventNotification(inconsistentBudget.envelope)).toThrow();

    const foreignExtraItem = structuredClone(secondStep);
    const foreignManifest = asWireObject(foreignExtraItem.payload.stepManifest, "Step Manifest");
    const foreignItems = asWireArray(foreignManifest.historyItems, "Step history Items");
    asWireObject(foreignItems[1], "extra Step Item").runId = "run_other";
    expect(() => parseRuntimeEventNotification(foreignExtraItem.envelope)).toThrow();

    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const changedManifest = asWireObject(payload.stepManifest, "Step Manifest");
      changedManifest.memory = [
        {
          memoryId: "memory_other",
          revision: 1,
          contentSha256: "b".repeat(64),
          scope: "global"
        }
      ];
      const changedMemoryBudget = asWireObject(changedManifest.budget, "Step budget");
      changedMemoryBudget.memoryCharacters = 1;
      changedMemoryBudget.totalCharacters = originalTotalCharacters + 1;
    });
    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const changedManifest = asWireObject(payload.stepManifest, "Step Manifest");
      changedManifest.omissions = [
        { sourceType: "history", sourceId: "item_other", reason: "omitted_by_budget" }
      ];
    });
  });

  it("rejects incomplete, unknown, or internally inconsistent input budgets", () => {
    for (const key of [
      "measurementVersion",
      "maximumCharacters",
      "reservedCurrentRunCharacters",
      "instructionCharacters",
      "contextDataCharacters",
      "toolCharacters"
    ]) {
      expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
        const snapshot = asWireObject(payload.contextSnapshot, "Context Snapshot");
        const budget = asWireObject(snapshot.budget, "Context budget");
        delete budget[key];
      });
    }

    for (const [key, value] of [
      ["mode", "unknown"],
      ["measurementVersion", "unknown-v1"],
      ["maximumCharacters", 1],
      ["reservedCurrentRunCharacters", 1]
    ] as const) {
      expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
        const snapshot = asWireObject(payload.contextSnapshot, "Context Snapshot");
        asWireObject(snapshot.budget, "Context budget")[key] = value;
      });
    }

    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const snapshot = asWireObject(payload.contextSnapshot, "Context Snapshot");
      const budget = asWireObject(snapshot.budget, "Context budget");
      budget.totalCharacters = (budget.totalCharacters as number) + 1;
    });

    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const manifest = asWireObject(payload.stepManifest, "Step Manifest");
      const budget = asWireObject(manifest.budget, "Step budget");
      budget.instructionCharacters = (budget.instructionCharacters as number) + 1;
      budget.totalCharacters = (budget.totalCharacters as number) + 1;
    });
  });

  it("rejects empty Context Snapshot history groups", () => {
    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const snapshot = asWireObject(payload.contextSnapshot, "Context Snapshot");
      asWireArray(snapshot.historyGroups, "history groups").push({
        turnId: "turn_empty",
        itemIds: []
      });
    });
  });

  it("rejects History Item references whose role does not match their kind", () => {
    for (const [kind, role] of [
      ["message", "tool"],
      ["tool_call", "user"],
      ["tool_call", "tool"],
      ["tool_result", "user"],
      ["tool_result", "assistant"]
    ] as const) {
      expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
        const manifest = asWireObject(payload.stepManifest, "Step Manifest");
        const historyItems = asWireArray(manifest.historyItems, "Step history Items");
        const item = asWireObject(historyItems[0], "Step history Item");
        item.kind = kind;
        item.role = role;
      });
    }
  });

  it("rejects malformed or scope-inconsistent canonical Tool Results", () => {
    const canonical = cloneGoldenNotification("tool-result-completed");
    const canonicalItem = asWireObject(canonical.payload.item, "Tool Result Item");
    const canonicalData = asWireObject(canonicalItem.data, "Tool Result data");
    expect(() => parseRuntimeEventNotification(canonical.envelope)).not.toThrow();

    const canonicalResult = asWireObject(canonicalData.result, "Tool Result");
    canonicalItem.content = JSON.stringify({
      truncated: canonicalResult.truncated,
      timedOut: canonicalResult.timedOut,
      durationMs: canonicalResult.durationMs,
      exitCode: canonicalResult.exitCode,
      cwd: canonicalResult.cwd,
      stderr: canonicalResult.stderr,
      stdout: canonicalResult.stdout,
      cancelled: canonicalResult.cancelled,
      output: canonicalResult.output,
      ok: canonicalResult.ok,
      toolName: canonicalResult.toolName,
      toolCallId: canonicalResult.toolCallId
    });
    expect(() => parseRuntimeEventNotification(canonical.envelope)).not.toThrow();

    const mutations: Array<(result: Record<string, unknown>) => void> = [
      (result) => {
        delete result.cancelled;
      },
      (result) => {
        result.toolCallId = "call_other";
      },
      (result) => {
        result.toolName = "read";
      },
      (result) => {
        result.ok = false;
      },
      (result) => {
        result.output = 42;
      },
      (result) => {
        result.cancelled = true;
      },
      (result) => {
        result.unexpected = "not canonical";
      }
    ];
    for (const mutate of mutations) {
      const changed = structuredClone(canonical);
      const item = asWireObject(changed.payload.item, "Tool Result Item");
      const data = asWireObject(item.data, "Tool Result data");
      const result = asWireObject(data.result, "Tool Result");
      mutate(result);
      item.content = JSON.stringify(result);
      expect(() => parseRuntimeEventNotification(changed.envelope)).toThrow();
    }

    const nakedContent = structuredClone(canonical);
    asWireObject(nakedContent.payload.item, "Tool Result Item").content = "golden";
    expect(() => parseRuntimeEventNotification(nakedContent.envelope)).toThrow();

    const inconsistentContent = structuredClone(canonical);
    const inconsistentItem = asWireObject(inconsistentContent.payload.item, "Tool Result Item");
    const inconsistentData = asWireObject(inconsistentItem.data, "Tool Result data");
    const inconsistentResult = asWireObject(inconsistentData.result, "Tool Result");
    inconsistentItem.content = JSON.stringify({ ...inconsistentResult, output: "other" });
    expect(() => parseRuntimeEventNotification(inconsistentContent.envelope)).toThrow();

    const crossToolDetail = cloneGoldenNotification("tool-result-completed");
    const crossToolItem = asWireObject(crossToolDetail.payload.item, "Tool Result Item");
    const crossToolData = asWireObject(crossToolItem.data, "Tool Result data");
    const crossToolResult = asWireObject(crossToolData.result, "Tool Result");
    crossToolResult.path = "C:/not-a-process-result";
    crossToolItem.content = JSON.stringify(crossToolResult);
    expect(() => parseRuntimeEventNotification(crossToolDetail.envelope)).toThrow();
  });

  it("rejects unknown top-level fields in paginated Item history", () => {
    const response = cloneGoldenResponse("turn-list-page");
    const turns = asWireArray(response.result.turns, "Turn history");
    const runs = asWireArray(asWireObject(turns[0], "Turn").runs, "Run history");
    const items = asWireArray(asWireObject(runs[0], "Run").items, "Item history");
    asWireObject(items[0], "Item").unexpected = true;

    expect(() =>
      parseRuntimeMethodResult(response.method, response.result, response.requestParams)
    ).toThrow();
  });

  it("rejects Token usage sub-counts larger than their parent counts", () => {
    expectGoldenMutationRejected("model-response-finished", ({ payload }) => {
      const usage = asWireObject(payload.usage, "model usage");
      usage.cachedInputTokens = (usage.inputTokens as number) + 1;
    });
    expectGoldenMutationRejected("model-response-finished", ({ payload }) => {
      const usage = asWireObject(payload.usage, "model usage");
      usage.reasoningOutputTokens = (usage.outputTokens as number) + 1;
    });
  });
});
