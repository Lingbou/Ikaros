import * as Dialog from "@radix-ui/react-dialog";
import { ChevronRight, Plus, Search, Sparkles, Trash2 } from "lucide-react";
import {
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
  type Ref
} from "react";
import { flushSync } from "react-dom";

import { useTranslation } from "../i18n";
import { cx } from "./ui";

export interface ProviderModel {
  id: string;
  displayName: string;
  enabled: boolean;
}

export interface CustomProvider {
  id: string;
  displayName: string;
  models: ProviderModel[];
}

export interface ProvidersSettingsProps {
  deepSeekConnected: boolean;
  customProviders: readonly CustomProvider[];
  onConnectDeepSeek(): void;
  onDisconnectDeepSeek(): void;
  onAddCustomProvider(provider: CustomProvider): void;
  onDisconnectCustomProvider(providerId: string): void;
}

export interface ModelsSettingsProps {
  deepSeekConnected: boolean;
  deepSeekModels: readonly ProviderModel[];
  customProviders: readonly CustomProvider[];
  onToggleDeepSeekModel(modelId: string): void;
  onToggleCustomModel(providerId: string, modelId: string): void;
}

interface DraftModel {
  key: number;
  id: string;
  displayName: string;
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

type ProviderDialog = "deepseek" | "custom";

let draftRowKey = 0;

function nextDraftRowKey(): number {
  draftRowKey += 1;
  return draftRowKey;
}

function createModelDraft(): DraftModel {
  return { key: nextDraftRowKey(), id: "", displayName: "" };
}

function createHeaderDraft(): DraftHeader {
  return { key: nextDraftRowKey(), name: "", value: "" };
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
      className="text-[20px] font-semibold leading-7 tracking-[-0.025em] text-[var(--text)] outline-none"
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
        "inline-flex h-8 shrink-0 items-center justify-center rounded-lg px-3 text-[12px] font-semibold leading-[18px] outline-none transition-colors focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]",
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
    <div className="flex min-h-[68px] items-center gap-3 border-t border-[var(--separator)] px-4 py-3 first:border-t-0">
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
      <span className="mb-1.5 block text-[12px] font-semibold leading-[18px] text-[var(--text)]">
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
      className="inline-flex h-8 items-center justify-center rounded-lg bg-[var(--text)] px-3.5 text-[12px] font-semibold leading-[18px] text-[var(--canvas)] outline-none transition-opacity hover:opacity-85 focus-visible:ring-2 focus-visible:ring-[var(--muted-strong)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--canvas)] disabled:cursor-not-allowed disabled:opacity-35"
    >
      {children}
    </button>
  );
}

function DeepSeekConnectForm({ onConnect }: { onConnect(): void }) {
  const { t } = useTranslation();
  const [apiKey, setApiKey] = useState("");
  const apiKeyInput = useRef<HTMLInputElement>(null);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!apiKey.trim()) return;

    // Credentials are deliberately discarded before control returns to the parent.
    if (apiKeyInput.current) apiKeyInput.current.value = "";
    flushSync(() => setApiKey(""));
    onConnect();
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
          onChange={(event) => setApiKey(event.currentTarget.value)}
          placeholder={t("settings.providers.apiKeyPlaceholder")}
          className={inputClassName}
        />
      </Field>
      <p className="mt-2 text-[11px] leading-4 text-[var(--muted)]">
        {t("settings.providers.credentialNotice")}
      </p>
      <div className="mt-5 flex justify-end">
        <SubmitButton disabled={!apiKey.trim()}>
          {t("settings.providers.continue")}
        </SubmitButton>
      </div>
    </form>
  );
}

function CustomProviderForm({
  onSubmit
}: {
  onSubmit(provider: CustomProvider): void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(createCustomProviderDraft);
  const providerId = draft.providerId.trim();
  const displayName = draft.displayName.trim();
  const baseUrl = draft.baseUrl.trim();
  const canSubmit =
    /^[a-z0-9_-]+$/.test(providerId) && Boolean(displayName) && Boolean(baseUrl);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit) return;

    const seenModelIds = new Set<string>();
    const models = draft.models.flatMap((model) => {
      const id = model.id.trim();
      if (!id || seenModelIds.has(id)) return [];
      seenModelIds.add(id);
      return [
        {
          id,
          displayName: model.displayName.trim() || id,
          enabled: true
        }
      ];
    });

    const provider: CustomProvider = {
      id: providerId,
      displayName,
      models
    };

    // Discard every connection field before returning the safe display summary.
    flushSync(() => setDraft(createCustomProviderDraft()));
    onSubmit(provider);
  };

  const updateModel = (key: number, patch: Partial<Omit<DraftModel, "key">>) => {
    setDraft((current) => ({
      ...current,
      models: current.models.map((model) => (model.key === key ? { ...model, ...patch } : model))
    }));
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
          pattern="[a-z0-9_-]+"
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

      <fieldset className="pt-1">
        <legend className="mb-2.5 text-[12px] font-semibold leading-[18px] text-[var(--text)]">
          {t("settings.providers.models")}
        </legend>
        <div className="space-y-2">
          {draft.models.map((model) => (
            <div key={model.key} className="flex items-center gap-2">
              <input
                value={model.id}
                onChange={(event) => updateModel(model.key, { id: event.currentTarget.value })}
                aria-label={t("settings.providers.modelId")}
                placeholder={t("settings.providers.modelIdPlaceholder")}
                className={cx(inputClassName, "min-w-0 flex-1")}
              />
              <input
                value={model.displayName}
                onChange={(event) =>
                  updateModel(model.key, { displayName: event.currentTarget.value })
                }
                aria-label={t("settings.providers.modelDisplayName")}
                placeholder={t("settings.providers.modelDisplayNamePlaceholder")}
                className={cx(inputClassName, "min-w-0 flex-1")}
              />
              <button
                type="button"
                aria-label={t("settings.providers.removeModel")}
                onClick={() =>
                  setDraft((current) => ({
                    ...current,
                    models: current.models.filter((item) => item.key !== model.key)
                  }))
                }
                className="flex size-9 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
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
              models: [...current.models, createModelDraft()]
            }))
          }
          className="mt-2 inline-flex h-8 items-center gap-2 rounded-lg px-2 text-[12px] font-semibold leading-[18px] text-[var(--muted-strong)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
        >
          <Plus size={13} aria-hidden="true" />
          {t("settings.providers.addModel")}
        </button>
      </fieldset>

      <fieldset className="pt-1">
        <legend className="mb-2.5 text-[12px] font-semibold leading-[18px] text-[var(--text)]">
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
                className="flex size-9 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
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
          className="mt-2 inline-flex h-8 items-center gap-2 rounded-lg px-2 text-[12px] font-semibold leading-[18px] text-[var(--muted-strong)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
        >
          <Plus size={13} aria-hidden="true" />
          {t("settings.providers.addHeader")}
        </button>
      </fieldset>

      <div className="flex justify-end pb-1 pt-1">
        <SubmitButton disabled={!canSubmit}>{t("settings.providers.submit")}</SubmitButton>
      </div>
    </form>
  );
}

