import * as Tooltip from "@radix-ui/react-tooltip";
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  CircleX,
  Clipboard,
  FileDiff,
  FileOutput,
  FilePenLine,
  FileText,
  LoaderCircle,
  Search,
  SquareTerminal,
  Wrench,
} from "lucide-react";
import { memo } from "react";
import type {
  AgentEvent,
  FileChangeEvent,
  StatusEvent,
  ToolCallEvent,
  ToolResultEvent,
} from "../domain";
import { resolveEventText } from "../eventCopy";
import { type Translate, useTranslation } from "../i18n";
import { useAppStore } from "../store";
import { ProcessSessionCard } from "./ProcessSessionCard";
import { cx, IconButton, Markdown } from "./ui";

function copyText(content: string) {
  void navigator.clipboard?.writeText(content);
}

const FILE_OPERATION_KEYS = {
  created: "events.file.created",
  modified: "events.file.modified",
  deleted: "events.file.deleted",
  renamed: "events.file.renamed",
} as const;

function isFileTool(toolName: string | undefined): toolName is "read" | "write" | "edit" {
  return toolName === "read" || toolName === "write" || toolName === "edit";
}

function isSearchTool(toolName: string | undefined): boolean {
  return toolName === "glob" || toolName === "grep" || toolName === "list";
}

function processCommand(event: ToolCallEvent): string | undefined {
  return event.toolName === "process_start" &&
    typeof event.arguments.command === "string"
    ? event.arguments.command
    : undefined;
}

function toolIcon(toolName: string) {
  if (toolName === "read") return <FileText size={14} />;
  if (toolName === "write") return <FileOutput size={14} />;
  if (toolName === "edit") return <FilePenLine size={14} />;
  if (isSearchTool(toolName)) return <Search size={14} />;
  if (toolName.startsWith("process_") || toolName.includes("extract")) {
    return <SquareTerminal size={14} />;
  }
  return <Wrench size={14} />;
}

function fileToolCallDetails(event: ToolCallEvent, t: Translate) {
  const path = typeof event.arguments.filePath === "string" ? event.arguments.filePath : "";
  const metadata: string[] = [];
  if (event.toolName === "read") {
    if (Number.isInteger(event.arguments.offset) && Number(event.arguments.offset) > 0) {
      metadata.push(t("events.tool.offset", { value: Number(event.arguments.offset) }));
    }
    if (Number.isInteger(event.arguments.limit) && Number(event.arguments.limit) > 0) {
      metadata.push(t("events.tool.limit", { value: Number(event.arguments.limit) }));
    }
  } else if (event.toolName === "edit" && event.arguments.replaceAll === true) {
    metadata.push(t("events.tool.replaceAll"));
  }
  return { path, metadata };
}

function searchToolCallDetails(event: ToolCallEvent) {
  const pattern = typeof event.arguments.pattern === "string" ? event.arguments.pattern : "";
  const metadata: string[] = [];
  if (typeof event.arguments.path === "string" && event.arguments.path) {
    metadata.push(event.arguments.path);
  }
  if (typeof event.arguments.include === "string" && event.arguments.include) {
    metadata.push(event.arguments.include);
  }
  return { path: pattern, metadata };
}

function searchToolResultDetails(event: ToolResultEvent, t: Translate): string[] {
  const details = event.details;
  if (!details) return [];
  const metadata: string[] = [];
  if (Number.isInteger(details.entryCount)) {
    metadata.push(t("events.result.entryCount", { count: details.entryCount as number }));
  }
  if (Number.isInteger(details.matchCount)) {
    metadata.push(t("events.result.matchCount", { count: details.matchCount as number }));
  }
  if (details.truncated === true) metadata.push(t("events.result.outputTruncated"));
  return metadata;
}

function fileToolResultDetails(event: ToolResultEvent, t: Translate): string[] {
  const details = event.details;
  if (!details) return [];
  const metadata: string[] = [];
  if (Number.isInteger(details.lineStart) && Number.isInteger(details.lineEnd)) {
    metadata.push(
      Number.isInteger(details.totalLines)
        ? t("events.result.readRange", {
            start: details.lineStart as number,
            end: details.lineEnd as number,
            total: details.totalLines as number,
          })
        : t("events.result.readPage", {
            start: details.lineStart as number,
            end: details.lineEnd as number,
          }),
    );
  } else if (Number.isInteger(details.bytesRead)) {
    metadata.push(t("events.result.bytesRead", { count: details.bytesRead as number }));
  }
  if (Number.isInteger(details.bytesWritten)) {
    metadata.push(t("events.result.bytesWritten", { count: details.bytesWritten as number }));
  }
  if (Number.isInteger(details.replacements)) {
    metadata.push(t("events.result.replacements", { count: details.replacements as number }));
  }
  if (details.truncated === true) metadata.push(t("events.result.outputTruncated"));
  return metadata;
}

