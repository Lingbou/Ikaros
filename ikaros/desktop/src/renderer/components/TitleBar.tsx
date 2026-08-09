import { ChevronLeft, ChevronRight, Minus, PanelLeft, Square, X } from "lucide-react";
import { useTranslation } from "../i18n";
import { cx, IconButton } from "./ui";
import { useAppStore } from "../store";

const APPLICATION_MENUS = [
  ["file", "titlebar.file"],
  ["edit", "titlebar.edit"],
  ["view", "titlebar.view"],
  ["help", "titlebar.help"],
] as const;

export function TitleBar() {
  const { t } = useTranslation();
  const sidebarOpen = useAppStore((state) => state.sidebarOpen);
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);
  const windowControls = window.ikarosDesktop.windowControls;

  return (
    <header
      onDoubleClick={(event) => {
        if (windowControls.usesCustomTitleBar && event.target === event.currentTarget) {
          void windowControls.toggleMaximize();
        }
      }}
      className={cx(
        "titlebar-drag relative z-50 flex h-9 shrink-0 items-center border-b border-[var(--border-soft)] bg-[var(--panel)] text-[12px] text-[var(--muted-strong)]",
        !windowControls.usesCustomTitleBar && "pr-[146px]",
      )}
    >
      <div className="titlebar-no-drag flex h-full items-center gap-0.5 px-2">
        <IconButton
          label={sidebarOpen ? t("titlebar.hideSidebar") : t("titlebar.showSidebar")}
          tooltipSide="bottom"
          className="size-7 rounded-md"
          onClick={() => setSidebarOpen(!sidebarOpen)}
        >
          <PanelLeft size={14} strokeWidth={1.8} />
        </IconButton>
        <IconButton label={t("titlebar.back")} tooltipSide="bottom" className="size-7" disabled>
          <ChevronLeft size={14} />
        </IconButton>
        <IconButton label={t("titlebar.forward")} tooltipSide="bottom" className="size-7" disabled>
          <ChevronRight size={14} />
        </IconButton>
      </div>
      <nav aria-label={t("titlebar.applicationMenu")} className="titlebar-no-drag ml-1 flex items-center gap-0.5">
        {APPLICATION_MENUS.map(([id, labelKey]) => {
          const label = t(labelKey);
          return (
          <button
            key={id}
            type="button"
            disabled
            aria-label={t("titlebar.menuUnavailable", { menu: label })}
            title={t("common.notAvailableYet")}
            className="cursor-default rounded px-2 py-1 text-[var(--muted)]"
          >
            {label}
          </button>
          );
        })}
      </nav>
      {windowControls.usesCustomTitleBar ? (
        <div className="titlebar-no-drag ml-auto flex h-full items-stretch">
          <button
            type="button"
            aria-label={t("titlebar.minimize")}
            title={t("titlebar.minimize")}
            className="flex h-full w-[46px] items-center justify-center text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
            onClick={() => void windowControls.minimize()}
          >
            <Minus size={14} strokeWidth={1.5} />
          </button>
          <button
            type="button"
            aria-label={t("titlebar.maximize")}
            title={t("titlebar.maximize")}
            className="flex h-full w-[46px] items-center justify-center text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
            onClick={() => void windowControls.toggleMaximize()}
          >
            <Square size={11} strokeWidth={1.5} />
          </button>
          <button
            type="button"
            aria-label={t("titlebar.close")}
            title={t("titlebar.close")}
            className="flex h-full w-[46px] items-center justify-center text-[var(--muted-strong)] transition-colors hover:bg-[#c42b1c] hover:text-white"
            onClick={() => void windowControls.close()}
          >
            <X size={14} strokeWidth={1.5} />
          </button>
        </div>
      ) : null}
    </header>
  );
}
