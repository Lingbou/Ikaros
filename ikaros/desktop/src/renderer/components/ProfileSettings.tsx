import * as Dialog from "@radix-ui/react-dialog";
import {
  ChartNoAxesCombined,
  FilePenLine,
  GitBranch,
  Globe2,
  Pencil,
  ScanSearch,
  type LucideIcon
} from "lucide-react";
import { useState, type KeyboardEvent } from "react";

import { useTranslation, type TranslationKey, type UiLanguage } from "../i18n";
import { profileInitials } from "../localProfile";
import { useAppStore } from "../store";

type ActivityView = "daily" | "weekly" | "cumulative";

interface ProfileMetric {
  label: TranslationKey;
  value: Record<UiLanguage, string>;
}

interface ProfileInsight {
  label: TranslationKey;
  value: Record<UiLanguage, string>;
}

interface MockSkill {
  name: string;
  runs: number;
  Icon: LucideIcon;
}

const PROFILE_METRICS: readonly ProfileMetric[] = [
  {
    label: "settings.profileLifetimeTokens",
    value: { en: "761.1M", "zh-CN": "7.6亿" }
  },
  {
    label: "settings.profilePeakTokens",
    value: { en: "73.1M", "zh-CN": "7313.3万" }
  },
  {
    label: "settings.profileLongestChat",
    value: { en: "16h 17m", "zh-CN": "16 小时 17 分" }
  },
  {
    label: "settings.profileCurrentStreak",
    value: { en: "0 days", "zh-CN": "0 天" }
  },
  {
    label: "settings.profileLongestStreak",
    value: { en: "5 days", "zh-CN": "5 天" }
  }
];

const PROFILE_INSIGHTS: readonly ProfileInsight[] = [
  {
    label: "settings.profileFastMode",
    value: { en: "86%", "zh-CN": "86%" }
  },
  {
    label: "settings.profileMostUsedReasoning",
    value: { en: "Extra High · 80%", "zh-CN": "极高 · 80%" }
  },
  {
    label: "settings.profileSkillsExplored",
    value: { en: "50", "zh-CN": "50" }
  },
  {
    label: "settings.profileTotalSkillsUsed",
    value: { en: "1,305", "zh-CN": "1,305" }
  },
  {
    label: "settings.profileTotalChats",
    value: { en: "97", "zh-CN": "97" }
  }
];

const MOCK_SKILLS: readonly MockSkill[] = [
  { name: "web-research", runs: 474, Icon: Globe2 },
  { name: "git-workflow", runs: 282, Icon: GitBranch },
  { name: "browser-control", runs: 53, Icon: ScanSearch },
  { name: "document-drafting", runs: 46, Icon: FilePenLine },
  { name: "data-analysis", runs: 44, Icon: ChartNoAxesCombined }
];

const ACTIVITY_VIEWS: readonly { id: ActivityView; label: TranslationKey }[] = [
  { id: "daily", label: "settings.profileDaily" },
  { id: "weekly", label: "settings.profileWeekly" },
  { id: "cumulative", label: "settings.profileCumulative" }
];

const ENGLISH_MONTHS = [
  "Sep",
  "Oct",
  "Nov",
  "Dec",
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug"
] as const;

const CHINESE_MONTHS = [
  "9月",
  "10月",
  "11月",
  "12月",
  "1月",
  "2月",
  "3月",
  "4月",
  "5月",
  "6月",
  "7月",
  "8月"
] as const;

const ACTIVITY_COLORS = [
  "color-mix(in srgb, var(--text) 4%, var(--canvas))",
  "color-mix(in srgb, var(--accent) 18%, color-mix(in srgb, var(--text) 4%, var(--canvas)))",
  "color-mix(in srgb, var(--accent) 34%, color-mix(in srgb, var(--text) 8%, var(--canvas)))",
  "color-mix(in srgb, var(--accent) 52%, color-mix(in srgb, var(--text) 18%, var(--canvas)))",
  "color-mix(in srgb, var(--accent) 62%, var(--text))"
] as const;

