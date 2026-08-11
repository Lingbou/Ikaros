import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  BranchEvent,
  InterruptEvent,
  MessageEvent,
  PermissionEvent,
  StatusEvent,
  ToolCallEvent,
  ToolResultEvent,
} from "../domain";
import { appEventText, externalEventText } from "../domain";
import { setUiLanguage } from "../i18n";
import { useAppStore } from "../store";
import { EventCard } from "./EventCard";

const markdownRender = vi.hoisted(() => vi.fn());

vi.mock("react-markdown", () => ({
  default: ({ children }: { children: string }) => {
    markdownRender(children);
    return <>{children}</>;
  },
}));

const initialState = useAppStore.getState();

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
  setUiLanguage("en");
});

describe("EventCard localization boundary", () => {
  it("renders process command cards with localized copy and multiline invariant output", () => {
    const toolCall: ToolCallEvent = {
      id: "process-call",
      turnId: "process-turn",
      createdAt: "2026-08-12T00:00:00.000Z",
      type: "tool_call",
      toolName: "process.run",
      label: appEventText("tool.runProcess"),
      status: "interrupted",
      arguments: { command: "Write-Output original-command" },
      durationMs: 25,
    };
    const output = "original-output\nsecond-line";
    const result: ToolResultEvent = {
      id: "process-result",
      turnId: "process-turn",
      createdAt: "2026-08-12T00:00:00.025Z",
      type: "tool_result",
      toolCallId: toolCall.id,
      status: "interrupted",
      summary: appEventText("result.processInterrupted"),
      output,
    };

    const { container } = render(
      <>
        <EventCard event={toolCall} />
        <EventCard event={result} />
      </>,
    );

    expect(screen.getByText("Running command")).toBeInTheDocument();
    expect(screen.getByText("Command interrupted")).toBeInTheDocument();
    expect(container).toHaveTextContent("Write-Output original-command");
    expect(container.querySelector("pre")?.textContent).toBe(output);

    act(() => setUiLanguage("zh-CN"));

    expect(screen.getByText("正在运行命令")).toBeInTheDocument();
    expect(screen.getByText("命令已中断")).toBeInTheDocument();
    expect(container).toHaveTextContent("Write-Output original-command");
    expect(container.querySelector("pre")?.textContent).toBe(output);
  });

  it("retranslates app-owned event copy while preserving command data and paths", () => {
    setUiLanguage("en");
    useAppStore.setState({ runStatus: "waiting_permission" });

    const toolCall: ToolCallEvent = {
      id: "tool-call-localization",
      turnId: "turn-localization",
      createdAt: "2026-08-05T06:00:00.000Z",
      type: "tool_call",
      toolName: "archive.inspect",
      label: appEventText("tool.inspectArchiveMetadata"),
      status: "success",
      arguments: {
        path: "sources/field-notes.zip",
        verifyChecksums: true,
      },
    };
    const rawOutput = "42 entries · SHA-256 manifest valid";
    const toolResult: ToolResultEvent = {
      id: "tool-result-localization",
      turnId: "turn-localization",
      createdAt: "2026-08-05T06:00:01.000Z",
      type: "tool_result",
      toolCallId: toolCall.id,
      status: "success",
      summary: appEventText("result.archiveManifestVerified"),
      output: rawOutput,
    };
    const resource = "C:\\Users\\demo\\Downloads\\receipts\\*.pdf";
    const permission: PermissionEvent = {
      id: "permission-app-copy",
      turnId: "turn-localization",
      createdAt: "2026-08-05T06:00:02.000Z",
      type: "permission_request",
      requestId: "permission-app-copy",
      title: appEventText("permission.moveReceiptsTitle", { count: 18 }),
      description: appEventText("permission.moveReceiptsDescription"),
      resource,
      risk: "medium",
      status: "pending",
    };
    const status: StatusEvent = {
      id: "status-app-copy",
      turnId: "turn-localization",
      createdAt: "2026-08-05T06:00:03.000Z",
      type: "status",
      tone: "success",
      label: appEventText("status.buildingBrief", { count: 12 }),
      detail: appEventText("status.sourcesLinked"),
    };

    const { container } = render(
      <>
        <EventCard event={toolCall} />
        <EventCard event={toolResult} />
        <EventCard event={permission} />
        <EventCard event={status} />
      </>,
    );

    expect(screen.getByText("Inspecting archive metadata")).toBeInTheDocument();
    expect(screen.getByText("Archive manifest verified")).toBeInTheDocument();
    expect(screen.getByText("Allow moving 18 receipt files?")).toBeInTheDocument();
    expect(screen.getByText("Building brief from 12 notes")).toBeInTheDocument();
    expect(container).toHaveTextContent("Sources stay linked to the artifact.");

    const invariantText = `archive.inspect · ${JSON.stringify(toolCall.arguments)}`;
    expect(container).toHaveTextContent(invariantText);
    expect(container).toHaveTextContent(rawOutput);
    expect(container).toHaveTextContent(resource);

    act(() => setUiLanguage("zh-CN"));

    expect(screen.getByText("正在检查压缩包元数据")).toBeInTheDocument();
    expect(screen.getByText("压缩包清单已验证")).toBeInTheDocument();
    expect(screen.getByText("允许移动 18 个收据文件吗？")).toBeInTheDocument();
    expect(screen.getByText("正在根据 12 条笔记生成简报")).toBeInTheDocument();
    expect(container).toHaveTextContent("来源会继续与产物保持关联。");
    expect(screen.queryByText("Inspecting archive metadata")).not.toBeInTheDocument();
    expect(screen.queryByText("Archive manifest verified")).not.toBeInTheDocument();
    expect(container).toHaveTextContent(invariantText);
    expect(container).toHaveTextContent(rawOutput);
    expect(container).toHaveTextContent(resource);
  });

  it("translates product actions while preserving event content", () => {
    setUiLanguage("zh-CN");
    useAppStore.setState({ runStatus: "waiting_permission" });
    const event: PermissionEvent = {
      id: "permission-localization",
      turnId: "turn-localization",
      createdAt: "2026-08-05T06:00:00.000Z",
      type: "permission_request",
      requestId: "permission-localization",
      title: externalEventText("Allow deleting the temporary export?"),
      description: externalEventText("This sentence belongs to the event payload."),
      resource: "C:\\Temp\\export.pdf",
      risk: "medium",
      status: "pending",
    };

    render(<EventCard event={event} />);

    expect(screen.getByText("Allow deleting the temporary export?")).toBeInTheDocument();
    expect(screen.getByText("This sentence belongs to the event payload.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "拒绝" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "允许一次" })).toBeInTheDocument();
  });

  it("leaves user and assistant messages unchanged when the UI language changes", () => {
    const user: MessageEvent = {
      id: "user-localization-boundary",
      turnId: "turn-message-boundary",
      createdAt: "2026-08-05T06:00:00.000Z",
      type: "message",
      role: "user",
      content: "Please keep this English request unchanged.",
      status: "complete",
    };
    const assistant: MessageEvent = {
      id: "assistant-localization-boundary",
      turnId: "turn-message-boundary",
      createdAt: "2026-08-05T06:00:01.000Z",
      type: "message",
      role: "assistant",
      content: "This answer also belongs to the conversation.",
      status: "complete",
    };

    render(
      <Tooltip.Provider>
        <EventCard event={user} />
        <EventCard event={assistant} />
      </Tooltip.Provider>,
    );

    act(() => setUiLanguage("zh-CN"));

    expect(screen.getByText(user.content)).toBeInTheDocument();
    expect(screen.getByText(assistant.content)).toBeInTheDocument();
  });
});

describe("Assistant message presentation", () => {
  it("renders assistant content directly on the conversation rail without a bot avatar", () => {
    const event: MessageEvent = {
      id: "assistant-message",
      turnId: "turn-assistant-message",
      createdAt: "2026-08-06T06:00:00.000Z",
      type: "message",
      role: "assistant",
      content: "A direct assistant response.",
      status: "streaming",
    };

    const { container } = render(<EventCard event={event} />);

    expect(screen.getByText(event.content)).toBeInTheDocument();
    expect(container.querySelector(".lucide-bot")).not.toBeInTheDocument();
    const markdown = container.querySelector(".markdown-body");
    const assistantRoot = markdown?.parentElement;
    expect(assistantRoot).toHaveClass("group", "min-w-0");
    expect(assistantRoot?.children).toHaveLength(1);
    expect(assistantRoot?.firstElementChild).toBe(markdown);
  });

  it("skips old Markdown when the event reference is unchanged", () => {
    const event: MessageEvent = {
      id: "assistant-memo",
      turnId: "turn-assistant-memo",
      createdAt: "2026-08-06T06:00:00.000Z",
      type: "message",
      role: "assistant",
      content: "Stable response.",
      status: "streaming",
    };

    const { rerender } = render(<EventCard event={event} />);
    expect(markdownRender).toHaveBeenCalledTimes(1);

    rerender(<EventCard event={event} />);
    act(() => useAppStore.setState({ runStatus: "running" }));
    expect(markdownRender).toHaveBeenCalledTimes(1);

    const updated = { ...event, content: "Updated response." };
    rerender(<EventCard event={updated} />);
    expect(markdownRender).toHaveBeenCalledTimes(2);
  });

  it("marks a partial cancelled response as interrupted", () => {
    const event: MessageEvent = {
      id: "assistant-interrupted",
      turnId: "turn-interrupted",
      createdAt: "2026-08-11T06:00:00.000Z",
      type: "message",
      role: "assistant",
      content: "Partial response",
      status: "interrupted",
    };

    render(
      <Tooltip.Provider>
        <EventCard event={event} />
      </Tooltip.Provider>,
    );

    expect(screen.getByText(event.content)).toBeInTheDocument();
    expect(screen.getByText("Interrupted")).toBeInTheDocument();
  });
});

describe("Message typography", () => {
  it("uses the shared conversation body scale for user messages", () => {
    const event: MessageEvent = {
      id: "user-message",
      turnId: "turn-user-message",
      createdAt: "2026-08-06T06:00:00.000Z",
      type: "message",
      role: "user",
      content: "Keep message text compact and readable.",
      status: "complete",
    };

    render(
      <Tooltip.Provider>
        <EventCard event={event} />
      </Tooltip.Provider>,
    );

    expect(screen.getByText(event.content)).toHaveClass(
      "text-[14px]",
      "leading-[22px]",
    );
  });
});

describe("InterruptCard recovery states", () => {
  it("renders a recovered run as a successful terminal state", () => {
    useAppStore.setState({ runStatus: "completed" });
    const event: InterruptEvent = {
      id: "interrupt-recovered",
      turnId: "turn-recovered",
      createdAt: "2026-08-05T06:00:00.000Z",
      type: "interrupt",
      copy: {
        source: "external",
        title: "Recovery completed",
        description: "The saved run is healthy again.",
      },
      recoverable: true,
      status: "recovered",
    };

    const { container } = render(<EventCard event={event} />);

    expect(container.querySelector('[data-status="recovered"]')).toHaveClass(
      "border-[#72d3a7]/30",
      "bg-[#24463a]/22",
    );
    expect(screen.getByRole("img", { name: "Recovered" })).toHaveClass(
      "text-[#72d3a7]",
    );
    expect(container.querySelector(".animate-spin")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Recover session" })).not.toBeInTheDocument();
  });

  it("retranslates app-authored stop and branch copy without rewriting external events", () => {
    const stopped: InterruptEvent = {
      id: "interrupt-stopped",
      turnId: "turn-stopped",
      createdAt: "2026-08-05T06:00:00.000Z",
      type: "interrupt",
      copy: { source: "app", kind: "run_stopped" },
      recoverable: false,
      status: "interrupted",
    };
    const branched: BranchEvent = {
      id: "branch-created",
      turnId: "turn-stopped",
      createdAt: "2026-08-05T06:00:00.000Z",
      type: "branch_created",
      branchId: "branch-1",
      sourceEventId: "message-1",
      copy: { source: "app", kind: "edited_message" },
    };

    render(
      <>
        <EventCard event={stopped} />
        <EventCard event={branched} />
      </>,
    );

    expect(screen.getByText("Run stopped")).toBeInTheDocument();
    expect(screen.getByText("Branched from edited message")).toBeInTheDocument();

    act(() => setUiLanguage("zh-CN"));

    expect(screen.getByText("运行已停止")).toBeInTheDocument();
    expect(screen.getByText("已从编辑的消息创建分支")).toBeInTheDocument();
    expect(screen.queryByText("Run stopped")).not.toBeInTheDocument();
  });

  it("uses warning semantics instead of a success check for a stopped run", () => {
    const event: StatusEvent = {
      id: "status-stopped",
      turnId: "turn-stopped",
      createdAt: "2026-08-05T06:00:00.000Z",
      type: "status",
      tone: "warning",
      label: externalEventText("Stopped by you"),
    };

    const { container } = render(<EventCard event={event} />);

    expect(container.querySelector('[data-tone="warning"]')).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Warning" })).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "Complete" })).not.toBeInTheDocument();
  });
});
