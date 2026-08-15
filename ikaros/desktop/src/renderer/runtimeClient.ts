import type {
  IkarosRuntimeApi,
  IkarosRuntimeBridgeApi,
  RuntimeCancelRunResult,
  RuntimeInvocationResult,
  RuntimeJournalEvent,
  RuntimeHostStatus,
  RuntimeModelSetEnabledParams,
  RuntimeModelSetEnabledResult,
  RuntimeModelSummary,
  RuntimeProviderConfigureParams,
  RuntimeProviderConfigureResult,
  RuntimeProviderDiscoverModelsParams,
  RuntimeProviderDiscoverModelsResult,
  RuntimeProviderRemoveResult,
  RuntimeProviderSummary,
  RuntimeReplayResult,
  RuntimeSkillListResult,
  RuntimeSkillSetEnabledParams,
  RuntimeSkillSetEnabledResult,
  RuntimeThreadCreateParams,
  RuntimeThreadCreateResult,
  RuntimeThreadCatalogParams,
  RuntimeThreadGetResult,
  RuntimeThreadListPage,
  RuntimeThreadMutationResult,
  RuntimeThreadRenameParams,
  RuntimeTurnListPage,
  RuntimeTurnListParams,
  RuntimeTurnStartParams,
  RuntimeTurnStartResult,
  RuntimeUsageReadResult
} from "../shared/runtime";

export class RuntimeRpcError extends Error {
  readonly kind = "json_rpc" as const;

  constructor(
    readonly code: number,
    message: string
  ) {
    super(message);
    this.name = "RuntimeRpcError";
  }
}

export function isRuntimeRpcError(error: unknown): error is RuntimeRpcError {
  if (typeof error !== "object" || error === null) return false;
  const candidate = error as Partial<RuntimeRpcError>;
  return (
    candidate.kind === "json_rpc" &&
    typeof candidate.code === "number" &&
    Number.isInteger(candidate.code) &&
    typeof candidate.message === "string"
  );
}

function hasOwn(value: object, property: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(value, property);
}

function invalidBridgeResult(): Error {
  return new Error("Runtime bridge returned an invalid invocation result.");
}

async function unwrapRuntimeInvocation<TResult>(
  invocation: Promise<RuntimeInvocationResult<TResult>>
): Promise<TResult> {
  const result = await invocation;
  if (
    typeof result !== "object" ||
    result === null ||
    !hasOwn(result, "ok")
  ) {
    throw invalidBridgeResult();
  }

  const candidate = result as {
    ok?: unknown;
    value?: unknown;
    error?: { kind?: unknown; code?: unknown; message?: unknown };
  };
  if (candidate.ok === true) {
    if (!hasOwn(candidate, "value") || hasOwn(candidate, "error")) {
      throw invalidBridgeResult();
    }
    return candidate.value as TResult;
  }
  if (candidate.ok === false) {
    if (
      hasOwn(candidate, "value") ||
      !hasOwn(candidate, "error") ||
      typeof candidate.error !== "object" ||
      candidate.error === null ||
      candidate.error.kind !== "json_rpc" ||
      typeof candidate.error.code !== "number" ||
      !Number.isInteger(candidate.error.code) ||
      typeof candidate.error.message !== "string"
    ) {
      throw invalidBridgeResult();
    }
    throw new RuntimeRpcError(candidate.error.code, candidate.error.message);
  }
  throw invalidBridgeResult();
}

export class RuntimeClient implements IkarosRuntimeApi {
  constructor(private readonly api: IkarosRuntimeBridgeApi) {}

  listThreads(
    params: RuntimeThreadCatalogParams = {}
  ): Promise<RuntimeThreadListPage> {
    return unwrapRuntimeInvocation(this.api.listThreads(params));
  }

  getThread(threadId: string): Promise<RuntimeThreadGetResult> {
    return unwrapRuntimeInvocation(this.api.getThread(threadId));
  }

