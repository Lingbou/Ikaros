# API Reference

The Rust crate APIs are pre-release. The supported user surface is the `ikaros` CLI.

Use generated Rust docs for crate-level details:

```bash
cargo doc --workspace --all-features --no-deps
```

## Common Commands

Initialize and inspect local state:

```bash
ikaros init
ikaros doctor
```

Chat:

```bash
ikaros chat
ikaros chat --message "hello"
ikaros chat --stream --message "hello"
ikaros chat --context-token-budget 4000 --message "summarize @file:docs/en/architecture.md:1-80"
ikaros chat --sessions
ikaros chat --history
ikaros chat --history-search "query"
```

Running `ikaros chat` without `--message` starts the current interactive chat
REPL. It supports slash commands such as `/help`, `/agents`, `/agent <profile>`,
`/status`, and `/quit`.

Chat messages may include local context references such as `@file:path:line-line`,
`@folder:path`, `@git:rev`, `@diff`, and `@staged`. These references are
resolved under the current workspace and recorded in the session context diff.
`@url:` is parsed but not fetched.

`--context-token-budget 0` asks runtime chat to use the provider-derived
available context window. It does not bypass the model context window.
The persisted context diff records the selected token estimator adapter, such as
OpenAI-compatible, mock, or an explicit Anthropic/Ollama fallback.

Debug persisted session evidence:

```bash
ikaros debug context-diff <session-id>
ikaros debug context-diff <session-id> --turn-id <turn-id>
ikaros debug memory-lifecycle <session-id>
ikaros debug memory-lifecycle <session-id> --turn-id <turn-id>
ikaros debug continuations <session-id>
ikaros debug continuations <session-id> --turn-id <turn-id>
ikaros debug coding-turn <session-id>
ikaros debug coding-turn <session-id> --turn-id <turn-id>
```

`context-diff` reads `state.db` and reports the estimator, budget, context
window, section token estimates, added/removed/compressed context, parsed
references, compaction summary, continuation prompt, `ContextCompacted`, and
context-limit errors. `memory-lifecycle` reads the session timeline and
`memory_journal.jsonl` for matching `MemoryLifecycle` events,
`MemoryRef::SessionTurn` links, skipped writes, redaction-related notes, action
counts, and runtime memory policy actions. `continuations` reports durable
continuation queue status, status reason, lease owner, lease expiry, attempt
count, terminal summaries, worker-lease timeout evidence, errors, and redacted
payload data for queued/running/completed/failed/cancelled items. Filtering by
`--turn-id` returns an empty result when the turn exists but has no
continuations; it only errors when the turn is absent from the replay.
`coding-turn` reports persisted `CodingTurn` events, coding event-kind counts,
review findings, and custom session entries produced by `ikaros code workflow`.

Memory and relationship notes:

```bash
ikaros memory add "note" --kind project --scope ikaros
ikaros memory add --kind relationship --scope default --observer alice --subject bob "Bob likes pancakes"
ikaros memory search "query"
ikaros memory update <id> --content "new note"
ikaros memory delete --id <id>
ikaros memory projection render --scope ikaros
ikaros memory projection show --scope ikaros
ikaros memory candidate list
ikaros memory candidate accept <candidate-id> --reason "explicit user instruction"
ikaros memory candidate accept <candidate-id> --supersedes <memory-id> --reason "user corrected this"
ikaros memory candidate reject <candidate-id> --reason "temporary task scope"
ikaros memory working list --session <session-id>
ikaros memory working prune
ikaros relationship remember "preference" --scope user
ikaros relationship show --scope user
```

Runtime chat writes safe turn state into session working memory, not long-term
`Task` memory. Automatic relationship observations are pending candidates until
accepted. Projection commands render the accepted long-term memory surface used
by chat context.

RAG:

```bash
ikaros rag ingest docs --scope project
ikaros rag search "harness policy"
ikaros rag stale
ikaros rag reindex docs --scope project
ikaros rag delete-path docs/old.md
ikaros rag delete-scope scratch
```

When RAG uses a cloud embedding provider, `ingest`, `reindex`, and `search` may
return an approval id before the provider call. Run `ikaros approval approve
<approval-id>` to execute the original approved request.

