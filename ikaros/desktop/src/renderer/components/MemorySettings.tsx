import * as Dialog from "@radix-ui/react-dialog";
import {
  Brain,
  ChevronDown,
  CircleAlert,
  Eye,
  LoaderCircle,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
  X
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import type {
  RuntimeMemoryKind,
  RuntimeMemoryListParams,
  RuntimeMemoryProvenance,
  RuntimeMemoryRecord,
  RuntimeMemoryScope,
  RuntimeMemoryState,
  RuntimeMemorySummary
} from "../../shared/runtime";
import { useTranslation, type TranslationKey } from "../i18n";
import {
  createRuntimeClient,
  isRuntimeRpcError,
  type RuntimeClient
} from "../runtimeClient";
import { cx } from "./ui";

const PAGE_SIZE = 25;
const CONTENT_LIMIT = 2_048;

const MEMORY_KINDS: readonly RuntimeMemoryKind[] = [
  "fact",
  "preference",
  "relationship",
  "project"
];

const KIND_LABELS: Readonly<Record<RuntimeMemoryKind, TranslationKey>> = {
  fact: "settings.memory.kind.fact",
  preference: "settings.memory.kind.preference",
  relationship: "settings.memory.kind.relationship",
  project: "settings.memory.kind.project"
};

type LoadStatus = "loading" | "ready" | "error";
type LoadMoreStatus = "idle" | "loading" | "error";
type ScopeFilter = "all" | "global" | `workspace:${string}`;
type KindFilter = "all" | RuntimeMemoryKind;
type EditorMode = "create" | "correct" | "view";

interface MemoryWorkspace {
  id: string;
  name: string;
}

interface EditorState {
  mode: EditorMode;
  summary: RuntimeMemorySummary | null;
}

interface MemoryDetailState {
  revision: number;
  record: RuntimeMemoryRecord | null;
}

interface MemorySettingsProps {
  workspaces: readonly MemoryWorkspace[];
  preferredWorkspaceId: string | null;
}

function requestId(operation: "create" | "correct" | "forget"): string {
  return `memory_${operation}_${globalThis.crypto.randomUUID()}`;
}

function scopeFromFilter(filter: ScopeFilter): RuntimeMemoryScope | undefined {
  if (filter === "all") return undefined;
  if (filter === "global") return { type: "global", key: null };
  return { type: "workspace", key: filter.slice("workspace:".length) };
}

function scopeLabel(
  scope: RuntimeMemoryScope,
  workspaces: readonly MemoryWorkspace[],
  t: ReturnType<typeof useTranslation>["t"]
): string {
  if (scope.type === "global") return t("settings.memory.scope.global");
  return (
    workspaces.find((workspace) => workspace.id === scope.key)?.name ??
    t("settings.memory.scope.unknownWorkspace", { key: scope.key })
  );
}

function provenanceLabel(
  provenance: RuntimeMemoryProvenance,
  t: ReturnType<typeof useTranslation>["t"]
): string {
  if (provenance.sourceKind === "user_explicit") {
    return t("settings.memory.provenance.explicit");
  }
  return provenance.status === "available"
    ? t("settings.memory.provenance.sessionAvailable")
    : t("settings.memory.provenance.sessionUnavailable");
}

function formatDate(value: string, language: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(language === "zh-CN" ? "zh-CN" : "en", {
    dateStyle: "medium"
  }).format(date);
}

function SelectField({
  ariaLabel,
  value,
  disabled = false,
  onChange,
  children
}: {
  ariaLabel: string;
  value: string;
  disabled?: boolean;
  onChange(value: string): void;
  children: ReactNode;
}) {
  return (
    <label className="relative block min-w-0">
      <span className="sr-only">{ariaLabel}</span>
      <select
        aria-label={ariaLabel}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value)}
        className="h-8 w-full appearance-none rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] pl-3 pr-8 text-[11px] font-medium leading-4 text-[var(--text)] outline-none transition-colors hover:bg-[var(--panel-hover)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] disabled:cursor-not-allowed disabled:opacity-50"
      >
        {children}
      </select>
      <ChevronDown
        size={12}
        aria-hidden="true"
        className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-[var(--muted)]"
      />
    </label>
  );
}

