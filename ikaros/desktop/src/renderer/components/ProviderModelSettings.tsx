import * as Dialog from "@radix-ui/react-dialog";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import {
  Check,
  ChevronRight,
  ListPlus,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  Sparkles,
  Trash2,
  X
} from "lucide-react";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
  type Ref
} from "react";
import { flushSync } from "react-dom";

import type {
  RuntimeDiscoveredModel,
  RuntimeModelInput,
  RuntimeModelSummary,
  RuntimeProviderConfigureParams,
  RuntimeProviderSummary
} from "../../shared/runtime";
import { useTranslation } from "../i18n";
import { cx } from "./ui";

export interface ProvidersSettingsProps {
  providers: readonly RuntimeProviderSummary[];
  models: readonly RuntimeModelSummary[];
  onDiscoverDeepSeekModels(apiKey: string): Promise<RuntimeDiscoveredModel[]>;
  onConfigureProvider(params: RuntimeProviderConfigureParams): Promise<void>;
  onDisconnectDeepSeek(): Promise<void>;
  onRemoveCustomProvider(providerId: string): Promise<void>;
}

export interface ModelsSettingsProps {
  providers: readonly RuntimeProviderSummary[];
  models: readonly RuntimeModelSummary[];
  onSetModelEnabled(providerId: string, modelId: string, enabled: boolean): Promise<void>;
  onSetModelLimits(providerId: string, modelId: string, contextWindow: number): Promise<void>;
}

interface DraftModel {
  key: number;
  id: string;
  displayName: string;
  contextWindow: string;
}

interface DraftHeader {
  key: number;
  name: string;
  value: string;
}

interface CustomProviderDraft {
  providerId: string;
  displayName: string;
  baseUrl: string;
  apiKey: string;
  models: DraftModel[];
  headers: DraftHeader[];
}

type ModelDiscoveryStatus = "idle" | "loading" | "ready" | "empty" | "error";

type ProviderDialog = "deepseek" | "custom";
const CUSTOM_PROVIDER_ID = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const RESERVED_PROVIDER_IDS = new Set(["deepseek", "scripted"]);

let draftRowKey = 0;

function nextDraftRowKey(): number {
  draftRowKey += 1;
  return draftRowKey;
}

function createModelDraft(): DraftModel {
  return { key: nextDraftRowKey(), id: "", displayName: "", contextWindow: "32768" };
}

function createHeaderDraft(): DraftHeader {
  return { key: nextDraftRowKey(), name: "", value: "" };
}

function createModelDrafts(models: readonly RuntimeModelSummary[]): DraftModel[] {
  if (models.length === 0) return [createModelDraft()];
  return models.map((model) => ({
    key: nextDraftRowKey(),
    id: model.id,
    displayName: model.displayName,
    contextWindow: String(model.contextWindow)
  }));
}

function normalizedModels(drafts: readonly DraftModel[]): RuntimeModelInput[] | null {
  const models: RuntimeModelInput[] = [];
  const seen = new Set<string>();
  for (const draft of drafts) {
    const id = draft.id.trim();
    const displayName = draft.displayName.trim();
    if (!id && !displayName) continue;
    if (!id || seen.has(id)) return null;
    seen.add(id);
    const contextWindow = Number(draft.contextWindow);
    if (!Number.isSafeInteger(contextWindow) || contextWindow <= 1) return null;
    models.push({ id, displayName: displayName || id, contextWindow });
  }
  return models.length > 0 ? models : null;
}

function normalizedHeaders(drafts: readonly DraftHeader[]): Record<string, string> | null {
  const headers: Record<string, string> = {};
  const seen = new Set<string>();
  for (const draft of drafts) {
    const name = draft.name.trim();
    const value = draft.value;
    if (!name && !value) continue;
    const normalizedName = name.toLocaleLowerCase();
    if (!name || !value || seen.has(normalizedName)) return null;
    seen.add(normalizedName);
    headers[name] = value;
  }
  return headers;
}

function createCustomProviderDraft(): CustomProviderDraft {
  return {
    providerId: "",
    displayName: "",
    baseUrl: "",
    apiKey: "",
    models: [createModelDraft()],
    headers: [createHeaderDraft()]
  };
}

function SettingsHeading({ children }: { children: ReactNode }) {
  return (
    <h1
      data-settings-heading
      tabIndex={-1}
      className="text-[20px] font-semibold leading-7 text-[var(--text)] outline-none"
    >
      {children}
    </h1>
  );
}

function ProviderMark({ custom = false }: { custom?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className="flex size-7 shrink-0 items-center justify-center rounded-lg border border-[var(--border-soft)] bg-[var(--panel-raised)] text-[var(--muted-strong)]"
    >
      <Sparkles size={custom ? 14 : 13} strokeWidth={custom ? 1.8 : 2.2} />
    </span>
  );
}

