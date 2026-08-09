# Ikaros Desktop

Cross-platform, mock-driven interaction prototype for the Ikaros
general-purpose Agent on Windows and Linux. It deliberately does not connect to
a model or persist canonical conversations yet; its job is to validate the
interaction and event contract before a replacement runtime is designed.

## Run locally

Prerequisites: Node.js 22.12 or newer and pnpm 11.9.

```shell
pnpm install
pnpm dev
```

Useful verification commands:

```shell
pnpm check
pnpm package:dir
pnpm package:linux
pnpm package:win
```

`package:linux` creates unsigned x64 AppImage, DEB, and RPM artifacts in
`dist/`. The Linux packages install the `io.github.lingbou.ikaros.desktop`
launcher, the Ikaros application icon, and the `ikaros` executable.

Fedora build hosts need the FPM compatibility library and RPM build tools:

```shell
sudo dnf install libxcrypt-compat rpm-build
```

Debian and Ubuntu build hosts need the RPM tooling only when producing all
three formats:

```shell
sudo apt install rpm
```

Run the portable AppImage directly, or install the native package for the host
distribution:

```shell
./dist/Ikaros-0.1.0-linux-x86_64.AppImage
sudo apt install ./dist/Ikaros-0.1.0-linux-amd64.deb
sudo dnf install ./dist/Ikaros-0.1.0-linux-x86_64.rpm
```

`package:win` creates an unsigned Windows x64 NSIS development installer. It
is a prototype artifact, not a signed production release.

## Prototype scenarios

The deterministic `MockAgentClient` covers:

1. Streaming response and stop.
2. Tool success and failure.
3. Permission allow and deny.
4. Interrupt, retry, and recovery.
5. Artifact and file output.

Projects are conversational workspaces and may exist without a local folder.
Threads with a project assignment appear only under that project; standalone
threads appear only under Recents. Both use the same run, permission, recovery,
artifact, and branching capabilities.
Editing an earlier message creates a branch instead of truncating history.

## Boundary

The React renderer depends on `AgentClient` and platform ports, never on raw
Electron IPC. The preload exposes only a narrow typed API for window controls
and UI preferences. A future desktop adapter may connect Electron main to a
headless runtime over a versioned stdio protocol; the renderer must not own the
runtime process or canonical Agent state.
