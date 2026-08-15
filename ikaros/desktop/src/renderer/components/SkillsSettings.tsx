import { RefreshCw } from "lucide-react";
import { useState } from "react";

import type {
  RuntimeSkillDiagnostic,
  RuntimeSkillSummary
} from "../../shared/runtime";
import { useTranslation, type TranslationKey } from "../i18n";
import { cx } from "./ui";

type SkillCatalogStatus = "idle" | "loading" | "ready" | "error";
type SkillActionError = "load" | "refresh" | "update" | null;

const DIAGNOSTIC_TRANSLATIONS: Readonly<Partial<Record<string, TranslationKey>>> = {
  catalog_unavailable: "settings.skills.diagnostic.catalogUnavailable",
  file_too_large: "settings.skills.diagnostic.fileTooLarge",
  frontmatter_too_large: "settings.skills.diagnostic.frontmatterTooLarge",
  invalid_description: "settings.skills.diagnostic.invalidDescription",
  invalid_frontmatter: "settings.skills.diagnostic.invalidFrontmatter",
  invalid_name: "settings.skills.diagnostic.invalidName",
  invalid_utf8: "settings.skills.diagnostic.invalidUtf8",
  missing_file: "settings.skills.diagnostic.missingFile",
  name_mismatch: "settings.skills.diagnostic.nameMismatch",
  protected_value: "settings.skills.diagnostic.protectedValue",
  read_failed: "settings.skills.diagnostic.readFailed",
  unsafe_path: "settings.skills.diagnostic.unsafePath",
  unsafe_root: "settings.skills.diagnostic.unsafeRoot"
};

interface SkillsSettingsProps {
  skills: readonly RuntimeSkillSummary[];
  diagnostics: readonly RuntimeSkillDiagnostic[];
  status: SkillCatalogStatus;
  catalogError: string | null;
  onRefresh(): Promise<void>;
  onSetEnabled(name: string, enabled: boolean): Promise<void>;
}

