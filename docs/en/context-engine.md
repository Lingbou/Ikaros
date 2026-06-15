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
- provider-aware token estimator adapters
- `PriorityContextEngine`
- `TrajectoryCompressor`

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

Chat context starts with `DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET`, then caps it with
provider metadata when a model provider is present. `ModelContextProfile`
supplies:

- context window
- default output token reservation
- tokenizer kind
- metadata source

Runtime also reserves tokens for the persona/system part of the prompt before it
assembles local context. The persisted `ContextBudget` records the requested
budget, the effective max tokens, used tokens, provider window, output
reservation, system reservation, estimator, and metadata source.

In runtime chat, a requested context budget of `0` means "use the model-derived
available local-context window" when a provider profile is available. Direct
library callers can still construct an unbounded `ContextBudget`, but CLI turns
should not treat `0` as permission to exceed the model window.

The estimator is selected from the provider profile's tokenizer kind. The
current adapters are local and deterministic: OpenAI-compatible models use a
ChatML-oriented estimator, `mock` uses a stable word-count estimator for tests,
and Anthropic/Ollama use explicit fallback heuristic adapters until exact native
tokenizers are added. The persisted budget stores the adapter name so replay and
debug callers can see which accounting path shaped the turn.

## Quotas And Compression

`PriorityContextEngine` allocates the effective context budget by section:

- relationship: 10%
- explicit references: 35%
- history: 20%
- memory: 20%
- RAG: 15%

`TrajectoryCompressor` applies that quota policy and records deterministic
omission summaries for compressed sections. These summaries explain what was
left out; they are not model-generated semantic summaries yet. It does not rely
on single-line truncation as the normal behavior.

Relationship facts and explicit local references are protected boundaries. The
ordinary quota pass may compact history, memory, and RAG around them, but it
does not silently drop those protected sections. If protected context alone
cannot fit the effective budget, assembly fails with a structured context-limit
error instead of sending a misleading partial prompt.

When context is compacted, the compressor also emits a continuation prompt that
tells the model which sections were compacted and that omitted details must not
be invented. During a persisted chat turn, runtime writes both a
`ContextCompacted` event and a `SessionEntryKind::Compaction` entry. The
assistant message is attached after the compaction entry in the session tree.

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
- compressed sections
- compression summary
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
