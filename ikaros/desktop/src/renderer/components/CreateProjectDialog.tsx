import * as Dialog from "@radix-ui/react-dialog";
import { Folder, FolderPlus, LoaderCircle, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";

import type { RuntimeWorkspaceSummary } from "../../shared/runtime";
import { useTranslation } from "../i18n";
import { useAppStore } from "../store";

interface CreateProjectDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
}

const MAX_PROJECT_NAME_LENGTH = 200;

export function CreateProjectDialog({ open, onOpenChange }: CreateProjectDialogProps) {
  const { t } = useTranslation();
  const stageProjectWorkspace = useAppStore((state) => state.stageProjectWorkspace);
  const [projectName, setProjectName] = useState("");
  const [nameWasEdited, setNameWasEdited] = useState(false);
  const [workspace, setWorkspace] = useState<RuntimeWorkspaceSummary | null>(null);
  const [choosingFolder, setChoosingFolder] = useState(false);
  const [chooseFailed, setChooseFailed] = useState(false);
  const pickerRevision = useRef(0);

  const reset = useCallback(() => {
    pickerRevision.current += 1;
    setProjectName("");
    setNameWasEdited(false);
    setWorkspace(null);
    setChoosingFolder(false);
    setChooseFailed(false);
  }, []);

  useEffect(() => {
    if (!open) reset();
  }, [open, reset]);

  const changeOpen = (nextOpen: boolean) => {
    if (!nextOpen) reset();
    onOpenChange(nextOpen);
  };

  const chooseFolder = async () => {
    if (choosingFolder) return;

    const revision = ++pickerRevision.current;
    setChoosingFolder(true);
    setChooseFailed(false);
    try {
      const selected = await window.ikarosDesktop.workspace.chooseDirectory();
      if (revision !== pickerRevision.current || !selected) return;
      setWorkspace(selected);
      if (!nameWasEdited) setProjectName(selected.name);
    } catch {
      if (revision === pickerRevision.current) setChooseFailed(true);
    } finally {
      if (revision === pickerRevision.current) setChoosingFolder(false);
    }
  };

  const normalizedName = projectName.trim();
  const nameIsValid =
    normalizedName.length > 0 && normalizedName.length <= MAX_PROJECT_NAME_LENGTH;
  const canCreate = workspace !== null && nameIsValid && !choosingFolder;

  const createProject = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!workspace || !canCreate) return;

    stageProjectWorkspace({
      ...workspace,
      name: normalizedName
    });
    changeOpen(false);
  };

  return (
    <Dialog.Root open={open} onOpenChange={changeOpen}>
      <Dialog.Portal>
        <Dialog.Overlay
          data-testid="create-project-overlay"
          onPointerDown={() => changeOpen(false)}
          className="fixed inset-0 z-[110] bg-black/55 backdrop-blur-[2px]"
        />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[120] w-[min(520px,calc(100vw-28px))] -translate-x-1/2 -translate-y-1/2 overflow-hidden rounded-[18px] border border-[var(--border-soft)] bg-[var(--menu)] shadow-[0_24px_70px_var(--shadow-color)]">
          <form onSubmit={createProject}>
            <div className="flex items-center gap-3 px-5 pb-3 pt-[18px]">
              <Dialog.Title className="min-w-0 flex-1 text-[15px] font-semibold leading-6 text-[var(--text)]">
                {t("project.create.title")}
              </Dialog.Title>
              <Dialog.Description className="sr-only">
                {t("project.create.addFolderDescription")}
              </Dialog.Description>
              <button
                type="button"
                aria-label={t("project.create.close")}
                onClick={() => changeOpen(false)}
                className="flex size-7 shrink-0 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
              >
                <X size={13} aria-hidden="true" />
              </button>
            </div>

            <div className="px-5">
              <label className="flex h-10 items-center gap-2.5 rounded-[10px] border border-[var(--border)] bg-[var(--panel)] px-3 transition-colors focus-within:border-[var(--accent)]">
                <Folder size={14} aria-hidden="true" className="shrink-0 text-[var(--muted-strong)]" />
                <span className="sr-only">{t("project.create.namePlaceholder")}</span>
                <input
                  autoFocus
                  type="text"
                  maxLength={MAX_PROJECT_NAME_LENGTH}
                  disabled={choosingFolder}
                  value={projectName}
                  onChange={(event) => {
                    setProjectName(event.currentTarget.value);
                    setNameWasEdited(true);
                  }}
                  placeholder={t("project.create.namePlaceholder")}
                  className="min-w-0 flex-1 bg-transparent text-[13px] leading-5 text-[var(--text)] outline-none placeholder:text-[var(--muted)] disabled:opacity-60"
                />
              </label>

              <div className="mt-4 text-[12px] font-semibold leading-[18px] text-[var(--text)]">
                {t("project.create.sourceFolder")}
              </div>
              <button
                type="button"
                disabled={choosingFolder}
                aria-label={
                  workspace ? t("project.create.changeFolder") : t("project.create.addFolder")
                }
                onClick={() => void chooseFolder()}
                className="mt-2 flex h-12 w-full items-center justify-between rounded-[10px] border border-[var(--border)] bg-[color-mix(in_srgb,var(--panel)_45%,transparent)] px-3 text-left outline-none transition-colors hover:border-[var(--muted)] hover:bg-[var(--surface-hover)] focus-visible:border-[var(--muted-strong)] disabled:cursor-wait disabled:opacity-60"
              >
                {workspace ? (
                  <span className="flex min-w-0 w-full items-center gap-2.5">
                    <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-[var(--panel-raised)] text-[var(--muted-strong)]">
                      <Folder size={14} aria-hidden="true" />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[12px] font-medium leading-5 text-[var(--text)]">
                        {workspace.name}
                      </span>
                      {workspace.rootUri ? (
                        <span className="block truncate text-[11px] leading-4 text-[var(--muted)]">
                          {workspace.rootUri}
                        </span>
                      ) : null}
                    </span>
                    <span className="shrink-0 text-[11px] font-medium text-[var(--muted-strong)]">
                      {t("project.create.changeFolder")}
                    </span>
                  </span>
                ) : (
                  <span className="flex min-w-0 w-full items-center justify-center gap-2">
                    {choosingFolder ? (
                      <LoaderCircle
                        size={15}
                        aria-hidden="true"
                        className="animate-spin text-[var(--muted)]"
                      />
                    ) : (
                      <FolderPlus size={15} aria-hidden="true" className="text-[var(--muted)]" />
                    )}
                    <span className="text-[12px] font-medium leading-5 text-[var(--text)]">
                      {t("project.create.addFolderDescription")}
                    </span>
                  </span>
                )}
              </button>

              <p
                role={chooseFailed ? "alert" : undefined}
                aria-live="polite"
                className="mt-2 min-h-4 text-[11px] leading-4 text-[#e08b8b]"
              >
                {chooseFailed ? t("project.create.chooseFailed") : ""}
              </p>
            </div>

            <div className="flex items-center justify-end gap-2 px-5 pb-5 pt-2">
              <button
                type="button"
                onClick={() => changeOpen(false)}
                className="h-8 rounded-lg px-3 text-[12px] leading-[18px] text-[var(--muted-strong)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)]"
              >
                {t("common.cancel")}
              </button>
              <button
                type="submit"
                disabled={!canCreate}
                className="h-8 rounded-lg bg-[var(--text)] px-4 text-[12px] font-medium leading-[18px] text-[var(--canvas)] outline-none transition-opacity hover:opacity-90 focus-visible:ring-2 focus-visible:ring-[var(--muted-strong)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--menu)] disabled:cursor-not-allowed disabled:opacity-35"
              >
                {t("project.create.create")}
              </button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