function ActionButton({
  children,
  buttonRef,
  dialogControls,
  dialogExpanded,
  label,
  onClick,
  quiet = false
}: {
  children: ReactNode;
  buttonRef?: Ref<HTMLButtonElement>;
  dialogControls?: string;
  dialogExpanded?: boolean;
  label: string;
  onClick(): void;
  quiet?: boolean;
}) {
  return (
    <button
      ref={buttonRef}
      type="button"
      aria-label={label}
      aria-haspopup={dialogControls ? "dialog" : undefined}
      aria-expanded={dialogControls ? dialogExpanded : undefined}
      aria-controls={dialogControls}
      onClick={onClick}
      className={cx(
        "inline-flex h-8 shrink-0 items-center justify-center rounded-lg px-3 text-[12px] font-medium leading-[18px] outline-none transition-colors focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]",
        quiet
          ? "text-[var(--muted)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
          : "border border-[var(--border)] bg-[var(--panel-raised)] text-[var(--text)] shadow-sm hover:bg-[var(--panel-hover)]"
      )}
    >
      {children}
    </button>
  );
}

function ProviderRow({
  name,
  description,
  badge,
  action
}: {
  name: string;
  description?: string;
  badge?: string;
  action: ReactNode;
}) {
  return (
    <div className="flex min-h-[58px] items-center gap-3 border-t border-[var(--separator)] px-4 py-2.5 first:border-t-0">
      <ProviderMark custom={Boolean(badge)} />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-[13px] font-semibold leading-5 text-[var(--text)]">
            {name}
          </span>
          {badge ? (
            <span className="shrink-0 rounded border border-[var(--border)] bg-[var(--panel-raised)] px-1.5 py-px text-[10px] font-medium leading-[14px] text-[var(--muted)]">
              {badge}
            </span>
          ) : null}
        </div>
        {description ? (
          <p className="mt-0.5 truncate text-[11px] leading-4 text-[var(--muted)]">
            {description}
          </p>
        ) : null}
      </div>
      {action}
    </div>
  );
}

function Field({
  label,
  hint,
  children
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[12px] font-medium leading-[18px] text-[var(--text)]">
        {label}
      </span>
      {children}
      {hint ? (
        <span className="mt-1.5 block text-[11px] font-medium leading-4 text-[var(--muted)]">
          {hint}
        </span>
      ) : null}
    </label>
  );
}

const inputClassName =
  "h-9 w-full rounded-[9px] border border-[var(--border)] bg-[var(--panel)] px-3 text-[12px] leading-[18px] text-[var(--text)] outline-none transition-colors placeholder:text-[var(--muted)] hover:border-[var(--muted)] focus:border-[var(--muted-strong)]";

function SubmitButton({ children, disabled = false }: { children: ReactNode; disabled?: boolean }) {
  return (
    <button
      type="submit"
      disabled={disabled}
      className="inline-flex h-8 items-center justify-center rounded-lg bg-[var(--text)] px-3.5 text-[12px] font-medium leading-[18px] text-[var(--canvas)] outline-none transition-opacity hover:opacity-85 focus-visible:ring-2 focus-visible:ring-[var(--muted-strong)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--canvas)] disabled:cursor-not-allowed disabled:opacity-35"
    >
      {children}
    </button>
  );
}

