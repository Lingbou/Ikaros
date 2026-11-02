// @vitest-environment node

import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { createRuntimeLaunchResolver } from "./runtimeLaunchResolver";
import { developmentRuntimeRoot, runtimePython } from "./runtimePaths";

interface InstalledRuntimeFixture {
  runtimeRoot: string;
  pythonExecutable: string;
  manifestPath: string;
}

const temporaryRoots: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryRoots.splice(0).map((root) => rm(root, { recursive: true, force: true }))
  );
});

async function createTemporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "ikaros-runtime-launch-"));
  temporaryRoots.push(root);
  return root;
}

async function writeInstallManifest(
  runtimeHome: string,
  overrides: Record<string, unknown> = {}
): Promise<InstalledRuntimeFixture> {
  const runtimeRoot = join(runtimeHome, "runtime", "0.1.0");
  const pythonExecutable = join(runtimeRoot, "bin", "python");
  const manifestPath = join(runtimeHome, "runtime", "install.json");
  await mkdir(dirname(pythonExecutable), { recursive: true });
  await writeFile(pythonExecutable, "installed-python\n", "utf8");
  await writeFile(
    manifestPath,
    JSON.stringify({
      schemaVersion: 1,
      runtimeVersion: "0.1.0",
      protocolVersion: 12,
      runtimeRoot,
      pythonExecutable,
      installedAt: "2026-10-22T00:00:00.000Z",
      ...overrides
    })
  );
  return { runtimeRoot, pythonExecutable, manifestPath };
}

function packagedResolver(
  root: string,
  overrides: Partial<Parameters<typeof createRuntimeLaunchResolver>[0]> = {}
): () => Promise<{ runtimeRoot: string; pythonExecutable: string }> {
  return createRuntimeLaunchResolver({
    isPackaged: true,
    appPath: join(root, "resources", "app.asar"),
    runtimeHome: join(root, "home"),
    ...overrides
  });
}

describe("Runtime launch resolver", () => {
  it("uses the checkout virtual environment during development", async () => {
    const root = await createTemporaryRoot();
    const appPath = join(root, "checkout", "ikaros", "desktop");
    const runtimeRoot = developmentRuntimeRoot(appPath);

    const launch = await createRuntimeLaunchResolver({
      isPackaged: false,
      appPath,
      runtimeHome: join(root, "home")
    })();

    expect(launch).toEqual({
      runtimeRoot,
      pythonExecutable: runtimePython(runtimeRoot)
    });
  });

  it("prefers an explicit interpreter override", async () => {
    const root = await createTemporaryRoot();
    const appPath = join(root, "checkout", "ikaros", "desktop");
    const interpreter = join(root, "custom-python", "python");

    const launch = await createRuntimeLaunchResolver({
      isPackaged: true,
      appPath,
      runtimeHome: join(root, "home"),
      pythonExecutable: interpreter
    })();

    expect(launch).toEqual({
      runtimeRoot: developmentRuntimeRoot(appPath),
      pythonExecutable: interpreter
    });
  });

  it("reads IKAROS_RUNTIME_PYTHON from the supplied environment", async () => {
    const root = await createTemporaryRoot();
    const interpreter = join(root, "custom-python", "python");

    const launch = await createRuntimeLaunchResolver({
      isPackaged: true,
      appPath: join(root, "resources", "app.asar"),
      runtimeHome: join(root, "home"),
      env: { IKAROS_RUNTIME_PYTHON: `  ${interpreter}  ` }
    })();

    expect(launch.pythonExecutable).toBe(interpreter);
  });

  it("uses the Runtime installation manifest in packaged builds", async () => {
    const root = await createTemporaryRoot();
    const runtimeHome = join(root, "home");
    const installed = await writeInstallManifest(runtimeHome);

    const launch = await packagedResolver(root)();

    expect(launch).toEqual({
      runtimeRoot: installed.runtimeRoot,
      pythonExecutable: installed.pythonExecutable
    });
  });

  it("reports the install manifest when no Runtime is installed", async () => {
    const root = await createTemporaryRoot();
    const runtimeHome = join(root, "home");
    const legacyPayload = join(root, "resources", "runtime", "runtime-payload.json");
    await mkdir(dirname(legacyPayload), { recursive: true });
    await writeFile(legacyPayload, "{}\n", "utf8");

    await expect(packagedResolver(root)()).rejects.toThrow(
      `${join(runtimeHome, "runtime", "install.json")}`
    );
  });

  it("rejects a missing installed interpreter", async () => {
    const root = await createTemporaryRoot();
    const runtimeHome = join(root, "home");
    const installed = await writeInstallManifest(runtimeHome);
    await rm(installed.pythonExecutable);

    await expect(packagedResolver(root)()).rejects.toThrow(
      `interpreter is missing at ${installed.pythonExecutable}`
    );
  });

  it("rejects an install manifest that is not valid JSON", async () => {
    const root = await createTemporaryRoot();
    const runtimeHome = join(root, "home");
    const installed = await writeInstallManifest(runtimeHome);
    await writeFile(installed.manifestPath, "not json\n", "utf8");

    await expect(packagedResolver(root)()).rejects.toThrow(/is not valid JSON/);
  });

  it("rejects an unsupported install schema", async () => {
    const root = await createTemporaryRoot();
    await writeInstallManifest(join(root, "home"), { schemaVersion: 2 });

    await expect(packagedResolver(root)()).rejects.toThrow(/uses install schema 2/);
  });

  it("rejects an install manifest with missing fields", async () => {
    const root = await createTemporaryRoot();
    await writeInstallManifest(join(root, "home"), { runtimeRoot: undefined });

    await expect(packagedResolver(root)()).rejects.toThrow(/"runtimeRoot"/);
  });
});
