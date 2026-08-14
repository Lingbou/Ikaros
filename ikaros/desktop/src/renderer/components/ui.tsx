import * as Tooltip from "@radix-ui/react-tooltip";
import {
  Children,
  cloneElement,
  createElement,
  isValidElement,
  type ButtonHTMLAttributes,
  type HTMLAttributes,
  type ReactNode,
} from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

const markdownPlugins: NonNullable<Parameters<typeof ReactMarkdown>[0]["remarkPlugins"]> = [
  [remarkGfm, { singleTilde: false }],
];
const leadingHeadingEmoji = new RegExp(
  String.raw`^(?<emoji>\p{RGI_Emoji})(?<gap>\p{White_Space}+)`,
  "v",
);
const headingInlineContainers = new Set(["strong", "em", "del"]);

function hideLeadingHeadingEmoji(children: ReactNode): ReactNode {
  let inspected = false;

  function visitChildren(value: ReactNode): ReactNode {
    return Children.map(value, visitNode);
  }

  function visitNode(value: ReactNode): ReactNode {
    if (inspected || value == null || typeof value === "boolean") {
      return value;
    }
    if (typeof value === "string") {
      if (value.length === 0) return value;
      inspected = true;
      const match = leadingHeadingEmoji.exec(value);
      if (!match) return value;
      return (
        <>
          <span aria-hidden="true" className="markdown-heading-emoji">
            {match[0]}
          </span>
          {value.slice(match[0].length)}
        </>
      );
    }
    if (typeof value === "number" || typeof value === "bigint") {
      inspected = true;
      return value;
    }
    if (
      isValidElement<{ children?: ReactNode }>(value) &&
      typeof value.type === "string" &&
      headingInlineContainers.has(value.type)
    ) {
      return cloneElement(value, undefined, visitChildren(value.props.children));
    }
    inspected = true;
    return value;
  }

  return visitChildren(children);
}

type MarkdownHeadingProps = HTMLAttributes<HTMLHeadingElement> & { node?: unknown };

function createMarkdownHeading(tagName: "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
  return function MarkdownHeading({ children, node: _node, ...props }: MarkdownHeadingProps) {
    return createElement(tagName, props, hideLeadingHeadingEmoji(children));
  };
}

const markdownComponents: Components = {
  a: ({ children, node: _node, ...props }) => (
    <a {...props} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
  h1: createMarkdownHeading("h1"),
  h2: createMarkdownHeading("h2"),
  h3: createMarkdownHeading("h3"),
  h4: createMarkdownHeading("h4"),
  h5: createMarkdownHeading("h5"),
  h6: createMarkdownHeading("h6"),
  table: ({ children, node: _node, ...props }) => (
    <div className="markdown-table-scroll">
      <table {...props}>{children}</table>
    </div>
  ),
};

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
      <ReactMarkdown
        remarkPlugins={markdownPlugins}
        components={markdownComponents}
      >
        {content}
      </ReactMarkdown>
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
