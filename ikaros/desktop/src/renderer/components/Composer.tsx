import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import {
  ArrowUp,
  Check,
  ChevronDown,
  FolderPlus,
  Hand,
  Paperclip,
  Plus,
  ShieldAlert,
  ShieldCheck,
  Wrench,
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

const ACCESS_OPTIONS = [
  {
    id: "ask",
    labelKey: "composer.access.ask.label",
    shortLabelKey: "composer.access.ask.shortLabel",
    descriptionKey: "composer.access.ask.description",
  },
  {
    id: "safe",
    labelKey: "composer.access.safe.label",
    shortLabelKey: "composer.access.safe.shortLabel",
    descriptionKey: "composer.access.safe.description",
  },
  {
    id: "full",
    labelKey: "composer.access.full.label",
    shortLabelKey: "composer.access.full.shortLabel",
    descriptionKey: "composer.access.full.description",
  },
] as const satisfies ReadonlyArray<{
  id: string;
  labelKey: TranslationKey;
  shortLabelKey: TranslationKey;
  descriptionKey: TranslationKey;
}>;

const MODEL_OPTIONS = ["Ikaros", "DeepSeek", "local"] as const;

type AccessMode = (typeof ACCESS_OPTIONS)[number]["id"];
type ModelId = (typeof MODEL_OPTIONS)[number];

function AccessIcon({ mode, size = 11 }: { mode: AccessMode; size?: number }) {
  if (mode === "ask") return <Hand size={size} />;
  if (mode === "full") return <ShieldAlert size={size} />;
  return <ShieldCheck size={size} />;
}

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
  const selectModel = useAppStore((state) => state.selectModel);
  const setDraft = useAppStore((state) => state.setDraft);
  const sendDraft = useAppStore((state) => state.sendDraft);
  const stopRun = useAppStore((state) => state.stopRun);
  const bindWorkspaceFromFolder = useAppStore((state) => state.bindWorkspaceFromFolder);
  const [accessMode, setAccessMode] = useState<AccessMode>("full");
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
  const submitBlocked = (isRunActive(runStatus) && !canStop) || runtimeModelMissing;
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
  const access = ACCESS_OPTIONS.find((option) => option.id === accessMode) ?? ACCESS_OPTIONS[0];

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
    if (isRunActive(runStatus) || runtimeModelMissing) return;
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
                  <DropdownMenu.Item disabled className="outline-none">
                    <MenuItem icon={<Paperclip size={14} />} label={t("composer.attachFile")} detail={t("common.unavailable")} disabled />
                  </DropdownMenu.Item>
                  <DropdownMenu.Item onSelect={addProjectFolder} className="outline-none">
                    <MenuItem icon={<FolderPlus size={14} />} label={t("composer.addProjectFolder")} />
                  </DropdownMenu.Item>
                  <DropdownMenu.Item disabled className="outline-none">
                    <MenuItem icon={<Wrench size={14} />} label={t("composer.chooseTools")} detail={t("common.unavailable")} disabled />
                  </DropdownMenu.Item>
                </DropdownMenu.Content>
              </DropdownMenu.Portal>
            </DropdownMenu.Root>

            <DropdownMenu.Root>
              <DropdownMenu.Trigger asChild>
                <button
                  type="button"
                  aria-label={t(access.shortLabelKey)}
                  disabled={runtimeMode}
                  onPointerDown={() => setSlashDismissed(true)}
                  className={
                    accessMode === "full"
                      ? "flex h-7 items-center gap-1 rounded-full bg-[#4a2b1f] px-2 text-[10px] leading-4 text-[#ff9a67] hover:bg-[#553225]"
                      : "flex h-7 items-center gap-1 rounded-full px-2 text-[10px] leading-4 text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
                  }
                >
                  <AccessIcon mode={accessMode} />
                  <span className="hidden sm:inline">{t(access.shortLabelKey)}</span>
                  <ChevronDown size={9} className="text-[var(--muted)]" />
                </button>
              </DropdownMenu.Trigger>
              <DropdownMenu.Portal>
                <DropdownMenu.Content
                  side="top"
                  align="start"
                  sideOffset={8}
                  className="glass-menu z-[90] w-[360px] rounded-2xl p-1.5"
                >
                  <div className="px-2.5 pb-1.5 pt-1 text-[11px] text-[var(--muted)]">
                    {t("composer.accessPrompt")}
                  </div>
                  {ACCESS_OPTIONS.map((option) => (
                    <DropdownMenu.Item
                      key={option.id}
                      onSelect={() => setAccessMode(option.id)}
                      className="flex cursor-default items-start gap-2.5 rounded-xl px-2.5 py-2 outline-none data-[highlighted]:bg-[var(--surface-hover)]"
                    >
                      <span
                        className={
                          option.id === "full"
                            ? "mt-0.5 flex size-5 items-center justify-center text-[#ff8d52]"
                            : "mt-0.5 flex size-5 items-center justify-center text-[var(--muted-strong)]"
                        }
                      >
                        <AccessIcon mode={option.id} size={14} />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span
                          className={
                            option.id === "full"
                              ? "block text-[12px] font-medium text-[#ff9a67]"
                              : "block text-[12px] font-medium text-[var(--text)]"
                          }
                        >
                          {t(option.labelKey)}
                        </span>
                        <span className="mt-0.5 block text-[11px] leading-4 text-[var(--muted)]">
                          {t(option.descriptionKey)}
                        </span>
                      </span>
                      <span className="mt-1 flex size-4 items-center justify-center text-[var(--accent)]">
                        {accessMode === option.id ? <Check size={12} /> : null}
                      </span>
                    </DropdownMenu.Item>
                  ))}
                </DropdownMenu.Content>
              </DropdownMenu.Portal>
            </DropdownMenu.Root>

            <div className="min-w-0 flex-1" />

            <DropdownMenu.Root>
              <DropdownMenu.Trigger asChild>
                <button
                  type="button"
                  disabled={runtimeMode && runtimeModelOptions.length === 0}
                  onPointerDown={() => setSlashDismissed(true)}
                  className="hidden h-8 min-w-0 max-w-[180px] items-center gap-1 rounded-lg px-2 text-[10px] leading-4 text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)] sm:flex"
                >
                  <span className="min-w-0 max-w-[148px] truncate">
                    {runtimeMode
                      ? selectedRuntimeModel?.label ??
                        (runtimeModelOptions.length === 0
                          ? t("composer.configureModel")
                          : t("composer.selectModel"))
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
                          className="flex h-8 cursor-default items-center gap-2 rounded-lg px-2 text-[11px] leading-4 text-[var(--text)] outline-none data-[highlighted]:bg-[var(--surface-hover)]"
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
