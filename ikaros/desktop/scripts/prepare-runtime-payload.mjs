#!/usr/bin/env node
/**
 * Build-time generator for the bundled Ikaros Runtime payload.
 *
 * A packaged Ikaros must run without a user-provided Python, uv, or source
 * checkout. This script assembles a self-contained Runtime payload (relocatable
 * CPython from python-build-standalone, the dependency set locked in
 * runtime/uv.lock, and the ikaros_runtime sources), packs it into one archive
 * and writes the manifest that the desktop main process reads from the packaged
 * resources.
 *
 * Outputs (default directory: ikaros/desktop/build/runtime):
 *   ikaros-runtime-win-x64.tar.gz  payload archive carried by electron-builder extraResources
 *   runtime-payload.json           manifest consumed by src/main/runtimeProvision.ts
 *
 * The archive is deployed to ~/.ikaros/runtime/<payload-version>/ on first
 * launch; see src/main/runtimeProvision.ts and README section 4.11.
 *
 * Usage:
 *   node scripts/prepare-runtime-payload.mjs [--platform win32] [--arch x64]
 *     [--python-version 3.12.15] [--python-build-release 20261013]
 *     [--out <directory>] [--keep-extracted] [--allow-unsupported]
 *
 * Requirements: uv on PATH; network access on the first run (the CPython
 * archive is cached under <out>/cache and verified against SHA256SUMS).
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, createWriteStream, existsSync } from "node:fs";
import { cp, mkdir, readFile, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { basename, dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { create, extract } from "tar";

const PYTHON_BUILD_STANDALONE_REPOSITORY = "astral-sh/python-build-standalone";
const DEFAULT_PYTHON_VERSION = "3.12.15";
const DEFAULT_PYTHON_BUILD_RELEASE = "20261013";
const ARCHIVE_PREFIX = "ikaros-runtime";
const MANIFEST_NAME = "runtime-payload.json";
const PAYLOAD_ROOT_DIRECTORY = "python";

/** Supported payload targets. Linux packaging is tracked as a follow-up issue. */
const TARGETS = new Map([
  [
    "win32-x64",
    {
      platform: "win32",
      arch: "x64",
      platformTag: "win-x64",
      pythonBuildTriple: "x86_64-pc-windows-msvc",
      pythonExecutable: "python/python.exe",
      sitePackages: "python/Lib/site-packages",
      trim: [
        "python/DLLs/_tkinter.pyd",
        "python/DLLs/tcl86t.dll",
        "python/DLLs/tk86t.dll",
        "python/Lib/ensurepip",
        "python/Lib/idlelib",
        "python/Lib/lib2to3",
        "python/Lib/site-packages/README.txt",
        "python/Lib/site-packages/pip",
        "python/Lib/site-packages/pip-*.dist-info",
        "python/Lib/tkinter",
        "python/Lib/turtle.py",
        "python/Lib/turtledemo",
        "python/include",
        "python/libs",
        "python/tcl"
      ]
    }
  ]
]);

function parseArguments(argv) {
  const options = {
    platform: process.platform,
    arch: process.arch,
    pythonVersion: DEFAULT_PYTHON_VERSION,
    pythonBuildRelease: DEFAULT_PYTHON_BUILD_RELEASE,
    out: undefined,
    keepExtracted: false,
    allowUnsupported: false,
    help: false
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const value = () => {
      const next = argv[index + 1];
      if (next === undefined || next.startsWith("--")) {
        throw new Error(`Missing value for ${argument}.`);
      }
      index += 1;
      return next;
    };
    switch (argument) {
      case "--platform":
        options.platform = value();
        break;
      case "--arch":
        options.arch = value();
        break;
      case "--python-version":
        options.pythonVersion = value();
        break;
      case "--python-build-release":
        options.pythonBuildRelease = value();
        break;
      case "--out":
        options.out = value();
        break;
      case "--keep-extracted":
        options.keepExtracted = true;
        break;
      case "--allow-unsupported":
        options.allowUnsupported = true;
        break;
      case "--help":
      case "-h":
        options.help = true;
        break;
      default:
        throw new Error(`Unknown argument: ${argument}`);
    }
  }
  return options;
}

