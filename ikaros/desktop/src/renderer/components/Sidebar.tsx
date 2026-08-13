import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import * as ScrollArea from "@radix-ui/react-scroll-area";
import {
  ChevronDown,
  ChevronRight,
  SquarePen,
  Folder,
  PanelRightClose,
  Plus,
  Search,
  Settings,
} from "lucide-react";
import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  DEFAULT_SIDEBAR_WIDTH,
  MAX_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
} from "../../shared/platform";
import { standaloneThreads, type Thread } from "../domain";
import { useTranslation } from "../i18n";
import { profileInitials } from "../localProfile";
import { useAppStore } from "../store";
import { CreateProjectDialog } from "./CreateProjectDialog";
import { cx, IconButton } from "./ui";

type NavigationThread = Pick<Thread, "id" | "projectId" | "title">;
type ThreadNavigation = {
  threads: NavigationThread[];
  recentThreads: NavigationThread[];
};

let cachedThreadNavigation: ThreadNavigation | undefined;

const SIDEBAR_KEYBOARD_STEP = 10;
const SIDEBAR_KEYBOARD_LARGE_STEP = 40;
const RESIZABLE_SIDEBAR_MEDIA_QUERY = "(min-width: 900px)";

function clampSidebarWidth(width: number): number {
  return Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, Math.round(width)));
}

function canResizeSidebar(): boolean {
  return typeof window.matchMedia === "function"
    ? window.matchMedia(RESIZABLE_SIDEBAR_MEDIA_QUERY).matches
    : window.innerWidth >= 900;
}

