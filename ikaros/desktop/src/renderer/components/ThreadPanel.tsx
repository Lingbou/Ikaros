import { ArrowLeft, ArrowUpRight } from "lucide-react";
import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import {
  DEFAULT_THREAD_PANEL_WIDTH,
  MAX_THREAD_PANEL_WIDTH,
  MIN_THREAD_PANEL_WIDTH,
  THREAD_PANEL_MAIN_MIN_WIDTH,
} from "../../shared/platform";
import { useTranslation } from "../i18n";
import { selectThreadById, useAppStore } from "../store";
import { EventFeed } from "./EventFeed";

const PANEL_KEYBOARD_STEP = 16;
const PANEL_KEYBOARD_LARGE_STEP = 64;

function clampThreadPanelWidth(width: number): number {
  const viewport =
    typeof window === "undefined" ? MAX_THREAD_PANEL_WIDTH : window.innerWidth;
  const upper = Math.max(
    MIN_THREAD_PANEL_WIDTH,
    Math.min(MAX_THREAD_PANEL_WIDTH, viewport - THREAD_PANEL_MAIN_MIN_WIDTH),
  );
  return Math.round(Math.min(Math.max(width, MIN_THREAD_PANEL_WIDTH), upper));
}

/** Read a second Thread (for example a subagent session) beside the main conversation. */
export function ThreadPanel() {
  const { t } = useTranslation();
  const panelThreadId = useAppStore((state) => state.panelThreadId);
  const thread = useAppStore(selectThreadById(panelThreadId));
  const selectThread = useAppStore((state) => state.selectThread);
  const closeThreadPanel = useAppStore((state) => state.closeThreadPanel);
  const threadPanelWidth = useAppStore((state) => state.threadPanelWidth);
  const setThreadPanelWidth = useAppStore((state) => state.setThreadPanelWidth);
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startWidth: number;
  } | null>(null);
  const [resizing, setResizing] = useState(false);

  useEffect(
    () => () => {
      dragRef.current = null;
      delete document.documentElement.dataset.resizingThreadPanel;
    },
    [],
  );

  if (!panelThreadId || !thread) return null;

  const stopResizing = () => {
    setResizing(false);
    delete document.documentElement.dataset.resizingThreadPanel;
  };

  const beginResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: threadPanelWidth,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    setResizing(true);
    document.documentElement.dataset.resizingThreadPanel = "true";
  };

  const previewResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    setThreadPanelWidth(
      clampThreadPanelWidth(drag.startWidth - (event.clientX - drag.startX)),
    );
  };

  const finishResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    stopResizing();
  };

  const cancelResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setThreadPanelWidth(drag.startWidth);
    stopResizing();
  };

  const resizeWithKeyboard = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? PANEL_KEYBOARD_LARGE_STEP : PANEL_KEYBOARD_STEP;
    const next =
      event.key === "ArrowLeft"
        ? threadPanelWidth + step
        : event.key === "ArrowRight"
          ? threadPanelWidth - step
          : event.key === "Home"
            ? MIN_THREAD_PANEL_WIDTH
            : event.key === "End"
              ? MAX_THREAD_PANEL_WIDTH
              : null;
    if (next === null) return;
    event.preventDefault();
    setThreadPanelWidth(clampThreadPanelWidth(next));
  };

  return (
    <aside
      aria-label={t("panel.session")}
      data-resizing={resizing || undefined}
      className="relative flex h-full shrink-0 flex-col border-l border-[var(--separator)] bg-[var(--canvas)]"
      style={{ width: threadPanelWidth }}
    >
      <div
        role="separator"
        aria-label={t("panel.resize")}
        aria-orientation="vertical"
        aria-valuemin={MIN_THREAD_PANEL_WIDTH}
        aria-valuemax={MAX_THREAD_PANEL_WIDTH}
        aria-valuenow={threadPanelWidth}
        tabIndex={0}
        data-resizing={resizing || undefined}
        className="thread-panel-resizer"
        onPointerDown={beginResize}
        onPointerMove={previewResize}
        onPointerUp={finishResize}
        onPointerCancel={cancelResize}
        onLostPointerCapture={() => {
          dragRef.current = null;
          stopResizing();
        }}
        onDoubleClick={(event) => {
          event.preventDefault();
          setThreadPanelWidth(DEFAULT_THREAD_PANEL_WIDTH);
        }}
        onKeyDown={resizeWithKeyboard}
      />
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