function usage() {
  return [
    "Usage: node scripts/prepare-runtime-payload.mjs [options]",
    "",
    "  --platform <platform>              payload platform (default: host platform)",
    "  --arch <arch>                      payload architecture (default: host architecture)",
    `  --python-version <version>         CPython version (default: ${DEFAULT_PYTHON_VERSION})`,
    `  --python-build-release <release>   python-build-standalone release (default: ${DEFAULT_PYTHON_BUILD_RELEASE})`,
    "  --out <directory>                  output directory (default: build/runtime)",
    "  --keep-extracted                   keep the extracted payload tree for inspection",
    "  --allow-unsupported                skip targets without payload support instead of failing"
  ].join("\n");
}

function log(message) {
  process.stdout.write(`[prepare-runtime-payload] ${message}\n`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: options.capture ? "pipe" : "inherit",
    encoding: "utf8",
    shell: false,
    cwd: options.cwd
  });
  if (result.error) {
    if (result.error.code === "ENOENT") {
      throw new Error(`\`${command}\` was not found on PATH.`);
    }
    throw result.error;
  }
  if (result.status !== 0) {
    const detail = options.capture && result.stderr ? `\n${result.stderr.trim()}` : "";
    throw new Error(`\`${command} ${args.join(" ")}\` exited with code ${result.status}.${detail}`);
  }
  return (result.stdout ?? "").trim();
}

function sha256File(path) {
  return new Promise((resolvePromise, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(path);
    stream.on("error", reject);
    hash.on("error", reject);
    hash.on("readable", () => {
      const digest = hash.read();
      if (digest !== null) {
        resolvePromise(digest.toString("hex"));
      }
    });
    stream.pipe(hash);
  });
}

async function download(url, destination) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok || response.body === null) {
    throw new Error(`Failed to download ${url}: HTTP ${response.status}.`);
  }
  const temporary = `${destination}.partial`;
  await pipeline(Readable.fromWeb(response.body), createWriteStream(temporary));
  await rename(temporary, destination);
}

async function readRuntimeVersion(pyprojectPath) {
  const text = await readFile(pyprojectPath, "utf8");
  const match = /^version\s*=\s*"([^"]+)"/m.exec(text);
  if (match === null) {
    throw new Error(`Could not read the Runtime version from ${pyprojectPath}.`);
  }
  return match[1];
}

function pythonArchiveName(pythonVersion, release, target) {
  return `cpython-${pythonVersion}+${release}-${target.pythonBuildTriple}-install_only_stripped.tar.gz`;
}

async function expectedPythonArchiveSha256(cacheDir, release, archiveName) {
  const sumsPath = join(cacheDir, `SHA256SUMS-${release}`);
  if (!existsSync(sumsPath)) {
    await download(
      `https://github.com/${PYTHON_BUILD_STANDALONE_REPOSITORY}/releases/download/${release}/SHA256SUMS`,
      sumsPath
    );
  }
  const text = await readFile(sumsPath, "utf8");
  for (const line of text.split(/\r?\n/)) {
    const match = /^([0-9a-f]{64})\s+\*?(.+)$/.exec(line.trim());
    if (match !== null && match[2].trim() === archiveName) {
      return match[1];
    }
  }
  throw new Error(`SHA256SUMS for ${release} does not list ${archiveName}.`);
}

