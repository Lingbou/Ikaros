import * as Dialog from "@radix-ui/react-dialog";
import { Pencil } from "lucide-react";
import { useEffect, useMemo, useState, type KeyboardEvent } from "react";

import type { RuntimeUsageReadResult } from "../../shared/runtime";
import { useTranslation, type TranslationKey } from "../i18n";
import { profileInitials } from "../localProfile";
import {
  buildTokenActivityChart,
  formatCompactTokens,
  formatDurationSeconds,
  formatStreakDays,
  localCalendarDate,
  tokenActivityMonthLabels,
  TOKEN_ACTIVITY_WEEK_COUNT,
  type TokenActivityView
} from "../profileUsage";
import { createRuntimeClient } from "../runtimeClient";
import { useAppStore } from "../store";

interface ProfileMetric {
  label: TranslationKey;
  value: string;
}

type UsageLoadStatus = "loading" | "ready" | "error";

const ACTIVITY_VIEWS: readonly { id: TokenActivityView; label: TranslationKey }[] = [
  { id: "daily", label: "settings.profileDaily" },
  { id: "weekly", label: "settings.profileWeekly" },
  { id: "cumulative", label: "settings.profileCumulative" }
];

const ACTIVITY_COLORS = [
  "color-mix(in srgb, var(--text) 4%, var(--canvas))",
  "color-mix(in srgb, var(--accent) 18%, color-mix(in srgb, var(--text) 4%, var(--canvas)))",
  "color-mix(in srgb, var(--accent) 34%, color-mix(in srgb, var(--text) 8%, var(--canvas)))",
  "color-mix(in srgb, var(--accent) 52%, color-mix(in srgb, var(--text) 18%, var(--canvas)))",
  "color-mix(in srgb, var(--accent) 62%, var(--text))"
] as const;

