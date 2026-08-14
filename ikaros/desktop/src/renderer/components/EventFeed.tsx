import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { activeBranch } from "../domain";
import { useTranslation } from "../i18n";
import { selectCurrentThread, useAppStore } from "../store";
import { EventCard } from "./EventCard";
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
        <h1 className="text-[24px] font-medium leading-[30px] tracking-[-0.03em] text-[var(--text)]">
          {t("empty.heading")}
        </h1>
        <p className="mt-2 max-w-md text-[12px] leading-[18px] text-[var(--muted)]">
          {t("empty.description")}
        </p>
      </div>
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

export function EventFeed({ bottomClearance }: { bottomClearance: number }) {
  const { t } = useTranslation();
  const thread = useAppStore(selectCurrentThread);
  const branch = activeBranch(thread);
  const turns = branch?.turns;
  const events = useMemo(
    () => turns?.flatMap((turn) => turn.events) ?? [],
    [turns],
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
  const [toolResultExpansion, setToolResultExpansion] = useState<{
    scope: string | null;
    expandedIds: Set<string>;
  }>({ scope: null, expandedIds: new Set() });
  const [activeAnchorIndex, setActiveAnchorIndex] = useState(0);
  if (animationScopeRef.current !== animationScope) {
    animationScopeRef.current = animationScope;
    animatedEventIdsRef.current = new Set(events.map((event) => event.id));
  }
  const virtualizer = useVirtualizer({
    count: events.length,
    getScrollElement: () => scrollRef.current,
    // ResizeObserver can notify during a React commit. Let React schedule the
    // resulting render instead of forcing flushSync from inside that lifecycle.
    useFlushSync: false,
    estimateSize: (index) => {
      const event = events[index];
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
    setActiveAnchorIndex(Math.max(0, turnAnchors.length - 1));
  }, [branch?.id, thread?.id]);

  useEffect(() => {
    if (!shouldFollowRef.current || !turnAnchors.length) return;
    setActiveAnchorIndex(turnAnchors.length - 1);
  }, [turnAnchors.length]);

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
        setActiveAnchorIndex(turnAnchors.length - 1);
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
      setActiveAnchorIndex((current) =>
        current === nextAnchorIndex ? current : nextAnchorIndex,
      );
    },
    [turnAnchors, virtualizer],
  );

  const navigateToAnchor = useCallback(
    (index: number) => {
      const anchor = turnAnchors[index];
      if (!anchor) return;
      shouldFollowRef.current = index === turnAnchors.length - 1;
      setActiveAnchorIndex(index);
      virtualizer.scrollToIndex(anchor.eventIndex, {
        align: "start",
        behavior: "auto",
      });
    },
    [turnAnchors, virtualizer],
  );

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
        }}
        className="app-scrollbar absolute inset-0 overflow-y-auto px-4 sm:px-6"
      >
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
                  {isMatchedToolResult ? (
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
