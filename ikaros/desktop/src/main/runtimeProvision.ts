import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";

import { extract } from "tar";

import { developmentRuntimeRoot, runtimePython } from "./runtimePaths";

/** Manifest schema produced by scripts/prepare-runtime-payload.mjs. */
export const RUNTIME_PAYLOAD_SCHEMA_VERSION = 1;
/** Resources sub-directory that carries the Runtime payload archive and manifest. */
export const RUNTIME_PAYLOAD_RESOURCE_DIRECTORY = "runtime";
export const RUNTIME_PAYLOAD_MANIFEST_FILE = "runtime-payload.json";
/** Payload versions are deployed under <runtime home>/runtime/<payload version>. */
export const RUNTIME_PAYLOAD_DIRECTORY = "runtime";
/** Written last, inside the payload directory, once a deployment is complete. */
export const RUNTIME_READY_MARKER_FILE = ".ikaros-runtime.json";

export interface RuntimePayloadManifest {
  schemaVersion: number;
  payloadVersion: string;
  runtimeVersion: string;
  pythonVersion: string;
  pythonBuildRelease: string;
  platform: string;
  arch: string;
  archive: string;
  archiveSha256: string;
  pythonExecutable: string;
  unpackedBytes: number;
  unpackedFiles: number;
  builtAt: string;
}

/** Interpreter and working directory a Runtime launch should use. */
export interface RuntimeLaunch {
  runtimeRoot: string;
  pythonExecutable: string;
}

interface RuntimeReadyMarker {
  schemaVersion: number;
  payloadVersion: string;
  archiveSha256: string;
  pythonExecutable: string;
  deployedAt: string;
}

export interface RuntimeLaunchResolverOptions {
  /** `app.isPackaged`: only packaged applications use the bundled payload. */
  isPackaged: boolean;
  /** `app.getAppPath()`, used to locate the checkout-local Runtime. */
  appPath: string;
  /** `process.resourcesPath`, which carries the payload archive and manifest. */
  resourcesPath: string;
  /** Runtime home (`~/.ikaros` unless `IKAROS_HOME` overrides it). */
  runtimeHome: string;
  /** Explicit interpreter override, for example from tests or `IKAROS_RUNTIME_PYTHON`. */
  pythonExecutable?: string;
  env?: NodeJS.ProcessEnv;
  /** Called before a payload deployment that may take a few seconds. */
  onProgress?: (message: string) => void;
}

export interface RuntimePayloadDeploymentOptions {
  manifest: RuntimePayloadManifest;
  resourcesPath: string;
  runtimeHome: string;
  onProgress?: (message: string) => void;
}

