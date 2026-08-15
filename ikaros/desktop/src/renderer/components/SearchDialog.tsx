import * as Dialog from "@radix-ui/react-dialog";
import { Folder, LoaderCircle, MessageSquare, RotateCcw, Search, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { findProjectForThread } from "../domain";
import { useTranslation } from "../i18n";
import { useAppStore } from "../store";
import { IconButton } from "./ui";

export function SearchDialog() {
  const open = useAppStore((state) => state.searchOpen);

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(nextOpen) => useAppStore.getState().setSearchOpen(nextOpen)}
    >
      {open ? <SearchDialogContent /> : null}
    </Dialog.Root>
  );
}

function SearchDialogContent() {
  const { t } = useTranslation();
  const setOpen = useAppStore((state) => state.setSearchOpen);
  const projects = useAppStore((state) => state.projects);
  const threads = useAppStore((state) => state.threads);
  const selectThread = useAppStore((state) => state.selectThread);
  const searchCatalogStatus = useAppStore((state) => state.searchCatalogStatus);
  const searchCatalogError = useAppStore((state) => state.searchCatalogError);
  const loadAllThreadsForSearch = useAppStore(
    (state) => state.loadAllThreadsForSearch,
  );
  const [query, setQuery] = useState("");
  useEffect(() => {
    void loadAllThreadsForSearch();
  }, [loadAllThreadsForSearch]);
  const matches = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    return threads
      .filter((thread) => !normalized || thread.title.toLowerCase().includes(normalized))
      .map((thread) => ({ project: findProjectForThread(projects, thread), thread }));
  }, [projects, query, threads]);
  const openThread = (threadId: string) => {
    void selectThread(threadId);
    setOpen(false);
  };

  return (
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-[110] bg-black/55 backdrop-blur-[2px]" />
      <Dialog.Content className="glass-menu fixed left-1/2 top-[16%] z-[120] w-[min(620px,calc(100vw-28px))] -translate-x-1/2 rounded-2xl p-0">
        <Dialog.Title className="sr-only">{t("search.title")}</Dialog.Title>
        <div className="flex h-14 items-center gap-3 border-b border-[var(--border-soft)] px-4">
          <Search size={17} className="text-[var(--muted)]" />
          <input
            autoFocus
            aria-label={t("search.title")}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (
                event.key === "Enter" &&
                !event.nativeEvent.isComposing &&
                matches[0]
              ) {
                event.preventDefault();
                openThread(matches[0].thread.id);
              }
            }}
            placeholder={t("search.title")}
            className="min-w-0 flex-1 bg-transparent text-[14px] text-[var(--text)] outline-none placeholder:text-[var(--muted)]"
          />
          <IconButton label={t("search.close")} onClick={() => setOpen(false)}>
            <X size={15} />
          </IconButton>
        </div>
        <div className="app-scrollbar max-h-[430px] overflow-y-auto p-2">
          {matches.length ? (
            matches.map(({ project, thread }) => (
              <button
                key={thread.id}
                type="button"
                onClick={() => openThread(thread.id)}
                className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left hover:bg-[var(--surface-hover)]"
              >
                <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-[var(--panel-hover)] text-[var(--muted-strong)]">
                  <MessageSquare size={14} />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[12px] leading-5 text-[var(--text)]">{thread.title}</span>
                  <span className="mt-0.5 flex items-center gap-1 text-[11px] leading-4 text-[var(--muted)]">
                    {project ? (
                      <Folder size={10} style={{ color: project.color }} />
                    ) : (
                      <MessageSquare size={10} />
                    )}
                    {project?.name ?? t("common.chat")}
                  </span>
                </span>
              </button>
            ))
          ) : (
            <div className="px-4 py-12 text-center text-[12px] text-[var(--muted)]">
              {t("search.noMatches", { query })}
            </div>
          )}
        </div>
        <div className="flex min-h-10 items-center justify-between gap-3 border-t border-[var(--border-soft)] px-4 py-1.5 text-[11px] text-[var(--muted)]">
          <span>
            {matches.length === 1
              ? t("search.countOne")
              : t("search.countMany", { count: matches.length })}
          </span>
          {searchCatalogStatus === "loading" ? (
            <span role="status" className="flex items-center gap-1.5">
              <LoaderCircle size={11} className="animate-spin" aria-hidden="true" />
              {t("search.loadingAll")}
            </span>
          ) : searchCatalogStatus === "error" ? (
            <button
              type="button"
              title={searchCatalogError ?? undefined}
              onClick={() => void loadAllThreadsForSearch()}
              className="flex items-center gap-1.5 rounded-md px-1.5 py-1 text-[#d98b8b] transition-colors hover:bg-[var(--surface-hover)] hover:text-[#e7a1a1]"
            >
              <RotateCcw size={10} aria-hidden="true" />
              {t("search.retryAll")}
            </button>
          ) : (
            <span>{t("search.hint")}</span>
          )}
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  );
}
