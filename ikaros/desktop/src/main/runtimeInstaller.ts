import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

import { resolveRuntimeHome } from "./runtimePaths";

/** Result of running the standalone Runtime installer. */
export interface RuntimeInstallOutcome {
  ok: boolean;
  output: string;
}

export interface RuntimeInstallOptions {
  /** Absolute path to the installer script; omitted means "not installed". */
  scriptPath?: string;
  /** Directory that receives runtime/<version> and install.json. */
  runtimeHome?: string;
  /** Timeout for the whole installation. */
  timeoutMs?: number;
}

const INSTALL_TIMEOUT_MS = 10 * 60 * 1000;
const MAX_OUTPUT_CHARS = 200_000;

/** Where the installer ships: the checkout in development, resources when packaged. */
export function defaultInstallScriptPath(options: {
  isPackaged: boolean;
  appPath: string;
  resourcesPath: string;
}): string {
  const name = process.platform === "win32" ? "install-runtime.ps1" : "install-runtime.sh";
  const directory = options.isPackaged
    ? join(options.resourcesPath, "scripts")
    : join(options.appPath, "..", "..", "scripts");
  return join(directory, name);
}

/** Runs the installer once and returns its combined output. */
export async function installRuntime(
  options: RuntimeInstallOptions = {}
): Promise<RuntimeInstallOutcome> {
  const scriptPath = options.scriptPath;
  if (!scriptPath || !existsSync(scriptPath)) {
    return {
      ok: false,
      output:
        `Runtime 安装脚本不存在：${scriptPath ?? "(未定位)"}\n` +
        "开发态请确认仓库内存在 scripts/install-runtime.ps1（Windows）或 scripts/install-runtime.sh；" +
        "打包态请确认 resources/scripts 已随安装包发布。"
    };
  }
  const runtimeHome = options.runtimeHome ?? resolveRuntimeHome();
  const command = process.platform === "win32" ? "powershell.exe" : "bash";
  const args =
    process.platform === "win32"
      ? ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", scriptPath]
      : [scriptPath];
  const timeoutMs = options.timeoutMs ?? INSTALL_TIMEOUT_MS;

  return await new Promise<RuntimeInstallOutcome>((resolve) => {
    const child = spawn(command, args, {
      env: { ...process.env, IKAROS_HOME: runtimeHome },
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true
    });
    let output = "";
    let settled = false;
    const finish = (outcome: RuntimeInstallOutcome) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(outcome);
    };
    const append = (chunk: Buffer) => {
      output += chunk.toString("utf8");
      if (output.length > MAX_OUTPUT_CHARS) {
        output = output.slice(-MAX_OUTPUT_CHARS);
      }
    };
    const timer = setTimeout(() => {
      append(Buffer.from(`\n安装超时（${Math.round(timeoutMs / 1000)}s），已终止。\n`));
      child.kill();
    }, timeoutMs);
    child.stdout.on("data", append);
    child.stderr.on("data", append);
    child.once("error", (error) =>
      finish({ ok: false, output: `${output}\n${error.message}`.trim() })
    );
    child.once("close", (code) => finish({ ok: code === 0, output: output.trim() }));
  });
}
