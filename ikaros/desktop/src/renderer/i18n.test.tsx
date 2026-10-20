import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  setUiLanguage,
  translate,
  useTranslation,
} from "./i18n";

function TranslationProbe() {
  const { language, t } = useTranslation();
  return (
    <div>
      <span data-testid="language">{language}</span>
      <span>{t("settings.language")}</span>
    </div>
  );
}

afterEach(() => {
  cleanup();
  setUiLanguage("en");
});

describe("renderer i18n", () => {
  it("updates subscribed React chrome and the document language synchronously", () => {
    setUiLanguage("en");
    render(<TranslationProbe />);

    expect(screen.getByText("Language")).toBeInTheDocument();
    expect(screen.getByTestId("language")).toHaveTextContent("en");

    act(() => setUiLanguage("zh-CN"));

    expect(document.documentElement.lang).toBe("zh-CN");
    expect(screen.getByText("语言")).toBeInTheDocument();
    expect(screen.getByTestId("language")).toHaveTextContent("zh-CN");
  });

  it("supports typed interpolation without changing the active language", () => {
    setUiLanguage("en");

    expect(
      translate("sidebar.currentWorkspace", { name: "Ikaros" }, "zh-CN"),
    ).toBe("当前工作区：Ikaros");
    expect(document.documentElement.lang).toBe("en");
  });

  it("keeps the English and Simplified Chinese catalogs aligned with official UI terms", () => {
    expect(translate("settings.general", undefined, "zh-CN")).toBe("常规");
    expect(translate("settings.languageDescription", undefined, "zh-CN")).toBe(
      "应用 UI 语言",
    );
    expect(translate("titlebar.showSidebar", undefined, "zh-CN")).toBe("显示边栏");
    expect(translate("titlebar.hideSidebar", undefined, "zh-CN")).toBe("隐藏边栏");
    expect(translate("composer.stopRun", undefined, "zh-CN")).toBe("停止");
    expect(translate("sidebar.newChat", undefined, "zh-CN")).toBe("新对话");
    expect(translate("sidebar.noChats", undefined, "zh-CN")).toBe("没有聊天");
    expect(translate("sidebar.recents", undefined, "zh-CN")).toBe("最近");
    expect(translate("files.referenceText", { path: "a.ts", start: 1, end: 5 }, "zh-CN")).toBe("文件：a.ts（第 1–5 行）");

    expect(translate("settings.languageDescription", undefined, "en")).toBe(
      "Language for the app UI",
    );
    expect(translate("composer.stopRun", undefined, "en")).toBe("Stop");
    expect(translate("files.referenceText", { path: "a.ts", start: 1, end: 5 }, "en")).toBe("File: a.ts (lines 1–5)");
  });
});