interface ToolDisclosure {
  controlsId: string;
  expanded: boolean;
  onToggle: (toolCallId: string) => void;
}

function FileActions({
  path,
  toolName,
  toolCallItemId,
  running = false,
}: {
  path: string;
  toolName: string | undefined;
  toolCallItemId: string;
  running?: boolean;
}) {
  const { t } = useTranslation();
  const threadId = useAppStore((state) => state.selectedThreadId);
  const openFile = useAppStore((state) => state.openFile);
  if (!threadId || !path || !isFileTool(toolName)) return null;
  const canHaveChange = toolName === "write" || toolName === "edit";
  return (
    <div className="flex flex-wrap items-center gap-2 px-3.5 pb-2.5 pt-0.5">
      <button
        type="button"
        onClick={() => openFile({
          threadId, path, sourceToolCallItemId: toolCallItemId,
          ...(canHaveChange ? { toolCallItemId } : {}), view: "current",
        })}
        className="rounded-md px-2 py-1 text-[11px] font-medium text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
      >
        {t("files.current")}
      </button>
      {canHaveChange ? (
        <button
          type="button"
          disabled={running}
          onClick={() => openFile({
            threadId, path, sourceToolCallItemId: toolCallItemId, toolCallItemId, view: "change",
          })}
          className="rounded-md px-2 py-1 text-[11px] font-medium text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)] disabled:opacity-40"
        >
          {t("files.change")}
        </button>
      ) : null}
    </div>
  );
}

function ToolCallCard({
  event,
  disclosure,
}: {
  event: ToolCallEvent;
  disclosure?: ToolDisclosure;
}) {
  const { t } = useTranslation();
  const fileDetails = isFileTool(event.toolName) ? fileToolCallDetails(event, t) : undefined;
  const callTarget = fileDetails
    ?? (isSearchTool(event.toolName) ? searchToolCallDetails(event) : undefined);
  const command = processCommand(event);
  const status = {
    running: {
      icon: <LoaderCircle size={15} className="animate-spin" />,
      label: t("events.running"),
      className: "text-[var(--accent)]",
    },
    success: {
      icon: <CheckCircle2 size={15} />,
      label: t(event.toolName === "process_start" ? "events.processStarted" : "events.complete"),
      className: "text-[#72d3a7]",
    },
    error: {
      icon: <CircleX size={15} />,
      label: t("events.failed"),
      className: "text-[#ff7f7f]",
    },
    interrupted: {
      icon: <AlertTriangle size={15} />,
      label: t("events.interrupted"),
      className: "text-[#d8a36f]",
    },
  }[event.status];

  const content = (
    <>
      <span className={cx("mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg bg-[var(--panel-hover)]", status.className)}>
        {toolIcon(event.toolName)}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className="truncate text-[12px] font-medium leading-[18px] text-[var(--text)]">
            {resolveEventText(event.label, t)}
          </span>
          <span className={cx("ml-auto flex shrink-0 items-center gap-1 text-[11px] leading-4", status.className)}>
            {status.icon}
            {status.label}
          </span>
        </span>
        {callTarget ? (
          <span className="mt-1 flex min-w-0 items-center gap-1.5 font-mono text-[11px] leading-[16px] text-[var(--muted)]">
            <span className="truncate" title={callTarget.path || undefined}>
              {callTarget.path || event.toolName}
            </span>
            {callTarget.metadata.length > 0 ? (
              <span className="shrink-0">· {callTarget.metadata.join(" · ")}</span>
            ) : null}
          </span>
        ) : (
          <span
            data-command-preview={command ? "true" : undefined}
            className="mt-1 block truncate font-mono text-[11px] leading-[16px] text-[var(--muted)]"
          >
            {command ?? `${event.toolName} · ${JSON.stringify(event.arguments)}`}
          </span>
        )}
      </span>
    </>
  );

  const contentClassName =
    "flex w-full items-start gap-3 rounded-xl px-3.5 py-3 text-left";
  const mainContent = disclosure ? (
    <button
      type="button"
      aria-expanded={disclosure.expanded}
      aria-controls={disclosure.controlsId}
      data-tool-call-id={event.id}
      onClick={() => disclosure.onToggle(event.id)}
      className={cx(contentClassName, "cursor-pointer")}
    >
      {content}
    </button>
  ) : (
    <div className={contentClassName} tabIndex={command ? 0 : undefined}>
      {content}
    </div>
  );

  const commandTrigger = command ? (
    <Tooltip.Root delayDuration={320}>
      <Tooltip.Trigger asChild>{mainContent}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          side="bottom"
          align="start"
          sideOffset={7}
          collisionPadding={12}
          className="app-scrollbar z-[100] max-h-56 max-w-[min(680px,calc(100vw-32px))] overflow-auto whitespace-pre-wrap break-all rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 py-2 font-mono text-[11px] leading-[17px] text-[var(--text)] shadow-xl"
        >
          {command}
          <Tooltip.Arrow className="fill-[var(--border)]" />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  ) : (
    mainContent
  );

  return (
    <div className="event-card-shadow w-full rounded-xl border border-[var(--border-soft)] bg-[var(--panel)]">
      {commandTrigger}
      {fileDetails ? (
        <FileActions path={fileDetails.path} toolName={event.toolName} toolCallItemId={event.id} running={event.status === "running"} />
      ) : null}
    </div>
  );
}

