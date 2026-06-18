# Harness 模型

Harness 是 Ikaros 的工具执行边界。Runtime、chat loop、agent loop、计划任务、message drain、coding helper 和 plugin run 在修改本地状态或运行进程前都要经过 harness。

Harness 应被看作本地副作用周围的一层小内核：调用方提交 typed request，harness 做策略判断，记录决策，然后通过环境 backend 执行，或返回拒绝/等待审批状态。Harness 之外的代码不应代表模型选择的工具直接打开文件、启动进程或发送网络请求。

## 主要类型

- `Skill` 和 `SkillRegistry`：可执行的命名操作。
- `SkillDescriptor` 和 `SkillBundle`：区分 executable tool 与 prompt skill，并支持 `disable_model_invocation`。
- `ExecutionSession`：workspace、policy、approval、audit log、agent overlay、dry-run 状态和 `ExecutionEnv`。
- `ExecutionEnv`：由 `FileSystem`、`ProcessRunner`、`NetworkEgress` 组成的执行环境抽象。
- `PolicyRequest` 和 `PolicyEvaluation`：策略输入和结果。
- `ApprovalRequest` 和 `ApprovalLog`：持久化审批流。
- `AuditEvent` 和 `AuditLog`：决策和结果的 JSONL 事件流。
- `RuntimeTaskPlan`、`ExecutablePlanStep` 和 `TaskExecutionReport`：任务 runner 合约。
- `GuardrailConfig` 和 `GuardrailState`：重复失败和无进展观察。

## 调用上下文

每个 `ExecutionSession` 携带：

- workspace root
- audit log path
- 已解析的 agent profile 或 `AgentInstance` overlay
- dry-run 状态
- policy engine
- approval queue
- `ExecutionEnv`

Session 是 skill 使用的权限来源。如果调用方需要不同 workspace、agent identity 或环境 backend，应创建新的 session，而不是修改全局状态。

## 执行流程

1. 调用方请求 registry 执行一个 skill。
2. Skill 构造 `PolicyRequest`。
3. `ExecutionSession` 记录 `tool_call` 审计事件。
4. 策略引擎返回 allow、ask 或 deny。
5. Allow 结果通过 `ExecutionEnv` 执行，除非 session 处于 dry-run。
6. Ask 结果创建审批请求。
7. Deny 结果直接返回，不执行。
8. 结果写入审计。

Safe-read skill 可以使用脱敏审计输入，同时用真实本地输入执行。Chat 用这个能力做本地 memory/RAG 查询，避免审计日志保存完整用户 prompt。

Coding 有两条 harness 路径。`code_workflow` 是受控 turn workflow：默认是 safe-read
的 plan/review 路径；在 `test`/`edit` 中请求 `run_tests` 时策略风险升级为
shell-read；只有在 `edit` mode 明确应用候选 patch 时才升级为 local-write。
`self_modify` 在进入专用 self-modify 审批路径前会被普通 `code_workflow` 拒绝。
`--model-loop` 在审批后一定会调用配置的 model provider。当前 policy request 仍只携带一个
risk label：纯模型 loop 是 network risk，test loop 是 shell-read risk，patch 应用是
local-write risk；provider + shell/write 的组合审批展示仍是 hardening 项。它会组装 repo map、change plan、可选 patch attempt、
turn diff、test-matrix evidence、review、iteration plan、loop report、final report
和可选 session replay evidence。`code_workflow` 和 `code_edit_guarded` 里的 patch
写入都通过 session `ExecutionEnv` filesystem interface 执行，而不是在 skill 内直接调用
host filesystem API。已批准的 `code_workflow` replay 会重新使用 coding registry，
因此 provider-backed loop 会保留 session id、turn id、provider、budget、cancellation
和 event persistence 边界。`code_edit_guarded` 仍是直接应用指定 unified diff 的审批受控入口。

Skill descriptor 也携带 runtime 调度元数据：

- `execution_mode = parallel`：可以和同一模型响应里相邻的 parallel 调用成批执行。
- `execution_mode = sequential`：必须单独执行并保持严格顺序。
- `timeout_ms`：可选的 per-tool runtime timeout。超时会返回 failed tool result，并写进 lifecycle event。

Safe-read 和 shell-read descriptor 默认 `parallel`。write、network、remote、
destructive、secret 和 self-modify 风险 descriptor 默认 `sequential`。模型 provider
只看到可调用 tool schema；调度元数据属于 runtime/harness 边界。

## 策略决策

策略有效结果有三类：

