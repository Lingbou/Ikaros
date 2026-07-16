# Ikaros

Ikaros is a small terminal-first local agent. It has one runtime path:

```text
terminal input
  -> OpenAI-compatible chat completion
  -> explicit tool approval
  -> workspace file or host-process tool
  -> SQLite transcript
  -> terminal output
```

The project intentionally made a clean break from its pre-MVP architecture.
There is no migration layer for old commands, config files, databases, plugins,
memory, RAG, personas, MCP, or coding-workflow abstractions.

## Current scope

- Full-screen terminal chat UI.
- One OpenAI-compatible `/chat/completions` provider.
- Native model tool calls.
- `list_dir`, `read_file`, `write_file`, and `run_command`.
- Explicit approval before every tool call.
- SQLite-backed sessions that resume after restart.
- Durable pending approvals and fail-closed recovery for interrupted tools.

## Build

Ikaros requires Rust 1.85 or newer.

```bash
cargo build
```

## Configure

Create `~/.ikaros/config.toml`:

```powershell
$env:IKAROS_API_KEY = "your-key"
cargo run -- init --model gpt-4.1-mini
```

Custom OpenAI-compatible endpoint:

```powershell
cargo run -- init `
  --base-url https://provider.example/v1 `
  --model provider-model-id
```

Ikaros deliberately has no command-line or config-file API-key field. Keys are
read only from the environment so they do not enter shell arguments, local
configuration, or the session database. Resolution order is:

1. `IKAROS_API_KEY`
2. `OPENAI_API_KEY`

`IKAROS_BASE_URL`, `IKAROS_MODEL`, and `IKAROS_HOME` can override the matching
local settings.

## Run

Open the terminal UI in the current workspace:

```bash
cargo run --
```

Use another workspace:

```bash
cargo run -- --workspace C:\Workspace\project
```

Send one message without opening the full-screen UI:

```bash
cargo run -- chat "inspect this repository"
```

Useful commands:

```bash
cargo run -- doctor
cargo run -- sessions
cargo run -- --session <ID>
```

Terminal UI commands:

- `/new`
- `/sessions`
- `/resume <ID>`
- `/session`
- `/help`
- `/quit`

## Tool security boundary

Every tool call shows its exact persisted name and arguments. Approval defaults
to deny, there is no automatic-approval flag, and non-interactive input cannot
approve a tool. File tools accept only relative workspace paths and reject
`..`, resolved symlink/junction escapes, and symbolic-link replacement. File
content is installed atomically; existing Unix file modes are preserved and new
files respect the process umask. These checks are a model-tool boundary, not
protection from another process running as the same user and racing filesystem
mutations.

`run_command` starts a program directly with an argument array and a fixed
workspace working directory. It has a timeout and output cap, and common model
API-key environment variables are removed by clearing the environment and
restoring only basic process variables such as `PATH`, the user home, and temp
directories. It is still a host process, not an OS sandbox; an approved program
may access resources available to the user. Commands run in a Windows Job
Object or Unix process group so timeout and output-limit termination applies to
the whole group. OS interrupt and termination signals that reach the runtime
cancel it before exit, which also tears down its process group. The full-screen
UI uses raw input, so its Ctrl-C key is handled between synchronous operations,
not while an active request is awaiting completion. Provider requests and
`run_command` are each capped at 60 seconds.

If Ikaros exits while a tool is executing, the invocation is marked as
interrupted on restart. It is never replayed automatically because its side
effects may already have happened. A hard kill or power loss cannot run cleanup
code on Unix, so recovery still treats any interrupted command as having unknown
side effects even though normal cancellation kills the process group.

Only one Ikaros process may use a given home directory at a time. The process
lock is released automatically on exit or crash; this keeps provider turns and
tool recovery single-owner without a second distributed runtime.

## Data

New state is stored in:

```text
~/.ikaros/config.toml
~/.ikaros/sessions.sqlite3
~/.ikaros/runtime.lock
```

The database stores the canonical conversation and tool lifecycle. Provider
API keys are never written to it.

## Development checks

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
```

## Deliberate non-goals

The first usable core does not include memory, RAG, personas, MCP, plugins,
subagents, browser/API servers, voice, images, automation, streaming, provider
fallback pools, or compatibility with the removed pre-MVP system.
