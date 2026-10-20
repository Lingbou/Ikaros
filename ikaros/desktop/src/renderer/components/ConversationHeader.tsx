import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { Archive, FileText, Pencil } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  findThread,
  type Thread,
} from "../domain";
import { useTranslation } from "../i18n";
import { useAppStore } from "../store";

type HeaderThread = Pick<Thread, "id" | "title">;

let cachedHeaderThread: HeaderThread | undefined;

function selectHeaderThread(state: {
  threads: Thread[];
  selectedThreadId: string | null;
}): HeaderThread | undefined {
  const thread = findThread(state.threads, state.selectedThreadId);
  if (!thread) {
    cachedHeaderThread = undefined;
    return undefined;
  }
  const cached = cachedHeaderThread;
  if (cached?.id === thread.id && cached.title === thread.title) {
    return cached;
  }

  cachedHeaderThread = {
    id: thread.id,
    title: thread.title,
  };
  return cachedHeaderThread;
}

export function ConversationHeader() {
  const { t } = useTranslation();
  const thread = useAppStore(selectHeaderThread);
  const renameThread = useAppStore((state) => state.renameThread);
  const archiveThread = useAppStore((state) => state.archiveThread);
  const runtimeMode = useAppStore((state) => state.runtimeMode);
  const openFile = useAppStore((state) => state.openFile);
  const hasWorkspace = useAppStore((state) => {
    const current = findThread(state.threads, state.selectedThreadId);
    return Boolean(state.projects.find((project) => project.id === current?.projectId)?.rootUri);
  });
  const [renaming, setRenaming] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const titleInputRef = useRef<HTMLInputElement>(null);
  const cancelRenameRef = useRef(false);

  useEffect(() => {
    if (!renaming) return;
    titleInputRef.current?.focus();
    titleInputRef.current?.select();
  }, [renaming]);

  if (!thread) {
    return <div className="h-10 shrink-0" />;
  }

  const commitRename = (event?: FormEvent<HTMLFormElement>) => {
    event?.preventDefault();
    if (cancelRenameRef.current) {
      cancelRenameRef.current = false;
      return;
    }
    if (!renaming) return;
    setRenaming(false);
    const normalized = titleDraft.trim();
    void renameThread(thread.id, normalized || null).catch(() => undefined);
  };

  return (
    <div className="flex h-10 shrink-0 select-none items-center gap-3 border-b border-[var(--border-soft)] px-4 sm:px-5">
      {renaming ? (
        <form onSubmit={commitRename} className="min-w-0 flex-1">
          <input
            ref={titleInputRef}
            value={titleDraft}
            maxLength={200}
            aria-label={t("thread.renamePlaceholder")}
            onChange={(event) => setTitleDraft(event.currentTarget.value)}
            onBlur={() => commitRename()}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                cancelRenameRef.current = true;
                setRenaming(false);
              }
            }}
            className="h-7 w-full select-text rounded-md border border-[var(--border)] bg-[var(--panel)] px-2 text-[13px] leading-5 text-[var(--text)] outline-none focus:border-[var(--muted-strong)]"
          />
        </form>
      ) : (
        <h1
          aria-label={thread.title || t("sidebar.newChat")}
          className="min-w-0 flex-1 text-[13px] leading-5"
        >
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button
                type="button"
                title={t("thread.actions")}
                className="w-full truncate rounded-md px-1.5 py-1 text-left text-[13px] leading-5 text-[var(--text)] outline-none transition-colors hover:bg-[var(--surface-hover)] data-[state=open]:bg-[var(--surface-hover)]"
              >
                {thread.title || t("sidebar.newChat")}
              </button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content
                align="start"
                sideOffset={5}
                className="glass-menu z-[90] min-w-[170px] rounded-lg p-1"
              >
                <DropdownMenu.Item
                  onSelect={() => {
                    cancelRenameRef.current = false;
                    setTitleDraft(thread.title);
                    setRenaming(true);
                  }}
                  className="flex h-8 cursor-default items-center gap-2 rounded-md px-2 text-[12px] leading-[18px] text-[var(--text)] outline-none data-[highlighted]:bg-[var(--surface-hover)]"
                >
                  <Pencil size={12} aria-hidden="true" className="text-[var(--muted-strong)]" />
                  {t("thread.rename")}
                </DropdownMenu.Item>
                <DropdownMenu.Item
                  onSelect={() => void archiveThread(thread.id).catch(() => undefined)}
                  className="flex h-8 cursor-default items-center gap-2 rounded-md px-2 text-[12px] leading-[18px] text-[var(--text)] outline-none data-[highlighted]:bg-[var(--surface-hover)]"
                >
                  <Archive size={12} aria-hidden="true" className="text-[var(--muted-strong)]" />
                  {t("thread.archive")}
                </DropdownMenu.Item>
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </h1>
      )}

      {runtimeMode && hasWorkspace ? (
        <button
          type="button"
          onClick={() => openFile({ threadId: thread.id, path: "", view: "current" })}
          className="flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2 text-[12px] text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
        >
          <FileText size={13} aria-hidden="true" />
          {t("files.open")}
        </button>
      ) : null}
    </div>
  );
}