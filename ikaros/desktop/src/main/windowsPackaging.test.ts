import { describe, expect, it } from "vitest";

import desktopPackage from "../../package.json";
import configuration from "../../electron-builder.config";

describe("Windows distribution packaging", () => {
  it("ships the bundled Runtime payload as extra resources", () => {
    expect(configuration.win).toMatchObject({
      artifactName: "Ikaros-${version}-windows-${arch}-setup.${ext}",
      executableName: "Ikaros",
      target: [{ target: "nsis", arch: ["x64"] }],
      extraResources: [
        {
          from: "build/runtime/ikaros-runtime-win-x64.tar.gz",
          to: "runtime/ikaros-runtime-win-x64.tar.gz"
        },
        {
          from: "build/runtime/runtime-payload.json",
          to: "runtime/runtime-payload.json"
        }
      ]
    });
  });

  it("keeps user data when the installer is removed", () => {
    expect(configuration.nsis).toMatchObject({
      deleteAppDataOnUninstall: false,
      oneClick: false,
      perMachine: false
    });
  });

  it("generates the Runtime payload before packaging", () => {
    expect(desktopPackage.scripts["prepare:runtime"]).toContain(
      "scripts/prepare-runtime-payload.mjs"
    );
    expect(desktopPackage.scripts["package:win"]).toContain(
      "scripts/prepare-runtime-payload.mjs"
    );
    expect(desktopPackage.scripts["package:win"]).toContain("--win nsis --x64");
    expect(desktopPackage.scripts["package:dir"]).toContain(
      "scripts/prepare-runtime-payload.mjs"
    );
  });
});