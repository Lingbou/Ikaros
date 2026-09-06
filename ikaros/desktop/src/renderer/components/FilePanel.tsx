import { FileText, RefreshCw, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";

import type {
  RuntimeFileChangeResult,
  RuntimeFilePreviewResult,
  RuntimeFilePreviewText,
  RuntimeFileRevisionMetadata,
} from "../../shared/runtime";
import { type TranslationKey, useTranslation } from "../i18n";
import { createRuntimeClient } from "../runtimeClient";
import { type RuntimeFileSelection, useAppStore } from "../store";
import { cx } from "./ui";

const UNAVAILABLE_KEYS = {
  file_not_found: "files.reason.fileNotFound",
  not_a_file: "files.reason.notAFile",
  binary_file: "files.reason.binaryFile",
  unsupported_encoding: "files.reason.unsupportedEncoding",
  too_large: "files.reason.tooLarge",
  scan_limit: "files.reason.scanLimit",
  revision_changed: "files.reason.revisionChanged",
  read_failed: "files.reason.readFailed",
  protected_content: "files.reason.protectedContent",
  not_recorded: "files.reason.notRecorded",
  result_unknown: "files.reason.resultUnknown",
} satisfies Record<string, TranslationKey>;

const buttonClass = "rounded-md px-2.5 py-1.5 text-[11px] font-medium text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)] disabled:opacity-40";

function logicalLines(content: string): string[] {
  if (!content) return [];
  const lines = content.split(/\r\n|\n|\r/);
  if (/[\r\n]$/.test(content)) lines.pop();
  return lines;
}

function diffLines(diff: string) {
  let before: number | null = null;
  let after: number | null = null;
  return logicalLines(diff).map((text) => {
    const hunk = text.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if (hunk) {
      before = Number(hunk[1]);
      after = Number(hunk[2]);
      return { text, before: null, after: null };
    }
    const row = { text, before: null as number | null, after: null as number | null };
    if (before !== null && after !== null) {
      if (text.startsWith("-")) row.before = before++;
      else if (text.startsWith("+")) row.after = after++;
      else if (text.startsWith(" ")) { row.before = before++; row.after = after++; }
    }
    return row;
  });
}

function CurrentFile({ selection, onReference }: {
  selection: RuntimeFileSelection;
  onReference: (reference: string) => void;
}) {
  const { t } = useTranslation();
  const runtime = useMemo(() => createRuntimeClient(), []);
  const generation = useRef(0);
  const [page, setPage] = useState<RuntimeFilePreviewText | null>(null);
  const [offsets, setOffsets] = useState<number[]>([1]);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<TranslationKey | null>(null);
  const [blocked, setBlocked] = useState(false);
  const [stale, setStale] = useState(false);

  async function load(nextOffsets: number[], revision?: string) {
    const request = ++generation.current;
    setBusy(true);
    setProblem(null);
    try {
      if (!runtime) throw new Error("Runtime unavailable");
      const result: RuntimeFilePreviewResult = await runtime.previewFile({
        threadId: selection.threadId,
        path: selection.path,
        ...(selection.sourceToolCallItemId ? { sourceToolCallItemId: selection.sourceToolCallItemId } : {}),
        offset: nextOffsets.at(-1) ?? 1,
        ...(revision ? { expectedRevision: revision } : {}),
      });
      if (request !== generation.current) return;
      if (result.status === "unavailable") {
        setProblem(UNAVAILABLE_KEYS[result.reason]);
        setBlocked(true);
        // A rejected later page can leave the already read page visible. A
        // refresh must never relabel old content as current, especially when
        // the Runtime now withholds protected or unsupported contents.
        const retainPage = revision !== undefined &&
          (result.reason === "revision_changed" || result.reason === "scan_limit");
        if (!retainPage) setPage(null);
        setStale(result.reason === "revision_changed");
      } else {
        setPage(result);
        setOffsets(nextOffsets);
        setBlocked(false);
        setStale(false);
      }
    } catch {
      if (request === generation.current) {
        setProblem("files.loadFailed");
        if (revision === undefined) setPage(null);
      }
    } finally {
      if (request === generation.current) setBusy(false);
    }
  }

  useEffect(() => {
    if (selection.path) void load([1]);
    return () => { generation.current += 1; };
  }, [selection]);

  if (!selection.path) {
    return <p className="p-4 text-[12px] leading-5 text-[var(--muted)]">{t("files.pathHint")}</p>;
  }

  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-1 border-b border-[var(--border-soft)] px-3 py-2">
        <span className="text-[11px] text-[var(--muted)]">{t("files.currentHint")}</span>
        <button type="button" disabled={busy} onClick={() => void load([1])} className={cx(buttonClass, "flex items-center gap-1")}>
          <RefreshCw size={11} aria-hidden="true" />{t("files.refresh")}
        </button>
      </div>
      {problem ? <p role="alert" className="px-4 py-3 text-[11px] leading-[17px] text-[#d99a77]">{t(problem)}</p> : null}
      {busy ? <p role="status" className="px-4 py-2 text-[11px] text-[var(--muted)]">{t("common.loading")}</p> : null}
      {page ? (
        <>
          <div className="flex items-center justify-between gap-2 px-4 py-2 text-[11px] text-[var(--muted)]">
            <span>{page.lineEnd === 0 ? t("files.lineCount", { count: 0 }) : t("files.lines", { start: page.lineStart, end: page.lineEnd })}</span>
            <span>{page.bom ? "UTF-8 BOM" : "UTF-8"}</span>
          </div>
          <div role="region" aria-label={t("files.content")} tabIndex={0} className="app-scrollbar min-h-0 flex-1 overflow-auto border-y border-[var(--border-soft)] bg-[var(--canvas)] py-2">
            {page.content ? (
              <div data-file-content className="min-w-max font-mono text-[11px] leading-[19px]">
                {logicalLines(page.content).map((line, index) => (
                  <div key={page.lineStart + index} data-line-number={page.lineStart + index} className="flex">
                    <span aria-hidden="true" className="sticky left-0 w-14 shrink-0 select-none bg-[var(--canvas)] pr-3 text-right text-[var(--muted)]">{page.lineStart + index}</span>
                    <span className="whitespace-pre pr-4 text-[var(--text)]">{line}</span>
                  </div>
                ))}
              </div>
            ) : <p className="px-4 py-3 text-[11px] text-[var(--muted)]">{t("files.empty")}</p>}
          </div>
          {page.truncated ? <p className="px-4 pt-2 text-[11px] leading-4 text-[var(--muted)]">{t(page.truncationReason === "scan_limit" ? "files.scanLimitedPage" : "files.moreContent")}</p> : null}
          <div className="flex flex-wrap items-center justify-between gap-1 px-3 py-2">
            <div className="flex gap-1">
              <button type="button" disabled={busy || stale || offsets.length <= 1} onClick={() => void load(offsets.slice(0, -1), page.revision)} className={buttonClass}>{t("files.previous")}</button>
              <button type="button" disabled={busy || blocked || page.nextOffset === null} onClick={() => { if (page.nextOffset !== null) void load([...offsets, page.nextOffset], page.revision); }} className={buttonClass}>{t("files.next")}</button>
            </div>
            <button type="button" disabled={busy || stale} onClick={() => onReference(page.lineEnd === 0 ? t("files.referenceFileText", { path: page.path }) : t("files.referenceText", { path: page.path, start: page.lineStart, end: page.lineEnd }))} className={buttonClass}>{t("files.reference")}</button>
          </div>
        </>
      ) : null}
    </>
  );
}