function useResizableSidebar(): boolean {
  const [resizable, setResizable] = useState(canResizeSidebar);

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const query = window.matchMedia(RESIZABLE_SIDEBAR_MEDIA_QUERY);
    const update = () => setResizable(query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  return resizable;
}

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
  const sidebarWidth = useAppStore((state) => state.sidebarWidth);
  const profileUsername = useAppStore((state) => state.profileUsername);
  const expandedProjects = useAppStore((state) => state.expandedProjects);
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);
  const setSidebarWidth = useAppStore((state) => state.setSidebarWidth);
  const setSearchOpen = useAppStore((state) => state.setSearchOpen);
  const setSettingsOpen = useAppStore((state) => state.setSettingsOpen);
  const selectThread = useAppStore((state) => state.selectThread);
  const toggleProject = useAppStore((state) => state.toggleProject);
  const newChat = useAppStore((state) => state.newChat);
  const [createProjectOpen, setCreateProjectOpen] = useState(false);
  const [projectsExpanded, setProjectsExpanded] = useState(true);
  const [recentsExpanded, setRecentsExpanded] = useState(true);
  const [resizing, setResizing] = useState(false);
  const resizable = useResizableSidebar();
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startWidth: number;
    previewWidth: number;
  } | null>(null);

  const persistSidebarWidth = (width: number) => {
    const nextWidth = clampSidebarWidth(width);
    setSidebarWidth(nextWidth);
    void window.ikarosDesktop?.preferences.update({ sidebarWidth: nextWidth }).catch(() => undefined);
  };

  useEffect(
    () => () => {
      const drag = dragRef.current;
      dragRef.current = null;
      if (drag) setSidebarWidth(drag.startWidth);
      delete document.documentElement.dataset.resizingSidebar;
    },
    [setSidebarWidth],
  );

  useEffect(() => {
    if (resizable || !dragRef.current) return;
    const { startWidth } = dragRef.current;
    dragRef.current = null;
    setResizing(false);
    delete document.documentElement.dataset.resizingSidebar;
    setSidebarWidth(startWidth);
  }, [resizable, setSidebarWidth]);

  const beginResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !resizable) return;
    event.preventDefault();
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: sidebarWidth,
      previewWidth: sidebarWidth,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    setResizing(true);
    document.documentElement.dataset.resizingSidebar = "true";
  };

  const previewResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const nextWidth = clampSidebarWidth(drag.startWidth + event.clientX - drag.startX);
    drag.previewWidth = nextWidth;
    setSidebarWidth(nextWidth);
  };

  const finishResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const nextWidth = drag.previewWidth;
    dragRef.current = null;
    setResizing(false);
    delete document.documentElement.dataset.resizingSidebar;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    persistSidebarWidth(nextWidth);
  };

  const cancelResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setResizing(false);
    delete document.documentElement.dataset.resizingSidebar;
    setSidebarWidth(drag.startWidth);
  };

  const handleLostPointerCapture = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const nextWidth = drag.previewWidth;
    dragRef.current = null;
    setResizing(false);
    delete document.documentElement.dataset.resizingSidebar;
    persistSidebarWidth(nextWidth);
  };

  const resizeWithKeyboard = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!resizable) return;
    const step = event.shiftKey ? SIDEBAR_KEYBOARD_LARGE_STEP : SIDEBAR_KEYBOARD_STEP;
    const nextWidth =
      event.key === "ArrowLeft"
        ? sidebarWidth - step
        : event.key === "ArrowRight"
          ? sidebarWidth + step
          : event.key === "Home"
            ? MIN_SIDEBAR_WIDTH
            : event.key === "End"
              ? MAX_SIDEBAR_WIDTH
              : null;
    if (nextWidth === null) return;
    event.preventDefault();
    persistSidebarWidth(nextWidth);
  };

  return (
    <>
    <aside
      data-open={sidebarOpen}
      data-resizing={resizing || undefined}
      aria-label={t("sidebar.workspaceNavigation")}
      className="responsive-sidebar sidebar-gradient relative flex shrink-0 flex-col overflow-visible transition-[width,transform] duration-200"
      style={{ width: sidebarOpen ? sidebarWidth : 0 }}
    >
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden border-r border-[var(--border-soft)]">
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
          <section aria-labelledby="project-chats-label">
            <div className="group flex h-7 items-center px-1.5 text-[12px] font-semibold leading-4 text-[var(--muted)]">
              <button
                id="project-chats-label"
                type="button"
                aria-expanded={projectsExpanded}
                onClick={() => setProjectsExpanded((expanded) => !expanded)}
                className="flex min-w-0 items-center gap-1 rounded-md outline-none transition-colors hover:text-[var(--text)] focus-visible:text-[var(--text)]"
              >
                <span>{t("sidebar.projects")}</span>
                <span className="opacity-0 transition-opacity group-hover:opacity-100">
                  {projectsExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                </span>
              </button>
              <span className="min-w-0 flex-1" />
              <IconButton
                label={t("sidebar.newProjectChat")}
                className="size-6 opacity-0 transition-opacity group-hover:opacity-100"
                onClick={() => setCreateProjectOpen(true)}
              >
                <Plus size={13} />
              </IconButton>
            </div>
            {projectsExpanded ? <div className="space-y-2">
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
                            <span className="min-w-0 flex-1 truncate">
                              {thread.title || t("sidebar.newChat")}
                            </span>
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
            </div> : null}
          </section>

          <section aria-labelledby="recent-chats-label" className="mt-1">
            <div className="group flex h-7 items-center px-1.5 text-[12px] font-semibold leading-4 text-[var(--muted)]">
              <button
                id="recent-chats-label"
                type="button"
                aria-expanded={recentsExpanded}
                onClick={() => setRecentsExpanded((expanded) => !expanded)}
                className="flex min-w-0 items-center gap-1 rounded-md outline-none transition-colors hover:text-[var(--text)] focus-visible:text-[var(--text)]"
              >
                <span>{t("sidebar.recents")}</span>
                <span className="opacity-0 transition-opacity group-hover:opacity-100">
                  {recentsExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                </span>
              </button>
              <span className="min-w-0 flex-1" />
              <IconButton
                label={t("sidebar.newChat")}
                className="size-6 opacity-0 transition-opacity group-hover:opacity-100"
                onClick={newChat}
              >
                <SquarePen size={13} />
              </IconButton>
            </div>
            {recentsExpanded ? <div className="space-y-px">
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
                  <span className="min-w-0 flex-1 truncate">
                    {thread.title || t("sidebar.newChat")}
                  </span>
                </button>
              ))}
            </div> : null}
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
      </div>
      <div
        role="separator"
        aria-label={t("sidebar.resize")}
        aria-orientation="vertical"
        aria-valuemin={MIN_SIDEBAR_WIDTH}
        aria-valuemax={MAX_SIDEBAR_WIDTH}
        aria-valuenow={sidebarWidth}
        tabIndex={sidebarOpen && resizable ? 0 : -1}
        aria-hidden={!resizable || undefined}
        data-resizing={resizing || undefined}
        className="sidebar-resizer"
        onPointerDown={beginResize}
        onPointerMove={previewResize}
        onPointerUp={finishResize}
        onPointerCancel={cancelResize}
        onLostPointerCapture={handleLostPointerCapture}
        onDoubleClick={(event) => {
          event.preventDefault();
          if (resizable) persistSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
        }}
        onKeyDown={resizeWithKeyboard}
      />
    </aside>
    <CreateProjectDialog open={createProjectOpen} onOpenChange={setCreateProjectOpen} />
    </>
  );
}
