// @vitest-environment node

import { describe, expect, it } from "vitest";

import { defaultInstallScriptPath, installRuntime } from "./runtimeInstaller";

describe("installRuntime", () => {
  it("explains a missing installer instead of failing silently", async () => {
    const result = await installRuntime({ scriptPath: undefined });

    expect(result.ok).toBe(false);
    expect(result.output).toContain("安装脚本不存在");
  });

  it("locates the installer in the checkout or in bundled resources", () => {
    const name =
      process.platform === "win32" ? "install-runtime.ps1" : "install-runtime.sh";
    const development = defaultInstallScriptPath({
      isPackaged: false,
      appPath: "/repo/ikaros/desktop",
      resourcesPath: "/resources"
    });
    const packaged = defaultInstallScriptPath({
      isPackaged: true,
      appPath: "/repo/ikaros/desktop",
      resourcesPath: "/resources"
    });

    expect(development.endsWith(name)).toBe(true);
    expect(development).toContain("scripts");
    expect(packaged.endsWith(name)).toBe(true);
  });
});
