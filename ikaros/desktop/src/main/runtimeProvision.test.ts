// @vitest-environment node

import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { create } from "tar";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  RUNTIME_PAYLOAD_DIRECTORY,
  RUNTIME_PAYLOAD_RESOURCE_DIRECTORY,
  RUNTIME_READY_MARKER_FILE,
  createRuntimeLaunchResolver,
  ensureRuntimePayloadDeployed,
  readRuntimePayloadManifest,
  runtimeInstallManifestPath,
  runtimePayloadManifestPath,
  type RuntimeLaunchResolverOptions,
  type RuntimePayloadManifest
} from "./runtimeProvision";

const PAYLOAD_VERSION = "0.1.0-py3.12.15-win-x64";
const ARCHIVE_NAME = "ikaros-runtime-win-x64.tar.gz";
const PYTHON_RELATIVE_PATH = "python/python.exe";

const temporaryRoots: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryRoots.splice(0).map((root) => rm(root, { recursive: true, force: true }))
  );
});

async function createTemporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "ikaros-runtime-payload-"));
  temporaryRoots.push(root);
  return root;
}

function sha256(contents: Buffer): string {
  return createHash("sha256").update(contents).digest("hex");
}

interface Fixture {
  root: string;
  resourcesPath: string;
  runtimeHome: string;
  archivePath: string;
  manifestPath: string;
  manifest: RuntimePayloadManifest;
  pythonContents: string;
}

async function createFixture(options: { pythonContents?: string } = {}): Promise<Fixture> {
  const root = await createTemporaryRoot();
  const resourcesPath = join(root, "resources");
  const runtimeHome = join(root, "home");
  const source = join(root, "payload-source");
  const pythonContents = options.pythonContents ?? "bundled-python\n";
  await mkdir(join(source, "python"), { recursive: true });
  await writeFile(join(source, "python", "python.exe"), pythonContents, "utf8");
  const archivePath = join(resourcesPath, RUNTIME_PAYLOAD_RESOURCE_DIRECTORY, ARCHIVE_NAME);
  await mkdir(dirname(archivePath), { recursive: true });
  await create({ cwd: source, file: archivePath, gzip: true, portable: true }, ["python"]);
  const manifest: RuntimePayloadManifest = {
    schemaVersion: 1,
    payloadVersion: PAYLOAD_VERSION,
    runtimeVersion: "0.1.0",
    pythonVersion: "3.12.15",
    pythonBuildRelease: "20261013",
    platform: process.platform,
    arch: "x64",
    archive: ARCHIVE_NAME,
    archiveSha256: sha256(await readFile(archivePath)),
    pythonExecutable: PYTHON_RELATIVE_PATH,
    unpackedBytes: 1024,
    unpackedFiles: 1,
    builtAt: "2026-10-20T00:00:00.000Z"
  };
  const manifestPath = runtimePayloadManifestPath(resourcesPath);
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  return { root, resourcesPath, runtimeHome, archivePath, manifestPath, manifest, pythonContents };
}

