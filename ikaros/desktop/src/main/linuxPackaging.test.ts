import { describe, expect, it } from "vitest";

import desktopPackage from "../../package.json";
import configuration from "../../electron-builder.config";

describe("Linux distribution packaging", () => {
  it("builds installable artifacts with a stable desktop identity", () => {
    expect(configuration.linux).toMatchObject({
      artifactName: "Ikaros-${version}-linux-${arch}.${ext}",
      category: "Utility",
      executableName: "ikaros",
      icon: "build/icon.png",
      syncDesktopName: true,
      target: [
        { target: "AppImage", arch: ["x64"] },
        { target: "deb", arch: ["x64"] },
        { target: "rpm", arch: ["x64"] },
      ],
    });
    expect((desktopPackage as { desktopName?: string }).desktopName).toBe(
      "io.github.lingbou.ikaros.desktop",
    );
    expect(desktopPackage.scripts["package:linux"]).toContain("--linux");
  });

  it("uses one package name across Debian and RPM repositories", () => {
    expect(configuration.deb).toMatchObject({
      packageCategory: "utils",
      packageName: "ikaros",
    });
    expect(configuration.rpm).toMatchObject({
      packageName: "ikaros",
    });
  });
});
