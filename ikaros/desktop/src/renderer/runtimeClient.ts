import {
  RUNTIME_MEMORY_ERROR_REASON_CODES_BY_METHOD,
  RUNTIME_PROTOCOL_MANIFEST
} from "../shared/runtime";
import type {
  RuntimeFileChangeGetParams,
  RuntimeFileChangeResult,
  RuntimeFilePreviewParams,
  RuntimeFilePreviewResult,
  IkarosRuntimeApi,
  IkarosRuntimeBridgeApi,
  RuntimeCancelRunResult,
  RuntimeSteerRunParams,
  RuntimeSteerRunResult,
  RuntimeInvocationResult,
  RuntimeJournalEvent,
  RuntimeHostStatus,
  RuntimeMemoryCorrectParams,
  RuntimeMemoryCreateParams,
  RuntimeMemoryCreateResult,
  RuntimeMemoryErrorReasonCode,
  RuntimeMemoryForgetParams,
  RuntimeMemoryGetResult,
  RuntimeMemoryListPage,
  RuntimeMemoryListParams,
  RuntimeMemoryMutationResult,
  RuntimeMemoryRpcMethod,
  RuntimeModelSetEnabledParams,
  RuntimeModelSetLimitsParams,
  RuntimeModelSetEnabledResult,
  RuntimeModelSummary,
  RuntimeProviderConfigureParams,
  RuntimeProviderConfigureResult,
  RuntimeProviderDiscoverModelsParams,
  RuntimeProviderDiscoverModelsResult,
  RuntimeProviderRemoveResult,
  RuntimeProviderSummary,
  RuntimeProcessReadParams,
  RuntimeProcessReadResult,
  RuntimeProcessStopParams,
  RuntimeProcessStopResult,
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
    message: string,
    readonly reasonCode?: RuntimeMemoryErrorReasonCode
  ) {
    super(message);
    this.name = "RuntimeRpcError";
  }
}

const MEMORY_ERROR_REASON_CODES = new Set<RuntimeMemoryErrorReasonCode>([
  ...RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.reasonCodes
]);

function isMemoryErrorReasonCode(value: unknown): value is RuntimeMemoryErrorReasonCode {
  return (
    typeof value === "string" &&
    MEMORY_ERROR_REASON_CODES.has(value as RuntimeMemoryErrorReasonCode)
  );
}

export function isRuntimeRpcError(error: unknown): error is RuntimeRpcError {
  if (typeof error !== "object" || error === null) return false;
  const candidate = error as Partial<RuntimeRpcError>;
  return (
    candidate.kind === "json_rpc" &&
    typeof candidate.code === "number" &&
    Number.isInteger(candidate.code) &&
    typeof candidate.message === "string" &&
    (candidate.code === RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.code
      ? candidate.message === RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.message &&
        isMemoryErrorReasonCode(candidate.reasonCode)
      : candidate.reasonCode === undefined)
  );
}

function hasOwn(value: object, property: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(value, property);
}

function invalidBridgeResult(): Error {
  return new Error("Runtime bridge returned an invalid invocation result.");
}

async function unwrapRuntimeInvocation<TResult>(
  invocation: Promise<RuntimeInvocationResult<TResult>>,
  memoryMethod?: RuntimeMemoryRpcMethod
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
    error?: {
      kind?: unknown;
      code?: unknown;
      message?: unknown;
      reasonCode?: unknown;
    };
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
      typeof candidate.error.message !== "string" ||
      Object.keys(candidate.error).some(
        (key) => !["kind", "code", "message", "reasonCode"].includes(key)
      ) ||
      (candidate.error.code === RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.code
        ? memoryMethod === undefined ||
          candidate.error.message !==
            RUNTIME_PROTOCOL_MANIFEST.errors.memoryOperation.message ||
          !isMemoryErrorReasonAllowed(memoryMethod, candidate.error.reasonCode)
        : candidate.error.reasonCode !== undefined)
    ) {
      throw invalidBridgeResult();
    }
    throw new RuntimeRpcError(
      candidate.error.code,
      candidate.error.message,
      candidate.error.reasonCode as RuntimeMemoryErrorReasonCode | undefined
    );
  }
  throw invalidBridgeResult();
}

function isMemoryErrorReasonAllowed(
  method: RuntimeMemoryRpcMethod,
  reasonCode: unknown
): reasonCode is RuntimeMemoryErrorReasonCode {
  return (
    typeof reasonCode === "string" &&
    (
      RUNTIME_MEMORY_ERROR_REASON_CODES_BY_METHOD[method] as readonly string[]
    ).includes(reasonCode)
  );
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

  readProcess(params: RuntimeProcessReadParams): Promise<RuntimeProcessReadResult> {
    if (!this.api.readProcess) return Promise.reject(new Error("Process control is unavailable"));
    return unwrapRuntimeInvocation(this.api.readProcess(params));
  }

  stopProcess(params: RuntimeProcessStopParams): Promise<RuntimeProcessStopResult> {
    if (!this.api.stopProcess) return Promise.reject(new Error("Process control is unavailable"));
    return unwrapRuntimeInvocation(this.api.stopProcess(params));
  }

  steerRun(params: RuntimeSteerRunParams): Promise<RuntimeSteerRunResult> {
    if (!this.api.steerRun) return Promise.reject(new Error("Runtime steering is unavailable."));
    return unwrapRuntimeInvocation(this.api.steerRun(params));
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

  setModelLimits(params: RuntimeModelSetLimitsParams): Promise<RuntimeModelSetEnabledResult> {
    return unwrapRuntimeInvocation(this.api.setModelLimits(params));
  }

  listSkills(): Promise<RuntimeSkillListResult> {
    return unwrapRuntimeInvocation(this.api.listSkills());
  }

  setSkillEnabled(
    params: RuntimeSkillSetEnabledParams
  ): Promise<RuntimeSkillSetEnabledResult> {
    return unwrapRuntimeInvocation(this.api.setSkillEnabled(params));
  }

  createMemory(params: RuntimeMemoryCreateParams): Promise<RuntimeMemoryCreateResult> {
    return unwrapRuntimeInvocation(this.api.createMemory(params), "memory.create");
  }

  correctMemory(params: RuntimeMemoryCorrectParams): Promise<RuntimeMemoryMutationResult> {
    return unwrapRuntimeInvocation(this.api.correctMemory(params), "memory.correct");
  }

  forgetMemory(params: RuntimeMemoryForgetParams): Promise<RuntimeMemoryMutationResult> {
    return unwrapRuntimeInvocation(this.api.forgetMemory(params), "memory.forget");
  }

  listMemories(params: RuntimeMemoryListParams = {}): Promise<RuntimeMemoryListPage> {
    return unwrapRuntimeInvocation(this.api.listMemories(params), "memory.list");
  }

  getMemory(memoryId: string): Promise<RuntimeMemoryGetResult> {
    return unwrapRuntimeInvocation(this.api.getMemory(memoryId), "memory.get");
  }

  readUsage(): Promise<RuntimeUsageReadResult> {
    return unwrapRuntimeInvocation(this.api.readUsage());
  }

  previewFile(params: RuntimeFilePreviewParams): Promise<RuntimeFilePreviewResult> {
    return unwrapRuntimeInvocation(this.api.previewFile(params));
  }

  getFileChange(params: RuntimeFileChangeGetParams): Promise<RuntimeFileChangeResult> {
    return unwrapRuntimeInvocation(this.api.getFileChange(params));
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
