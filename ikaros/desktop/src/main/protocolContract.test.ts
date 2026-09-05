import { createHash } from "node:crypto";
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

function attachTestIdentityCore(
  event: ReturnType<typeof cloneGoldenNotification>,
  content = "Ikaros test identity"
): {
  block: Record<string, unknown>;
  manifest: Record<string, unknown>;
} {
  const block: Record<string, unknown> = {
    id: "ikaros-identity",
    version: 1,
    source: "ikaros-runtime:identity",
    authority: "runtime_identity",
    scope: "global",
    lifetime: "release",
    content
  };
  const frame = asWireObject(event.payload.submissionFrame, "Submission Frame");
  const instructions = asWireObject(frame.instructions, "Frame instructions");
  instructions.identityCore = block;

  const manifest: Record<string, unknown> = {
    id: block.id,
    version: block.version,
    source: block.source,
    authority: block.authority,
    scope: block.scope,
    lifetime: block.lifetime,
    characters: [...content].length,
    contentSha256: createHash("sha256")
      .update(JSON.stringify(content), "utf8")
      .digest("hex")
  };
  const runManifest = asWireObject(event.payload.runManifest, "Run Manifest");
  const manifestInstructions = asWireArray(
    runManifest.instructions,
    "manifest instructions"
  );
  const existingIndex = manifestInstructions.findIndex(
    (candidate) =>
      typeof candidate === "object" &&
      candidate !== null &&
      !Array.isArray(candidate) &&
      (candidate as Record<string, unknown>).id === "ikaros-identity"
  );
  if (existingIndex >= 0) {
    manifestInstructions[existingIndex] = manifest;
  } else {
    manifestInstructions.splice(0, 0, manifest);
  }
  return { block, manifest };
}

interface TestMemoryReference {
  memoryId: string;
  revision: number;
  scope: "global" | "workspace";
  characters: number;
}

type TestMemoryOmission = {
  sourceType: "memory";
  sourceId: string;
  revision: number;
  characters: number;
  reason: "omitted_by_budget" | "omitted_by_limit";
};

function testMemoryReference(
  digit: string,
  characters: number,
  scope: "global" | "workspace" = "global"
): TestMemoryReference {
  return {
    memoryId: `memory_${digit.repeat(32)}`,
    revision: 1,
    scope,
    characters
  };
}

function attachTestMemoryContext(
  event: ReturnType<typeof cloneGoldenNotification>,
  memory: TestMemoryReference[],
  omissions: Array<Record<string, unknown>> = [],
  contextDataCharacters = 128
): void {
  const selectedCharacters = memory.reduce((total, item) => total + item.characters, 0);
  const nextContextDataCharacters = memory.length === 0 ? 0 : contextDataCharacters;
  const snapshot = asWireObject(event.payload.contextSnapshot, "Context Snapshot");
  const manifest = asWireObject(event.payload.stepManifest, "Step Manifest");
  snapshot.memory = structuredClone(memory);
  manifest.memory = structuredClone(memory);
  snapshot.omissions = structuredClone(omissions);
  manifest.omissions = structuredClone(omissions);

  for (const budgetValue of [snapshot.budget, manifest.budget]) {
    const budget = asWireObject(budgetValue, "Memory input budget");
    const previousMemory = budget.memoryCharacters as number;
    const previousContextData = budget.contextDataCharacters as number;
    budget.memoryCharacters = selectedCharacters;
    budget.contextDataCharacters = nextContextDataCharacters;
    budget.totalCharacters =
      (budget.totalCharacters as number) -
      previousMemory -
      previousContextData +
      selectedCharacters +
      nextContextDataCharacters;
  }
}

interface TestHistoryRun {
  turnId: string;
  runId: string;
  status: "failed" | "cancelled";
  reasonCode: string | null;
  details: "included" | "omitted_by_budget";
}

