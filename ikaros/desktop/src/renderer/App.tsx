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

export function App() {
  usePlatformPreferences();
  const { t } = useTranslation();
  const [composerClearance, setComposerClearance] = useState(150);
  const newChat = useAppStore((state) => state.newChat);
  const settingsOpen = useAppStore((state) => state.settingsOpen);
  const setSearchOpen = useAppStore((state) => state.setSearchOpen);

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
