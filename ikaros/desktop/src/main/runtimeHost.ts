import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { createInterface } from "node:readline";

import WebSocket, { type RawData } from "ws";

const PROTOCOL_VERSION = 1;

interface RuntimeReadyRecord {
  type: "ikaros_runtime.ready";
  protocolVersion: number;
  host: string;
  port: number;
  pid: number;
}

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: { code: number; message: string };
}

interface PendingRequest {
  resolve(value: unknown): void;
  reject(error: Error): void;
  timeout: ReturnType<typeof setTimeout>;
}

export interface RuntimeConnectionInfo {
  protocolVersion: number;
  host: string;
  port: number;
  pid: number;
  server: { name: string; version: string };
}

export interface RuntimeHostOptions {
  runtimeRoot: string;
  runtimeHome?: string;
  pythonExecutable?: string;
  parentPid?: number;
  startTimeoutMs?: number;
  stopTimeoutMs?: number;
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, message: string): Promise<T> {
  return new Promise<T>((resolvePromise, reject) => {
    const timeout = setTimeout(() => reject(new Error(message)), timeoutMs);
    promise.then(
      (value) => {
        clearTimeout(timeout);
        resolvePromise(value);
      },
      (error: unknown) => {
        clearTimeout(timeout);
        reject(error);
      }
    );
  });
}

function parseReadyRecord(line: string): RuntimeReadyRecord {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch (error) {
    throw new Error("Runtime readiness output was not valid JSON.", { cause: error });
  }
  if (
    typeof value !== "object" ||
    value === null ||
    (value as Partial<RuntimeReadyRecord>).type !== "ikaros_runtime.ready" ||
    typeof (value as Partial<RuntimeReadyRecord>).protocolVersion !== "number" ||
    typeof (value as Partial<RuntimeReadyRecord>).host !== "string" ||
    typeof (value as Partial<RuntimeReadyRecord>).port !== "number" ||
    typeof (value as Partial<RuntimeReadyRecord>).pid !== "number"
  ) {
    throw new Error("Runtime readiness output did not match the expected schema.");
  }
  return value as RuntimeReadyRecord;
}

function waitForReadiness(child: ChildProcessWithoutNullStreams): Promise<RuntimeReadyRecord> {
  return new Promise((resolvePromise, reject) => {
    const reader = createInterface({ input: child.stdout });
    const cleanup = (): void => {
      reader.removeAllListeners();
      child.off("error", onError);
      child.off("exit", onExit);
      reader.close();
    };
    const onError = (error: Error): void => {
      cleanup();
      reject(error);
    };
    const onExit = (code: number | null): void => {
      cleanup();
      reject(new Error(`Runtime exited before readiness (exit code ${String(code)}).`));
    };

    child.once("error", onError);
    child.once("exit", onExit);
    reader.once("line", (line) => {
      try {
        const ready = parseReadyRecord(line);
        cleanup();
        resolvePromise(ready);
      } catch (error) {
        cleanup();
        reject(error);
      }
    });
  });
}

function openAuthenticatedSocket(ready: RuntimeReadyRecord, token: string): Promise<WebSocket> {
  return new Promise((resolvePromise, reject) => {
    const socket = new WebSocket(`ws://${ready.host}:${ready.port}`, {
      headers: { Authorization: `Bearer ${token}` }
    });
    const cleanup = (): void => {
      socket.off("open", onOpen);
      socket.off("error", onError);
      socket.off("unexpected-response", onUnexpectedResponse);
    };
    const onOpen = (): void => {
      cleanup();
      resolvePromise(socket);
    };
    const onError = (error: Error): void => {
      cleanup();
      reject(error);
    };
    const onUnexpectedResponse = (): void => {
      cleanup();
      reject(new Error("Runtime rejected the authenticated WebSocket connection."));
    };

    socket.once("open", onOpen);
    socket.once("error", onError);
    socket.once("unexpected-response", onUnexpectedResponse);
  });
}