export function ProfileSettings() {
  const { language, t } = useTranslation();
  const profileUsername = useAppStore((state) => state.profileUsername);
  const setProfileUsername = useAppStore((state) => state.setProfileUsername);
  const [editOpen, setEditOpen] = useState(false);
  const [usernameDraft, setUsernameDraft] = useState(profileUsername);
  const [savingUsername, setSavingUsername] = useState(false);
  const [usernameSaveFailed, setUsernameSaveFailed] = useState(false);
  const [usage, setUsage] = useState<RuntimeUsageReadResult | null>(null);
  const [usageStatus, setUsageStatus] = useState<UsageLoadStatus>("loading");
  const [activityView, setActivityView] = useState<TokenActivityView>("daily");
  const [todayDate, setTodayDate] = useState(localCalendarDate);
  const trimmedUsername = usernameDraft.trim();

  useEffect(() => {
    let disposed = false;
    let requestGeneration = 0;
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;
    const runtime = createRuntimeClient();
    if (!runtime) {
      setUsageStatus("error");
      return () => {
        disposed = true;
      };
    }

    const readUsage = (generation: number) => {
      void runtime
        .readUsage()
        .then((result) => {
          if (disposed || generation !== requestGeneration) return;
          setUsage(result);
          setTodayDate(localCalendarDate());
          setUsageStatus("ready");
        })
        .catch(() => {
          if (disposed || generation !== requestGeneration) return;
          setUsage(null);
          setUsageStatus("error");
        });
    };
    const scheduleRefresh = () => {
      if (refreshTimer !== undefined) return;
      const generation = ++requestGeneration;
      refreshTimer = setTimeout(() => {
        refreshTimer = undefined;
        readUsage(generation);
      }, 100);
    };
    const unsubscribe = runtime.onEvent((event) => {
      if (event.type === "model.usage_recorded" || event.type === "run.settled") {
        scheduleRefresh();
      }
    });
    readUsage(++requestGeneration);

    return () => {
      disposed = true;
      requestGeneration += 1;
      if (refreshTimer !== undefined) clearTimeout(refreshTimer);
      unsubscribe();
    };
  }, []);

  const profileMetrics = useMemo<readonly ProfileMetric[]>(() => {
    const summary = usage?.summary;
    return [
      {
        label: "settings.profileLifetimeTokens",
        value: formatCompactTokens(summary?.lifetimeTokens ?? null, language)
      },
      {
        label: "settings.profilePeakTokens",
        value: formatCompactTokens(summary?.peakDailyTokens ?? null, language)
      },
      {
        label: "settings.profileLongestTask",
        value: formatDurationSeconds(summary?.longestRunningTurnSec ?? null, language)
      },
      {
        label: "settings.profileCurrentStreak",
        value: formatStreakDays(summary?.currentStreakDays ?? null, language)
      },
      {
        label: "settings.profileLongestStreak",
        value: formatStreakDays(summary?.longestStreakDays ?? null, language)
      }
    ];
  }, [language, usage]);
  const activityChart = useMemo(
    () => buildTokenActivityChart(usage?.dailyUsageBuckets ?? [], activityView, todayDate),
    [activityView, todayDate, usage]
  );
  const monthLabels = useMemo(
    () => tokenActivityMonthLabels(todayDate, language),
    [language, todayDate]
  );
  const activityMessage =
    usageStatus === "loading"
      ? t("settings.profileUsageLoading")
      : usageStatus === "error"
        ? t("settings.profileUsageUnavailable")
        : activityChart.totalTokens === 0
          ? t("settings.profileNoTokenActivity")
          : "";

  const setDialogOpen = (open: boolean) => {
    if (!open && savingUsername) return;
    if (open) setUsernameDraft(profileUsername);
    if (open) setUsernameSaveFailed(false);
    setEditOpen(open);
  };

  const saveProfile = () => {
    if (!trimmedUsername || savingUsername) return;
    const api = window.ikarosDesktop;
    if (!api) {
      setUsernameSaveFailed(true);
      return;
    }
    setSavingUsername(true);
    setUsernameSaveFailed(false);
    void api.preferences
      .update({ username: trimmedUsername })
      .then((preferences) => {
        setProfileUsername(preferences.username);
        setEditOpen(false);
      })
      .catch(() => setUsernameSaveFailed(true))
      .finally(() => setSavingUsername(false));
  };

  const selectActivityView = (view: TokenActivityView) => {
    setActivityView(view);
    window.requestAnimationFrame(() => {
      document.getElementById(`profile-activity-tab-${view}`)?.focus();
    });
  };

  const handleActivityTabKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    index: number
  ) => {
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (index + 1) % ACTIVITY_VIEWS.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (index - 1 + ACTIVITY_VIEWS.length) % ACTIVITY_VIEWS.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = ACTIVITY_VIEWS.length - 1;
    }

    if (nextIndex === null) return;
    event.preventDefault();
    selectActivityView(ACTIVITY_VIEWS[nextIndex].id);
  };

  return (
    <div className="min-w-0">
      <Dialog.Root open={editOpen} onOpenChange={setDialogOpen}>
        <div className="flex min-w-0 items-center justify-between gap-4">
          <h1
            data-settings-heading
            tabIndex={-1}
            className="min-w-0 truncate text-[14px] font-semibold leading-5 tracking-[-0.015em] text-[var(--text)] outline-none"
          >
            {t("settings.profile")}
          </h1>
          <Dialog.Trigger asChild>
            <button
              type="button"
              className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-[12px] font-medium leading-[18px] text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
            >
              <Pencil size={13} aria-hidden="true" />
              {t("settings.profileEdit")}
            </button>
          </Dialog.Trigger>
        </div>

        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-[110] bg-black/55 backdrop-blur-[2px]" />
          <Dialog.Content className="glass-menu fixed left-1/2 top-1/2 z-[120] w-[min(420px,calc(100vw-28px))] -translate-x-1/2 -translate-y-1/2 rounded-2xl p-4">
            <Dialog.Title className="text-[13px] font-semibold leading-5 text-[var(--text)]">
              {t("settings.profileEditTitle")}
            </Dialog.Title>
            <Dialog.Description className="sr-only">
              {t("settings.profileEditDescription")}
            </Dialog.Description>

            <form
              className="mt-5"
              onSubmit={(event) => {
                event.preventDefault();
                saveProfile();
              }}
            >
              <div
                aria-hidden="true"
                className="mx-auto flex size-16 items-center justify-center rounded-full bg-[var(--accent)] text-[20px] font-medium leading-[28px] text-white"
              >
                {profileInitials(usernameDraft)}
              </div>

              <label className="mt-5 block">
                <span className="mb-1.5 block text-[12px] font-medium leading-[18px] text-[var(--text)]">
                  {t("settings.profileUsername")}
                </span>
                <input
                  autoFocus
                  type="text"
                  maxLength={32}
                  value={usernameDraft}
                  onChange={(event) => setUsernameDraft(event.currentTarget.value)}
                  className="h-9 w-full rounded-[10px] border border-[var(--border)] bg-[var(--panel)] px-3 text-[13px] leading-5 text-[var(--text)] outline-none transition-colors focus:border-[var(--muted)]"
                />
              </label>

              {usernameSaveFailed ? (
                <p role="alert" className="mt-2 text-[11px] leading-4 text-red-400">
                  {t("settings.profileSaveFailed")}
                </p>
              ) : null}

              <div className="mt-5 flex justify-end gap-2">
                <Dialog.Close asChild>
                  <button
                    type="button"
                    disabled={savingUsername}
                    className="h-8 rounded-lg px-3 text-[12px] leading-[18px] text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
                  >
                    {t("common.cancel")}
                  </button>
                </Dialog.Close>
                <button
                  type="submit"
                  disabled={!trimmedUsername || savingUsername}
                  className="h-8 rounded-lg bg-[var(--text)] px-3 text-[12px] font-medium leading-[18px] text-[var(--canvas)] transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {t("settings.profileSave")}
                </button>
              </div>
            </form>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>

      <section
        aria-labelledby="profile-display-name"
        className="flex justify-center px-2 pt-[clamp(44px,7vh,68px)]"
      >
        <div className="flex min-w-0 max-w-full flex-col items-center text-center">
          <div
            aria-hidden="true"
            className="flex size-[clamp(68px,8vw,80px)] shrink-0 items-center justify-center rounded-full bg-[var(--accent)] text-[24px] font-medium leading-[30px] text-white"
          >
            {profileInitials(profileUsername)}
          </div>
          <h2
            id="profile-display-name"
            className="mt-3.5 max-w-full truncate text-[20px] font-medium leading-[28px] text-[var(--text)]"
          >
            {profileUsername}
          </h2>
        </div>
      </section>

      <dl
        aria-busy={usageStatus === "loading"}
        className="mx-auto mt-[clamp(48px,7vh,76px)] grid h-[62px] w-full max-w-[732px] grid-cols-5 overflow-hidden rounded-2xl border border-[color-mix(in_srgb,var(--text)_4%,var(--canvas))] bg-transparent"
      >
        {profileMetrics.map((metric, index) => (
          <div
            key={metric.label}
            className={
              index < profileMetrics.length - 1
                ? "relative flex min-w-0 flex-col items-center justify-center px-2 text-center after:absolute after:right-0 after:top-1/2 after:h-9 after:w-px after:-translate-y-1/2 after:bg-[color-mix(in_srgb,var(--text)_4%,var(--canvas))]"
                : "flex min-w-0 flex-col items-center justify-center px-2 text-center"
            }
          >
            <dt className="order-2 mt-1 truncate text-[11px] leading-4 text-[var(--muted)]">
              {t(metric.label)}
            </dt>
            <dd className="order-1 truncate text-[13px] font-medium leading-5 text-[var(--text)]">
              {metric.value}
            </dd>
          </div>
        ))}
      </dl>

      <section
        aria-labelledby="profile-token-activity"
        aria-busy={usageStatus === "loading"}
        className="mx-auto mt-[clamp(32px,4vh,40px)] w-full max-w-[732px] pb-8"
      >
        <div className="flex items-center justify-between gap-4">
          <h2
            id="profile-token-activity"
            className="text-[13px] font-semibold leading-5 text-[var(--text)]"
          >
            {t("settings.profileTokenActivity")}
          </h2>
          <div
            role="tablist"
            aria-label={t("settings.profileTokenActivity")}
            className="flex items-center gap-3 text-[12px] leading-[18px]"
          >
            {ACTIVITY_VIEWS.map((view, index) => (
              <button
                key={view.id}
                id={`profile-activity-tab-${view.id}`}
                type="button"
                role="tab"
                aria-controls="profile-activity-panel"
                aria-selected={activityView === view.id}
                tabIndex={activityView === view.id ? 0 : -1}
                onClick={() => selectActivityView(view.id)}
                onKeyDown={(event) => handleActivityTabKeyDown(event, index)}
                className={
                  activityView === view.id
                    ? "font-semibold text-[var(--text)]"
                    : "text-[var(--muted)] transition-colors hover:text-[var(--muted-strong)]"
                }
              >
                {t(view.label)}
              </button>
            ))}
          </div>
        </div>

        <div
          id="profile-activity-panel"
          role="tabpanel"
          aria-labelledby={`profile-activity-tab-${activityView}`}
          data-activity-view={activityView}
          className="mt-3.5"
        >
          <div
            role="img"
            aria-label={`${t("settings.profileTokenActivity")} · ${t(
              ACTIVITY_VIEWS.find((view) => view.id === activityView)!.label
            )}`}
            className="flex w-full gap-[3px]"
          >
            {activityChart.levels.map((week, weekIndex) => (
              <div key={weekIndex} aria-hidden="true" className="flex min-w-0 flex-1 flex-col gap-[3px]">
                {week.map((level, dayIndex) => {
                  const future =
                    activityView === "daily" && activityChart.future[weekIndex][dayIndex];
                  return (
                    <span
                      key={dayIndex}
                      data-date={activityChart.dates[weekIndex][dayIndex]}
                      data-level={level}
                      data-tokens={
                        activityView === "daily"
                          ? activityChart.dailyTokens[weekIndex][dayIndex]
                          : activityChart.columnValues[weekIndex]
                      }
                      data-future={future ? "true" : undefined}
                      className="aspect-square w-full rounded-[3px] transition-colors duration-200"
                      style={{
                        backgroundColor: future ? "transparent" : ACTIVITY_COLORS[level]
                      }}
                    />
                  );
                })}
              </div>
            ))}
          </div>
          <div
            aria-hidden="true"
            className="mt-2 grid overflow-hidden text-[10px] leading-4 text-[var(--muted)]"
            style={{
              gridTemplateColumns: `repeat(${TOKEN_ACTIVITY_WEEK_COUNT}, minmax(0, 1fr))`
            }}
          >
            {monthLabels.map(({ column, label }) => (
              <span
                key={`${column}-${label}`}
                className="col-span-4 whitespace-nowrap text-left"
                style={{ gridColumnStart: column + 1 }}
              >
                {label}
              </span>
            ))}
          </div>
          <p
            aria-live="polite"
            className="mt-2 min-h-4 text-[10px] leading-4 text-[var(--muted)]"
          >
            {activityMessage}
          </p>
        </div>
      </section>
    </div>
  );
}
