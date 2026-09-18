import { useVirtualizer } from "@tanstack/react-virtual";
import { AlertCircle, LoaderCircle, RotateCcw } from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { activeBranch, type AgentEvent, type Turn } from "../domain";
import { useTranslation } from "../i18n";
import { selectCurrentThread, useAppStore } from "../store";
import { EventCard } from "./EventCard";
import { RuntimeRunProgress } from "./RuntimeRunProgress";
import { RuntimeTurnOutcome } from "./RuntimeTurnOutcome";
import { cx } from "./ui";
import {
  getTurnAnchors,
  MIN_TURN_NAV_ITEMS,
  TurnNavigator,
} from "./TurnNavigator";

function EmptyConversation() {
  const { t } = useTranslation();
  return (
    <div className="absolute inset-0 flex items-center justify-center pb-[120px]">
      <div className="mx-auto flex max-w-xl flex-col items-center px-6 text-center">
        <h1 className="text-[24px] font-medium leading-[30px] text-[var(--text)]">
          {t("empty.heading")}
        </h1>
        <p className="mt-2 max-w-md text-[12px] leading-[18px] text-[var(--muted)]">
          {t("empty.description")}
        </p>
      </div>
    </div>
  );
}

function ConversationLoadState({
  error,
  onRetry,
}: {
  error?: string;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="absolute inset-0 flex items-center justify-center pb-[120px]">
      {error ? (
        <div role="alert" className="mx-auto flex max-w-md flex-col items-center px-6 text-center">
          <AlertCircle size={18} aria-hidden="true" className="text-[#e07070]" />
          <h2 className="mt-2 text-[13px] font-medium leading-5 text-[var(--text)]">
            {t("runtime.error.history")}
          </h2>
          <p className="mt-1 max-w-sm break-words text-[11px] leading-[17px] text-[var(--muted)]">
            {error}
          </p>
          {onRetry ? (
            <button
              type="button"
              onClick={onRetry}
              className="mt-3 flex h-8 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--panel)] px-3 text-[11px] font-medium text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
            >
              <RotateCcw size={11} aria-hidden="true" />
              {t("common.retry")}
            </button>
          ) : null}
        </div>
      ) : (
        <div role="status" className="flex items-center gap-2 text-[11px] text-[var(--muted)]">
          <LoaderCircle size={13} aria-hidden="true" className="animate-spin text-[var(--accent)]" />
          {t("runtime.historyLoading")}
        </div>
      )}
    </div>
  );
}

function blocksTurnShortcut(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    Boolean(
      target.closest(
        "input, textarea, select, [contenteditable='true'], [role='dialog']",
      ),
    )
  );
}

function toolResultRegionId(toolCallId: string) {
  return `tool-result-${encodeURIComponent(toolCallId)}`;
}

type FeedRow = AgentEvent | {
  type: "runtime_outcome";
  id: string;
  turn: Turn;
};

function eventRowVisualOffset(viewport: HTMLElement, eventId: string): number | null {
  const row = [...viewport.querySelectorAll<HTMLElement>("[data-event-id]")].find(
    (candidate) => candidate.dataset.eventId === eventId,
  );
  const match = row?.style.transform.match(/translateY\(([-\d.]+)px\)/);
  return match ? Number(match[1]) : null;
}

