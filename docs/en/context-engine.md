# Context Engine

The context boundary controls what local state is allowed to enter a model turn.
It owns structured context sections, reference parsing, token budgeting, prompt
sections, and the diff record that explains why a turn saw the context it saw.
The final system prompt is renderer output from these structured sections; UI
and replay code should inspect the sections, not reverse-parse the prompt text.
`PromptBuildReport::system_messages_for_prompt_cache()` splits cache-stable
persona, policy, and tool guidance from dynamic context sections so provider
adapters can mark only the stable prefix for prompt caching. Prompt metadata
also carries a deterministic stable-prefix hash, message count, and token
estimate so replay/debug can prove whether the cacheable prefix changed without
storing prompt text.

## Ownership

`ikaros-context` owns the reusable primitives:

- `ContextBundle`
- `ContextSection`
- `ContextReference`
- `ContextBudget`
- `ContextDiff`
- `PromptBuilder`
- `PromptSection`
- provider-aware token estimator adapters
- `PriorityContextEngine`
- `TrajectoryCompressor`
- `LlmSummaryCompressor`, which prepares a provider summary request and turns
  the redacted provider summary into a budgeted `ContextCompressionReport`

`ikaros-runtime` owns orchestration around those primitives. Runtime chat still
calls harness safe-read skills for memory and RAG, resolves workspace-local
references, adds runtime/tool guidance, renders the final system prompt through
`PromptBuilder`, sends cache-stable and dynamic system prompt layers as
separate model messages where supported, and emits the session event. The
default context engine is `deterministic`; `--context-engine llm-summary`
explicitly enables the provider-backed summary compressor. Unknown engine names
fail fast instead of silently falling back.

This split keeps context accounting and replay/debug data reusable without
letting the context crate depend on runtime, harness, or model providers.

## Sections

Current chat context sections are:

- relationship
- references
- history
- memory projection
- working memory
- retrieved memory
- RAG

`system`, `developer`, and `tool_results` section kinds exist as protocol
shapes, but the current chat prompt does not yet use them as independent
budgeted sections.

Each persisted `ContextSection` also carries a contract:

- `trust_level`
- `source_kind`
- `injection_reason`
- `freshness`
- `scope`

The default mapping is:

- `relationship`: high trust, accepted-memory source, stable freshness, user
  scope, injected for relationship core facts.
- `references`: high trust, explicit-reference source, current freshness,
  workspace scope, injected because the user explicitly referenced it.
- `history`: medium trust, session-history source, recent freshness, session
  scope, injected as recent episode history.
- `memory_projection`: high trust, memory-projection source, stable freshness,
  user scope, injected as accepted memory projection.
- `working_memory`: medium trust, working-memory source, current freshness,
  session scope, injected as session working memory.
- `retrieved_memory`: medium-low trust, retrieved-memory source, retrieved
  freshness, user scope, injected by explicit `memory_search` or
  `--memory-search-limit`.
- `RAG`: medium-low trust, RAG-index source, retrieved freshness, workspace
  scope, injected as explicit reference retrieval.

This contract is intentionally part of the event payload. Replay, debug, and UI
callers can explain whether a visible line came from accepted memory, session
scratchpad/history, explicit local references, or cited RAG snippets.

## Prompt Sections

`ContextSection` records what local context was assembled. `PromptSection`
records how that context, persona text, policy text, compression guidance, and
tool guidance became the actual system prompt.

Each prompt section carries:

- `kind`
- `title`
- `content`
- `source`
- `priority`
- `estimated_tokens`
- `redaction`

Current chat prompt section kinds include persona, policy, relationship,
references, history, memory projection, working memory, retrieved memory, RAG,
context compression, and tool guidance. `content` exists only in the in-memory
render input. Persisted `ContextDiff`, audit, replay, and debug output use
`PromptSectionMetadata`, which contains only kind, title, source, priority,
estimated token count, and redaction state. Secret-like values are redacted
before prompt content reaches the renderer, and full prompt section content is
not stored as session evidence.

Optional local-context sections are emitted only when they contain content.
Empty relationship, reference, history, memory, or RAG inputs remain visible in
context accounting, but they are not rendered as `none` blocks in the system
prompt and do not appear as prompt section metadata.

Ordinary chat does not run long-term memory search by default. The default
memory surface is accepted projection plus session working memory. Retrieved
memory is only populated when a caller opts in with `--memory-search-limit` or a
tool explicitly performs `memory_search`.

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
- memory projection, working memory, and retrieved memory: 20% total
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
the turn because the user explicitly requested that context. Binary or non-UTF8
`@file` targets do not fail the turn; they are represented as a structured
reference notice with the workspace-relative path and byte size. Text `@file`
content is capped before compression so explicit references cannot consume more
than half of the effective local-context token budget; capped references include
a truncation marker explaining the limit.

`@url` is fetched through the session `NetworkEgress` boundary. The governed
egress policy is deny-by-default and only allows exact hosts from
`execution.network.allowed_hosts` or configured provider defaults, and only for
`http` or `https` URLs. Denied hosts or unsupported schemes fail the turn.
Successful responses are still guarded before entering context: missing content
type is accepted for local test transports, but explicit content types must be
plain text, Markdown, JSON, XML, or YAML. HTML and binary responses are
represented as skipped reference notices. Response bodies larger than 64 KiB are
skipped rather than truncated into the prompt, and URL/body text is redacted
before any visible reference notice is emitted.

## Session Events

Every chat turn emits `AgentEventKind::ContextDiff` after context assembly. The
payload includes:

- budget
- sections
- compressed sections
- compression summary
- prompt section metadata
- parsed references
- before/after token estimates
- added, removed, and compressed context previews

Consumers should use this event for replay/debug/UI context inspection instead
of parsing the rendered prompt.

Use the debug CLI to inspect the persisted event payload without reconstructing
the prompt:

```bash
ikaros debug context-diff <session-id>
ikaros debug context-diff <session-id> --turn-id <turn-id>
```

The command fails if the session or requested turn is missing. JSON output is
redacted and includes the estimator, model-derived budget, section token
counts, prompt section source/priority/token metadata, parsed references,
compressed/protected context evidence, compaction summary, continuation prompt,
and any context-limit error event for the turn.

## Safety

Context assembly may call safe-read skills with real local inputs, but those
calls use redacted audit inputs. Reference content is redacted before it is put
into the prompt or session event payload.

The context engine does not execute tools, bypass policy, or grant write
permissions. It only prepares read-only information for a model turn.