function MemoryEditorDialog({
  state,
  runtime,
  workspaces,
  preferredWorkspaceId,
  onOpenChange,
  onCommitted
}: {
  state: EditorState | null;
  runtime: RuntimeClient | null;
  workspaces: readonly MemoryWorkspace[];
  preferredWorkspaceId: string | null;
  onOpenChange(open: boolean): void;
  onCommitted(): void;
}) {
  const { t } = useTranslation();
  const [record, setRecord] = useState<RuntimeMemoryRecord | null>(null);
  const [kind, setKind] = useState<RuntimeMemoryKind>("fact");
  const [scopeValue, setScopeValue] = useState<"global" | `workspace:${string}`>(
    preferredWorkspaceId ? `workspace:${preferredWorkspaceId}` : "global"
  );
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [errorKey, setErrorKey] = useState<TranslationKey | null>(null);
  const [conflicted, setConflicted] = useState(false);
  const mutationRequestRef = useRef<{ fingerprint: string; id: string } | null>(null);
  const open = state !== null;
  const isCreate = state?.mode === "create";
  const isView = state?.mode === "view";
  const trimmedContent = content.trim();
  const contentCharacters = Array.from(content).length;

  useEffect(() => {
    if (!state) return;
    setErrorKey(null);
    setConflicted(false);
    mutationRequestRef.current = null;
    setRecord(null);
    if (state.mode === "create") {
      setKind("fact");
      setScopeValue(preferredWorkspaceId ? `workspace:${preferredWorkspaceId}` : "global");
      setContent("");
      setLoading(false);
      return;
    }
    setContent("");
    if (!runtime || !state.summary) {
      setLoading(false);
      setErrorKey("settings.memory.error.detail");
      return;
    }

    let disposed = false;
    setLoading(true);
    void runtime
      .getMemory(state.summary.id)
      .then(({ memory }) => {
        if (disposed) return;
        if (state.mode === "correct" && memory.state !== "active") {
          setConflicted(true);
          setErrorKey("settings.memory.error.alreadyForgotten");
          onCommitted();
          return;
        }
        setRecord(memory);
        setContent(memory.content ?? "");
      })
      .catch(() => {
        if (!disposed) setErrorKey("settings.memory.error.detail");
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [preferredWorkspaceId, runtime, state]);

  const close = (nextOpen: boolean) => {
    if (!nextOpen && saving) return;
    onOpenChange(nextOpen);
  };

  const save = () => {
    if (!runtime || !state || !trimmedContent || saving || conflicted) return;
    setSaving(true);
    setErrorKey(null);
    const fingerprint = isCreate
      ? JSON.stringify(["create", kind, scopeValue, content])
      : JSON.stringify(["correct", record?.id, record?.revision, content]);
    if (mutationRequestRef.current?.fingerprint !== fingerprint) {
      mutationRequestRef.current = {
        fingerprint,
        id: requestId(isCreate ? "create" : "correct")
      };
    }
    const clientRequestId = mutationRequestRef.current.id;
    const operation = isCreate
      ? runtime.createMemory({
          kind,
          scope:
            scopeValue === "global"
              ? { type: "global", key: null }
              : { type: "workspace", key: scopeValue.slice("workspace:".length) },
          content,
          clientRequestId
        })
      : record
        ? runtime.correctMemory({
            memoryId: record.id,
            expectedRevision: record.revision,
            content,
            clientRequestId
          })
        : Promise.reject(new Error("Memory detail is unavailable."));

    void operation
      .then(() => {
        onOpenChange(false);
        onCommitted();
      })
      .catch((error: unknown) => {
        if (isRuntimeRpcError(error) && error.reasonCode === "memory_revision_conflict") {
          setConflicted(true);
          setErrorKey("settings.memory.error.revisionConflict");
          onCommitted();
          return;
        }
        if (isRuntimeRpcError(error) && error.reasonCode === "memory_forgotten") {
          setConflicted(true);
          setErrorKey("settings.memory.error.alreadyForgotten");
          onCommitted();
          return;
        }
        setErrorKey(
          isCreate ? "settings.memory.error.create" : "settings.memory.error.correct"
        );
      })
      .finally(() => setSaving(false));
  };

  const title = isCreate
    ? t("settings.memory.createTitle")
    : isView
      ? t("settings.memory.viewTitle")
      : t("settings.memory.correctTitle");

  return (
    <Dialog.Root open={open} onOpenChange={close}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[130] bg-black/55 backdrop-blur-[2px]" />
        <Dialog.Content className="glass-menu fixed left-1/2 top-1/2 z-[140] w-[min(520px,calc(100vw-28px))] -translate-x-1/2 -translate-y-1/2 rounded-[18px] p-5 shadow-[0_24px_70px_var(--shadow-color)]">
          <div className="flex items-start gap-3">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-[10px] bg-[var(--panel-raised)] text-[var(--muted-strong)]">
              <Brain size={15} aria-hidden="true" />
            </div>
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-[14px] font-semibold leading-5 text-[var(--text)]">
                {title}
              </Dialog.Title>
              <Dialog.Description className="mt-0.5 text-[11px] leading-4 text-[var(--muted)]">
                {isCreate
                  ? t("settings.memory.createDescription")
                  : isView
                    ? t("settings.memory.viewDescription")
                    : t("settings.memory.correctDescription")}
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <button
                type="button"
                aria-label={t("common.close")}
                className="flex size-7 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
              >
                <X size={13} aria-hidden="true" />
              </button>
            </Dialog.Close>
          </div>

          {loading ? (
            <div className="flex h-44 items-center justify-center text-[var(--muted)]">
              <LoaderCircle size={17} className="animate-spin" aria-label={t("common.loading")} />
            </div>
          ) : (
            <form
              className="mt-5 space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                if (!isView) save();
              }}
            >
              {isCreate ? (
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <span className="mb-1.5 block text-[11px] font-medium leading-4 text-[var(--text)]">
                      {t("settings.memory.kindLabel")}
                    </span>
                    <SelectField
                      ariaLabel={t("settings.memory.kindLabel")}
                      value={kind}
                      onChange={(value) => setKind(value as RuntimeMemoryKind)}
                    >
                      {MEMORY_KINDS.map((option) => (
                        <option key={option} value={option}>
                          {t(KIND_LABELS[option])}
                        </option>
                      ))}
                    </SelectField>
                  </div>
                  <div>
                    <span className="mb-1.5 block text-[11px] font-medium leading-4 text-[var(--text)]">
                      {t("settings.memory.scopeLabel")}
                    </span>
                    <SelectField
                      ariaLabel={t("settings.memory.scopeLabel")}
                      value={scopeValue}
                      onChange={(value) =>
                        setScopeValue(value as "global" | `workspace:${string}`)
                      }
                    >
                      <option value="global">{t("settings.memory.scope.global")}</option>
                      {workspaces.map((workspace) => (
                        <option key={workspace.id} value={`workspace:${workspace.id}`}>
                          {workspace.name}
                        </option>
                      ))}
                    </SelectField>
                  </div>
                </div>
              ) : record ? (
                <dl className="grid grid-cols-2 gap-x-4 gap-y-2 rounded-xl border border-[var(--border-soft)] bg-[var(--panel)] px-3 py-2.5 text-[10px] leading-4">
                  <div className="min-w-0">
                    <dt className="text-[var(--muted)]">{t("settings.memory.kindLabel")}</dt>
                    <dd className="truncate font-medium text-[var(--text)]">
                      {t(KIND_LABELS[record.kind])}
                    </dd>
                  </div>
                  <div className="min-w-0">
                    <dt className="text-[var(--muted)]">{t("settings.memory.scopeLabel")}</dt>
                    <dd className="truncate font-medium text-[var(--text)]">
                      {scopeLabel(record.scope, workspaces, t)}
                    </dd>
                  </div>
                  <div className="col-span-2 min-w-0">
                    <dt className="text-[var(--muted)]">{t("settings.memory.provenance")}</dt>
                    <dd className="truncate font-medium text-[var(--text)]">
                      {provenanceLabel(record.provenance, t)}
                    </dd>
                  </div>
                </dl>
              ) : null}

              <label className="block">
                <span className="mb-1.5 flex items-center justify-between gap-3 text-[11px] font-medium leading-4 text-[var(--text)]">
                  <span>{t("settings.memory.contentLabel")}</span>
                  {!isView ? (
                    <span className="font-normal text-[var(--muted)]">
                      {contentCharacters}/{CONTENT_LIMIT}
                    </span>
                  ) : null}
                </span>
                <textarea
                  autoFocus={!isView}
                  aria-label={t("settings.memory.contentLabel")}
                  readOnly={isView}
                  value={
                    isView && record?.state === "forgotten"
                      ? t("settings.memory.forgottenContent")
                      : content
                  }
                  onChange={(event) => {
                    const nextContent = event.currentTarget.value;
                    if (Array.from(nextContent).length <= CONTENT_LIMIT) {
                      setContent(nextContent);
                    }
                  }}
                  placeholder={isCreate ? t("settings.memory.contentPlaceholder") : undefined}
                  className="app-scrollbar min-h-36 w-full resize-y rounded-xl border border-[var(--border)] bg-[var(--panel)] px-3 py-2.5 text-[12px] leading-5 text-[var(--text)] outline-none placeholder:text-[var(--muted)] focus-visible:ring-1 focus-visible:ring-[var(--muted-strong)] read-only:resize-none read-only:text-[var(--muted-strong)]"
                />
              </label>

              <p aria-live="polite" className="min-h-4 text-[10px] leading-4 text-[#e08b8b]">
                {errorKey ? t(errorKey) : ""}
              </p>

              <div className="flex justify-end gap-2">
                <Dialog.Close asChild>
                  <button
                    type="button"
                    className="h-8 rounded-lg px-3 text-[11px] font-medium text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
                  >
                    {isView ? t("common.close") : t("common.cancel")}
                  </button>
                </Dialog.Close>
                {!isView ? (
                  <button
                    type="submit"
                    disabled={
                      !runtime ||
                      !trimmedContent ||
                      (!isCreate && !record) ||
                      saving ||
                      loading ||
                      conflicted
                    }
                    className="flex h-8 items-center gap-1.5 rounded-lg bg-[var(--text)] px-3 text-[11px] font-medium text-[var(--canvas)] transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    {saving ? <LoaderCircle size={12} className="animate-spin" aria-hidden="true" /> : null}
                    {isCreate ? t("settings.memory.create") : t("settings.memory.saveCorrection")}
                  </button>
                ) : null}
              </div>
            </form>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function ForgetDialog({
  target,
  runtime,
  onOpenChange,
  onCommitted
}: {
  target: RuntimeMemorySummary | null;
  runtime: RuntimeClient | null;
  onOpenChange(open: boolean): void;
  onCommitted(): void;
}) {
  const { t } = useTranslation();
  const [forgetting, setForgetting] = useState(false);
  const [loading, setLoading] = useState(false);
  const [record, setRecord] = useState<RuntimeMemoryRecord | null>(null);
  const [errorKey, setErrorKey] = useState<TranslationKey | null>(null);
  const requestIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (!target) {
      setRecord(null);
      setLoading(false);
      requestIdRef.current = null;
      return;
    }
    setErrorKey(null);
    setRecord(null);
    requestIdRef.current = requestId("forget");
    if (!runtime) {
      setErrorKey("settings.memory.error.detail");
      return;
    }
    let disposed = false;
    setLoading(true);
    void runtime
      .getMemory(target.id)
      .then(({ memory }) => {
        if (disposed) return;
        if (memory.state !== "active") {
          setErrorKey("settings.memory.error.alreadyForgotten");
          onCommitted();
          return;
        }
        setRecord(memory);
      })
      .catch(() => {
        if (!disposed) setErrorKey("settings.memory.error.detail");
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [runtime, target]);

  const forget = () => {
    if (!runtime || !record || !requestIdRef.current || forgetting) return;
    setForgetting(true);
    setErrorKey(null);
    void runtime
      .forgetMemory({
        memoryId: record.id,
        expectedRevision: record.revision,
        clientRequestId: requestIdRef.current
      })
      .then(() => {
        onOpenChange(false);
        onCommitted();
      })
      .catch((error: unknown) => {
        if (
          isRuntimeRpcError(error) &&
          (error.reasonCode === "memory_revision_conflict" ||
            error.reasonCode === "memory_forgotten")
        ) {
          setErrorKey("settings.memory.error.forgetConflict");
          setRecord(null);
          onCommitted();
          return;
        }
        setErrorKey("settings.memory.error.forget");
      })
      .finally(() => setForgetting(false));
  };

  return (
    <Dialog.Root
      open={target !== null}
      onOpenChange={(open) => {
        if (!open && forgetting) return;
        onOpenChange(open);
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[130] bg-black/55 backdrop-blur-[2px]" />
        <Dialog.Content className="glass-menu fixed left-1/2 top-1/2 z-[140] w-[min(420px,calc(100vw-28px))] -translate-x-1/2 -translate-y-1/2 rounded-[18px] p-5 shadow-[0_24px_70px_var(--shadow-color)]">
          <Dialog.Title className="text-[14px] font-semibold leading-5 text-[var(--text)]">
            {t("settings.memory.forgetTitle")}
          </Dialog.Title>
          <Dialog.Description className="mt-1.5 text-[11px] leading-4 text-[var(--muted)]">
            {t("settings.memory.forgetDescription")}
          </Dialog.Description>
          {target?.preview ? (
            <p className="mt-4 line-clamp-3 rounded-xl border border-[var(--border-soft)] bg-[var(--panel)] px-3 py-2 text-[11px] leading-[18px] text-[var(--muted-strong)]">
            {record?.content ?? target.preview}
            </p>
          ) : null}
          <p aria-live="polite" className="mt-2 min-h-4 text-[10px] leading-4 text-[#e08b8b]">
            {errorKey ? t(errorKey) : ""}
          </p>
          <div className="mt-3 flex justify-end gap-2">
            <Dialog.Close asChild>
              <button
                type="button"
                className="h-8 rounded-lg px-3 text-[11px] font-medium text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
              >
                {t("common.cancel")}
              </button>
            </Dialog.Close>
            <button
              type="button"
              disabled={!runtime || !record || loading || forgetting}
              onClick={forget}
              className="flex h-8 items-center gap-1.5 rounded-lg bg-[#c95353] px-3 text-[11px] font-medium text-white transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {forgetting ? <LoaderCircle size={12} className="animate-spin" aria-hidden="true" /> : null}
              {t("settings.memory.forget")}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function MemorySettings({
  workspaces,
  preferredWorkspaceId
}: MemorySettingsProps) {
  const { language, t } = useTranslation();
  const runtime = useMemo(() => createRuntimeClient(), []);
  const [memories, setMemories] = useState<RuntimeMemorySummary[]>([]);
  const [details, setDetails] = useState<Record<string, MemoryDetailState>>({});
  const [status, setStatus] = useState<LoadStatus>("loading");
  const [loadMoreStatus, setLoadMoreStatus] = useState<LoadMoreStatus>("idle");
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [scopeFilter, setScopeFilter] = useState<ScopeFilter>("all");
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [stateFilter, setStateFilter] = useState<RuntimeMemoryState>("active");
  const [reloadToken, setReloadToken] = useState(0);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [forgetTarget, setForgetTarget] = useState<RuntimeMemorySummary | null>(null);
  const requestGenerationRef = useRef(0);

  const loadDetails = (
    pageMemories: readonly RuntimeMemorySummary[],
    generation: number
  ) => {
    if (!runtime || pageMemories.length === 0) return;
    void Promise.all(
      pageMemories.map(async (summary) => {
        try {
          const { memory } = await runtime.getMemory(summary.id);
          return { id: summary.id, revision: summary.revision, record: memory };
        } catch {
          return { id: summary.id, revision: summary.revision, record: null };
        }
      })
    ).then((loaded) => {
      if (generation !== requestGenerationRef.current) return;
      setDetails((current) => {
        const next = { ...current };
        for (const detail of loaded) next[detail.id] = detail;
        return next;
      });
    });
  };

  const listParams = useMemo<RuntimeMemoryListParams>(() => {
    const scope = scopeFromFilter(scopeFilter);
    return {
      limit: PAGE_SIZE,
      ...(scope ? { scope } : {}),
      ...(kindFilter === "all" ? {} : { kind: kindFilter }),
      state: stateFilter
    };
  }, [kindFilter, scopeFilter, stateFilter]);

  useEffect(() => {
    const generation = ++requestGenerationRef.current;
    if (!runtime) {
      setMemories([]);
      setStatus("error");
      setHasMore(false);
      setNextCursor(null);
      return;
    }

    setStatus("loading");
    setLoadMoreStatus("idle");
    setDetails({});
    void runtime
      .listMemories(listParams)
      .then((page) => {
        if (generation !== requestGenerationRef.current) return;
        setMemories(page.memories);
        setNextCursor(page.nextCursor);
        setHasMore(page.hasMore);
        setStatus("ready");
        loadDetails(page.memories, generation);
      })
      .catch(() => {
        if (generation !== requestGenerationRef.current) return;
        setMemories([]);
        setNextCursor(null);
        setHasMore(false);
        setStatus("error");
      });
  }, [listParams, reloadToken, runtime]);

  const refresh = () => setReloadToken((value) => value + 1);

  const loadMore = () => {
    if (!runtime || !hasMore || !nextCursor || loadMoreStatus === "loading") return;
    const generation = requestGenerationRef.current;
    setLoadMoreStatus("loading");
    void runtime
      .listMemories({ ...listParams, cursor: nextCursor })
      .then((page) => {
        if (generation !== requestGenerationRef.current) return;
        setMemories((current) => {
          const seen = new Set(current.map((memory) => memory.id));
          return [...current, ...page.memories.filter((memory) => !seen.has(memory.id))];
        });
        setNextCursor(page.nextCursor);
        setHasMore(page.hasMore);
        setLoadMoreStatus("idle");
        loadDetails(page.memories, generation);
      })
      .catch(() => {
        if (generation === requestGenerationRef.current) setLoadMoreStatus("error");
      });
  };

  return (
    <div>
      <div className="flex items-start justify-between gap-6">
        <div>
          <h1
            data-settings-heading
            tabIndex={-1}
            className="text-[20px] font-semibold leading-7 tracking-[-0.025em] text-[var(--text)] outline-none"
          >
            {t("settings.memory.title")}
          </h1>
          <p className="mt-1.5 max-w-[590px] text-[11px] leading-4 text-[var(--muted)]">
            {t("settings.memory.description")}
          </p>
        </div>
        <button
          type="button"
          onClick={() => setEditor({ mode: "create", summary: null })}
          className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg bg-[var(--text)] px-3 text-[11px] font-medium text-[var(--canvas)] transition-opacity hover:opacity-90"
        >
          <Plus size={13} aria-hidden="true" />
          {t("settings.memory.create")}
        </button>
      </div>

      <section aria-labelledby="memory-list-heading" className="mt-9">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <h2
            id="memory-list-heading"
            className="text-[13px] font-semibold leading-5 text-[var(--text)]"
          >
            {t("settings.memory.saved")}
          </h2>
          <div className="grid w-full grid-cols-3 gap-2 sm:w-auto sm:min-w-[420px]">
            <SelectField
              ariaLabel={t("settings.memory.filter.scope")}
              value={scopeFilter}
              onChange={(value) => setScopeFilter(value as ScopeFilter)}
            >
              <option value="all">{t("settings.memory.filter.allScopes")}</option>
              <option value="global">{t("settings.memory.scope.global")}</option>
              {workspaces.map((workspace) => (
                <option key={workspace.id} value={`workspace:${workspace.id}`}>
                  {workspace.name}
                </option>
              ))}
            </SelectField>
            <SelectField
              ariaLabel={t("settings.memory.filter.kind")}
              value={kindFilter}
              onChange={(value) => setKindFilter(value as KindFilter)}
            >
              <option value="all">{t("settings.memory.filter.allKinds")}</option>
              {MEMORY_KINDS.map((kind) => (
                <option key={kind} value={kind}>
                  {t(KIND_LABELS[kind])}
                </option>
              ))}
            </SelectField>
            <SelectField
              ariaLabel={t("settings.memory.filter.state")}
              value={stateFilter}
              onChange={(value) => setStateFilter(value as RuntimeMemoryState)}
            >
              <option value="active">{t("settings.memory.state.active")}</option>
              <option value="forgotten">{t("settings.memory.state.forgotten")}</option>
            </SelectField>
          </div>
        </div>

        <div aria-busy={status === "loading"} className="mt-3">
          {status === "loading" ? (
            <div className="flex h-40 items-center justify-center rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)] text-[var(--muted)]">
              <LoaderCircle size={17} className="animate-spin" aria-label={t("common.loading")} />
            </div>
          ) : status === "error" ? (
            <div role="alert" className="flex h-40 flex-col items-center justify-center rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)] px-6 text-center">
              <CircleAlert size={20} aria-hidden="true" className="text-[#e07070]" />
              <p className="mt-2 text-[12px] font-medium leading-[18px] text-[var(--text)]">
                {t("settings.memory.error.list")}
              </p>
              <button
                type="button"
                onClick={refresh}
                className="mt-3 flex h-8 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 text-[11px] font-medium text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
              >
                <RefreshCw size={11} aria-hidden="true" />
                {t("common.retry")}
              </button>
            </div>
          ) : memories.length === 0 ? (
            <div className="flex h-40 flex-col items-center justify-center rounded-2xl border border-dashed border-[var(--border)] px-6 text-center">
              <Brain size={20} aria-hidden="true" className="text-[var(--muted)]" />
              <p className="mt-2 text-[12px] font-medium leading-[18px] text-[var(--text)]">
                {t("settings.memory.empty")}
              </p>
              <p className="mt-0.5 text-[11px] leading-4 text-[var(--muted)]">
                {t("settings.memory.emptyHint")}
              </p>
            </div>
          ) : (
            <div className="overflow-hidden rounded-2xl border border-[var(--border-soft)] bg-[var(--panel)]">
              {memories.map((memory) => {
                const detail = details[memory.id];
                return (
                <article
                  key={memory.id}
                  className="flex min-h-[76px] items-center gap-4 border-t border-[var(--separator)] px-4 py-3 first:border-t-0"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="shrink-0 rounded-md bg-[var(--panel-raised)] px-2 py-0.5 text-[10px] font-medium leading-4 text-[var(--muted-strong)]">
                        {t(KIND_LABELS[memory.kind])}
                      </span>
                      <span className="truncate text-[10px] leading-4 text-[var(--muted)]">
                        {scopeLabel(memory.scope, workspaces, t)} · {t("settings.memory.revision", { revision: memory.revision })} · {formatDate(memory.forgottenAt ?? memory.updatedAt, language)}
                      </span>
                    </div>
                    <p className="mt-1 line-clamp-2 text-[12px] leading-[18px] text-[var(--text)]">
                      {memory.preview ?? t("settings.memory.forgottenContent")}
                    </p>
                    <p className="mt-0.5 truncate text-[10px] leading-4 text-[var(--muted)]">
                      {detail?.revision === memory.revision
                        ? detail.record
                          ? provenanceLabel(detail.record.provenance, t)
                          : t("settings.memory.provenance.unverified")
                        : t("settings.memory.provenance.loading")}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <button
                      type="button"
                      aria-label={
                        memory.state === "active"
                          ? t("settings.memory.correct")
                          : t("settings.memory.view")
                      }
                      onClick={() =>
                        setEditor({
                          mode: memory.state === "active" ? "correct" : "view",
                          summary: memory
                        })
                      }
                      className="flex size-8 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)]"
                    >
                      {memory.state === "active" ? (
                        <Pencil size={13} aria-hidden="true" />
                      ) : (
                        <Eye size={13} aria-hidden="true" />
                      )}
                    </button>
                    {memory.state === "active" ? (
                      <button
                        type="button"
                        aria-label={t("settings.memory.forget")}
                        onClick={() => setForgetTarget(memory)}
                        className="flex size-8 items-center justify-center rounded-lg text-[var(--muted)] outline-none transition-colors hover:bg-[color-mix(in_srgb,#c95353_12%,transparent)] hover:text-[#d66a6a]"
                      >
                        <Trash2 size={13} aria-hidden="true" />
                      </button>
                    ) : null}
                  </div>
                </article>
                );
              })}
            </div>
          )}

          {status === "ready" && (hasMore || loadMoreStatus === "error") ? (
            <div className="mt-3 flex justify-center">
              <button
                type="button"
                disabled={loadMoreStatus === "loading"}
                onClick={loadMore}
                className="flex h-8 items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--panel-raised)] px-3 text-[11px] font-medium text-[var(--muted-strong)] transition-colors hover:bg-[var(--surface-hover)] hover:text-[var(--text)] disabled:cursor-wait disabled:opacity-50"
              >
                {loadMoreStatus === "loading" ? (
                  <LoaderCircle size={12} className="animate-spin" aria-hidden="true" />
                ) : loadMoreStatus === "error" ? (
                  <RefreshCw size={11} aria-hidden="true" />
                ) : null}
                {loadMoreStatus === "error"
                  ? t("common.retry")
                  : t("settings.memory.loadMore")}
              </button>
            </div>
          ) : null}
        </div>
      </section>

      <MemoryEditorDialog
        state={editor}
        runtime={runtime}
        workspaces={workspaces}
        preferredWorkspaceId={preferredWorkspaceId}
        onOpenChange={(open) => {
          if (!open) setEditor(null);
        }}
        onCommitted={refresh}
      />
      <ForgetDialog
        target={forgetTarget}
        runtime={runtime}
        onOpenChange={(open) => {
          if (!open) setForgetTarget(null);
        }}
        onCommitted={refresh}
      />
    </div>
  );
}
