import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { Markdown } from "./ui";

afterEach(cleanup);

describe("Markdown", () => {
  it("visually hides decorative emoji only at the start of headings", () => {
    const source = `## ⚠️ Clean heading

### **👩‍💻 Nested heading**

#### 🌐No-gap heading

💡 Paragraph emoji stays visible.`;
    const { container } = render(<Markdown content={source} />);

    const cleanHeading = screen.getByRole("heading", { level: 2, name: "Clean heading" });
    expect(cleanHeading.querySelector(".markdown-heading-emoji")).toHaveTextContent("⚠️");
    expect(cleanHeading).toHaveTextContent("⚠️ Clean heading");

    const nestedHeading = screen.getByRole("heading", { level: 3, name: "Nested heading" });
    expect(nestedHeading.querySelector(".markdown-heading-emoji")).toHaveTextContent("👩‍💻");
    expect(screen.getByRole("heading", { level: 4, name: "🌐No-gap heading" })).toBeInTheDocument();
    expect(container).toHaveTextContent("💡 Paragraph emoji stays visible.");
  });

  it("renders GFM tables instead of exposing their pipe syntax", () => {
    const { container } = render(
      <Markdown
        content={`| Hemisphere | Precision | Recall |
|---|---:|---:|
| Left | 0.8204 | 0.6912 |
| Right | 0.8565 | 0.5912 |`}
      />,
    );

    const table = screen.getByRole("table");
    expect(table).toBeInTheDocument();
    expect(table.parentElement).toHaveClass("markdown-table-scroll");
    expect(screen.getAllByRole("columnheader")).toHaveLength(3);
    expect(screen.getByRole("cell", { name: "Left" })).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "0.8204" })).toHaveStyle({ textAlign: "right" });
    expect(container).not.toHaveTextContent("|---|---:|---:|");
  });

  it("supports GFM task lists, autolinks, and strict double-tilde deletion", () => {
    const { container } = render(
      <Markdown
        content={`- [x] Complete

Visit https://example.com and ~~remove~~ this, but keep ~one tilde~.`}
      />,
    );

    const checkbox = screen.getByRole("checkbox");
    expect(checkbox).toBeChecked();
    expect(checkbox).toBeDisabled();

    const link = screen.getByRole("link", { name: "https://example.com" });
    expect(link).toHaveAttribute("href", "https://example.com");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");

    expect(container.querySelector("del")).toHaveTextContent("remove");
    expect(container).toHaveTextContent("~one tilde~");
  });
});
