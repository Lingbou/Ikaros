import * as Tooltip from "@radix-ui/react-tooltip";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import ReactMarkdown from "react-markdown";

export function cx(...classes: Array<string | false | null | undefined>) {
  return classes.filter(Boolean).join(" ");
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  children: ReactNode;
  tooltipSide?: "top" | "right" | "bottom" | "left";
}

export function IconButton({
  label,
  children,
  className,
  tooltipSide = "top",
  ...props
}: IconButtonProps) {
  return (
    <Tooltip.Root delayDuration={380}>
      <Tooltip.Trigger asChild>
        <button
          type="button"
          aria-label={label}
          className={cx(
            "inline-flex size-8 shrink-0 items-center justify-center rounded-lg text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] disabled:pointer-events-none disabled:opacity-35",
            className,
          )}
          {...props}
        >
          {children}
        </button>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          side={tooltipSide}
          sideOffset={7}
          className="z-[100] rounded-md border border-[var(--border)] bg-[var(--panel-raised)] px-2 py-1 text-[11px] text-[var(--text)] shadow-xl"
        >
          {label}
          <Tooltip.Arrow className="fill-[var(--border)]" />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

export function Markdown({ content, streaming = false }: { content: string; streaming?: boolean }) {
  return (
    <div className={cx("markdown-body", streaming && "stream-caret")}>
      <ReactMarkdown>{content}</ReactMarkdown>
    </div>
  );
}

function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded border border-[var(--border)] bg-[var(--panel-hover)] px-1.5 py-0.5 font-sans text-[10px] text-[var(--muted-strong)] shadow-sm">
      {children}
    </kbd>
  );
}

export function MenuItem({
  icon,
  label,
  detail,
  shortcut,
  danger,
  disabled = false,
}: {
  icon: ReactNode;
  label: string;
  detail?: string;
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
}) {
  return (
    <div
      className={cx(
        "flex min-h-9 cursor-default select-none items-center gap-2.5 rounded-md px-2.5 py-1.5 text-[12px] leading-5 outline-none data-[highlighted]:bg-[var(--surface-hover)]",
        danger ? "text-[#ff8989]" : "text-[var(--text)]",
        disabled && "opacity-45",
      )}
    >
      <span className="flex size-4 items-center justify-center text-[var(--muted-strong)]">{icon}</span>
      <span className="flex-1">{label}</span>
      {detail ? <span className="text-[11px] text-[var(--muted)]">{detail}</span> : null}
      {shortcut ? <Kbd>{shortcut}</Kbd> : null}
    </div>
  );
}
