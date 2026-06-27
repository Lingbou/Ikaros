# Harness 模型

Harness 是 Ikaros 的受治理工具调用边界。Runtime、chat loop、agent loop、计划任务、
message drain、coding helper 和 plugin run 在修改本地状态或运行进程前都要经过 harness。

Harness 应被看作本地副作用周围的一层小内核：调用方提交 typed request，harness 做策略判断，记录决策，然后通过环境 backend 执行，或返回拒绝/等待审批状态。
可复用的工具契约在 `ikaros-toolkit`；具体执行环境在 `ikaros-sandbox`。Harness 之外的
代码不应代表模型选择的工具直接打开文件、启动进程或发送网络请求。

## 主要类型

- `Skill` 和 `SkillRegistry`：来自 `ikaros-toolkit` 的命名操作和 prompt skill
  document。
- `ToolRegistry`：从 `SkillRegistry` 派生出的 executable-tool view；它会把 prompt skill document 排除在 callable
  tool lookup 和 provider tool schema 之外。
- `SkillDescriptor`、`PromptSkillDocument` 和 `SkillBundle`：区分 executable tool 与 prompt skill，携带
  `toolset` 元数据，并支持 `disable_model_invocation`。
- `ExecutionSession`：workspace、policy、approval、audit log、agent overlay、dry-run 状态和 `ExecutionEnv`。
- `ExecutionEnv`：`ikaros-toolkit` 提供的共享环境接口，local、dry-run、Docker、
  workspace 和 network 实现位于 `ikaros-sandbox`。
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

同一边界还会发出结构化 `tracing` 事件，覆盖 policy decision、tool
start/completion/failure、approval decision 和 approval replay、process execution
以及受控 network egress。Tracing 字段只包含 metadata：tool/approval/call id、skill
name、risk label、decision、command name、argument count、byte count、status code
和脱敏后的错误文本。请求输入、process stdout/stderr、network header 和 network body
不会写进 tracing event。

Coding 有两条 harness 路径。`code_workflow` 是受控 turn workflow：默认是 safe-read
的 plan/review 路径；在 `test`/`edit` 中请求 `run_tests` 时策略风险升级为
shell-read；只有在 `edit` mode 明确应用候选 patch 时才升级为 local-write。
`self_modify` 在进入专用 self-modify 审批路径前会被普通 `code_workflow` 拒绝。
`--model-loop` 在审批后一定会调用配置的 model provider。当前 policy request 仍只有一个
effective risk label：纯模型 loop 是 network risk，test loop 是 shell-read risk，patch 应用是
local-write risk。为了不隐藏组合风险，审批请求还会携带 provider call、workspace write、
shell/test command、session/turn identity、候选 diff 大小和 replay 指令的结构化 context。
CLI 会把这些 context 渲染为 `approval_scope`；成功执行或审批重放后的 coding turn 会渲染
`coding_progress` 和 `coding_result` 摘要，常见路径不需要先查 `debug coding-turn`
JSON。它会组装 repo map、change plan、可选 patch attempt、
turn diff、test-matrix evidence、review、iteration plan、loop report、final report
和可选 session replay evidence。`code_workflow` 和 `code_edit_guarded` 里的 patch
写入都通过 session `ExecutionEnv` filesystem interface 执行，而不是在 skill 内直接调用
host filesystem API。已批准的 `code_workflow` replay 会重新使用 coding registry，
因此 provider-backed loop 会保留 session id、turn id、provider、budget、cancellation
和 event persistence 边界。Provider-backed coding loop 在等待 provider response 时也会响应取消；
取消会写入 `coding_loop_cancelled` event，并在后续 patch/test/review 前停止。
`code_edit_guarded` 仍是直接应用指定 unified diff 的审批受控入口。

Skill descriptor 也携带 runtime 调度元数据：

- `execution_mode = parallel`：可以和同一模型响应里相邻的 parallel 调用成批执行。
- `execution_mode = sequential`：必须单独执行并保持严格顺序。
- `timeout_ms`：可选的 per-tool runtime timeout。超时会返回 failed tool result，并写进 lifecycle event。

Safe-read 和 shell-read descriptor 默认 `parallel`。write、network、remote、
destructive、secret 和 self-modify 风险 descriptor 默认 `sequential`。模型 provider
只看到可调用 tool schema；调度元数据属于 runtime/harness 边界。

