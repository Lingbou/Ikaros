import type { UiLanguagePreference } from "../shared/platform";
import type { RuntimeJournalEvent } from "../shared/runtime";

type TerminalProcessState = "exited" | "terminated" | "unknown";

export interface BackgroundProcessCompletion {
  processId: string;
  state: TerminalProcessState;
  exitCode: number | null;
  command: string;
}

export interface DesktopNotificationOptions {
  title: string;
  body: string;
}

export interface DesktopNotificationHandle {
  show(): void;
  on(event: "click", listener: () => void): void;
}

export interface NotificationWindow {
  isDestroyed(): boolean;
  isFocused(): boolean;
  isMinimized(): boolean;
  restore(): void;
  show(): void;
  focus(): void;
}

export interface BackgroundProcessNotifierOptions {
  getWindows(): readonly NotificationWindow[];
  createNotification(
    options: DesktopNotificationOptions,
  ): DesktopNotificationHandle | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function backgroundProcessCompletion(
  event: RuntimeJournalEvent,
): BackgroundProcessCompletion | null {
  if (event.type !== "process.recorded") return null;
  const process = event.payload.process;
  if (!isRecord(process) || process.background !== true) return null;
  if (
    process.state !== "exited" &&
    process.state !== "terminated" &&
    process.state !== "unknown"
  ) {
    return null;
  }
  if (process.state === "unknown" && process.errorCode === "start_pending") return null;
  if (typeof process.processId !== "string" || process.processId.length === 0) return null;
  return {
    processId: process.processId,
    state: process.state,
    exitCode:
      typeof process.exitCode === "number" && Number.isSafeInteger(process.exitCode)
        ? process.exitCode
        : null,
    command: typeof process.command === "string" ? process.command : "",
  };
}

function commandExcerpt(completion: BackgroundProcessCompletion): string {
  const value = completion.command.replace(/\s+/g, " ").trim();
  const excerpt = value.length > 160 ? `${value.slice(0, 157)}...` : value;
  return excerpt || completion.processId;
}

function terminalStatus(
  completion: BackgroundProcessCompletion,
  language: UiLanguagePreference,
): string {
  const chinese = language === "zh-CN";
  if (completion.state === "terminated") {
    return chinese ? "命令已停止。" : "The command was stopped.";
  }
  if (completion.state === "unknown") {
    return chinese ? "命令最终状态未知。" : "The command's final state is unknown.";
  }
  if (completion.exitCode === 0) {
    return chinese ? "命令已成功退出。" : "The command exited successfully.";
  }
  return completion.exitCode === null
    ? chinese
      ? "命令已退出。"
      : "The command exited."
    : chinese
      ? `命令已退出，退出码 ${completion.exitCode}。`
      : `The command exited with code ${completion.exitCode}.`;
}

export function backgroundProcessNotificationCopy(
  completion: BackgroundProcessCompletion,
  language: UiLanguagePreference,
): DesktopNotificationOptions {
  return {
    title: language === "zh-CN" ? "后台命令已结束" : "Background command finished",
    body: `${commandExcerpt(completion)}\n${terminalStatus(completion, language)}`,
  };
}

export class BackgroundProcessNotifier {
  private readonly notifiedProcessIds = new Set<string>();

  constructor(private readonly options: BackgroundProcessNotifierOptions) {}

  notify(completion: BackgroundProcessCompletion, language: UiLanguagePreference): void {
    if (this.notifiedProcessIds.has(completion.processId)) return;
    const windows = this.options.getWindows().filter((window) => !window.isDestroyed());
    if (windows.length === 0 || windows.some((window) => window.isFocused())) return;

    const notification = this.options.createNotification(
      backgroundProcessNotificationCopy(completion, language),
    );
    if (notification === null) return;

    this.notifiedProcessIds.add(completion.processId);
    notification.on("click", () => {
      const target =
        windows.find((window) => !window.isDestroyed()) ??
        this.options.getWindows().find((window) => !window.isDestroyed());
      if (target === undefined || target.isDestroyed()) return;
      if (target.isMinimized()) target.restore();
      target.show();
      target.focus();
    });
    notification.show();
  }
}