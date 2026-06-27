# Deployment

Ikaros deployment is pre-MVP. The supported deployment artifact in this tree is
a local Docker image for development and smoke testing. It is not a hosted,
multi-user, or hardened production deployment.

## Docker Image

Build from the repository root:

```bash
docker build -f docker/Dockerfile -t ikaros:local .
```

The Dockerfile uses a Rust builder stage and copies only the final `ikaros`
binary into a Debian slim runtime image. The runtime image runs as the
unprivileged `ikaros` user, sets `IKAROS_HOME=/data/ikaros`, and uses
`/workspace` as the working directory.

Run one command:

```bash
docker run --rm -it \
  -e IKAROS_HOME=/data/ikaros \
  -v ikaros-home:/data/ikaros \
  -v "$PWD":/workspace \
  -w /workspace \
  ikaros:local config show
```

Use Compose from the repository root:

```bash
docker compose -f docker/compose.yml run --rm ikaros --help
docker compose -f docker/compose.yml run --rm ikaros setup \
  --api-key "$MODEL_API_KEY" \
  --base-url https://api.example.com/v1 \
  --model provider-model-id
docker compose -f docker/compose.yml run --rm ikaros tui
```

## State And Secrets

Do not bake provider credentials into the image. Runtime configuration and local
state belong in `/data/ikaros`, which should be a Docker volume or bind mount.
The image expects plaintext provider credentials to live in
`/data/ikaros/config.yaml`, matching the normal local `IKAROS_HOME/config.yaml`
contract.

The repository mount at `/workspace` is intended for local development
workflows. Keep the same harness and workspace policy assumptions as a host
run: Docker here packages the CLI, but it is not a replacement for a full
process sandbox.

## Installers And Packaging

The repository also contains early packaging entry points:

- `install.sh` and `install.ps1` for local release-binary installation.
- `flake.nix` for Nix development and package experiments.
- `packaging/arch/PKGBUILD` for Arch packaging experiments.
- cargo-dist metadata for future release artifact generation.

These files are maintainer/developer packaging scaffolds. They may expect tagged
release artifacts, a local build, or platform-specific tools, and they do not
yet define a stable published release channel.

## Limitations

- No published image is defined yet.
- Packaging templates exist, but no Nix cache, Windows package feed, Homebrew
  tap, AUR package, or cargo-dist release artifact is guaranteed yet.
- The deployment image is separate from the `execution.sandbox.backend: docker`
  process sandbox. Use `execution.sandbox.image` to choose the image for
  containerized command/test execution.
