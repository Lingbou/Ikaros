# Plugin System

Plugins are local manifests under:

```text
IKAROS_HOME/skills
```

The plugin system is conservative. Discovery and validation are read-only. Command-backed plugin
execution only happens through the harness.

## Manifest Locations

Supported:

- `IKAROS_HOME/skills/<plugin>/plugin.toml`
- optional marketplace metadata at `IKAROS_HOME/skills/marketplace.toml`

`IKAROS_HOME/skills/<name>/SKILL.md` is a different artifact. It is loaded as a
prompt skill document, not as an executable plugin. The model can discover it
with `tool_search` and read it with `tool_describe` when the active toolset
allows it, but `tool_call` always rejects it as non-executable. Search results
expose descriptor metadata such as provenance and support-file names, not the
instruction body or support-file contents; `tool_describe` loads those contents
on demand and redacts secret-like text before returning them to the model.

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

Non-`safe_read` command-backed skills must declare a matching runtime
permission. A permission matches when `action` is `run`, the skill name, the
qualified skill name, or `plugin:<qualified skill name>`, and when `risk` equals
the skill risk.

```toml
[[skills]]
name = "write-report"
description = "Write a report under the workspace."
risk = "local_write"

[[skills.permissions]]
action = "run"
risk = "local_write"
paths = ["reports/output.md"]

[skills.command]
program = "bin/write-report.sh"
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
ikaros skill quarantine hello --reason "needs operator review"
ikaros skill unquarantine hello
ikaros skill run hello.echo --input-json '{"message":"hello"}'
ikaros skill uninstall hello
```

## Rules

- Invalid manifests are reported as warnings and are not loaded.
- Plugin and skill names use simple ASCII identifiers.
- `SKILL.md` prompt skill documents use the directory name as the skill name and
  may declare `description`, `toolset`, `provenance`, and safe relative
  `support_files` in front matter. Safe support files are loaded only by
  `tool_describe`, not by search results or default prompt injection.
- Commands are relative to the plugin manifest directory.
- Commands must not use `..` or protected reference-material paths.
- Installed plugins are disabled by default unless explicitly enabled.
- Quarantined plugins remain installed and visible to `inspect`/`audit`, but
  their executable skills are hidden from runtime/model invocation until
  `ikaros skill unquarantine <name>` releases them.
- Declaration-only skills can be inspected but not executed.
- Command-backed skills run through `plugin_command_run`.
- Command-backed skills with risk above `safe_read` are rejected unless the
  skill has a matching runtime permission declaration.
- Permission paths must stay under the workspace and must not target `.temp`.
- `network` and `remote_server` plugin skills must use a matching permission
  with `network = true`.
- Plugin input and output are redacted before audit/reporting.
- The declared risk still goes through policy.

The CLI never executes plugin code directly.