function ToolResultCard({ event }: { event: ToolResultEvent }) {
  const { t } = useTranslation();
  const isFileResult = isFileTool(event.toolName);
  const isSearchResult = isSearchTool(event.toolName);
  const metadata = isFileResult
    ? fileToolResultDetails(event, t)
    : isSearchResult
      ? searchToolResultDetails(event, t)
      : [];
  if (event.toolName?.startsWith("process_") && event.details) {
    if (typeof event.details.exitCode === "number") metadata.push(t("events.result.exitCode", { code: event.details.exitCode }));
    if (event.details.processId) metadata.push(event.details.processId);
    if (event.details.truncated) metadata.push(t("events.result.outputTruncated"));
  }
  const presentation = {
    success: {
      icon: <Check size={13} className="mt-0.5 shrink-0 text-[#72d3a7]" />,
      className: "border-[#396b58]/40 bg-[#24463a]/15",
    },
    error: {
      icon: <AlertTriangle size={13} className="mt-0.5 shrink-0 text-[#ff8585]" />,
      className: "border-[#8b4444]/45 bg-[#4b2626]/18",
    },
    interrupted: {
      icon: <AlertTriangle size={13} className="mt-0.5 shrink-0 text-[#d8a36f]" />,
      className: "border-[#8b6a44]/45 bg-[#4b3b26]/18",
    },
  }[event.status];
  if (event.details?.processState === "running") {
    presentation.icon = <LoaderCircle size={13} className="mt-0.5 shrink-0 animate-spin text-[var(--muted)]" />;
    presentation.className = "border-[var(--border)] bg-[var(--panel)]";
  }
  return (
    <div
      className={cx(
        "ml-10 flex w-[calc(100%-2.5rem)] items-start gap-2.5 rounded-lg border px-3 py-2.5 text-left",
        presentation.className,
      )}
    >
      {presentation.icon}
      <div className="min-w-0 flex-1">
        <div className="text-[11px] leading-[16px] text-[var(--text)]">
          {resolveEventText(event.summary, t)}
        </div>
        {isFileResult && event.path ? (
          <div className="mt-1 truncate font-mono text-[11px] leading-4 text-[var(--muted)]" title={event.path}>
            {event.path}
          </div>
        ) : null}
        {metadata.length > 0 ? (
          <div className="mt-1 text-[11px] leading-4 text-[var(--muted)]">
            {metadata.join(" · ")}
          </div>
        ) : null}
        {isFileResult && event.errorCode ? (
          <div className="mt-1 font-mono text-[11px] leading-4 text-[var(--muted-strong)]">
            {t("events.result.errorCode", { code: event.errorCode })}
          </div>
        ) : null}
        {event.output && (!isFileResult || event.status !== "success") ? (
          <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-4 text-[var(--muted)]">
            {event.output}
          </pre>
        ) : null}
        {isFileResult && event.path ? (
          <FileActions path={event.path} toolName={event.toolName} toolCallItemId={event.toolCallId} />
        ) : null}
      </div>
    </div>
  );
}

