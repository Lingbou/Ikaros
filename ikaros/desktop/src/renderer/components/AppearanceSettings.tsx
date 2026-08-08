import { ChevronDown } from "lucide-react";
import { useEffect, useRef, type ReactNode } from "react";

import type {
  CodeFontPreference,
  ColorSchemePreference,
  ThemePreferences,
  ThemePreferencesPatch,
  UiFontPreference,
  UiPreferences
} from "../../shared/platform";
import { effectiveColorScheme, readableTextColor } from "../../shared/theme";
import type { ThemeTransitionOrigin } from "../applyUiPreferences";
import { useTranslation, type Translate } from "../i18n";
import { CodeDiffPreview, ThemeCard } from "./ThemePreview";
import { cx } from "./ui";

interface AppearanceSettingsProps {
  preferences: Readonly<UiPreferences>;
  systemPrefersDark: boolean;
  saving: boolean;
  onColorSchemeChange(value: ColorSchemePreference, origin: ThemeTransitionOrigin): void;
  onPreviewTheme(scheme: "light" | "dark", patch: ThemePreferencesPatch): void;
  onPersistTheme(scheme: "light" | "dark"): void;
  onReduceMotionChange(value: boolean): void;
}

const UI_FONT_OPTIONS: readonly UiFontPreference[] = ["system", "inter", "sego-ui"];
const CODE_FONT_OPTIONS: readonly CodeFontPreference[] = [
  "system-mono",
  "cascadia-code",
  "consolas"
];

function uiFontLabel(value: UiFontPreference, t: Translate): string {
  if (value === "inter") return t("settings.font.inter");
  if (value === "sego-ui") return t("settings.font.segoUi");
  return t("settings.font.system");
}

function codeFontLabel(value: CodeFontPreference, t: Translate): string {
  if (value === "cascadia-code") return t("settings.font.cascadiaCode");
  if (value === "consolas") return t("settings.font.consolas");
  return t("settings.font.systemMono");
}

function SettingsRow({
  label,
  children,
  className
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cx(
        "flex min-h-11 items-center justify-between gap-5 border-t border-[var(--separator)] px-4 py-2",
        className
      )}
    >
      <span className="text-[13px] font-semibold leading-5 text-[var(--text)]">{label}</span>
      {children}
    </div>
  );
}

function ColorControl({
  label,
  value,
  onChange,
  onCommit
}: {
  label: string;
  value: string;
  onChange(value: string): void;
  onCommit(): void;
}) {
  const foreground = readableTextColor(value);
  return (
    <label
      className="relative flex h-7 w-[137px] cursor-pointer items-center gap-2 overflow-hidden rounded-[9px] border border-[var(--border-soft)] px-2.5 text-[12px] font-semibold leading-[18px] shadow-sm"
      style={{ background: value, color: foreground }}
    >
      <input
        type="color"
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
        onBlur={onCommit}
        className="absolute inset-0 cursor-pointer opacity-0"
      />
      <span
        aria-hidden="true"
        className="size-3.5 shrink-0 rounded-full border"
        style={{ borderColor: foreground, opacity: 0.42 }}
      />
      <span className="relative z-10 uppercase">{value}</span>
    </label>
  );
}

function SelectControl({
  label,
  value,
  options,
  onChange
}: {
  label: string;
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange(value: string): void;
}) {
  return (
    <label className="relative block w-[137px]">
      <span className="sr-only">{label}</span>
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
        className="h-7 w-full appearance-none truncate rounded-[9px] border border-[var(--border)] bg-[var(--panel-raised)] pl-2.5 pr-8 text-[12px] leading-[18px] text-[var(--muted-strong)] outline-none hover:bg-[var(--panel-hover)]"
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <ChevronDown
        size={13}
        aria-hidden="true"
        className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-[var(--muted)]"
      />
    </label>
  );
}

