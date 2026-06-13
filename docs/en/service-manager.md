# Service Manager Templates

Ikaros can render service-manager templates for local worker processes. Rendering does not install, enable, reload, or start services.

## Render Targets

```bash
ikaros service render --kind schedule-worker --manager systemd
ikaros service render --kind message-worker --manager systemd
ikaros service render --kind message-webhook --manager launchd
```

Useful options:

```bash
ikaros service render \
  --kind message-worker \
  --manager systemd \
  --interval-seconds 30 \
  --limit 10 \
  --output services/ikaros-message-worker.service
```

`--output` is limited to paths under `IKAROS_HOME`.

## Template Contents

Templates include local runtime arguments such as:

- `--ikaros-home`
- `--workspace`
- optional `--agent`
- worker interval and limit
- webhook host and port

Templates do not include API keys. Worker-triggered provider calls read
`providers.*` settings from the selected local `IKAROS_HOME/config.yaml`.
