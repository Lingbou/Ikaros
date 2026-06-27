# Harness Model

The harness is Ikaros's tool execution boundary. Runtime code, chat loops, agent loops, scheduled
jobs, message drains, coding helpers, and plugin runs all pass through it before mutating local
state or running processes.

The harness should be treated like a small kernel around local effects: callers
submit typed requests, the harness evaluates policy, records the decision, and
then either executes through an environment backend or returns a denial/approval
state. Code outside the harness should not open files, start processes, or send
network requests on behalf of model-selected tools.

## Main Types

- `Skill` and `SkillRegistry`: named operations and prompt-skill documents known to the harness.
- `ToolRegistry`: the executable-tool view derived from `SkillRegistry`; it excludes prompt-skill
  documents from callable tool lookup and provider tool-schema generation.
- `SkillDescriptor`, `PromptSkillDocument`, and `SkillBundle`: separate executable tools from prompt
  skills, carry `toolset` metadata, and support `disable_model_invocation`.
- `ExecutionSession`: workspace, policy, approvals, audit log, agent overlay,
  dry-run state, and `ExecutionEnv`.
- `ExecutionEnv`: execution environment abstraction composed of `FileSystem`,
  `ProcessRunner`, and `NetworkEgress`.
- `PolicyRequest` and `PolicyEvaluation`: the input and result for a policy decision.
- `ApprovalRequest` and `ApprovalLog`: persisted approval workflow.
- `AuditEvent` and `AuditLog`: JSONL event stream for decisions and results.
- `RuntimeTaskPlan`, `ExecutablePlanStep`, and `TaskExecutionReport`: task-runner contracts.
- `GuardrailConfig` and `GuardrailState`: repeated-failure and no-progress observation.

## Caller Context

Each `ExecutionSession` carries:

- workspace root
- audit log path
- resolved agent profile or `AgentInstance` overlay
- dry-run state
- policy engine
- approval queue
- `ExecutionEnv`

The session is the authority used by skills. If a caller needs a different
workspace, agent identity, or environment backend, it must create a new session
instead of mutating global state.

## Execution Flow

1. A caller asks the registry to execute a skill.
2. The skill builds a `PolicyRequest`.
3. `ExecutionSession` records a `tool_call` audit event.
4. The policy engine returns allow, ask, or deny.
5. Allowed work executes through `ExecutionEnv` unless the session is in dry-run mode.
6. Ask results create an approval request.
7. Deny results return without execution.
8. The result is written to audit.

Safe-read skills may pass redacted audit input while executing with the real local input. Chat uses
this for local memory/RAG lookup so the audit log does not store full user prompts.

The same boundary emits structured `tracing` events for policy decisions, tool
start/completion/failure, approval decisions and approval replay, process
execution, and governed network egress. Trace fields are metadata only:
tool/approval/call ids, skill names, risk labels, decisions, command names,
argument counts, byte counts, status codes, and redacted error text. Request
inputs, process stdout/stderr, network headers, and network bodies are not
written into tracing events.

Coding has two harness paths. `code_workflow` is the controlled turn workflow:
by default it is a safe-read plan/review path, but its policy risk upgrades to
shell-read when `run_tests` is requested in `test`/`edit`, and to local-write
only when an explicit candidate patch is applied in `edit` mode. `self_modify`
is rejected by ordinary `code_workflow` until it enters the dedicated
self-modify approval path. `--model-loop` always contacts the configured model
provider after approval. The policy request still has one effective risk label:
model-only loops are network risk, test loops are shell-read risk, and patch
application is local-write risk. To avoid hiding combined risk, approval
requests also carry structured context for provider calls, workspace writes,
shell/test commands, session and turn identity, candidate diff size, and replay
instructions. The CLI renders that context as `approval_scope`, and successful
or replayed coding turns render `coding_progress` and `coding_result` summaries
without requiring `debug coding-turn` JSON for the common path. The workflow
builds the repo map, change plan, optional patch attempt, turn diff,
test-matrix evidence, review, iteration plan, loop report, final report, and
optional session replay evidence. Patch application in `code_workflow` and
`code_edit_guarded` both go through the session `ExecutionEnv` filesystem
interface rather than calling host filesystem APIs from the skill. Approved
`code_workflow` replay uses the coding registry again, so provider-backed loops
keep their session id, turn id, provider, budget, cancellation, and event
persistence boundary. Provider-backed coding loops also observe cancellation
while waiting for the provider response; cancellation records a
`coding_loop_cancelled` event and stops before later patch/test/review work.
`code_edit_guarded` remains the direct approval-gated patch entry point for
applying a provided unified diff.

Skill descriptors also carry runtime scheduling metadata:

- `execution_mode = parallel`: eligible to run in the same batch as adjacent
  parallel calls from one model response.
- `execution_mode = sequential`: must run alone and preserve strict ordering.
- `timeout_ms`: optional per-tool runtime timeout. Timeout returns a failed tool
  result and is reflected in lifecycle events.

