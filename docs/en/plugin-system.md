# Plugin System

Plugins are local manifests under:

```text
IKAROS_HOME/skills
```

The plugin system is conservative. Discovery and validation are read-only. Command-backed plugin execution only happens through the harness.

## Manifest Locations

Supported:

- `IKAROS_HOME/skills/<plugin>/plugin.toml`
- `IKAROS_HOME/skills/<plugin>.toml`
- optional marketplace metadata at `IKAROS_HOME/skills/marketplace.toml`

## Example Manifest

```toml
name = "hello"
version = "0.1.0"
description = "Example plugin."

[[skills]]
name = "echo"
description = "Echo JSON input."
risk = "safe_read"
input_schema = { type = "object", properties = { message = { type = "string" } } }

[skills.command]
program = "bin/echo.sh"
args = ["--json"]
timeout_ms = 1000
```

## CLI

```bash
ikaros skill list
ikaros skill audit
ikaros skill validate ./plugins/hello
ikaros skill install ./plugins/hello
ikaros skill inspect hello.echo
ikaros skill enable hello
ikaros skill disable hello
ikaros skill run hello.echo --input-json '{"message":"hello"}'
ikaros skill uninstall hello
```

## Rules

- Invalid manifests are reported as warnings and are not loaded.
- Plugin and skill names use simple ASCII identifiers.
- Commands are relative to the plugin manifest directory.
- Commands must not use `..` or protected reference-material paths.
- Installed plugins are disabled by default unless explicitly enabled.
- Declaration-only skills can be inspected but not executed.
- Command-backed skills run through `plugin_command_run`.
- Plugin input and output are redacted before audit/reporting.
- The declared risk still goes through policy.

The CLI never executes plugin code directly.
