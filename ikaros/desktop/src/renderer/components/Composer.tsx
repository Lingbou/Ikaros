import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import {
  ArrowUp,
  Check,
  ChevronDown,
  FileText,
  Paperclip,
  Plus,
  X,
} from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { isRunActive } from "../domain";
import { type TranslationKey, useTranslation } from "../i18n";
import { useAppStore } from "../store";
import { MenuItem } from "./ui";

const MIN_COMPOSER_CLEARANCE = 150;
const MESSAGE_TO_COMPOSER_GAP = 24;

function attachmentName(path: string): string {
  const separator = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return separator === -1 ? path : path.slice(separator + 1);
}

export function Composer({
  onClearanceChange,
}: {
  onClearanceChange: (clearance: number) => void;
}) {
  const { t } = useTranslation();
  const draft = useAppStore((state) => state.draft);
  const runStatus = useAppStore((state) => state.runStatus);
  const providers = useAppStore((state) => state.providers);
  const models = useAppStore((state) => state.models);
  const selectedModel = useAppStore((state) => state.selectedModel);
  const connectionStatus = useAppStore((state) => state.runtimeConnectionStatus);
  const providerCatalogStatus = useAppStore((state) => state.providerCatalogStatus);
  const loadProviderCatalog = useAppStore((state) => state.loadProviderCatalog);
  const retryRuntimeConnection = useAppStore((state) => state.retryRuntimeConnection);
  const setSettingsOpen = useAppStore((state) => state.setSettingsOpen);
  const selectModel = useAppStore((state) => state.selectModel);
  const setDraft = useAppStore((state) => state.setDraft);
  const sendDraft = useAppStore((state) => state.sendDraft);
  const steerRun = useAppStore((state) => state.steerRun);
  const stopRun = useAppStore((state) => state.stopRun);
  const attachComposerContext = useAppStore((state) => state.attachComposerContext);
  const composerAttachments = useAppStore((state) => state.composerAttachments);
  const removeComposerAttachment = useAppStore((state) => state.removeComposerAttachment);
  const [projectFolderError, setProjectFolderError] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const overlayRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const canStop = runStatus === "queued" || runStatus === "running";
  const canSteer = runStatus === "running";
  const primaryStops = canStop && (!canSteer || !draft.trim());
  const runtimeModelOptions = useMemo(() => {
    const configuredProviderIds = new Set(
      providers.filter((provider) => provider.configured).map((provider) => provider.id),
    );
    return models.flatMap((candidate) => {
      if (!configuredProviderIds.has(candidate.providerId) || !candidate.enabled) return [];
      return [
        {
          providerId: candidate.providerId,
          modelId: candidate.id,
          label: candidate.displayName,
        },
      ];
    });
  }, [models, providers]);
  const selectedRuntimeModel = runtimeModelOptions.find(
    (candidate) =>
      candidate.providerId === selectedModel?.providerId &&
      candidate.modelId === selectedModel.modelId,
  );
  const runtimeModelMissing = !selectedRuntimeModel;
  const runtimeModelUnavailable =
    runtimeModelMissing || connectionStatus !== "connected" || providerCatalogStatus !== "ready";
  let modelAccess: {
    label: TranslationKey;
    message: TranslationKey;
    action?: () => void;
  } | null = null;
  if (connectionStatus === "offline") {
    modelAccess = {
      label: "composer.reconnectRuntime",
      message: "composer.runtimeOffline",
      action: () => void retryRuntimeConnection(),
    };
  } else if (connectionStatus === "starting" || connectionStatus === "reconnecting") {
    modelAccess = {
      label: connectionStatus === "starting" ? "runtime.starting" : "runtime.reconnecting",
      message: "composer.waitForRuntime",
    };
  } else if (providerCatalogStatus === "idle" || providerCatalogStatus === "loading") {
    modelAccess = { label: "composer.loadingModels", message: "composer.waitForModels" };
  } else if (providerCatalogStatus === "error") {
    modelAccess = {
      label: "composer.retryModels",
      message: "composer.modelsLoadFailed",
      action: () => void loadProviderCatalog().catch(() => undefined),
    };
  } else if (runtimeModelOptions.length === 0) {
    const hasConfiguredModels = models.some((candidate) =>
      providers.some((provider) => provider.id === candidate.providerId && provider.configured),
    );
    // Without any provider the composer stays quiet: first-run setup happens in
    // the onboarding dialog, and settings remain reachable from the sidebar.
    if (hasConfiguredModels) {
      modelAccess = {
        label: "composer.enableModel",
        message: "composer.modelsDisabled",
        action: () => setSettingsOpen(true, "models"),
      };
    }
  }
  useLayoutEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "0px";
    textarea.style.height = `${Math.min(154, Math.max(34, textarea.scrollHeight))}px`;
  }, [draft]);


  useLayoutEffect(() => {
    const overlay = overlayRef.current;
    const surface = surfaceRef.current;
    if (!overlay || !surface) return;

    const measure = () => {
      const overlayBottom = overlay.getBoundingClientRect().bottom;
      const surfaceTop = surface.getBoundingClientRect().top;
      onClearanceChange(
        Math.max(
          MIN_COMPOSER_CLEARANCE,
          Math.ceil(overlayBottom - surfaceTop + MESSAGE_TO_COMPOSER_GAP),
        ),
      );
    };

    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(overlay);
    observer.observe(surface);
    return () => observer.disconnect();
  }, [onClearanceChange]);

  const send = () => {
    if (isRunActive(runStatus) && !canSteer) return;
    if (canSteer ? connectionStatus !== "connected" : runtimeModelUnavailable) return;
    if (canSteer) {
      void steerRun();
      return;
    }
    void sendDraft();
  };

  const runPrimaryAction = () => {
    if (primaryStops) {
      stopRun();
      return;
    }
    send();
  };


  const addFilesAndFolders = () => {
    setProjectFolderError(false);
    void attachComposerContext().catch(() => {
      setProjectFolderError(true);
    });
  };

  return (
    <div
      ref={overlayRef}
      data-composer-overlay
      className="pointer-events-none absolute inset-x-0 bottom-0 z-20 bg-gradient-to-t from-[var(--canvas)] via-[var(--canvas)]/96 to-transparent px-3 pb-3 pt-16 sm:px-5 sm:pb-3.5"
    >
      <div className="conversation-rail pointer-events-auto relative mx-auto w-full">

        <div
          ref={surfaceRef}
          data-composer-surface
          className="composer-shadow rounded-[22px] border border-[var(--border)] bg-[var(--panel-raised)] px-3 pb-2.5 pt-3.5"
        >
          {composerAttachments.length ? (
            <div data-composer-attachments className="mb-2 flex flex-wrap gap-1.5 px-1">
              {composerAttachments.map((path) => (
                <span
                  key={path}
                  className="flex h-7 max-w-[240px] items-center gap-1.5 rounded-lg border border-[var(--border-soft)] bg-[var(--panel)] pl-2 pr-1 text-[11px] leading-4 text-[var(--muted-strong)]"
                >
                  <FileText size={12} className="shrink-0" aria-hidden="true" />
                  <span className="min-w-0 truncate" title={path}>
                    {attachmentName(path)}
                  </span>
                  <button
                    type="button"
                    aria-label={t("composer.removeAttachment", { name: attachmentName(path) })}
                    onClick={() => removeComposerAttachment(path)}
                    className="flex size-5 shrink-0 items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
                  >
                    <X size={11} />
                  </button>
                </span>
              ))}
            </div>
          ) : null}

          <textarea
            ref={textareaRef}
            rows={1}
            value={draft}
            onChange={(event) => {
              setDraft(event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing) return;


              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                send();
              }
            }}
            placeholder={t(canSteer ? "composer.steerPlaceholder" : "composer.placeholder")}
            aria-label={t("composer.messageLabel")}
            className="app-scrollbar block max-h-[154px] min-h-[34px] w-full resize-none overflow-y-auto bg-transparent px-1 text-[14px] leading-[22px] text-[var(--text)] outline-none placeholder:text-[var(--muted)] focus-visible:outline-none"
          />

          <div className="mt-2 flex min-w-0 items-center gap-1.5">
            <DropdownMenu.Root>
              <DropdownMenu.Trigger asChild>
                <button
                  type="button"
                  aria-label={t("composer.addContext")}
                  className="flex size-7 shrink-0 items-center justify-center rounded-full border border-[var(--border-soft)] bg-[var(--panel)] text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] data-[state=open]:bg-[var(--surface-hover)] data-[state=open]:text-[var(--text)]"
                >
                  <Plus size={13} />
                </button>
              </DropdownMenu.Trigger>
              <DropdownMenu.Portal>
                <DropdownMenu.Content
                  side="top"
                  align="start"
                  sideOffset={8}
                  className="glass-menu z-[90] w-60 rounded-xl p-1.5"
                >
                  <DropdownMenu.Label className="px-2.5 pb-1 pt-1.5 text-[11px] font-medium leading-4 text-[var(--muted)]">
                    {t("composer.addContext")}
                  </DropdownMenu.Label>
                  <DropdownMenu.Item
                    onSelect={addFilesAndFolders}
                    className="rounded-lg outline-none data-[highlighted]:bg-[var(--surface-hover)]"
                  >
                    <MenuItem
                      icon={<Paperclip size={14} />}
                      label={t("composer.addFilesAndFolders")}
                    />
                  </DropdownMenu.Item>
                </DropdownMenu.Content>
              </DropdownMenu.Portal>
            </DropdownMenu.Root>

            <div className="min-w-0 flex-1" />

            {modelAccess ? (
              <button
                type="button"
                data-model-settings-trigger
                disabled={!modelAccess.action}
                onClick={modelAccess.action}
                className="h-7 max-w-[220px] truncate rounded-lg px-2 text-[11px] font-medium leading-4 text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)] disabled:cursor-wait disabled:opacity-60"
              >
                {t(modelAccess.label)}
              </button>
            ) : runtimeModelOptions.length === 0 ? null : (
              <DropdownMenu.Root>
                <DropdownMenu.Trigger asChild>
                  <button
                    type="button"
                    data-model-settings-trigger
                    className="hidden h-7 min-w-0 max-w-[180px] items-center gap-1 rounded-lg px-2 text-[11px] leading-4 text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)] sm:flex"
                  >
                    <span className="min-w-0 max-w-[148px] truncate">
                      {selectedRuntimeModel?.label ?? t("composer.selectModel")}
                    </span>
                    <ChevronDown size={11} />
                  </button>
                </DropdownMenu.Trigger>
                <DropdownMenu.Portal>
                  <DropdownMenu.Content
                    side="top"
                    align="end"
                    sideOffset={8}
                    className="glass-menu z-[90] min-w-48 max-w-[280px] rounded-xl p-1"
                  >
                    {runtimeModelOptions.map((candidate) => (
                          <DropdownMenu.Item
                            key={`${candidate.providerId}/${candidate.modelId}`}
                            onSelect={() =>
                              selectModel({
                                providerId: candidate.providerId,
                                modelId: candidate.modelId,
                              })
                            }
                            className="flex h-8 cursor-default items-center gap-2 rounded-lg px-2 text-[12px] text-[var(--text)] outline-none data-[highlighted]:bg-[var(--surface-hover)]"
                          >
                            <span className="flex size-4 items-center justify-center text-[var(--accent)]">
                              {selectedRuntimeModel?.providerId === candidate.providerId &&
                              selectedRuntimeModel.modelId === candidate.modelId ? (
                                <Check size={12} />
                              ) : null}
                            </span>
                            <span className="min-w-0 truncate">{candidate.label}</span>
                          </DropdownMenu.Item>
                        ))
}
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu.Root>
            )}

            <button
              type="button"
              aria-label={primaryStops ? t("composer.stopRun") : t(canSteer ? "composer.sendSteer" : "composer.sendMessage")}
              onClick={runPrimaryAction}
              disabled={!primaryStops && ((canSteer ? connectionStatus !== "connected" : runtimeModelUnavailable) || !draft.trim())}
              className="flex size-8 shrink-0 items-center justify-center rounded-full bg-[var(--text)] text-[var(--canvas)] transition-transform hover:scale-[1.03] disabled:bg-[var(--border)] disabled:text-[var(--muted)]"
            >
              {primaryStops ? (
                <span className="size-2.5 rounded-[2px] bg-[var(--canvas)]" />
              ) : (
                <ArrowUp size={16} strokeWidth={2.2} />
              )}
            </button>
          </div>
          {modelAccess ? (
            <p role="status" className="px-1 pt-1.5 text-[11px] leading-4 text-[var(--muted)]">
              {t(modelAccess.message)}
            </p>
          ) : null}
          {projectFolderError ? (
            <div role="alert" className="px-1 pt-1.5 text-[11px] leading-4 text-[#e07070]">
              {t("composer.addFilesAndFoldersFailed")}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
