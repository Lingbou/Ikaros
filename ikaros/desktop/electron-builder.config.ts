import type { Configuration } from "electron-builder";

const configuration: Configuration = {
  appId: "io.github.lingbou.ikaros",
  productName: "Ikaros",
  copyright: "Copyright © 2026 Ikaros contributors",
  asar: true,
  compression: "maximum",
  npmRebuild: false,
  directories: {
    output: "dist"
  },
  files: ["out/**/*", "package.json"],
  linux: {
    target: [
      {
        target: "AppImage",
        arch: ["x64"]
      },
      {
        target: "deb",
        arch: ["x64"]
      },
      {
        target: "rpm",
        arch: ["x64"]
      }
    ],
    artifactName: "Ikaros-${version}-linux-${arch}.${ext}",
    executableName: "ikaros",
    icon: "build/icon.png",
    category: "Utility",
    synopsis: "General-purpose AI agent desktop client",
    description: "Ikaros desktop client for planning, research, creation, and agent workflows.",
    maintainer: "Lingbou <Lingbou@users.noreply.github.com>",
    vendor: "Ikaros contributors",
    syncDesktopName: true
  },
  deb: {
    packageName: "ikaros",
    packageCategory: "utils"
  },
  rpm: {
    packageName: "ikaros"
  },
  win: {
    icon: "build/icon.ico",
    target: [
      {
        target: "nsis",
        arch: ["x64"]
      }
    ],
    artifactName: "Ikaros-${version}-windows-${arch}-setup.${ext}",
    executableName: "Ikaros",
    requestedExecutionLevel: "asInvoker"
  },
  nsis: {
    oneClick: false,
    perMachine: false,
    allowElevation: true,
    allowToChangeInstallationDirectory: true,
    createDesktopShortcut: "always",
    createStartMenuShortcut: true,
    shortcutName: "Ikaros",
    installerIcon: "build/icon.ico",
    uninstallerIcon: "build/icon.ico",
    deleteAppDataOnUninstall: false
  }
};

export default configuration;
