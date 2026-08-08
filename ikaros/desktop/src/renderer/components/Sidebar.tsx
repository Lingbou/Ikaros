import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import * as ScrollArea from "@radix-ui/react-scroll-area";
import {
  Folder,
  PanelRightClose,
  Plus,
  Search,
  Settings,
} from "lucide-react";
import { standaloneThreads, type Thread } from "../domain";
import { useTranslation } from "../i18n";
import { profileInitials } from "../localProfile";
import { useAppStore } from "../store";
import { cx, IconButton } from "./ui";

type NavigationThread = Pick<Thread, "id" | "projectId" | "title">;
type ThreadNavigation = {
  threads: NavigationThread[];
  recentThreads: NavigationThread[];
};

let cachedThreadNavigation: ThreadNavigation | undefined;

function selectThreadNavigation(state: { threads: Thread[] }): ThreadNavigation {
  const cached = cachedThreadNavigation;
  if (
    cached &&
    cached.threads.length === state.threads.length &&
    state.threads.every((thread, index) => {
      const cachedThread = cached.threads[index];
      return (
        cachedThread?.id === thread.id &&
        cachedThread.title === thread.title &&
        cachedThread.projectId === thread.projectId
      );
    })
  ) {
    return cached;
  }

  cachedThreadNavigation = {
    threads: state.threads.map(({ id, projectId, title }) => ({ id, projectId, title })),
    recentThreads: standaloneThreads(state.threads).map(({ id, projectId, title }) => ({
      id,
      projectId,
      title,
    })),
  };
  return cachedThreadNavigation;
}

function NavigationButton({
  icon,
  label,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="group flex h-[30px] w-full items-center gap-2 rounded-[10px] px-2 text-left text-[13px] leading-5 text-[var(--text)] transition-colors hover:bg-[var(--surface-hover)]"
    >
      <span className="flex size-4 items-center justify-center text-[var(--muted-strong)] group-hover:text-[var(--text)]">
        {icon}
      </span>
      <span className="flex-1 truncate">{label}</span>
    </button>
  );
}