function RevisionMetadata({ value, label }: { value: RuntimeFileRevisionMetadata; label: string }) {
  const { t } = useTranslation();
  const details = !value.exists ? [t("files.absent")] : [
    value.byteCount === null ? null : t("files.bytes", { count: value.byteCount }),
    value.lineCount === null ? null : t("files.lineCount", { count: value.lineCount }),
    value.encoding === "utf-8" ? (value.bom ? "UTF-8 BOM" : "UTF-8") : null,
    value.newline === "mixed" ? t("files.mixedNewlines") : value.newline?.toUpperCase(),
  ].filter(Boolean);
  return <div><span className="font-medium text-[var(--muted-strong)]">{label}</span><p className="mt-0.5">{details.length ? details.join(" · ") : t("files.metadataUnavailable")}</p></div>;
}

function FileChange({ selection, onReference }: {
  selection: RuntimeFileSelection;
  onReference: (reference: string) => void;
}) {
  const { t, language } = useTranslation();
  const runtime = useMemo(() => createRuntimeClient(), []);
  const generation = useRef(0);
  const [result, setResult] = useState<RuntimeFileChangeResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  async function load() {
    const request = ++generation.current;
    setBusy(true);
    setFailed(false);
    try {
      if (!runtime || !selection.toolCallItemId) throw new Error("File change unavailable");
      const value = await runtime.getFileChange({ threadId: selection.threadId, toolCallItemId: selection.toolCallItemId });
      if (request === generation.current) setResult(value);
    } catch {
      if (request === generation.current) {
        setResult(null);
        setFailed(true);
      }
    } finally {
      if (request === generation.current) setBusy(false);
    }
  }

  useEffect(() => {
    void load();
    return () => { generation.current += 1; };
  }, [selection]);

  const unavailableKey = result?.status === "unavailable" ? UNAVAILABLE_KEYS[result.reason] : null;

  return (
    <>
      <p className="border-b border-[var(--border-soft)] px-4 py-3 text-[11px] leading-4 text-[var(--muted)]">{t("files.changeHint")}</p>
      {busy ? <p role="status" className="px-4 py-3 text-[11px] text-[var(--muted)]">{t("common.loading")}</p> : null}
      {failed || unavailableKey ? (
        <div className="p-4">
          <p role="alert" className="text-[11px] leading-[17px] text-[#d99a77]">{t(failed ? "files.loadFailed" : unavailableKey ?? "files.loadFailed")}</p>
          <button type="button" disabled={busy} onClick={() => void load()} className={cx(buttonClass, "mt-2")}>{t("files.refresh")}</button>
        </div>
      ) : null}
      {result ? (
        <div className="grid grid-cols-2 gap-4 border-b border-[var(--border-soft)] px-4 py-3 text-[11px] leading-4 text-[var(--muted)]">
          {result.before ? <RevisionMetadata label={t("files.before")} value={result.before} /> : null}
          {result.after ? <RevisionMetadata label={t("files.after")} value={result.after} /> : null}
        </div>
      ) : null}
      {result?.status === "recorded" ? (
        <>
          <div className="flex items-center justify-between gap-2 px-4 py-2 text-[11px] text-[var(--muted)]">
            <time dateTime={result.recordedAt}>{new Date(result.recordedAt).toLocaleString(language === "zh-CN" ? "zh-CN" : "en")}</time>
            <span><span className="text-[#72d3a7]">+{result.additions}</span> / <span className="text-[#ff8585]">−{result.deletions}</span></span>
          </div>
          <div role="region" aria-label={t("files.diff")} tabIndex={0} className="app-scrollbar min-h-0 flex-1 overflow-auto border-y border-[var(--border-soft)] bg-[var(--canvas)] py-2">
            {result.diff ? (
              <pre className="min-w-max font-mono text-[11px] leading-[19px]">
                {diffLines(result.diff).map((line, index) => (
                  <div key={index} className={cx("flex pr-4", line.text.startsWith("@@") ? "text-[var(--accent)]" : line.after !== null && line.before === null ? "bg-[#72d3a7]/10 text-[#72d3a7]" : line.before !== null && line.after === null ? "bg-[#ff8585]/10 text-[#ff8585]" : "text-[var(--muted-strong)]")}>
                    <span aria-hidden="true" className="w-11 shrink-0 select-none pr-2 text-right text-[var(--muted)]">{line.before}</span>
                    <span aria-hidden="true" className="mr-3 w-11 shrink-0 select-none pr-2 text-right text-[var(--muted)]">{line.after}</span>
                    <span>{line.text}</span>
                  </div>
                ))}
              </pre>
            ) : <p className="px-4 py-3 text-[11px] leading-5 text-[var(--muted)]">{t(result.before.revision !== result.after.revision || result.before.exists !== result.after.exists ? "files.formatOnly" : "files.noChange")}</p>}
          </div>
          <div className="flex justify-end px-3 py-2">
            <button type="button" onClick={() => onReference(t("files.changeReferenceText", { path: result.path, time: new Date(result.recordedAt).toLocaleString(language === "zh-CN" ? "zh-CN" : "en") }))} className={buttonClass}>{t("files.reference")}</button>
          </div>
        </>
      ) : null}
    </>
  );
}

