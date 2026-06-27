# Docker Deployment

This directory contains the first Docker deployment slice for Ikaros.

Build the image from the repository root:

```bash
docker build -f docker/Dockerfile -t ikaros:local .
```

Run a one-shot command with local state stored in a named Docker volume:

```bash
docker run --rm -it \
  -e IKAROS_HOME=/data/ikaros \
  -v ikaros-home:/data/ikaros \
  -v "$PWD":/workspace \
  -w /workspace \
  ikaros:local doctor
```

Initialize and configure a real model provider:

```bash
docker run --rm -it \
  -e IKAROS_HOME=/data/ikaros \
  -v ikaros-home:/data/ikaros \
  -v "$PWD":/workspace \
  -w /workspace \
  ikaros:local setup \
    --api-key "$MODEL_API_KEY" \
    --base-url https://api.example.com/v1 \
    --model provider-model-id
```

Use Compose from the repository root:

```bash
docker compose -f docker/compose.yml run --rm ikaros --help
docker compose -f docker/compose.yml run --rm ikaros config show
docker compose -f docker/compose.yml run --rm ikaros tui
```

The image does not bake credentials into layers. Runtime state belongs under
`/data/ikaros`, which maps to the `ikaros-home` volume in the Compose file.
The repository is mounted at `/workspace` for local development workflows.
