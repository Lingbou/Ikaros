import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

import { developmentRuntimeRoot, runtimePython } from "./runtimePaths";

const RUNTIME_DIRECTORY = "runtime";
const RUNTIME_INSTALL_MANIFEST_FILE = "install.json";
const RUNTIME_INSTALL_SCHEMA_VERSION = 1;

/** Interpreter and working directory a Runtime launch should use. */
export interface RuntimeLaunch {
  runtimeRoot: string;
  pythonExecutable: string;
}

/**
 * Installation record written by the Runtime install script. It is the contract
 * between the desktop app and a Runtime installed into the Runtime home.
 */
interface RuntimeInstallManifest {
  schemaVersion: number;
  runtimeVersion: string;
  protocolVersion: number;
  runtimeRoot: string;
  pythonExecutable: string;
  installedAt: string;
}

interface RuntimeLaunchResolverOptions {
  /** `app.isPackaged`: packaged applications use the installed Runtime manifest. */
  isPackaged: boolean;
  /** `app.getAppPath()`, used to locate the checkout-local Runtime. */
  appPath: string;
  /** Runtime home (`~/.ikaros` unless `IKAROS_HOME` overrides it). */
  runtimeHome: string;
  /** Explicit interpreter override, for example from tests or `IKAROS_RUNTIME_PYTHON`. */
  pythonExecutable?: string;
  env?: NodeJS.ProcessEnv;
}

function runtimeInstallManifestPath(runtimeHome: string): string {
  return join(runtimeHome, RUNTIME_DIRECTORY, RUNTIME_INSTALL_MANIFEST_FILE);
}

function requireString(value: unknown, field: string, source: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${source} is missing a valid "${field}" value.`);
  }
  return value;
}

function requireNumber(value: unknown, field: string, source: string): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new Error(`${source} is missing a valid "${field}" value.`);
  }
  return value;
}

async function readRuntimeInstallManifest(
  runtimeHome: string
): Promise<RuntimeInstallManifest | undefined> {
  const path = runtimeInstallManifestPath(runtimeHome);
  let raw: string;
  try {
    raw = await readFile(path, "utf8");
  } catch {
    return undefined;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error(`${path} is not valid JSON.`);
  }
  if (typeof parsed !== "object" || parsed === null) {
    throw new Error(`${path} is not a Runtime install manifest.`);
  }
  const record = parsed as Record<string, unknown>;
  const schemaVersion = requireNumber(record.schemaVersion, "schemaVersion", path);
  if (schemaVersion !== RUNTIME_INSTALL_SCHEMA_VERSION) {
    throw new Error(
      `${path} uses install schema ${schemaVersion}, but this Desktop understands schema ${RUNTIME_INSTALL_SCHEMA_VERSION}.`
    );
  }
  return {
    schemaVersion,
    runtimeVersion: requireString(record.runtimeVersion, "runtimeVersion", path),
    protocolVersion: requireNumber(record.protocolVersion, "protocolVersion", path),
    runtimeRoot: requireString(record.runtimeRoot, "runtimeRoot", path),
    pythonExecutable: requireString(record.pythonExecutable, "pythonExecutable", path),
    installedAt: requireString(record.installedAt, "installedAt", path)
  };
}

/**
 * Resolves how to launch the Runtime for the current installation:
 * an explicit interpreter override wins, development runs use the checkout
 * virtual environment, and packaged runs use the manifest written by the
 * standalone Runtime installer.
 */
export function createRuntimeLaunchResolver(
  options: RuntimeLaunchResolverOptions
): () => Promise<RuntimeLaunch> {
  return async (): Promise<RuntimeLaunch> => {
    const override = options.pythonExecutable ?? options.env?.IKAROS_RUNTIME_PYTHON?.trim();
    const developmentRoot = developmentRuntimeRoot(options.appPath);
    if (override !== undefined && override.length > 0) {
      return { runtimeRoot: developmentRoot, pythonExecutable: override };
    }
    if (!options.isPackaged) {
      return { runtimeRoot: developmentRoot, pythonExecutable: runtimePython(developmentRoot) };
    }
    const installed = await readRuntimeInstallManifest(options.runtimeHome);
    if (installed === undefined) {
      throw new Error(
        `No Ikaros Runtime installation was found at ${runtimeInstallManifestPath(options.runtimeHome)}. Install it from Ikaros, run the Runtime install script, or set IKAROS_RUNTIME_PYTHON to a Python 3.12/3.13 interpreter.`
      );
    }
    if (!existsSync(installed.pythonExecutable)) {
      throw new Error(
        `The installed Ikaros Runtime interpreter is missing at ${installed.pythonExecutable}. Re-run the Runtime install script.`
      );
    }
    return {
      runtimeRoot: installed.runtimeRoot,
      pythonExecutable: installed.pythonExecutable
    };
  };
}
