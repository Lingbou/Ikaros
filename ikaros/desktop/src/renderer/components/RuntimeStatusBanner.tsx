import { AlertCircle, LoaderCircle, RotateCcw, WifiOff, X } from "lucide-react";

import { type TranslationKey, useTranslation } from "../i18n";
import { type RuntimeIssue, useAppStore } from "../store";
import { IconButton } from "./ui";

const ISSUE_TITLE_KEYS: Record<RuntimeIssue["kind"], TranslationKey> = {
  connection: "runtime.offline",
  initialization: "runtime.error.initialization",
  send: "runtime.error.send",
  cancel: "runtime.error.cancel",
  history: "runtime.error.history",
  archive: "runtime.error.archive",
  unarchive: "runtime.error.unarchive",
  rename: "runtime.error.rename",
  archived_catalog: "runtime.error.archivedCatalog",
};

export function RuntimeStatusBanner() {
  const { t } = useTranslation();
  const runtimeMode = useAppStore((state) => state.runtimeMode);
  const status = useAppStore((state) => state.runtimeConnectionStatus);
  const error = useAppStore((state) => state.runtimeError);
  const issue = useAppStore((state) => state.runtimeIssue);
  const selectedThreadId = useAppStore((state) => state.selectedThreadId);
  const draft = useAppStore((state) => state.draft);
  const activeRunId = useAppStore((state) => state.activeRun?.runId ?? null);
  const retryConnection = useAppStore((state) => state.retryRuntimeConnection);
  const retryIssue = useAppStore((state) => state.retryRuntimeIssue);
  const clearError = useAppStore((state) => state.clearRuntimeError);

  if (!runtimeMode) return null;

  if (status === "starting" || status === "reconnecting") {
    return (
      <div
        role="status"
        className="flex min-h-9 shrink-0 select-none items-center justify-center gap-2 border-b border-[var(--border-soft)] bg-[var(--panel)] px-4 text-[11px] leading-4 text-[var(--muted-strong)]"
      >
        <LoaderCircle size={12} aria-hidden="true" className="animate-spin text-[var(--accent)]" />
        {t(status === "starting" ? "runtime.starting" : "runtime.reconnecting")}
      </div>
    );
  }

  const matchingIssue = issue?.message === error ? issue : null;
  if (
    status === "connected" &&
    (matchingIssue?.kind === "history" || matchingIssue?.kind === "archived_catalog")
  ) {
    return null;
  }
  if (status === "connected" && !error) return null;

  const offline = status === "offline";
  const title = offline
    ? t("runtime.offline")
    : t(matchingIssue ? ISSUE_TITLE_KEYS[matchingIssue.kind] : "runtime.error.generic");
  const issueStillRetryable =
    matchingIssue !== null &&
    (matchingIssue.kind === "send"
      ? matchingIssue.threadId === selectedThreadId && matchingIssue.prompt === draft
      : matchingIssue.kind === "cancel"
        ? matchingIssue.runId === activeRunId
        : true);
  const retry = offline ? retryConnection : issueStillRetryable ? retryIssue : null;

  return (
    <div
      role="alert"
      className="flex min-h-10 shrink-0 select-none items-center gap-2.5 border-b border-[#e07070]/20 bg-[#e07070]/[0.07] px-3.5 text-[11px] leading-4 sm:px-5"
    >
      <span className="flex size-5 shrink-0 items-center justify-center text-[#e07070]">
        {offline ? <WifiOff size={13} aria-hidden="true" /> : <AlertCircle size={13} aria-hidden="true" />}
      </span>
      <span className="shrink-0 font-medium text-[var(--text)]">{title}</span>
      {error ? (
        <span title={error} className="min-w-0 flex-1 truncate text-[var(--muted)]">
          {error}
        </span>
      ) : (
        <span className="min-w-0 flex-1" />
      )}
      {retry ? (
        <button
          type="button"
          onClick={() => void retry()}
          className="flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2 text-[11px] font-medium text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
        >
          <RotateCcw size={11} aria-hidden="true" />
          {t("common.retry")}
        </button>
      ) : null}
      {!offline ? (
        <IconButton
          label={t("common.dismiss")}
          className="size-7 rounded-lg"
          onClick={clearError}
        >
          <X size={12} aria-hidden="true" />
        </IconButton>
      ) : null}
    </div>
  );
}