const DAILY_CLUSTER = [
  [0, 0, 0, 0, 0, 1, 0],
  [0, 0, 1, 3, 2, 0, 1],
  [0, 2, 4, 4, 4, 2, 0],
  [1, 3, 4, 3, 2, 1, 0],
  [0, 2, 3, 4, 4, 2, 1],
  [1, 3, 4, 3, 2, 1, 0],
  [0, 1, 3, 4, 3, 1, 0],
  [0, 0, 2, 3, 1, 0, 0],
  [0, 0, 0, 1, 0, 0, 0]
] as const;

function createActivityPattern(view: ActivityView): number[][] {
  const pattern = Array.from({ length: 52 }, () => Array.from({ length: 7 }, () => 0));

  if (view === "daily") {
    DAILY_CLUSTER.forEach((days, offset) => {
      pattern[31 + offset] = [...days];
    });
    pattern[29][6] = 2;
    pattern[40][0] = 2;
    pattern[47][3] = 1;
    return pattern;
  }

  if (view === "weekly") {
    for (let week = 9; week < 49; week += 1) {
      for (let day = 0; day < 7; day += 1) {
        const cadence = (week * 3 + day * 5) % 17;
        const inActiveSeason = (week >= 28 && week <= 41) || (week >= 13 && week <= 20);
        pattern[week][day] = cadence < (inActiveSeason ? 7 : 2)
          ? Math.min(4, 1 + ((week + day) % (inActiveSeason ? 4 : 2)))
          : 0;
      }
    }
    return pattern;
  }

  for (let week = 5; week < 52; week += 1) {
    for (let day = 0; day < 7; day += 1) {
      const threshold = Math.min(11, 2 + Math.floor(week / 6));
      const cadence = (week * 7 + day * 11) % 19;
      pattern[week][day] = cadence < threshold
        ? Math.min(4, 1 + Math.floor(week / 15) + ((week + day) % 2))
        : 0;
    }
  }
  return pattern;
}

const ACTIVITY_PATTERNS: Record<ActivityView, number[][]> = {
  daily: createActivityPattern("daily"),
  weekly: createActivityPattern("weekly"),
  cumulative: createActivityPattern("cumulative")
};