function FileChangeCard({ event }: { event: FileChangeEvent }) {
  const { t } = useTranslation();
  return (
    <div className="flex w-full items-center gap-3 rounded-xl border border-[var(--border-soft)] bg-[var(--panel)] px-3.5 py-3 text-left">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-[#72d3a7]/9 text-[#74d5aa]">
        <FileDiff size={15} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-mono text-[11px] leading-[16px] text-[var(--text)]">{event.path}</span>
        <span className="mt-0.5 block text-[11px] leading-4 text-[var(--muted)]">
          {t(FILE_OPERATION_KEYS[event.operation])}
        </span>
      </span>
      <span className="flex shrink-0 items-center gap-2 font-mono text-[11px] leading-4">
        <span className="text-[#72d3a7]">+{event.additions}</span>
        <span className="text-[#ff8585]">−{event.deletions}</span>
      </span>
    </div>
  );
}

function StatusRow({ event }: { event: StatusEvent }) {
  const { t } = useTranslation();
  const runStatus = useAppStore((state) => state.runStatus);
  const active = event.tone === "neutral" && (runStatus === "queued" || runStatus === "running");
  const colors = {
    neutral: "text-[var(--accent)]",
    success: "text-[#72d3a7]",
    warning: "text-[#ff9b68]",
    danger: "text-[#ff7f7f]",
  }[event.tone];
  const icon = active ? (
    <LoaderCircle size={11} className="animate-spin" />
  ) : event.tone === "warning" ? (
    <AlertTriangle size={11} />
  ) : event.tone === "danger" ? (
    <CircleX size={11} />
  ) : (
    <Check size={11} />
  );
  const iconLabel = active
    ? t("events.running")
    : event.tone === "warning"
      ? t("events.warning")
      : event.tone === "danger"
        ? t("events.error")
        : t("events.complete");
  return (
    <div data-tone={event.tone} className="flex items-center gap-2.5 px-1 py-1.5 text-[11px] leading-[16px] text-[var(--muted)]">
      <span
        role="img"
        aria-label={iconLabel}
        className={cx("flex size-5 items-center justify-center rounded-full bg-[var(--panel-hover)]", colors)}
      >
        {icon}
      </span>
      <span className="text-[var(--muted-strong)]">{resolveEventText(event.label, t)}</span>
      {event.detail ? (
        <span className="truncate text-[var(--muted)]">
          · {resolveEventText(event.detail, t)}
        </span>
      ) : null}
    </div>
  );
}

function UserMessageCard({ event }: { event: Extract<AgentEvent, { type: "message" }> }) {
  const { t } = useTranslation();

  return (
    <div className="group flex justify-end gap-1.5">
      <div className="flex items-start opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
        <IconButton label={t("events.copyMessage")} onClick={() => copyText(event.content)}>
          <Clipboard size={12} />
        </IconButton>
      </div>
      <div className="max-w-[78%] whitespace-pre-wrap rounded-[18px] rounded-br-md bg-[var(--panel-hover)] px-4 py-2.5 text-[14px] leading-[22px] text-[var(--text)]">
        {event.content}
      </div>
    </div>
  );
}

function AssistantMessageCard({ event }: { event: Extract<AgentEvent, { type: "message" }> }) {
  const { t } = useTranslation();

  return (
    <div className="group min-w-0">
      <Markdown content={event.content} streaming={event.status === "streaming"} />
      {event.status === "failed" ? (
        <div className="mt-1.5 flex items-center gap-1.5 text-[11px] leading-4 text-[#ff7f7f]">
          <CircleX size={11} />
          {t("events.failed")}
        </div>
      ) : null}
      {event.status === "interrupted" ? (
        <div className="mt-1.5 flex items-center gap-1.5 text-[11px] leading-4 text-[var(--text-muted)]">
          <AlertTriangle size={11} />
          {t("events.interrupted")}
        </div>
      ) : null}
      {event.status !== "streaming" ? (
        <div className="mt-1.5 flex h-6 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          <IconButton label={t("events.copyResponse")} className="size-6" onClick={() => copyText(event.content)}>
            <Clipboard size={12} />
          </IconButton>
        </div>
      ) : null}
    </div>
  );
}

interface EventCardProps {
  event: AgentEvent;
  toolDisclosure?: ToolDisclosure;
}

function EventCardView({ event, toolDisclosure }: EventCardProps) {
  if (event.type === "tool_call") {
    return <ToolCallCard event={event} disclosure={toolDisclosure} />;
  }
  if (event.type === "tool_result") {
    return event.toolName?.startsWith("process_") && event.details?.processId
      ? <ProcessSessionCard event={event} />
      : <ToolResultCard event={event} />;
  }
  if (event.type === "file_change") return <FileChangeCard event={event} />;
  if (event.type === "status") return <StatusRow event={event} />;

  if (event.role === "user") {
    return <UserMessageCard event={event} />;
  }

  return <AssistantMessageCard event={event} />;
}

export const EventCard = memo(
  EventCardView,
  (previous, next) =>
    previous.event === next.event &&
    previous.toolDisclosure?.controlsId === next.toolDisclosure?.controlsId &&
    previous.toolDisclosure?.expanded === next.toolDisclosure?.expanded &&
    previous.toolDisclosure?.onToggle === next.toolDisclosure?.onToggle,
);
EventCard.displayName = "EventCard";
