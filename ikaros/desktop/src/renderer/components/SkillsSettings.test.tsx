import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUiLanguage } from "../i18n";
import { SkillsSettings } from "./SkillsSettings";

const skills = [
  {
    name: "imagegen-plus",
    description: "Generate images with the configured provider.",
    location: "C:/Users/User/.ikaros/skills/imagegen-plus/SKILL.md",
    enabled: true
  },
  {
    name: "grill-me",
    description: "Resolve a design one decision at a time.",
    location: "C:/Users/User/.ikaros/skills/grill-me/SKILL.md",
    enabled: false
  }
] as const;

afterEach(() => {
  cleanup();
  setUiLanguage("en");
});

describe("SkillsSettings", () => {
  it("renders the real catalog, diagnostics, and changes one skill at a time", async () => {
    const onSetEnabled = vi.fn(async () => undefined);

    render(
      <SkillsSettings
        skills={skills}
        diagnostics={[
          {
            entry: "broken-skill",
            code: "invalid_frontmatter",
            message: "Frontmatter must contain a description."
          }
        ]}
        status="ready"
        catalogError={null}
        onRefresh={async () => undefined}
        onSetEnabled={onSetEnabled}
      />
    );

    expect(screen.getByRole("heading", { level: 1, name: "Skills" })).toBeTruthy();
    expect(screen.getByText("Generate images with the configured provider.")).toBeTruthy();
    expect(screen.getByText("broken-skill")).toBeTruthy();
    expect(screen.getByText("SKILL.md frontmatter is invalid.")).toBeTruthy();
    expect(screen.queryByText("invalid_frontmatter")).toBeNull();

    const enabledSwitch = screen.getByRole("switch", { name: "Toggle imagegen-plus" });
    const disabledSwitch = screen.getByRole("switch", { name: "Toggle grill-me" });
    expect(enabledSwitch.getAttribute("aria-checked")).toBe("true");
    expect(disabledSwitch.getAttribute("aria-checked")).toBe("false");

    fireEvent.click(disabledSwitch);
    await waitFor(() => expect(onSetEnabled).toHaveBeenCalledWith("grill-me", true));
  });

  it("refreshes on demand and keeps the visible catalog when refresh fails", async () => {
    const onRefresh = vi.fn(async () => {
      throw new Error("runtime unavailable");
    });

    render(
      <SkillsSettings
        skills={skills}
        diagnostics={[]}
        status="ready"
        catalogError={null}
        onRefresh={onRefresh}
        onSetEnabled={async () => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    expect(
      await screen.findByText("Could not refresh skills. The current list was kept.")
    ).toBeTruthy();
    expect(screen.getByText("imagegen-plus")).toBeTruthy();
    expect(onRefresh).toHaveBeenCalledOnce();
  });

  it("shows honest loading and empty states without mock skills", () => {
    const view = render(
      <SkillsSettings
        skills={[]}
        diagnostics={[]}
        status="loading"
        catalogError={null}
        onRefresh={async () => undefined}
        onSetEnabled={async () => undefined}
      />
    );

    expect(screen.getByText("Loading skills…")).toBeTruthy();
    expect(screen.queryByRole("switch")).toBeNull();

    view.rerender(
      <SkillsSettings
        skills={[]}
        diagnostics={[]}
        status="ready"
        catalogError={null}
        onRefresh={async () => undefined}
        onSetEnabled={async () => undefined}
      />
    );

    expect(screen.getByText("No skills found")).toBeTruthy();
    expect(screen.getByText(/~\/.ikaros\/skills/)).toBeTruthy();
  });

  it("translates fixed UI while preserving skill-authored content", () => {
    setUiLanguage("zh-CN");

    render(
      <SkillsSettings
        skills={skills.slice(0, 1)}
        diagnostics={[]}
        status="ready"
        catalogError={null}
        onRefresh={async () => undefined}
        onSetEnabled={async () => undefined}
      />
    );

    expect(screen.getByRole("heading", { level: 1, name: "技能" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "刷新" })).toBeTruthy();
    expect(screen.getByText("已安装的技能")).toBeTruthy();
    expect(screen.getByText("Generate images with the configured provider.")).toBeTruthy();
  });
});