async function ensurePythonArchive(options) {
  const { cacheDir, release, target } = options;
  const archiveName = pythonArchiveName(options.pythonVersion, release, target);
  const archivePath = join(cacheDir, archiveName);
  const expected = await expectedPythonArchiveSha256(cacheDir, release, archiveName);
  if (existsSync(archivePath)) {
    if ((await sha256File(archivePath)) === expected) {
      log(`using cached ${archiveName}`);
      return archivePath;
    }
    log(`cached ${archiveName} failed verification, downloading again`);
  }
  log(`downloading ${archiveName}`);
  await download(
    `https://github.com/${PYTHON_BUILD_STANDALONE_REPOSITORY}/releases/download/${release}/${archiveName}`,
    archivePath
  );
  const actual = await sha256File(archivePath);
  if (actual !== expected) {
    await rm(archivePath, { force: true });
    throw new Error(`${archiveName} failed SHA-256 verification (expected ${expected}, got ${actual}).`);
  }
  return archivePath;
}

function globToRegExp(pattern) {
  const escaped = pattern.replace(/[.*+?^${}()|[\]\\]/g, (match) =>
    match === "*" ? "\u0000" : `\\${match}`
  );
  return new RegExp(`^${escaped.replace(/\u0000/g, ".*")}$`);
}

async function trimPayload(payloadDir, patterns) {
  for (const pattern of patterns) {
    const target = join(payloadDir, ...pattern.split("/"));
    if (!pattern.includes("*")) {
      await rm(target, { recursive: true, force: true });
      continue;
    }
    const matcher = globToRegExp(basename(target));
    const directory = dirname(target);
    for (const entry of await readdir(directory).catch(() => [])) {
      if (matcher.test(entry)) {
        await rm(join(directory, entry), { recursive: true, force: true });
      }
    }
  }
}

function exportRequirements(options) {
  const { repositoryRoot, requirementsFile } = options;
  const runtimeProject = join(repositoryRoot, "runtime");
  if (!existsSync(join(runtimeProject, "uv.lock"))) {
    throw new Error(`${runtimeProject} has no uv.lock; run \`uv lock --project runtime\` first.`);
  }
  run("uv", [
    "export",
    "--project",
    runtimeProject,
    "--locked",
    "--no-dev",
    "--no-emit-project",
    "--format",
    "requirements.txt",
    "-o",
    requirementsFile
  ]);
}

function installDependencies(options) {
  const { pythonExecutable, sitePackages, requirementsFile } = options;
  run("uv", [
    "pip",
    "install",
    "--python",
    pythonExecutable,
    "--target",
    sitePackages,
    "--require-hashes",
    "-r",
    requirementsFile
  ]);
}

async function copyRuntimeSource(options) {
  const { repositoryRoot, sitePackages } = options;
  const source = join(repositoryRoot, "runtime", "src", "ikaros_runtime");
  const destination = join(sitePackages, "ikaros_runtime");
  await rm(destination, { recursive: true, force: true });
  await cp(source, destination, {
    recursive: true,
    filter: (entry) => !entry.split(sep).includes("__pycache__") && !entry.endsWith(".pyc")
  });
}

function verifyPayload(options) {
  const { pythonExecutable, payloadDir } = options;
  const code = [
    "import httpx, websockets, yaml",
    "import ikaros_runtime",
    "print(ikaros_runtime.__version__)"
  ].join("; ");
  const version = run(pythonExecutable, ["-c", code], { capture: true, cwd: payloadDir });
  if (!/^\d+\.\d+/.test(version)) {
    throw new Error(`Payload smoke check returned an unexpected result: ${version}`);
  }
  return version;
}

async function measureDirectory(directory) {
  let files = 0;
  let bytes = 0;
  const pending = [directory];
  while (pending.length > 0) {
    const current = pending.pop();
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) {
        pending.push(path);
      } else {
        files += 1;
        bytes += (await stat(path)).size;
      }
    }
  }
  return { files, bytes };
}

