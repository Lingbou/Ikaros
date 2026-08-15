import "@testing-library/jest-dom/vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  cloneUiPreferences,
  DEFAULT_UI_PREFERENCES,
  mergeUiPreferences,
  type UiPreferencesPatch
} from "../../shared/platform";
import {
  RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
  type RuntimeInvocationResult,
  type RuntimeJournalEvent,
  type RuntimeUsageReadResult
} from "../../shared/runtime";
import { setUiLanguage } from "../i18n";
import { LOCAL_PROFILE } from "../localProfile";
import { localCalendarDate } from "../profileUsage";
import { useAppStore } from "../store";
import { ProfileSettings } from "./ProfileSettings";

const EMPTY_USAGE: RuntimeUsageReadResult = {
  summary: {
    lifetimeTokens: null,
    peakDailyTokens: null,
    longestRunningTurnSec: null,
    currentStreakDays: 0,
    longestStreakDays: 0
  },
  dailyUsageBuckets: []
};

function realUsage(): RuntimeUsageReadResult {
  return {
    summary: {
      lifetimeTokens: 21_400_000_000,
      peakDailyTokens: 835_000_000,
      longestRunningTurnSec: 13_920,
      currentStreakDays: 12,
      longestStreakDays: 54
    },
    dailyUsageBuckets: [{ startDate: localCalendarDate(), tokens: 835_000_000 }]
  };
}

interface DesktopApiOptions {
  updatePreferences?: (
    patch: UiPreferencesPatch
  ) => Promise<typeof DEFAULT_UI_PREFERENCES>;
  readUsage?: () => Promise<RuntimeInvocationResult<RuntimeUsageReadResult>>;
}

function installDesktopApi(options: DesktopApiOptions = {}) {
  let current = cloneUiPreferences(DEFAULT_UI_PREFERENCES);
  const update = vi.fn(
    options.updatePreferences ??
      (async (patch: UiPreferencesPatch) => {
        current = mergeUiPreferences(current, patch);
        return cloneUiPreferences(current);
      })
  );
  const readUsage = vi.fn(
    options.readUsage ??
      (async () => ({ ok: true as const, value: EMPTY_USAGE }))
  );
  let eventListener: ((event: RuntimeJournalEvent) => void) | undefined;
  const unsubscribe = vi.fn();
  const onEvent = vi.fn((listener: (event: RuntimeJournalEvent) => void) => {
    eventListener = listener;
    return unsubscribe;
  });
  Object.defineProperty(window, "ikarosDesktop", {
    configurable: true,
    value: {
      runtime: { readUsage, onEvent },
      preferences: {
        get: vi.fn(async () => cloneUiPreferences(current)),
        update,
        onChanged: vi.fn(() => vi.fn())
      }
    }
  });
  return {
    emitEvent: (event: RuntimeJournalEvent) => eventListener?.(event),
    readUsage,
    unsubscribe,
    update
  };
}

function runtimeEvent(type: RuntimeJournalEvent["type"]): RuntimeJournalEvent {
  return {
    seq: 1,
    schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type,
    threadId: "thread-usage",
    branchId: "branch-usage",
    turnId: "turn-usage",
    runId: "run-usage",
    itemId: null,
    timestamp: "2026-08-15T00:00:00Z",
    payload: {}
  };
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  setUiLanguage("en");
  useAppStore.getState().setProfileUsername(LOCAL_PROFILE.name);
  Reflect.deleteProperty(window, "ikarosDesktop");
});

