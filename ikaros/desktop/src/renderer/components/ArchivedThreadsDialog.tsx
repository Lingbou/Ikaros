import * as Dialog from "@radix-ui/react-dialog";
import { Archive, LoaderCircle, RotateCcw, X } from "lucide-react";
import { useEffect, useState } from "react";

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
  const loadArchivedThreads = useAppStore((state) => state.loadArchivedThreads);
  const unarchiveThread = useAppStore((state) => state.unarchiveThread);
  const [restoring, setRestoring] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    if (!open) {
      setRestoring(new Set());
      return;
    }
    void loadArchivedThreads().catch(() => undefined);
  }, [loadArchivedThreads, open]);

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

          <div className="app-scrollbar min-h-[180px] overflow-y-auto p-3">
            {status === "loading" && archivedThreads.length === 0 ? (
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
              </div>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