- `allow`：通过 `ExecutionEnv` 执行，并记录结果。
- `ask`：持久化审批请求，不执行。
- `deny`：直接返回，不执行。

Profile overlay 可以收紧或请求普通写入、shell 和 network 操作。硬性拒绝仍然优先于 profile 配置，例如破坏性命令、受保护路径、直接 secret 访问、发布动作、workspace 外写入和普通自修改。

审批重放不是通用 capability token。它必须匹配原始审批的 workspace、skill、risk、input 和 agent identity。

## ExecutionEnv

`ExecutionEnv` 把 host 操作收敛成三个接口：

- `FileSystem`：读取 path metadata、读写文本和二进制文件、创建目录、列目录。
- `ProcessRunner`：运行结构化进程请求。
- `NetworkEgress`：网络出口请求。

默认 session 环境是 `WorkspaceExecutionEnv`，它在本地 backend 外面加 workspace
scope。`LocalExecutionEnv` 仍然是原始 host backend，主要给测试和后续环境实现使用；
普通 runtime session 不应直接挂它，除非调用方明确要绕过 workspace scope。

`WorkspaceExecutionEnv` 会把相对路径解析到 session workspace。文件写入、二进制
写入、创建目录、删除文件和进程 cwd 都必须留在 workspace root 下。Scope 检查同时做
lexical normalize 和已有路径的 canonical anchor 校验，因此 `..` 路径和 symlink
逃逸不能把一次获批的 workspace 操作变成外部 host 写入。读取 API 也会从 workspace
解析相对路径，但读取授权仍属于 skill policy 或 reference resolver；不能把
environment wrapper 本身当作完整 read sandbox。文件、shell、coding、RAG maintenance、
voice output、voice ASR 音频读取、self-modify 的 workspace 读写/check 和
command-backed plugin 都应走 session/env，不应直接调用 host API。

`ProcessRequest` 有两种模式：

- `program`：用 program + args 执行。
- `shell`：通过平台 shell 执行。

面向模型的 skill 应优先使用 `program`。`shell` 只用于已经完成 allowlist 校验的内部 adapter。本地 backend 会捕获 stdout/stderr，支持可选 stdin、timeout，并能在输出超过 `max_output_bytes` 时拒绝。Timeout 会尝试 kill 子进程，然后返回 `command timed out`。

`NetworkEgress` 是接口的一部分，但本地 backend 不提供网络实现。需要网络的 provider 调用在策略审批后由 provider adapter 处理，不应由任意 plugin 或 shell 代码直接处理。

## Shell 和 Plugin

- `shell_guarded` 不再执行任意 shell 字符串；它只接受 allowlisted test/check command，并解析成 program + args 执行。
- `git_status` 和 `git_diff` 是固定结构化命令。
- `run_tests` 复用同一套 test/check allowlist。
- Command-backed plugin 不通过 shell 执行；manifest 的 `program` 必须是相对路径，canonicalize 后仍在 plugin 目录内。解析后的程序会以 session workspace 作为 cwd 运行，而不是以 plugin 安装目录作为 cwd。
- Plugin manifest 会拒绝异常 timeout、过多参数、超长参数和控制字符。
- Plugin runtime 会限制 stdin、stdout/stderr 和 timeout，并在输出审计前脱敏。

Plugin command execution 有两层边界：

- Catalog validation 判断 manifest 是否可加载。
- Harness policy 判断某次 invocation 是否可执行。

Manifest 合法不代表拥有执行权限。声明的 risk 是给 policy 和 audit 的输入，不是覆盖规则。

## 任务 Runner

任务 runner 执行有序 skill 步骤。它处理：

- 每步状态
- 瞬时失败重试
- 超时
- 取消
- 等待审批
- guardrail warning 或 halt
- 最终任务报告

`task run --dry-run` 使用相同路径并启用 dry-run。`task run --agent-loop` 允许模型选择 harness skill，但 dispatch 仍经过 `ExecutionSession`。

## 审计规则

Audit event 应解释决策，但不保存不必要的敏感内容。Safe-read chat context lookup 可以用真实本地输入执行，同时写入脱敏后的 audit input。Command output 和 plugin output 在报告前会脱敏。Provider usage log 应记录 provider、model、时间和 token count，不记录 prompt。

## 扩展规则

新增 skill 应说明：

- risk level
- policy input
- path 是否必须在 workspace 内
- 是否 safe-read
- 是否会调用 provider 或 network
- audit 写入的数据
- dry-run 行为

新增环境 backend 必须实现一致的 file、process 和 network 语义，使已有 skill 不需要 backend-specific 分支。