describe("ProfileSettings", () => {
  it("renders Runtime-backed metrics and removes unsupported mock sections", async () => {
    const usage = realUsage();
    const { readUsage } = installDesktopApi({
      readUsage: async () => ({ ok: true, value: usage })
    });
    const { container } = render(<ProfileSettings />);

    expect(screen.getByRole("heading", { level: 1, name: "Profile" })).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { level: 2, name: LOCAL_PROFILE.name })
    ).toBeInTheDocument();
    expect(await screen.findByText("21.4B")).toBeInTheDocument();
    for (const value of ["835M", "3h 52m", "12 days", "54 days"]) {
      expect(screen.getByText(value)).toBeInTheDocument();
    }
    for (const label of [
      "Lifetime tokens",
      "Peak tokens",
      "Longest task",
      "Current streak",
      "Longest streak"
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    expect(readUsage).toHaveBeenCalledOnce();
    expect(container.querySelectorAll("[data-level]")).toHaveLength(52 * 7);
    expect(container.querySelector('[data-tokens="835000000"]')).toBeInTheDocument();
    expect(screen.queryByText("761.1M")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Activity insights" })).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Most used skills" })).not.toBeInTheDocument();
  });

  it("shows unavailable values while usage is loading", () => {
    installDesktopApi({
      readUsage: () => new Promise(() => undefined)
    });
    render(<ProfileSettings />);

    expect(screen.getAllByText("—")).toHaveLength(5);
    expect(screen.getByText("Loading token usage…")).toBeInTheDocument();
    expect(screen.queryByText("761.1M")).not.toBeInTheDocument();
  });

  it("keeps the profile honest when Runtime usage cannot be read", async () => {
    installDesktopApi({
      readUsage: async () => {
        throw new Error("Runtime unavailable");
      }
    });
    const { container } = render(<ProfileSettings />);

    expect(await screen.findByText("Token usage is unavailable.")).toBeInTheDocument();
    expect(screen.getAllByText("—")).toHaveLength(5);
    expect(container.querySelectorAll('[data-level="0"]')).toHaveLength(52 * 7);
  });

  it("switches between real daily, weekly, and cumulative projections", async () => {
    installDesktopApi({
      readUsage: async () => ({ ok: true, value: realUsage() })
    });
    const { container } = render(<ProfileSettings />);
    await screen.findByText("21.4B");
    const daily = screen.getByRole("tab", { name: "Daily" });
    const weekly = screen.getByRole("tab", { name: "Weekly" });
    const cumulative = screen.getByRole("tab", { name: "Cumulative" });

    expect(daily).toHaveAttribute("aria-selected", "true");
    fireEvent.click(weekly);
    expect(weekly).toHaveAttribute("aria-selected", "true");
    expect(container.querySelector("[data-activity-view]")).toHaveAttribute(
      "data-activity-view",
      "weekly"
    );
    expect(container.querySelectorAll('[data-level="4"]')).toHaveLength(7);

    fireEvent.click(cumulative);
    expect(cumulative).toHaveAttribute("aria-selected", "true");
    expect(container.querySelector("[data-activity-view]")).toHaveAttribute(
      "data-activity-view",
      "cumulative"
    );
    expect(container.querySelectorAll('[data-level="4"]')).toHaveLength(7);
  });

  it("coalesces usage events and refreshes the open profile", async () => {
    const initial = realUsage();
    const refreshed: RuntimeUsageReadResult = {
      ...initial,
      summary: { ...initial.summary, lifetimeTokens: 42_000_000_000 }
    };
    let reads = 0;
    const { emitEvent, readUsage } = installDesktopApi({
      readUsage: async () => ({ ok: true, value: reads++ === 0 ? initial : refreshed })
    });
    render(<ProfileSettings />);
    await screen.findByText("21.4B");

    act(() => {
      emitEvent(runtimeEvent("model.usage_recorded"));
      emitEvent(runtimeEvent("run.settled"));
    });

    expect(await screen.findByText("42B")).toBeInTheDocument();
    expect(readUsage).toHaveBeenCalledTimes(2);
  });

  it("does not let an older usage request overwrite an event-driven refresh", async () => {
    const initial = realUsage();
    const refreshed: RuntimeUsageReadResult = {
      ...initial,
      summary: { ...initial.summary, lifetimeTokens: 42_000_000_000 }
    };
    let resolveInitial: (
      result: RuntimeInvocationResult<RuntimeUsageReadResult>
    ) => void = () => undefined;
    const initialRequest = new Promise<RuntimeInvocationResult<RuntimeUsageReadResult>>(
      (resolve) => {
        resolveInitial = resolve;
      }
    );
    let reads = 0;
    const { emitEvent, readUsage } = installDesktopApi({
      readUsage: () =>
        reads++ === 0
          ? initialRequest
          : Promise.resolve({ ok: true as const, value: refreshed })
    });
    render(<ProfileSettings />);

    act(() => emitEvent(runtimeEvent("model.usage_recorded")));
    expect(await screen.findByText("42B")).toBeInTheDocument();
    await act(async () => {
      resolveInitial({ ok: true, value: initial });
      await initialRequest;
    });

    expect(screen.getByText("42B")).toBeInTheDocument();
    expect(screen.queryByText("21.4B")).not.toBeInTheDocument();
    expect(readUsage).toHaveBeenCalledTimes(2);
  });

  it("cancels a scheduled usage refresh when the profile unmounts", async () => {
    vi.useFakeTimers();
    const { emitEvent, readUsage, unsubscribe } = installDesktopApi();
    const { unmount } = render(<ProfileSettings />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(readUsage).toHaveBeenCalledOnce();

    act(() => emitEvent(runtimeEvent("run.settled")));
    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });

    expect(unsubscribe).toHaveBeenCalledOnce();
    expect(readUsage).toHaveBeenCalledOnce();
  });

  it("leaves future daily cells transparent and ignores their buckets", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 4, 27, 12, 0, 0));
    installDesktopApi({
      readUsage: async () => ({
        ok: true,
        value: {
          summary: {
            lifetimeTokens: 4,
            peakDailyTokens: 4,
            longestRunningTurnSec: 1,
            currentStreakDays: 1,
            longestStreakDays: 1
          },
          dailyUsageBuckets: [
            { startDate: "2026-05-27", tokens: 4 },
            { startDate: "2026-05-28", tokens: 999 }
          ]
        }
      })
    });
    const { container } = render(<ProfileSettings />);
    await act(async () => {
      await Promise.resolve();
    });

    const future = container.querySelector<HTMLElement>('[data-future="true"]');
    expect(future).toBeInTheDocument();
    expect(future).toHaveAttribute("data-tokens", "0");
    expect(future?.style.backgroundColor).toBe("transparent");
  });

  it("advances the activity window when an open profile refreshes after local midnight", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 4, 27, 23, 59, 0));
    const usage: RuntimeUsageReadResult = {
      summary: {
        lifetimeTokens: 9,
        peakDailyTokens: 9,
        longestRunningTurnSec: 1,
        currentStreakDays: 1,
        longestStreakDays: 1
      },
      dailyUsageBuckets: [{ startDate: "2026-05-28", tokens: 9 }]
    };
    const { emitEvent, readUsage } = installDesktopApi({
      readUsage: async () => ({ ok: true, value: usage })
    });
    const { container } = render(<ProfileSettings />);
    await act(async () => {
      await Promise.resolve();
    });

    const beforeMidnight = container.querySelector<HTMLElement>(
      '[data-date="2026-05-28"]'
    );
    expect(beforeMidnight).toHaveAttribute("data-future", "true");
    expect(beforeMidnight).toHaveAttribute("data-tokens", "0");

    vi.setSystemTime(new Date(2026, 4, 28, 0, 1, 0));
    act(() => emitEvent(runtimeEvent("run.settled")));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });

    const afterMidnight = container.querySelector<HTMLElement>(
      '[data-date="2026-05-28"]'
    );
    expect(afterMidnight).not.toHaveAttribute("data-future");
    expect(afterMidnight).toHaveAttribute("data-tokens", "9");
    expect(readUsage).toHaveBeenCalledTimes(2);
  });

  it("persists a normalized username and resets canceled drafts", async () => {
    const { update } = installDesktopApi();
    render(<ProfileSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const username = screen.getByRole("textbox", { name: "Username" });
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    expect(username).toHaveAttribute("maxlength", "32");
    expect(username).toHaveValue("User");

    fireEvent.change(username, { target: { value: "  Nova Lane  " } });
    expect(screen.getByText("NL")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(useAppStore.getState().profileUsername).toBe("Nova Lane"));
    expect(update).toHaveBeenCalledWith({ username: "Nova Lane" });
    expect(screen.getByRole("heading", { level: 2, name: "Nova Lane" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const reopenedUsername = screen.getByRole("textbox", { name: "Username" });
    fireEvent.change(reopenedUsername, { target: { value: "Discard me" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(useAppStore.getState().profileUsername).toBe("Nova Lane");
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByRole("textbox", { name: "Username" })).toHaveValue("Nova Lane");
  });

  it("disables saving an empty username", () => {
    installDesktopApi();
    render(<ProfileSettings />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Username" }), {
      target: { value: "   " }
    });

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("keeps the dialog open and the previous username when persistence fails", async () => {
    installDesktopApi({
      updatePreferences: async () => {
        throw new Error("disk unavailable");
      }
    });
    render(<ProfileSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Username" }), {
      target: { value: "Nova Lane" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save the username.");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(useAppStore.getState().profileUsername).toBe(LOCAL_PROFILE.name);
  });

  it("translates Runtime-backed profile UI into Simplified Chinese", async () => {
    setUiLanguage("zh-CN");
    installDesktopApi({
      readUsage: async () => ({ ok: true, value: realUsage() })
    });
    render(<ProfileSettings />);

    expect(await screen.findByText("3 小时 52 分")).toBeInTheDocument();
    for (const label of [
      "累计 Token 数",
      "峰值 Token 数",
      "最长任务时长",
      "当前连续天数",
      "最长连续天数"
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    expect(screen.getByRole("tab", { name: "每日" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "每周" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "累计" })).toBeInTheDocument();
    expect(screen.queryByText("活动洞察")).not.toBeInTheDocument();
    expect(screen.queryByText("最常用的技能")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "编辑" }));
    expect(screen.getByRole("textbox", { name: "用户名" })).toBeInTheDocument();
  });
});