function formatMiB(bytes) {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options.help) {
    process.stdout.write(`${usage()}\n`);
    return;
  }
  const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const repositoryRoot = resolve(desktopRoot, "..", "..");
  const target = TARGETS.get(`${options.platform}-${options.arch}`);
  if (target === undefined) {
    const message =
      `Runtime payload generation is not implemented for ${options.platform}/${options.arch} yet; ` +
      `supported targets: ${[...TARGETS.keys()].join(", ")} (Linux packaging is a follow-up issue).`;
    if (options.allowUnsupported) {
      log(`skipping: ${message}`);
      return;
    }
    throw new Error(message);
  }
  const runtimeVersion = await readRuntimeVersion(join(repositoryRoot, "runtime", "pyproject.toml"));
  const payloadVersion = `${runtimeVersion}-py${options.pythonVersion}-${target.platformTag}`;
  const outDir = resolve(options.out ?? join(desktopRoot, "build", "runtime"));
  const cacheDir = join(outDir, "cache");
  const workDir = join(outDir, `work-${target.platformTag}`);
  const payloadDir = join(workDir, "payload");
  await mkdir(cacheDir, { recursive: true });
  await rm(workDir, { recursive: true, force: true });
  await mkdir(payloadDir, { recursive: true });

  const pythonArchive = await ensurePythonArchive({
    cacheDir,
    release: options.pythonBuildRelease,
    pythonVersion: options.pythonVersion,
    target
  });
  log(`extracting ${basename(pythonArchive)}`);
  await extract({ file: pythonArchive, cwd: payloadDir, strict: true });
  if (!existsSync(join(payloadDir, PAYLOAD_ROOT_DIRECTORY))) {
    throw new Error(`${basename(pythonArchive)} did not contain ${PAYLOAD_ROOT_DIRECTORY}/.`);
  }
  await trimPayload(payloadDir, target.trim);

  const requirementsFile = join(workDir, "requirements.txt");
  log("exporting locked Runtime dependencies");
  exportRequirements({ repositoryRoot, requirementsFile });
  const pythonExecutable = join(payloadDir, target.pythonExecutable);
  const sitePackages = join(payloadDir, target.sitePackages);
  log("installing locked Runtime dependencies into the payload");
  installDependencies({ pythonExecutable, sitePackages, requirementsFile });
  await copyRuntimeSource({ repositoryRoot, sitePackages });
  log(`payload smoke check: ikaros_runtime ${verifyPayload({ pythonExecutable, payloadDir })}`);

  const unpacked = await measureDirectory(payloadDir);
  const archiveName = `${ARCHIVE_PREFIX}-${target.platformTag}.tar.gz`;
  const archivePath = join(outDir, archiveName);
  await rm(archivePath, { force: true });
  log(`creating ${archiveName}`);
  await create({ cwd: payloadDir, file: archivePath, gzip: true, portable: true }, [
    PAYLOAD_ROOT_DIRECTORY
  ]);
  const archiveSha256 = await sha256File(archivePath);
  const manifest = {
    schemaVersion: 1,
    payloadVersion,
    runtimeVersion,
    pythonVersion: options.pythonVersion,
    pythonBuildRelease: options.pythonBuildRelease,
    platform: target.platform,
    arch: target.arch,
    archive: archiveName,
    archiveSha256,
    pythonExecutable: target.pythonExecutable,
    unpackedBytes: unpacked.bytes,
    unpackedFiles: unpacked.files,
    builtAt: new Date().toISOString()
  };
  await writeFile(join(outDir, MANIFEST_NAME), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  if (!options.keepExtracted) {
    await rm(workDir, { recursive: true, force: true });
  }
  log(`payload version ${payloadVersion}`);
  log(`unpacked ${formatMiB(unpacked.bytes)} in ${unpacked.files} files`);
  log(`archive ${archiveName} (${formatMiB((await stat(archivePath)).size)}, sha256 ${archiveSha256})`);
  log(`manifest ${join(outDir, MANIFEST_NAME)}`);
}

main().catch((error) => {
  process.stderr.write(
    `[prepare-runtime-payload] ${error instanceof Error ? error.message : String(error)}\n`
  );
  process.exitCode = 1;
});