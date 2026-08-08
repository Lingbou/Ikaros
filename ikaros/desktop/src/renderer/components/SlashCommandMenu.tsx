import { Boxes, ListChecks, Sparkles } from "lucide-react";
import { type TranslationKey, useTranslation } from "../i18n";

export interface SlashCommand {
  id: string;
  token: string;
  labelKey: TranslationKey;
  descriptionKey: TranslationKey;
  icon: "sparkles" | "plan" | "tools";
}

export const SLASH_COMMANDS: SlashCommand[] = [
  {
    id: "mock-feature-1",
    token: "/mock-1",
    labelKey: "slash.mock1.label",
    descriptionKey: "slash.mock1.description",
    icon: "sparkles",
  },
  {
    id: "mock-feature-2",
    token: "/mock-2",
    labelKey: "slash.mock2.label",
    descriptionKey: "slash.mock2.description",
    icon: "plan",
  },
  {
    id: "mock-feature-3",
    token: "/mock-3",
    labelKey: "slash.mock3.label",
    descriptionKey: "slash.mock3.description",
    icon: "tools",
  },
];

function CommandIcon({ icon }: { icon: SlashCommand["icon"] }) {
  if (icon === "plan") return <ListChecks size={15} />;
  if (icon === "tools") return <Boxes size={15} />;
  return <Sparkles size={15} />;
}

export function SlashCommandMenu({
  commands,
  activeIndex,
  onActiveIndexChange,
  onSelect,
}: {
  commands: SlashCommand[];
  activeIndex: number;
  onActiveIndexChange: (index: number) => void;
  onSelect: (command: SlashCommand) => void;
}) {
  const { t } = useTranslation();

  return (
    <div
      id="slash-command-menu"
      role="listbox"
      aria-label={t("slash.label")}
      className="glass-menu absolute inset-x-0 bottom-[calc(100%+8px)] z-[80] max-h-[286px] overflow-hidden rounded-2xl p-1.5 shadow-[0_20px_60px_rgba(0,0,0,.38)]"
    >
      <div className="px-2.5 pb-1.5 pt-1 text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--muted)]">
        {t("slash.heading")}
      </div>
      <div className="app-scrollbar max-h-[244px] overflow-y-auto">
        {commands.length ? (
          commands.map((command, index) => (
            <button
              id={`slash-command-${command.id}`}
              key={command.id}
              type="button"
              role="option"
              aria-selected={index === activeIndex}
              onMouseDown={(event) => event.preventDefault()}
              onMouseEnter={() => onActiveIndexChange(index)}
              onClick={() => onSelect(command)}
              className="flex min-h-11 w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left outline-none transition-colors aria-selected:bg-[var(--panel-selected)] hover:bg-[var(--surface-hover)]"
            >
              <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-[var(--panel-hover)] text-[var(--accent)]">
                <CommandIcon icon={command.icon} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[12px] font-medium text-[var(--text)]">
                  {t(command.labelKey)}
                </span>
                <span className="mt-0.5 block truncate text-[11px] leading-4 text-[var(--muted)]">
                  {t(command.descriptionKey)}
                </span>
              </span>
              <span className="font-mono text-[11px] text-[var(--muted)]">{command.token}</span>
            </button>
          ))
        ) : (
          <div className="px-3 py-6 text-center text-[12px] text-[var(--muted)]">
            {t("slash.noMatches")}
          </div>
        )}
      </div>
    </div>
  );
}
