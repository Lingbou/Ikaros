import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appEventText } from "../domain";
import type { AgentEvent, MessageEvent, Turn } from "../domain";
import { setUiLanguage, translate, type Translate } from "../i18n";
import {
  getTurnAnchors,
  TurnNavigator,
  type TurnAnchor,
} from "./TurnNavigator";

const createdAt = "2026-08-05T06:00:00.000Z";
const englishTranslate: Translate = (key, values) => translate(key, values, "en");
const chineseTranslate: Translate = (key, values) =>
  translate(key, values, "zh-CN");

function message(
  id: string,
  turnId: string,
  role: MessageEvent["role"],
  content: string,
): MessageEvent {
  return { id, turnId, role, content, createdAt, type: "message" };
}

function turn(id: string, events: AgentEvent[]): Turn {
  return {
    id,
    branchId: "branch-1",
    status: "completed",
    events,
  };
}

function anchor(index: number): TurnAnchor {
  return {
    id: `turn-${index}`,
    eventIndex: index * 2,
    preview: `Prompt ${index + 1}`,
    detail: `Response ${index + 1}`,
  };
}

beforeEach(() => {
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  });
});

afterEach(() => {
  cleanup();
  setUiLanguage("en");
  vi.unstubAllGlobals();
});

describe("TurnNavigator", () => {
  it("creates one anchor per navigable turn and preserves flattened event indexes", () => {
    const firstTurnId = "turn-1";
    const thirdTurnId = "turn-3";
    const turns = [
      turn(firstTurnId, [
        {
          id: "status-1",
          turnId: firstTurnId,
          type: "status",
          tone: "neutral",
          label: appEventText("status.buildingBrief", { count: 12 }),
          createdAt,
        },
        message("user-1", firstTurnId, "user", "  First\n prompt  "),
        message("user-2", firstTurnId, "user", "Follow-up in the same turn"),
        message("assistant-1", firstTurnId, "assistant", "  **Answer**\n preview "),
      ]),
      turn("turn-empty", []),
      turn(thirdTurnId, [
        message("assistant-3", thirdTurnId, "assistant", "Assistant-only turn"),
      ]),
    ];

    expect(getTurnAnchors(turns, englishTranslate)).toEqual([
      {
        id: firstTurnId,
        eventIndex: 1,
        preview: "First prompt",
        detail: "Answer preview",
      },
      {
        id: thirdTurnId,
        eventIndex: 4,
        preview: "Assistant-only turn",
        detail: undefined,
      },
    ]);
  });

  it("rebuilds app-owned preview copy with the translate function passed by the caller", () => {
    const turns = Array.from({ length: 4 }, (_, index) => {
      const turnId = `localized-turn-${index + 1}`;
      return turn(turnId, [
        {
          id: `localized-status-${index + 1}`,
          turnId,
          type: "status" as const,
          tone: "success" as const,
          label: appEventText("status.checkpointRestored"),
          createdAt,
        },
        {
          id: `localized-detail-${index + 1}`,
          turnId,
          type: "status" as const,
          tone: "success" as const,
          label: appEventText("status.outputsValidated"),
          createdAt,
        },
      ]);
    });
    const englishAnchors = getTurnAnchors(turns, englishTranslate);
    const chineseAnchors = getTurnAnchors(turns, chineseTranslate);

    expect(englishAnchors[0]).toMatchObject({
      preview: "Checkpoint restored",
      detail: "Both output files passed validation.",
    });
    expect(chineseAnchors[0]).toMatchObject({
      preview: "检查点已恢复",
      detail: "两个输出文件均已通过验证。",
    });

    const { rerender } = render(
      <TurnNavigator
        anchors={englishAnchors}
        activeIndex={0}
        onNavigate={() => undefined}
      />,
    );
    fireEvent.pointerEnter(
      screen.getByRole("button", { name: "Jump to turn 1 of 4" }),
    );
    expect(screen.getByRole("tooltip")).toHaveTextContent("Checkpoint restored");

    rerender(
      <TurnNavigator
        anchors={chineseAnchors}
        activeIndex={0}
        onNavigate={() => undefined}
      />,
    );
    expect(screen.getByRole("tooltip")).toHaveTextContent("检查点已恢复");
    expect(screen.getByRole("tooltip")).toHaveTextContent(
      "两个输出文件均已通过验证。",
    );
  });

  it("stays hidden until the conversation has four turns", () => {
    const { rerender } = render(
      <TurnNavigator
        anchors={[0, 1, 2].map(anchor)}
        activeIndex={2}
        onNavigate={() => undefined}
      />,
    );

    expect(screen.queryByRole("navigation", { name: "Conversation turns" })).toBeNull();

    rerender(
      <TurnNavigator
        anchors={[0, 1, 2, 3].map(anchor)}
        activeIndex={3}
        onNavigate={() => undefined}
      />,
    );

    expect(screen.getByRole("navigation", { name: "Conversation turns" })).toBeVisible();
  });

  it("marks exactly one current turn, clamps stale indexes, and navigates on click", () => {
    const onNavigate = vi.fn();
    render(
      <TurnNavigator
        anchors={[0, 1, 2, 3, 4].map(anchor)}
        activeIndex={99}
        onNavigate={onNavigate}
      />,
    );

    const current = screen.getByRole("button", { name: "Jump to turn 5 of 5" });
    expect(current).toHaveAttribute("aria-current", "location");
    expect(current.firstElementChild).toHaveStyle({ width: "6px" });
    expect(screen.getAllByRole("button").filter((button) => button.hasAttribute("aria-current"))).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "Jump to turn 2 of 5" }));
    expect(onNavigate).toHaveBeenCalledOnce();
    expect(onNavigate).toHaveBeenCalledWith(1);
  });

  it("shows a turn preview on hover and focus, then fully collapses the wave", () => {
    render(
      <TurnNavigator
        anchors={[0, 1, 2, 3, 4].map(anchor)}
        activeIndex={4}
        onNavigate={() => undefined}
      />,
    );

    const navigation = screen.getByRole("navigation", { name: "Conversation turns" });
    const hovered = screen.getByRole("button", { name: "Jump to turn 3 of 5" });
    for (const marker of screen.getAllByRole("button")) {
      expect(marker.firstElementChild).toHaveStyle({ width: "6px" });
    }

    fireEvent.pointerEnter(hovered);

    expect(screen.getByRole("tooltip")).toHaveTextContent("Prompt 3");
    expect(screen.getByRole("tooltip")).toHaveTextContent("Response 3");
    expect(screen.getByRole("tooltip")).toHaveTextContent("Turn 3 of 5");
    expect(hovered.firstElementChild).toHaveStyle({ width: "26px" });
    expect(screen.getByRole("button", { name: "Jump to turn 2 of 5" }).firstElementChild).toHaveStyle({
      width: "20px",
    });

    fireEvent.pointerLeave(navigation);
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(screen.getByRole("button", { name: "Jump to turn 5 of 5" }).firstElementChild).toHaveStyle({
      width: "6px",
    });
    for (const marker of screen.getAllByRole("button")) {
      expect(marker.firstElementChild).toHaveStyle({ width: "6px" });
    }

    fireEvent.focus(hovered);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Prompt 3");
    fireEvent.blur(hovered);
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(hovered.firstElementChild).toHaveStyle({ width: "6px" });
  });

  it("scrubs across turns only while the pointer is held", () => {
    const onNavigate = vi.fn();
    render(
      <TurnNavigator
        anchors={[0, 1, 2, 3].map(anchor)}
        activeIndex={0}
        onNavigate={onNavigate}
      />,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: "Jump to turn 1 of 4" }));
    fireEvent.pointerEnter(screen.getByRole("button", { name: "Jump to turn 3 of 4" }));
    expect(onNavigate).toHaveBeenLastCalledWith(2);

    fireEvent.pointerUp(window);
    fireEvent.pointerEnter(screen.getByRole("button", { name: "Jump to turn 4 of 4" }));
    expect(onNavigate).toHaveBeenCalledTimes(1);
  });

  it("uses one tab stop and supports arrow, Home, and End navigation", () => {
    const onNavigate = vi.fn();
    render(
      <TurnNavigator
        anchors={[0, 1, 2, 3].map(anchor)}
        activeIndex={1}
        onNavigate={onNavigate}
      />,
    );

    const buttons = screen.getAllByRole("button");
    expect(buttons.map((button) => button.tabIndex)).toEqual([-1, 0, -1, -1]);

    buttons[1].focus();
    fireEvent.keyDown(buttons[1], { key: "End" });
    expect(onNavigate).toHaveBeenLastCalledWith(3);
    expect(buttons[3]).toHaveFocus();

    fireEvent.keyDown(buttons[3], { key: "ArrowUp" });
    expect(onNavigate).toHaveBeenLastCalledWith(2);
    expect(buttons[2]).toHaveFocus();

    fireEvent.keyDown(buttons[2], { key: "Home" });
    expect(onNavigate).toHaveBeenLastCalledWith(0);
    expect(buttons[0]).toHaveFocus();
  });

  it("localizes navigation, position, and button labels in Simplified Chinese", () => {
    setUiLanguage("zh-CN");
    render(
      <TurnNavigator
        anchors={[0, 1, 2, 3].map(anchor)}
        activeIndex={0}
        onNavigate={() => undefined}
      />,
    );

    expect(screen.getByRole("navigation", { name: "对话轮次" })).toBeVisible();
    const fourth = screen.getByRole("button", { name: "跳转到第 4 轮，共 4 轮" });
    fireEvent.pointerEnter(fourth);
    expect(screen.getByRole("tooltip")).toHaveTextContent("第 4 轮，共 4 轮");
  });
});
