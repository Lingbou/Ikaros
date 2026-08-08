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