function attachHistoryStatusV2(
  event: ReturnType<typeof cloneGoldenNotification>,
  runs: TestHistoryRun[] = []
): void {
  const preamble = "Runtime history status (contextual data): failed or cancelled Runs did not complete. Earlier tools may already have changed files or external state; failed, cancelled, or missing results do not prove that an action was not executed. Do not replay old Tool Calls. If details are omitted, inspect the current state or ask for missing information before continuing.";
  // Deliberately spell out canonical field order independently of the wire parser.
  const content = `${preamble}\n${JSON.stringify({
    runs: runs.map((run) => ({
      details: run.details,
      reasonCode: run.reasonCode,
      runId: run.runId,
      status: run.status,
      turnId: run.turnId
    })),
    version: 1
  })}`;
  const status = { version: 1, characters: runs.length ? [...content].length : 0, runs };
  const snapshot = asWireObject(event.payload.contextSnapshot, "Context Snapshot");
  const manifest = asWireObject(event.payload.stepManifest, "Step Manifest");
  snapshot.selectionVersion = "bounded-history-v2";
  manifest.contextSnapshotVersion = 2;
  for (const value of [snapshot, manifest]) {
    value.schemaVersion = 2;
    value.historyStatus = structuredClone(status);
    const budget = asWireObject(value.budget, "Context budget");
    budget.contextDataCharacters = (budget.contextDataCharacters as number) + status.characters;
    budget.totalCharacters = (budget.totalCharacters as number) + status.characters;
  }
}