Tasks and agents:

```bash
ikaros task run "summarize the repository" --dry-run
ikaros task run "inspect runtime" --agent-loop
ikaros agent list
ikaros agent show plan
ikaros agent run --profile plan --dry-run "inspect docs"
ikaros agent batch --profile plan --task "inspect docs" --task "inspect runtime"
```

Policy and approvals:

```bash
ikaros policy explain write_note --risk local-write --path note.txt --write
ikaros approval list
ikaros approval approve <approval-id>
ikaros approval deny <approval-id>
```

Gateway and schedules:

```bash
ikaros schedule add "summarize status" --at now
ikaros schedule add "summarize status" --at now --delivery gateway-outbox
ikaros schedule run-due --dry-run
ikaros schedule worker --once
ikaros message send "hello" --kind chat
ikaros message drain --dry-run
ikaros message webhook --port 8002
```

Voice and body surfaces:

```bash
ikaros voice tts "hello" --output speech.wav
ikaros voice asr input.wav --language en
ikaros body status
ikaros body dashboard
ikaros body dashboard --refresh-seconds 5 --snapshot-output previews/frame.json
ikaros body serve --port 8001
```

Cloud TTS and ASR calls follow the same approval flow. TTS output renders byte
length and optional file path, not raw audio bytes.

Local filesystem and git helpers:

```bash
ikaros fs read README.md
ikaros fs list docs
ikaros fs write notes/example.txt "local note"
ikaros git status
ikaros git diff --stat
```

Plugins:

```bash
ikaros skill list
ikaros skill audit
ikaros skill validate ./plugins/example
ikaros skill install ./plugins/example
ikaros skill inspect example.tool
ikaros skill run example.tool --input-json '{"message":"hello"}'
```

Coding helpers:

```bash
ikaros repo scan
ikaros test infer
ikaros test run --command "cargo test"
ikaros code plan "add focused tests"
ikaros code workflow "prepare guarded patch" --diff "<unified diff>"
ikaros code workflow "apply candidate patch" --mode edit --diff "<unified diff>" --apply-patch
ikaros code workflow "run focused tests" --mode test --run-tests --test-command "cargo test -p ikaros-coding"
ikaros code workflow "persist replay evidence" --session-id <session-id> --turn-id <turn-id>
ikaros code review
ikaros code iterate
ikaros code guarded-edit "apply approved patch" --diff "<unified diff>"
```

`code workflow` builds a `CodingTurnContext`, repo map, change plan, optional
patch attempt, turn diff, test-matrix evidence, review, iteration plan, loop
report, and final report. It supports `--mode plan|edit|review|test|self_modify`.
The mode policy is explicit: `plan`/`review` are read-oriented, `test` can run
the test matrix, `edit` can apply a candidate patch only with `--apply-patch`,
and `self_modify` is rejected by ordinary workflow until the dedicated
self-modify approval path is used. The context captures a git baseline including
HEAD, branch/detached state, clean/dirty/not-git/unknown state, and
staged/unstaged/untracked flags when available. When session and turn ids are
present, coding events are persisted into `state.db` and can be inspected with
`ikaros debug coding-turn`.

Service-manager templates:

```bash
ikaros service render --kind schedule-worker --manager systemd
ikaros service render --kind message-worker --manager systemd --output services/ikaros-message-worker.service
ikaros service render --kind message-webhook --manager launchd
```

Self-modify:

```bash
ikaros self-modify propose --kind documentation-patch --target README.md --diff "<unified diff>"
ikaros self-modify request-apply <proposal-id>
ikaros self-modify apply-approved <proposal-id> --approval-id <approval-id>
ikaros self-modify rollback <proposal-id>
```

## Global Options

`--ikaros-home <path>` selects local state.

`--agent <profile>` selects the active profile for commands that create a harness session. It can be placed before or after the subcommand:

```bash
ikaros --agent plan chat --message "read only"
ikaros chat --agent plan --message "read only"
```

## Compatibility

CLI output is primarily human-readable. Automation should depend on structured report fields that are covered by tests.

After upgrading Ikaros, rerun the relevant validation commands to confirm that the fields you depend on still match expectations.