async function writeManifest(fixture: Fixture, manifest: RuntimePayloadManifest): Promise<void> {
  fixture.manifest = manifest;
  await writeFile(fixture.manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

async function rebuildArchive(fixture: Fixture, pythonContents: string): Promise<void> {
  const source = await mkdtemp(join(fixture.root, "payload-source-"));
  await mkdir(join(source, "python"), { recursive: true });
  await writeFile(join(source, "python", "python.exe"), pythonContents, "utf8");
  await rm(fixture.archivePath, { force: true });
  await create({ cwd: source, file: fixture.archivePath, gzip: true, portable: true }, ["python"]);
  await writeManifest(fixture, {
    ...fixture.manifest,
    archiveSha256: sha256(await readFile(fixture.archivePath))
  });
}

function packagedResolver(
  fixture: Fixture,
  overrides: Partial<RuntimeLaunchResolverOptions> = {}
): () => Promise<{ runtimeRoot: string; pythonExecutable: string }> {
  return createRuntimeLaunchResolver({
    isPackaged: true,
    appPath: join(fixture.root, "resources", "app.asar"),
    resourcesPath: fixture.resourcesPath,
    runtimeHome: fixture.runtimeHome,
    ...overrides
  });
}

function deployedRoot(fixture: Fixture): string {
  return join(fixture.runtimeHome, RUNTIME_PAYLOAD_DIRECTORY, fixture.manifest.payloadVersion);
}

function deployedPython(fixture: Fixture): string {
  return join(deployedRoot(fixture), ...PYTHON_RELATIVE_PATH.split("/"));
}

async function exists(path: string): Promise<boolean> {
  return stat(path).then(
    () => true,
    () => false
  );
}

describe("Runtime payload deployment", () => {
  it("deploys the bundled payload into the runtime home and marks it ready", async () => {
    const fixture = await createFixture();
    const launch = await packagedResolver(fixture)();

    expect(launch.runtimeRoot).toBe(deployedRoot(fixture));
    expect(launch.pythonExecutable).toBe(deployedPython(fixture));
    expect(await readFile(launch.pythonExecutable, "utf8")).toBe(fixture.pythonContents);
    const marker = JSON.parse(
      await readFile(join(launch.runtimeRoot, RUNTIME_READY_MARKER_FILE), "utf8")
    ) as Record<string, unknown>;
    expect(marker).toMatchObject({
      schemaVersion: 1,
      payloadVersion: fixture.manifest.payloadVersion,
      archiveSha256: fixture.manifest.archiveSha256,
      pythonExecutable: PYTHON_RELATIVE_PATH
    });
    expect(await exists(join(fixture.runtimeHome, RUNTIME_PAYLOAD_DIRECTORY, `${PAYLOAD_VERSION}.staging`))).toBe(
      false
    );
  });

  it("reuses an already deployed payload version instead of extracting again", async () => {
    const fixture = await createFixture();
    await packagedResolver(fixture)();
    await writeFile(deployedPython(fixture), "patched-after-deployment\n", "utf8");

    const launch = await packagedResolver(fixture)();

    expect(launch.pythonExecutable).toBe(deployedPython(fixture));
    expect(await readFile(launch.pythonExecutable, "utf8")).toBe("patched-after-deployment\n");
  });

  it("reports progress only when a deployment is required", async () => {
    const fixture = await createFixture();
    const onProgress = vi.fn();

    await packagedResolver(fixture, { onProgress })();
    expect(onProgress).toHaveBeenCalledTimes(1);
    expect(onProgress.mock.calls[0][0]).toContain(PAYLOAD_VERSION);

    onProgress.mockClear();
    await packagedResolver(fixture, { onProgress })();
    expect(onProgress).not.toHaveBeenCalled();
  });

  it("re-deploys when the deployed payload does not match the manifest", async () => {
    const fixture = await createFixture();
    await packagedResolver(fixture)();
    await writeFile(join(deployedRoot(fixture), RUNTIME_READY_MARKER_FILE), "{}\n", "utf8");

    await packagedResolver(fixture)();

    expect(await readFile(deployedPython(fixture), "utf8")).toBe(fixture.pythonContents);
  });

  it("re-deploys when the same payload version ships different content", async () => {
    const fixture = await createFixture();
    await packagedResolver(fixture)();
    await rebuildArchive(fixture, "rebuilt-python\n");

    const launch = await packagedResolver(fixture)();

    expect(await readFile(launch.pythonExecutable, "utf8")).toBe("rebuilt-python\n");
  });

  it("re-deploys when the deployed interpreter disappeared", async () => {
    const fixture = await createFixture();
    await packagedResolver(fixture)();
    await rm(deployedPython(fixture), { force: true });

    const launch = await packagedResolver(fixture)();

    expect(await readFile(launch.pythonExecutable, "utf8")).toBe(fixture.pythonContents);
  });

  it("cleans up a staging directory left behind by an interrupted deployment", async () => {
    const fixture = await createFixture();
    const staging = join(fixture.runtimeHome, RUNTIME_PAYLOAD_DIRECTORY, `${PAYLOAD_VERSION}.staging`);
    await mkdir(join(staging, "python"), { recursive: true });
    await writeFile(join(staging, "python", "leftover.txt"), "leftover\n", "utf8");

    await packagedResolver(fixture)();

    expect(await exists(join(staging, "python", "leftover.txt"))).toBe(false);
  });

  it("leaves previously deployed payload versions untouched", async () => {
    const fixture = await createFixture();
    const olderRoot = join(fixture.runtimeHome, RUNTIME_PAYLOAD_DIRECTORY, "0.0.9-py3.12.15-win-x64");
    await mkdir(olderRoot, { recursive: true });
    await writeFile(join(olderRoot, "keep.txt"), "keep\n", "utf8");

    await packagedResolver(fixture)();

    expect(await readFile(join(olderRoot, "keep.txt"), "utf8")).toBe("keep\n");
  });

  it("rejects an archive that does not match the manifest checksum", async () => {
    const fixture = await createFixture();
    await writeManifest(fixture, { ...fixture.manifest, archiveSha256: "0".repeat(64) });

    await expect(packagedResolver(fixture)()).rejects.toThrow(/failed its SHA-256 check/);
    expect(await exists(deployedRoot(fixture))).toBe(false);
    expect(await exists(join(fixture.runtimeHome, RUNTIME_PAYLOAD_DIRECTORY, `${PAYLOAD_VERSION}.staging`))).toBe(
      false
    );
  });

  it("reports a payload archive that is missing from the installation", async () => {
    const fixture = await createFixture();
    await rm(fixture.archivePath, { force: true });

    await expect(packagedResolver(fixture)()).rejects.toThrow(/archive is missing/);
  });

  it("reports a payload manifest that is missing from the installation", async () => {
    const fixture = await createFixture();
    await rm(fixture.manifestPath, { force: true });

    await expect(packagedResolver(fixture)()).rejects.toThrow(/No Ikaros Runtime installation was found/);
  });

  it("rejects a payload built for another platform", async () => {
    const fixture = await createFixture();
    await writeManifest(fixture, {
      ...fixture.manifest,
      platform: process.platform === "win32" ? "linux" : "win32"
    });

    await expect(packagedResolver(fixture)()).rejects.toThrow(/targets/);
  });

  it("keeps using the checkout Runtime when the application is not packaged", async () => {
    const fixture = await createFixture();
    const appPath = join(fixture.root, "checkout", "ikaros", "desktop");

    const launch = await createRuntimeLaunchResolver({
      isPackaged: false,
      appPath,
      resourcesPath: fixture.resourcesPath,
      runtimeHome: fixture.runtimeHome
    })();

    expect(launch.runtimeRoot).toBe(join(fixture.root, "checkout", "runtime"));
    expect(launch.pythonExecutable).toBe(
      join(
        fixture.root,
        "checkout",
        "runtime",
        ".venv",
        process.platform === "win32" ? "Scripts" : "bin",
        process.platform === "win32" ? "python.exe" : "python"
      )
    );
    expect(await exists(join(fixture.runtimeHome, RUNTIME_PAYLOAD_DIRECTORY))).toBe(false);
  });

  it("prefers an explicit interpreter override over the bundled payload", async () => {
    const fixture = await createFixture();
    const interpreter = join(fixture.root, "custom-python", "python.exe");
    await mkdir(dirname(interpreter), { recursive: true });
    await writeFile(interpreter, "custom\n", "utf8");

    const launch = await packagedResolver(fixture, { pythonExecutable: interpreter })();
    const environmentLaunch = await packagedResolver(fixture, {
      env: { IKAROS_RUNTIME_PYTHON: interpreter }
    })();

    expect(launch.pythonExecutable).toBe(interpreter);
    expect(environmentLaunch.pythonExecutable).toBe(interpreter);
    expect(await exists(deployedRoot(fixture))).toBe(false);
  });
});

describe("Runtime payload manifest", () => {
  it("returns undefined when the installation has no payload manifest", async () => {
    const root = await createTemporaryRoot();

    expect(await readRuntimePayloadManifest(root)).toBeUndefined();
  });

  it("reads the manifest shipped with the installation", async () => {
    const fixture = await createFixture();

    await expect(readRuntimePayloadManifest(fixture.resourcesPath)).resolves.toMatchObject({
      payloadVersion: PAYLOAD_VERSION,
      archive: ARCHIVE_NAME,
      pythonExecutable: PYTHON_RELATIVE_PATH
    });
  });

  it("rejects a manifest that is not valid JSON", async () => {
    const fixture = await createFixture();
    await writeFile(fixture.manifestPath, "not json\n", "utf8");

    await expect(readRuntimePayloadManifest(fixture.resourcesPath)).rejects.toThrow(
      /is not valid JSON/
    );
  });

  it("rejects a manifest from an unsupported payload schema", async () => {
    const fixture = await createFixture();
    await writeManifest(fixture, { ...fixture.manifest, schemaVersion: 2 });

    await expect(readRuntimePayloadManifest(fixture.resourcesPath)).rejects.toThrow(
      /Runtime payload schema 2/
    );
  });

  it("rejects a manifest with missing fields", async () => {
    const fixture = await createFixture();
    const manifest = { ...fixture.manifest } as Partial<RuntimePayloadManifest>;
    delete manifest.archive;
    await writeFile(fixture.manifestPath, JSON.stringify(manifest), "utf8");

    await expect(readRuntimePayloadManifest(fixture.resourcesPath)).rejects.toThrow(
      /"archive"/
    );
  });

  it("rejects a manifest that points outside the payload directory", async () => {
    const fixture = await createFixture();
    await writeManifest(fixture, { ...fixture.manifest, pythonExecutable: "../python.exe" });

    await expect(readRuntimePayloadManifest(fixture.resourcesPath)).rejects.toThrow(
      /unsafe "pythonExecutable"/
    );
  });

  it("rejects a payload version that is not a single path segment", async () => {
    const fixture = await createFixture();
    await writeManifest(fixture, { ...fixture.manifest, payloadVersion: "0.1.0/../.." });

    await expect(readRuntimePayloadManifest(fixture.resourcesPath)).rejects.toThrow(
      /unsafe "payloadVersion"/
    );
  });

  it("deploys the payload of the manifest that was passed in", async () => {
    const fixture = await createFixture();

    const launch = await ensureRuntimePayloadDeployed({
      manifest: fixture.manifest,
      resourcesPath: fixture.resourcesPath,
      runtimeHome: fixture.runtimeHome
    });

    expect(launch.pythonExecutable).toBe(deployedPython(fixture));
  });
});

describe("Runtime install manifest", () => {
  it("prefers a standalone installation over the bundled payload", async () => {
    const fixture = await createFixture();
    const installedRoot = join(fixture.runtimeHome, "runtime", "0.1.0-standalone");
    const python = join(installedRoot, "bin", "python");
    await mkdir(dirname(python), { recursive: true });
    await writeFile(python, "");
    await writeFile(
      runtimeInstallManifestPath(fixture.runtimeHome),
      JSON.stringify({
        schemaVersion: 1,
        runtimeVersion: "0.1.0",
        protocolVersion: 12,
        runtimeRoot: installedRoot,
        pythonExecutable: python,
        installedAt: "2026-10-22T00:00:00.000Z"
      })
    );

    const launch = await packagedResolver(fixture)();

    expect(launch.pythonExecutable).toBe(python);
    expect(launch.runtimeRoot).toBe(installedRoot);
  });

  it("explains how to install a runtime when neither source exists", async () => {
    const fixture = await createFixture();
    await rm(join(fixture.resourcesPath, RUNTIME_PAYLOAD_RESOURCE_DIRECTORY), {
      recursive: true,
      force: true
    });

    await expect(packagedResolver(fixture)()).rejects.toThrow(/install\.json/);
  });
});