function ModelDraftEditor({
  models,
  onChange,
  discoveredModels,
  discoveryStatus = "idle",
  discoveryDisabled = false,
  onDiscover
}: {
  models: readonly DraftModel[];
  onChange(models: DraftModel[]): void;
  discoveredModels?: readonly RuntimeDiscoveredModel[];
  discoveryStatus?: ModelDiscoveryStatus;
  discoveryDisabled?: boolean;
  onDiscover?(): void;
}) {
  const { t } = useTranslation();
  const update = (key: number, patch: Partial<Omit<DraftModel, "key">>) => {
    onChange(models.map((model) => (model.key === key ? { ...model, ...patch } : model)));
  };
  const canChooseDiscoveredModel = Boolean(discoveredModels?.length);
  const discoverButtonLabel =
    discoveryStatus === "loading"
      ? t("settings.providers.fetchingModels")
      : discoveryStatus === "error" || discoveryStatus === "empty"
        ? t("settings.providers.retryFetchModels")
        : discoveryStatus === "ready"
          ? t("settings.providers.refreshModels")
          : t("settings.providers.fetchModels");
  const discoveryMessage =
    discoveryStatus === "loading"
      ? t("settings.providers.fetchingModels")
      : discoveryStatus === "empty"
        ? t("settings.providers.noFetchedModels")
        : discoveryStatus === "error"
          ? t("settings.providers.fetchModelsFailed")
          : discoveryStatus === "ready"
            ? t("settings.providers.fetchedModels", {
                count: discoveredModels?.length ?? 0
              })
            : "";
  return (
    <fieldset className="pt-1">
      <legend className="sr-only">
        {t("settings.providers.models")}
      </legend>
      <div className="mb-2.5 flex min-h-8 items-center justify-between gap-3">
        <span className="text-[12px] font-medium leading-[18px] text-[var(--text)]">
          {t("settings.providers.models")}
        </span>
        {onDiscover ? (
          <button
            type="button"
            disabled={discoveryDisabled || discoveryStatus === "loading"}
            onClick={onDiscover}
            className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg px-2 text-[11px] font-medium leading-4 text-[var(--muted-strong)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] disabled:cursor-not-allowed disabled:opacity-35 disabled:hover:bg-transparent disabled:hover:text-[var(--muted-strong)]"
          >
            <RefreshCw
              size={12}
              aria-hidden="true"
              className={discoveryStatus === "loading" ? "animate-spin" : undefined}
            />
            {discoverButtonLabel}
          </button>
        ) : null}
      </div>
      {onDiscover ? (
        <p
          aria-live="polite"
          className={cx(
            "mb-2 min-h-4 text-[11px] leading-4",
            discoveryStatus === "error" ? "text-[#e08b8b]" : "text-[var(--muted)]"
          )}
        >
          {discoveryMessage}
        </p>
      ) : null}
      <div className="space-y-2">
        {models.map((model, rowIndex) => (
          <div key={model.key} className="rounded-lg border border-[var(--border-soft)] p-2">
          <div className="flex items-center gap-2">
            <input
              value={model.displayName}
              onChange={(event) =>
                update(model.key, { displayName: event.currentTarget.value })
              }
              aria-label={t("settings.providers.modelDisplayName")}
              placeholder={t("settings.providers.modelDisplayNamePlaceholder")}
              className={cx(inputClassName, "min-w-0 flex-1")}
            />
            <input
              value={model.id}
              onChange={(event) => update(model.key, { id: event.currentTarget.value })}
              aria-label={t("settings.providers.modelId")}
              placeholder={t("settings.providers.modelIdPlaceholder")}
              className={cx(inputClassName, "min-w-0 flex-1")}
            />
            {onDiscover ? (
              <DropdownMenu.Root>
                <DropdownMenu.Trigger asChild>
                  <button
                    type="button"
                    disabled={!canChooseDiscoveredModel}
                    aria-label={t("settings.providers.chooseFetchedModel", {
                      row: rowIndex + 1
                    })}
                    className="flex size-8 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] data-[state=open]:bg-[var(--surface-hover)] data-[state=open]:text-[var(--text)] disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-[var(--muted)]"
                  >
                    <ListPlus size={14} aria-hidden="true" />
                  </button>
                </DropdownMenu.Trigger>
                <DropdownMenu.Portal>
                  <DropdownMenu.Content
                    side="bottom"
                    align="end"
                    sideOffset={5}
                    className="glass-menu z-[140] max-h-[260px] min-w-[260px] max-w-[420px] overflow-y-auto rounded-xl p-1.5"
                  >
                    {discoveredModels?.map((candidate) => {
                      const selectedElsewhere = models.some(
                        (item) => item.key !== model.key && item.id.trim() === candidate.id
                      );
                      const selectedHere = model.id.trim() === candidate.id;
                      return (
                        <DropdownMenu.Item
                          key={candidate.id}
                          disabled={selectedElsewhere}
                          onSelect={() => update(model.key, { id: candidate.id })}
                          className="flex min-h-9 cursor-default select-none items-center gap-2 rounded-lg px-2.5 py-1.5 text-[12px] leading-4 text-[var(--text)] outline-none data-[disabled]:opacity-35 data-[highlighted]:bg-[var(--surface-hover)]"
                        >
                          <span className="flex size-4 shrink-0 items-center justify-center text-[var(--accent)]">
                            {selectedHere ? <Check size={13} aria-hidden="true" /> : null}
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="block truncate font-medium">
                              {candidate.displayName}
                            </span>
                            {candidate.displayName !== candidate.id ? (
                              <span className="block truncate text-[10px] text-[var(--muted)]">
                                {candidate.id}
                              </span>
                            ) : null}
                          </span>
                        </DropdownMenu.Item>
                      );
                    })}
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu.Root>
            ) : null}
            <button
              type="button"
              aria-label={t("settings.providers.removeModel")}
              onClick={() => onChange(models.filter((item) => item.key !== model.key))}
              className="flex size-8 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
            >
              <Trash2 size={14} aria-hidden="true" />
            </button>
          </div>
          <details className="group mt-2 border-t border-[var(--separator)] pt-2">
            <summary className="flex cursor-pointer list-none items-center gap-1.5 rounded text-[11px] leading-4 text-[var(--muted)] outline-none hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] [&::-webkit-details-marker]:hidden">
              <ChevronRight size={12} aria-hidden="true" className="transition-transform group-open:rotate-90" />
              {t("settings.models.advancedLimits")}
            </summary>
          <div className="mt-3">
            <Field label={t("settings.models.contextWindow")}>
              <input type="number" min={2} step={1} value={model.contextWindow}
                onChange={(event) => update(model.key, { contextWindow: event.currentTarget.value })}
                className={inputClassName} />
            </Field>
          </div>
          </details>
          </div>
        ))}
      </div>
      <button
        type="button"
        onClick={() => onChange([...models, createModelDraft()])}
        className="mt-2 inline-flex h-8 items-center gap-2 rounded-lg px-2 text-[12px] font-medium leading-[18px] text-[var(--muted-strong)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
      >
        <Plus size={13} aria-hidden="true" />
        {t("settings.providers.addModel")}
      </button>
    </fieldset>
  );
}