function requireString(value: unknown, field: string, source: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${source} is missing a valid "${field}" value.`);
  }
  return value;
}

/** Payload paths come from the installation; keep them relative and well-formed. */
const SAFE_PATH_SEGMENT = /^[A-Za-z0-9][A-Za-z0-9._+-]*$/;

function requirePath(value: unknown, field: string, source: string): string {
  const path = requireString(value, field, source);
  if (path.includes("\\") || path.split("/").some((segment) => !SAFE_PATH_SEGMENT.test(segment))) {
    throw new Error(`${source} has an unsafe "${field}" value: ${path}`);
  }
  return path;
}

function requireNumber(value: unknown, field: string, source: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`${source} is missing a valid "${field}" value.`);
  }
  return value;
}

export function parseRuntimePayloadManifest(value: unknown, source: string): RuntimePayloadManifest {
  if (typeof value !== "object" || value === null) {
    throw new Error(`${source} is not a Runtime payload manifest.`);
  }
  const record = value as Record<string, unknown>;
  const schemaVersion = requireNumber(record.schemaVersion, "schemaVersion", source);
  if (schemaVersion !== RUNTIME_PAYLOAD_SCHEMA_VERSION) {
    throw new Error(
      `${source} uses Runtime payload schema ${schemaVersion}, but this Desktop understands schema ${RUNTIME_PAYLOAD_SCHEMA_VERSION}.`
    );
  }
  return {
    schemaVersion,
    payloadVersion: requirePath(record.payloadVersion, "payloadVersion", source),
    runtimeVersion: requireString(record.runtimeVersion, "runtimeVersion", source),
    pythonVersion: requireString(record.pythonVersion, "pythonVersion", source),
    pythonBuildRelease: requireString(record.pythonBuildRelease, "pythonBuildRelease", source),
    platform: requireString(record.platform, "platform", source),
    arch: requireString(record.arch, "arch", source),
    archive: requirePath(record.archive, "archive", source),
    archiveSha256: requireString(record.archiveSha256, "archiveSha256", source),
    pythonExecutable: requirePath(record.pythonExecutable, "pythonExecutable", source),
    unpackedBytes: requireNumber(record.unpackedBytes, "unpackedBytes", source),
    unpackedFiles: requireNumber(record.unpackedFiles, "unpackedFiles", source),
    builtAt: requireString(record.builtAt, "builtAt", source)
  };
}

export function runtimePayloadManifestPath(resourcesPath: string): string {
  return join(
    resourcesPath,
    RUNTIME_PAYLOAD_RESOURCE_DIRECTORY,
    RUNTIME_PAYLOAD_MANIFEST_FILE
  );
}

/** Reads the manifest shipped with the installation; undefined when it is absent. */
export async function readRuntimePayloadManifest(
  resourcesPath: string
): Promise<RuntimePayloadManifest | undefined> {
  const manifestPath = runtimePayloadManifestPath(resourcesPath);
  if (!existsSync(manifestPath)) {
    return undefined;
  }
  let value: unknown;
  try {
    value = JSON.parse(await readFile(manifestPath, "utf8"));
  } catch (error) {
    throw new Error(`${manifestPath} is not valid JSON.`, { cause: error });
  }
  return parseRuntimePayloadManifest(value, manifestPath);
}

function payloadPath(payloadRoot: string, relativePath: string): string {
  return join(payloadRoot, ...relativePath.split("/"));
}

async function readReadyMarker(payloadRoot: string): Promise<RuntimeReadyMarker | undefined> {
  try {
    const value = JSON.parse(
      await readFile(join(payloadRoot, RUNTIME_READY_MARKER_FILE), "utf8")
    ) as Partial<RuntimeReadyMarker>;
    if (
      value.schemaVersion !== RUNTIME_PAYLOAD_SCHEMA_VERSION ||
      typeof value.payloadVersion !== "string" ||
      typeof value.archiveSha256 !== "string" ||
      typeof value.pythonExecutable !== "string"
    ) {
      return undefined;
    }
    return value as RuntimeReadyMarker;
  } catch {
    return undefined;
  }
}

/** A payload directory is usable only when its ready marker matches the manifest. */
async function isPayloadReady(
  payloadRoot: string,
  manifest: RuntimePayloadManifest
): Promise<boolean> {
  const marker = await readReadyMarker(payloadRoot);
  if (marker === undefined) {
    return false;
  }
  if (
    marker.payloadVersion !== manifest.payloadVersion ||
    marker.archiveSha256 !== manifest.archiveSha256 ||
    marker.pythonExecutable !== manifest.pythonExecutable
  ) {
    return false;
  }
  return existsSync(payloadPath(payloadRoot, manifest.pythonExecutable));
}

async function sha256File(path: string): Promise<string> {
  return createHash("sha256").update(await readFile(path)).digest("hex");
}

async function promotePayloadDirectory(
  stagingDirectory: string,
  payloadRoot: string,
  manifest: RuntimePayloadManifest
): Promise<void> {
  try {
    await rename(stagingDirectory, payloadRoot);
    return;
  } catch {
    // Windows cannot replace an existing directory atomically: a concurrent launch
    // may have deployed this payload version already, or an earlier attempt died
    // halfway through. Both cases are settled below.
  }
  if (await isPayloadReady(payloadRoot, manifest)) {
    await rm(stagingDirectory, { recursive: true, force: true });
    return;
  }
  await rm(payloadRoot, { recursive: true, force: true });
  await rename(stagingDirectory, payloadRoot);
}

/**
 * Deploys the bundled Runtime payload into <runtime home>/runtime/<payload version>
 * unless that version is already deployed. Deployment extracts into a staging
 * directory and renames it into place, so an interrupted first launch can never
 * leave a payload that looks ready. Older payload versions are left untouched.
 */
export async function ensureRuntimePayloadDeployed(
  options: RuntimePayloadDeploymentOptions
): Promise<RuntimeLaunch> {
  const { manifest, resourcesPath, runtimeHome } = options;
  const runtimeDirectory = join(runtimeHome, RUNTIME_PAYLOAD_DIRECTORY);
  const payloadRoot = join(runtimeDirectory, manifest.payloadVersion);
  const pythonExecutable = payloadPath(payloadRoot, manifest.pythonExecutable);
  if (await isPayloadReady(payloadRoot, manifest)) {
    return { runtimeRoot: payloadRoot, pythonExecutable };
  }

  const archivePath = join(
    resourcesPath,
    RUNTIME_PAYLOAD_RESOURCE_DIRECTORY,
    manifest.archive
  );
  if (!existsSync(archivePath)) {
    throw new Error(`The bundled Ikaros Runtime archive is missing at ${archivePath}.`);
  }
  const archiveSha256 = await sha256File(archivePath);
  if (archiveSha256 !== manifest.archiveSha256) {
    throw new Error(
      `The bundled Ikaros Runtime archive ${manifest.archive} failed its SHA-256 check (expected ${manifest.archiveSha256}, got ${archiveSha256}).`
    );
  }

  options.onProgress?.(`Preparing the bundled Ikaros Runtime ${manifest.payloadVersion}.`);
  await mkdir(runtimeDirectory, { recursive: true });
  const stagingDirectory = join(runtimeDirectory, `${manifest.payloadVersion}.staging`);
  await rm(stagingDirectory, { recursive: true, force: true });
  await mkdir(stagingDirectory, { recursive: true });
  try {
    await extract({ file: archivePath, cwd: stagingDirectory, strict: true });
    if (!existsSync(payloadPath(stagingDirectory, manifest.pythonExecutable))) {
      throw new Error(
        `The bundled Ikaros Runtime archive ${manifest.archive} does not contain ${manifest.pythonExecutable}.`
      );
    }
    const marker: RuntimeReadyMarker = {
      schemaVersion: RUNTIME_PAYLOAD_SCHEMA_VERSION,
      payloadVersion: manifest.payloadVersion,
      archiveSha256: manifest.archiveSha256,
      pythonExecutable: manifest.pythonExecutable,
      deployedAt: new Date().toISOString()
    };
    await writeFile(
      join(stagingDirectory, RUNTIME_READY_MARKER_FILE),
      `${JSON.stringify(marker, null, 2)}\n`,
      "utf8"
    );
    await promotePayloadDirectory(stagingDirectory, payloadRoot, manifest);
  } catch (error) {
    await rm(stagingDirectory, { recursive: true, force: true }).catch(() => undefined);
    throw error;
  }
  return { runtimeRoot: payloadRoot, pythonExecutable };
}

/**
 * Resolves how to launch the Runtime for the current installation:
 * an explicit interpreter override wins, development runs use the checkout
 * virtual environment, and packaged runs deploy the bundled payload under
 * <runtime home>/runtime/<payload version>.
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
    const manifest = await readRuntimePayloadManifest(options.resourcesPath);
    if (manifest === undefined) {
      throw new Error(
        `The bundled Ikaros Runtime payload is missing from ${options.resourcesPath}. Reinstall Ikaros, or set IKAROS_RUNTIME_PYTHON to a Python 3.12/3.13 interpreter.`
      );
    }
    if (manifest.platform !== process.platform) {
      throw new Error(
        `The bundled Ikaros Runtime payload targets ${manifest.platform}, but this system is ${process.platform}.`
      );
    }
    return ensureRuntimePayloadDeployed({
      manifest,
      resourcesPath: options.resourcesPath,
      runtimeHome: options.runtimeHome,
      onProgress: options.onProgress
    });
  };
}