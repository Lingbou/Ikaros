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
  event: ReturnType<typeof cloneGoldenNotification>, content = "Ikaros test identity"
): { block: Record<string, unknown> } {
  const block = { id: "ikaros-identity", version: 1, source: "ikaros-runtime:identity",
    authority: "runtime_identity", scope: "global", lifetime: "release", content };
  const config = asWireObject(event.payload.runConfig, "Run configuration");
  asWireObject(config.instructions, "Instructions").identityCore = block;
  return { block };
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
  contextDataTokens = 128
): void {
  const selectedCharacters = memory.reduce((total, item) => total + item.characters, 0) * 4;
  const nextContextDataCharacters = memory.length === 0 ? 0 : contextDataTokens * 4;
  const snapshot = asWireObject(event.payload.contextRevision, "Context revision");
  const manifest = asWireObject(event.payload.stepInput, "Step input");
  snapshot.memoryContextCharacters = memory.length === 0 ? 0 : contextDataTokens;
  manifest.memoryContextCharacters = memory.length === 0 ? 0 : contextDataTokens;
  snapshot.memory = structuredClone(memory);
  manifest.memory = structuredClone(memory);
  snapshot.omissions = structuredClone(omissions);
  manifest.omissions = structuredClone(omissions);

  for (const budgetValue of [snapshot.budget, manifest.budget]) {
    const budget = asWireObject(budgetValue, "Memory input budget");
    const previousMemory = budget.memoryTokens as number;
    const previousContextData = budget.contextDataTokens as number;
    budget.memoryTokens = selectedCharacters;
    budget.contextDataTokens = nextContextDataCharacters;
    budget.totalTokens =
      (budget.totalTokens as number) -
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

function attachHistoryStatus(
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
  const snapshot = asWireObject(event.payload.contextRevision, "Context revision");
  const manifest = asWireObject(event.payload.stepInput, "Step input");
  const tokens = runs.length ? Buffer.byteLength(content, "utf8") + 64 : 0;
  for (const value of [snapshot, manifest]) {
    value.historyStatus = structuredClone(status);
    const budget = asWireObject(value.budget, "Context budget");
    budget.contextDataTokens = (budget.contextDataTokens as number) + tokens;
    budget.totalTokens = (budget.totalTokens as number) + tokens;
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
    { itemId: "item_previous_user", kind: "message", role: "user", tokens: 12 },
    { itemId: "item_previous_call", kind: "tool_call", role: "assistant", tokens: 43 },
    { itemId: "item_previous_result", kind: "tool_result", role: "tool", tokens: 25 }
  ].map((item) => ({ ...item, turnId: run.turnId, runId: run.runId }));
  const snapshot = asWireObject(event.payload.contextRevision, "Context revision");
  const manifest = asWireObject(event.payload.stepInput, "Step input");
  asWireArray(snapshot.historyGroups, "History groups").unshift({
    turnId: run.turnId,
    itemIds: items.map((item) => item.itemId)
  });
  for (const value of [snapshot, manifest]) {
    asWireArray(value.historyItems, "History items").unshift(...structuredClone(items));
    const budget = asWireObject(value.budget, "Context budget");
    budget.historyTokens = (budget.historyTokens as number) + 80;
    budget.totalTokens = (budget.totalTokens as number) + 80;
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
    expect(RUNTIME_PROVIDER_TOOL_IDS).toContain("process_start");
    expect(RUNTIME_PROVIDER_TOOL_IDS).not.toContain("process_run");
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

  it("rejects initial Turn fields that disagree with the frozen Run configuration", () => {
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
      }
    ];

    for (const mutate of mutations) {
      expectGoldenMutationRejected("initial-user-item-completed", mutate);
    }
  });

  it("accepts a valid non-empty Identity Core in its Run configuration", () => {
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
        const { block } = attachTestIdentityCore(event);
        block[field] = replacement;
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

  it("rejects the unavailable Memory slot and an empty current-Run history", () => {

    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      const frame = asWireObject(payload.runConfig, "Run configuration");
      frame.contextData = { memory: [
        {
          memoryId: `memory_${"1".repeat(32)}`,
          revision: 1,
          scope: "global",
          characters: 1
        }
      ] };
    });

    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const snapshot = asWireObject(payload.contextRevision, "Context revision");
      snapshot.historyGroups = [];
      snapshot.historyItems = [];
      const snapshotBudget = asWireObject(snapshot.budget, "Context budget");
      const snapshotCurrent = snapshotBudget.currentRunTokens as number;
      snapshotBudget.currentRunTokens = 0;
      snapshotBudget.totalTokens =
        (snapshotBudget.totalTokens as number) - snapshotCurrent;

      const manifest = asWireObject(payload.stepInput, "Step input");
      manifest.historyItems = [];
      const manifestBudget = asWireObject(manifest.budget, "Step budget");
      const manifestCurrent = manifestBudget.currentRunTokens as number;
      manifestBudget.currentRunTokens = 0;
      manifestBudget.totalTokens =
        (manifestBudget.totalTokens as number) - manifestCurrent;
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
      const snapshot = asWireObject(event.payload.contextRevision, "Context revision");
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
    const wrongSumSnapshot = asWireObject(wrongSum.payload.contextRevision, "Context revision");
    const wrongSumBudget = asWireObject(wrongSumSnapshot.budget, "Context budget");
    wrongSumBudget.memoryTokens = 18;
    wrongSumBudget.totalTokens = (wrongSumBudget.totalTokens as number) + 1;
    expect(() => parseRuntimeEventNotification(wrongSum.envelope)).toThrow();

    const missingWrapper = cloneGoldenNotification("model-input-prepared");
    attachTestMemoryContext(missingWrapper, [testMemoryReference("1", 17)], [], 0);
    expect(() => parseRuntimeEventNotification(missingWrapper.envelope)).toThrow();

    const ghostWrapper = cloneGoldenNotification("model-input-prepared");
    const ghostSnapshot = asWireObject(ghostWrapper.payload.contextRevision, "Context revision");
    const ghostBudget = asWireObject(ghostSnapshot.budget, "Context budget");
    ghostBudget.contextDataTokens = 1;
    ghostBudget.totalTokens = (ghostBudget.totalTokens as number) + 1;
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
      const snapshot = asWireObject(event.payload.contextRevision, "Context revision");
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

  it("rejects initial Step Memory, omission, and frozen-budget drift", () => {
    const event = cloneGoldenNotification("model-input-prepared");
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
      memoryDrift.payload.stepInput,
      "Step input"
    );
    const stepMemory = asWireArray(memoryDriftManifest.memory, "Step Memory");
    asWireObject(stepMemory[0], "Step Memory reference").revision = 2;
    expect(() => parseRuntimeEventNotification(memoryDrift.envelope)).toThrow();

    const omissionDrift = structuredClone(event);
    const omissionDriftManifest = asWireObject(
      omissionDrift.payload.stepInput,
      "Step input"
    );
    const stepOmissions = asWireArray(omissionDriftManifest.omissions, "Step omissions");
    asWireObject(stepOmissions[0], "Step Memory omission").reason = "omitted_by_limit";
    expect(() => parseRuntimeEventNotification(omissionDrift.envelope)).toThrow();

    const budgetDrift = structuredClone(event);
    const budgetDriftManifest = asWireObject(
      budgetDrift.payload.stepInput,
      "Step input"
    );
    const stepBudget = asWireObject(budgetDriftManifest.budget, "Step budget");
    stepBudget.contextDataTokens = (stepBudget.contextDataTokens as number) + 1;
    stepBudget.totalTokens = (stepBudget.totalTokens as number) + 1;
    expect(() => parseRuntimeEventNotification(budgetDrift.envelope)).toThrow();
  });

  it("rejects incomplete, unknown, or internally inconsistent input budgets", () => {
    for (const key of [
      "measurementVersion",
      "maximumTokens",
      "reservedCurrentRunTokens",
      "instructionTokens",
      "contextDataTokens",
      "toolTokens"
    ]) {
      expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
        const snapshot = asWireObject(payload.contextRevision, "Context revision");
        const budget = asWireObject(snapshot.budget, "Context budget");
        delete budget[key];
      });
    }

    for (const [key, value] of [
      ["mode", "unknown"],
      ["measurementVersion", "unknown-v1"],
      ["maximumTokens", 1],
      ["reservedCurrentRunTokens", 1]
    ] as const) {
      expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
        const snapshot = asWireObject(payload.contextRevision, "Context revision");
        asWireObject(snapshot.budget, "Context budget")[key] = value;
      });
    }

    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const snapshot = asWireObject(payload.contextRevision, "Context revision");
      const budget = asWireObject(snapshot.budget, "Context budget");
      budget.totalTokens = (budget.totalTokens as number) + 1;
    });

    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const manifest = asWireObject(payload.stepInput, "Step input");
      const budget = asWireObject(manifest.budget, "Step budget");
      budget.instructionTokens = (budget.instructionTokens as number) + 1;
      budget.totalTokens = (budget.totalTokens as number) + 1;
    });
  });

  it("accepts one omission boundary and rejects selected or over-budget boundaries", () => {
    const omitted = cloneGoldenNotification("model-input-prepared");
    const snapshot = asWireObject(omitted.payload.contextRevision, "Context revision");
    const manifest = asWireObject(omitted.payload.stepInput, "Step input");
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
      selectedBoundary.payload.contextRevision,
      "Context revision"
    );
    const selectedManifest = asWireObject(
      selectedBoundary.payload.stepInput,
      "Step input"
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
      exhaustedReserve.payload.contextRevision,
      "Context revision"
    );
    const exhaustedManifest = asWireObject(
      exhaustedReserve.payload.stepInput,
      "Step input"
    );
    const snapshotBudget = asWireObject(exhaustedSnapshot.budget, "Context budget");
    const stepBudget = asWireObject(exhaustedManifest.budget, "Step budget");
    const invalidMaximum =
      (snapshotBudget.totalTokens as number) +
      (snapshotBudget.reservedCurrentRunTokens as number) -
      1;
    snapshotBudget.maximumTokens = invalidMaximum;
    stepBudget.maximumTokens = invalidMaximum;
    expect(() => parseRuntimeEventNotification(exhaustedReserve.envelope)).toThrow();

    const alternateLimits = structuredClone(omitted);
    const alternateSnapshot = asWireObject(
      alternateLimits.payload.contextRevision,
      "Context revision"
    );
    const alternateManifest = asWireObject(
      alternateLimits.payload.stepInput,
      "Step input"
    );
    for (const budget of [
      asWireObject(alternateSnapshot.budget, "Context budget"),
      asWireObject(alternateManifest.budget, "Step budget")
    ]) {
      budget.maximumTokens = 47_000;
      budget.reservedCurrentRunTokens = 11_000;
    }
    expect(() => parseRuntimeEventNotification(alternateLimits.envelope)).not.toThrow();
  });

  it("counts Unicode compaction summaries as code points", () => {
    const event = cloneGoldenNotification("model-input-prepared");
    for (const key of ["contextRevision", "stepInput"]) {
      const value = asWireObject(event.payload[key], key);
      value.compactionSummary = "😀";
      const budget = asWireObject(value.budget, `${key} budget`);
      budget.contextDataTokens = (budget.contextDataTokens as number) + 4;
      budget.totalTokens = (budget.totalTokens as number) + 4;
    }
    expect(() => parseRuntimeEventNotification(event.envelope)).not.toThrow();
  });

  it("accepts a latest failed Turn omission with a separately budgeted warning", () => {
    const event = cloneGoldenNotification("model-input-prepared");
    const omission = {
      sourceType: "history", sourceId: "turn_too_large", reason: "omitted_by_budget"
    };
    for (const key of ["contextRevision", "stepInput"]) {
      asWireObject(event.payload[key], key).omissions = [structuredClone(omission)];
    }
    attachHistoryStatus(event, [{
      turnId: "turn_too_large", runId: "run_too_large", status: "cancelled",
      reasonCode: null, details: "omitted_by_budget"
    }]);
    expect(() => parseRuntimeEventNotification(event.envelope)).not.toThrow();
    const unrelated = structuredClone(event);
    for (const key of ["contextRevision", "stepInput"]) {
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
      attachHistoryStatus(event, mutate(run, event));
      expect(() => parseRuntimeEventNotification(event.envelope)).toThrow();
    }
  });

  it("rejects empty Context revision history groups", () => {
    expectGoldenMutationRejected("model-input-prepared", ({ payload }) => {
      const snapshot = asWireObject(payload.contextRevision, "Context revision");
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
        const manifest = asWireObject(payload.stepInput, "Step input");
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
    canonicalItem.content = JSON.stringify(Object.fromEntries(Object.entries(canonicalResult).reverse()));
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

  it("accepts history_read item references and synthetic terminal results", () => {
    const historyRead = cloneGoldenNotification("tool-result-completed");
    const item = asWireObject(historyRead.payload.item, "Tool Result Item");
    const data = asWireObject(item.data, "Tool Result data");
    const result = asWireObject(data.result, "Tool Result");
    data.toolName = "history_read";
    result.toolName = "history_read";
    result.itemId = "item_00000000000000000000000000000001";
    delete result.processId;
    delete result.state;
    delete result.exitCode;
    delete result.cwd;
    delete result.pid;
    delete result.startedAt;
    delete result.finishedAt;
    delete result.cursor;
    delete result.nextCursor;
    delete result.hasMore;
    item.content = JSON.stringify(result);
    expect(() => parseRuntimeEventNotification(historyRead.envelope)).not.toThrow();

    for (const status of ["failed", "cancelled"] as const) {
      for (const toolName of [
        "process_start",
        "process_read",
        "process_wait",
        "process_stop",
        "history_read",
        "read",
        "write",
        "edit"
      ]) {
        const terminal = cloneGoldenNotification("tool-result-completed");
        const terminalItem = asWireObject(terminal.payload.item, "Tool Result Item");
        const terminalData = asWireObject(terminalItem.data, "Tool Result data");
        const terminalResult = asWireObject(terminalData.result, "Tool Result");
        terminalItem.status = status;
        terminalData.toolName = toolName;
        terminalResult.toolName = toolName;
        terminalResult.ok = false;
        terminalResult.cancelled = status === "cancelled";
        terminalResult.output = "";
        terminalResult.truncated = false;
        terminalResult.errorCode = status === "cancelled" ? "cancelled" : "runtime_interrupted";
        for (const key of [
          "processId", "state", "exitCode", "cwd", "pid", "startedAt", "finishedAt",
          "cursor", "nextCursor", "hasMore", "durationMs", "timedOut", "path", "lineStart",
          "lineEnd", "bytesRead", "bom", "lineTruncations", "totalLines", "nextOffset",
          "created", "bytesWritten", "verified", "newline", "replacements", "itemId"
        ]) delete terminalResult[key];
        terminalItem.content = JSON.stringify(terminalResult);
        expect(() => parseRuntimeEventNotification(terminal.envelope)).not.toThrow();
      }
    }
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

  it("checks file preview pagination, UTF-8 byte limits and request identity", () => {
    const source = cloneGoldenResponse("file-preview");
    for (const mutate of [
      (value: Record<string, unknown>) => { value.threadId = "foreign"; },
      (value: Record<string, unknown>) => { value.lineEnd = 2; },
      (value: Record<string, unknown>) => { value.nextOffset = 2; },
      (value: Record<string, unknown>) => { value.revision = "not-a-version"; },
      (value: Record<string, unknown>) => { value.content = "界".repeat(18_000); },
      (value: Record<string, unknown>) => { value.contents = "extra"; }
    ]) {
      const copy = structuredClone(source.result);
      mutate(copy);
      expect(() => parseRuntimeMethodResult("file.preview", copy, source.requestParams)).toThrow();
    }
    const page: Record<string, unknown> = { ...source.result, content: "a\r\nb\r\n", lineStart: 2001, lineEnd: 2002,
      nextOffset: 2003, truncated: true, truncationReason: "byte_limit" };
    const params: Record<string, unknown> = { ...source.requestParams, offset: 2001, expectedRevision: source.result.revision };
    expect(() => parseRuntimeMethodResult("file.preview", page, params)).not.toThrow();
    expect(() => parseRuntimeMethodResult("file.preview", page, { ...params, expectedRevision: "b".repeat(64) })).toThrow();
    expect(() => parseRuntimeMethodResult("file.preview", {
      threadId: params.threadId, path: null, status: "unavailable", reason: "protected_content"
    }, params)).not.toThrow();
    const changed = { threadId: params.threadId, path: page.path, status: "unavailable", reason: "revision_changed" };
    expect(() => parseRuntimeMethodResult("file.preview", changed, params)).not.toThrow();
  });

  it("binds immutable file changes to their source operation and bounds stored records", () => {
    const source = cloneGoldenResponse("file-change");
    for (const mutate of [
      (value: Record<string, unknown>) => { value.toolCallItemId = "other"; },
      (value: Record<string, unknown>) => { value.threadId = "foreign"; },
      (value: Record<string, unknown>) => { value.operation = "process_run"; },
      (value: Record<string, unknown>) => { value.additions = -1; },
      (value: Record<string, unknown>) => { value.diff = "界".repeat(90_000); },
      (value: Record<string, unknown>) => { asWireObject(value.before, "Before").byteCount = 1; }
    ]) {
      const copy = structuredClone(source.result);
      mutate(copy);
      expect(() => parseRuntimeMethodResult("file.change.get", copy, source.requestParams)).toThrow();
    }
    const old = { ...source.result, status: "unavailable", reason: "not_recorded",
      recordedAt: null, before: null, after: null };
    for (const key of ["diff", "additions", "deletions"]) delete (old as Record<string, unknown>)[key];
    expect(() => parseRuntimeMethodResult("file.change.get", old, source.requestParams)).not.toThrow();
    expectGoldenMutationRejected("file-change-recorded", ({ payload }) => {
      payload.toolCallItemId = "foreign";
    });
    expectGoldenMutationRejected("file-change-recorded", ({ payload }) => {
      payload.recordedAt = "2026-01-01T00:00:00.000Z";
    });
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
  it("requires the current journal schema and rejects removed fields", () => {
    for (const version of [1, 5, 6, 7, 8, RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION + 1]) {
      const event = cloneGoldenNotification("model-input-prepared");
      event.params.schemaVersion = version;
      expect(() => parseRuntimeEventNotification(event.envelope)).toThrow();
    }
    for (const key of ["submissionFrame", "runManifest"]) {
      expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => { payload[key] = {}; });
    }
    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      asWireObject(payload.runConfig, "Run configuration").schemaVersion = 1;
    });
    for (const key of ["maxModelCalls", "maxDurationSeconds"]) {
      expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
        asWireObject(payload.runConfig, "Run configuration")[key] = 100;
      });
    }
    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      asWireObject(payload.run, "initial Run").executionLimits = {};
    });
    expect(() => parseRuntimeEventNotification(cloneGoldenNotification("file-change-recorded").envelope)).not.toThrow();
  });

  it("freezes valid model capacities without execution ceilings", () => {
    for (const key of ["contextWindow", "maxOutputTokens"]) {
      for (const invalid of [0, -1, true, 1.5]) {
        expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
          asWireObject(payload.runConfig, "Run configuration")[key] = invalid;
        });
      }
    }
    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      const config = asWireObject(payload.runConfig, "Run configuration");
      config.maxOutputTokens = config.contextWindow;
    });
    expectGoldenMutationRejected("initial-user-item-completed", ({ payload }) => {
      const config = asWireObject(payload.runConfig, "Run configuration");
      const tools = asWireArray(config.tools, "Tools");
      asWireObject(tools[0], "Tool").definitionSha256 = "b".repeat(64);
    });
  });

  it("requires the first revision and accepts later explicit revision references", () => {
    const first = cloneGoldenNotification("model-input-prepared");
    const later = cloneGoldenNotification("model-input-prepared-step-2");
    expect(later.payload.contextRevision).toBeNull();
    expect(() => parseRuntimeEventNotification(later.envelope)).not.toThrow();
    first.payload.contextRevision = null;
    expect(() => parseRuntimeEventNotification(first.envelope)).toThrow();
    for (const key of ["contextRevision", "stepOrdinal"]) {
      const changed = structuredClone(later);
      asWireObject(changed.payload.stepInput, "Step input")[key] = 0;
      expect(() => parseRuntimeEventNotification(changed.envelope)).toThrow();
    }
    const changed = structuredClone(later);
    const budget = asWireObject(asWireObject(changed.payload.stepInput, "Step input").budget, "Budget");
    budget.currentRunTokens = (budget.currentRunTokens as number) + 1;
    budget.totalTokens = (budget.totalTokens as number) + 1;
    expect(() => parseRuntimeEventNotification(changed.envelope)).toThrow();
  });

  it("validates process facts and binds them to the owning tool call", () => {
    const source = cloneGoldenNotification("process-recorded-1");
    expect(() => parseRuntimeEventNotification(source.envelope)).not.toThrow();
    for (const [key, value] of [["runId", "foreign"], ["threadId", "foreign"], ["itemId", "foreign"], ["state", "done"], ["stepOrdinal", 0], ["pid", -1], ["stdout", 3]]) {
      const changed = structuredClone(source);
      asWireObject(changed.payload.process, "Process")[key as string] = value;
      expect(() => parseRuntimeEventNotification(changed.envelope)).toThrow();
    }
  });

  it("requires explicit historical Run progress and nonnegative usage", () => {
    const source = cloneGoldenResponse("turn-list-page");
    const firstRun = (response: typeof source) => asWireObject(asWireArray(asWireObject(asWireArray(response.result.turns, "Turns")[0], "Turn").runs, "Runs")[0], "Run");
    for (const key of ["startedAt", "modelCalls", "reasonCode"]) {
      const changed = structuredClone(source);
      delete firstRun(changed)[key];
      expect(() => parseRuntimeMethodResult(changed.method, changed.result, changed.requestParams)).toThrow();
    }
    const changed = structuredClone(source);
    firstRun(changed).modelCalls = -1;
    expect(() => parseRuntimeMethodResult(changed.method, changed.result, changed.requestParams)).toThrow();
  });

});
