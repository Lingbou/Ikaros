import type { CSSProperties } from "react";

import type { ColorSchemePreference, ThemePreferences } from "../../shared/platform";
import { mixHexColors, readableTextColor } from "../../shared/theme";
import type { ThemeTransitionOrigin } from "../applyUiPreferences";
import { cx } from "./ui";

interface ThemeCardProps {
  label: string;
  value: ColorSchemePreference;
  selected: boolean;
  lightTheme: Readonly<ThemePreferences>;
  darkTheme: Readonly<ThemePreferences>;
  onSelect(value: ColorSchemePreference, origin: ThemeTransitionOrigin): void;
}

function PreviewHalf({
  theme,
  side
}: {
  theme: Readonly<ThemePreferences>;
  side: "left" | "right" | "full";
}) {
  const dark = readableTextColor(theme.background) === "#ffffff";
  const previewBackground = dark
    ? mixHexColors(theme.background, theme.foreground, 0.28)
    : mixHexColors(theme.background, theme.foreground, 0.018);
  const panel = dark
    ? mixHexColors(theme.background, theme.foreground, 0.22)
    : mixHexColors(theme.background, theme.foreground, 0.11);
  const muted = dark
    ? mixHexColors(theme.background, theme.foreground, 0.46)
    : mixHexColors(theme.background, theme.foreground, 0.22);
  const sheet = dark
    ? mixHexColors(theme.background, theme.foreground, 0.14)
    : mixHexColors(theme.background, "#ffffff", 0.72);
  const style: CSSProperties = {
    background: previewBackground,
    color: theme.foreground,
    left: side === "right" ? "50%" : 0,
    right: side === "left" ? "50%" : 0,
    width: side === "full" ? "100%" : "50%"
  };

  return (
    <span className="absolute inset-y-0 overflow-hidden" style={style}>
      <span
        className="absolute inset-x-[15%] top-[22%] h-2 rounded-full"
        style={{ background: muted }}
      />
      <span
        className="absolute inset-x-[7%] top-[29%] h-1 rounded-full opacity-65"
        style={{ background: muted }}
      />
      <span
        className="absolute bottom-[-12%] left-[8%] right-[8%] top-[37%] overflow-hidden rounded-t-xl border"
        style={{ background: sheet, borderColor: panel }}
      >
        <span className="absolute left-[8%] top-[13%] h-2 w-[34%] rounded-full" style={{ background: muted }} />
        <span className="absolute left-[8%] top-[27%] h-1 w-[54%] rounded-full opacity-55" style={{ background: muted }} />
        <span className="absolute left-[8%] top-[42%] h-2 w-[29%] rounded-full opacity-80" style={{ background: muted }} />
        <span className="absolute inset-x-0 top-[57%] h-px" style={{ background: panel }} />
        <span className="absolute left-[8%] top-[68%] h-2 w-[31%] rounded-full opacity-75" style={{ background: muted }} />
      </span>
    </span>
  );
}

export function ThemeCard({
  label,
  value,
  selected,
  lightTheme,
  darkTheme,
  onSelect
}: ThemeCardProps) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={(event) => {
        const bounds = event.currentTarget.getBoundingClientRect();
        const origin =
          event.detail === 0
            ? { x: bounds.left + bounds.width / 2, y: bounds.top + bounds.height / 2 }
            : { x: event.clientX, y: event.clientY };
        onSelect(value, origin);
      }}
      className="group min-w-0 text-center outline-none"
    >
      <span
        className={cx(
          "relative block aspect-[31/22] w-full overflow-hidden rounded-[10px] border transition-colors",
          selected
            ? "border-[var(--text)] shadow-[0_0_0_1px_var(--text)]"
            : "border-[var(--border)] group-hover:border-[var(--muted)]"
        )}
      >
        {value === "system" ? (
          <>
            <PreviewHalf theme={lightTheme} side="left" />
            <PreviewHalf theme={darkTheme} side="right" />
          </>
        ) : (
          <PreviewHalf theme={value === "light" ? lightTheme : darkTheme} side="full" />
        )}
      </span>
      <span
        className={cx(
          "mt-2 block text-[12px] leading-[18px]",
          selected ? "font-medium text-[var(--text)]" : "text-[var(--muted)]"
        )}
      >
        {label}
      </span>
    </button>
  );
}

const CODE_LINES = [
  ["1", "const", " themePreview: ThemeConfig = {"],
  ["2", "", '  surface: "sidebar",'],
  ["3", "", '  accent: "#339cff",'],
  ["4", "", "  contrast: 60,"],
  ["5", "", "};"]
] as const;

function DiffPane({ side }: { side: "removed" | "added" }) {
  const added = side === "added";
  return (
    <div className="min-w-0 flex-1 overflow-hidden py-1 font-[var(--code-font)] text-[11px] leading-[20px]">
      {CODE_LINES.map(([number, keyword, rest], index) => {
        const highlighted = index >= 1 && index <= 3;
        return (
          <div
            key={number}
            className="relative flex h-5 min-w-[330px]"
            style={
              highlighted
                ? {
                    background: added ? "rgba(64, 201, 119, 0.13)" : "rgba(250, 66, 62, 0.14)",
                    boxShadow: `inset 4px 0 0 ${added ? "#40c977" : "#fa423e"}`
                  }
                : undefined
            }
          >
            <span className="w-12 shrink-0 pr-3 text-right text-[var(--muted)]">{number}</span>
            <code className="whitespace-pre text-[var(--text)]">
              {keyword ? <span style={{ color: "#b589ff" }}>{keyword}</span> : null}
              <span style={{ color: index === 2 ? "#ff9f43" : "var(--text)" }}>{rest}</span>
            </code>
          </div>
        );
      })}
    </div>
  );
}

export function CodeDiffPreview() {
  return (
    <div
      aria-hidden="true"
      className="mt-3 flex h-[120px] overflow-hidden rounded-[11px] border border-[var(--border-soft)] bg-[var(--canvas)]"
    >
      <DiffPane side="removed" />
      <DiffPane side="added" />
    </div>
  );
}
