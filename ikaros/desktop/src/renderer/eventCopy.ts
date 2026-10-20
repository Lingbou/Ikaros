import type {
  AppEventTextKind,
  EventText,
  InterruptCopy
} from "./domain";
import type { Translate, TranslationKey } from "./i18n";

const APP_EVENT_TEXT_KEYS: Record<AppEventTextKind, TranslationKey> = {
  "tool.runProcess": "events.tool.runProcess",
  "tool.startProcess": "events.tool.startProcess",
  "tool.readProcess": "events.tool.readProcess",
  "tool.waitProcess": "events.tool.waitProcess",
  "tool.stopProcess": "events.tool.stopProcess",
  "tool.writeProcess": "events.tool.writeProcess",
  "tool.resizeProcess": "events.tool.resizeProcess",
  "tool.readFile": "events.tool.readFile",
  "tool.writeFile": "events.tool.writeFile",
  "tool.editFile": "events.tool.editFile",
  "tool.globFiles": "events.tool.globFiles",
  "tool.searchFiles": "events.tool.searchFiles",
  "tool.listDirectory": "events.tool.listDirectory",
  "result.processRunning": "events.result.processRunning",
  "result.processStopped": "events.result.processStopped",
  "result.processUnknown": "events.result.processUnknown",
  "result.processCompleted": "events.result.processCompleted",
  "result.processFailed": "events.result.processFailed",
  "result.processInterrupted": "events.result.processInterrupted",
  "result.readCompleted": "events.result.readCompleted",
  "result.readFailed": "events.result.readFailed",
  "result.readInterrupted": "events.result.readInterrupted",
  "result.writeCompleted": "events.result.writeCompleted",
  "result.writeFailed": "events.result.writeFailed",
  "result.writeInterrupted": "events.result.writeInterrupted",
  "result.editCompleted": "events.result.editCompleted",
  "result.editFailed": "events.result.editFailed",
  "result.editInterrupted": "events.result.editInterrupted",
  "result.globCompleted": "events.result.globCompleted",
  "result.globFailed": "events.result.globFailed",
  "result.globInterrupted": "events.result.globInterrupted",
  "result.searchCompleted": "events.result.searchCompleted",
  "result.searchFailed": "events.result.searchFailed",
  "result.searchInterrupted": "events.result.searchInterrupted",
  "result.listCompleted": "events.result.listCompleted",
  "result.listFailed": "events.result.listFailed",
  "result.listInterrupted": "events.result.listInterrupted",
  "result.entryCount": "events.result.entryCount",
  "result.matchCount": "events.result.matchCount"
};

const APP_INTERRUPT_COPY_KEYS: Record<
  Extract<InterruptCopy, { source: "app" }>["kind"],
  { title: TranslationKey; description: TranslationKey }
> = {
  run_stopped: {
    title: "events.runStopped.title",
    description: "events.runStopped.description"
  },
  worker_disconnected: {
    title: "events.workerDisconnected.title",
    description: "events.workerDisconnected.description"
  },
  recovering_checkpoint: {
    title: "events.recoveringCheckpoint.title",
    description: "events.recoveringCheckpoint.description"
  },
  retrying_export: {
    title: "events.retryingExport.title",
    description: "events.retryingExport.description"
  },
  recovery_completed: {
    title: "events.recoveryCompleted.title",
    description: "events.recoveryCompleted.description"
  }
};

export function resolveEventText(copy: EventText, t: Translate): string {
  return copy.source === "app"
    ? t(APP_EVENT_TEXT_KEYS[copy.kind], copy.values)
    : copy.value;
}

export function resolveInterruptCopy(
  copy: InterruptCopy,
  t: Translate
): { title: string; description: string } {
  if (copy.source === "external") return copy;
  const keys = APP_INTERRUPT_COPY_KEYS[copy.kind];
  return {
    title: t(keys.title),
    description: t(keys.description)
  };
}