function SkillSwitch({
  name,
  checked,
  disabled,
  onChange
}: {
  name: string;
  checked: boolean;
  disabled: boolean;
  onChange(): void;
}) {
  const { t } = useTranslation();

  return (
    <button
      type="button"
      role="switch"
      aria-label={t("settings.skills.toggleSkill", { skill: name })}
      aria-checked={checked}
      disabled={disabled}
      onClick={onChange}
      className={cx(
        "relative h-5 w-8 shrink-0 rounded-full border outline-none transition-[background-color,border-color,opacity] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] disabled:cursor-wait disabled:opacity-50",
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

export function SkillsSettings({
  skills,
  diagnostics,
  status,
  catalogError,
  onRefresh,
  onSetEnabled
}: SkillsSettingsProps) {
  const { t } = useTranslation();
  const [pendingSkills, setPendingSkills] = useState<ReadonlySet<string>>(new Set());
  const [actionError, setActionError] = useState<SkillActionError>(null);
  const loadingInitialCatalog = (status === "idle" || status === "loading") && skills.length === 0;
  const diagnosticMessage = (diagnostic: RuntimeSkillDiagnostic) => {
    const translationKey = DIAGNOSTIC_TRANSLATIONS[diagnostic.code];
    return translationKey ? t(translationKey) : diagnostic.message;
  };

  const refresh = () => {
    setActionError(null);
    void onRefresh().catch(() => setActionError("refresh"));
  };

  const setEnabled = (skill: RuntimeSkillSummary) => {
    setActionError(null);
    setPendingSkills((current) => new Set(current).add(skill.name));
    void onSetEnabled(skill.name, !skill.enabled)
      .catch(() => setActionError("update"))
      .finally(() => {
        setPendingSkills((current) => {
          const next = new Set(current);
          next.delete(skill.name);
          return next;
        });
      });
  };

  const visibleError: SkillActionError =
    actionError ?? (status === "error" || catalogError ? "load" : null);

  return (
    <div>
      <div className="flex items-start justify-between gap-6">
        <div>
          <h1
            data-settings-heading
            tabIndex={-1}
            className="text-[20px] font-semibold leading-7 tracking-[-0.025em] text-[var(--text)] outline-none"
          >
            {t("settings.skills.title")}
          </h1>
          <p className="mt-1.5 max-w-[580px] text-[11px] leading-4 text-[var(--muted)]">
            {t("settings.skills.description")}
          </p>
        </div>
        <button
          type="button"
          disabled={status === "loading"}
          onClick={refresh}
          className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 text-[12px] font-medium leading-[18px] text-[var(--text)] outline-none transition-colors hover:bg-[var(--panel-hover)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] disabled:cursor-wait disabled:opacity-50"
        >
          <RefreshCw
            size={13}
            aria-hidden="true"
            className={cx(status === "loading" && "animate-spin")}
          />
          {t("settings.skills.refresh")}
        </button>
      </div>

      <section
        aria-labelledby="installed-skills-heading"
        aria-busy={status === "loading"}
        className="mt-10"
      >
        <h2
          id="installed-skills-heading"
          className="mb-3 text-[13px] font-semibold leading-5 text-[var(--text)]"
        >
          {t("settings.skills.installed")}
        </h2>

        {loadingInitialCatalog ? (
          <div className="rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)] px-5 py-8 text-center text-[12px] leading-5 text-[var(--muted)]">
            {t("settings.skills.loading")}
          </div>
        ) : skills.length > 0 ? (
          <div className="overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)]">
            {skills.map((skill) => (
              <div
                key={skill.name}
                className="flex min-h-[68px] items-center gap-5 border-t border-[var(--separator)] px-4 py-3 first:border-t-0"
              >
                <div className="min-w-0 flex-1">
                  <div
                    title={skill.location}
                    className="truncate text-[13px] font-semibold leading-5 text-[var(--text)]"
                  >
                    {skill.name}
                  </div>
                  <div className="mt-0.5 text-[11px] leading-4 text-[var(--muted)]">
                    {skill.description}
                  </div>
                </div>
                <SkillSwitch
                  name={skill.name}
                  checked={skill.enabled}
                  disabled={status === "loading" || pendingSkills.has(skill.name)}
                  onChange={() => setEnabled(skill)}
                />
              </div>
            ))}
          </div>
        ) : (
          <div className="rounded-2xl border border-dashed border-[var(--border)] px-6 py-10 text-center">
            <p className="text-[12px] leading-5 text-[var(--muted)]">
              {t("settings.skills.empty")}
            </p>
            <p className="mt-1 text-[11px] leading-4 text-[var(--muted)]">
              {t("settings.skills.emptyHint")}
            </p>
          </div>
        )}
      </section>

      {diagnostics.length > 0 ? (
        <section aria-labelledby="skill-diagnostics-heading" className="mt-8">
          <h2
            id="skill-diagnostics-heading"
            className="mb-3 text-[13px] font-semibold leading-5 text-[var(--text)]"
          >
            {t("settings.skills.problems")}
          </h2>
          <div className="overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)]">
            {diagnostics.map((diagnostic, index) => (
              <div
                key={`${diagnostic.entry}:${diagnostic.code}:${index}`}
                className="border-t border-[var(--separator)] px-4 py-3 first:border-t-0"
              >
                <div className="truncate text-[12px] font-semibold leading-[18px] text-[var(--text)]">
                  {diagnostic.entry}
                </div>
                <p className="mt-0.5 text-[11px] leading-4 text-[var(--muted)]">
                  {diagnosticMessage(diagnostic)}
                </p>
              </div>
            ))}
          </div>
        </section>
      ) : null}

      <p aria-live="polite" className="mt-3 min-h-4 text-[11px] leading-4 text-[#e08b8b]">
        {visibleError === "update"
          ? t("settings.skills.updateFailed")
          : visibleError === "refresh"
            ? t("settings.skills.refreshFailed")
            : visibleError === "load"
              ? t("settings.skills.catalogFailed")
              : ""}
      </p>
    </div>
  );
}
