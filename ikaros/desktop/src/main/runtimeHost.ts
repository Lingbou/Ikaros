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

export interface RuntimeConnectionInfo {
  protocolVersion: number;
  host: string;
  port: number;
  pid: number;
  server: { name: string; version: string };
}

export interface RuntimeHostOptions {
  runtimeRoot: string;
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

function requestOnce(
  socket: WebSocket,
  id: number,
  method: string,
  params: Record<string, unknown>
): Promise<JsonRpcResponse> {
  return new Promise((resolvePromise, reject) => {
    const cleanup = (): void => {
      socket.off("message", onMessage);
      socket.off("error", onError);
      socket.off("close", onClose);
    };
    const onMessage = (raw: RawData): void => {
      try {
        const response = JSON.parse(raw.toString()) as JsonRpcResponse;
        if (response.jsonrpc !== "2.0" || response.id !== id) {
          throw new Error("Runtime returned a mismatched JSON-RPC response.");
        }
        cleanup();
        resolvePromise(response);
      } catch (error) {
        cleanup();
        reject(error);
      }
    };
    const onError = (error: Error): void => {
      cleanup();
      reject(error);
    };
    const onClose = (): void => {
      cleanup();
      reject(new Error("Runtime WebSocket closed before the request completed."));
    };

    socket.once("message", onMessage);
    socket.once("error", onError);
    socket.once("close", onClose);
    socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }), (error) => {
      if (error) {
        cleanup();
        reject(error);
      }
    });
  });
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
  private socket: WebSocket | undefined;
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
        env: { ...process.env, PYTHONUNBUFFERED: "1", PYTHONUTF8: "1" },
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
      this.socket = socket;
      const initialized = await withTimeout(
        requestOnce(socket, 1, "initialize", {
          protocolVersion: PROTOCOL_VERSION,
          client: { name: "ikaros-desktop", version: "0.1.0" }
        }),
        this.options.startTimeoutMs,
        "Timed out initializing Ikaros Runtime."
      );
      if (initialized.error) {
        throw new Error(`Runtime initialization failed: ${initialized.error.message}`);
      }
      const result = initialized.result as {
        protocolVersion?: number;
        server?: { name?: string; version?: string };
      };
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
      this.socket = undefined;
      throw error;
    }
  }

  async stop(): Promise<void> {
    const child = this.child;
    const socket = this.socket;
    this.connectionInfo = undefined;
    this.socket = undefined;
    this.child = undefined;
    if (!child) {
      return;
    }

    if (socket?.readyState === WebSocket.OPEN) {
      try {
        await withTimeout(
          requestOnce(socket, 2, "runtime.shutdown", {}),
          this.options.stopTimeoutMs,
          "Timed out requesting Runtime shutdown."
        );
      } catch {
        child.kill();
      } finally {
        socket.close();
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