function addFailedHistory(event: ReturnType<typeof cloneGoldenNotification>): TestHistoryRun {
  const run: TestHistoryRun = {
    turnId: "turn_previous_中文",
    runId: "run_previous",
    status: "failed",
    reasonCode: "provider_transport",
    details: "included"
  };
  const items = [
    { itemId: "item_previous_user", kind: "message", role: "user", characters: 12 },
    { itemId: "item_previous_call", kind: "tool_call", role: "assistant", characters: 43 },
    { itemId: "item_previous_result", kind: "tool_result", role: "tool", characters: 25 }
  ].map((item) => ({ ...item, turnId: run.turnId, runId: run.runId }));
  const snapshot = asWireObject(event.payload.contextSnapshot, "Context Snapshot");
  const manifest = asWireObject(event.payload.stepManifest, "Step Manifest");
  asWireArray(snapshot.historyGroups, "History groups").unshift({
    turnId: run.turnId,
    itemIds: items.map((item) => item.itemId)
  });
  for (const value of [snapshot, manifest]) {
    asWireArray(value.historyItems, "History items").unshift(...structuredClone(items));
    const budget = asWireObject(value.budget, "Context budget");
    budget.historyCharacters = (budget.historyCharacters as number) + 80;
    budget.totalCharacters = (budget.totalCharacters as number) + 80;
  }
  return run;
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
    expect(RUNTIME_RPC_METHODS).toHaveLength(26);
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

  it("accepts a valid non-empty Identity Core bound to its Run Manifest", () => {
    const event = cloneGoldenNotification("initial-user-item-completed");
    attachTestIdentityCore(event);

    expect(() => parseRuntimeEventNotification(event.envelope)).not.toThrow();
  });

  it("rejects Identity Core metadata drift and empty content", () => {
    const metadataMutations: Array<[string, unknown]> = [
      ["id", "other-identity"],
      ["version", 2],
      ["source", "other-source"],
      ["authority", "runtime_instruction"],
      ["scope", "run"],
      ["lifetime", "run"]
    ];
    for (const [field, replacement] of metadataMutations) {
      expectGoldenMutationRejected("initial-user-item-completed", (event) => {
        const { block, manifest } = attachTestIdentityCore(event);
        block[field] = replacement;
        manifest[field] = replacement;
      });
    }

    expectGoldenMutationRejected("initial-user-item-completed", (event) => {
      attachTestIdentityCore(event, "");
    });
    expectGoldenMutationRejected("initial-user-item-completed", (event) => {
      attachTestIdentityCore(event, " \n\t");
    });
    expectGoldenMutationRejected("initial-user-item-completed", (event) => {
      attachTestIdentityCore(event, "x".repeat(2049));
    });
  });

  it("rejects Identity Core Manifest character and hash mismatches", () => {
    expectGoldenMutationRejected("initial-user-item-completed", (event) => {
      const { manifest } = attachTestIdentityCore(event);
      manifest.characters = (manifest.characters as number) + 1;
    });
    expectGoldenMutationRejected("initial-user-item-completed", (event) => {
      const { manifest } = attachTestIdentityCore(event);
      manifest.contentSha256 = "b".repeat(64);
    });
  });

  it("requires the Run Manifest Memory Context contract version", () => {
    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      const manifest = asWireObject(payload.runManifest, "Run Manifest");
      manifest.memoryContextVersion = 1;
    });
  });

  it("rejects the unavailable Memory slot and an empty current-Run history", () => {

    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      const frame = asWireObject(payload.submissionFrame, "Submission Frame");
      const contextData = asWireObject(frame.contextData, "Frame Context Data");
      contextData.memory = [
        {
          memoryId: `memory_${"1".repeat(32)}`,
          revision: 1,
          scope: "global",
          characters: 1
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

  it("accepts bounded Global and Workspace Memory references with ordered omissions", () => {
    const event = cloneGoldenNotification("model-input-prepared");
    const memory = [testMemoryReference("1", 17), testMemoryReference("2", 31, "workspace")];
    const memoryOmission: TestMemoryOmission = {
      sourceType: "memory",
      sourceId: `memory_${"3".repeat(32)}`,
      revision: 4,
      characters: 53,
      reason: "omitted_by_budget"
    };
    attachTestMemoryContext(event, memory, [
      {
        sourceType: "history",
        sourceId: "turn_omitted_boundary",
        reason: "omitted_by_budget"
      },
      memoryOmission
    ]);

    expect(() => parseRuntimeEventNotification(event.envelope)).not.toThrow();
  });

  it("accepts a limit omission only with a full eight-item Memory selection", () => {
    const selected = Array.from({ length: 8 }, (_, index) =>
      testMemoryReference(
        (index + 1).toString(16),
        index + 1,
        index === 7 ? "workspace" : "global"
      )
    );
    const omission: TestMemoryOmission = {
      sourceType: "memory",
      sourceId: `memory_${"9".repeat(32)}`,
      revision: 1,
      characters: 9,
      reason: "omitted_by_limit"
    };

    const fullSelection = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(fullSelection, selected, [omission]);
    expect(() => parseRuntimeEventNotification(fullSelection.envelope)).not.toThrow();

    const incompleteSelection = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(incompleteSelection, selected.slice(0, 7), [omission]);
    expect(() => parseRuntimeEventNotification(incompleteSelection.envelope)).toThrow();
  });

  it("rejects malformed Memory references", () => {
    const mutations: Array<(memory: Record<string, unknown>) => void> = [
      (memory) => {
        memory.memoryId = "memory_invalid";
      },
      (memory) => {
        memory.revision = 0;
      },
      (memory) => {
        memory.scope = "thread";
      },
      (memory) => {
        memory.characters = 0;
      },
      (memory) => {
        memory.characters = 2_049;
      },
      (memory) => {
        memory.snapshotSha256 = "a".repeat(64);
      }
    ];

    for (const mutate of mutations) {
      const event = cloneGoldenNotification("model-input-prepared");
      attachTestMemoryContext(event, [testMemoryReference("1", 17)]);
      const snapshot = asWireObject(event.payload.contextSnapshot, "Context Snapshot");
      const selected = asWireArray(snapshot.memory, "selected Memory");
      mutate(asWireObject(selected[0], "Memory reference"));
      expect(() => parseRuntimeEventNotification(event.envelope)).toThrow();
    }
  });

  it("rejects duplicate, over-limit, and incorrectly budgeted Memory selections", () => {
    const duplicate = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(duplicate, [
      testMemoryReference("1", 17),
      { ...testMemoryReference("1", 19), revision: 2 }
    ]);
    expect(() => parseRuntimeEventNotification(duplicate.envelope)).toThrow();

    const tooMany = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(
      tooMany,
      Array.from({ length: 9 }, (_, index) =>
        testMemoryReference((index + 1).toString(16), 1)
      )
    );
    expect(() => parseRuntimeEventNotification(tooMany.envelope)).toThrow();

    const tooLarge = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(tooLarge, [
      testMemoryReference("1", 1_501),
      testMemoryReference("2", 1_501),
      testMemoryReference("3", 1_501),
      testMemoryReference("4", 1_501)
    ]);
    expect(() => parseRuntimeEventNotification(tooLarge.envelope)).toThrow();

    const wrongSum = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(wrongSum, [testMemoryReference("1", 17)]);
    const wrongSumSnapshot = asWireObject(wrongSum.payload.contextSnapshot, "Context Snapshot");
    const wrongSumBudget = asWireObject(wrongSumSnapshot.budget, "Context budget");
    wrongSumBudget.memoryCharacters = 18;
    wrongSumBudget.totalCharacters = (wrongSumBudget.totalCharacters as number) + 1;
    expect(() => parseRuntimeEventNotification(wrongSum.envelope)).toThrow();

    const missingWrapper = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(missingWrapper, [testMemoryReference("1", 17)], [], 0);
    expect(() => parseRuntimeEventNotification(missingWrapper.envelope)).toThrow();

    const ghostWrapper = cloneGoldenNotification("model-input-prepared");
    const ghostSnapshot = asWireObject(ghostWrapper.payload.contextSnapshot, "Context Snapshot");
    const ghostBudget = asWireObject(ghostSnapshot.budget, "Context budget");
    ghostBudget.contextDataCharacters = 1;
    ghostBudget.totalCharacters = (ghostBudget.totalCharacters as number) + 1;
    expect(() => parseRuntimeEventNotification(ghostWrapper.envelope)).toThrow();
  });

  it("rejects malformed, duplicate, overlapping, or misordered Memory omissions", () => {
    const selected = testMemoryReference("1", 17);
    const omitted: TestMemoryOmission = {
      sourceType: "memory",
      sourceId: `memory_${"2".repeat(32)}`,
      revision: 1,
      characters: 19,
      reason: "omitted_by_budget"
    };

    for (const mutate of [
      (value: Record<string, unknown>) => {
        value.revision = 0;
      },
      (value: Record<string, unknown>) => {
        value.characters = 0;
      },
      (value: Record<string, unknown>) => {
        value.characters = 2_049;
      },
      (value: Record<string, unknown>) => {
        value.reason = "unknown";
      },
      (value: Record<string, unknown>) => {
        value.extra = true;
      }
    ]) {
      const event = cloneGoldenNotification("model-input-prepared");
      attachTestMemoryContext(event, [selected], [omitted]);
      const snapshot = asWireObject(event.payload.contextSnapshot, "Context Snapshot");
      const omissions = asWireArray(snapshot.omissions, "Memory omissions");
      mutate(asWireObject(omissions[0], "Memory omission"));
      expect(() => parseRuntimeEventNotification(event.envelope)).toThrow();
    }

    const duplicate = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(duplicate, [selected], [omitted, { ...omitted, revision: 2 }]);
    expect(() => parseRuntimeEventNotification(duplicate.envelope)).toThrow();

    const overlap = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(overlap, [selected], [
      { ...omitted, sourceId: selected.memoryId }
    ]);
    expect(() => parseRuntimeEventNotification(overlap.envelope)).toThrow();

    const historyAfterMemory = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(historyAfterMemory, [selected], [
      omitted,
      {
        sourceType: "history",
        sourceId: "turn_omitted_boundary",
        reason: "omitted_by_budget"
      }
    ]);
    expect(() => parseRuntimeEventNotification(historyAfterMemory.envelope)).toThrow();

    const twoHistory = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(twoHistory, [selected], [
      {
        sourceType: "history",
        sourceId: "turn_omitted_boundary",
        reason: "omitted_by_budget"
      },
      {
        sourceType: "history",
        sourceId: "turn_older_boundary",
        reason: "omitted_by_budget"
      }
    ]);
    expect(() => parseRuntimeEventNotification(twoHistory.envelope)).toThrow();

    const impossibleLimit = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(impossibleLimit, [selected], [
      { ...omitted, reason: "omitted_by_limit" }
    ]);
    expect(() => parseRuntimeEventNotification(impossibleLimit.envelope)).toThrow();
  });

  it("rejects Step Memory, omission, and frozen-budget drift", () => {
    const event = cloneGoldenNotification("model-input-prepared-step-2");
    const memory = [testMemoryReference("1", 17)];
    const omission: TestMemoryOmission = {
      sourceType: "memory",
      sourceId: `memory_${"2".repeat(32)}`,
      revision: 1,
      characters: 19,
      reason: "omitted_by_budget"
    };
    attachTestMemoryContext(event, memory, [omission]);
    expect(() => parseRuntimeEventNotification(event.envelope)).not.toThrow();

    const memoryDrift = structuredClone(event);
    const memoryDriftManifest = asWireObject(
      memoryDrift.payload.stepManifest,
      "Step Manifest"
    );
    const stepMemory = asWireArray(memoryDriftManifest.memory, "Step Memory");
    asWireObject(stepMemory[0], "Step Memory reference").revision = 2;
    expect(() => parseRuntimeEventNotification(memoryDrift.envelope)).toThrow();

    const omissionDrift = structuredClone(event);
    const omissionDriftManifest = asWireObject(
      omissionDrift.payload.stepManifest,
      "Step Manifest"
    );
    const stepOmissions = asWireArray(omissionDriftManifest.omissions, "Step omissions");
    asWireObject(stepOmissions[0], "Step Memory omission").reason = "omitted_by_limit";
    expect(() => parseRuntimeEventNotification(omissionDrift.envelope)).toThrow();

    const budgetDrift = structuredClone(event);
    const budgetDriftManifest = asWireObject(
      budgetDrift.payload.stepManifest,
      "Step Manifest"
    );
    const stepBudget = asWireObject(budgetDriftManifest.budget, "Step budget");
    stepBudget.contextDataCharacters = (stepBudget.contextDataCharacters as number) + 1;
    stepBudget.totalCharacters = (stepBudget.totalCharacters as number) + 1;
    expect(() => parseRuntimeEventNotification(budgetDrift.envelope)).toThrow();
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
        const outputStyle = instructions.find(
          (candidate) =>
            typeof candidate === "object" &&
            candidate !== null &&
            !Array.isArray(candidate) &&
            (candidate as Record<string, unknown>).id === "output-style"
        );
        asWireObject(outputStyle, "output-style manifest")[field] = replacement;
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
      unicodeManifestInstructions.find(
        (candidate) =>
          typeof candidate === "object" &&
          candidate !== null &&
          !Array.isArray(candidate) &&
          (candidate as Record<string, unknown>).id === "output-style"
      ),
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

    const beyondReserve = structuredClone(secondStep);
    const beyondReserveManifest = asWireObject(
      beyondReserve.payload.stepManifest,
      "Step Manifest"
    );
    const beyondReserveItems = asWireArray(
      beyondReserveManifest.historyItems,
      "Step history Items"
    );
    const addedCharacters = 12_001;
    const lastItem = asWireObject(
      beyondReserveItems[beyondReserveItems.length - 1],
      "last Step history Item"
    );
    lastItem.characters = (lastItem.characters as number) + addedCharacters;
    const beyondReserveBudget = asWireObject(
      beyondReserveManifest.budget,
      "Step budget"
    );
    beyondReserveBudget.currentRunCharacters =
      (beyondReserveBudget.currentRunCharacters as number) + addedCharacters;
    beyondReserveBudget.totalCharacters =
      (beyondReserveBudget.totalCharacters as number) + addedCharacters;
    expect(() => parseRuntimeEventNotification(beyondReserve.envelope)).not.toThrow();

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

  it("accepts one omission boundary and rejects selected or over-budget boundaries", () => {
    const omitted = cloneGoldenNotification("model-input-prepared");
    const snapshot = asWireObject(omitted.payload.contextSnapshot, "Context Snapshot");
    const manifest = asWireObject(omitted.payload.stepManifest, "Step Manifest");
    const omission = {
      sourceType: "history",
      sourceId: "turn_omitted_boundary",
      reason: "omitted_by_budget"
    };
    snapshot.omissions = [omission];
    manifest.omissions = [structuredClone(omission)];
    expect(() => parseRuntimeEventNotification(omitted.envelope)).not.toThrow();

    const selectedBoundary = structuredClone(omitted);
    const selectedSnapshot = asWireObject(
      selectedBoundary.payload.contextSnapshot,
      "Context Snapshot"
    );
    const selectedManifest = asWireObject(
      selectedBoundary.payload.stepManifest,
      "Step Manifest"
    );
    const selectedGroups = asWireArray(selectedSnapshot.historyGroups, "history groups");
    const selectedTurnId = asWireObject(selectedGroups[0], "selected history group").turnId;
    asWireObject(
      asWireArray(selectedSnapshot.omissions, "Snapshot omissions")[0],
      "Snapshot omission"
    ).sourceId = selectedTurnId;
    asWireObject(
      asWireArray(selectedManifest.omissions, "Step omissions")[0],
      "Step omission"
    ).sourceId = selectedTurnId;
    expect(() => parseRuntimeEventNotification(selectedBoundary.envelope)).toThrow();

    const exhaustedReserve = structuredClone(omitted);
    const exhaustedSnapshot = asWireObject(
      exhaustedReserve.payload.contextSnapshot,
      "Context Snapshot"
    );
    const exhaustedManifest = asWireObject(
      exhaustedReserve.payload.stepManifest,
      "Step Manifest"
    );
    const snapshotBudget = asWireObject(exhaustedSnapshot.budget, "Context budget");
    const stepBudget = asWireObject(exhaustedManifest.budget, "Step budget");
    const invalidMaximum =
      (snapshotBudget.totalCharacters as number) +
      (snapshotBudget.reservedCurrentRunCharacters as number) -
      1;
    snapshotBudget.maximumCharacters = invalidMaximum;
    stepBudget.maximumCharacters = invalidMaximum;
    expect(() => parseRuntimeEventNotification(exhaustedReserve.envelope)).toThrow();

    const alternateLimits = structuredClone(omitted);
    const alternateSnapshot = asWireObject(
      alternateLimits.payload.contextSnapshot,
      "Context Snapshot"
    );
    const alternateManifest = asWireObject(
      alternateLimits.payload.stepManifest,
      "Step Manifest"
    );
    for (const budget of [
      asWireObject(alternateSnapshot.budget, "Context budget"),
      asWireObject(alternateManifest.budget, "Step budget")
    ]) {
      budget.maximumCharacters = 47_000;
      budget.reservedCurrentRunCharacters = 11_000;
    }
    expect(() => parseRuntimeEventNotification(alternateLimits.envelope)).toThrow();
  });

  it("keeps v1 readable and accepts v2 with empty or included failure status", () => {
    const submitted = cloneGoldenNotification("initial-user-item-completed");
    const runManifest = asWireObject(submitted.payload.runManifest, "Run Manifest");
    runManifest.contextSelectionVersion = "bounded-history-v2";
    expect(() => parseRuntimeEventNotification(submitted.envelope)).not.toThrow();
    runManifest.contextSelectionVersion = "bounded-history-v3";
    expect(() => parseRuntimeEventNotification(submitted.envelope)).toThrow();
    const legacy = cloneGoldenNotification("model-input-prepared");
    expect(() => parseRuntimeEventNotification(legacy.envelope)).not.toThrow();
    const empty = structuredClone(legacy);
    attachHistoryStatusV2(empty);
    expect(() => parseRuntimeEventNotification(empty.envelope)).not.toThrow();
    const included = structuredClone(legacy);
    const run = addFailedHistory(included);
    attachHistoryStatusV2(included, [run]);
    expect(() => parseRuntimeEventNotification(included.envelope)).not.toThrow();
    const withoutMemory = structuredClone(legacy);
    attachTestMemoryContext(withoutMemory, []);
    attachHistoryStatusV2(withoutMemory, [addFailedHistory(withoutMemory)]);
    expect(() => parseRuntimeEventNotification(withoutMemory.envelope)).not.toThrow();
  });

  it("accepts a latest failed Turn omission with a separately budgeted warning", () => {
    const event = cloneGoldenNotification("model-input-prepared");
    const omission = {
      sourceType: "history", sourceId: "turn_too_large", reason: "omitted_by_budget"
    };
    for (const key of ["contextSnapshot", "stepManifest"]) {
      asWireObject(event.payload[key], key).omissions = [structuredClone(omission)];
    }
    attachHistoryStatusV2(event, [{
      turnId: "turn_too_large", runId: "run_too_large", status: "cancelled",
      reasonCode: null, details: "omitted_by_budget"
    }]);
    expect(() => parseRuntimeEventNotification(event.envelope)).not.toThrow();
    const unrelated = structuredClone(event);
    for (const key of ["contextSnapshot", "stepManifest"]) {
      asWireObject(unrelated.payload[key], key).omissions = [];
    }
    expect(() => parseRuntimeEventNotification(unrelated.envelope)).toThrow();
  });

  it("rejects invalid, foreign, current, or duplicate history status Runs", () => {
    const mutations: Array<(run: TestHistoryRun, event: ReturnType<typeof cloneGoldenNotification>) => TestHistoryRun[]> = [
      (run) => [{ ...run, runId: "run_foreign" }],
      (run) => [{ ...run, turnId: "turn_foreign" }],
      (run, event) => [{ ...run, runId: event.params.runId as string }],
      (run) => [run, structuredClone(run)],
      (run) => [{ ...run, status: "completed" as "failed" }],
      (run) => [{ ...run, reasonCode: "invalid reason" }],
      (run) => [{ ...run, details: "omitted_by_budget" }]
    ];
    for (const mutate of mutations) {
      const event = cloneGoldenNotification("model-input-prepared");
      const run = addFailedHistory(event);
      attachHistoryStatusV2(event, mutate(run, event));
      expect(() => parseRuntimeEventNotification(event.envelope)).toThrow();
    }
  });

  it("rejects status budget tampering, frozen Step drift, and cross-version manifests", () => {
    const event = cloneGoldenNotification("model-input-prepared");
    attachHistoryStatusV2(event, [addFailedHistory(event)]);
    const mutations: Array<(snapshot: Record<string, unknown>, manifest: Record<string, unknown>) => void> = [
      (snapshot) => { delete snapshot.historyStatus; },
      (snapshot) => { snapshot.selectionVersion = "bounded-history-v1"; },
      (snapshot) => { snapshot.schemaVersion = 1; },
      (_, manifest) => { manifest.contextSnapshotVersion = 1; },
      (_, manifest) => { manifest.schemaVersion = 1; },
      (_, manifest) => {
        const runs = asWireArray(asWireObject(manifest.historyStatus, "History status").runs, "Runs");
        asWireObject(runs[0], "Run").reasonCode = "provider_protocol";
      },
      (snapshot, manifest) => {
        for (const value of [snapshot, manifest]) {
          const status = asWireObject(value.historyStatus, "History status");
          status.characters = (status.characters as number) + 1;
          const budget = asWireObject(value.budget, "Budget");
          budget.contextDataCharacters = (budget.contextDataCharacters as number) + 1;
          budget.totalCharacters = (budget.totalCharacters as number) + 1;
        }
      },
      (snapshot, manifest) => {
        for (const value of [snapshot, manifest]) {
          const status = asWireObject(value.historyStatus, "History status");
          const budget = asWireObject(value.budget, "Budget");
          const previous = budget.contextDataCharacters as number;
          budget.contextDataCharacters = status.characters;
          budget.totalCharacters = (budget.totalCharacters as number) - previous + (status.characters as number);
        }
      }
    ];
    for (const mutate of mutations) {
      const copy = structuredClone(event);
      mutate(asWireObject(copy.payload.contextSnapshot, "Snapshot"), asWireObject(copy.payload.stepManifest, "Step"));
      expect(() => parseRuntimeEventNotification(copy.envelope)).toThrow();
    }
    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      asWireObject(payload.contextSnapshot, "Snapshot").historyStatus = { version: 1, characters: 0, runs: [] };
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

  it("rejects malformed, over-broad, or scope-inconsistent Memory results", () => {
    const created = cloneGoldenResponse("memory-created");
    created.result.memoryId = "memory_invalid";
    expect(() =>
      parseRuntimeMethodResult(created.method, created.result, created.requestParams)
    ).toThrow();

    const listed = cloneGoldenResponse("memory-list-page");
    const summaries = asWireArray(listed.result.memories, "Memory summaries");
    asWireObject(summaries[0], "Memory summary").content = "full content must stay lazy";
    expect(() =>
      parseRuntimeMethodResult(listed.method, listed.result, listed.requestParams)
    ).toThrow();

    const overRequestedLimit = cloneGoldenResponse("memory-list-page");
    overRequestedLimit.requestParams.limit = 1;
    const overLimitSummaries = asWireArray(
      overRequestedLimit.result.memories,
      "Memory summaries"
    );
    const secondSummary = structuredClone(
      asWireObject(overLimitSummaries[0], "Memory summary")
    );
    secondSummary.id = `memory_${"2".repeat(32)}`;
    overLimitSummaries.push(secondSummary);
    expect(() =>
      parseRuntimeMethodResult(
        overRequestedLimit.method,
        overRequestedLimit.result,
        overRequestedLimit.requestParams
      )
    ).toThrow();

    const repeatedCursor = cloneGoldenResponse("memory-list-page");
    repeatedCursor.result.hasMore = true;
    repeatedCursor.result.nextCursor = "same-cursor";
    repeatedCursor.requestParams.cursor = "same-cursor";
    expect(() =>
      parseRuntimeMethodResult(
        repeatedCursor.method,
        repeatedCursor.result,
        repeatedCursor.requestParams
      )
    ).toThrow();

    const invalidTimestamp = cloneGoldenResponse("memory-list-page");
    asWireObject(
      asWireArray(invalidTimestamp.result.memories, "Memory summaries")[0],
      "Memory summary"
    ).updatedAt = "2026-02-31T12:00:00.000Z";
    expect(() =>
      parseRuntimeMethodResult(
        invalidTimestamp.method,
        invalidTimestamp.result,
        invalidTimestamp.requestParams
      )
    ).toThrow();

    const wrongScope = cloneGoldenResponse("memory-list-page");
    const wrongSummaries = asWireArray(wrongScope.result.memories, "Memory summaries");
    asWireObject(asWireObject(wrongSummaries[0], "Memory summary").scope, "scope").type =
      "workspace";
    asWireObject(asWireObject(wrongSummaries[0], "Memory summary").scope, "scope").key =
      "workspace-1";
    expect(() =>
      parseRuntimeMethodResult(wrongScope.method, wrongScope.result, wrongScope.requestParams)
    ).toThrow();

    const fetched = cloneGoldenResponse("memory-record");
    const memory = asWireObject(fetched.result.memory, "Memory record");
    asWireObject(memory.provenance, "Memory provenance").status = "not_applicable";
    expect(() =>
      parseRuntimeMethodResult(fetched.method, fetched.result, fetched.requestParams)
    ).toThrow();

    const sessionSourced = cloneGoldenResponse("memory-record");
    const sessionMemory = asWireObject(sessionSourced.result.memory, "Memory record");
    const provenance = asWireObject(sessionMemory.provenance, "Memory provenance");
    provenance.sourceKind = "session_item";
    provenance.threadId = `thread_${"1".repeat(32)}`;
    provenance.turnId = `turn_${"2".repeat(32)}`;
    provenance.itemId = `item_${"3".repeat(32)}`;
    provenance.status = "available";
    expect(() =>
      parseRuntimeMethodResult(
        sessionSourced.method,
        sessionSourced.result,
        sessionSourced.requestParams
      )
    ).not.toThrow();
    provenance.status = "unavailable";
    expect(() =>
      parseRuntimeMethodResult(
        sessionSourced.method,
        sessionSourced.result,
        sessionSourced.requestParams
      )
    ).not.toThrow();
    provenance.threadId = "thread-source";
    expect(() =>
      parseRuntimeMethodResult(
        sessionSourced.method,
        sessionSourced.result,
        sessionSourced.requestParams
      )
    ).toThrow();
    provenance.threadId = `thread_${"1".repeat(32)}`;
    provenance.itemId = `item_${"g".repeat(32)}`;
    expect(() =>
      parseRuntimeMethodResult(
        sessionSourced.method,
        sessionSourced.result,
        sessionSourced.requestParams
      )
    ).toThrow();
    provenance.itemId = `item_${"3".repeat(32)}`;
    provenance.turnId = null;
    expect(() =>
      parseRuntimeMethodResult(
        sessionSourced.method,
        sessionSourced.result,
        sessionSourced.requestParams
      )
    ).toThrow();

    const memoryId = `memory_${"3".repeat(32)}`;
    const correctionParams = { memoryId, expectedRevision: 1 };
    const corrected = { memoryId, resultingRevision: 2, created: true };
    expect(
      parseRuntimeMethodResult("memory.correct", corrected, correctionParams)
    ).toEqual(corrected);
    expect(() =>
      parseRuntimeMethodResult(
        "memory.correct",
        { ...corrected, memoryId: `memory_${"4".repeat(32)}` },
        correctionParams
      )
    ).toThrow();
    expect(() =>
      parseRuntimeMethodResult(
        "memory.correct",
        { ...corrected, resultingRevision: 3 },
        correctionParams
      )
    ).toThrow();

    const forgetParams = { memoryId, expectedRevision: 2 };
    const forgotten = { memoryId, resultingRevision: 3, created: false };
    expect(parseRuntimeMethodResult("memory.forget", forgotten, forgetParams)).toEqual(
      forgotten
    );
    expect(() =>
      parseRuntimeMethodResult(
        "memory.forget",
        { ...forgotten, unexpected: true },
        forgetParams
      )
    ).toThrow();
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

  it("matches Python Unicode stripping for valid Memory content", () => {
    const fetched = cloneGoldenResponse("memory-record");
    const memory = asWireObject(fetched.result.memory, "Memory record");
    memory.content = "\uFEFF";

    expect(() =>
      parseRuntimeMethodResult(fetched.method, fetched.result, fetched.requestParams)
    ).not.toThrow();
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
