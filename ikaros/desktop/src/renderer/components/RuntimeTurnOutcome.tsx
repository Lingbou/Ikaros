import { AlertCircle, CircleStop } from "lucide-react";

import type { ToolResultEvent, Turn } from "../domain";
import { type TranslationKey, useTranslation } from "../i18n";

const MUTATING_TOOLS = new Set(["write", "edit", "process_start"]);
const REASON_KEYS: Readonly<Record<string, TranslationKey>> = {
  runtime_interrupted: "runtime.outcome.reason.runtimeInterrupted",
  cancelled: "runtime.outcome.reason.cancelled",
  context_budget_exceeded: "runtime.outcome.reason.contextBudgetExceeded",
  model_input_unavailable: "runtime.outcome.reason.modelInputUnavailable",
  provider_authentication: "runtime.outcome.reason.providerAuthentication",
  provider_rate_limit: "runtime.outcome.reason.providerRateLimit",
  provider_context_overflow: "runtime.outcome.reason.contextBudgetExceeded",
  provider_invalid_request: "runtime.outcome.reason.providerInvalidRequest",
  provider_timeout: "runtime.outcome.reason.providerTimeout",
  provider_network: "runtime.outcome.reason.providerConnection",
  provider_connection: "runtime.outcome.reason.providerConnection",
  provider_protocol: "runtime.outcome.reason.providerProtocol",
  provider_unavailable: "runtime.outcome.reason.providerUnavailable",
  provider_server: "runtime.outcome.reason.providerUnavailable",
  provider_unknown: "runtime.outcome.reason.providerUnavailable",
  provider_cancelled: "runtime.outcome.reason.cancelled",
  agent_error: "runtime.outcome.reason.agentError",
  model_call_budget_exceeded: "runtime.outcome.reason.modelCallLimit",
  run_time_limit: "runtime.outcome.reason.timeLimit",
};

function hasUncertainChanges(turn: Turn): boolean {
  const results = new Map<string, ToolResultEvent>();
  const latestProcessResults = new Map<string, ToolResultEvent>();
  for (const event of turn.events) {
    if (event.type === "tool_result") {
      results.set(event.toolCallId, event);
      if (event.details?.processId) latestProcessResults.set(event.details.processId, event);
    }
  }

  const uncertain = (result: ToolResultEvent) => {
    const latest = result.details?.processId ? latestProcessResults.get(result.details.processId) ?? result : result;
    return latest.status !== "success" ||
      (latest.details?.processState !== undefined && latest.details.processState !== "exited");
  };
  return turn.events.some((event) => {
    if (event.type === "tool_result") {
      return MUTATING_TOOLS.has(event.toolName ?? "") && uncertain(event);
    }
    if (event.type !== "tool_call" || !MUTATING_TOOLS.has(event.toolName)) return false;
    const result = results.get(event.id);
    // A recorded success remains authoritative even if a stale call still says running.
    if (result) return uncertain(result);
    return event.status !== "success";
  });
}

export function RuntimeTurnOutcome({ turn }: { turn: Turn }) {
  const { t } = useTranslation();
  if (turn.status !== "failed" && turn.status !== "interrupted") return null;

  const cancelled = turn.status === "interrupted";
  const reason =
    typeof turn.reasonCode === "string" && /^[A-Za-z0-9_.-]{1,200}$/.test(turn.reasonCode)
      ? turn.reasonCode
      : null;
  const completedTools = turn.events.some(
    (event) =>
      (event.type === "tool_call" || event.type === "tool_result") &&
      event.status === "success",
  );
  const reasonKey =
    reason !== null && Object.hasOwn(REASON_KEYS, reason) ? REASON_KEYS[reason] : undefined;

  return (
    <div
      role="status"
      aria-label={t(cancelled ? "runtime.outcome.cancelled" : "runtime.outcome.failed")}
      className="my-3 flex gap-2.5 rounded-xl border border-[#e07070]/20 bg-[#e07070]/[0.05] px-3.5 py-3 text-[11px] leading-[17px]"
    >
      {cancelled ? (
        <CircleStop size={14} aria-hidden="true" className="mt-0.5 shrink-0 text-[var(--muted-strong)]" />
      ) : (
        <AlertCircle size={14} aria-hidden="true" className="mt-0.5 shrink-0 text-[#e07070]" />
      )}
      <div className="min-w-0 space-y-1 text-[var(--muted)]">
        <p className="font-medium text-[var(--text)]">
          {t(cancelled ? "runtime.outcome.cancelled" : "runtime.outcome.failed")}
        </p>
        <p className="break-words">
          {reasonKey ? t(reasonKey) : t("runtime.outcome.reasonUnavailable")}
        </p>
        {completedTools ? <p>{t("runtime.outcome.successfulTools")}</p> : null}
        {hasUncertainChanges(turn) ? <p>{t("runtime.outcome.uncertainChanges")}</p> : null}
        <p>{t("runtime.outcome.newTurn")}</p>
        {reason ? (
          <p className="break-words text-[10px] leading-4 text-[var(--muted)]">
            {t("runtime.outcome.reason", { reason })}
          </p>
        ) : null}
      </div>
    </div>
  );
}
