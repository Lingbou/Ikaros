# Service Manager Templates

Ikaros can render service-manager templates for local worker processes. Rendering does not install,
enable, reload, or start services.

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
- positional workspace path
- optional `--agent`
- worker interval and limit
- webhook host and port

Systemd `message-worker` templates keep `ExecStart` as the foreground
`message worker` process so the service manager owns the process. They also add
an `ExecStop` hook that runs `message worker-stop --reason "service manager
stop"` and gives the worker a short cooperative shutdown window, allowing the
gateway worker to consume `message-worker.stop` and write shutdown forensics.

Templates do not include API keys. Worker-triggered provider calls read the
selected local `IKAROS_HOME/config.yaml`, including `model.default` and shared
`providers.*` resource settings.