function DeepSeekConnectForm({
  existingModels,
  onConnected,
  onConnect,
  onDiscoverModels
}: {
  existingModels: readonly RuntimeModelSummary[];
  onConnected(): void;
  onConnect(params: RuntimeProviderConfigureParams): Promise<void>;
  onDiscoverModels(apiKey: string): Promise<RuntimeDiscoveredModel[]>;
}) {
  const { t } = useTranslation();
  const [apiKey, setApiKey] = useState("");
  const [models, setModels] = useState(() => createModelDrafts(existingModels));
  const [discoveredModels, setDiscoveredModels] = useState<RuntimeDiscoveredModel[]>([]);
  const [discoveryStatus, setDiscoveryStatus] = useState<ModelDiscoveryStatus>("idle");
  const [submitting, setSubmitting] = useState(false);
  const [failed, setFailed] = useState(false);
  const apiKeyInput = useRef<HTMLInputElement>(null);
  const discoveryRevision = useRef(0);
  const configuredModels = normalizedModels(models);

  useEffect(
    () => () => {
      discoveryRevision.current += 1;
    },
    []
  );

  const changeApiKey = (nextApiKey: string) => {
    discoveryRevision.current += 1;
    setApiKey(nextApiKey);
    setDiscoveredModels([]);
    setDiscoveryStatus("idle");
  };

  const discoverModels = async () => {
    const normalizedKey = apiKey.trim();
    if (!normalizedKey || discoveryStatus === "loading" || submitting) return;

    const revision = ++discoveryRevision.current;
    setDiscoveryStatus("loading");
    try {
      const candidates = await onDiscoverModels(normalizedKey);
      if (revision !== discoveryRevision.current) return;
      setDiscoveredModels(candidates);
      setDiscoveryStatus(candidates.length > 0 ? "ready" : "empty");
    } catch {
      if (revision !== discoveryRevision.current) return;
      setDiscoveryStatus("error");
    }
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const normalizedKey = apiKey.trim();
    if (!normalizedKey || !configuredModels || submitting) return;

    setSubmitting(true);
    setFailed(false);
    try {
      await onConnect({ kind: "deepseek", apiKey: normalizedKey, models: configuredModels });
      discoveryRevision.current += 1;
      if (apiKeyInput.current) apiKeyInput.current.value = "";
      flushSync(() => {
        setApiKey("");
        setModels([createModelDraft()]);
        setDiscoveredModels([]);
        setDiscoveryStatus("idle");
      });
      onConnected();
    } catch {
      setFailed(true);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form onSubmit={submit}>
      <Field label={t("settings.providers.deepSeekApiKey")}>
        <input
          ref={apiKeyInput}
          autoFocus
          type="password"
          required
          autoComplete="off"
          aria-label={t("settings.providers.deepSeekApiKey")}
          value={apiKey}
          onChange={(event) => changeApiKey(event.currentTarget.value)}
          placeholder={t("settings.providers.apiKeyPlaceholder")}
          className={inputClassName}
        />
      </Field>
      <p className="mt-2 text-[11px] leading-4 text-[var(--muted)]">
        {t("settings.providers.credentialNotice")}
      </p>
      <div className="mt-5">
        <ModelDraftEditor
          models={models}
          onChange={setModels}
          discoveredModels={discoveredModels}
          discoveryStatus={discoveryStatus}
          discoveryDisabled={!apiKey.trim() || submitting}
          onDiscover={() => void discoverModels()}
        />
      </div>
      <p aria-live="polite" className="mt-3 min-h-4 text-[11px] leading-4 text-[#e08b8b]">
        {failed ? t("settings.providers.saveFailed") : ""}
      </p>
      <div className="mt-5 flex justify-end">
        <SubmitButton disabled={!apiKey.trim() || !configuredModels || submitting}>
          {t("settings.providers.continue")}
        </SubmitButton>
      </div>
    </form>
  );
}

function CustomProviderForm({
  onConfigured,
  onSubmit
}: {
  onConfigured(): void;
  onSubmit(params: RuntimeProviderConfigureParams): Promise<void>;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(createCustomProviderDraft);
  const [submitting, setSubmitting] = useState(false);
  const [failed, setFailed] = useState(false);
  const providerId = draft.providerId.trim();
  const displayName = draft.displayName.trim();
  const baseUrl = draft.baseUrl.trim();
  const models = normalizedModels(draft.models);
  const headers = normalizedHeaders(draft.headers);
  const canSubmit =
    CUSTOM_PROVIDER_ID.test(providerId) &&
    !RESERVED_PROVIDER_IDS.has(providerId) &&
    Boolean(displayName) &&
    Boolean(baseUrl) &&
    models !== null &&
    headers !== null;

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit || !models || !headers || submitting) return;

    const apiKey = draft.apiKey.trim();
    const params: RuntimeProviderConfigureParams = {
      kind: "custom",
      providerId,
      displayName,
      baseUrl,
      models,
      ...(apiKey ? { apiKey } : {}),
      ...(Object.keys(headers).length > 0 ? { headers } : {})
    };
    setSubmitting(true);
    setFailed(false);
    try {
      await onSubmit(params);
      flushSync(() => setDraft(createCustomProviderDraft()));
      onConfigured();
    } catch {
      setFailed(true);
    } finally {
      setSubmitting(false);
    }
  };

  const updateHeader = (key: number, patch: Partial<Omit<DraftHeader, "key">>) => {
    setDraft((current) => ({
      ...current,
      headers: current.headers.map((header) =>
        header.key === key ? { ...header, ...patch } : header
      )
    }));
  };

  return (
    <form onSubmit={submit} className="space-y-5">
      <Field
        label={t("settings.providers.providerId")}
        hint={t("settings.providers.providerIdHint")}
      >
        <input
          autoFocus
          required
          pattern="[a-z0-9][a-z0-9._-]{0,63}"
          aria-label={t("settings.providers.providerId")}
          value={draft.providerId}
          onChange={(event) => {
            const providerId = event.currentTarget.value;
            setDraft((current) => ({ ...current, providerId }));
          }}
          placeholder={t("settings.providers.providerIdPlaceholder")}
          className={inputClassName}
        />
      </Field>

      <Field label={t("settings.providers.displayName")}>
        <input
          required
          aria-label={t("settings.providers.displayName")}
          value={draft.displayName}
          onChange={(event) => {
            const displayName = event.currentTarget.value;
            setDraft((current) => ({ ...current, displayName }));
          }}
          placeholder={t("settings.providers.displayNamePlaceholder")}
          className={inputClassName}
        />
      </Field>

      <Field label={t("settings.providers.baseUrl")}>
        <input
          required
          type="url"
          aria-label={t("settings.providers.baseUrl")}
          value={draft.baseUrl}
          onChange={(event) => {
            const baseUrl = event.currentTarget.value;
            setDraft((current) => ({ ...current, baseUrl }));
          }}
          placeholder={t("settings.providers.baseUrlPlaceholder")}
          className={inputClassName}
        />
      </Field>

      <Field
        label={t("settings.providers.apiKey")}
        hint={t("settings.providers.apiKeyOptionalHint")}
      >
        <input
          type="password"
          autoComplete="off"
          aria-label={t("settings.providers.apiKey")}
          value={draft.apiKey}
          onChange={(event) => {
            const apiKey = event.currentTarget.value;
            setDraft((current) => ({ ...current, apiKey }));
          }}
          placeholder={t("settings.providers.apiKeyPlaceholder")}
          className={inputClassName}
        />
      </Field>

      <ModelDraftEditor
        models={draft.models}
        onChange={(models) => setDraft((current) => ({ ...current, models }))}
      />

      <fieldset className="pt-1">
        <legend className="mb-2.5 text-[12px] font-medium leading-[18px] text-[var(--text)]">
          {t("settings.providers.headersOptional")}
        </legend>
        <div className="space-y-2">
          {draft.headers.map((header) => (
            <div key={header.key} className="flex items-center gap-2">
              <input
                value={header.name}
                onChange={(event) =>
                  updateHeader(header.key, { name: event.currentTarget.value })
                }
                aria-label={t("settings.providers.headerName")}
                placeholder={t("settings.providers.headerNamePlaceholder")}
                className={cx(inputClassName, "min-w-0 flex-1")}
              />
              <input
                value={header.value}
                onChange={(event) =>
                  updateHeader(header.key, { value: event.currentTarget.value })
                }
                aria-label={t("settings.providers.headerValue")}
                placeholder={t("settings.providers.headerValuePlaceholder")}
                className={cx(inputClassName, "min-w-0 flex-1")}
              />
              <button
                type="button"
                aria-label={t("settings.providers.removeHeader")}
                onClick={() =>
                  setDraft((current) => ({
                    ...current,
                    headers: current.headers.filter((item) => item.key !== header.key)
                  }))
                }
                className="flex size-8 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
              >
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
        <button
          type="button"
          onClick={() =>
            setDraft((current) => ({
              ...current,
              headers: [...current.headers, createHeaderDraft()]
            }))
          }
          className="mt-2 inline-flex h-8 items-center gap-2 rounded-lg px-2 text-[12px] font-medium leading-[18px] text-[var(--muted-strong)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
        >
          <Plus size={13} aria-hidden="true" />
          {t("settings.providers.addHeader")}
        </button>
      </fieldset>

      <p aria-live="polite" className="min-h-4 text-[11px] leading-4 text-[#e08b8b]">
        {failed ? t("settings.providers.saveFailed") : ""}
      </p>
      <div className="flex justify-end pb-1 pt-1">
        <SubmitButton disabled={!canSubmit || submitting}>
          {t("settings.providers.submit")}
        </SubmitButton>
      </div>
    </form>
  );
}

export function ProvidersSettings({
  providers,
  models,
  onDiscoverDeepSeekModels,
  onConfigureProvider,
  onDisconnectDeepSeek,
  onRemoveCustomProvider
}: ProvidersSettingsProps) {
  const { t } = useTranslation();
  const [activeDialog, setActiveDialog] = useState<ProviderDialog | null>(null);
  const [actionFailed, setActionFailed] = useState(false);
  const deepSeekTriggerRef = useRef<HTMLButtonElement>(null);
  const customTriggerRef = useRef<HTMLButtonElement>(null);
  const activeTriggerRef = useRef<HTMLButtonElement | null>(null);

  const openProviderDialog = (dialog: ProviderDialog) => {
    setActionFailed(false);
    activeTriggerRef.current =
      dialog === "deepseek" ? deepSeekTriggerRef.current : customTriggerRef.current;
    setActiveDialog(dialog);
  };

  const runProviderAction = (action: () => Promise<void>) => {
    setActionFailed(false);
    void action().catch(() => setActionFailed(true));
  };

  const deepSeek = providers.find((provider) => provider.id === "deepseek");
  const customProviders = providers.filter(
    (provider) => provider.origin === "custom" && provider.configured
  );
  const deepSeekConnected = deepSeek?.configured ?? false;
  const deepSeekModels = models.filter((model) => model.providerId === "deepseek");
  const hasConnectedProviders = deepSeekConnected || customProviders.length > 0;

  return (
    <Dialog.Root
      open={activeDialog !== null}
      onOpenChange={(open) => {
        if (!open) setActiveDialog(null);
      }}
    >
      <div>
        <SettingsHeading>{t("settings.providers.title")}</SettingsHeading>

        {hasConnectedProviders ? (
          <section aria-labelledby="connected-providers-heading" className="mt-10">
            <h2
              id="connected-providers-heading"
              className="mb-3 text-[13px] font-semibold leading-5 text-[var(--text)]"
            >
              {t("settings.providers.connectedProviders")}
            </h2>
            <div className="overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)]">
              {deepSeekConnected ? (
                <ProviderRow
                  name={t("settings.providers.deepSeekName")}
                  action={
                    <ActionButton
                      quiet
                      label={t("settings.providers.disconnectProvider", {
                        provider: t("settings.providers.deepSeekName")
                      })}
                      onClick={() => runProviderAction(onDisconnectDeepSeek)}
                    >
                      {t("settings.providers.disconnect")}
                    </ActionButton>
                  }
                />
              ) : null}
              {customProviders.map((provider) => (
                <ProviderRow
                  key={provider.id}
                  name={provider.displayName}
                  badge={t("settings.providers.customBadge")}
                  action={
                    <ActionButton
                      quiet
                      label={t("settings.providers.removeProvider", {
                        provider: provider.displayName
                      })}
                      onClick={() =>
                        runProviderAction(() => onRemoveCustomProvider(provider.id))
                      }
                    >
                      {t("settings.providers.remove")}
                    </ActionButton>
                  }
                />
              ))}
            </div>
          </section>
        ) : null}

        <section aria-labelledby="available-providers-heading" className="mt-10">
          <h2
            id="available-providers-heading"
            className="mb-3 text-[13px] font-semibold leading-5 text-[var(--text)]"
          >
            {t("settings.providers.availableProviders")}
          </h2>
          <div className="overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)]">
            {!deepSeekConnected ? (
              <ProviderRow
                name={t("settings.providers.deepSeekName")}
                description={t("settings.providers.deepSeekDescription")}
                action={
                  <ActionButton
                    buttonRef={deepSeekTriggerRef}
                    dialogControls="provider-connection-dialog"
                    dialogExpanded={activeDialog === "deepseek"}
                    label={t("settings.providers.connectProvider", {
                      provider: t("settings.providers.deepSeekName")
                    })}
                    onClick={() => openProviderDialog("deepseek")}
                  >
                    <Plus size={13} aria-hidden="true" className="mr-1.5" />
                    {t("settings.providers.connect")}
                  </ActionButton>
                }
              />
            ) : null}
            <ProviderRow
              name={t("settings.providers.customProvider")}
              description={t("settings.providers.customProviderDescription")}
              badge={t("settings.providers.customBadge")}
              action={
                <ActionButton
                  buttonRef={customTriggerRef}
                  dialogControls="provider-connection-dialog"
                  dialogExpanded={activeDialog === "custom"}
                  label={t("settings.providers.connectProvider", {
                    provider: t("settings.providers.customProvider")
                  })}
                  onClick={() => openProviderDialog("custom")}
                >
                  <Plus size={13} aria-hidden="true" className="mr-1.5" />
                  {t("settings.providers.connect")}
                </ActionButton>
              }
            />
          </div>
        </section>
        <p aria-live="polite" className="mt-3 min-h-4 text-[11px] leading-4 text-[#e08b8b]">
          {actionFailed ? t("settings.providers.saveFailed") : ""}
        </p>
      </div>

      <Dialog.Portal>
        <Dialog.Overlay
          data-testid="provider-dialog-overlay"
          onClick={() => setActiveDialog(null)}
          className="fixed inset-0 z-[110] bg-black/55 backdrop-blur-[2px]"
        />
        <Dialog.Content
          id="provider-connection-dialog"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const trigger = activeTriggerRef.current;
            if (trigger?.isConnected) {
              trigger.focus();
            } else {
              document.querySelector<HTMLElement>("[data-settings-heading]")?.focus();
            }
          }}
          className="glass-menu fixed left-1/2 top-1/2 z-[120] flex max-h-[calc(100vh-40px)] w-[min(680px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl"
        >
          <div className="flex shrink-0 items-start gap-3 border-b border-[var(--separator)] px-5 py-4">
            <ProviderMark custom={activeDialog === "custom"} />
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-[15px] font-semibold leading-6 text-[var(--text)]">
                {activeDialog === "deepseek"
                  ? t("settings.providers.connectDeepSeek")
                  : t("settings.providers.customProvider")}
              </Dialog.Title>
              <Dialog.Description className="mt-1 text-[11px] leading-4 text-[var(--muted)]">
                {activeDialog === "deepseek"
                  ? t("settings.providers.deepSeekConnectDescription")
                  : t("settings.providers.customProviderDescription")}
              </Dialog.Description>
            </div>
          </div>

          <div className="app-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-5">
            {activeDialog === "deepseek" ? (
              <DeepSeekConnectForm
                existingModels={deepSeekModels}
                onConnected={() => setActiveDialog(null)}
                onConnect={onConfigureProvider}
                onDiscoverModels={onDiscoverDeepSeekModels}
              />
            ) : activeDialog === "custom" ? (
              <CustomProviderForm
                onConfigured={() => setActiveDialog(null)}
                onSubmit={onConfigureProvider}
              />
            ) : null}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function ModelSwitch({
  label,
  checked,
  onChange
}: {
  label: string;
  checked: boolean;
  onChange(): void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      onClick={onChange}
      className={cx(
        "relative h-5 w-8 shrink-0 rounded-full border outline-none transition-colors focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]",
        checked
          ? "border-[var(--accent)] bg-[var(--accent)]"
          : "border-[var(--border)] bg-[var(--panel-raised)]"
      )}
    >
      <span
        aria-hidden="true"
        className={cx(
          "absolute left-0 top-px size-4 rounded-full bg-white shadow-sm transition-transform",
          checked ? "translate-x-[14px]" : "translate-x-px"
        )}
      />
    </button>
  );
}

interface ModelGroup {
  id: string;
  name: string;
  custom: boolean;
  models: readonly RuntimeModelSummary[];
  onToggle(modelId: string, enabled: boolean): void;
  onSetLimits(modelId: string, contextWindow: number): Promise<void>;
}

function ModelLimitsEditor({ model, onSave }: {
  model: RuntimeModelSummary;
  onSave(modelId: string, contextWindow: number): Promise<void>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [context, setContext] = useState(String(model.contextWindow));
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const contextWindow = Number(context);
  const valid = Number.isSafeInteger(contextWindow) && contextWindow > 1;
  const changed = contextWindow !== model.contextWindow;
  const updateOpen = (nextOpen: boolean) => {
    if (saving) return;
    if (nextOpen) {
      setContext(String(model.contextWindow));
      setFailed(false);
    }
    setOpen(nextOpen);
  };
  const save = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!valid || saving || !changed) return;
    setSaving(true);
    setFailed(false);
    try {
      await onSave(model.id, contextWindow);
      setOpen(false);
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog.Root open={open} onOpenChange={updateOpen}>
      <Dialog.Trigger asChild>
        <button
          type="button"
          aria-label={t("settings.models.configureModel", { model: model.displayName })}
          title={t("settings.models.configureModel", { model: model.displayName })}
          className="flex size-8 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
        >
          <Settings2 size={15} strokeWidth={1.7} aria-hidden="true" />
        </button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[110] bg-black/55 backdrop-blur-[2px]" />
        <Dialog.Content className="glass-menu fixed left-1/2 top-1/2 z-[120] w-[min(440px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 rounded-2xl">
          <div className="flex items-start gap-4 border-b border-[var(--separator)] px-5 py-4">
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-[15px] font-semibold leading-6 text-[var(--text)]">
                {t("settings.models.capacityTitle")}
              </Dialog.Title>
              <Dialog.Description title={model.displayName} className="mt-1 truncate text-[12px] leading-4 text-[var(--muted)]">
                {model.displayName}
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <button
                type="button"
                disabled={saving}
                aria-label={t("settings.models.close")}
                className="flex size-7 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] disabled:opacity-35"
              >
                <X size={15} aria-hidden="true" />
              </button>
            </Dialog.Close>
          </div>
          <form onSubmit={(event) => void save(event)}>
            <div className="px-5 pt-5">
              <div>
                <Field label={t("settings.models.contextWindow")}>
                  <input type="number" min={2} step={1} value={context} disabled={saving}
                    onChange={(event) => setContext(event.currentTarget.value)}
                    className={cx(inputClassName, "tabular-nums disabled:opacity-60")} />
                </Field>
              </div>
              <p className="mt-3 text-[11px] leading-[18px] text-[var(--muted)]">
                {t("settings.models.capacityHint")}
              </p>
              <div className="min-h-12 pt-2 text-[11px] leading-4 text-[#e08b8b]">
                {!valid ? <p role="alert">{t("settings.models.invalidLimits")}</p>
                  : failed ? <p role="alert">{t("settings.models.saveFailed")}</p> : null}
              </div>
            </div>
            <div className="flex items-center justify-end gap-2 border-t border-[var(--separator)] px-5 py-3">
              <Dialog.Close asChild>
                <button type="button" disabled={saving} className="h-8 rounded-lg px-3 text-[12px] font-medium text-[var(--muted-strong)] outline-none hover:bg-[var(--surface-hover)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] disabled:opacity-35">
                  {t("settings.models.cancel")}
                </button>
              </Dialog.Close>
              <SubmitButton disabled={!valid || !changed || saving}>
                <span className="inline-flex min-w-14 justify-center">
                  {saving ? t("settings.models.savingLimits") : t("settings.models.saveLimits")}
                </span>
              </SubmitButton>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function ModelsGroup({
  group,
  expanded,
  forceExpanded,
  onToggleExpanded
}: {
  group: ModelGroup;
  expanded: boolean;
  forceExpanded: boolean;
  onToggleExpanded(): void;
}) {
  const { t } = useTranslation();
  const open = forceExpanded || expanded;

  return (
    <section aria-labelledby={`model-provider-${group.id}`}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={`model-list-${group.id}`}
        onClick={onToggleExpanded}
        className="flex h-10 w-full items-center gap-2 rounded-lg px-2 text-left text-[12px] font-medium leading-[18px] text-[var(--text)] outline-none transition-colors hover:bg-[var(--surface-hover)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
      >
        <ChevronRight
          size={13}
          aria-hidden="true"
          className={cx("text-[var(--muted)] transition-transform", open && "rotate-90")}
        />
        <ProviderMark custom={group.custom} />
        <span id={`model-provider-${group.id}`} className="truncate">
          {group.name}
        </span>
      </button>

      {open ? (
        <div
          id={`model-list-${group.id}`}
          className="mt-1 overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)]"
        >
          {group.models.map((model) => (
            <div
              key={model.id}
              className="flex min-h-[72px] items-center gap-3 border-t border-[var(--separator)] px-4 py-3 first:border-t-0"
            >
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-baseline gap-2">
                  <div title={model.displayName} className="truncate text-[13px] font-medium leading-5 text-[var(--text)]">
                    {model.displayName}
                  </div>
                  {model.id !== model.displayName ? (
                    <span title={model.id} className="min-w-0 truncate font-mono text-[10px] leading-4 text-[var(--muted)]">
                      {model.id}
                    </span>
                  ) : null}
                </div>
                <p className="mt-1 truncate text-[11px] leading-4 text-[var(--muted)] tabular-nums">
                  {t("settings.models.capacitySummary", {
                    context: model.contextWindow.toLocaleString()
                  })}
                </p>
              </div>
              <ModelLimitsEditor model={model} onSave={group.onSetLimits} />
              <ModelSwitch
                label={t("settings.models.toggleModel", { model: model.displayName })}
                checked={model.enabled}
                onChange={() => group.onToggle(model.id, !model.enabled)}
              />
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}

export function ModelsSettings({
  providers,
  models,
  onSetModelEnabled,
  onSetModelLimits
}: ModelsSettingsProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [mutationFailed, setMutationFailed] = useState(false);
  const [expandedProviders, setExpandedProviders] = useState<Record<string, boolean>>({
    deepseek: true
  });

  const groups = useMemo<ModelGroup[]>(() => {
    return providers.flatMap((provider) => {
      const providerModels = models.filter((model) => model.providerId === provider.id);
      if (providerModels.length === 0) return [];
      return [
        {
          id: provider.id,
          name: provider.displayName,
          custom: provider.origin === "custom",
          models: providerModels,
          onSetLimits: (modelId: string, contextWindow: number) =>
            onSetModelLimits(provider.id, modelId, contextWindow),
          onToggle: (modelId: string, enabled: boolean) => {
            setMutationFailed(false);
            void onSetModelEnabled(provider.id, modelId, enabled).catch(() =>
              setMutationFailed(true)
            );
          }
        }
      ];
    });
  }, [models, onSetModelEnabled, onSetModelLimits, providers]);

  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleGroups = useMemo(
    () =>
      groups
        .map((group) => {
          if (!normalizedQuery || group.name.toLocaleLowerCase().includes(normalizedQuery)) {
            return group;
          }
          return {
            ...group,
            models: group.models.filter(
              (model) =>
                model.displayName.toLocaleLowerCase().includes(normalizedQuery) ||
                model.id.toLocaleLowerCase().includes(normalizedQuery)
            )
          };
        })
        .filter((group) => !normalizedQuery || group.models.length > 0),
    [groups, normalizedQuery]
  );

  return (
    <div>
      <SettingsHeading>{t("settings.models.title")}</SettingsHeading>

      <label className="relative mt-7 block">
        <span className="sr-only">{t("settings.models.searchPlaceholder")}</span>
        <Search
          size={13}
          aria-hidden="true"
          className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--muted)]"
        />
        <input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.currentTarget.value)}
          placeholder={t("settings.models.searchPlaceholder")}
          className={cx(inputClassName, "pl-9")}
        />
      </label>

      <div className="mt-7 space-y-3">
        {visibleGroups.map((group) => (
          <ModelsGroup
            key={group.id}
            group={group}
            expanded={expandedProviders[group.id] ?? false}
            forceExpanded={Boolean(normalizedQuery)}
            onToggleExpanded={() =>
              setExpandedProviders((current) => ({
                ...current,
                [group.id]: !(current[group.id] ?? false)
              }))
            }
          />
        ))}
      </div>

      {groups.length === 0 ? (
        <div className="mt-10 rounded-2xl border border-dashed border-[var(--border)] px-6 py-10 text-center">
          <p className="text-[12px] leading-5 text-[var(--muted)]">
            {t("settings.models.noProviders")}
          </p>
        </div>
      ) : visibleGroups.length === 0 ? (
        <div className="mt-10 text-center text-[12px] leading-5 text-[var(--muted)]">
          {t("settings.models.noMatches")}
        </div>
      ) : null}
      <p aria-live="polite" className="mt-3 min-h-4 text-[11px] leading-4 text-[#e08b8b]">
        {mutationFailed ? t("settings.providers.saveFailed") : ""}
      </p>
    </div>
  );
}
