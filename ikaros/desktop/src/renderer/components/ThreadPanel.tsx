import { ArrowLeft, ArrowUpRight } from "lucide-react";

import { useTranslation } from "../i18n";
import { selectThreadById, useAppStore } from "../store";
import { EventFeed } from "./EventFeed";

/** Read a second Thread (for example a subagent session) beside the main conversation. */
export function ThreadPanel() {
  const { t } = useTranslation();
  const panelThreadId = useAppStore((state) => state.panelThreadId);
  const thread = useAppStore(selectThreadById(panelThreadId));
  const selectThread = useAppStore((state) => state.selectThread);
  const closeThreadPanel = useAppStore((state) => state.closeThreadPanel);
  if (!panelThreadId || !thread) return null;

  return (
    <aside
      aria-label={t("panel.session")}
      className="flex h-full w-[min(460px,42vw)] shrink-0 flex-col border-l border-[var(--separator)] bg-[var(--canvas)]"
    >
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-[var(--separator)] px-2">
        <button
          type="button"
          aria-label={t("common.close")}
          title={t("common.close")}
          onClick={closeThreadPanel}
          className="flex size-7 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] transition-colors hover:bg-[var(--panel-hover)] hover:text-[var(--text)]"
        >
          <ArrowLeft size={15} aria-hidden="true" />
        </button>
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium leading-4 text-[var(--text)]">
          {thread.title || t("panel.untitled")}
        </span>
        <button
          type="button"
          onClick={() => {
            void selectThread(thread.id);
            closeThreadPanel();
          }}
          className="inline-flex h-7 shrink-0 items-center gap-1 rounded-lg px-2 text-[11px] font-medium text-[var(--muted-strong)] transition-colors hover:bg-[var(--panel-hover)] hover:text-[var(--text)]"
        >
          <ArrowUpRight size={12} aria-hidden="true" />
          {t("panel.openInMain")}
        </button>
      </header>
      <div className="conversation-stage relative min-h-0 flex-1">
        <EventFeed threadId={thread.id} bottomClearance={12} />
      </div>
    </aside>
  );
}