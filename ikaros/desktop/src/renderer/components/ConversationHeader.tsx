import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { Check, ChevronDown, GitBranch } from "lucide-react";
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
  const branch = thread?.branches.find((candidate) => candidate.id === thread.activeBranchId);

  if (!thread) {
    return <div className="h-10 shrink-0" />;
  }

  return (
    <div className="flex h-10 shrink-0 select-none items-center gap-3 border-b border-[var(--border-soft)] px-4 sm:px-5">
      <h1 className="min-w-0 flex-1 truncate text-[13px] leading-5 tracking-[-0.005em] text-[var(--text)]">
        {thread.title}
      </h1>

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
