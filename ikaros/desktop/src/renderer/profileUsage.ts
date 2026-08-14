import type { RuntimeUsageDailyBucket } from "../shared/runtime";

export const TOKEN_ACTIVITY_WEEK_COUNT = 52;
export const TOKEN_ACTIVITY_DAY_COUNT = 7;

const MILLISECONDS_PER_DAY = 86_400_000;
const TOKEN_ACTIVITY_CELL_COUNT = TOKEN_ACTIVITY_WEEK_COUNT * TOKEN_ACTIVITY_DAY_COUNT;

export type TokenActivityView = "daily" | "weekly" | "cumulative";
export type TokenActivityLevel = 0 | 1 | 2 | 3 | 4;
export type ProfileUsageLanguage = "en" | "zh-CN";

export interface TokenActivityChart {
  levels: TokenActivityLevel[][];
  dailyTokens: number[][];
  dates: string[][];
  future: boolean[][];
  columnValues: number[];
  totalTokens: number;
  startDate: string;
}

export interface TokenActivityMonthLabel {
  column: number;
  label: string;
}

function calendarDayFromParts(year: number, month: number, day: number): number | null {
  if (!Number.isInteger(year) || !Number.isInteger(month) || !Number.isInteger(day)) {
    return null;
  }
  const date = new Date(0);
  date.setUTCHours(0, 0, 0, 0);
  date.setUTCFullYear(year, month - 1, day);
  if (
    date.getUTCFullYear() !== year ||
    date.getUTCMonth() !== month - 1 ||
    date.getUTCDate() !== day
  ) {
    return null;
  }
  return Math.floor(date.getTime() / MILLISECONDS_PER_DAY);
}

function parseCalendarDay(value: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return null;
  return calendarDayFromParts(Number(match[1]), Number(match[2]), Number(match[3]));
}

function requiredCalendarDay(value: string): number {
  const day = parseCalendarDay(value);
  if (day === null) {
    throw new Error(`Invalid calendar date: ${value}`);
  }
  return day;
}

function calendarDateFromDay(day: number): string {
  const date = new Date(day * MILLISECONDS_PER_DAY);
  const year = String(date.getUTCFullYear()).padStart(4, "0");
  const month = String(date.getUTCMonth() + 1).padStart(2, "0");
  const dateOfMonth = String(date.getUTCDate()).padStart(2, "0");
  return `${year}-${month}-${dateOfMonth}`;
}

function calendarWeekday(day: number): number {
  return new Date(day * MILLISECONDS_PER_DAY).getUTCDay();
}

function chartStartDay(today: number): number {
  return today - calendarWeekday(today) - (TOKEN_ACTIVITY_WEEK_COUNT - 1) * TOKEN_ACTIVITY_DAY_COUNT;
}

function weeksFrom<T>(values: readonly T[]): T[][] {
  return Array.from({ length: TOKEN_ACTIVITY_WEEK_COUNT }, (_, week) =>
    values.slice(
      week * TOKEN_ACTIVITY_DAY_COUNT,
      (week + 1) * TOKEN_ACTIVITY_DAY_COUNT
    )
  );
}

function normalizedDailyValues(
  buckets: readonly RuntimeUsageDailyBucket[],
  today: number
): { start: number; values: number[] } {
  const start = chartStartDay(today);
  const end = start + TOKEN_ACTIVITY_CELL_COUNT;
  const values = Array.from({ length: TOKEN_ACTIVITY_CELL_COUNT }, () => 0);

  for (const bucket of buckets) {
    const day = parseCalendarDay(bucket.startDate);
    if (
      day === null ||
      day < start ||
      day >= end ||
      day > today ||
      !Number.isFinite(bucket.tokens)
    ) {
      continue;
    }
    values[day - start] += Math.max(0, bucket.tokens);
  }
  return { start, values };
}

function weeklyTotals(values: readonly number[]): number[] {
  return Array.from({ length: TOKEN_ACTIVITY_WEEK_COUNT }, (_, week) => {
    const start = week * TOKEN_ACTIVITY_DAY_COUNT;
    return values
      .slice(start, start + TOKEN_ACTIVITY_DAY_COUNT)
      .reduce((sum, value) => sum + value, 0);
  });
}

