import type {
  AgentEvent,
  ArtifactEvent,
  Branch,
  FileChangeEvent,
  MessageEvent,
  Project,
  RunStatus,
  ScenarioId,
  StatusEvent,
  Thread,
  ToolCallEvent,
  Turn,
} from "./domain";
import { appEventText } from "./domain";

export type EventSink = (event: AgentEvent, turnStatus?: RunStatus) => void;

export interface AgentClient {
  playScenario(id: ScenarioId, sink: EventSink): Promise<RunStatus>;
  submitPrompt(prompt: string, turnId: string, sink: EventSink): Promise<RunStatus>;
  resolvePermission(
    decision: "allow" | "deny",
    turnId: string,
    sink: EventSink,
  ): Promise<RunStatus>;
  recover(
    strategy: "retry" | "resume",
    turnId: string,
    sink: EventSink,
  ): Promise<RunStatus>;
  cancel(): void;
}

const AT = "2026-08-05T06:00:00.000Z";

const message = (
  id: string,
  turnId: string,
  role: MessageEvent["role"],
  content: string,
): MessageEvent => ({ id, turnId, type: "message", role, content, createdAt: AT });

const baseThread = (
  projectId: string,
  threadId: string,
  title: string,
  scenarioId: ScenarioId,
  prompt: string,
): Thread => {
  const branchId = `${threadId}-main`;
  const turnId = `${threadId}-turn`;
  return {
    id: threadId,
    projectId,
    title,
    scenarioId,
    activeBranchId: branchId,
    updatedAt: AT,
    branches: [
      {
        id: branchId,
        threadId,
        label: { source: "app", kind: "main" },
        createdAt: AT,
        turns: [
          {
            id: turnId,
            branchId,
            status: "queued",
            events: [message(`${threadId}-user`, turnId, "user", prompt)],
          },
        ],
      },
    ],
  };
};

export function createInitialProjects(): Project[] {
  return [
    {
      id: "project-research",
      name: "Research",
      color: "#9b8cff",
      rootUri: "C:\\Workspace\\research",
      instructions: "Verify sources and preserve useful working artifacts.",
    },
    {
      id: "project-personal",
      name: "Personal operations",
      color: "#55c6a9",
      rootUri: "C:\\Users\\demo\\Documents",
    },
  ];
}

interface ScenarioThreadSeed {
  scenarioId: ScenarioId;
  projectId: string;
  threadId: string;
  title: string;
  prompt: string;
}

const SCENARIO_THREAD_SEEDS: readonly ScenarioThreadSeed[] = [
  {
    scenarioId: "streaming",
    projectId: "project-research",
    threadId: "thread-streaming",
    title: "Summarize the field notes",
    prompt: "Turn these field notes into a concise research summary with clear next steps.",
  },
  {
    scenarioId: "tools",
    projectId: "project-research",
    threadId: "thread-tools",
    title: "Check the source archive",
    prompt: "Inspect the source archive, verify its metadata, and tell me what needs attention.",
  },
  {
    scenarioId: "artifact",
    projectId: "project-research",
    threadId: "thread-artifact",
    title: "Build the weekly brief",
    prompt: "Create a reusable weekly brief from the notes and save it beside the source material.",
  },
  {
    scenarioId: "permission",
    projectId: "project-personal",
    threadId: "thread-permission",
    title: "Organize download receipts",
    prompt: "Move the receipt PDFs into the 2026 expense folders and keep the originals recoverable.",
  },
  {
    scenarioId: "recovery",
    projectId: "project-personal",
    threadId: "thread-recovery",
    title: "Recover interrupted export",
    prompt: "Continue the report export that stopped when the local worker disconnected.",
  },
];

function scenarioThread(seed: ScenarioThreadSeed) {
  return baseThread(
    seed.projectId,
    seed.threadId,
    seed.title,
    seed.scenarioId,
    seed.prompt,
  );
}

const standaloneThread = (
  threadId: string,
  title: string,
  prompt: string,
  response: string,
): Thread => {
  const branchId = `${threadId}-main`;
  const turnId = `${threadId}-turn`;
  return {
    id: threadId,
    projectId: null,
    title,
    activeBranchId: branchId,
    updatedAt: AT,
    branches: [
      {
        id: branchId,
        threadId,
        label: { source: "app", kind: "main" },
        createdAt: AT,
        turns: [
          {
            id: turnId,
            branchId,
            status: "completed",
            events: [
              message(`${threadId}-user`, turnId, "user", prompt),
              message(`${threadId}-assistant`, turnId, "assistant", response),
            ],
          },
        ],
      },
    ],
  };
};

