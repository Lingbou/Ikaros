import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { setUiLanguage } from "../i18n";
import { LOCAL_PROFILE } from "../localProfile";
import { useAppStore } from "../store";
import { ProfileSettings } from "./ProfileSettings";

afterEach(() => {
  cleanup();
  setUiLanguage("en");
  useAppStore.getState().setProfileUsername(LOCAL_PROFILE.name);
});

describe("ProfileSettings", () => {
  it("renders the complete mock activity profile without account-only details", () => {
    render(<ProfileSettings />);

    expect(screen.getByRole("heading", { level: 1, name: "Profile" })).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { level: 2, name: LOCAL_PROFILE.name })
    ).toBeInTheDocument();
    expect(screen.getByText(LOCAL_PROFILE.initials)).toBeInTheDocument();

    for (const label of [
      "Lifetime tokens",
      "Peak tokens",
      "Longest chat",
      "Current streak",
      "Longest streak"
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    for (const value of ["761.1M", "73.1M", "16h 17m", "0 days", "5 days"]) {
      expect(screen.getByText(value)).toBeInTheDocument();
    }

    expect(
      screen.getByRole("heading", { level: 2, name: "Token activity" })
    ).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "Token activity" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Daily" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    expect(screen.getByRole("tab", { name: "Weekly" })).toHaveAttribute(
      "aria-selected",
      "false"
    );
    expect(screen.getByRole("tab", { name: "Cumulative" })).toHaveAttribute(
      "aria-selected",
      "false"
    );

    expect(
      screen.getByRole("heading", { level: 2, name: "Activity insights" })
    ).toBeInTheDocument();
    for (const label of [
      "Fast mode",
      "Most used reasoning",
      "Skills explored",
      "Total skills used",
      "Total chats"
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    expect(screen.getByText("86%")).toBeInTheDocument();
    expect(screen.getByText("Extra High · 80%")).toBeInTheDocument();

    expect(
      screen.getByRole("heading", { level: 2, name: "Most used skills" })
    ).toBeInTheDocument();
    expect(screen.getAllByText(/\d+ runs/)).toHaveLength(5);
    expect(screen.queryByText(/plugin|account|@/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/superpowers/i)).not.toBeInTheDocument();
  });

  it("switches the token activity view with an accessible tab state", () => {
    const { container } = render(<ProfileSettings />);
    const daily = screen.getByRole("tab", { name: "Daily" });
    const weekly = screen.getByRole("tab", { name: "Weekly" });
    const cumulative = screen.getByRole("tab", { name: "Cumulative" });

    expect(daily).toHaveAttribute("aria-selected", "true");
    expect(container.querySelector("[data-activity-view]")).toHaveAttribute(
      "data-activity-view",
      "daily"
    );

    fireEvent.click(weekly);
    expect(daily).toHaveAttribute("aria-selected", "false");
    expect(weekly).toHaveAttribute("aria-selected", "true");
    expect(cumulative).toHaveAttribute("aria-selected", "false");
    expect(container.querySelector("[data-activity-view]")).toHaveAttribute(
      "data-activity-view",
      "weekly"
    );

    fireEvent.click(cumulative);
    expect(weekly).toHaveAttribute("aria-selected", "false");
    expect(cumulative).toHaveAttribute("aria-selected", "true");
    expect(container.querySelector("[data-activity-view]")).toHaveAttribute(
      "data-activity-view",
      "cumulative"
    );
  });

  it("edits only the in-memory username and resets canceled drafts", () => {
    render(<ProfileSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const username = screen.getByRole("textbox", { name: "Username" });
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    expect(username).toHaveAttribute("maxlength", "32");
    expect(username).toHaveValue("hc");
    expect(screen.queryByText(/display name/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/@handle/i)).not.toBeInTheDocument();

    fireEvent.change(username, { target: { value: "  Nova Lane  " } });
    expect(screen.getByText("NL")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(useAppStore.getState().profileUsername).toBe("Nova Lane");
    expect(screen.getByRole("heading", { level: 2, name: "Nova Lane" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const reopenedUsername = screen.getByRole("textbox", { name: "Username" });
    expect(reopenedUsername).toHaveValue("Nova Lane");
    fireEvent.change(reopenedUsername, { target: { value: "Discard me" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(useAppStore.getState().profileUsername).toBe("Nova Lane");
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByRole("textbox", { name: "Username" })).toHaveValue("Nova Lane");
  });

  it("disables saving an empty username", () => {
    render(<ProfileSettings />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Username" }), {
      target: { value: "   " }
    });

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("translates the complete activity profile into Simplified Chinese", () => {
    setUiLanguage("zh-CN");
    render(<ProfileSettings />);

    expect(
      screen.getByRole("heading", { level: 1, name: "个人资料" })
    ).toBeInTheDocument();

    for (const label of [
      "累计 Token 数",
      "峰值 Token 数",
      "最长聊天时长",
      "当前连续天数",
      "最长连续天数"
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    for (const value of ["7.6亿", "7313.3万", "16 小时 17 分", "0 天", "5 天"]) {
      expect(screen.getByText(value)).toBeInTheDocument();
    }

    expect(
      screen.getByRole("heading", { level: 2, name: "Token 活动" })
    ).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "Token 活动" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "每日" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    expect(screen.getByRole("tab", { name: "每周" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "累计" })).toBeInTheDocument();

    expect(
      screen.getByRole("heading", { level: 2, name: "活动洞察" })
    ).toBeInTheDocument();
    for (const label of [
      "快速模式",
      "最常用的推理强度",
      "已探索的技能",
      "使用的技能总数",
      "聊天总数"
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    expect(screen.getByText("极高 · 80%")).toBeInTheDocument();

    expect(
      screen.getByRole("heading", { level: 2, name: "最常用的技能" })
    ).toBeInTheDocument();
    expect(screen.getByText("运行 474 次")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "编辑" }));
    expect(
      screen.getByRole("heading", { level: 2, name: "编辑个人资料" })
    ).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "用户名" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存" })).toBeInTheDocument();
  });
});
