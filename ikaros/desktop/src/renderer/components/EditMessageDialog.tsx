import * as Dialog from "@radix-ui/react-dialog";
import { GitBranch, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "../i18n";
import { useAppStore } from "../store";
import { IconButton } from "./ui";

export function EditMessageDialog() {
  const { t } = useTranslation();
  const editing = useAppStore((state) => state.editingMessage);
  const cancel = useAppStore((state) => state.cancelEditMessage);
  const commit = useAppStore((state) => state.commitMessageEdit);
  const [content, setContent] = useState("");

  useEffect(() => {
    setContent(editing?.content ?? "");
  }, [editing]);

  return (
    <Dialog.Root open={Boolean(editing)} onOpenChange={(open) => !open && cancel()}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[110] bg-black/55 backdrop-blur-[2px]" />
        <Dialog.Content className="glass-menu fixed left-1/2 top-1/2 z-[120] w-[min(620px,calc(100vw-28px))] -translate-x-1/2 -translate-y-1/2 rounded-2xl p-4">
          <div className="flex items-start gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-[color-mix(in_srgb,var(--accent)_10%,transparent)] text-[var(--accent)]">
              <GitBranch size={16} />
            </span>
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-[14px] font-medium text-[var(--text)]">{t("edit.title")}</Dialog.Title>
              <Dialog.Description className="mt-1 text-[11px] leading-4 text-[var(--muted)]">
                {t("edit.description")}
              </Dialog.Description>
            </div>
            <IconButton label={t("edit.cancel")} onClick={cancel}>
              <X size={14} />
            </IconButton>
          </div>
          <textarea
            autoFocus
            value={content}
            onChange={(event) => setContent(event.target.value)}
            className="app-scrollbar mt-4 min-h-28 w-full resize-y rounded-xl border border-[var(--border)] bg-[var(--panel)] px-3 py-2.5 text-[14px] leading-[22px] text-[var(--text)] outline-none placeholder:text-[var(--muted)]"
          />
          <div className="mt-4 flex justify-end gap-2">
            <button type="button" onClick={cancel} className="h-8 rounded-lg px-3 text-[12px] text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]">
              {t("common.cancel")}
            </button>
            <button
              type="button"
              onClick={() => commit(content)}
              disabled={!content.trim()}
              className="h-8 rounded-lg bg-[var(--text)] px-3 text-[12px] font-medium text-[var(--canvas)] hover:opacity-90 disabled:opacity-40"
            >
              {t("edit.createBranch")}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
