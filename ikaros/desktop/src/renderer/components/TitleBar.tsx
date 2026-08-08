import { ChevronLeft, ChevronRight, PanelLeft } from "lucide-react";
import { useTranslation } from "../i18n";
import { IconButton } from "./ui";
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

  return (
    <header className="titlebar-drag relative z-50 flex h-9 shrink-0 items-center border-b border-[var(--border-soft)] bg-[var(--panel)] pr-[146px] text-[12px] text-[var(--muted-strong)]">
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
    </header>
  );
}