export function createInitialThreads(): Thread[] {
  return [
    ...SCENARIO_THREAD_SEEDS.map(scenarioThread),
    standaloneThread(
      "thread-recent-priorities",
      "Plan tomorrow's priorities",
      "Help me choose the three most important things to finish tomorrow.",
      "Start with the deadline-bound task, protect one focused block for the hardest item, and leave the smallest task for the end of the day.",
    ),
    standaloneThread(
      "thread-recent-notes",
      "Compare note-taking methods",
      "Compare a daily log with topic-based notes for personal research.",
      "Use a daily log for capture and topic notes for synthesis. Linking the two gives you chronology without losing long-term structure.",
    ),
    standaloneThread(
      "thread-recent-follow-up",
      "Draft a follow-up message",
      "Write a short, friendly follow-up asking whether the proposal was reviewed.",
      "Hi — just checking whether you had a chance to review the proposal. Happy to clarify anything or adjust the next steps.",
    ),
  ];
}

export function freshScenarioThread(id: ScenarioId) {
  const seed = SCENARIO_THREAD_SEEDS.find((candidate) => candidate.scenarioId === id);
  if (seed) return scenarioThread(seed);
  throw new Error(`Unknown mock scenario: ${id}`);
}

type Step = {
  delay: number;
  event: AgentEvent;
  status?: RunStatus;
};

export class MockAgentClient implements AgentClient {
  private generation = 0;
  private localId = 0;

  cancel() {
    this.generation += 1;
  }

  private async wait(delay: number, token: number) {
    await new Promise<void>((resolve) => window.setTimeout(resolve, delay));
    return token === this.generation;
  }

  private async emitSteps(steps: Step[], sink: EventSink) {
    const token = ++this.generation;
    let terminal: RunStatus = "completed";
    for (const step of steps) {
      if (!(await this.wait(step.delay, token))) return "interrupted";
      sink(step.event, step.status);
      terminal = step.status ?? terminal;
    }
    return terminal;
  }