Skill descriptor 同时携带 `toolset`。Agent profile 会选择启用哪些 toolset。直接进入
模型 tool manifest 的工具面仅限已启用的 `core`、`workspace`、`memory`，以及 bridge
tools；`rag`、`coding`、`voice`、`plugin` 即使启用也保持 deferred。因此模型可以发现并
调用 deferred tool，而不需要每轮都注入所有 RAG/coding/voice/plugin schema。
Bridge 会遵守当前 agent profile 的 toolset selection：
未启用 toolset 里的 deferred tool 不能被 `tool_search` 发现、不能被 `tool_describe`
描述，也不能通过 `tool_call` 调用。披露状态限定在当前 `ExecutionSession` 内：
`tool_search` 要求非空 query，并且只披露本次返回的 deferred descriptor；
`tool_describe` 只披露指定的 descriptor，`tool_call` 会拒绝同一 session 中尚未经过
这两条路径披露的 deferred tool。`tool_call` 会通过 `ExecutionSession` 委托到目标 skill，所以目标工具仍拥有自己的
policy decision、approval request、audit event 和 `ExecutionEnv` 执行边界。审计日志会显示
bridge 调用、带目标 descriptor metadata 的 `deferred_tool_invocation` 关联事件，以及底层
deferred tool 的 call/result。

Prompt skill 和 executable tool 是不同边界。Prompt skill 以
`PromptSkillDocument` 注册，只包含说明文档和 descriptor metadata，不携带可执行
`Skill` 实现。它不会进入 provider tool schema，`tool_call` 也会始终拒绝把它当工具执行。
模型侧唯一入口是渐进披露：当前 toolset 允许时，`tool_search` 可以返回它的
descriptor metadata，包括 provenance 和安全相对路径形式的 support-file 列表；
`tool_describe` 可以返回经过同一套 secret redaction 处理后的 instruction document
和安全 support-file 内容。`tool_search` 永远不返回 instruction body 或 support-file
内容。这样文档型指导和 support files 不会污染可调用工具命名空间，但需要时仍能被某一轮显式加载。
本地 prompt skill document 会从 `IKAROS_HOME/skills/<name>/SKILL.md`
发现。目录名就是 skill name。文档可以带一个很小的 front matter，包含
`description`、`toolset`、`provenance` 和 `support_files`；正文就是 instruction
text。`support_files` 只能列出同一 skill 目录下的相对文件；父目录逃逸和绝对路径会被忽略。
安全 support file 不会出现在 `tool_search` 结果里，但 `tool_describe` 会按需加载、
脱敏，并在文件过大时带上 truncation 标记。这类文档仍然是 prompt skill，不是
plugin：它们可以通过 bridge 被搜索和描述，但永远不能执行。

## 策略决策

策略有效结果有三类：

- `allow`：通过 `ExecutionEnv` 执行，并记录结果。
- `ask`：持久化审批请求，不执行。
- `deny`：直接返回，不执行。

Profile overlay 可以收紧或请求普通写入、shell 和 network 操作。硬性拒绝仍然优先于
profile 配置，例如破坏性命令、受保护路径、直接 secret 访问、发布动作、workspace
外写入和普通自修改。

审批重放不是通用 capability token。它必须匹配原始审批的 workspace、skill、risk、input 和 agent identity。

## ExecutionEnv

`ExecutionEnv` 是 `ikaros-toolkit` 提供的共享接口。`ikaros-sandbox` 提供 local、
dry-run、Docker、workspace-scoped 和 governed-network 实现。Harness 会把一个
environment 挂到每个 `ExecutionSession` 上，并要求 skill 使用它。

这个接口把 host 操作收敛成三个能力：

- `FileSystem`：读取 path metadata、读写文本和二进制文件、创建目录、列目录。
- `ProcessRunner`：运行结构化进程请求。
- `NetworkEgress`：网络出口请求。

默认 session 环境是 `WorkspaceExecutionEnv`，它在本地 backend 外面加 workspace
scope。`LocalExecutionEnv` 仍然是原始 host backend，主要给测试和后续环境实现使用；
普通 runtime session 不应直接挂它，除非调用方明确要绕过 workspace scope。

普通 runtime session 的环境由 `ikaros-host` 提供。`ikaros-host` 会把当前配置里的
sandbox backend、workspace scope、dry-run wrapper、Docker process backend 和受治理
network egress 组合成挂到 session 上的 `ExecutionEnv`。Harness 拥有策略判断和
session 边界；host assembly 负责按配置组装；sandbox 拥有具体 filesystem/process/network
adapter。

`WorkspaceExecutionEnv` 会把相对路径解析到 session workspace。文件读取/写入、二进制
读取/写入、列目录、创建目录、删除文件和进程 cwd 都必须留在 workspace root 下。Scope
检查同时做 lexical normalize 和已有路径的 canonical anchor 校验，因此 `..` 路径和
symlink 逃逸不能把一次获批的 workspace 操作变成外部 host 读写。读取是否允许仍由
skill policy 或 reference resolver 决定，但 env 层会强制已有路径不能越过 workspace
边界。在 Unix 本地文件写入路径上，最终目标还会用 no-follow 方式打开，因此 workspace
scope 检查和真实写入之间发生的 symlink swap 会被拒绝，不会跟随被替换的路径写到
workspace 外。文件、shell、coding、RAG maintenance、voice output、voice ASR 音频读取、self-modify 的 workspace
读写/check 和 command-backed plugin 都应走 session/env，不应直接调用 host API。

