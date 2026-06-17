# Harness Model

The harness is Ikaros's tool execution boundary. Runtime code, chat loops, agent loops, scheduled jobs, message drains, coding helpers, and plugin runs all pass through it before mutating local state or running processes.

The harness should be treated like a small kernel around local effects: callers
submit typed requests, the harness evaluates policy, records the decision, and
then either executes through an environment backend or returns a denial/approval
state. Code outside the harness should not open files, start processes, or send
network requests on behalf of model-selected tools.

## Main Types

- `Skill` and `SkillRegistry`: named operations that can be executed.
- `SkillDescriptor` and `SkillBundle`: separate executable tools from prompt skills and support `disable_model_invocation`.
- `ExecutionSession`: workspace, policy, approvals, audit log, agent overlay, dry-run state, and `ExecutionEnv`.
- `ExecutionEnv`: execution environment abstraction composed of `FileSystem`, `ProcessRunner`, and `NetworkEgress`.
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

Safe-read skills may pass redacted audit input while executing with the real local input. Chat uses this for local memory/RAG lookup so the audit log does not store full user prompts.

Coding has two separate harness paths. `code_workflow` is a safe-read control
plane that assembles the repo map, change plan, candidate diff review, test
evidence, iteration plan, and final report. It does not write files. Actual
patch application still goes through `code_edit_guarded`, which is
approval-gated and must match approval replay rules before modifying the
workspace. Approved guarded patches read, write, create parent directories, and
roll back through the session `ExecutionEnv` filesystem interface rather than
calling host filesystem APIs from the skill.

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

`WorkspaceExecutionEnv` resolves relative paths against the session workspace.
Filesystem writes, byte writes, directory creation, file removal, and process
working directories must stay under the workspace root. The scope check uses both
lexical normalization and canonical existing-path anchors, so `..` paths and
symlink escapes cannot turn an approved workspace operation into an external
host write. Read APIs also resolve relative paths from the workspace, but read
authorization still belongs to the skill policy or reference resolver; the
environment wrapper alone should not be treated as a complete read sandbox.
Filesystem skills, shell commands, coding helpers, RAG maintenance,
voice output, voice ASR audio reads, self-modify workspace reads/writes/checks,
and command-backed plugins should use session/env instead of calling host APIs
directly.

`ProcessRequest` has two modes:

- `program`: executes a program with an argument vector.
- `shell`: executes through the platform shell.

Model-facing skills should prefer `program`. `shell` is reserved for internal
adapters that already performed allowlist validation. The local backend captures
stdout/stderr, supports optional stdin, supports a timeout, and can reject output
that exceeds `max_output_bytes`. A timeout attempts to kill the child before
returning `command timed out`.

`NetworkEgress` is part of the interface, but the local backend does not provide
a network implementation. Provider calls that need network access are handled by
their provider adapters after policy approval, not by arbitrary plugin or shell
code.

## Shell and Plugins

- `shell_guarded` no longer executes arbitrary shell strings; it accepts only allowlisted test/check commands and runs them as program + args.
- `git_status` and `git_diff` are fixed structured commands.
- `run_tests` reuses the same test/check allowlist.
- Command-backed plugins do not execute through a shell; manifest `program` must be relative and must canonicalize under the plugin directory. The resolved program is executed with the session workspace as cwd, not the plugin installation directory.
- Plugin manifests reject abnormal timeouts, too many args, oversized args, and control characters.
- Plugin runtime limits stdin, stdout/stderr, and timeout, then redacts output before audit/reporting.

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

`task run --dry-run` uses the same path with dry-run enabled. `task run --agent-loop` lets the model choose harness skills, but dispatch still goes through `ExecutionSession`.

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