class JsonRpcConnection {
  private readonly pending = new Map<number, PendingRequest>();
  private nextRequestId = 1;

  constructor(private readonly socket: WebSocket) {
    socket.on("message", this.handleMessage);
    socket.on("error", this.handleError);
    socket.on("close", this.handleClose);
  }

  get isOpen(): boolean {
    return this.socket.readyState === WebSocket.OPEN;
  }

  request<TResult>(
    method: string,
    params: Record<string, unknown>,
    timeoutMs: number
  ): Promise<TResult> {
    if (!this.isOpen) {
      return Promise.reject(new Error("Runtime WebSocket is not open."));
    }
    const id = this.nextRequestId++;
    return new Promise<TResult>((resolvePromise, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Runtime request ${method} timed out.`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (value) => resolvePromise(value as TResult),
        reject,
        timeout
      });
      this.socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }), (error) => {
        if (!error) {
          return;
        }
        const pending = this.pending.get(id);
        if (pending) {
          clearTimeout(pending.timeout);
          this.pending.delete(id);
          pending.reject(error);
        }
      });
    });
  }

  close(): void {
    this.rejectAll(new Error("Runtime connection closed."));
    this.socket.close();
  }

  private readonly handleMessage = (raw: RawData): void => {
    let response: JsonRpcResponse;
    try {
      response = JSON.parse(raw.toString()) as JsonRpcResponse;
    } catch {
      this.rejectAll(new Error("Runtime returned invalid JSON."));
      return;
    }
    if (response.jsonrpc !== "2.0" || typeof response.id !== "number") {
      return;
    }
    const pending = this.pending.get(response.id);
    if (!pending) {
      return;
    }
    clearTimeout(pending.timeout);
    this.pending.delete(response.id);
    if (response.error) {
      pending.reject(
        new Error(`Runtime RPC failed (${response.error.code}): ${response.error.message}`)
      );
    } else {
      pending.resolve(response.result);
    }
  };

  private readonly handleError = (error: Error): void => {
    this.rejectAll(error);
  };

  private readonly handleClose = (): void => {
    this.rejectAll(new Error("Runtime WebSocket closed."));
  };

  private rejectAll(error: Error): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(error);
    }
    this.pending.clear();
  }
}

function runtimePython(runtimeRoot: string): string {
  const relative = process.platform === "win32" ? [".venv", "Scripts", "python.exe"] : [
    ".venv",
    "bin",
    "python"
  ];
  return resolve(runtimeRoot, ...relative);
}

export function developmentRuntimeRoot(desktopAppPath: string): string {
  return resolve(desktopAppPath, "..", "..", "runtime");
}

export class RuntimeHost {
  private readonly options: Required<
    Pick<RuntimeHostOptions, "parentPid" | "startTimeoutMs" | "stopTimeoutMs">
  > &
    Omit<RuntimeHostOptions, "parentPid" | "startTimeoutMs" | "stopTimeoutMs">;
  private child: ChildProcessWithoutNullStreams | undefined;
  private connection: JsonRpcConnection | undefined;
  private connectionInfo: RuntimeConnectionInfo | undefined;
  private starting: Promise<RuntimeConnectionInfo> | undefined;

  constructor(options: RuntimeHostOptions) {
    this.options = {
      ...options,
      parentPid: options.parentPid ?? process.pid,
      startTimeoutMs: options.startTimeoutMs ?? 15_000,
      stopTimeoutMs: options.stopTimeoutMs ?? 5_000
    };
  }

  get isRunning(): boolean {
    return this.connectionInfo !== undefined && this.child?.exitCode === null;
  }

  get pid(): number | undefined {
    return this.child?.pid;
  }

  start(): Promise<RuntimeConnectionInfo> {
    if (this.connectionInfo) {
      return Promise.resolve(this.connectionInfo);
    }
    if (!this.starting) {
      this.starting = this.startOnce().finally(() => {
        this.starting = undefined;
      });
    }
    return this.starting;
  }

  async request<TResult>(
    method: string,
    params: Record<string, unknown> = {}
  ): Promise<TResult> {
    await this.start();
    if (!this.connection) {
      throw new Error("Runtime connection is unavailable.");
    }
    return this.connection.request<TResult>(method, params, this.options.startTimeoutMs);
  }

  private async startOnce(): Promise<RuntimeConnectionInfo> {
    const pythonExecutable =
      this.options.pythonExecutable ??
      process.env.IKAROS_RUNTIME_PYTHON ??
      runtimePython(this.options.runtimeRoot);
    if (!existsSync(pythonExecutable)) {
      throw new Error(
        `Ikaros Runtime Python was not found at ${pythonExecutable}. Run uv sync --project runtime --locked.`
      );
    }

    const token = randomBytes(32).toString("base64url");
    const childEnvironment: NodeJS.ProcessEnv = {
      ...process.env,
      PYTHONUNBUFFERED: "1",
      PYTHONUTF8: "1"
    };
    if (this.options.runtimeHome) {
      childEnvironment.IKAROS_HOME = this.options.runtimeHome;
    }
    const child = spawn(
      pythonExecutable,
      [
        "-m",
        "ikaros_runtime",
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        "--token",
        token,
        "--parent-pid",
        String(this.options.parentPid)
      ],
      {
        cwd: this.options.runtimeRoot,
        env: childEnvironment,
        stdio: ["pipe", "pipe", "pipe"],
        windowsHide: true
      }
    );
    this.child = child;
    child.stderr.on("data", (chunk: Buffer) => {
      process.stderr.write(`[ikaros-runtime] ${chunk.toString()}`);
    });

    try {
      const ready = await withTimeout(
        waitForReadiness(child),
        this.options.startTimeoutMs,
        "Timed out waiting for Ikaros Runtime readiness."
      );
      if (ready.protocolVersion !== PROTOCOL_VERSION) {
        throw new Error(
          `Runtime readiness protocol ${ready.protocolVersion} is incompatible with Desktop protocol ${PROTOCOL_VERSION}.`
        );
      }
      const socket = await withTimeout(
        openAuthenticatedSocket(ready, token),
        this.options.startTimeoutMs,
        "Timed out connecting to Ikaros Runtime."
      );
      const connection = new JsonRpcConnection(socket);
      this.connection = connection;
      const result = await connection.request<{
        protocolVersion?: number;
        server?: { name?: string; version?: string };
      }>(
        "initialize",
        {
          protocolVersion: PROTOCOL_VERSION,
          client: { name: "ikaros-desktop", version: "0.1.0" }
        },
        this.options.startTimeoutMs
      );
      if (
        result.protocolVersion !== PROTOCOL_VERSION ||
        typeof result.server?.name !== "string" ||
        typeof result.server.version !== "string"
      ) {
        throw new Error("Runtime initialization result did not match the expected schema.");
      }
      this.connectionInfo = {
        protocolVersion: result.protocolVersion,
        host: ready.host,
        port: ready.port,
        pid: ready.pid,
        server: { name: result.server.name, version: result.server.version }
      };
      return this.connectionInfo;
    } catch (error) {
      child.kill();
      this.child = undefined;
      this.connection?.close();
      this.connection = undefined;
      throw error;
    }
  }

  async stop(): Promise<void> {
    const child = this.child;
    const connection = this.connection;
    this.connectionInfo = undefined;
    this.connection = undefined;
    this.child = undefined;
    if (!child) {
      return;
    }

    if (connection?.isOpen) {
      try {
        await connection.request("runtime.shutdown", {}, this.options.stopTimeoutMs);
      } catch {
        child.kill();
      } finally {
        connection.close();
      }
    } else {
      child.kill();
    }

    if (child.exitCode === null) {
      try {
        await withTimeout(
          new Promise<void>((resolvePromise) => child.once("exit", () => resolvePromise())),
          this.options.stopTimeoutMs,
          "Timed out waiting for Runtime process exit."
        );
      } catch {
        child.kill();
      }
    }
  }
}
