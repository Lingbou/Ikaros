import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { Archive, Check, ChevronDown, FileText, GitBranch, Pencil } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  findThread,
  type Branch,
  type Thread,
} from "../domain";
import { useTranslation, type Translate } from "../i18n";
import { useAppStore } from "../store";

type HeaderBranch = Pick<Branch, "id" | "label">;
type HeaderThread = Pick<
  Thread,
  "id" | "title" | "activeBranchId"
> & {
  branches: HeaderBranch[];
};

let cachedHeaderThread: HeaderThread | undefined;

function sameBranchLabel(left: Branch["label"], right: Branch["label"]): boolean {
  if (left.source !== right.source) return false;
  if (left.source === "external") {
    return right.source === "external" && left.value === right.value;
  }
  if (right.source !== "app" || left.kind !== right.kind) return false;
  return left.kind === "main" || (right.kind === "number" && left.number === right.number);
}

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
  if (
    cached?.id === thread.id &&
    cached.title === thread.title &&
    cached.activeBranchId === thread.activeBranchId &&
    cached.branches.length === thread.branches.length &&
    thread.branches.every((branch, index) => {
      const cachedBranch = cached.branches[index];
      return (
        cachedBranch?.id === branch.id && sameBranchLabel(cachedBranch.label, branch.label)
      );
    })
  ) {
    return cached;
  }

  cachedHeaderThread = {
    id: thread.id,
    title: thread.title,
    activeBranchId: thread.activeBranchId,
    branches: thread.branches.map(({ id, label }) => ({ id, label })),
  };
  return cachedHeaderThread;
}

function branchLabel(branch: Pick<Branch, "label">, t: Translate): string {
  if (branch.label.source === "external") return branch.label.value;
  if (branch.label.kind === "main") return t("branch.main");
  return t("branch.number", { number: branch.label.number });
}

export function ConversationHeader() {
  const { t } = useTranslation();
  const thread = useAppStore(selectHeaderThread);
  const switchBranch = useAppStore((state) => state.switchBranch);
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
  const branch = thread?.branches.find((candidate) => candidate.id === thread.activeBranchId);

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
          className="min-w-0 flex-1 text-[13px] leading-5 tracking-[-0.005em]"
        >
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button
                type="button"
                title={t("thread.actions")}
                className="w-full truncate rounded-md px-1.5 py-1 text-left text-[13px] leading-5 tracking-[-0.005em] text-[var(--text)] outline-none transition-colors hover:bg-[var(--surface-hover)] data-[state=open]:bg-[var(--surface-hover)]"
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
          className="flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2 text-[11px] text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
        >
          <FileText size={13} aria-hidden="true" />
          {t("files.open")}
        </button>
      ) : null}

      {thread.branches.length > 1 && branch ? (
        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button
              type="button"
              className="hidden h-7 items-center gap-1.5 rounded-lg px-2 text-[12px] leading-[18px] text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)] sm:flex"
            >
              <GitBranch size={12} />
              {branchLabel(branch, t)}
              <ChevronDown size={11} />
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content
              align="end"
              sideOffset={7}
              className="glass-menu z-[90] min-w-[180px] rounded-lg p-1"
            >
              {thread.branches.map((candidate) => (
                <DropdownMenu.Item
                  key={candidate.id}
                  onSelect={() => switchBranch(candidate.id)}
                  className="flex h-8 cursor-default items-center gap-2 rounded-md px-2 text-[12px] leading-[18px] text-[var(--text)] outline-none data-[highlighted]:bg-[var(--surface-hover)]"
                >
                  <span className="flex size-4 items-center justify-center text-[var(--accent)]">
                    {candidate.id === thread.activeBranchId ? <Check size={12} /> : null}
                  </span>
                  {branchLabel(candidate, t)}
                </DropdownMenu.Item>
              ))}
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      ) : null}
    </div>
  );
}
