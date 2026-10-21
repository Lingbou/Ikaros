import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  MessageEvent,
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

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
  setUiLanguage("en");
  vi.unstubAllGlobals();
});

describe("EventCard localization boundary", () => {
  it("opens file views using the Tool Call identity without toggling its result disclosure", () => {
    useAppStore.setState({ selectedThreadId: "thread-file-entry" });
    const onToggle = vi.fn();
    const event: ToolCallEvent = {
      id: "call-file-entry", turnId: "turn-file-entry", createdAt: "2026-09-05T00:00:00Z",
      type: "tool_call", toolName: "edit", label: appEventText("tool.editFile"), status: "success",
      arguments: { filePath: "notes.txt", oldString: "before", newString: "after" },
    };
    render(<EventCard event={event} toolDisclosure={{ controlsId: "result-entry", expanded: false, onToggle }} />);
    fireEvent.click(screen.getByRole("button", { name: "This operation’s diff" }));
    expect(onToggle).not.toHaveBeenCalled();
    expect(useAppStore.getState().fileSelection).toEqual({ threadId: "thread-file-entry", path: "notes.txt", sourceToolCallItemId: event.id, toolCallItemId: event.id, view: "change" });
    fireEvent.click(screen.getByRole("button", { name: "Current file" }));
    expect(useAppStore.getState().fileSelection?.view).toBe("current");
    expect(screen.queryByText("before")).toBeNull();
  });

  it("uses the bound call ID when opening a file from its result", () => {
    useAppStore.setState({ selectedThreadId: "thread-file-entry" });
    const event: ToolResultEvent = {
      id: "result-file-entry", toolCallId: "call-original", turnId: "turn-file-entry", createdAt: "2026-09-05T00:00:00Z",
      type: "tool_result", toolName: "write", summary: appEventText("result.writeCompleted"), status: "success", output: "", path: "/workspace/notes.txt",
    };
    render(<EventCard event={event} />);
    fireEvent.click(screen.getByRole("button", { name: "This operation’s diff" }));
    expect(useAppStore.getState().fileSelection).toMatchObject({ toolCallItemId: "call-original", sourceToolCallItemId: "call-original" });
  });

  it("renders process command cards with localized copy and multiline invariant output", () => {
    const toolCall: ToolCallEvent = {
      id: "process-call",
      turnId: "process-turn",
      createdAt: "2026-08-12T00:00:00.000Z",
      type: "tool_call",
      toolName: "process_start",
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
      <Tooltip.Provider>
        <EventCard event={toolCall} />
        <EventCard event={result} />
      </Tooltip.Provider>,
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

  it("shows the raw process command and exposes its full value", async () => {
    const command =
      'Get-ChildItem -Path "C:\\Workspace\\Game\\internal" -Recurse | Select-Object FullName, Length, LastWriteTime';
    const toolCall: ToolCallEvent = {
      id: "process-call-long",
      turnId: "process-turn-long",
      createdAt: "2026-08-12T00:00:00.000Z",
      type: "tool_call",
      toolName: "process_start",
      label: appEventText("tool.runProcess"),
      status: "success",
      arguments: { command },
    };
    const onToggle = vi.fn();
    const { container } = render(
      <Tooltip.Provider delayDuration={0}>
        <EventCard
          event={toolCall}
          toolDisclosure={{
            controlsId: "tool-result-process-call-long",
            expanded: false,
            onToggle,
          }}
        />
      </Tooltip.Provider>,
    );

    const preview = container.querySelector('[data-command-preview="true"]');
    const disclosure = container.querySelector<HTMLButtonElement>(
      '[data-tool-call-id="process-call-long"]',
    );
    if (!preview || !disclosure) throw new Error("Expected process command controls");

    expect(preview).toHaveTextContent(command);
    expect(preview).not.toHaveTextContent("process_start");
    expect(preview).not.toHaveTextContent('{"command"');
    expect(disclosure).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelectorAll("button")).toHaveLength(1);

    fireEvent.focus(disclosure);
    await waitFor(() => {
      expect(screen.getByRole("tooltip")).toHaveTextContent(command);
    });
    expect(onToggle).not.toHaveBeenCalled();
  });

  it("renders compact localized file tool cards without exposing content or replacement text", () => {
    const writeContent = "SECRET-WRITE-CONTENT";
    const oldString = "SECRET-OLD-STRING";
    const newString = "SECRET-NEW-STRING";
    const readCall: ToolCallEvent = {
      id: "read-call",
      turnId: "file-turn",
      createdAt: "2026-08-12T00:00:00.000Z",
      type: "tool_call",
      toolName: "read",
      label: appEventText("tool.readFile"),
      status: "success",
      arguments: { filePath: "notes/readme.txt", offset: 5, limit: 20 },
    };
    const writeCall: ToolCallEvent = {
      ...readCall,
      id: "write-call",
      toolName: "write",
      label: appEventText("tool.writeFile"),
      arguments: { filePath: "notes/new.txt", content: writeContent },
    };
    const editCall: ToolCallEvent = {
      ...readCall,
      id: "edit-call",
      toolName: "edit",
      label: appEventText("tool.editFile"),
      arguments: {
        filePath: "notes/readme.txt",
        oldString,
        newString,
        replaceAll: true,
      },
    };
    const readResult: ToolResultEvent = {
      id: "read-result",
      turnId: "file-turn",
      createdAt: "2026-08-12T00:00:00.025Z",
      type: "tool_result",
      toolCallId: readCall.id,
      toolName: "read",
      status: "success",
      summary: appEventText("result.readCompleted"),
      output: "SECRET-READ-OUTPUT",
      path: "C:\\work\\notes\\readme.txt",
      details: { lineStart: 5, lineEnd: 20, totalLines: 42 },
    };
    const editResult: ToolResultEvent = {
      ...readResult,
      id: "edit-result",
      toolCallId: editCall.id,
      toolName: "edit",
      status: "error",
      summary: appEventText("result.editFailed"),
      output: "Could not find oldString in the file.",
      errorCode: "no_match",
      details: undefined,
    };
    const pagedReadResult: ToolResultEvent = {
      ...readResult,
      id: "paged-read-result",
      details: {
        lineStart: 21,
        lineEnd: 40,
        nextOffset: 41,
        bytesRead: 512,
        truncated: true,
      },
    };

    const { container } = render(
      <>
        <EventCard event={readCall} />
        <EventCard event={writeCall} />
        <EventCard event={editCall} />
        <EventCard event={readResult} />
        <EventCard event={pagedReadResult} />
        <EventCard event={editResult} />
      </>,
    );

    expect(screen.getByText("Reading file")).toBeInTheDocument();
    expect(screen.getByText("Writing file")).toBeInTheDocument();
    expect(screen.getByText("Editing file")).toBeInTheDocument();
    expect(container).toHaveTextContent("notes/readme.txt");
    expect(container).toHaveTextContent("offset 5 · limit 20");
    expect(container).toHaveTextContent("replace all");
    expect(container).toHaveTextContent("Lines 5–20 of 42");
    expect(container).toHaveTextContent("Lines 21–40");
    expect(container).toHaveTextContent("Error code: no_match");
    expect(container).toHaveTextContent("Could not find oldString in the file.");
    expect(container).not.toHaveTextContent(writeContent);
    expect(container).not.toHaveTextContent(oldString);
    expect(container).not.toHaveTextContent(newString);
    expect(container).not.toHaveTextContent(readResult.output);
    expect(container.querySelector(".lucide-file-text")).toBeInTheDocument();
    expect(container.querySelector(".lucide-file-output")).toBeInTheDocument();
    expect(container.querySelector(".lucide-file-pen-line")).toBeInTheDocument();

    act(() => setUiLanguage("zh-CN"));

    expect(screen.getByText("正在读取文件")).toBeInTheDocument();
    expect(screen.queryByText("文件编辑完成")).not.toBeInTheDocument();
    expect(screen.getByText("无法编辑文件")).toBeInTheDocument();
    expect(container).toHaveTextContent("起始行 5 · 最多 20 行");
    expect(container).toHaveTextContent("第 5–20 行，共 42 行");
    expect(container).toHaveTextContent("第 21–40 行");
    expect(container).toHaveTextContent("错误代码：no_match");
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

describe("stopped run status semantics", () => {
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

describe("managed process results", () => {
  it("shows running separately from completion and preserves output and actual exit status", () => {
    const event: ToolResultEvent = {
      id: "process-result", turnId: "turn-process", createdAt: "2026-09-07T00:00:00Z",
      type: "tool_result", toolCallId: "process-call", toolName: "process_start", status: "success",
      summary: appEventText("result.processRunning"), output: "中文 output\r\n",
      details: { processId: "process-1", processState: "running", exitCode: null },
    };
    const view = render(<Tooltip.Provider><EventCard event={event} /></Tooltip.Provider>);
    expect(screen.getByText("Command is still running")).toBeVisible();
    expect(screen.queryByText("Command completed")).toBeNull();
    expect(screen.getByText("中文 output")).toBeVisible();
    expect(screen.getByText("process-1")).toBeVisible();
    view.rerender(<Tooltip.Provider><EventCard event={{ ...event, toolName: "process_wait", status: "error", summary: appEventText("result.processFailed"), details: { processId: "process-1", processState: "exited", exitCode: 2 } }} /></Tooltip.Provider>);
    expect(screen.getByText("Command failed")).toBeVisible();
    expect(screen.getByText(/Exit code 2/)).toBeVisible();
    expect(screen.getByText("中文 output")).toBeVisible();
  });

  it("labels background commands while running and after they finish", () => {
    const event: ToolResultEvent = {
      id: "background-process",
      turnId: "turn-process",
      createdAt: "2026-10-21T00:00:00Z",
      type: "tool_result",
      toolCallId: "background-call",
      toolName: "process_start",
      status: "success",
      summary: appEventText("result.processRunning"),
      output: "",
      details: {
        processId: "process-background",
        background: true,
        processState: "running",
        exitCode: null,
      },
    };
    const view = render(
      <Tooltip.Provider>
        <EventCard event={event} />
      </Tooltip.Provider>,
    );

    expect(screen.getByText("Background running")).toBeVisible();
    view.rerender(
      <Tooltip.Provider>
        <EventCard
          event={{
            ...event,
            status: "success",
            summary: appEventText("result.processCompleted"),
            details: { ...event.details, processState: "exited", exitCode: 0 },
          }}
        />
      </Tooltip.Provider>,
    );
    expect(screen.getByText("Background finished")).toBeVisible();
  });
});


describe("session tool affordances", () => {
  it("opens the child conversation from a subagent call", () => {
    const openThreadPanel = vi.fn();
    useAppStore.setState({ openThreadPanel });
    const toolCall: ToolCallEvent = {
      id: "subagent-call",
      turnId: "subagent-turn",
      createdAt: "2026-08-12T00:00:00.000Z",
      type: "tool_call",
      toolName: "subagent",
      label: appEventText("tool.subagent"),
      status: "success",
      arguments: { prompt: "do the thing" },
    };

    render(
      <Tooltip.Provider>
        <EventCard event={toolCall} sessionThreadId="thread_child" />
      </Tooltip.Provider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Open conversation" }));
    expect(openThreadPanel).toHaveBeenCalledWith("thread_child");
  });

  it("opens the created conversation from a thread tool result", () => {
    const openThreadPanel = vi.fn();
    useAppStore.setState({ openThreadPanel });
    const result: ToolResultEvent = {
      id: "thread-result",
      turnId: "thread-turn",
      createdAt: "2026-08-12T00:00:00.000Z",
      type: "tool_result",
      toolCallId: "thread-call",
      toolName: "thread_create",
      status: "success",
      summary: externalEventText("created"),
      output: "{}",
      details: { threadId: "thread_created" },
    };

    render(
      <Tooltip.Provider>
        <EventCard event={result} />
      </Tooltip.Provider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Open conversation" }));
    expect(openThreadPanel).toHaveBeenCalledWith("thread_created");
  });
});