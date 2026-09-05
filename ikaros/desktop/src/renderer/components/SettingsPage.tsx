import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import {
  ArrowLeft,
  Archive,
  BookOpen,
  Brain,
  Cable,
  Check,
  ChevronDown,
  CircleUserRound,
  Palette,
  Search,
  Sparkles,
  Settings
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";

import {
  cloneUiPreferences,
  DEFAULT_UI_PREFERENCES,
  mergeUiPreferences,
  type ColorSchemePreference,
  type ThemePreferencesPatch,
  type UiLanguagePreference,
  type UiPreferences,
  type UiPreferencesPatch
} from "../../shared/platform";
import { effectiveColorScheme } from "../../shared/theme";
import {
  applyDocumentPreferences,
  prefersDarkColorScheme,
  themeTransitionCoordinator,
  type ThemeTransitionOrigin
} from "../applyUiPreferences";
import { useTranslation } from "../i18n";
import { useAppStore } from "../store";
import { AppearanceSettings } from "./AppearanceSettings";
import { ArchivedThreadsDialog } from "./ArchivedThreadsDialog";
import { MemorySettings } from "./MemorySettings";
import { ProfileSettings } from "./ProfileSettings";
import {
  ModelsSettings,
  ProvidersSettings
} from "./ProviderModelSettings";
import { SkillsSettings } from "./SkillsSettings";
import { cx } from "./ui";

const LANGUAGE_OPTIONS: readonly UiLanguagePreference[] = ["en", "zh-CN"];
type SettingsSection =
  | "general"
  | "profile"
  | "appearance"
  | "providers"
  | "models"
  | "skills"
  | "memory";

function GeneralSettings({
  preferences,
  onLanguageChange
}: {
  preferences: Readonly<UiPreferences>;
  onLanguageChange(value: UiLanguagePreference): void;
}) {
  const { t } = useTranslation();
  const [archivedOpen, setArchivedOpen] = useState(false);
  const languageLabel = (value: UiLanguagePreference) =>
    value === "en" ? t("settings.english") : t("settings.simplifiedChinese");

  return (
    <div>
      <h1
        data-settings-heading
        tabIndex={-1}
        className="text-[20px] font-semibold leading-[28px] tracking-[-0.025em] text-[var(--text)] outline-none"
      >
        {t("settings.general")}
      </h1>

      <section aria-labelledby="general-settings-heading" className="mt-10">
        <h2
          id="general-settings-heading"
          className="mb-3 text-[13px] font-semibold leading-5 text-[var(--text)]"
        >
          {t("settings.general")}
        </h2>
        <div className="overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)] px-4">
          <div className="flex min-h-[68px] items-center gap-5 py-3">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold leading-5 text-[var(--text)]">
                {t("settings.language")}
              </div>
              <div className="mt-0.5 text-[11px] leading-4 text-[var(--muted)]">
                {t("settings.languageDescription")}
              </div>
            </div>

            <DropdownMenu.Root>
              <DropdownMenu.Trigger asChild>
                <button
                  type="button"
                  aria-label={t("settings.languageMenu")}
                  className="flex h-8 min-w-[126px] shrink-0 items-center justify-between gap-2 rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 text-[12px] font-medium leading-[18px] text-[var(--text)] outline-none transition-colors hover:bg-[var(--panel-hover)] data-[state=open]:bg-[var(--panel-selected)]"
                >
                  <span>{languageLabel(preferences.language)}</span>
                  <ChevronDown size={14} className="text-[var(--muted)]" aria-hidden="true" />
                </button>
              </DropdownMenu.Trigger>
              <DropdownMenu.Portal>
                <DropdownMenu.Content
                  side="bottom"
                  align="end"
                  sideOffset={6}
                  className="glass-menu z-[120] min-w-[170px] rounded-xl p-1.5"
                >
                  <DropdownMenu.RadioGroup
                    value={preferences.language}
                    onValueChange={(value) => onLanguageChange(value as UiLanguagePreference)}
                  >
                    {LANGUAGE_OPTIONS.map((option) => (
                      <DropdownMenu.RadioItem
                        key={option}
                        value={option}
                        className="flex h-9 cursor-default select-none items-center gap-2 rounded-lg px-2.5 text-[12px] leading-[18px] text-[var(--text)] outline-none data-[highlighted]:bg-[var(--surface-hover)]"
                      >
                        <span className="flex size-4 items-center justify-center">
                          <DropdownMenu.ItemIndicator>
                            <Check size={14} aria-hidden="true" className="text-[var(--accent)]" />
                          </DropdownMenu.ItemIndicator>
                        </span>
                        <span>{languageLabel(option)}</span>
                      </DropdownMenu.RadioItem>
                    ))}
                  </DropdownMenu.RadioGroup>
                </DropdownMenu.Content>
              </DropdownMenu.Portal>
            </DropdownMenu.Root>
          </div>
          <div className="flex min-h-[68px] items-center gap-5 border-t border-[var(--border-soft)] py-3">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-[10px] bg-[var(--panel-raised)] text-[var(--muted-strong)]">
              <Archive size={14} aria-hidden="true" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold leading-5 text-[var(--text)]">
                {t("settings.archivedChats")}
              </div>
              <div className="mt-0.5 text-[11px] leading-4 text-[var(--muted)]">
                {t("settings.archivedChatsDescription")}
              </div>
            </div>
            <button
              type="button"
              onClick={() => setArchivedOpen(true)}
              className="h-8 shrink-0 rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 text-[12px] font-medium leading-[18px] text-[var(--text)] outline-none transition-colors hover:bg-[var(--panel-hover)]"
            >
              {t("settings.manage")}
            </button>
          </div>
        </div>
      </section>
      <ArchivedThreadsDialog open={archivedOpen} onOpenChange={setArchivedOpen} />
    </div>
  );
}

export function SettingsPage() {
  const { t } = useTranslation();
  const setSettingsOpen = useAppStore((state) => state.setSettingsOpen);
  const initialSection = useAppStore((state) => state.settingsInitialSection);
  const providers = useAppStore((state) => state.providers);
  const models = useAppStore((state) => state.models);
  const providerCatalogStatus = useAppStore((state) => state.providerCatalogStatus);
  const loadProviderCatalog = useAppStore((state) => state.loadProviderCatalog);
  const discoverDeepSeekModels = useAppStore((state) => state.discoverDeepSeekModels);
  const configureProvider = useAppStore((state) => state.configureProvider);
  const disconnectProvider = useAppStore((state) => state.disconnectProvider);
  const removeProvider = useAppStore((state) => state.removeProvider);
  const setModelEnabled = useAppStore((state) => state.setModelEnabled);
  const skillCatalogStatus = useAppStore((state) => state.skillCatalogStatus);
  const skillCatalogError = useAppStore((state) => state.skillCatalogError);
  const skills = useAppStore((state) => state.skills);
  const skillDiagnostics = useAppStore((state) => state.skillDiagnostics);
  const loadSkillCatalog = useAppStore((state) => state.loadSkillCatalog);
  const setSkillEnabled = useAppStore((state) => state.setSkillEnabled);
  const projects = useAppStore((state) => state.projects);
  const threads = useAppStore((state) => state.threads);
  const selectedThreadId = useAppStore((state) => state.selectedThreadId);
  const newThreadWorkspace = useAppStore((state) => state.newThreadWorkspace);
  const [activeSection, setActiveSection] = useState<SettingsSection>(initialSection);
  const [query, setQuery] = useState("");
  const [preferences, setPreferences] = useState<UiPreferences>(() =>
    cloneUiPreferences(DEFAULT_UI_PREFERENCES)
  );
  const [systemPrefersDark, setSystemPrefersDark] = useState(prefersDarkColorScheme);
  const [savingCount, setSavingCount] = useState(0);
  const [saveFailed, setSaveFailed] = useState(false);
  const preferencesRef = useRef(preferences);
  const persistedPreferencesRef = useRef(cloneUiPreferences(DEFAULT_UI_PREFERENCES));
  const saveRevisionRef = useRef(0);

  useEffect(() => {
    void loadProviderCatalog().catch(() => undefined);
  }, [loadProviderCatalog]);

  useEffect(() => {
    if (activeSection !== "skills") return;
    void loadSkillCatalog().catch(() => undefined);
  }, [activeSection, loadSkillCatalog]);

  const applyLocalPreferences = (
    nextPreferences: UiPreferences,
    transitionOrigin?: ThemeTransitionOrigin
  ) => {
    if (transitionOrigin) {
      themeTransitionCoordinator.apply(
        () => {
          preferencesRef.current = nextPreferences;
          flushSync(() => setPreferences(nextPreferences));
          applyDocumentPreferences(nextPreferences, systemPrefersDark);
        },
        {
          nextScheme: effectiveColorScheme(nextPreferences.colorScheme, systemPrefersDark),
          reduceMotion: nextPreferences.reduceMotion,
          origin: transitionOrigin
        }
      );
      return;
    }

    preferencesRef.current = nextPreferences;
    setPreferences(nextPreferences);
    applyDocumentPreferences(nextPreferences, systemPrefersDark);
  };

  useEffect(() => {
    const api = window.ikarosDesktop;
    if (!api) return;

    let disposed = false;
    void api.preferences
      .get()
      .then((storedPreferences) => {
        if (disposed) return;
        const nextPreferences = cloneUiPreferences(storedPreferences);
        persistedPreferencesRef.current = cloneUiPreferences(nextPreferences);
        applyLocalPreferences(nextPreferences);
      })
      .catch(() => {
        if (!disposed) setSaveFailed(true);
      });

    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const queryList = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => {
      setSystemPrefersDark(queryList.matches);
    };
    queryList.addEventListener("change", onChange);
    return () => queryList.removeEventListener("change", onChange);
  }, []);

  useEffect(() => {
    document.querySelector<HTMLElement>("[data-settings-heading]")?.focus();
  }, [activeSection]);

  const persistPatch = (patch: UiPreferencesPatch, transitionOrigin?: ThemeTransitionOrigin) => {
    const nextPreferences = mergeUiPreferences(preferencesRef.current, patch);
    applyLocalPreferences(nextPreferences, transitionOrigin);
    setSaveFailed(false);

    const api = window.ikarosDesktop;
    if (!api) {
      setSaveFailed(true);
      applyLocalPreferences(cloneUiPreferences(persistedPreferencesRef.current), transitionOrigin);
      return;
    }

    const revision = ++saveRevisionRef.current;
    setSavingCount((count) => count + 1);
    void api.preferences
      .update(patch)
      .then((storedPreferences) => {
        const confirmed = cloneUiPreferences(storedPreferences);
        persistedPreferencesRef.current = confirmed;
        if (revision === saveRevisionRef.current) {
          applyLocalPreferences(cloneUiPreferences(confirmed));
        }
      })
      .catch(() => {
        if (revision === saveRevisionRef.current) {
          setSaveFailed(true);
          applyLocalPreferences(
            cloneUiPreferences(persistedPreferencesRef.current),
            transitionOrigin
          );
        }
      })
      .finally(() => setSavingCount((count) => Math.max(0, count - 1)));
  };

  const previewTheme = (scheme: "light" | "dark", patch: ThemePreferencesPatch) => {
    const preferencePatch: UiPreferencesPatch =
      scheme === "dark" ? { darkTheme: patch } : { lightTheme: patch };
    applyLocalPreferences(mergeUiPreferences(preferencesRef.current, preferencePatch));
  };

  const persistTheme = (scheme: "light" | "dark") => {
    const current = preferencesRef.current;
    persistPatch(
      scheme === "dark"
        ? { darkTheme: { ...current.darkTheme } }
        : { lightTheme: { ...current.lightTheme } }
    );
  };

  const closeSettings = () => {
    setSettingsOpen(false);
    window.requestAnimationFrame(() => {
      const selector = initialSection === "general"
        ? "[data-profile-menu-trigger]"
        : "[data-model-settings-trigger]";
      document.querySelector<HTMLButtonElement>(selector)?.focus();
    });
  };

  const navigation = useMemo(
    () => [
      { id: "general" as const, label: t("settings.general"), Icon: Settings },
      { id: "profile" as const, label: t("settings.profile"), Icon: CircleUserRound },
      { id: "appearance" as const, label: t("settings.appearance"), Icon: Palette },
      { id: "providers" as const, label: t("settings.providers.title"), Icon: Cable },
      { id: "models" as const, label: t("settings.models.title"), Icon: Sparkles },
      { id: "skills" as const, label: t("settings.skills.title"), Icon: BookOpen },
      { id: "memory" as const, label: t("settings.memory.title"), Icon: Brain }
    ],
    [t]
  );
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleNavigation = normalizedQuery
    ? navigation.filter((item) => item.label.toLocaleLowerCase().includes(normalizedQuery))
    : navigation;
  const memoryWorkspaces = useMemo(
    () =>
      projects.map((project) => ({
        id: project.id,
        name: project.name
      })),
    [projects]
  );
  const preferredMemoryWorkspaceId =
    threads.find((thread) => thread.id === selectedThreadId)?.projectId ??
    newThreadWorkspace?.id ??
    null;

  return (
    <div className="relative flex min-h-0 flex-1 overflow-hidden bg-[var(--canvas)]">
      <aside
        aria-label={t("settings.navigation")}
        className="settings-sidebar flex w-[clamp(248px,14.3vw,274px)] shrink-0 flex-col border-r border-[var(--separator)] bg-[var(--sidebar)] px-2 py-1"
      >
        <button
          type="button"
          onClick={closeSettings}
          className="flex h-10 w-full items-center gap-2 rounded-lg px-2.5 text-left text-[12px] leading-[18px] text-[var(--muted)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
        >
          <ArrowLeft size={15} aria-hidden="true" />
          <span>{t("settings.backToApp")}</span>
        </button>

        <label className="relative mt-0.5 block">
          <span className="sr-only">{t("settings.searchPlaceholder")}</span>
          <Search
            size={13}
            aria-hidden="true"
            className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--muted)]"
          />
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder={t("settings.searchPlaceholder")}
            className="h-[29px] w-full rounded-[10px] border border-[var(--border-soft)] bg-[var(--panel)] pl-8 pr-2.5 text-[12px] leading-[18px] text-[var(--text)] placeholder:text-[var(--muted)]"
          />
        </label>

        <div className="mt-3 px-2 pb-2 text-[12px] font-semibold leading-[18px] text-[var(--muted)]">
          {t("settings.personal")}
        </div>
        <nav className="space-y-0.5">
          {visibleNavigation.map(({ id, label, Icon }) => (
            <button
              key={id}
              type="button"
              aria-current={activeSection === id ? "page" : undefined}
              onClick={() => setActiveSection(id)}
              className={cx(
                "flex h-[31px] w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[12px] font-medium leading-[18px] transition-colors",
                activeSection === id
                  ? "bg-[var(--panel-selected)] text-[var(--text)] shadow-[inset_0_0_0_1px_var(--border-soft)]"
                  : "text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
              )}
            >
              <Icon size={14} aria-hidden="true" />
              <span>{label}</span>
            </button>
          ))}
        </nav>
      </aside>

      <main className="app-scrollbar min-w-0 flex-1 overflow-y-auto">
        <div aria-hidden="true" className="h-11 border-b border-[var(--separator)]" />
        <div
          className={cx(
            "w-full px-5 pb-20",
            activeSection === "profile"
              ? "pt-4"
              : "mx-auto max-w-[808px] pt-[clamp(48px,6vh,64px)]"
          )}
        >
          {activeSection === "general" ? (
            <GeneralSettings
              preferences={preferences}
              onLanguageChange={(language) => persistPatch({ language })}
            />
          ) : activeSection === "profile" ? (
            <ProfileSettings />
          ) : activeSection === "appearance" ? (
            <AppearanceSettings
              preferences={preferences}
              systemPrefersDark={systemPrefersDark}
              saving={savingCount > 0}
              onColorSchemeChange={(
                colorScheme: ColorSchemePreference,
                origin: ThemeTransitionOrigin
              ) =>
                persistPatch({ colorScheme }, origin)
              }
              onPreviewTheme={previewTheme}
              onPersistTheme={persistTheme}
              onReduceMotionChange={(reduceMotion) => persistPatch({ reduceMotion })}
            />
          ) : activeSection === "providers" ? (
            <ProvidersSettings
              providers={providers}
              models={models}
              onDiscoverDeepSeekModels={discoverDeepSeekModels}
              onConfigureProvider={configureProvider}
              onDisconnectDeepSeek={() => disconnectProvider("deepseek")}
              onRemoveCustomProvider={removeProvider}
            />
          ) : activeSection === "models" ? (
            <ModelsSettings
              providers={providers}
              models={models}
              onSetModelEnabled={(providerId, modelId, enabled) =>
                setModelEnabled({ providerId, modelId, enabled })
              }
            />
          ) : activeSection === "skills" ? (
            <SkillsSettings
              skills={skills}
              diagnostics={skillDiagnostics}
              status={skillCatalogStatus}
              catalogError={skillCatalogError}
              onRefresh={loadSkillCatalog}
              onSetEnabled={(name, enabled) => setSkillEnabled({ name, enabled })}
            />
          ) : (
            <MemorySettings
              workspaces={memoryWorkspaces}
              preferredWorkspaceId={preferredMemoryWorkspaceId}
            />
          )}
          <p aria-live="polite" className="mt-2 min-h-4 px-1 text-[11px] leading-4 text-[#e08b8b]">
            {(activeSection === "providers" || activeSection === "models") &&
            providerCatalogStatus === "error"
              ? t("settings.providers.catalogFailed")
              : saveFailed
                ? t("settings.error")
                : ""}
          </p>
        </div>
      </main>
    </div>
  );
}