function gradedLevels(values: readonly number[]): TokenActivityLevel[] {
  const maximum = Math.max(0, ...values);
  return values.map((value): TokenActivityLevel => {
    if (value <= 0 || maximum <= 0) return 0;
    if (value * 4 > maximum * 3) return 4;
    if (value * 2 > maximum) return 3;
    if (value * 4 > maximum) return 2;
    return 1;
  });
}

function barLevels(values: readonly number[]): TokenActivityLevel[] {
  const maximum = Math.max(0, ...values);
  return values.flatMap((value) => {
    const height =
      value <= 0 || maximum <= 0
        ? 0
        : Math.min(TOKEN_ACTIVITY_DAY_COUNT, Math.ceil((value / maximum) * TOKEN_ACTIVITY_DAY_COUNT));
    return Array.from(
      { length: TOKEN_ACTIVITY_DAY_COUNT },
      (_, row): TokenActivityLevel =>
        TOKEN_ACTIVITY_DAY_COUNT - row <= height ? 4 : 0
    );
  });
}

export function localCalendarDate(date = new Date()): string {
  const year = String(date.getFullYear()).padStart(4, "0");
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function buildTokenActivityChart(
  buckets: readonly RuntimeUsageDailyBucket[],
  view: TokenActivityView,
  todayDate: string
): TokenActivityChart {
  const today = requiredCalendarDay(todayDate);
  const { start, values } = normalizedDailyValues(buckets, today);
  const totals = weeklyTotals(values);
  const cumulativeTotals = totals.reduce<number[]>((result, value) => {
    result.push((result.at(-1) ?? 0) + value);
    return result;
  }, []);
  const flatLevels =
    view === "daily"
      ? gradedLevels(values)
      : barLevels(view === "weekly" ? totals : cumulativeTotals);
  const dates = Array.from({ length: TOKEN_ACTIVITY_CELL_COUNT }, (_, offset) =>
    calendarDateFromDay(start + offset)
  );

  return {
    levels: weeksFrom(flatLevels),
    dailyTokens: weeksFrom(values),
    dates: weeksFrom(dates),
    future: weeksFrom(dates.map((date) => requiredCalendarDay(date) > today)),
    columnValues: view === "cumulative" ? cumulativeTotals : totals,
    totalTokens: values.reduce((sum, value) => sum + value, 0),
    startDate: calendarDateFromDay(start)
  };
}

export function tokenActivityMonthLabels(
  todayDate: string,
  language: ProfileUsageLanguage
): TokenActivityMonthLabel[] {
  const today = requiredCalendarDay(todayDate);
  const start = chartStartDay(today);
  const formatter = new Intl.DateTimeFormat(language, {
    month: "short",
    timeZone: "UTC"
  });

  const labels: TokenActivityMonthLabel[] = [];
  for (let column = 0; column < TOKEN_ACTIVITY_WEEK_COUNT; column += 1) {
    const day = start + column * TOKEN_ACTIVITY_DAY_COUNT;
    const date = new Date(day * MILLISECONDS_PER_DAY);
    if (date.getUTCDate() <= 7) {
      labels.push({ column, label: formatter.format(date) });
    }
  }
  return labels;
}

export function formatCompactTokens(
  value: number | null,
  language: ProfileUsageLanguage
): string {
  if (value === null || !Number.isFinite(value) || value < 0) return "—";
  return new Intl.NumberFormat(language, {
    notation: "compact",
    compactDisplay: "short",
    maximumFractionDigits: 1
  }).format(value);
}

export function formatDurationSeconds(
  value: number | null,
  language: ProfileUsageLanguage
): string {
  if (value === null || !Number.isFinite(value) || value < 0) return "—";
  const seconds = Math.floor(value);
  const hours = Math.floor(seconds / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);

  if (language === "zh-CN") {
    if (hours === 0 && minutes === 0) return `${seconds} 秒`;
    if (hours === 0) return `${minutes} 分`;
    if (minutes === 0) return `${hours} 小时`;
    return `${hours} 小时 ${minutes} 分`;
  }
  if (hours === 0 && minutes === 0) return `${seconds}s`;
  if (hours === 0) return `${minutes}m`;
  if (minutes === 0) return `${hours}h`;
  return `${hours}h ${minutes}m`;
}

export function formatStreakDays(
  value: number | null,
  language: ProfileUsageLanguage
): string {
  if (value === null || !Number.isFinite(value) || value < 0) return "—";
  const days = Math.floor(value);
  if (language === "zh-CN") return `${days} 天`;
  return `${days} ${days === 1 ? "day" : "days"}`;
}