Safe read and shell read descriptors default to `parallel`. Write, network,
remote, destructive, secret, and self-modification risk descriptors default to
`sequential`. The model provider sees only the callable tool schema; scheduling
metadata belongs to the runtime/harness boundary.

Skill descriptors also carry `toolset`. Agent profiles choose the enabled
toolsets. The direct model-visible surface is limited to enabled
`core`, `workspace`, and `memory` tools, plus the bridge tools. `rag`, `coding`,
`voice`, and `plugin` stay deferred even when enabled, so a model can discover
and invoke them without every RAG/coding/voice/plugin schema being injected into
every turn. The bridge respects the active agent profile's toolset selection: a
deferred tool outside that selection is not searchable, describable, or callable
through `tool_call`. Disclosure is scoped to the current `ExecutionSession`:
`tool_search` requires a non-empty query and discloses only the returned
deferred descriptors, `tool_describe` discloses the named descriptor, and
`tool_call` rejects any deferred tool that has not been disclosed by one of
those two paths in the same session. `tool_call`
delegates to the target skill through `ExecutionSession`, so the target tool
still gets its own policy decision, approval request, audit events, and
`ExecutionEnv` execution. Audit logs therefore show the bridge call, a
`deferred_tool_invocation` linkage event with target descriptor metadata, and
the underlying deferred tool call/result.

Prompt skills are intentionally different from executable tools. A prompt skill
is registered as a `PromptSkillDocument` with instructions and descriptor
metadata, but without an executable `Skill` implementation. It never enters the
provider tool schema and `tool_call` always rejects it as non-executable. The
only model-facing path is progressive disclosure: `tool_search` can return its
descriptor metadata when the active toolset allows it, including provenance and
the safe relative support-file list, and `tool_describe` can return the
instruction document plus the safe support-file contents after applying the
same secret redaction used for other model-visible payloads. `tool_search`
never returns the instruction body or support-file contents. This keeps
document-style guidance and support files out of the callable tool namespace
while still letting a turn load them explicitly.
Local prompt skill documents are discovered from
`IKAROS_HOME/skills/<name>/SKILL.md`. The directory name becomes the skill name.
The document may start with a small front matter block containing
`description`, `toolset`, `provenance`, and `support_files`; the body is the
instruction text. `support_files` may list relative files under the same skill
directory. Parent-directory and absolute paths are ignored. Safe support files
are not returned by `tool_search`, but `tool_describe` loads them on demand,
redacts secret-like content, and includes a truncation flag when a file is too
large for the prompt-skill payload.
These documents are still prompt skills, not plugins: they are searchable and
describable through the bridge, but never executable.

## Policy Decisions

Policy returns one of three effective states:

- `allow`: execute through `ExecutionEnv` and record the result.
- `ask`: persist an approval request and return without executing.
- `deny`: return without executing.

Profile overlays may narrow or ask for ordinary writes, shell, and network
operations. Hard denials still win over profile settings. Examples include
destructive commands, protected paths, direct secret access, publishing actions,
workspace-external writes, and ordinary self-modification.

Approval replay is not a generic capability token. It must match the original
workspace, skill, risk, input, and agent identity that were approved.

## ExecutionEnv

`ExecutionEnv` narrows host operations into three interfaces:

- `FileSystem`: read path metadata, read/write text and bytes, create directories, list directories.
- `ProcessRunner`: run structured process requests.
- `NetworkEgress`: network egress requests.

The default session environment is `WorkspaceExecutionEnv`, a scoped wrapper
around the local backend. `LocalExecutionEnv` remains the raw host backend used
by tests and future environment implementations; normal runtime sessions should
not attach it directly unless they intentionally want to bypass workspace
scoping.

Normal runtime sessions receive their environment from `ikaros-host`.
`ikaros-host` composes the configured sandbox backend, workspace scope, dry-run
wrapper, Docker process backend, and governed network egress into the
`ExecutionEnv` attached to the session. The harness still owns the execution
interfaces and policy decisions; host assembly owns config-driven wiring.

`WorkspaceExecutionEnv` resolves relative paths against the session workspace.
Filesystem reads, writes, byte reads/writes, directory listing/creation, file
removal, and process working directories must stay under the workspace root. The
scope check uses both lexical normalization and canonical existing-path anchors,
so `..` paths and symlink escapes cannot turn an approved workspace operation
into an external host read or write. Skill policy still decides whether a read is
allowed, but the env layer now enforces the workspace boundary for existing
paths. On Unix local file writes also open the final path with no-follow flags,
so a symlink swap between the workspace scope check and the write is rejected
instead of following the replaced path outside the workspace. Filesystem skills,
shell commands, coding helpers, RAG maintenance, voice output, voice ASR audio
reads, self-modify workspace reads/writes/checks, and command-backed plugins
should use session/env instead of calling host APIs directly.

`ProcessRequest` has two modes:

- `program`: executes a program with an argument vector.
- `shell`: executes through the platform shell.

