import * as Dialog from "@radix-ui/react-dialog";
import { WifiOff } from "lucide-react";
import { useState } from "react";

import { useTranslation } from "../i18n";
import { useAppStore } from "../store";

/** Blocking notice shown while the local Runtime is unreachable. */
export function RuntimeConnectionDialog() {
  const { t } = useTranslation();
  const status = useAppStore((state) => state.runtimeConnectionStatus);
  const error = useAppStore((state) => state.runtimeError);
  const retry = useAppStore((state) => state.retryRuntimeConnection);
  const [detailsOpen, setDetailsOpen] = useState(false);

  if (status !== "offline") return null;

  return (
    <Dialog.Root open>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[130] bg-black/55 backdrop-blur-[2px]" />
        <Dialog.Content
          role="alertdialog"
          aria-describedby="runtime-offline-description"
          onEscapeKeyDown={(event) => event.preventDefault()}
          onInteractOutside={(event) => event.preventDefault()}
          className="glass-menu fixed left-1/2 top-1/2 z-[140] w-[min(460px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 overflow-hidden rounded-2xl"
        >
          <div className="flex items-start gap-3 px-6 pt-6">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-[#e07070]/12 text-[#e07070]">
              <WifiOff size={17} aria-hidden="true" />
            </span>
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-[15px] font-semibold leading-6 text-[var(--text)]">
                {t("runtime.offline.title")}
              </Dialog.Title>
              <Dialog.Description
                id="runtime-offline-description"
                className="mt-1.5 text-[12px] leading-5 text-[var(--muted)]"
              >
                {t("runtime.offline.description")}
              </Dialog.Description>
            </div>
          </div>
          {error ? (
            <div className="px-6 pt-4">
              <button
                type="button"
                onClick={() => setDetailsOpen((open) => !open)}
                className="text-[11px] font-medium leading-4 text-[var(--muted-strong)] transition-colors hover:text-[var(--text)]"
              >
                {t(
                  detailsOpen
                    ? "runtime.offline.hideDetails"
                    : "runtime.offline.showDetails",
                )}
              </button>
              {detailsOpen ? (
                <pre className="app-scrollbar mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-[var(--border-soft)] bg-[var(--panel)] px-3 py-2 font-mono text-[11px] leading-4 text-[var(--muted)]">
                  {error}
                </pre>
              ) : null}
            </div>
          ) : null}
          <div className="flex items-center justify-end gap-2 px-6 py-5">
            <button
              type="button"
              onClick={() => void window.ikarosDesktop?.windowControls.close()}
              className="inline-flex h-8 items-center rounded-lg px-3 text-[12px] font-medium text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
            >
              {t("runtime.offline.quit")}
            </button>
            <button
              type="button"
              onClick={() => void retry()}
              className="inline-flex h-8 items-center rounded-lg bg-[var(--text)] px-3.5 text-[12px] font-medium text-[var(--canvas)] transition-opacity hover:opacity-85"
            >
              {t("runtime.offline.reconnect")}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