function FilePanelContent({ selection }: { selection: RuntimeFileSelection }) {
  const { t } = useTranslation();
  const openFile = useAppStore((state) => state.openFile);
  const closeFile = useAppStore((state) => state.closeFile);
  const [path, setPath] = useState(selection.path);

  function openPath(event: FormEvent) {
    event.preventDefault();
    if (path.trim()) openFile(path.trim() === selection.path
      ? { ...selection, view: "current" }
      : { threadId: selection.threadId, path: path.trim(), view: "current" });
  }
  function reference(value: string) {
    const state = useAppStore.getState();
    if (state.selectedThreadId !== selection.threadId) return;
    state.setDraft(state.draft ? `${state.draft}\n\n${value}` : value);
    window.requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>("textarea")?.focus());
  }

  return (
    <aside aria-label={t("files.title")} className="file-panel flex min-h-0 shrink-0 flex-col border-l border-[var(--separator)] bg-[var(--panel)]">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-[var(--border-soft)] px-4">
        <FileText size={13} aria-hidden="true" className="text-[var(--muted)]" />
        <h2 className="flex-1 text-[12px] text-[var(--text)]">{t("files.title")}</h2>
        <button type="button" aria-label={t("files.close")} onClick={closeFile} className={buttonClass}><X size={14} /></button>
      </div>
      <form onSubmit={openPath} className="flex gap-2 border-b border-[var(--border-soft)] p-3">
        <input autoFocus={!selection.path} aria-label={t("files.path")} placeholder={t("files.pathPlaceholder")} value={path} onChange={(event) => setPath(event.currentTarget.value)} maxLength={4096} className="min-w-0 flex-1 rounded-md border border-[var(--border)] bg-[var(--canvas)] px-2 py-1.5 font-mono text-[11px] text-[var(--text)]" />
        <button type="submit" disabled={!path.trim()} className={buttonClass}>{t("files.openPath")}</button>
      </form>
      {selection.toolCallItemId ? (
        <div role="tablist" aria-label={t("files.views")} className="flex shrink-0 gap-1 border-b border-[var(--border-soft)] px-3 py-1.5">
          {(["current", "change"] as const).map((view) => <button key={view} type="button" role="tab" aria-selected={selection.view === view} onClick={() => openFile({ ...selection, view })} className={cx(buttonClass, selection.view === view && "bg-[var(--surface-hover)] text-[var(--text)]")}>{t(view === "current" ? "files.current" : "files.change")}</button>)}
        </div>
      ) : null}
      {selection.view === "change" && selection.toolCallItemId ? <FileChange selection={selection} onReference={reference} /> : <CurrentFile selection={selection} onReference={reference} />}
    </aside>
  );
}

export function FilePanel() {
  const selection = useAppStore((state) => state.fileSelection);
  const threadId = useAppStore((state) => state.selectedThreadId);
  const closeFile = useAppStore((state) => state.closeFile);
  useEffect(() => {
    if (selection && selection.threadId !== threadId) closeFile();
  }, [selection, threadId, closeFile]);
  if (!selection || selection.threadId !== threadId) return null;
  return <FilePanelContent key={`${selection.threadId}:${selection.path}:${selection.toolCallItemId ?? ""}:${selection.sourceToolCallItemId ?? ""}`} selection={selection} />;
}
