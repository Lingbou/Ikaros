import * as Dialog from "@radix-ui/react-dialog";
import { Archive, LoaderCircle, RotateCcw, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { useTranslation } from "../i18n";
import { useAppStore } from "../store";

interface ArchivedThreadsDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
}

export function ArchivedThreadsDialog({
  open,
  onOpenChange,
}: ArchivedThreadsDialogProps) {
  const { t } = useTranslation();
  const archivedThreads = useAppStore((state) => state.archivedThreads);
  const status = useAppStore((state) => state.archivedCatalogStatus);
  const runtimeError = useAppStore((state) => state.runtimeError);
  const runtimeIssue = useAppStore((state) => state.runtimeIssue);
  const loadArchivedThreads = useAppStore((state) => state.loadArchivedThreads);
  const archivedCatalogHasMore = useAppStore(
    (state) => state.archivedCatalogHasMore,
  );
  const archivedCatalogMoreStatus = useAppStore(
    (state) => state.archivedCatalogMoreStatus,
  );
  const archivedCatalogMoreError = useAppStore(
    (state) => state.archivedCatalogMoreError,
  );
  const loadMoreArchivedThreads = useAppStore(
    (state) => state.loadMoreArchivedThreads,
  );
  const unarchiveThread = useAppStore((state) => state.unarchiveThread);
  const [restoring, setRestoring] = useState<ReadonlySet<string>>(new Set());
  const continuationSentinelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) {
      setRestoring(new Set());
      return;
    }
    void loadArchivedThreads().catch(() => undefined);
  }, [loadArchivedThreads, open]);

  useEffect(() => {
    const sentinel = continuationSentinelRef.current;
    if (
      !open ||
      !sentinel ||
      !archivedCatalogHasMore ||
      archivedCatalogMoreStatus !== "idle" ||
      typeof IntersectionObserver === "undefined"
    ) {
      return;
    }
    const viewport = sentinel.closest<HTMLElement>("[data-archived-viewport]");
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          void loadMoreArchivedThreads();
        }
      },
      { root: viewport, rootMargin: "0px 0px 72px" },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [
    archivedCatalogHasMore,
    archivedCatalogMoreStatus,
    loadMoreArchivedThreads,
    open,
  ]);

  const restore = async (threadId: string) => {
    setRestoring((current) => new Set(current).add(threadId));
    try {
      await unarchiveThread(threadId);
    } catch {
      return;
    } finally {
      setRestoring((current) => {
        const next = new Set(current);
        next.delete(threadId);
        return next;
      });
    }
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[130] bg-black/55 backdrop-blur-[2px]" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[140] flex max-h-[min(620px,calc(100vh-40px))] w-[min(560px,calc(100vw-28px))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-[18px] border border-[var(--border-soft)] bg-[var(--menu)] shadow-[0_24px_70px_var(--shadow-color)]">
          <div className="flex items-center gap-3 border-b border-[var(--border-soft)] px-5 py-4">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-[10px] bg-[var(--panel-raised)] text-[var(--muted-strong)]">
              <Archive size={15} aria-hidden="true" />
            </div>
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-[15px] font-semibold leading-5 text-[var(--text)]">
                {t("settings.archivedChats")}
              </Dialog.Title>
              <Dialog.Description className="mt-0.5 text-[11px] leading-4 text-[var(--muted)]">
                {t("settings.archivedChatsDescription")}
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <button
                type="button"
                aria-label={t("common.close")}
                className="flex size-7 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
              >
                <X size={13} aria-hidden="true" />
              </button>
            </Dialog.Close>
          </div>

          <div
            data-archived-viewport
            className="app-scrollbar min-h-[180px] overflow-y-auto p-3"
          >
            {status === "error" ? (
              <div role="alert" className="flex h-40 flex-col items-center justify-center px-6 text-center">
                <Archive size={20} aria-hidden="true" className="text-[#e07070]" />
                <div className="mt-2 text-[12px] font-medium leading-[18px] text-[var(--text)]">
                  {t("runtime.error.archivedCatalog")}
                </div>
                {runtimeIssue?.kind === "archived_catalog" && runtimeIssue.message === runtimeError ? (
                  <div className="mt-1 max-w-sm break-words text-[10px] leading-4 text-[var(--muted)]">
                    {runtimeIssue.message}
                  </div>
                ) : null}
                <button
                  type="button"
                  onClick={() => void loadArchivedThreads().catch(() => undefined)}
                  className="mt-3 flex h-8 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--panel)] px-3 text-[11px] font-medium text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
                >
                  <RotateCcw size={11} aria-hidden="true" />
                  {t("common.retry")}
                </button>
              </div>
            ) : status === "loading" && archivedThreads.length === 0 ? (
              <div className="flex h-40 items-center justify-center text-[var(--muted)]">
                <LoaderCircle size={17} className="animate-spin" aria-label={t("common.loading")} />
              </div>
            ) : archivedThreads.length === 0 ? (
              <div className="flex h-40 flex-col items-center justify-center text-center">
                <Archive size={20} aria-hidden="true" className="text-[var(--muted)]" />
                <div className="mt-2 text-[12px] font-medium leading-[18px] text-[var(--text)]">
                  {t("settings.noArchivedChats")}
                </div>
              </div>
            ) : (
              <div className="space-y-1">
                {archivedThreads.map((thread) => {
                  const isRestoring = restoring.has(thread.id);
                  const restoreError =
                    runtimeIssue?.kind === "unarchive" &&
                    runtimeIssue.threadId === thread.id &&
                    runtimeIssue.message === runtimeError;
                  return (
                    <div
                      key={thread.id}
                      className="flex min-h-12 items-center gap-3 rounded-[10px] px-3 py-2 transition-colors hover:bg-[var(--surface-hover)]"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[12px] font-medium leading-[18px] text-[var(--text)]">
                          {thread.title || t("sidebar.newChat")}
                        </div>
                        {thread.workspace ? (
                          <div className="truncate text-[10px] leading-4 text-[var(--muted)]">
                            {thread.workspace.name}
                          </div>
                        ) : null}
                        {restoreError ? (
                          <div title={runtimeIssue.message} className="truncate text-[10px] leading-4 text-[#e07070]">
                            {t("runtime.error.unarchive")}
                          </div>
                        ) : null}
                      </div>
                      <button
                        type="button"
                        disabled={isRestoring}
                        onClick={() => void restore(thread.id)}
                        className="flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-[11px] font-medium leading-4 text-[var(--muted-strong)] outline-none transition-colors hover:bg-[var(--panel-raised)] hover:text-[var(--text)] disabled:cursor-wait disabled:opacity-50"
                      >
                        {isRestoring ? (
                          <LoaderCircle size={12} className="animate-spin" aria-hidden="true" />
                        ) : (
                          <RotateCcw size={12} aria-hidden="true" />
                        )}
                        {t("settings.unarchive")}
                      </button>
                    </div>
                  );
                })}
                <div
                  ref={continuationSentinelRef}
                  aria-live="polite"
                  className="flex min-h-9 items-center justify-center"
                >
                  {archivedCatalogMoreStatus === "loading" ? (
                    <span role="status" className="flex items-center gap-1.5 text-[10px] text-[var(--muted)]">
                      <LoaderCircle size={11} className="animate-spin" aria-hidden="true" />
                      {t("common.loading")}
                    </span>
                  ) : archivedCatalogMoreStatus === "error" ? (
                    <button
                      type="button"
                      title={archivedCatalogMoreError ?? undefined}
                      onClick={() => void loadMoreArchivedThreads()}
                      className="flex h-7 items-center gap-1.5 rounded-lg px-2.5 text-[10px] font-medium text-[var(--muted)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
                    >
                      <RotateCcw size={10} aria-hidden="true" />
                      {t("common.retry")}
                    </button>
                  ) : null}
                </div>
              </div>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
