import type {
  AppEventTextKind,
  EventText
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
  "tool.listThreads": "events.tool.listThreads",
  "tool.readThread": "events.tool.readThread",
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
  "result.threadListCompleted": "events.result.threadListCompleted",
  "result.threadListFailed": "events.result.threadListFailed",
  "result.threadListInterrupted": "events.result.threadListInterrupted",
  "result.threadReadCompleted": "events.result.threadReadCompleted",
  "result.threadReadFailed": "events.result.threadReadFailed",
  "result.threadReadInterrupted": "events.result.threadReadInterrupted",
  "result.entryCount": "events.result.entryCount",
  "result.matchCount": "events.result.matchCount"
};

export function resolveEventText(copy: EventText, t: Translate): string {
  return copy.source === "app"
    ? t(APP_EVENT_TEXT_KEYS[copy.kind], copy.values)
    : copy.value;
}
