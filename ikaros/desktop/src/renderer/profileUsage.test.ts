import { describe, expect, it } from "vitest";

import {
  buildTokenActivityChart,
  formatCompactTokens,
  formatDurationSeconds,
  formatStreakDays,
  tokenActivityMonthLabels,
  TOKEN_ACTIVITY_DAY_COUNT,
  TOKEN_ACTIVITY_WEEK_COUNT
} from "./profileUsage";

function cellIndex(chart: ReturnType<typeof buildTokenActivityChart>, date: string): number {
  return chart.dates.flat().indexOf(date);
}

describe("profile token usage", () => {
  it("normalizes the fixed 52-week daily window without inventing usage", () => {
    const chart = buildTokenActivityChart(
      [
        { startDate: "2026-05-29", tokens: 10 },
        { startDate: "2026-05-29", tokens: 5 },
        { startDate: "2026-05-28", tokens: -4 },
        { startDate: "2026-05-30", tokens: 100 },
        { startDate: "2025-05-01", tokens: 100 },
        { startDate: "not-a-date", tokens: 100 }
      ],
      "daily",
      "2026-05-29"
    );

    expect(chart.levels).toHaveLength(TOKEN_ACTIVITY_WEEK_COUNT);
    expect(chart.levels.every((week) => week.length === TOKEN_ACTIVITY_DAY_COUNT)).toBe(true);
    expect(chart.startDate).toBe("2025-06-01");
    expect(chart.totalTokens).toBe(15);
    expect(chart.dailyTokens.flat()[cellIndex(chart, "2026-05-29")]).toBe(15);
    expect(chart.dailyTokens.flat()[cellIndex(chart, "2026-05-28")]).toBe(0);
    expect(chart.dailyTokens.flat()[cellIndex(chart, "2026-05-30")]).toBe(0);
    expect(chart.future.flat()[cellIndex(chart, "2026-05-30")]).toBe(true);
  });

  it("uses Codex daily grading thresholds against the largest visible day", () => {
    const chart = buildTokenActivityChart(
      [
        { startDate: "2026-05-24", tokens: 100 },
        { startDate: "2026-05-25", tokens: 75 },
        { startDate: "2026-05-26", tokens: 50 },
        { startDate: "2026-05-27", tokens: 25 },
        { startDate: "2026-05-28", tokens: 1 }
      ],
      "daily",
      "2026-05-29"
    );
    const levels = chart.levels.flat();

    expect(levels[cellIndex(chart, "2026-05-24")]).toBe(4);
    expect(levels[cellIndex(chart, "2026-05-25")]).toBe(3);
    expect(levels[cellIndex(chart, "2026-05-26")]).toBe(2);
    expect(levels[cellIndex(chart, "2026-05-27")]).toBe(1);
    expect(levels[cellIndex(chart, "2026-05-28")]).toBe(1);
  });

  it("derives weekly and cumulative bottom-filled bars from daily buckets", () => {
    const buckets = [
      { startDate: "2026-05-10", tokens: 10 },
      { startDate: "2026-05-17", tokens: 20 },
      { startDate: "2026-05-24", tokens: 30 }
    ];
    const weekly = buildTokenActivityChart(buckets, "weekly", "2026-05-29");
    const cumulative = buildTokenActivityChart(buckets, "cumulative", "2026-05-29");
    const weekFor = (date: string) => Math.floor(cellIndex(weekly, date) / TOKEN_ACTIVITY_DAY_COUNT);
    const filled = (levels: readonly number[]) => levels.filter((level) => level === 4).length;

    expect(filled(weekly.levels[weekFor("2026-05-10")])).toBe(3);
    expect(filled(weekly.levels[weekFor("2026-05-17")])).toBe(5);
    expect(filled(weekly.levels[weekFor("2026-05-24")])).toBe(7);
    expect(filled(cumulative.levels[weekFor("2026-05-10")])).toBe(2);
    expect(filled(cumulative.levels[weekFor("2026-05-17")])).toBe(4);
    expect(filled(cumulative.levels[weekFor("2026-05-24")])).toBe(7);
    expect(cumulative.columnValues.at(-1)).toBe(60);
  });

  it("derives localized month labels from the actual chart window", () => {
    const english = tokenActivityMonthLabels("2026-05-29", "en");
    const chinese = tokenActivityMonthLabels("2026-05-29", "zh-CN");

    expect(english[0]).toEqual({ column: 0, label: "Jun" });
    expect(chinese[0]).toEqual({ column: 0, label: "6月" });
    expect(english.every(({ column }) => column >= 0 && column < 52)).toBe(true);
  });

  it("formats real summary values and preserves unavailable values", () => {
    expect(formatCompactTokens(761_100_000, "en")).toBe("761.1M");
    expect(formatCompactTokens(761_100_000, "zh-CN")).toBe("7.6亿");
    expect(formatCompactTokens(null, "en")).toBe("—");
    expect(formatDurationSeconds(13_920, "en")).toBe("3h 52m");
    expect(formatDurationSeconds(13_920, "zh-CN")).toBe("3 小时 52 分");
    expect(formatStreakDays(1, "en")).toBe("1 day");
    expect(formatStreakDays(5, "zh-CN")).toBe("5 天");
  });
});
