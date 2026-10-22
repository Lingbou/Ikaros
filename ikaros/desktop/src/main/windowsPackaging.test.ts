import { describe, expect, it } from "vitest";

import desktopPackage from "../../package.json";
import configuration from "../../electron-builder.config";

describe("Windows distribution packaging", () => {
  it("ships the Runtime installer and wheel as extra resources", () => {
    expect(configuration.win).toMatchObject({
      artifactName: "Ikaros-${version}-windows-${arch}-setup.${ext}",
      executableName: "Ikaros",
      target: [{ target: "nsis", arch: ["x64"] }],
      extraResources: [
        {
          from: "../../scripts/install-runtime.ps1",
          to: "scripts/install-runtime.ps1"
        },
        {
          from: "build/runtime",
          to: "runtime",
          filter: ["*.whl", "runtime-bundle.json"]
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

  it("builds the Runtime wheel before packaging", () => {
    expect(desktopPackage.scripts["prepare:wheel"]).toContain(
      "scripts/prepare-runtime-wheel.mjs"
    );
    expect(desktopPackage.scripts["package:win"]).toContain(
      "scripts/prepare-runtime-wheel.mjs"
    );
    expect(desktopPackage.scripts["package:win"]).not.toContain(
      "scripts/prepare-runtime-payload.mjs"
    );
    expect(desktopPackage.scripts["package:win"]).toContain("--win nsis --x64");
    expect(desktopPackage.scripts["package:dir"]).toContain(
      "scripts/prepare-runtime-wheel.mjs"
    );
  });
});