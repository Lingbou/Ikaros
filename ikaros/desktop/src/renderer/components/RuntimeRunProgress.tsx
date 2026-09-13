import { useEffect, useState } from "react";
import type { Turn } from "../domain";
import { type TranslationKey, useTranslation } from "../i18n";

const STATUS_KEYS: Record<string, TranslationKey> = {
  queued: "runtime.progress.queued",
  running: "runtime.progress.running",
  completed: "runtime.progress.completed",
  failed: "runtime.progress.failed",
  interrupted: "runtime.progress.interrupted",
};

function duration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export function RuntimeRunProgress({ turn }: { turn: Turn }) {
  const { t } = useTranslation();
  const progress = turn.runProgress;
  const [now, setNow] = useState(Date.now);
  const active = turn.status === "queued" || turn.status === "running";
  useEffect(() => {
    if (!active || !progress) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active, Boolean(progress)]);
  if (!progress) return null;
  const finish = progress.settledAt ? Date.parse(progress.settledAt) : now;
  const start = progress.startedAt ? Date.parse(progress.startedAt) : null;
  const queuedFor = (finish - Date.parse(progress.queuedAt)) / 1000;
  const elapsed = start === null ? 0 : (finish - start) / 1000;
  return (
    <div aria-label={t("runtime.progress.label")} className="my-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] leading-4 text-[var(--muted)]">
      <span className="font-medium text-[var(--muted-strong)]">{t(STATUS_KEYS[turn.status] ?? "runtime.progress.running")}</span>
      <span>{t("runtime.progress.calls", { used: progress.modelCalls })}</span>
      {progress.compacting ? <span>{t("runtime.progress.compacting")}</span> : null}
      {!progress.compacting && progress.compactions ? (
        <span>{t("runtime.progress.compactions", { used: progress.compactions })}</span>
      ) : null}
      <span>{start === null
        ? t("runtime.progress.waiting", { time: duration(queuedFor) })
        : t("runtime.progress.time", { elapsed: duration(elapsed) })}</span>
    </div>
  );
}
