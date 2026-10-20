import { describe, expect, it, vi } from "vitest";

import type { RuntimeJournalEvent } from "../shared/runtime";
import { RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION } from "../shared/generated/runtimeProtocol";
import {
  BackgroundProcessNotifier,
  backgroundProcessCompletion,
  backgroundProcessNotificationCopy,
  type NotificationWindow,
} from "./backgroundProcessNotifications";

function processEvent(
  process: Record<string, unknown>,
  type: RuntimeJournalEvent["type"] = "process.recorded",
): RuntimeJournalEvent {
  return {
    seq: 1,
    schemaVersion: RUNTIME_JOURNAL_EVENT_SCHEMA_VERSION,
    type,
    threadId: "thread-1",
    branchId: "branch-1",
    turnId: "turn-1",
    runId: "run-1",
    itemId: "call-1",
    timestamp: "2026-10-21T00:00:00.000Z",
    payload: { process },
  };
}

function fakeWindow(focused = false): NotificationWindow {
  return {
    isDestroyed: vi.fn(() => false),
    isFocused: vi.fn(() => focused),
    isMinimized: vi.fn(() => true),
    restore: vi.fn(),
    show: vi.fn(),
    focus: vi.fn(),
  };
}

describe("background process completion notifications", () => {
  it("recognizes only terminal background process facts", () => {
    const process = {
      processId: "process-1",
      background: true,
      state: "exited",
      exitCode: 0,
      command: "npm test",
    };
    const terminal = processEvent(process);
    expect(backgroundProcessCompletion(terminal)).toEqual({
      processId: "process-1",
      state: "exited",
      exitCode: 0,
      command: "npm test",
    });
    expect(backgroundProcessCompletion(processEvent({ ...process, state: "running" }))).toBeNull();
    expect(backgroundProcessCompletion(processEvent({ ...process, background: false }))).toBeNull();
    expect(
      backgroundProcessCompletion(
        processEvent({ ...process, state: "unknown", errorCode: "start_pending" }),
      ),
    ).toBeNull();
    expect(backgroundProcessCompletion(processEvent({ ...process, state: "unknown", exitCode: null }))).toMatchObject({
      state: "unknown",
      exitCode: null,
    });
  });

  it("notifies once for an unfocused window and focuses it when clicked", () => {
    const window = fakeWindow(false);
    let click: (() => void) | undefined;
    const notifications: Array<{ title: string; body: string }> = [];
    const notifier = new BackgroundProcessNotifier({
      getWindows: () => [window],
      createNotification: (options) => {
        notifications.push(options);
        return {
          show: vi.fn(),
          on: vi.fn((_event, listener) => {
            click = listener;
          }),
        };
      },
    });
    const completion = {
      processId: "process-1",
      state: "exited" as const,
      exitCode: 0,
      command: "npm test",
    };

    notifier.notify(completion, "en");
    notifier.notify(completion, "en");

    expect(notifications).toHaveLength(1);
    expect(notifications[0]).toEqual(
      backgroundProcessNotificationCopy(completion, "en"),
    );
    click?.();
    expect(window.restore).toHaveBeenCalledOnce();
    expect(window.show).toHaveBeenCalledOnce();
    expect(window.focus).toHaveBeenCalledOnce();
  });

  it("suppresses notifications while the window is focused", () => {
    const createNotification = vi.fn(() => ({
      show: vi.fn(),
      on: vi.fn(),
    }));
    const notifier = new BackgroundProcessNotifier({
      getWindows: () => [fakeWindow(true)],
      createNotification,
    });

    notifier.notify(
      {
        processId: "process-1",
        state: "terminated",
        exitCode: null,
        command: "npm test",
      },
      "zh-CN",
    );

    expect(createNotification).not.toHaveBeenCalled();
  });
});