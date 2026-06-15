# Context Engine

The context boundary controls what local state is allowed to enter a model turn.
It is not a prompt string builder. It owns structured context sections,
reference parsing, token budgeting, and the diff record that explains why a
turn saw the context it saw.

## Ownership

`ikaros-context` owns the reusable primitives:

- `ContextBundle`
- `ContextSection`
- `ContextReference`
- `ContextBudget`
- `ContextDiff`
- heuristic token estimation

`ikaros-runtime` owns orchestration around those primitives. Runtime chat still
calls harness safe-read skills for memory and RAG, resolves workspace-local
references, renders the final system prompt, and emits the session event.

This split keeps context accounting and replay/debug data reusable without
letting the context crate depend on runtime, harness, or model providers.

## Sections

Current chat context sections are:

- relationship
- references
- history
- memory
- RAG

`system`, `developer`, and `tool_results` section kinds exist as protocol
shapes, but the current chat prompt does not yet use them as independent
budgeted sections.

## Token Budget

Chat context uses `DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET` and a heuristic token
estimator. A budget of `0` means unbounded.

The estimator is intentionally local and deterministic. It is good enough for
MVP context accounting, but it is not a provider-native tokenizer. Provider
registry work should later supply model-specific context windows and tokenizer
adapters.

The budget order currently prioritizes:

1. relationship
2. explicit references
3. history
4. memory
5. RAG

When a line exceeds the remaining budget, it may be truncated with a
`[truncated]` marker. Omitted and truncated content is reported in the context
diff.

## References

The parser recognizes:

```text
@file:path:line-line
@folder:path
@git:rev
@diff
@staged
@url:https://example.test
```

Local reference resolution is workspace-bound:

- `@file` reads a file under the current workspace and can select a line range.
- `@folder` lists direct children under a workspace folder.
- `@git` uses local `git show --stat --oneline`.
- `@diff` uses local `git diff -- .`.
- `@staged` uses local `git diff --cached -- .`.

Paths that escape the workspace fail the turn. Missing local paths also fail
the turn because the user explicitly requested that context.

`@url` is parsed but not fetched. Network-backed context references require a
separate network policy boundary before they become executable.

## Session Events

Every chat turn emits `AgentEventKind::ContextDiff` after context assembly. The
payload includes:

- budget
- sections
- parsed references
- before/after token estimates
- added, removed, and compressed context previews

Consumers should use this event for replay/debug/UI context inspection instead
of parsing the rendered prompt.

## Safety

Context assembly may call safe-read skills with real local inputs, but those
calls use redacted audit inputs. Reference content is redacted before it is put
into the prompt or session event payload.

The context engine does not execute tools, bypass policy, or grant write
permissions. It only prepares read-only information for a model turn.
