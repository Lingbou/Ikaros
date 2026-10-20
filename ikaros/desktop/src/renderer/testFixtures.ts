import {
  type RuntimeItemHistory,
  type RuntimeThreadSummary,
  type RuntimeTurnHistory,
  type RuntimeWorkspaceSummary,
} from "../shared/runtime";
import type { Project, Thread } from "./domain";
import { projectRuntimeThreadHistory } from "./runtimeProjection";

export const TEST_RUNTIME_AT = "2026-08-05T06:00:00.000Z";

export const TEST_RESEARCH_WORKSPACE: RuntimeWorkspaceSummary = {
  id: "project-research",
  name: "Research",
  rootUri: "C:\\Workspace\\research",
};

const PERSONAL_WORKSPACE: RuntimeWorkspaceSummary = {
  id: "project-personal",
  name: "Personal operations",
  rootUri: "C:\\Users\\demo\\Documents",
};

export const TEST_PROJECTS: Project[] = [
  {
    id: TEST_RESEARCH_WORKSPACE.id,
    name: TEST_RESEARCH_WORKSPACE.name,
    rootUri: TEST_RESEARCH_WORKSPACE.rootUri ?? undefined,
    color: "#9b8cff",
    instructions: "Verify sources and preserve useful working artifacts.",
  },
  {
    id: PERSONAL_WORKSPACE.id,
    name: PERSONAL_WORKSPACE.name,
    rootUri: PERSONAL_WORKSPACE.rootUri ?? undefined,
    color: "#55c6a9",
  },
];

export const TEST_EXPANDED_PROJECTS: Record<string, boolean> = {
  "project-research": true,
  "project-personal": true,
};

function messageItem(
  id: string,
  turnId: string,
  role: "user" | "assistant",
  content: string,
): RuntimeItemHistory {
  return {
    id,
    turnId,
    runId: `${turnId}-run`,
    ordinal: 0,
    kind: "message",
    role,
    status: "completed",
    content,
    data: {},
    createdAt: TEST_RUNTIME_AT,
    updatedAt: TEST_RUNTIME_AT,
  };
}

function completedTurn(
  threadId: string,
  ordinal: number,
  prompt: string,
  response: string,
): RuntimeTurnHistory {
  const turnId = `${threadId}-turn-${ordinal}`;
  return {
    id: turnId,
    threadId,
    branchId: `${threadId}-main`,
    ordinal,
    status: "completed",
    createdAt: TEST_RUNTIME_AT,
    updatedAt: TEST_RUNTIME_AT,
    runs: [
      {
        id: `${turnId}-run`,
        turnId,
        providerId: "deepseek",
        modelId: "deepseek-chat",
        executionPolicy: "full_access",
        status: "completed",
        reasonCode: null,
        createdAt: TEST_RUNTIME_AT,
        startedAt: TEST_RUNTIME_AT,
        settledAt: TEST_RUNTIME_AT,
        modelCalls: 1,
        compactions: 0,
        items: [
          messageItem(`${threadId}-user-${ordinal}`, turnId, "user", prompt),
          messageItem(`${threadId}-assistant-${ordinal}`, turnId, "assistant", response),
        ],
      },
    ],
  };
}

function summary(
  threadId: string,
  title: string,
  workspace: RuntimeWorkspaceSummary | null,
): RuntimeThreadSummary {
  return {
    id: threadId,
    title,
    defaultBranchId: `${threadId}-main`,
    workspace,
    createdAt: TEST_RUNTIME_AT,
    updatedAt: TEST_RUNTIME_AT,
    archivedAt: null,
  };
}

function fixtureThread(
  threadId: string,
  title: string,
  workspace: RuntimeWorkspaceSummary | null,
  prompt: string,
  response: string,
): Thread {
  return projectRuntimeThreadHistory(summary(threadId, title, workspace), [
    completedTurn(threadId, 1, prompt, response),
  ]);
}

export function createFixtureThreads(): Thread[] {
  return [
    fixtureThread(
      "thread-streaming",
      "Summarize the field notes",
      TEST_RESEARCH_WORKSPACE,
      "Turn these field notes into a concise research summary with clear next steps.",
      "The notes point to scope clarity, observable agent work, and recoverable state.",
    ),
    fixtureThread(
      "thread-tools",
      "Check the source archive",
      TEST_RESEARCH_WORKSPACE,
      "Inspect the source archive, verify its metadata, and tell me what needs attention.",
      "The archive metadata is consistent and ready for review.",
    ),
    fixtureThread(
      "thread-artifact",
      "Build the weekly brief",
      TEST_RESEARCH_WORKSPACE,
      "Create a reusable weekly brief from the notes and save it beside the source material.",
      "The weekly brief is ready for review.",
    ),
    fixtureThread(
      "thread-permission",
      "Organize download receipts",
      PERSONAL_WORKSPACE,
      "Move the receipt PDFs into the 2026 expense folders and keep the originals recoverable.",
      "The receipts are organized by month.",
    ),
    fixtureThread(
      "thread-recovery",
      "Recover interrupted export",
      PERSONAL_WORKSPACE,
      "Continue the report export that stopped when the local worker disconnected.",
      "The report export finished after recovery.",
    ),
    fixtureThread(
      "thread-recent-priorities",
      "Plan tomorrow's priorities",
      null,
      "Help me choose the three most important things to finish tomorrow.",
      "Start with the deadline-bound task, protect one focused block for the hardest item, and leave the smallest task for the end of the day.",
    ),
    fixtureThread(
      "thread-recent-notes",
      "Compare note-taking methods",
      null,
      "Compare a daily log with topic-based notes for personal research.",
      "Use a daily log for capture and topic notes for synthesis. Linking the two gives you chronology without losing long-term structure.",
    ),
    fixtureThread(
      "thread-recent-follow-up",
      "Draft a follow-up message",
      null,
      "Write a short, friendly follow-up asking whether the proposal was reviewed.",
      "Hi - just checking whether you had a chance to review the proposal. Happy to clarify anything or adjust the next steps.",
    ),
  ];
}
