import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import {
  ArrowUp,
  Check,
  ChevronDown,
  FolderPlus,
  Plus,
} from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { isRunActive } from "../domain";
import { type TranslationKey, useTranslation } from "../i18n";
import { useAppStore } from "../store";
import {
  SLASH_COMMANDS,
  SlashCommandMenu,
  type SlashCommand,
} from "./SlashCommandMenu";
import { IconButton, MenuItem } from "./ui";

const MIN_COMPOSER_CLEARANCE = 150;
const MESSAGE_TO_COMPOSER_GAP = 24;

const MODEL_OPTIONS = ["Ikaros", "DeepSeek", "local"] as const;

type ModelId = (typeof MODEL_OPTIONS)[number];

export function Composer({
  onClearanceChange,
}: {
  onClearanceChange: (clearance: number) => void;
}) {
  const { t } = useTranslation();
  const draft = useAppStore((state) => state.draft);
  const runStatus = useAppStore((state) => state.runStatus);
  const runtimeMode = useAppStore((state) => state.runtimeMode);
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
  const stopRun = useAppStore((state) => state.stopRun);
  const bindWorkspaceFromFolder = useAppStore((state) => state.bindWorkspaceFromFolder);
  const [model, setModel] = useState<ModelId>("Ikaros");
  const [projectFolderError, setProjectFolderError] = useState(false);
  const [activeSlashIndex, setActiveSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [isComposing, setIsComposing] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const overlayRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const canStop = runStatus === "queued" || runStatus === "running";
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
  const runtimeModelMissing = runtimeMode && !selectedRuntimeModel;
  const runtimeModelUnavailable = runtimeMode &&
    (runtimeModelMissing || connectionStatus !== "connected" || providerCatalogStatus !== "ready");
  const submitBlocked = (isRunActive(runStatus) && !canStop) || runtimeModelUnavailable;
  let modelAccess: {
    label: TranslationKey;
    message: TranslationKey;
    action?: () => void;
  } | null = null;
  if (runtimeMode) {
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
      modelAccess = hasConfiguredModels
        ? {
            label: "composer.enableModel",
            message: "composer.modelsDisabled",
            action: () => setSettingsOpen(true, "models"),
          }
        : {
            label: "composer.configureModel",
            message: "composer.connectProvider",
            action: () => setSettingsOpen(true, "providers"),
          };
    }
  }
  const slashMatch = draft.match(/^\/([^\s]*)$/);
  const slashQuery = slashMatch?.[1].toLowerCase() ?? "";
  const slashCommands = useMemo(
    () =>
      SLASH_COMMANDS.filter((command) => {
        if (!slashQuery) return true;
        return `${command.token} ${t(command.labelKey)} ${t(command.descriptionKey)}`
          .toLowerCase()
          .includes(slashQuery);
      }),
    [slashQuery, t],
  );
  const slashMenuOpen = Boolean(slashMatch) && !slashDismissed && !isComposing;

  useLayoutEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "0px";
    textarea.style.height = `${Math.min(154, Math.max(34, textarea.scrollHeight))}px`;
  }, [draft]);

  useEffect(() => {
    setActiveSlashIndex(0);
  }, [slashQuery]);

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
    if (isRunActive(runStatus) || runtimeModelUnavailable) return;
    void sendDraft();
  };

  const runPrimaryAction = () => {
    if (canStop) {
      stopRun();
      return;
    }
    send();
  };

  const selectSlashCommand = (command: SlashCommand) => {
    setDraft(`${command.token} `);
    setSlashDismissed(true);
    queueMicrotask(() => textareaRef.current?.focus());
  };

  const addProjectFolder = () => {
    setProjectFolderError(false);
    void bindWorkspaceFromFolder().catch(() => {
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
        {slashMenuOpen ? (
          <SlashCommandMenu
            commands={slashCommands}
            activeIndex={activeSlashIndex}
            onActiveIndexChange={setActiveSlashIndex}
            onSelect={selectSlashCommand}
          />
        ) : null}

        <div
          ref={surfaceRef}
          data-composer-surface
          className="composer-shadow rounded-[22px] border border-[var(--border)] bg-[var(--panel-raised)] px-3 pb-2.5 pt-3.5"
        >
          <textarea
            ref={textareaRef}
            rows={1}
            value={draft}
            onChange={(event) => {
              setDraft(event.target.value);
              setSlashDismissed(false);
            }}
            onCompositionStart={() => setIsComposing(true)}
            onCompositionEnd={() => setIsComposing(false)}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing) return;

              if (slashMenuOpen) {
                if (event.key === "ArrowDown" && slashCommands.length) {
                  event.preventDefault();
                  setActiveSlashIndex((index) => (index + 1) % slashCommands.length);
                  return;
                }
                if (event.key === "ArrowUp" && slashCommands.length) {
                  event.preventDefault();
                  setActiveSlashIndex(
                    (index) => (index - 1 + slashCommands.length) % slashCommands.length,
                  );
                  return;
                }
                if (event.key === "Escape") {
                  event.preventDefault();
                  setSlashDismissed(true);
                  return;
                }
                if (
                  (event.key === "Tab" || (event.key === "Enter" && !event.shiftKey)) &&
                  slashCommands[activeSlashIndex]
                ) {
                  event.preventDefault();
                  selectSlashCommand(slashCommands[activeSlashIndex]);
                  return;
                }
              }

              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                send();
              }
            }}
            placeholder={t("composer.placeholder")}
            aria-label={t("composer.messageLabel")}
            aria-autocomplete="list"
            aria-controls={slashMenuOpen ? "slash-command-menu" : undefined}
            aria-expanded={slashMenuOpen}
            aria-activedescendant={
              slashMenuOpen && slashCommands[activeSlashIndex]
                ? `slash-command-${slashCommands[activeSlashIndex].id}`
                : undefined
            }
            className="app-scrollbar block max-h-[154px] min-h-[34px] w-full resize-none overflow-y-auto bg-transparent px-1 text-[14px] leading-[22px] text-[var(--text)] outline-none placeholder:text-[var(--muted)] focus-visible:outline-none"
          />

          <div className="mt-2 flex min-w-0 items-center gap-1.5">
            <DropdownMenu.Root>
              <DropdownMenu.Trigger asChild>
                <IconButton
                  label={t("composer.addContext")}
                  className="size-7 rounded-full border border-[var(--border-soft)] bg-[var(--panel)]"
                  onPointerDown={() => setSlashDismissed(true)}
                >
                  <Plus size={13} />
                </IconButton>
              </DropdownMenu.Trigger>
              <DropdownMenu.Portal>
                <DropdownMenu.Content
                  side="top"
                  align="start"
                  sideOffset={8}
                  className="glass-menu z-[90] w-52 rounded-xl p-1.5"
                >
                  <DropdownMenu.Item onSelect={addProjectFolder} className="outline-none">
                    <MenuItem icon={<FolderPlus size={14} />} label={t("composer.addProjectFolder")} />
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
            ) : (
              <DropdownMenu.Root>
                <DropdownMenu.Trigger asChild>
                  <button
                    type="button"
                    data-model-settings-trigger
                    onPointerDown={() => setSlashDismissed(true)}
                    className="hidden h-7 min-w-0 max-w-[180px] items-center gap-1 rounded-lg px-2 text-[11px] leading-4 text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)] sm:flex"
                  >
                    <span className="min-w-0 max-w-[148px] truncate">
                      {runtimeMode
                        ? selectedRuntimeModel?.label ?? t("composer.selectModel")
                        : model === "local"
                          ? t("composer.localModel")
                          : model}
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
                    {runtimeMode
                      ? runtimeModelOptions.map((candidate) => (
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
                      : MODEL_OPTIONS.map((candidate) => (
                          <DropdownMenu.Item
                            key={candidate}
                            onSelect={() => setModel(candidate)}
                            className="flex h-8 cursor-default items-center gap-2 rounded-lg px-2 text-[12px] text-[var(--text)] outline-none data-[highlighted]:bg-[var(--surface-hover)]"
                          >
                            <span className="flex size-4 items-center justify-center text-[var(--accent)]">
                              {model === candidate ? <Check size={12} /> : null}
                            </span>
                            {candidate === "local" ? t("composer.localModel") : candidate}
                          </DropdownMenu.Item>
                        ))}
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu.Root>
            )}

            <button
              type="button"
              aria-label={canStop ? t("composer.stopRun") : t("composer.sendMessage")}
              onClick={runPrimaryAction}
              disabled={!canStop && (submitBlocked || !draft.trim())}
              className="flex size-8 shrink-0 items-center justify-center rounded-full bg-[var(--text)] text-[var(--canvas)] transition-transform hover:scale-[1.03] disabled:bg-[var(--border)] disabled:text-[var(--muted)]"
            >
              {canStop ? (
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
              {t("composer.addProjectFolderFailed")}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