export function ProvidersSettings({
  deepSeekConnected,
  customProviders,
  onConnectDeepSeek,
  onDisconnectDeepSeek,
  onAddCustomProvider,
  onDisconnectCustomProvider
}: ProvidersSettingsProps) {
  const { t } = useTranslation();
  const [activeDialog, setActiveDialog] = useState<ProviderDialog | null>(null);
  const deepSeekTriggerRef = useRef<HTMLButtonElement>(null);
  const customTriggerRef = useRef<HTMLButtonElement>(null);
  const activeTriggerRef = useRef<HTMLButtonElement | null>(null);

  const openProviderDialog = (dialog: ProviderDialog) => {
    activeTriggerRef.current =
      dialog === "deepseek" ? deepSeekTriggerRef.current : customTriggerRef.current;
    setActiveDialog(dialog);
  };

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
                      onClick={onDisconnectDeepSeek}
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
                      label={t("settings.providers.disconnectProvider", {
                        provider: provider.displayName
                      })}
                      onClick={() => onDisconnectCustomProvider(provider.id)}
                    >
                      {t("settings.providers.disconnect")}
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
              <Dialog.Title className="text-[14px] font-semibold leading-5 text-[var(--text)]">
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
                onConnect={() => {
                  onConnectDeepSeek();
                  setActiveDialog(null);
                }}
              />
            ) : activeDialog === "custom" ? (
              <CustomProviderForm
                onSubmit={(provider) => {
                  onAddCustomProvider(provider);
                  setActiveDialog(null);
                }}
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
  models: readonly ProviderModel[];
  onToggle(modelId: string): void;
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
        className="flex h-10 w-full items-center gap-2 rounded-lg px-2 text-left text-[12px] font-semibold leading-[18px] text-[var(--text)] outline-none transition-colors hover:bg-[var(--surface-hover)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
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
              className="flex min-h-[56px] items-center justify-between gap-5 border-t border-[var(--separator)] px-4 py-2.5 first:border-t-0"
            >
              <div className="min-w-0">
                <div className="truncate text-[13px] font-semibold leading-5 text-[var(--text)]">
                  {model.displayName}
                </div>
                {model.id !== model.displayName ? (
                  <div className="mt-0.5 truncate font-mono text-[10px] leading-4 text-[var(--muted)]">
                    {model.id}
                  </div>
                ) : null}
              </div>
              <ModelSwitch
                label={t("settings.models.toggleModel", { model: model.displayName })}
                checked={model.enabled}
                onChange={() => group.onToggle(model.id)}
              />
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}

export function ModelsSettings({
  deepSeekConnected,
  deepSeekModels,
  customProviders,
  onToggleDeepSeekModel,
  onToggleCustomModel
}: ModelsSettingsProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [expandedProviders, setExpandedProviders] = useState<Record<string, boolean>>({
    deepseek: true
  });

  const groups = useMemo<ModelGroup[]>(() => {
    const connectedGroups: ModelGroup[] = [];
    if (deepSeekConnected) {
      connectedGroups.push({
        id: "deepseek",
        name: t("settings.providers.deepSeekName"),
        custom: false,
        models: deepSeekModels,
        onToggle: onToggleDeepSeekModel
      });
    }
    customProviders.forEach((provider) => {
      connectedGroups.push({
        id: `custom-${provider.id}`,
        name: provider.displayName,
        custom: true,
        models: provider.models,
        onToggle: (modelId) => onToggleCustomModel(provider.id, modelId)
      });
    });
    return connectedGroups;
  }, [
    customProviders,
    deepSeekConnected,
    deepSeekModels,
    onToggleCustomModel,
    onToggleDeepSeekModel,
    t
  ]);

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
    </div>
  );
}