function Switch({ label, checked, onChange }: { label: string; checked: boolean; onChange(): void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      onClick={onChange}
      className={cx(
        "relative h-5 w-8 rounded-full border transition-colors",
        checked
          ? "border-[var(--accent)] bg-[var(--accent)]"
          : "border-[var(--border)] bg-[var(--panel-raised)]"
      )}
    >
      <span
        aria-hidden="true"
        className={cx(
          "absolute left-0 top-[1px] size-4 rounded-full bg-white shadow-sm transition-transform",
          checked ? "translate-x-[14px]" : "translate-x-[1px]"
        )}
      />
    </button>
  );
}

export function AppearanceSettings({
  preferences,
  systemPrefersDark,
  saving,
  onColorSchemeChange,
  onPreviewTheme,
  onPersistTheme,
  onReduceMotionChange
}: AppearanceSettingsProps) {
  const { t } = useTranslation();
  const persistTimer = useRef<number | undefined>(undefined);
  const scheme = effectiveColorScheme(preferences.colorScheme, systemPrefersDark);
  const theme: Readonly<ThemePreferences> =
    scheme === "dark" ? preferences.darkTheme : preferences.lightTheme;

  const themeLabel = (value: ColorSchemePreference) => {
    if (value === "system") return t("settings.system");
    if (value === "light") return t("settings.light");
    return t("settings.dark");
  };

  const scheduleThemePersistence = () => {
    window.clearTimeout(persistTimer.current);
    persistTimer.current = window.setTimeout(() => onPersistTheme(scheme), 220);
  };

  const flushThemePersistence = () => {
    if (persistTimer.current === undefined) return;
    window.clearTimeout(persistTimer.current);
    persistTimer.current = undefined;
    onPersistTheme(scheme);
  };

  const previewTheme = (patch: ThemePreferencesPatch) => {
    onPreviewTheme(scheme, patch);
    scheduleThemePersistence();
  };

  useEffect(
    () => () => {
      window.clearTimeout(persistTimer.current);
    },
    []
  );

  return (
    <div>
      <h1
        data-settings-heading
        tabIndex={-1}
        className="text-[20px] font-semibold leading-[28px] tracking-[-0.025em] text-[var(--text)] outline-none"
      >
        {t("settings.appearance")}
      </h1>

      <section aria-labelledby="appearance-theme-heading" className="mt-10">
        <h2
          id="appearance-theme-heading"
          className="mb-4 text-[13px] font-semibold leading-5 text-[var(--text)]"
        >
          {t("settings.theme")}
        </h2>

        <div role="radiogroup" aria-label={t("settings.theme")} className="grid grid-cols-3 gap-3">
          {(["system", "light", "dark"] as const).map((value) => (
            <ThemeCard
              key={value}
              value={value}
              label={themeLabel(value)}
              selected={preferences.colorScheme === value}
              lightTheme={preferences.lightTheme}
              darkTheme={preferences.darkTheme}
              onSelect={onColorSchemeChange}
            />
          ))}
        </div>

        <CodeDiffPreview />

        <div className="mt-4 overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)]">
          <div className="flex h-[52px] items-center justify-between gap-4 px-4">
            <h2 className="text-[13px] font-semibold leading-5 text-[var(--text)]">
              {t("settings.themeName", { theme: themeLabel(scheme) })}
            </h2>
            <div className="flex h-7 w-44 items-center gap-2 rounded-[9px] border border-[var(--border-soft)] bg-[var(--canvas)] px-2.5 text-[12px] leading-[18px] text-[var(--text)]">
              <span className="flex size-[18px] items-center justify-center rounded-md border border-[var(--border)] text-[10px] leading-none text-[var(--accent)]">
                Aa
              </span>
              <span className="flex-1">Ikaros</span>
            </div>
          </div>

          <SettingsRow label={t("settings.accent")}>
            <ColorControl
              label={t("settings.chooseColor", { setting: t("settings.accent") })}
              value={theme.accent}
              onChange={(accent) => previewTheme({ accent })}
              onCommit={flushThemePersistence}
            />
          </SettingsRow>
          <SettingsRow label={t("settings.background")}>
            <ColorControl
              label={t("settings.chooseColor", { setting: t("settings.background") })}
              value={theme.background}
              onChange={(background) => previewTheme({ background })}
              onCommit={flushThemePersistence}
            />
          </SettingsRow>
          <SettingsRow label={t("settings.foreground")}>
            <ColorControl
              label={t("settings.chooseColor", { setting: t("settings.foreground") })}
              value={theme.foreground}
              onChange={(foreground) => previewTheme({ foreground })}
              onCommit={flushThemePersistence}
            />
          </SettingsRow>
          <SettingsRow label={t("settings.uiFont")}>
            <SelectControl
              label={t("settings.chooseUiFont")}
              value={theme.uiFont}
              options={UI_FONT_OPTIONS.map((value) => ({ value, label: uiFontLabel(value, t) }))}
              onChange={(value) => {
                onPreviewTheme(scheme, { uiFont: value as UiFontPreference });
                onPersistTheme(scheme);
              }}
            />
          </SettingsRow>
          <SettingsRow label={t("settings.codeFont")}>
            <SelectControl
              label={t("settings.chooseCodeFont")}
              value={theme.codeFont}
              options={CODE_FONT_OPTIONS.map((value) => ({ value, label: codeFontLabel(value, t) }))}
              onChange={(value) => {
                onPreviewTheme(scheme, { codeFont: value as CodeFontPreference });
                onPersistTheme(scheme);
              }}
            />
          </SettingsRow>
          <SettingsRow label={t("settings.translucentSidebar")}>
            <Switch
              label={t("settings.translucentSidebar")}
              checked={theme.translucentSidebar}
              onChange={() => {
                onPreviewTheme(scheme, { translucentSidebar: !theme.translucentSidebar });
                onPersistTheme(scheme);
              }}
            />
          </SettingsRow>
          <SettingsRow label={t("settings.contrast")}>
            <div className="flex w-[194px] items-center gap-3">
              <input
                type="range"
                aria-label={t("settings.contrast")}
                min="0"
                max="100"
                value={theme.contrast}
                onChange={(event) => previewTheme({ contrast: Number(event.currentTarget.value) })}
                onPointerUp={flushThemePersistence}
                onKeyUp={flushThemePersistence}
                onBlur={flushThemePersistence}
                className="h-5 min-w-0 flex-1 cursor-pointer accent-[var(--accent)]"
              />
              <output className="w-7 text-right text-[12px] leading-[18px] text-[var(--text)]">
                {theme.contrast}
              </output>
            </div>
          </SettingsRow>
        </div>
      </section>

      <section aria-labelledby="appearance-preferences-heading" className="mt-12">
        <h2
          id="appearance-preferences-heading"
          className="mb-3 text-[13px] font-semibold leading-5 text-[var(--text)]"
        >
          {t("settings.preferences")}
        </h2>
        <div className="overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)]">
          <div className="flex min-h-[64px] items-center justify-between gap-5 px-4 py-3">
            <div className="min-w-0">
              <div className="text-[13px] font-semibold leading-5 text-[var(--text)]">
                {t("settings.reduceMotion")}
              </div>
              <div className="mt-0.5 text-[11px] leading-4 text-[var(--muted)]">
                {t("settings.reduceMotionDescription")}
              </div>
            </div>
            <Switch
              label={t("settings.reduceMotion")}
              checked={preferences.reduceMotion}
              onChange={() => onReduceMotionChange(!preferences.reduceMotion)}
            />
          </div>
        </div>
      </section>

      <p aria-live="polite" className="mt-2 min-h-4 px-1 text-[11px] leading-4 text-[#e08b8b]">
        {saving ? t("settings.saving") : ""}
      </p>
    </div>
  );
}