  listTurns(params: RuntimeTurnListParams): Promise<RuntimeTurnListPage> {
    return unwrapRuntimeInvocation(this.api.listTurns(params));
  }

  createThread(params: RuntimeThreadCreateParams): Promise<RuntimeThreadCreateResult> {
    return unwrapRuntimeInvocation(this.api.createThread(params));
  }

  renameThread(params: RuntimeThreadRenameParams): Promise<RuntimeThreadMutationResult> {
    return unwrapRuntimeInvocation(this.api.renameThread(params));
  }

  archiveThread(threadId: string): Promise<RuntimeThreadMutationResult> {
    return unwrapRuntimeInvocation(this.api.archiveThread(threadId));
  }

  unarchiveThread(threadId: string): Promise<RuntimeThreadMutationResult> {
    return unwrapRuntimeInvocation(this.api.unarchiveThread(threadId));
  }

  startTurn(params: RuntimeTurnStartParams): Promise<RuntimeTurnStartResult> {
    return unwrapRuntimeInvocation(this.api.startTurn(params));
  }

  cancelRun(runId: string): Promise<RuntimeCancelRunResult> {
    return unwrapRuntimeInvocation(this.api.cancelRun(runId));
  }

  replayEvents(afterSeq: number, limit = 500): Promise<RuntimeReplayResult> {
    return unwrapRuntimeInvocation(this.api.replayEvents(afterSeq, limit));
  }

  listProviders(): Promise<{ providers: RuntimeProviderSummary[] }> {
    return unwrapRuntimeInvocation(this.api.listProviders());
  }

  configureProvider(
    params: RuntimeProviderConfigureParams
  ): Promise<RuntimeProviderConfigureResult> {
    return unwrapRuntimeInvocation(this.api.configureProvider(params));
  }

  discoverProviderModels(
    params: RuntimeProviderDiscoverModelsParams
  ): Promise<RuntimeProviderDiscoverModelsResult> {
    return unwrapRuntimeInvocation(this.api.discoverProviderModels(params));
  }

  disconnectProvider(providerId: "deepseek"): Promise<RuntimeProviderConfigureResult> {
    return unwrapRuntimeInvocation(this.api.disconnectProvider(providerId));
  }

  removeProvider(providerId: string): Promise<RuntimeProviderRemoveResult> {
    return unwrapRuntimeInvocation(this.api.removeProvider(providerId));
  }

  listModels(): Promise<{ models: RuntimeModelSummary[] }> {
    return unwrapRuntimeInvocation(this.api.listModels());
  }

  setModelEnabled(
    params: RuntimeModelSetEnabledParams
  ): Promise<RuntimeModelSetEnabledResult> {
    return unwrapRuntimeInvocation(this.api.setModelEnabled(params));
  }

  listSkills(): Promise<RuntimeSkillListResult> {
    return unwrapRuntimeInvocation(this.api.listSkills());
  }

  setSkillEnabled(
    params: RuntimeSkillSetEnabledParams
  ): Promise<RuntimeSkillSetEnabledResult> {
    return unwrapRuntimeInvocation(this.api.setSkillEnabled(params));
  }

  readUsage(): Promise<RuntimeUsageReadResult> {
    return unwrapRuntimeInvocation(this.api.readUsage());
  }

  onEvent(listener: (event: RuntimeJournalEvent) => void): () => void {
    return this.api.onEvent(listener);
  }

  onStatus(listener: (status: RuntimeHostStatus) => void): () => void {
    return this.api.onStatus?.(listener) ?? (() => undefined);
  }
}

export function createRuntimeClient(): RuntimeClient | null {
  if (typeof window === "undefined") {
    return null;
  }
  const desktop = (
    window as Window & { ikarosDesktop?: { runtime?: IkarosRuntimeBridgeApi } }
  ).ikarosDesktop;
  return desktop?.runtime ? new RuntimeClient(desktop.runtime) : null;
}