export function ProfileSettings() {
  const { language, t } = useTranslation();
  const profileUsername = useAppStore((state) => state.profileUsername);
  const setProfileUsername = useAppStore((state) => state.setProfileUsername);
  const [editOpen, setEditOpen] = useState(false);
  const [usernameDraft, setUsernameDraft] = useState(profileUsername);
  const [activityView, setActivityView] = useState<ActivityView>("daily");
  const trimmedUsername = usernameDraft.trim();
  const monthLabels = language === "zh-CN" ? CHINESE_MONTHS : ENGLISH_MONTHS;

  const setDialogOpen = (open: boolean) => {
    if (open) setUsernameDraft(profileUsername);
    setEditOpen(open);
  };

  const saveProfile = () => {
    if (!trimmedUsername) return;
    setProfileUsername(trimmedUsername);
    setEditOpen(false);
  };

  const selectActivityView = (view: ActivityView) => {
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

              <div className="mt-5 flex justify-end gap-2">
                <Dialog.Close asChild>
                  <button
                    type="button"
                    className="h-8 rounded-lg px-3 text-[12px] leading-[18px] text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
                  >
                    {t("common.cancel")}
                  </button>
                </Dialog.Close>
                <button
                  type="submit"
                  disabled={!trimmedUsername}
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

      <dl className="mx-auto mt-[clamp(48px,7vh,76px)] grid h-[62px] w-full max-w-[732px] grid-cols-5 overflow-hidden rounded-2xl border border-[color-mix(in_srgb,var(--text)_4%,var(--canvas))] bg-transparent">
        {PROFILE_METRICS.map((metric, index) => (
          <div
            key={metric.label}
            className={
              index < PROFILE_METRICS.length - 1
                ? "relative flex min-w-0 flex-col items-center justify-center px-2 text-center after:absolute after:right-0 after:top-1/2 after:h-9 after:w-px after:-translate-y-1/2 after:bg-[color-mix(in_srgb,var(--text)_4%,var(--canvas))]"
                : "flex min-w-0 flex-col items-center justify-center px-2 text-center"
            }
          >
            <dt className="order-2 mt-1 truncate text-[11px] leading-4 text-[var(--muted)]">
              {t(metric.label)}
            </dt>
            <dd className="order-1 truncate text-[13px] font-medium leading-5 text-[var(--text)]">
              {metric.value[language]}
            </dd>
          </div>
        ))}
      </dl>

      <section
        aria-labelledby="profile-token-activity"
        className="mx-auto mt-[clamp(32px,4vh,40px)] w-full max-w-[732px]"
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
            {ACTIVITY_PATTERNS[activityView].map((week, weekIndex) => (
              <div key={weekIndex} aria-hidden="true" className="flex min-w-0 flex-1 flex-col gap-[3px]">
                {week.map((level, dayIndex) => (
                  <span
                    key={dayIndex}
                    data-level={level}
                    className="aspect-square w-full rounded-[3px] transition-colors duration-200"
                    style={{ backgroundColor: ACTIVITY_COLORS[level] }}
                  />
                ))}
              </div>
            ))}
          </div>
          <div
            aria-hidden="true"
            className="mt-2 grid grid-cols-12 gap-1 text-[10px] leading-4 text-[var(--muted)]"
          >
            {monthLabels.map((month) => (
              <span key={month} className="text-left last:text-right">
                {month}
              </span>
            ))}
          </div>
        </div>
      </section>

      <div className="mx-auto mt-[clamp(32px,4vh,40px)] grid w-full max-w-[732px] grid-cols-1 gap-8 pb-6 md:grid-cols-2 md:gap-10">
        <section aria-labelledby="profile-activity-insights">
          <h2
            id="profile-activity-insights"
            className="text-[13px] font-semibold leading-5 text-[var(--text)]"
          >
            {t("settings.profileActivityInsights")}
          </h2>
          <dl className="mt-2.5 space-y-2.5">
            {PROFILE_INSIGHTS.map((insight) => (
              <div key={insight.label} className="flex min-w-0 items-baseline justify-between gap-4 py-0.5">
                <dt className="min-w-0 truncate text-[12px] leading-[18px] text-[var(--muted-strong)]">
                  {t(insight.label)}
                </dt>
                <dd className="shrink-0 text-right text-[12px] font-medium leading-[18px] text-[var(--text)]">
                  {insight.value[language]}
                </dd>
              </div>
            ))}
          </dl>
        </section>

        <section aria-labelledby="profile-most-used-skills">
          <h2
            id="profile-most-used-skills"
            className="text-[13px] font-semibold leading-5 text-[var(--text)]"
          >
            {t("settings.profileMostUsedSkills")}
          </h2>
          <ol className="mt-2 space-y-1">
            {MOCK_SKILLS.map(({ name, runs, Icon }) => (
              <li
                key={name}
                className="group flex min-h-7 min-w-0 items-center gap-2 rounded-lg px-1 py-0.5 transition-colors hover:bg-[var(--surface-hover)]"
              >
                <span
                  aria-hidden="true"
                  className="flex size-5 shrink-0 items-center justify-center rounded-md border border-[var(--border-soft)] bg-[var(--panel)] text-[var(--muted-strong)]"
                >
                  <Icon size={12} strokeWidth={1.8} />
                </span>
                <span className="min-w-0 flex-1 truncate text-[12px] font-medium leading-[18px] text-[var(--text)]">
                  {name}
                </span>
                <span className="shrink-0 text-[11px] leading-4 text-[var(--muted)]">
                  {t("settings.profileSkillRuns", { count: runs })}
                </span>
              </li>
            ))}
          </ol>
        </section>
      </div>
    </div>
  );
}