export function Sidebar() {
  const { t } = useTranslation();
  const projects = useAppStore((state) => state.projects);
  const { threads, recentThreads } = useAppStore(selectThreadNavigation);
  const selectedThreadId = useAppStore((state) => state.selectedThreadId);
  const sidebarOpen = useAppStore((state) => state.sidebarOpen);
  const profileUsername = useAppStore((state) => state.profileUsername);
  const expandedProjects = useAppStore((state) => state.expandedProjects);
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);
  const setSearchOpen = useAppStore((state) => state.setSearchOpen);
  const setSettingsOpen = useAppStore((state) => state.setSettingsOpen);
  const selectThread = useAppStore((state) => state.selectThread);
  const toggleProject = useAppStore((state) => state.toggleProject);
  const newChat = useAppStore((state) => state.newChat);

  return (
    <aside
      data-open={sidebarOpen}
      aria-label={t("sidebar.workspaceNavigation")}
      className="responsive-sidebar sidebar-gradient flex w-[clamp(248px,14.3vw,274px)] shrink-0 flex-col overflow-hidden border-r border-[var(--border-soft)] transition-[width,transform] duration-200"
    >
      <div className="flex h-[50px] shrink-0 items-center gap-2 px-3.5">
        <div
          aria-label={t("sidebar.currentWorkspace", { name: "Ikaros" })}
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
        >
          <span className="truncate text-[13px] font-semibold leading-5 tracking-[-0.01em]">Ikaros</span>
        </div>
        <IconButton
          label={t("sidebar.searchConversations")}
          tooltipSide="bottom"
          className="size-7"
          onClick={() => setSearchOpen(true)}
        >
          <Search size={15} />
        </IconButton>
        <IconButton
          label={t("sidebar.hideSidebar")}
          tooltipSide="bottom"
          className="size-7 md:hidden"
          onClick={() => setSidebarOpen(false)}
        >
          <PanelRightClose size={15} />
        </IconButton>
      </div>

      <div className="shrink-0 space-y-0.5 px-2">
        <NavigationButton icon={<Plus size={15} />} label={t("sidebar.newChat")} onClick={newChat} />
      </div>

      <ScrollArea.Root className="mt-4 min-h-0 flex-1 overflow-hidden">
        <ScrollArea.Viewport className="app-scrollbar h-full w-full px-2 pb-5">
          <div className="px-1.5 pb-1.5 text-[12px] font-semibold leading-4 text-[var(--muted)]">
            {t("sidebar.projects")}
          </div>
          <div className="space-y-2">
            {projects.map((project) => {
              const expanded = expandedProjects[project.id];
              const projectThreads = threads.filter(
                (thread) => thread.projectId === project.id,
              );
              return (
                <section key={project.id} aria-labelledby={`${project.id}-label`}>
                  <button
                    id={`${project.id}-label`}
                    type="button"
                    aria-expanded={expanded}
                    onClick={() => toggleProject(project.id)}
                    className="group flex h-[30px] w-full items-center gap-2 rounded-[10px] px-2 text-left text-[13px] font-medium leading-5 text-[var(--text)] transition-colors hover:bg-[var(--surface-hover)]"
                  >
                    <Folder size={15} className="shrink-0" style={{ color: project.color }} />
                    <span className="flex-1 truncate">{project.name}</span>
                  </button>
                  {expanded ? (
                    <div className="mt-0.5 space-y-px">
                      {projectThreads.length ? (
                        projectThreads.map((thread) => (
                          <button
                            key={thread.id}
                            type="button"
                            aria-current={selectedThreadId === thread.id ? "page" : undefined}
                            onClick={() => void selectThread(thread.id)}
                            className={cx(
                              "flex h-[30px] w-full items-center rounded-[10px] pl-[31px] pr-2 text-left text-[13px] leading-5 transition-colors",
                              selectedThreadId === thread.id
                                ? "bg-[var(--panel-selected)] font-medium text-[var(--text)]"
                                : "text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]",
                            )}
                          >
                            <span className="min-w-0 flex-1 truncate">{thread.title}</span>
                          </button>
                        ))
                      ) : (
                        <div className="flex h-[30px] items-center pl-[31px] pr-2 text-[12px] leading-[18px] text-[var(--muted)]">
                          {t("sidebar.noChats")}
                        </div>
                      )}
                    </div>
                  ) : null}
                </section>
              );
            })}
          </div>

          <section aria-labelledby="recent-chats-label">
            <div id="recent-chats-label" className="mt-1 px-1.5 pb-1.5 text-[12px] font-semibold leading-4 text-[var(--muted)]">
              {t("sidebar.recents")}
            </div>
            <div className="space-y-px">
              {recentThreads.map((thread) => (
                <button
                  key={`recent-${thread.id}`}
                  type="button"
                  aria-current={selectedThreadId === thread.id ? "page" : undefined}
                  onClick={() => void selectThread(thread.id)}
                  className={cx(
                    "flex h-[30px] w-full items-center rounded-[10px] px-2 text-left text-[13px] leading-5 transition-colors",
                    selectedThreadId === thread.id
                      ? "bg-[var(--panel-selected)] font-medium text-[var(--text)]"
                      : "text-[var(--muted-strong)] hover:bg-[var(--surface-hover)] hover:text-[var(--text)]",
                  )}
                >
                  <span className="min-w-0 flex-1 truncate">{thread.title}</span>
                </button>
              ))}
            </div>
          </section>
        </ScrollArea.Viewport>
        <ScrollArea.Scrollbar orientation="vertical" className="flex w-2.5 touch-none p-0.5">
          <ScrollArea.Thumb className="relative flex-1 rounded-full bg-[var(--border)]" />
        </ScrollArea.Scrollbar>
      </ScrollArea.Root>

      <div className="shrink-0 border-t border-[var(--border-soft)] px-2 py-1">
        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button
              type="button"
              data-profile-menu-trigger
              aria-label={t("sidebar.openProfileMenu")}
              className="flex h-[38px] w-full items-center gap-2 rounded-[10px] px-[7px] text-left outline-none transition-colors hover:bg-[var(--surface-hover)] data-[state=open]:bg-[var(--panel-hover)]"
            >
              <span className="flex size-[18px] items-center justify-center rounded-full bg-[var(--accent)] text-[10px] font-medium leading-[14px] text-[var(--canvas)]">
                {profileInitials(profileUsername)}
              </span>
              <span className="min-w-0 flex-1 truncate text-[12px] leading-[18px] text-[var(--text)]">
                {profileUsername}
              </span>
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content
              side="top"
              align="start"
              sideOffset={8}
              className="glass-menu z-[100] w-[230px] rounded-xl p-1.5"
            >
              <DropdownMenu.Item
                onSelect={() => setSettingsOpen(true)}
                className="flex h-9 cursor-default items-center gap-2.5 rounded-lg px-2.5 text-[12px] leading-[18px] text-[var(--text)] outline-none data-[highlighted]:bg-[var(--surface-hover)]"
              >
                <Settings size={14} className="text-[var(--muted-strong)]" />
                {t("sidebar.settings")}
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      </div>
    </aside>
  );
}
