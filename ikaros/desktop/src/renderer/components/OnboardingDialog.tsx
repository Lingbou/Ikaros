import * as Dialog from "@radix-ui/react-dialog";
import { ArrowLeft, CheckCircle2 } from "lucide-react";
import { useEffect, useState } from "react";

import {
  MAX_PROFILE_USERNAME_LENGTH,
  type ColorSchemePreference,
  type UiLanguagePreference,
} from "../../shared/platform";
import { useTranslation } from "../i18n";
import { useAppStore } from "../store";
import { CustomProviderForm, DeepSeekConnectForm } from "./ProviderModelSettings";

type ProviderMode = "deepseek" | "custom";
type OnboardingPhase = "checking" | "show" | "hidden";
type OnboardingStep = 1 | 2 | 3;

const LANGUAGE_OPTIONS: readonly UiLanguagePreference[] = ["zh-CN", "en"];
const THEME_OPTIONS: readonly ColorSchemePreference[] = ["system", "light", "dark"];

const STEP_TITLES = {
  1: "onboarding.title",
  2: "onboarding.providerTitle",
  3: "onboarding.connectedTitle",
} as const;
const STEP_DESCRIPTIONS = {
  1: "onboarding.description",
  2: "onboarding.providerDescription",
  3: "onboarding.connectedDescription",
} as const;

const INPUT_CLASS =
  "h-9 w-full rounded-lg border border-[var(--border-soft)] bg-[var(--panel)] px-3 text-[12px] leading-5 text-[var(--text)] outline-none transition-colors placeholder:text-[var(--muted)] focus-visible:border-[var(--muted-strong)]";
const OPTION_GROUP_CLASS =
  "flex gap-1 rounded-xl border border-[var(--border-soft)] bg-[var(--panel)] p-1";
const OPTION_CLASS = "h-8 flex-1 rounded-lg text-[12px] font-medium transition-colors";
const OPTION_ACTIVE_CLASS = "bg-[var(--surface-hover)] text-[var(--text)]";
const OPTION_IDLE_CLASS = "text-[var(--muted)] hover:text-[var(--text)]";
const PRIMARY_BUTTON_CLASS =
  "h-9 rounded-xl bg-[var(--text)] px-5 text-[12px] font-medium text-[var(--canvas)] transition-transform hover:scale-[1.02] disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:scale-100";
const QUIET_BUTTON_CLASS =
  "text-[11px] leading-4 text-[var(--muted)] transition-colors hover:text-[var(--text)]";

