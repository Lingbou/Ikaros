import type {
  AppEventTextKind,
  EventText,
  InterruptCopy
} from "./domain";
import type { Translate, TranslationKey } from "./i18n";

const APP_EVENT_TEXT_KEYS: Record<AppEventTextKind, TranslationKey> = {
  "tool.inspectArchiveMetadata": "events.tool.inspectArchiveMetadata",
  "tool.testArchiveExtraction": "events.tool.testArchiveExtraction",
  "tool.exportReportPackage": "events.tool.exportReportPackage",
  "tool.runProcess": "events.tool.runProcess",
  "tool.startProcess": "events.tool.startProcess",
  "tool.readProcess": "events.tool.readProcess",
  "tool.waitProcess": "events.tool.waitProcess",
  "tool.stopProcess": "events.tool.stopProcess",
  "tool.readFile": "events.tool.readFile",
  "tool.writeFile": "events.tool.writeFile",
  "tool.editFile": "events.tool.editFile",
  "result.archiveManifestVerified": "events.result.archiveManifestVerified",
  "result.windowsReservedNames": "events.result.windowsReservedNames",
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
  "permission.moveReceiptsTitle": "events.permission.moveReceiptsTitle",
  "permission.moveReceiptsDescription": "events.permission.moveReceiptsDescription",
  "status.buildingBrief": "events.status.buildingBrief",
  "status.briefBuilt": "events.status.briefBuilt",
  "status.sourcesLinked": "events.status.sourcesLinked",
  "status.checkpointRestored": "events.status.checkpointRestored",
  "status.exportStepRetried": "events.status.exportStepRetried",
  "status.outputsValidated": "events.status.outputsValidated"
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