`ProcessRequest` 有两种模式：

- `program`：用 program + args 执行。
- `shell`：通过平台 shell 执行。

面向模型的 skill 应优先使用 `program`。`shell` 只用于已经完成 allowlist 校验的内部 adapter。本地 backend 会捕获 stdout/stderr，
支持可选 stdin、timeout，并能在输出超过 `max_output_bytes` 时拒绝。Timeout 和 output-cap failure 会在返回结构化错误前终止 Unix 上的
spawned process group。
进程执行会在 spawn 前清空宿主环境，只恢复少量平台基线变量（例如 `PATH`、home、临时目录、system root），再叠加显式的 `ProcessRequest.env`。
`ProcessRequest` 的诊断 `Debug` 输出会脱敏敏感环境变量名和疑似 secret 的值，因此可以看到请求了哪些 env key，但不会泄漏凭证。

Command-backed plugin 会使用显式 plugin cwd scope 运行。Manifest 里的程序必须
canonicalize 后仍在 plugin 目录内；该 scope 会拒绝 shell 执行，命令 cwd 是 plugin root，而不是用户 workspace。普通 workspace
command 仍使用默认 workspace cwd scope。

`NetworkEgress` 是接口的一部分。Runtime session 会把 workspace-scoped
filesystem/process backend 与 `GovernedNetworkEgress`、`HttpNetworkEgress` 组合起来。
受控 wrapper 默认拒绝，只允许 `ikaros-host` 构造的 allowlist 中精确解析出的 URL
host，并会脱敏被拒绝的 host 摘要；URL scheme 只允许 `http` 和 `https`。HTTP
transport 会解析已允许的 host，拒绝解析到受限地址的结果，关闭 redirect，并把验证后的
socket address pin 到本次 request client，避免同一次请求再做第二次独立 DNS lookup。
Chat、task agent-loop 和 provider-backed coding 路径中的 provider HTTP adapter 会拿到
egress-backed transport。
Provider-backed RAG embedding skill 在审批通过后，也会通过当前 session environment 执行
OpenAI-compatible 和 Ollama embedding HTTP。任意 plugin 或 shell 代码不应绕过 harness
直接发网络请求；面向模型的 shell command 仍限制在结构化 test/check allowlist 内。
`NetworkEgressRequest` 和 `NetworkEgressResponse` 会保留 transport 解析所需的原始
headers/body，但它们的诊断 `Debug` 输出会先脱敏 URL、header 和 body 中疑似 secret
的值，避免进入日志或错误报告。

`execution.sandbox.backend: dry-run` 会安装面向文件系统和进程副作用的 dry-run backend。
它保留 workspace 读取，但跳过写入和进程执行，并返回结构化 dry-run 输出。`docker`
会安装第一版进程容器 backend：process request 会被转换成
`docker run --rm --network none`，workspace 会挂载到容器内 `/workspace`，配置的
`execution.sandbox.image` 提供 Cargo、Git、npm 或 pytest 等工具。网络出口由
`execution.network.enabled` 和受控 allowlist 单独控制；如果 dry-run session 也必须避免网络副作用，需要关闭
`execution.network.enabled`。Docker 进程执行是一个有用的本地隔离层，但它不是完整 seccomp、VM 或多租户 sandbox。

当前 sandbox 面还提供一份诊断用 isolation matrix 和本地 sandbox debug report。已可用等级是 `dry_run`、`workspace_only`、
`network_restricted` 和 Docker-backed `container` 第一版；原始 no-op host execution 不作为正常 runtime backend
暴露。这份 report 用来解释 cwd scope、env allowlist、timeout/output cap、process timeout strategy、file-write
scope 和受控 network egress，是 debug/UX contract，不是完整 process namespace、seccomp 或 VM 边界。

## Shell 和 Plugin

- `shell_guarded` 不再执行任意 shell 字符串；它只接受 allowlisted test/check command，并解析成 program + args 执行。
- `git_status` 和 `git_diff` 是固定结构化命令。
- `run_tests` 复用同一套 test/check allowlist。
- Command-backed plugin 不通过 shell 执行；manifest 的 `program` 必须是相对路径，canonicalize 后仍在 plugin 目录内。
  解析后的程序会在显式 plugin cwd scope 下以 plugin 目录作为 cwd 运行，policy/audit 仍来自当前 session。
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

`task run --dry-run` 使用相同路径并启用 dry-run。`task run --agent-loop` 允许模型选择
受治理 skill，但 dispatch 仍经过 `ExecutionSession`。

## 审计规则

Audit event 应解释决策，但不保存不必要的敏感内容。Safe-read chat context lookup 可以用真实本地输入执行，同时写入脱敏后的 audit input。
Command output 和 plugin output 在报告前会脱敏。Provider usage log 应记录 provider、model、时间和 token count，不记录
prompt。

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
