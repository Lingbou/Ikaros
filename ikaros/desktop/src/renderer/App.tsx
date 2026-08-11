import * as Tooltip from "@radix-ui/react-tooltip";
import { useEffect, useState } from "react";
import { Composer } from "./components/Composer";
import { ConversationHeader } from "./components/ConversationHeader";
import { EditMessageDialog } from "./components/EditMessageDialog";
import { EventFeed } from "./components/EventFeed";
import { SearchDialog } from "./components/SearchDialog";
import { SettingsPage } from "./components/SettingsPage";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { useTranslation } from "./i18n";
import { usePlatformPreferences } from "./platformPreferences";
import { useAppStore } from "./store";

const RUNTIME_INITIALIZATION_RETRY_DELAYS_MS = [100, 250, 500] as const;

function waitForInitializationRetry(delayMs: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve();
      return;
    }

    const finish = () => {
      signal.removeEventListener("abort", abort);
      resolve();
    };
    const timer = globalThis.setTimeout(finish, delayMs);
    const abort = () => {
      globalThis.clearTimeout(timer);
      finish();
    };
    signal.addEventListener("abort", abort, { once: true });
  });
}

export async function initializeRuntimeWithRetry(
  initializeRuntime: () => Promise<void>,
  isReady: () => boolean,
  signal: AbortSignal,
): Promise<void> {
  for (
    let attempt = 0;
    attempt <= RUNTIME_INITIALIZATION_RETRY_DELAYS_MS.length;
    attempt += 1
  ) {
    if (signal.aborted || isReady()) {
      return;
    }
    await initializeRuntime();
    if (signal.aborted || isReady()) {
      return;
    }

    const delayMs = RUNTIME_INITIALIZATION_RETRY_DELAYS_MS[attempt];
    if (delayMs === undefined) {
      return;
    }
    await waitForInitializationRetry(delayMs, signal);
  }
}

export function App() {
  usePlatformPreferences();
  const { t } = useTranslation();
  const [composerClearance, setComposerClearance] = useState(150);
  const newChat = useAppStore((state) => state.newChat);
  const initializeRuntime = useAppStore((state) => state.initializeRuntime);
  const settingsOpen = useAppStore((state) => state.settingsOpen);
  const setSearchOpen = useAppStore((state) => state.setSearchOpen);

  useEffect(() => {
    const controller = new AbortController();
    void initializeRuntimeWithRetry(
      initializeRuntime,
      () => useAppStore.getState().runtimeReady,
      controller.signal,
    );
    return () => controller.abort();
  }, [initializeRuntime]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key.toLowerCase() === "k" && !settingsOpen) {
        event.preventDefault();
        setSearchOpen(true);
      }
      if (event.key.toLowerCase() === "n") {
        event.preventDefault();
        newChat();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [newChat, setSearchOpen, settingsOpen]);

  return (
    <Tooltip.Provider>
      <div className="flex h-full min-h-0 flex-col bg-[var(--canvas)]">
        <TitleBar />
        {settingsOpen ? (
          <SettingsPage />
        ) : (
          <div className="relative flex min-h-0 flex-1 overflow-hidden">
            <Sidebar />
            <main className="relative flex min-w-0 flex-1 flex-col bg-[var(--canvas)]">
              <ConversationHeader />
              <section
                aria-label={t("app.conversation")}
                className="conversation-stage relative min-h-0 flex-1"
              >
                <EventFeed bottomClearance={composerClearance} />
                <Composer onClearanceChange={setComposerClearance} />
              </section>
            </main>
          </div>
        )}
      </div>
      <SearchDialog />
      <EditMessageDialog />
    </Tooltip.Provider>
  );
}
