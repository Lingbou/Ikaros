import { homedir } from "node:os";
import { resolve } from "node:path";

/**
 * Resolves the Runtime home that owns config.yaml, state.db, memory.db and
 * runtime.lock. Mirrors the Runtime default (`IKAROS_HOME`, else `~/.ikaros`).
 */
export function resolveRuntimeHome(env: NodeJS.ProcessEnv = process.env): string {
  const configured = env.IKAROS_HOME?.trim();
  return configured ? resolve(configured) : resolve(homedir(), ".ikaros");
}

/** Interpreter of the checkout-local virtual environment used by development runs. */
export function runtimePython(runtimeRoot: string): string {
  const relative =
    process.platform === "win32"
      ? [".venv", "Scripts", "python.exe"]
      : [".venv", "bin", "python"];
  return resolve(runtimeRoot, ...relative);
}

/** Runtime source root of a checkout, resolved relative to the desktop app path. */
export function developmentRuntimeRoot(desktopAppPath: string): string {
  return resolve(desktopAppPath, "..", "..", "runtime");
}