  async playScenario(id: ScenarioId, sink: EventSink) {
    const thread = freshScenarioThread(id);
    const turnId = thread.branches[0].turns[0].id;
    const event = (suffix: string) => `${thread.id}-${suffix}`;

    if (id === "streaming") {
      const chunks = [
        "I found three themes in the notes: **scope clarity**, **observable agent work**, and **recoverable state**.",
        "I found three themes in the notes: **scope clarity**, **observable agent work**, and **recoverable state**.\n\nThe strongest signal is that the interface should make the agent's current commitment obvious without exposing protocol noise.",
        "I found three themes in the notes: **scope clarity**, **observable agent work**, and **recoverable state**.\n\nThe strongest signal is that the interface should make the agent's current commitment obvious without exposing protocol noise.\n\n### Recommended next steps\n\n1. Validate the five core run states in the desktop prototype.\n2. Keep projects, threads, runs, and artifacts as separate records.\n3. Connect the real kernel only after the interaction contract feels stable.",
      ];
      return this.emitSteps(
        chunks.map((content, index) => ({
          delay: index === 0 ? 180 : 420,
          status: index === chunks.length - 1 ? "completed" : "running",
          event: {
            ...message(event("assistant"), turnId, "assistant", content),
            status: index === chunks.length - 1 ? "complete" : "streaming",
          },
        })),
        sink,
      );
    }

    if (id === "tools") {
      const searchCall: ToolCallEvent = {
        id: event("search"),
        turnId,
        type: "tool_call",
        toolName: "archive.inspect",
        label: appEventText("tool.inspectArchiveMetadata"),
        status: "running",
        arguments: { path: "sources/field-notes.zip", verifyChecksums: true },
        createdAt: AT,
      };
      const shellCall: ToolCallEvent = {
        id: event("extract"),
        turnId,
        type: "tool_call",
        toolName: "workspace.extract",
        label: appEventText("tool.testArchiveExtraction"),
        status: "running",
        arguments: { destination: ".ikaros/tmp/source-preview", mode: "read-only" },
        createdAt: AT,
      };
      return this.emitSteps(
        [
          { delay: 160, event: searchCall, status: "running" },
          {
            delay: 500,
            event: { ...searchCall, status: "success", durationMs: 428 },
            status: "running",
          },
          {
            delay: 80,
            event: {
              id: event("search-result"),
              turnId,
              type: "tool_result",
              toolCallId: searchCall.id,
              status: "success",
              summary: appEventText("result.archiveManifestVerified"),
              output: "42 entries · SHA-256 manifest valid · last updated 2026-08-04",
              createdAt: AT,
            },
            status: "running",
          },
          { delay: 340, event: shellCall, status: "running" },
          {
            delay: 520,
            event: { ...shellCall, status: "error", durationMs: 503 },
            status: "running",
          },
          {
            delay: 80,
            event: {
              id: event("extract-result"),
              turnId,
              type: "tool_result",
              toolCallId: shellCall.id,
              status: "error",
              summary: appEventText("result.windowsReservedNames"),
              output:
                "EINVAL: reserved device name `con.txt`\nThe archive is intact; extraction stopped before writing files.",
              createdAt: AT,
            },
            status: "running",
          },
          {
            delay: 360,
            event: message(
              event("assistant"),
              turnId,
              "assistant",
              "The archive itself is healthy, but two entries use Windows-reserved filenames. No files were written. Rename those entries on a Unix-compatible system or extract with a sanitizing archive tool.",
            ),
            status: "completed",
          },
        ],
        sink,
      );
    }

    if (id === "permission") {
      return this.emitSteps(
        [
          {
            delay: 240,
            status: "waiting_permission",
            event: {
              id: event("permission"),
              turnId,
              type: "permission_request",
              requestId: "permission-move-receipts",
              title: appEventText("permission.moveReceiptsTitle", { count: 18 }),
              description: appEventText("permission.moveReceiptsDescription"),
              resource: "C:\\Users\\demo\\Downloads\\receipts\\*.pdf",
              risk: "medium",
              status: "pending",
              createdAt: AT,
            },
          },
        ],
        sink,
      );
    }

    if (id === "recovery") {
      const exportCall: ToolCallEvent = {
        id: event("export"),
        turnId,
        type: "tool_call",
        toolName: "document.export",
        label: appEventText("tool.exportReportPackage"),
        status: "running",
        arguments: { format: ["docx", "pdf"], output: "reports/weekly" },
        createdAt: AT,
      };
      return this.emitSteps(
        [
          { delay: 180, event: exportCall, status: "running" },
          {
            delay: 540,
            event: {
              ...exportCall,
              status: "interrupted",
              durationMs: 540,
            },
            status: "interrupted",
          },
          {
            delay: 80,
            event: {
              id: event("interrupt"),
              turnId,
              type: "interrupt",
              copy: { source: "app", kind: "worker_disconnected" },
              recoverable: true,
              status: "interrupted",
              createdAt: AT,
            },
            status: "interrupted",
          },
        ],
        sink,
      );
    }

    const artifact: ArtifactEvent = {
      id: event("artifact"),
      turnId,
      type: "artifact",
      artifactId: "artifact-weekly-brief",
      title: "Weekly research brief",
      mediaType: "text/markdown",
      version: 1,
      content:
        "# Weekly research brief\n\n## Signal\n\nTeams understand autonomous work faster when progress, permissions, and recoverability are visible in the same surface.\n\n## Evidence\n\n- **Tool logs are noisy:** render typed activity cards.\n- **Long chats lose structure:** make projects and branches explicit.\n- **Generated files disappear in chat:** persist artifacts independently.\n\n## Next step\n\nValidate the event model with a high-fidelity desktop prototype before binding it to a kernel protocol.",
      createdAt: AT,
    };
    const change: FileChangeEvent = {
      id: event("file"),
      turnId,
      type: "file_change",
      path: "research/briefs/2026-08-05-weekly.md",
      operation: "created",
      additions: 17,
      deletions: 0,
      diff:
        "+# Weekly research brief\n+\n+## Signal\n+\n+Progress, permissions, and recoverability belong in one surface.\n+\n+## Next step\n+\n+Validate the event model before protocol binding.",
      createdAt: AT,
    };
    const buildStatus: StatusEvent = {
      id: event("status"),
      turnId,
      type: "status",
      tone: "neutral",
      label: appEventText("status.buildingBrief", { count: 12 }),
      detail: appEventText("status.sourcesLinked"),
      createdAt: AT,
    };
    return this.emitSteps(
      [
        {
          delay: 160,
          event: buildStatus,
          status: "running",
        },
        { delay: 600, event: artifact, status: "running" },
        { delay: 180, event: change, status: "running" },
        {
          delay: 180,
          event: {
            ...buildStatus,
            tone: "success",
            label: appEventText("status.briefBuilt", { count: 12 }),
          },
          status: "running",
        },
        {
          delay: 320,
          event: message(
            event("assistant"),
            turnId,
            "assistant",
            "I created the brief and saved a Markdown copy in the research workspace. The generated artifact and related file change are recorded above.",
          ),
          status: "completed",
        },
      ],
      sink,
    );
  }

  async submitPrompt(prompt: string, turnId: string, sink: EventSink) {
    const id = `local-${++this.localId}`;
    const first = `I'm mapping **${prompt.trim()}** onto the current project context.`;
    const final = `${first}\n\nThis prototype is using a deterministic local event stream. The production AgentClient can replace it while preserving the same turns, typed events, and recovery controls.`;
    return this.emitSteps(
      [
        {
          delay: 160,
          event: { ...message(`${id}-assistant`, turnId, "assistant", first), status: "streaming" },
          status: "running",
        },
        {
          delay: 520,
          event: { ...message(`${id}-assistant`, turnId, "assistant", final), status: "complete" },
          status: "completed",
        },
      ],
      sink,
    );
  }

