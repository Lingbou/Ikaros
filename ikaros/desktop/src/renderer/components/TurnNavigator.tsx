import { createPortal } from "react-dom";
import { useEffect, useId, useRef, useState, type CSSProperties } from "react";
import type { AgentEvent, Turn } from "../domain";
import { resolveEventText, resolveInterruptCopy } from "../eventCopy";
import { useTranslation, type Translate } from "../i18n";

export const MIN_TURN_NAV_ITEMS = 4;

export interface TurnAnchor {
  id: string;
  eventIndex: number;
  preview: string;
  detail?: string;
}

interface TurnNavigatorProps {
  anchors: TurnAnchor[];
  activeIndex: number;
  bottomClearance?: number;
  onNavigate: (index: number) => void;
}

interface PreviewPosition {
  left: number;
  top: number;
}

const PREVIEW_WIDTH = 320;
const PREVIEW_HEIGHT = 126;
const PREVIEW_GAP = 3;
const VIEWPORT_MARGIN = 12;

function cleanExcerpt(value: string): string {
  return value
    .replace(/\[([^\]]+)]\([^)]+\)/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/^\s*[-+]\s+/gm, "")
    .replace(/[*_`~]+/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function eventExcerpt(event: AgentEvent, t: Translate): string {
  switch (event.type) {
    case "message":
      return cleanExcerpt(event.content);
    case "tool_call":
      return cleanExcerpt(resolveEventText(event.label, t));
    case "tool_result":
      return cleanExcerpt(resolveEventText(event.summary, t));
    case "permission_request":
      return cleanExcerpt(resolveEventText(event.title, t));
    case "interrupt":
      return cleanExcerpt(resolveInterruptCopy(event.copy, t).title);
    case "artifact":
      return cleanExcerpt(event.title);
    case "file_change":
      return cleanExcerpt(event.path);
    case "status":
      return cleanExcerpt(resolveEventText(event.label, t));
    case "branch_created":
      return "";
  }
}

export function getTurnAnchors(turns: Turn[], t: Translate): TurnAnchor[] {
  const anchors: TurnAnchor[] = [];
  let eventOffset = 0;

  for (const turn of turns) {
    const events = turn.events;
    if (!events.length) continue;

    const userEventIndex = events.findIndex(
      (event) => event.type === "message" && event.role === "user",
    );
    const landingIndex = userEventIndex >= 0 ? userEventIndex : 0;
    const landingEvent = events[landingIndex];
    const preview = eventExcerpt(landingEvent, t);
    const assistantEvent = events.find(
      (event) => event.type === "message" && event.role === "assistant",
    );
    const fallbackDetailEvent = events.find(
      (event, index) => index !== landingIndex && eventExcerpt(event, t),
    );
    const detail = eventExcerpt(assistantEvent ?? fallbackDetailEvent ?? landingEvent, t);

    anchors.push({
      id: turn.id,
      eventIndex: eventOffset + landingIndex,
      preview,
      detail: detail && detail !== preview ? detail : undefined,
    });
    eventOffset += events.length;
  }

  return anchors;
}

function markerWidth(index: number, waveIndex: number | null): number {
  if (waveIndex === null) return 6;
  const distance = Math.abs(index - waveIndex);
  if (distance === 0) return 26;
  if (distance === 1) return 20;
  if (distance === 2) return 14;
  if (distance === 3) return 10;
  return 6;
}

export function TurnNavigator({
  anchors,
  activeIndex,
  bottomClearance = 0,
  onNavigate,
}: TurnNavigatorProps) {
  const { t } = useTranslation();
  const previewId = useId();
  const [pointedIndex, setPointedIndex] = useState<number | null>(null);
  const [previewPosition, setPreviewPosition] = useState<PreviewPosition | null>(null);
  const scrubbingRef = useRef(false);
  const markerRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const safeActiveIndex = Math.min(
    Math.max(activeIndex, 0),
    Math.max(anchors.length - 1, 0),
  );
  const waveIndex = pointedIndex;
  const pointedAnchor = pointedIndex === null ? undefined : anchors[pointedIndex];

  useEffect(() => {
    const stopScrubbing = () => {
      scrubbingRef.current = false;
    };
    window.addEventListener("pointerup", stopScrubbing);
    window.addEventListener("pointercancel", stopScrubbing);
    return () => {
      window.removeEventListener("pointerup", stopScrubbing);
      window.removeEventListener("pointercancel", stopScrubbing);
    };
  }, []);

  useEffect(() => {
    if (pointedIndex !== null && pointedIndex >= anchors.length) {
      setPointedIndex(null);
      setPreviewPosition(null);
    }
  }, [anchors.length, pointedIndex]);

  useEffect(() => {
    markerRefs.current[safeActiveIndex]?.scrollIntoView?.({ block: "nearest" });
  }, [anchors.length, safeActiveIndex]);

  if (anchors.length < MIN_TURN_NAV_ITEMS) return null;

  const showPreview = (index: number, marker: HTMLButtonElement) => {
    const bounds = marker.getBoundingClientRect();
    const left = Math.max(
      VIEWPORT_MARGIN,
      Math.min(
        bounds.right + PREVIEW_GAP,
        window.innerWidth - PREVIEW_WIDTH - VIEWPORT_MARGIN,
      ),
    );
    const top = Math.max(
      PREVIEW_HEIGHT / 2 + VIEWPORT_MARGIN,
      Math.min(
        bounds.top + bounds.height / 2,
        window.innerHeight - PREVIEW_HEIGHT / 2 - VIEWPORT_MARGIN,
      ),
    );
    setPointedIndex(index);
    setPreviewPosition({ left, top });
  };

  const hidePreview = () => {
    setPointedIndex(null);
    setPreviewPosition(null);
  };

  const focusAndNavigate = (index: number) => {
    const nextIndex = Math.min(anchors.length - 1, Math.max(0, index));
    onNavigate(nextIndex);
    window.requestAnimationFrame(() => markerRefs.current[nextIndex]?.focus());
  };

  return (
    <nav
      aria-label={t("turnNavigator.label")}
      className="turn-navigator absolute z-10"
      style={{
        "--turn-navigator-bottom-clearance": `${bottomClearance}px`,
      } as CSSProperties}
      onPointerLeave={hidePreview}
    >
      <ol className="turn-navigator__list m-0 flex list-none flex-col items-start p-0">
        {anchors.map((anchor, index) => {
          const isActive = index === safeActiveIndex;
          const isWaveCenter = waveIndex !== null && index === waveIndex;
          const waveDistance =
            waveIndex === null ? Number.POSITIVE_INFINITY : Math.abs(index - waveIndex);
          return (
            <li key={anchor.id} className="h-[10px]">
              <button
                ref={(node) => {
                  markerRefs.current[index] = node;
                }}
                type="button"
                aria-label={t("turnNavigator.jumpTo", {
                  position: index + 1,
                  count: anchors.length,
                })}
                aria-current={isActive ? "location" : undefined}
                aria-describedby={pointedIndex === index ? previewId : undefined}
                tabIndex={isActive ? 0 : -1}
                data-turn-index={index}
                onClick={() => onNavigate(index)}
                onFocus={(event) => showPreview(index, event.currentTarget)}
                onBlur={hidePreview}
                onKeyDown={(event) => {
                  let nextIndex: number | undefined;
                  if (event.key === "ArrowUp") nextIndex = index - 1;
                  if (event.key === "ArrowDown") nextIndex = index + 1;
                  if (event.key === "Home") nextIndex = 0;
                  if (event.key === "End") nextIndex = anchors.length - 1;
                  if (nextIndex === undefined) return;
                  event.preventDefault();
                  focusAndNavigate(nextIndex);
                }}
                onPointerDown={() => {
                  scrubbingRef.current = true;
                }}
                onPointerEnter={(event) => {
                  showPreview(index, event.currentTarget);
                  if (scrubbingRef.current) onNavigate(index);
                }}
                className="group flex h-[10px] w-8 cursor-pointer items-center justify-start border-0 bg-transparent p-0"
              >
                <span
                  aria-hidden="true"
                  className={`block h-0.5 rounded-full transition-[width,height,background-color,opacity] duration-150 ease-out ${
                    isWaveCenter
                      ? "bg-[var(--text)] opacity-100"
                      : isActive
                        ? "bg-[var(--muted-strong)] opacity-100"
                        : waveDistance === 1
                          ? "bg-[var(--muted-strong)] opacity-90"
                          : waveDistance === 2
                            ? "bg-[var(--muted-strong)] opacity-70"
                            : "bg-[var(--muted)] opacity-55 group-hover:opacity-85"
                  }`}
                  style={{ width: markerWidth(index, waveIndex) }}
                />
              </button>
            </li>
          );
        })}
      </ol>

      {pointedAnchor && previewPosition
        ? createPortal(
            <div
              id={previewId}
              role="tooltip"
              className="turn-navigator__preview pointer-events-none fixed z-50"
              style={{ left: previewPosition.left, top: previewPosition.top }}
            >
              <p className="turn-navigator__preview-title m-0">
                {pointedAnchor.preview || t("turnNavigator.untitled")}
              </p>
              {pointedAnchor.detail ? (
                <p className="turn-navigator__preview-detail m-0">
                  {pointedAnchor.detail}
                </p>
              ) : null}
              <p className="turn-navigator__preview-position m-0">
                {t("turnNavigator.position", {
                  position: (pointedIndex ?? 0) + 1,
                  count: anchors.length,
                })}
              </p>
            </div>,
            document.body,
          )
        : null}
    </nav>
  );
}
