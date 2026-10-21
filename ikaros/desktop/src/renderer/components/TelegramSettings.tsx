import { CheckCircle2, LoaderCircle, RefreshCw, Send, XCircle } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { RuntimeChannelSummary } from "../../shared/runtime";
import { useTranslation } from "../i18n";
import { createRuntimeClient } from "../runtimeClient";
import { useAppStore } from "../store";
import { cx } from "./ui";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function statusTone(status: RuntimeChannelSummary["status"]): string {
  if (status === "connected") return "text-[#66c98b]";
  if (status === "error") return "text-[#e08b8b]";
  if (status === "connecting") return "text-[#d6ad63]";
  return "text-[var(--muted)]";
}

export function TelegramSettings() {
  const { t } = useTranslation();
  const runtime = useMemo(() => createRuntimeClient(), []);
  const providers = useAppStore((state) => state.providers);
  const models = useAppStore((state) => state.models);
  const loadProviderCatalog = useAppStore((state) => state.loadProviderCatalog);

  const [channel, setChannel] = useState<RuntimeChannelSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [token, setToken] = useState("");
  const [tokenDirty, setTokenDirty] = useState(false);
  const [proxy, setProxy] = useState("");
  const [proxyDirty, setProxyDirty] = useState(false);
  const [providerId, setProviderId] = useState("");
  const [modelId, setModelId] = useState("");
  const [modelDirty, setModelDirty] = useState(false);

  const enabledModels = useMemo(() => models.filter((model) => model.enabled), [models]);
  const providerNames = useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider.displayName])),
    [providers]
  );
  const selectedModelKey = providerId && modelId ? `${providerId}\u0000${modelId}` : "";

  const applyChannel = useCallback((next: RuntimeChannelSummary) => {
    setChannel(next);
    setEnabled(next.enabled);
    if (!tokenDirty) setToken("");
    if (!proxyDirty) setProxy("");
    if (!modelDirty) {
      if (next.defaultProviderId && next.defaultModelId) {
        setProviderId(next.defaultProviderId);
        setModelId(next.defaultModelId);
      } else if (enabledModels[0]) {
        setProviderId(enabledModels[0].providerId);
        setModelId(enabledModels[0].id);
      }
    }
  }, [enabledModels, modelDirty, proxyDirty, tokenDirty]);

  const refresh = useCallback(async () => {
    if (!runtime) {
      setError(t("settings.telegram.unavailable"));
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const result = await runtime.listChannels();
      const telegram = result.channels.find((item) => item.id === "telegram") ?? null;
      if (telegram) applyChannel(telegram);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, [applyChannel, runtime, t]);

  // Load once per mount. `refresh` changes identity whenever the provider
  // catalog is replaced, so depending on it here would re-run the effect on
  // every catalog update and spin the whole renderer.
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  const loadProviderCatalogRef = useRef(loadProviderCatalog);
  loadProviderCatalogRef.current = loadProviderCatalog;
  useEffect(() => {
    void loadProviderCatalogRef.current().catch(() => undefined);
    void refreshRef.current();
  }, []);

  const save = async () => {
    if (!runtime) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const params: Parameters<typeof runtime.configureChannel>[0] = {
        channelId: "telegram",
        enabled,
        defaultProviderId: providerId || null,
        defaultModelId: modelId || null
      };
      if (tokenDirty) params.token = token.trim() || null;
      if (proxyDirty) params.proxy = proxy.trim() || null;
      const result = await runtime.configureChannel(params);
      applyChannel(result.channel);
      setTokenDirty(false);
      setProxyDirty(false);
      setModelDirty(false);
      setToken("");
      setProxy("");
      setNotice(t("settings.telegram.saved"));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  const test = async () => {
    if (!runtime) return;
    setTesting(true);
    setError(null);
    setNotice(null);
    try {
      const params: Parameters<typeof runtime.testChannel>[0] = {
        channelId: "telegram",
        proxy: proxyDirty ? proxy.trim() || null : undefined
      };
      if (token.trim()) params.token = token.trim();
      const result = await runtime.testChannel(params);
      if (result.ok && result.bot) {
        setNotice(
          t("settings.telegram.testSuccess", {
            username: `@${result.bot.username}`
          })
        );
      } else {
        setError(
          t("settings.telegram.testFailed", {
            reason: result.reasonCode ?? "unknown"
          })
        );
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setTesting(false);
    }
  };

  const resetBinding = async () => {
    if (!runtime) return;
    if (!window.confirm(t("settings.telegram.resetConfirm"))) return;
    setError(null);
    setNotice(null);
    try {
      const result = await runtime.resetChannelBinding({ channelId: "telegram" });
      applyChannel(result.channel);
      setNotice(t("settings.telegram.resetDone"));
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  return (
    <div>
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1
            data-settings-heading
            tabIndex={-1}
            className="text-[20px] font-semibold leading-[28px] text-[var(--text)] outline-none"
          >
            {t("settings.telegram.title")}
          </h1>
          <p className="mt-2 max-w-[620px] text-[12px] leading-5 text-[var(--muted)]">
            {t("settings.telegram.description")}
          </p>
        </div>
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={loading}
          aria-label={t("settings.telegram.refresh")}
          className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 text-[12px] font-medium text-[var(--text)] outline-none transition-colors hover:bg-[var(--panel-hover)] disabled:opacity-50"
        >
          <RefreshCw size={13} className={loading ? "animate-spin" : undefined} aria-hidden="true" />
          {t("settings.telegram.refresh")}
        </button>
      </div>

      <section aria-labelledby="telegram-status-heading" className="mt-8">
        <h2 id="telegram-status-heading" className="mb-3 text-[13px] font-semibold leading-5">
          {t("settings.telegram.status")}
        </h2>
        <div className="rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)] p-4">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
            <span className={cx("flex items-center gap-1.5 text-[13px] font-semibold", statusTone(channel?.status ?? "disabled"))}>
              {channel?.status === "connected" ? (
                <CheckCircle2 size={15} aria-hidden="true" />
              ) : channel?.status === "error" ? (
                <XCircle size={15} aria-hidden="true" />
              ) : channel?.status === "connecting" || loading ? (
                <LoaderCircle size={15} className="animate-spin" aria-hidden="true" />
              ) : (
                <Send size={15} aria-hidden="true" />
              )}
              {t(`settings.telegram.connection.${channel?.status ?? "disabled"}`)}
            </span>
            {channel?.bot ? (
              <span className="text-[12px] text-[var(--muted)]">@{channel.bot.username}</span>
            ) : null}
            {channel?.owner ? (
              <span className="text-[12px] text-[var(--muted)]">
                {t("settings.telegram.owner", {
                  owner: channel.owner.username
                    ? `@${channel.owner.username}`
                    : channel.owner.userId ?? "—"
                })}
              </span>
            ) : null}
          </div>
          <p className="mt-2 text-[11px] leading-4 text-[var(--muted)]">
            {t("settings.telegram.fixedThreadHint")}
          </p>
          {channel?.lastErrorCode ? (
            <p className="mt-2 text-[11px] leading-4 text-[#e08b8b]">
              {t("settings.telegram.lastError", { code: channel.lastErrorCode })}
            </p>
          ) : null}
        </div>
      </section>

      <section aria-labelledby="telegram-config-heading" className="mt-8">
        <h2 id="telegram-config-heading" className="mb-3 text-[13px] font-semibold leading-5">
          {t("settings.telegram.configuration")}
        </h2>
        <div className="space-y-4 rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)] p-4">
          <label className="flex items-center justify-between gap-4">
            <span>
              <span className="block text-[13px] font-semibold leading-5">
                {t("settings.telegram.enabled")}
              </span>
              <span className="mt-0.5 block text-[11px] leading-4 text-[var(--muted)]">
                {t("settings.telegram.enabledDescription")}
              </span>
            </span>
            <input
              type="checkbox"
              checked={enabled}
              onChange={(event) => setEnabled(event.currentTarget.checked)}
              className="size-4 accent-[var(--accent)]"
            />
          </label>

          <label className="block">
            <span className="text-[12px] font-medium leading-5">{t("settings.telegram.botToken")}</span>
            <div className="mt-1.5 flex gap-2">
              <input
                type="password"
                value={token}
                onChange={(event) => {
                  setToken(event.currentTarget.value);
                  setTokenDirty(true);
                }}
                placeholder={
                  channel?.configured && !tokenDirty
                    ? t("settings.telegram.tokenConfigured")
                    : t("settings.telegram.botTokenPlaceholder")
                }
                className="h-9 min-w-0 flex-1 rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 text-[12px] text-[var(--text)] placeholder:text-[var(--muted)]"
              />
              {channel?.configured && !tokenDirty ? (
                <button
                  type="button"
                  onClick={() => setTokenDirty(true)}
                  className="h-9 shrink-0 rounded-lg border border-[var(--border)] px-3 text-[12px] font-medium hover:bg-[var(--panel-hover)]"
                >
                  {t("settings.telegram.replaceToken")}
                </button>
              ) : null}
            </div>
          </label>

          <label className="block">
            <span className="text-[12px] font-medium leading-5">{t("settings.telegram.proxy")}</span>
            <input
              type="text"
              value={proxy}
              onChange={(event) => {
                setProxy(event.currentTarget.value);
                setProxyDirty(true);
              }}
              placeholder={
                channel?.proxyConfigured && !proxyDirty
                  ? t("settings.telegram.proxyConfigured")
                  : t("settings.telegram.proxyPlaceholder")
              }
              className="mt-1.5 h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 text-[12px] text-[var(--text)] placeholder:text-[var(--muted)]"
            />
          </label>

          <label className="block">
            <span className="text-[12px] font-medium leading-5">{t("settings.telegram.defaultModel")}</span>
            <select
              value={selectedModelKey}
              onChange={(event) => {
                const [nextProvider, nextModel] = event.currentTarget.value.split("\u0000");
                setProviderId(nextProvider ?? "");
                setModelId(nextModel ?? "");
                setModelDirty(true);
              }}
              className="mt-1.5 h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-2 text-[12px] text-[var(--text)]"
            >
              <option value="">{t("settings.telegram.noModel")}</option>
              {enabledModels.map((model) => (
                <option key={`${model.providerId}:${model.id}`} value={`${model.providerId}\u0000${model.id}`}>
                  {providerNames.get(model.providerId) ?? model.providerId} / {model.displayName}
                </option>
              ))}
            </select>
            <span className="mt-1 block text-[11px] leading-4 text-[var(--muted)]">
              {t("settings.telegram.defaultModelDescription")}
            </span>
          </label>

          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => void test()}
              disabled={testing || saving}
              className="h-8 rounded-lg border border-[var(--border)] px-3 text-[12px] font-medium hover:bg-[var(--panel-hover)] disabled:opacity-50"
            >
              {testing ? t("settings.telegram.testing") : t("settings.telegram.test")}
            </button>
            <button
              type="button"
              onClick={() => void save()}
              disabled={saving || testing}
              className="h-8 rounded-lg bg-[var(--accent)] px-3 text-[12px] font-semibold text-white disabled:opacity-50"
            >
              {saving ? t("settings.telegram.saving") : t("settings.telegram.save")}
            </button>
            <button
              type="button"
              onClick={() => void resetBinding()}
              disabled={saving || testing}
              className="h-8 rounded-lg border border-[#6b3b3b] px-3 text-[12px] font-medium text-[#e08b8b] hover:bg-[#3a2626] disabled:opacity-50"
            >
              {t("settings.telegram.resetBinding")}
            </button>
          </div>
          <p aria-live="polite" className="min-h-4 text-[11px] leading-4">
            {error ? <span className="text-[#e08b8b]">{error}</span> : null}
            {!error && notice ? <span className="text-[#66c98b]">{notice}</span> : null}
          </p>
        </div>
      </section>
    </div>
  );
}