  async resolvePermission(
    decision: "allow" | "deny",
    turnId: string,
    sink: EventSink,
  ) {
    const allowed = decision === "allow";
    return this.emitSteps(
      [
        {
          delay: 120,
          status: "running",
          event: {
            id: "thread-permission-permission",
            turnId,
            type: "permission_request",
            requestId: "permission-move-receipts",
            title: appEventText("permission.moveReceiptsTitle", { count: 18 }),
            description: appEventText("permission.moveReceiptsDescription"),
            resource: "C:\\Users\\demo\\Downloads\\receipts\\*.pdf",
            risk: "medium",
            status: allowed ? "allowed" : "denied",
            createdAt: AT,
          },
        },
        {
          delay: allowed ? 420 : 180,
          status: "completed",
          event: message(
            `permission-${decision}-assistant`,
            turnId,
            "assistant",
            allowed
              ? "Done. I moved 18 PDFs into month folders and left a reversible operation record."
              : "Understood. I did not touch the files. I can prepare a dry-run list instead.",
          ),
        },
      ],
      sink,
    );
  }

  async recover(
    strategy: "retry" | "resume",
    turnId: string,
    sink: EventSink,
  ) {
    const resume = strategy === "resume";
    const prefix = `recovery-${strategy}`;
    const exportCall: ToolCallEvent = {
      id: "thread-recovery-export",
      turnId,
      type: "tool_call",
      toolName: "document.export",
      label: appEventText("tool.exportReportPackage"),
      status: "success",
      arguments: { format: ["docx", "pdf"], output: "reports/weekly" },
      durationMs: resume ? 640 : 720,
      createdAt: AT,
    };
    return this.emitSteps(
      [
        {
          delay: 100,
          status: "running",
          event: {
            id: "thread-recovery-interrupt",
            turnId,
            type: "interrupt",
            copy: {
              source: "app",
              kind: resume ? "recovering_checkpoint" : "retrying_export",
            },
            recoverable: true,
            status: "recovering",
            createdAt: AT,
          },
        },
        {
          delay: 420,
          status: "running",
          event: exportCall,
        },
        {
          delay: 100,
          status: "running",
          event: {
            id: `${prefix}-status`,
            turnId,
            type: "status",
            tone: "success",
            label: appEventText(
              resume ? "status.checkpointRestored" : "status.exportStepRetried",
            ),
            detail: appEventText("status.outputsValidated"),
            createdAt: AT,
          },
        },
        {
          delay: 120,
          status: "running",
          event: {
            id: "thread-recovery-interrupt",
            turnId,
            type: "interrupt",
            copy: { source: "app", kind: "recovery_completed" },
            recoverable: false,
            status: "recovered",
            createdAt: AT,
          },
        },
        {
          delay: 260,
          status: "completed",
          event: {
            ...message(
              `${prefix}-assistant`,
              turnId,
              "assistant",
              "Recovery completed. The DOCX and PDF are available in `reports/weekly`, and the original interrupted run remains in the activity history.",
            ),
            status: "complete",
          },
        },
      ],
      sink,
    );
  }
}

export function createBranchFromMessage(
  thread: Thread,
  sourceEventId: string,
  replacement: string,
  branchId: string,
): Thread {
  const sourceBranch = thread.branches.find(
    (branch) => branch.id === thread.activeBranchId,
  );
  if (!sourceBranch) return thread;

  const copiedTurns: Turn[] = [];
  let found = false;
  for (const turn of sourceBranch.turns) {
    if (found) break;
    const copiedEvents: AgentEvent[] = [];
    for (const event of turn.events) {
      if (found) break;
      if (event.id === sourceEventId && event.type === "message") {
        copiedEvents.push({ ...event, content: replacement, status: "complete" });
        copiedEvents.push({
          id: `${branchId}-created`,
          turnId: turn.id,
          type: "branch_created",
          branchId,
          sourceEventId,
          copy: { source: "app", kind: "edited_message" },
          createdAt: AT,
        });
        found = true;
      } else {
        copiedEvents.push(event);
      }
    }
    copiedTurns.push({ ...turn, branchId, status: "completed", events: copiedEvents });
  }
  if (!found) return thread;

  const branch: Branch = {
    id: branchId,
    threadId: thread.id,
    label: {
      source: "app",
      kind: "number",
      number: thread.branches.length,
    },
    parentBranchId: sourceBranch.id,
    forkedFromEventId: sourceEventId,
    createdAt: AT,
    turns: copiedTurns,
  };
  return {
    ...thread,
    activeBranchId: branchId,
    branches: [...thread.branches, branch],
    updatedAt: AT,
  };
}