export function EventFeed({ bottomClearance }: { bottomClearance: number }) {
  const { t } = useTranslation();
  const thread = useAppStore(selectCurrentThread);
  const runtimeMode = useAppStore((state) => state.runtimeMode);
  const detail = useAppStore((state) =>
    thread ? state.runtimeThreadDetails[thread.id] : undefined,
  );
  const retryRuntimeThread = useAppStore((state) => state.retryRuntimeThread);
  const loadOlderRuntimeTurns = useAppStore(
    (state) => state.loadOlderRuntimeTurns,
  );
  const branch = activeBranch(thread);
  const turns = branch?.turns;
  const events = useMemo<FeedRow[]>(
    () => turns?.flatMap((turn): FeedRow[] => {
      const rows: FeedRow[] = [...turn.events];
      if (
        !runtimeMode ||
        !turn.runId ||
        (!turn.runProgress && turn.status !== "failed" && turn.status !== "interrupted")
      ) {
        return rows;
      }
      let insertAt = rows.length;
      for (let index = rows.length - 1; index >= 0; index -= 1) {
        const row = rows[index];
        if (row.type === "message" && row.role === "assistant") {
          insertAt = index;
          break;
        }
      }
      rows.splice(insertAt, 0, {
        type: "runtime_outcome",
        id: `outcome:${turn.runId}`,
        turn,
      });
      return rows;
    }) ?? [],
    [runtimeMode, turns],
  );
  const toolCallIds = useMemo(
    () =>
      new Set(
        events
          .filter((event) => event.type === "tool_call")
          .map((event) => event.id),
      ),
    [events],
  );
  const toolCallIdsWithResults = useMemo(
    () =>
      new Set(
        events
          .filter((event) => event.type === "tool_result")
          .map((event) => event.toolCallId),
      ),
    [events],
  );
  const turnAnchors = useMemo(() => getTurnAnchors(turns ?? [], t), [t, turns]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const shouldFollowRef = useRef(true);
  const animatedEventIdsRef = useRef(new Set<string>());
  const animationScopeRef = useRef<string | null>(null);
  const animationScope = thread && branch ? `${thread.id}:${branch.id}` : null;
  const pendingPrependAnchorRef = useRef<{
    scope: string;
    eventId: string;
    offsetFromViewportTop: number;
  } | null>(null);
  const [toolResultExpansion, setToolResultExpansion] = useState<{
    scope: string | null;
    expandedIds: Set<string>;
  }>({ scope: null, expandedIds: new Set() });
  const [activeAnchorId, setActiveAnchorId] = useState<string | null>(null);
  if (animationScopeRef.current !== animationScope) {
    animationScopeRef.current = animationScope;
    animatedEventIdsRef.current = new Set(events.map((event) => event.id));
  }
  if (pendingPrependAnchorRef.current?.scope === animationScope) {
    for (const event of events) animatedEventIdsRef.current.add(event.id);
  }
  const virtualizer = useVirtualizer({
    count: events.length,
    getScrollElement: () => scrollRef.current,
    // ResizeObserver can notify during a React commit. Let React schedule the
    // resulting render instead of forcing flushSync from inside that lifecycle.
    useFlushSync: false,
    estimateSize: (index) => {
      const event = events[index];
      if (event?.type === "runtime_outcome") return event.turn.status === "failed" || event.turn.status === "interrupted" ? 200 : 48;
      if (event?.type === "message") return event.role === "user" ? 72 : 128;
      if (event?.type === "permission_request" || event?.type === "interrupt") return 168;
      if (
        event?.type === "tool_result" &&
        toolCallIds.has(event.toolCallId) &&
        !(
          toolResultExpansion.scope === animationScope &&
          toolResultExpansion.expandedIds.has(event.toolCallId)
        )
      ) {
        return 0;
      }
      return 92;
    },
    getItemKey: (index) => events[index]?.id ?? index,
    overscan: 7,
  });
  const totalSize = virtualizer.getTotalSize();
  const activeAnchorIndex = useMemo(() => {
    const index = activeAnchorId
      ? turnAnchors.findIndex((anchor) => anchor.id === activeAnchorId)
      : -1;
    return index >= 0 ? index : Math.max(0, turnAnchors.length - 1);
  }, [activeAnchorId, turnAnchors]);
  const isToolResultExpanded = useCallback(
    (toolCallId: string) =>
      toolResultExpansion.scope === animationScope &&
      toolResultExpansion.expandedIds.has(toolCallId),
    [animationScope, toolResultExpansion],
  );
  const toggleToolResult = useCallback(
    (toolCallId: string) => {
      setToolResultExpansion((current) => {
        const expandedIds =
          current.scope === animationScope
            ? new Set(current.expandedIds)
            : new Set<string>();
        if (expandedIds.has(toolCallId)) expandedIds.delete(toolCallId);
        else expandedIds.add(toolCallId);
        return { scope: animationScope, expandedIds };
      });
    },
    [animationScope],
  );

  useEffect(() => {
    shouldFollowRef.current = true;
    setActiveAnchorId(turnAnchors.at(-1)?.id ?? null);
  }, [branch?.id, thread?.id]);

  useEffect(() => {
    if (!shouldFollowRef.current || !turnAnchors.length) return;
    setActiveAnchorId(turnAnchors.at(-1)?.id ?? null);
  }, [turnAnchors]);

  useLayoutEffect(() => {
    const pending = pendingPrependAnchorRef.current;
    if (!pending || pending.scope !== animationScope) return;
    if (detail?.olderStatus === "error") {
      pendingPrependAnchorRef.current = null;
      return;
    }
    const nextIndex = events.findIndex((event) => event.id === pending.eventId);
    if (nextIndex < 0) return;
    const viewport = scrollRef.current;
    if (!viewport) return;
    const measuredOffset = eventRowVisualOffset(viewport, pending.eventId);
    const estimatedOffset = virtualizer.getOffsetForIndex(nextIndex, "start")?.[0];
    const nextVisualOffset = measuredOffset ??
      (estimatedOffset === undefined ? undefined : estimatedOffset + 18);
    if (nextVisualOffset === undefined) return;
    shouldFollowRef.current = false;
    viewport.scrollTo({
      top: Math.max(0, nextVisualOffset - pending.offsetFromViewportTop),
      behavior: "auto",
    });
    pendingPrependAnchorRef.current = null;
  }, [animationScope, detail?.olderStatus, events, virtualizer]);

  useEffect(() => {
    if (!events.length) return;
    // Mounted rows keep their real sizes through TanStack Virtual's ResizeObserver.
    // Calling virtualizer.measure() here would clear that cache and make stable
    // rows fall back to estimates, which lets later absolute rows overlap them.
    if (!shouldFollowRef.current) return;
    const frame = window.requestAnimationFrame(() => {
      const viewportHeight = scrollRef.current?.clientHeight ?? 0;
      const bottomOffset = Math.max(
        0,
        totalSize + bottomClearance - viewportHeight,
      );
      virtualizer.scrollToOffset(bottomOffset, { align: "start" });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [
    bottomClearance,
    branch?.id,
    events,
    thread?.id,
    totalSize,
    virtualizer,
  ]);

  const updateActiveAnchor = useCallback(
    (element: HTMLDivElement) => {
      if (!turnAnchors.length) return;
      const distanceFromBottom =
        element.scrollHeight - element.scrollTop - element.clientHeight;
      if (distanceFromBottom < 96) {
        setActiveAnchorId(turnAnchors.at(-1)?.id ?? null);
        return;
      }

      const probeOffset =
        element.scrollTop + Math.min(56, element.clientHeight * 0.12);
      const rows = virtualizer.getVirtualItems();
      const rowAtProbe =
        rows.find((row) => row.start + row.size >= probeOffset) ?? rows.at(0);
      if (!rowAtProbe) return;

      let nextAnchorIndex = 0;
      for (let index = 0; index < turnAnchors.length; index += 1) {
        if (turnAnchors[index].eventIndex > rowAtProbe.index) break;
        nextAnchorIndex = index;
      }
      const nextAnchorId = turnAnchors[nextAnchorIndex]?.id ?? null;
      setActiveAnchorId((current) =>
        current === nextAnchorId ? current : nextAnchorId,
      );
    },
    [turnAnchors, virtualizer],
  );

  const navigateToAnchor = useCallback(
    (index: number) => {
      const anchor = turnAnchors[index];
      if (!anchor) return;
      shouldFollowRef.current = index === turnAnchors.length - 1;
      setActiveAnchorId(anchor.id);
      virtualizer.scrollToIndex(anchor.eventIndex, {
        align: "start",
        behavior: "auto",
      });
    },
    [turnAnchors, virtualizer],
  );

  const requestOlderTurns = useCallback((retry = false) => {
    if (
      !thread ||
      !animationScope ||
      !detail?.hasMore ||
      detail.olderStatus === "loading" ||
      (detail.olderStatus === "error" && !retry)
    ) {
      return;
    }
    const viewport = scrollRef.current;
    const firstVisible = viewport
      ? virtualizer.getVirtualItemForOffset(viewport.scrollTop + 1)
      : undefined;
    const anchorEvent = firstVisible ? events[firstVisible.index] : events[0];
    if (viewport && anchorEvent) {
      const currentVisualOffset =
        eventRowVisualOffset(viewport, anchorEvent.id) ??
        (firstVisible?.start ?? 0) + 18;
      pendingPrependAnchorRef.current = {
        scope: animationScope,
        eventId: anchorEvent.id,
        offsetFromViewportTop: currentVisualOffset - viewport.scrollTop,
      };
    }
    shouldFollowRef.current = false;
    void loadOlderRuntimeTurns(thread.id);
  }, [
    animationScope,
    detail?.hasMore,
    detail?.olderStatus,
    events,
    loadOlderRuntimeTurns,
    thread,
    virtualizer,
  ]);

  useEffect(() => {
    if (turnAnchors.length < MIN_TURN_NAV_ITEMS) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.isComposing ||
        blocksTurnShortcut(event.target) ||
        document.querySelector("[role='dialog']") ||
        !event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey
      ) {
        return;
      }
      const direction =
        event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0;
      if (!direction) return;
      event.preventDefault();
      const nextIndex = Math.min(
        turnAnchors.length - 1,
        Math.max(0, activeAnchorIndex + direction),
      );
      navigateToAnchor(nextIndex);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeAnchorIndex, navigateToAnchor, turnAnchors.length]);

  if (!thread) return <EmptyConversation />;
  if (runtimeMode && detail?.status === "error") {
    return (
      <ConversationLoadState
        error={detail.error ?? t("runtime.error.history")}
        onRetry={() => void retryRuntimeThread(thread.id)}
      />
    );
  }
  if (runtimeMode && detail?.status === "loading" && events.length === 0) {
    return <ConversationLoadState />;
  }
  if (events.length === 0) return <EmptyConversation />;

  return (
    <>
      <div
        ref={scrollRef}
        role="log"
        aria-label={t("events.conversationLog")}
        aria-live="polite"
        aria-relevant="additions text"
        aria-atomic="false"
        onScroll={(event) => {
          const element = event.currentTarget;
          const distanceFromBottom =
            element.scrollHeight - element.scrollTop - element.clientHeight;
          shouldFollowRef.current = distanceFromBottom < 96;
          updateActiveAnchor(element);
          if (element.scrollTop < 80) requestOlderTurns();
        }}
        className="app-scrollbar absolute inset-0 overflow-y-auto px-4 sm:px-6"
      >
        {detail?.hasMore || detail?.olderStatus === "loading" || detail?.olderStatus === "error" ? (
          <div className="pointer-events-none sticky top-2 z-20 flex h-0 justify-center" aria-live="polite">
            {detail.olderStatus === "loading" ? (
              <span role="status" className="flex items-center gap-1.5 rounded-full border border-[var(--border-soft)] bg-[var(--panel)] px-2.5 py-1 text-[11px] text-[var(--muted)] shadow-sm">
                <LoaderCircle size={10} className="animate-spin" aria-hidden="true" />
                {t("runtime.loadingOlder")}
              </span>
            ) : detail.olderStatus === "error" ? (
              <button
                type="button"
                title={detail.olderError ?? undefined}
                onClick={() => requestOlderTurns(true)}
                className="pointer-events-auto flex items-center gap-1.5 rounded-full border border-[var(--border-soft)] bg-[var(--panel)] px-2.5 py-1 text-[11px] text-[#d98b8b] shadow-sm transition-colors hover:bg-[var(--panel-raised)]"
              >
                <RotateCcw size={10} aria-hidden="true" />
                {t("common.retry")}
              </button>
            ) : null}
          </div>
        ) : null}
        <div
          className="conversation-rail relative mx-auto w-full"
          style={{ height: totalSize + bottomClearance }}
        >
          {virtualizer.getVirtualItems().map((row) => {
            const event = events[row.index];
            const isMatchedToolResult =
              event.type === "tool_result" && toolCallIds.has(event.toolCallId);
            const isCollapsed =
              isMatchedToolResult && !isToolResultExpanded(event.toolCallId);
            const toolDisclosure =
              event.type === "tool_call" && toolCallIdsWithResults.has(event.id)
                ? {
                    controlsId: toolResultRegionId(event.id),
                    expanded: isToolResultExpanded(event.id),
                    onToggle: toggleToolResult,
                  }
                : undefined;
            const shouldAnimate = !animatedEventIdsRef.current.has(event.id);
            animatedEventIdsRef.current.add(event.id);
            return (
              <div
                key={event.id}
                data-index={row.index}
                data-event-id={event.id}
                data-user-message-id={
                  event.type === "message" && event.role === "user"
                    ? event.id
                    : undefined
                }
                ref={virtualizer.measureElement}
                className={cx(
                  "event-feed-row absolute left-0 top-0 w-full",
                  isCollapsed && "event-feed-row--collapsed",
                )}
                style={{ transform: `translateY(${row.start + 18}px)` }}
              >
                <div className={shouldAnimate ? "event-enter" : undefined}>
                  {event.type === "runtime_outcome" ? (
                    <>
                      <RuntimeRunProgress turn={event.turn} />
                      <RuntimeTurnOutcome turn={event.turn} />
                    </>
                  ) : isMatchedToolResult ? (
                    <div
                      id={toolResultRegionId(event.toolCallId)}
                      aria-hidden={isCollapsed}
                      className={cx(
                        "tool-result-disclosure",
                        isCollapsed && "tool-result-disclosure--collapsed",
                      )}
                    >
                      <div className="tool-result-disclosure__content">
                        <EventCard event={event} />
                      </div>
                    </div>
                  ) : (
                    <EventCard event={event} toolDisclosure={toolDisclosure} />
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
      <TurnNavigator
        key={animationScope ?? "empty"}
        anchors={turnAnchors}
        activeIndex={activeAnchorIndex}
        bottomClearance={bottomClearance}
        onNavigate={navigateToAnchor}
      />
    </>
  );
}