Model-facing skills should prefer `program`. `shell` is reserved for internal
adapters that already performed allowlist validation. The local backend captures
stdout/stderr, supports optional stdin, supports a timeout, and can reject output
that exceeds `max_output_bytes`. Timeouts and output-cap failures terminate the
spawned process group on Unix before returning the structured error.
Process execution clears the ambient host environment before spawn, restores
only a small platform baseline such as `PATH`/home/temp/system-root variables,
and then applies explicit `ProcessRequest.env` entries. `ProcessRequest` debug
output redacts sensitive environment variable names and secret-like values, so
diagnostics can show which env keys were requested without leaking credentials.

Command-backed plugins run with an explicit plugin cwd scope. Their manifest
program must canonicalize under the plugin directory, shell execution is
rejected for that scope, and the command cwd is the plugin root rather than the
user workspace. Ordinary workspace commands still use the default workspace cwd
scope.

`NetworkEgress` is part of the interface. Runtime sessions compose the
workspace-scoped filesystem/process backend with `GovernedNetworkEgress` and
`HttpNetworkEgress`. The governed wrapper is deny-by-default, allows only exact
parsed URL hosts from the allowlist built by `ikaros-host`, permits only `http`
and `https` schemes, and redacts denied host summaries. The HTTP transport
resolves the allowed host, rejects restricted resolved addresses, disables
redirects, and pins the verified socket addresses into the request client so the
transport does not perform a second independent DNS lookup for the same request.
Provider HTTP adapters receive an egress-backed transport in chat, task agent-loop, and
provider-backed coding paths. Provider-backed RAG embedding skills also use the
session environment for OpenAI-compatible and Ollama embedding HTTP after the
approval request is accepted. Arbitrary plugin or shell code should not bypass
the harness to make network requests; model-facing shell commands remain
limited to structured test/check allowlists. `NetworkEgressRequest` and
`NetworkEgressResponse` preserve raw headers and bodies for transport parsing,
but their diagnostic `Debug` output redacts secret-like URL, header, and body
values before they can enter logs or error reports.

`execution.sandbox.backend: dry-run` installs a dry-run backend for filesystem
and process side-effect operations. It preserves workspace reads but skips
writes and process execution with structured dry-run output. `docker` installs
the first process-container backend: process requests are translated to
`docker run --rm --network none`, the workspace is mounted at `/workspace`, and
the configured `execution.sandbox.image` supplies tools such as Cargo, Git, npm,
or pytest. Network egress is controlled separately by
`execution.network.enabled` and the governed allowlist; disable
`execution.network.enabled` when a dry-run session must also avoid network
effects. Docker process execution is a useful local isolation layer, but it is
not a complete seccomp, VM, or multi-tenant sandbox.

The current sandbox surface also exposes a diagnostic isolation matrix and a
local sandbox debug report. Available levels are `dry_run`, `workspace_only`,
`network_restricted`, and the Docker-backed `container` first slice; raw no-op
host execution is not exposed as a normal runtime backend. This report is a
debug/UX contract for explaining cwd scope, env allowlisting, timeout/output
caps, the process timeout strategy, file-write scope, and governed network
egress. It is not a complete process namespace, seccomp, or VM boundary.

## Shell and Plugins

- `shell_guarded` no longer executes arbitrary shell strings; it accepts only allowlisted test/check
  commands and runs them as program + args.
- `git_status` and `git_diff` are fixed structured commands.
- `run_tests` reuses the same test/check allowlist.
- Command-backed plugins do not execute through a shell; manifest `program` must be relative and
  must canonicalize under the plugin directory. The resolved program is executed with the plugin
  directory as cwd under an explicit plugin cwd scope, while policy/audit still come from the
  session.
- Plugin manifests reject abnormal timeouts, too many args, oversized args, and control characters.
- Plugin runtime limits stdin, stdout/stderr, and timeout, then redacts output
  before audit/reporting.

Plugin command execution has two boundaries:

- Catalog validation decides whether the manifest is loadable.
- Harness policy decides whether a specific invocation may run.

A valid manifest does not imply permission to execute. The declared risk is
advisory input to policy and audit, not an override.

## Task Runner

The task runner executes ordered skill steps. It handles:

- per-step status
- retries for transient failures
- timeouts
- cancellation
- approval waits
- guardrail warnings or halts
- final task reports

`task run --dry-run` uses the same path with dry-run enabled. `task run --agent-loop` lets the model
choose harness skills, but dispatch still goes through `ExecutionSession`.

## Audit Rules

Audit events should explain decisions without storing unnecessary sensitive
content. Safe-read chat context lookups may execute with real local input while
recording redacted audit input. Command output and plugin output are redacted
before reporting. Provider usage logs should record provider, model, time, and
token counts, not prompts.

## Extension Rules

New skills should document:

- risk level
- policy inputs
- required workspace relation for paths
- whether the skill is safe-read
- whether it can call providers or network
- what data is written to audit
- dry-run behavior

New environment backends must implement file, process, and network semantics
consistently enough that existing skills do not need backend-specific branches.
