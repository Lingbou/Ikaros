import {
  AlertTriangle,
  Check,
  CheckCircle2,
  CircleX,
  Clipboard,
  FileDiff,
  FileText,
  GitBranch,
  LoaderCircle,
  Pencil,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  SquareTerminal,
  Wrench,
  X,
} from "lucide-react";
import { memo } from "react";
import type {
  AgentEvent,
  ArtifactEvent,
  BranchEvent,
  FileChangeEvent,
  InterruptEvent,
  PermissionEvent,
  StatusEvent,
  ToolCallEvent,
  ToolResultEvent,
} from "../domain";
import { isRunActive } from "../domain";
import { resolveEventText, resolveInterruptCopy } from "../eventCopy";
import { useTranslation } from "../i18n";
import { useAppStore } from "../store";
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

function ToolCallCard({ event }: { event: ToolCallEvent }) {
  const { t } = useTranslation();
  const status = {
    running: {
      icon: <LoaderCircle size={15} className="animate-spin" />,
      label: t("events.running"),
      className: "text-[var(--accent)]",
    },
    success: {
      icon: <CheckCircle2 size={15} />,
      label: t("events.complete"),
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

  return (
    <div className="event-card-shadow flex w-full items-start gap-3 rounded-xl border border-[var(--border-soft)] bg-[var(--panel)] px-3.5 py-3 text-left">
      <span className={cx("mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg bg-[var(--panel-hover)]", status.className)}>
        {event.toolName === "process.run" || event.toolName.includes("extract") ? (
          <SquareTerminal size={14} />
        ) : (
          <Wrench size={14} />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className="truncate text-[12px] font-medium leading-[18px] text-[var(--text)]">
            {resolveEventText(event.label, t)}
          </span>
          <span className={cx("ml-auto flex shrink-0 items-center gap-1 text-[10px] leading-[14px]", status.className)}>
            {status.icon}
            {status.label}
          </span>
        </span>
        <span className="mt-1 block truncate font-mono text-[11px] leading-[16px] text-[var(--muted)]">
          {event.toolName} · {JSON.stringify(event.arguments)}
        </span>
      </span>
    </div>
  );
}

function ToolResultCard({ event }: { event: ToolResultEvent }) {
  const { t } = useTranslation();
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
        {event.output ? (
          <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[10px] leading-[14px] text-[var(--muted)]">
            {event.output}
          </pre>
        ) : null}
      </div>
    </div>
  );
}

function PermissionCard({ event }: { event: PermissionEvent }) {
  const { t } = useTranslation();
  const resolvePermission = useAppStore((state) => state.resolvePermission);
  const runStatus = useAppStore((state) => state.runStatus);
  const unresolved = event.status === "pending";
  const actionable = unresolved && runStatus === "waiting_permission";
  return (
    <div
      role={unresolved ? "alert" : undefined}
      aria-live={unresolved ? "assertive" : undefined}
      aria-atomic={unresolved ? "true" : undefined}
      className="event-card-shadow overflow-hidden rounded-xl border border-[#ff8549]/35 bg-[#3b291f]/45"
    >
      <div className="flex gap-3 px-4 py-3.5">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-xl bg-[#ff8549]/12 text-[#ff9b68]">
          <ShieldAlert size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="text-[12px] font-medium leading-[18px] text-[var(--text)]">
              {resolveEventText(event.title, t)}
            </h3>
            {!unresolved ? (
              <span
                className={cx(
                  "ml-auto rounded-full px-2 py-0.5 text-[10px] font-medium leading-[14px]",
                  event.status === "allowed"
                    ? "bg-[#72d3a7]/10 text-[#82dcb5]"
                    : "bg-[var(--surface-hover)] text-[var(--muted-strong)]",
                )}
              >
                {event.status === "allowed" ? t("events.allowed") : t("events.denied")}
              </span>
            ) : null}
          </div>
          <p className="mt-1 text-[11px] leading-[16px] text-[var(--muted-strong)]">
            {resolveEventText(event.description, t)}
          </p>
          <div className="mt-2.5 overflow-hidden rounded-lg border border-[#ff8549]/15 bg-[var(--panel)] px-2.5 py-2 font-mono text-[11px] leading-[16px] text-[var(--muted-strong)]">
            <div className="truncate">{event.resource}</div>
          </div>
        </div>
      </div>
      {actionable ? (
        <div className="flex items-center justify-end gap-2 border-t border-[#ff8549]/16 bg-[color-mix(in_srgb,var(--panel)_55%,transparent)] px-3 py-2.5">
          <button
            type="button"
            onClick={() => void resolvePermission("deny")}
            className="flex h-8 items-center gap-1.5 rounded-lg px-3 text-[12px] leading-[18px] text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
          >
            <X size={12} />
            {t("events.deny")}
          </button>
          <button
            type="button"
            onClick={() => void resolvePermission("allow")}
            className="flex h-8 items-center gap-1.5 rounded-lg bg-[var(--text)] px-3 text-[12px] font-medium leading-[18px] text-[var(--canvas)] hover:opacity-90"
          >
            <Check size={12} />
            {t("events.allowOnce")}
          </button>
        </div>
      ) : null}
    </div>
  );
}

function InterruptCard({ event }: { event: InterruptEvent }) {
  const { t } = useTranslation();
  const recoverRun = useAppStore((state) => state.recoverRun);
  const runStatus = useAppStore((state) => state.runStatus);
  const interrupted = event.status === "interrupted" && runStatus === "interrupted";
  const copy = resolveInterruptCopy(event.copy, t);
  const visual = {
    interrupted: {
      card: "border-[#ff7474]/27 bg-[#472629]/28",
      icon: "bg-[#ff7474]/10 text-[#ff8585]",
      iconLabel: t("events.interrupted"),
      glyph: <AlertTriangle size={15} />,
    },
    recovering: {
      card: "border-[color-mix(in_srgb,var(--accent)_30%,transparent)] bg-[color-mix(in_srgb,var(--accent)_12%,transparent)]",
      icon: "bg-[color-mix(in_srgb,var(--accent)_12%,transparent)] text-[var(--accent)]",
      iconLabel: t("events.recovering"),
      glyph: <RefreshCw size={15} className="animate-spin" />,
    },
    recovered: {
      card: "border-[#72d3a7]/30 bg-[#24463a]/22",
      icon: "bg-[#72d3a7]/12 text-[#72d3a7]",
      iconLabel: t("events.recovered"),
      glyph: <CheckCircle2 size={15} />,
    },
  }[event.status];
  return (
    <div
      data-status={event.status}
      className={cx("event-card-shadow rounded-xl border px-4 py-3.5", visual.card)}
    >
      <div className="flex items-start gap-3">
        <span
          role="img"
          aria-label={visual.iconLabel}
          className={cx("flex size-8 shrink-0 items-center justify-center rounded-xl", visual.icon)}
        >
          {visual.glyph}
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="text-[12px] font-medium leading-[18px] text-[var(--text)]">{copy.title}</h3>
          <p className="mt-1 text-[11px] leading-[16px] text-[var(--muted)]">{copy.description}</p>
          {interrupted && event.recoverable ? (
            <div className="mt-3 flex flex-wrap gap-2">
              <button
                type="button"
                onClick={() => void recoverRun("retry")}
                className="flex h-8 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--panel)] px-3 text-[12px] leading-[18px] text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
              >
                <RotateCcw size={12} />
                {t("events.retryStep")}
              </button>
              <button
                type="button"
                onClick={() => void recoverRun("resume")}
                className="flex h-8 items-center gap-1.5 rounded-lg bg-[var(--text)] px-3 text-[12px] font-medium leading-[18px] text-[var(--canvas)] hover:opacity-90"
              >
                <Play size={12} fill="currentColor" />
                {t("events.recoverSession")}
              </button>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function ArtifactCard({ event }: { event: ArtifactEvent }) {
  const { t } = useTranslation();
  return (
    <div className="event-card-shadow flex w-full items-center gap-3 rounded-xl border border-[color-mix(in_srgb,var(--accent)_24%,transparent)] bg-[color-mix(in_srgb,var(--accent)_10%,transparent)] px-3.5 py-3 text-left">
      <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-[color-mix(in_srgb,var(--accent)_12%,transparent)] text-[var(--accent)]">
        <FileText size={17} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[12px] font-medium leading-[18px] text-[var(--text)]">{event.title}</span>
        <span className="mt-0.5 block text-[11px] leading-[16px] text-[var(--muted)]">
          {t("events.artifactMetadata", { version: event.version })}
        </span>
      </span>
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
        <span className="mt-0.5 block text-[10px] leading-[14px] text-[var(--muted)]">
          {t(FILE_OPERATION_KEYS[event.operation])}
        </span>
      </span>
      <span className="flex shrink-0 items-center gap-2 font-mono text-[10px] leading-[14px]">
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

function BranchRow({ event }: { event: BranchEvent }) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-2 px-2 py-1.5 text-[11px] leading-[16px] text-[var(--accent)]">
      <GitBranch size={12} />
      {event.copy.source === "app" ? t("events.branchCreatedFromEdit") : event.copy.value}
    </div>
  );
}

function UserMessageCard({ event }: { event: Extract<AgentEvent, { type: "message" }> }) {
  const { t } = useTranslation();
  const beginEditMessage = useAppStore((state) => state.beginEditMessage);
  const runStatus = useAppStore((state) => state.runStatus);
  const runtimeMode = useAppStore((state) => state.runtimeMode);

  return (
    <div className="group flex justify-end gap-1.5">
      <div className="flex items-start opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
        <IconButton label={t("events.copyMessage")} onClick={() => copyText(event.content)}>
          <Clipboard size={12} />
        </IconButton>
        {!runtimeMode && !isRunActive(runStatus) ? (
          <IconButton label={t("events.editAndBranch")} onClick={() => beginEditMessage(event.id, event.content)}>
            <Pencil size={12} />
          </IconButton>
        ) : null}
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
          <IconButton
            label={t("events.regenerateUnavailable")}
            className="size-6"
            disabled
          >
            <RefreshCw size={12} />
          </IconButton>
        </div>
      ) : null}
    </div>
  );
}

function EventCardView({ event }: { event: AgentEvent }) {
  if (event.type === "tool_call") return <ToolCallCard event={event} />;
  if (event.type === "tool_result") return <ToolResultCard event={event} />;
  if (event.type === "permission_request") return <PermissionCard event={event} />;
  if (event.type === "interrupt") return <InterruptCard event={event} />;
  if (event.type === "artifact") return <ArtifactCard event={event} />;
  if (event.type === "file_change") return <FileChangeCard event={event} />;
  if (event.type === "status") return <StatusRow event={event} />;
  if (event.type === "branch_created") return <BranchRow event={event} />;

  if (event.role === "user") {
    return <UserMessageCard event={event} />;
  }

  return <AssistantMessageCard event={event} />;
}

export const EventCard = memo(
  EventCardView,
  (previous, next) => previous.event === next.event,
);
EventCard.displayName = "EventCard";