/** First-run setup: profile, provider connection, and a summary before the first chat. */
export function OnboardingDialog() {
  const { t } = useTranslation();
  const connectionStatus = useAppStore((state) => state.runtimeConnectionStatus);
  const providerCatalogStatus = useAppStore((state) => state.providerCatalogStatus);
  const providers = useAppStore((state) => state.providers);
  const models = useAppStore((state) => state.models);
  const configureProvider = useAppStore((state) => state.configureProvider);
  const discoverDeepSeekModels = useAppStore((state) => state.discoverDeepSeekModels);
  const loadProviderCatalog = useAppStore((state) => state.loadProviderCatalog);
  const [completed, setCompleted] = useState<boolean | null>(null);
  const [phase, setPhase] = useState<OnboardingPhase>("checking");
  const [step, setStep] = useState<OnboardingStep>(1);
  const [mode, setMode] = useState<ProviderMode>("deepseek");
  const [nickname, setNickname] = useState("");
  const [language, setLanguage] = useState<UiLanguagePreference>("en");
  const [colorScheme, setColorScheme] = useState<ColorSchemePreference>("dark");
  const [nicknameTouched, setNicknameTouched] = useState(false);

  useEffect(() => {
    const api = window.ikarosDesktop;
    if (!api) {
      setCompleted(true);
      return;
    }
    let disposed = false;
    void api.preferences
      .get()
      .then((preferences) => {
        if (disposed) return;
        setCompleted(preferences.onboardingCompleted);
        setNickname(preferences.username);
        setLanguage(preferences.language);
        setColorScheme(preferences.colorScheme);
      })
      .catch(() => {
        if (!disposed) setCompleted(true);
      });
    return () => {
      disposed = true;
    };
  }, []);

  const ready = connectionStatus === "connected" && providerCatalogStatus === "ready";
  const hasConfiguredProvider = providers.some((provider) => provider.configured);

  useEffect(() => {
    if (phase !== "checking" || completed === null || !ready) return;
    if (completed === false && !hasConfiguredProvider) {
      setPhase("show");
      return;
    }
    if (completed === false) {
      // A provider was configured elsewhere (or before onboarding existed):
      // remember that setup is done and stay out of the way.
      void window.ikarosDesktop?.preferences
        .update({ onboardingCompleted: true })
        .catch(() => undefined);
    }
    setPhase("hidden");
  }, [phase, completed, ready, hasConfiguredProvider]);

  const updatePreferences = (patch: Parameters<NonNullable<Window["ikarosDesktop"]>["preferences"]["update"]>[0]) => {
    void window.ikarosDesktop?.preferences.update(patch).catch(() => undefined);
  };

  const trimmedNickname = nickname.trim();
  const nicknameValid =
    trimmedNickname.length > 0 && trimmedNickname.length <= MAX_PROFILE_USERNAME_LENGTH;

  const finish = () => {
    setPhase("hidden");
    updatePreferences({ onboardingCompleted: true });
  };

  const continueFromProfile = () => {
    setNicknameTouched(true);
    if (!nicknameValid) return;
    updatePreferences({ username: trimmedNickname });
    setStep(2);
  };

  const chooseLanguage = (next: UiLanguagePreference) => {
    setLanguage(next);
    updatePreferences({ language: next });
  };

  const chooseColorScheme = (next: ColorSchemePreference) => {
    setColorScheme(next);
    updatePreferences({ colorScheme: next });
  };

  const connectedHandler = () => {
    void loadProviderCatalog();
    setStep(3);
  };

  if (phase !== "show") return null;

  const configuredProvider = providers.find((provider) => provider.configured);
  const providerLabel = configuredProvider
    ? configuredProvider.id === "deepseek"
      ? t("settings.providers.deepSeekName")
      : configuredProvider.displayName
    : "—";
  const summaryRows = [
    { label: t("onboarding.summaryNickname"), value: trimmedNickname || "—" },
    {
      label: t("onboarding.summaryLanguage"),
      value: language === "zh-CN" ? t("onboarding.languageZh") : t("onboarding.languageEn"),
    },
    {
      label: t("onboarding.summaryTheme"),
      value:
        colorScheme === "system"
          ? t("settings.system")
          : colorScheme === "light"
            ? t("settings.light")
            : t("settings.dark"),
    },
    { label: t("onboarding.summaryProvider"), value: providerLabel },
    {
      label: t("onboarding.summaryModels"),
      value: String(models.filter((model) => model.enabled).length),
    },
  ];

  return (
    <Dialog.Root
      open
      onOpenChange={(next) => {
        if (!next) finish();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay
          data-testid="onboarding-overlay"
          className="fixed inset-0 z-[130] bg-black/55 backdrop-blur-[2px]"
        />
        <Dialog.Content
          aria-describedby="onboarding-description"
          className="glass-menu fixed left-1/2 top-1/2 z-[140] flex max-h-[calc(100vh-40px)] w-[min(640px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl"
        >
          <div className="shrink-0 border-b border-[var(--separator)] px-6 py-5">
            <div className="flex items-center gap-1.5">
              {([1, 2, 3] as const).map((candidate) => (
                <span
                  key={candidate}
                  aria-hidden="true"
                  className={
                    candidate <= step
                      ? "size-1.5 rounded-full bg-[var(--accent)]"
                      : "size-1.5 rounded-full bg-[var(--border-soft)]"
                  }
                />
              ))}
              <span className="ml-1.5 text-[11px] leading-4 text-[var(--muted)]">
                {t("onboarding.step", { current: step, total: 3 })}
              </span>
            </div>
            <Dialog.Title className="mt-3 text-[16px] font-semibold leading-6 text-[var(--text)]">
              {t(STEP_TITLES[step])}
            </Dialog.Title>
            <Dialog.Description
              id="onboarding-description"
              className="mt-1.5 text-[12px] leading-5 text-[var(--muted)]"
            >
              {t(STEP_DESCRIPTIONS[step])}
            </Dialog.Description>
          </div>

          <div className="app-scrollbar min-h-0 flex-1 overflow-y-auto px-6 py-5">
            {step === 1 ? (
              <div className="space-y-5">
                <label className="block">
                  <span className="mb-1.5 block text-[12px] font-medium leading-[18px] text-[var(--text)]">
                    {t("onboarding.nicknameLabel")}
                  </span>
                  <input
                    value={nickname}
                    autoFocus
                    maxLength={MAX_PROFILE_USERNAME_LENGTH * 2}
                    aria-label={t("onboarding.nicknameLabel")}
                    aria-invalid={nicknameTouched && !nicknameValid}
                    onChange={(event) => {
                      setNickname(event.currentTarget.value);
                      setNicknameTouched(true);
                    }}
                    className={INPUT_CLASS}
                  />
                  <span className="mt-1.5 block text-[11px] leading-4 text-[var(--muted)]">
                    {t("onboarding.nicknameHint")}
                  </span>
                  {nicknameTouched && !nicknameValid ? (
                    <span role="alert" className="mt-1 block text-[11px] leading-4 text-[#e08b8b]">
                      {t("onboarding.nicknameInvalid")}
                    </span>
                  ) : null}
                </label>

                <div>
                  <span className="mb-1.5 block text-[12px] font-medium leading-[18px] text-[var(--text)]">
                    {t("onboarding.languageLabel")}
                  </span>
                  <div className={OPTION_GROUP_CLASS}>
                    {LANGUAGE_OPTIONS.map((candidate) => (
                      <button
                        key={candidate}
                        type="button"
                        aria-pressed={language === candidate}
                        onClick={() => chooseLanguage(candidate)}
                        className={
                          language === candidate
                            ? OPTION_CLASS + " " + OPTION_ACTIVE_CLASS
                            : OPTION_CLASS + " " + OPTION_IDLE_CLASS
                        }
                      >
                        {t(candidate === "zh-CN" ? "onboarding.languageZh" : "onboarding.languageEn")}
                      </button>
                    ))}
                  </div>
                </div>

                <div>
                  <span className="mb-1.5 block text-[12px] font-medium leading-[18px] text-[var(--text)]">
                    {t("onboarding.themeLabel")}
                  </span>
                  <div className={OPTION_GROUP_CLASS}>
                    {THEME_OPTIONS.map((candidate) => (
                      <button
                        key={candidate}
                        type="button"
                        aria-pressed={colorScheme === candidate}
                        onClick={() => chooseColorScheme(candidate)}
                        className={
                          colorScheme === candidate
                            ? OPTION_CLASS + " " + OPTION_ACTIVE_CLASS
                            : OPTION_CLASS + " " + OPTION_IDLE_CLASS
                        }
                      >
                        {t(
                          candidate === "system"
                            ? "settings.system"
                            : candidate === "light"
                              ? "settings.light"
                              : "settings.dark",
                        )}
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            ) : step === 2 ? (
              <div>
                <div
                  role="tablist"
                  aria-label={t("onboarding.providerTitle")}
                  className="mb-5 flex gap-1 rounded-xl border border-[var(--border-soft)] bg-[var(--panel)] p-1"
                >
                  {(["deepseek", "custom"] as const).map((candidate) => (
                    <button
                      key={candidate}
                      type="button"
                      role="tab"
                      aria-selected={mode === candidate}
                      onClick={() => setMode(candidate)}
                      className={
                        mode === candidate
                          ? OPTION_CLASS + " " + OPTION_ACTIVE_CLASS
                          : OPTION_CLASS + " " + OPTION_IDLE_CLASS
                      }
                    >
                      {t(candidate === "deepseek" ? "onboarding.deepSeekTab" : "onboarding.customTab")}
                    </button>
                  ))}
                </div>
                {mode === "deepseek" ? (
                  <DeepSeekConnectForm
                    existingModels={[]}
                    onConnected={connectedHandler}
                    onConnect={configureProvider}
                    onDiscoverModels={discoverDeepSeekModels}
                  />
                ) : (
                  <CustomProviderForm
                    onConfigured={connectedHandler}
                    onSubmit={configureProvider}
                  />
                )}
              </div>
            ) : (
              <div className="space-y-4">
                <div className="flex justify-center pt-1">
                  <CheckCircle2 size={30} aria-hidden="true" className="text-[var(--accent)]" />
                </div>
                <dl className="space-y-2 rounded-xl border border-[var(--border-soft)] bg-[var(--panel)] p-4">
                  {summaryRows.map((row) => (
                    <div
                      key={row.label}
                      className="flex items-center justify-between gap-3 text-[12px] leading-5"
                    >
                      <dt className="text-[var(--muted)]">{row.label}</dt>
                      <dd className="min-w-0 truncate text-[var(--text)]">{row.value}</dd>
                    </div>
                  ))}
                </dl>
              </div>
            )}
          </div>

          <div className="flex shrink-0 items-center justify-between gap-3 border-t border-[var(--separator)] px-6 py-3">
            {step === 2 ? (
              <button
                type="button"
                onClick={() => setStep(1)}
                className={QUIET_BUTTON_CLASS + " inline-flex items-center gap-1"}
              >
                <ArrowLeft size={12} aria-hidden="true" />
                {t("onboarding.back")}
              </button>
            ) : (
              <span />
            )}
            <div className="flex items-center gap-4">
              {step !== 3 ? (
                <button type="button" onClick={finish} className={QUIET_BUTTON_CLASS}>
                  {t("onboarding.skip")}
                </button>
              ) : null}
              {step === 1 ? (
                <button
                  type="button"
                  disabled={!nicknameValid}
                  onClick={continueFromProfile}
                  className={PRIMARY_BUTTON_CLASS}
                >
                  {t("onboarding.continue")}
                </button>
              ) : null}
              {step === 3 ? (
                <button type="button" autoFocus onClick={finish} className={PRIMARY_BUTTON_CLASS}>
                  {t("onboarding.start")}
                </button>
              ) : null}
            </div>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
