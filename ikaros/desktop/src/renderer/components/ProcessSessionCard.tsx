import { AlertTriangle, Check, LoaderCircle, Square } from "lucide-react";
import { useEffect, useState } from "react";
import type { ToolResultEvent } from "../domain";
import { resolveEventText } from "../eventCopy";
import { useTranslation } from "../i18n";
import { createRuntimeClient } from "../runtimeClient";
import { useAppStore } from "../store";
import { cx } from "./ui";

export function ProcessSessionCard({ event }: { event: ToolResultEvent }) {
  const { t } = useTranslation();
  const threadId = useAppStore((state) => state.selectedThreadId);
  const [output, setOutput] = useState(event.output);
  const [cursor, setCursor] = useState(event.details?.nextCursor ?? 0);
  const [hasMore, setHasMore] = useState(event.details?.hasMore ?? false);
  const [state, setState] = useState(event.details?.processState);
  const [exitCode, setExitCode] = useState(event.details?.exitCode);
  const [truncated, setTruncated] = useState(event.details?.truncated ?? false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const processId = event.details?.processId;
  const running = state === "running";
  const failed = state === "exited" && exitCode !== 0;
  const uncertain = state === "terminated" || state === "unknown";
  const summary = running ? t("events.result.processRunning")
    : state === "terminated" ? t("events.result.processStopped")
    : state === "unknown" ? t("events.result.processUnknown")
    : state === "exited" ? t(failed ? "events.result.processFailed" : "events.result.processCompleted")
    : resolveEventText(event.summary, t);
  useEffect(() => {
    setOutput(event.output);
    setCursor(event.details?.nextCursor ?? 0);
    setHasMore(event.details?.hasMore ?? false);
    setState(event.details?.processState);
    setExitCode(event.details?.exitCode);
    setTruncated(event.details?.truncated ?? false);
    setError(null);
  }, [event]);

  const readMore = () => {
    if (!threadId || !processId) return;
    const runtime = createRuntimeClient();
    if (!runtime) return;
    setBusy(true);
    setError(null);
    void runtime.readProcess({ threadId, processId, cursor }).then((page) => {
      setOutput((current) => current + page.output);
      setCursor(page.nextCursor);
      setHasMore(page.hasMore);
      setState(page.state);
      setExitCode(page.exitCode);
      setTruncated(page.truncated);
    }).catch((failure: unknown) => {
      setError(failure instanceof Error ? failure.message : t("events.result.processControlFailed"));
    }).finally(() => setBusy(false));
  };

  const stop = () => {
    if (!threadId || !processId) return;
    const runtime = createRuntimeClient();
    if (!runtime) return;
    setBusy(true);
    setError(null);
    void runtime.stopProcess({ threadId, processId }).then((page) => {
      setOutput(page.output);
      setCursor(page.nextCursor);
      setHasMore(page.hasMore);
      setState(page.state);
      setExitCode(page.exitCode);
      setTruncated(page.truncated);
    }).catch((failure: unknown) => {
      setError(failure instanceof Error ? failure.message : t("events.result.processControlFailed"));
    }).finally(() => setBusy(false));
  };

  return (
    <div className={cx("ml-10 w-[calc(100%-2.5rem)] rounded-lg border px-3 py-2.5", running ? "border-[var(--border)] bg-[var(--panel)]" : failed ? "border-[#8b4444]/45 bg-[#4b2626]/18" : uncertain ? "border-[#8b6a44]/45 bg-[#4b3b26]/18" : "border-[#396b58]/40 bg-[#24463a]/15")}>
      <div className="flex items-center gap-2 text-[11px] text-[var(--text)]">
        {running ? <LoaderCircle size={13} className="animate-spin text-[var(--muted)]" /> : failed || uncertain ? <AlertTriangle size={13} className={failed ? "text-[#ff8585]" : "text-[#d8a36f]"} /> : <Check size={13} className="text-[#72d3a7]" />}
        <span>{summary}</span>
        <code className="ml-auto text-[10px] text-[var(--muted)]">{processId}</code>
      </div>
      {typeof exitCode === "number" ? <div className="mt-1 text-[11px] text-[var(--muted)]">{t("events.result.exitCode", { code: exitCode })}</div> : null}
      {event.toolName === "process_wait" && running ? <p className="mt-1 text-[11px] text-[var(--muted)]">{t("events.result.waitFinished")}</p> : null}
      {truncated ? <p className="mt-1 text-[11px] text-[var(--muted)]">{t("events.result.outputTruncated")}</p> : null}
      {output ? <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-4 text-[var(--muted)]">{output}</pre> : null}
      {threadId && processId ? <div className="mt-2 flex gap-2">
        {hasMore || running ? <button type="button" disabled={busy} onClick={readMore} className="rounded-md border border-[var(--border)] px-2 py-1 text-[11px] text-[var(--muted-strong)] disabled:opacity-50">{t(hasMore ? "events.result.readMore" : "events.result.refreshProcess")}</button> : null}
        {running ? <button type="button" disabled={busy} onClick={stop} className="flex items-center gap-1 rounded-md border border-[#8b4444]/50 px-2 py-1 text-[11px] text-[#ff9b9b] disabled:opacity-50"><Square size={10} />{t("events.result.stopProcess")}</button> : null}
      </div> : null}
      {error ? <p role="alert" className="mt-2 break-words text-[11px] text-[#ff8585]">{error}</p> : null}
    </div>
  );
}
