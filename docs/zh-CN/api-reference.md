# API 参考

Rust crate API 仍处于 pre-release。当前支持的用户界面是 `ikaros` CLI。

生成 crate 文档：

```bash
cargo doc --workspace --all-features --no-deps
```

## 常用命令

初始化和检查本地状态：

```bash
ikaros init
ikaros init --full
ikaros setup --interactive
ikaros setup \
  --api-key "$MODEL_API_KEY" \
  --base-url https://api.example.com/v1 \
  --model provider-model-id
ikaros config validate
ikaros config show
ikaros config budget
ikaros config budget --set 200000
ikaros config budget --disable
ikaros doctor
ikaros provider inspect
ikaros provider health
ikaros provider health --live
ikaros provider matrix
ikaros provider matrix --live
ikaros provider profiles
ikaros api serve --port 8003 --bearer-token "$IKAROS_API_TOKEN"
ikaros acp serve --agent build --workspace .
ikaros mcp serve-stdio
ikaros mcp status
ikaros mcp probe <id> [--force]
ikaros mcp probe-stdio <command> -- <args...>
ikaros mcp probe-http https://mcp.example/rpc --include-tool search
ikaros mcp call-http https://mcp.example/rpc search --arguments-json '{"query":"ikaros"}'
ikaros web search "Ikaros runtime" --max-results 5
ikaros web extract https://example.com --max-chars 4000
ikaros vision describe screenshots/workbench.png
ikaros image generate "small local-first agent logo"
ikaros browser launch --headless --url https://example.com
ikaros browser status
ikaros browser list
```

`init` 会写入极简启动配置。它只包含 `schema_version` 和 `model.default` inline 字段，
其中 `preset: auto`，`model`、`api_key` 和 `base_url` 为空。需要一开始就看到完整
默认 YAML 时使用 `init --full`；完整文件会包含 provider pool、agent profile、memory、
RAG、voice、gateway 和 execution 等配置段。

`setup --interactive` 会交互式询问首次 provider 字段；同一批字段也可以用
`setup --api-key ... --base-url ... --model ...` 非交互传入。两条路径都会在需要时创建
runtime home。如果当前配置仍是极简文件，setup 会先把它展开成完整 YAML，再写入
provider/resource 字段。它会把明文模型 `api_key` 存在这个本地文件里，embedding 默认
使用本地 `hash`，TTS/ASR 在没有显式 provider 三元组时默认使用 `mock`，写完后校验
配置，并且不会把 secret 值打印到终端。

`config show` 会把当前 runtime 配置输出成脱敏摘要。它展示 provider family、模型名、
存储 backend、execution 设置，以及 credential/endpoint 是否已配置的布尔值；不会打印 API key
或 base URL。自动化可以使用 `--json`。

`doctor` 会在子系统摘要前输出当前配置 schema version、`config_valid` 和脱敏的
`config_issue` 行。模型摘要还会报告当天 token 用量、daily budget 剩余量和 budget
状态；RAG 摘要会报告 embedding 是否使用 session network egress。它适合启动诊断；
脚本仍应使用 `config validate` 做严格校验。

`provider inspect` 会读取本地 `IKAROS_HOME/config.yaml` 的 provider 设置，并输出解析后的
provider descriptor：provider family、model、profile、context window、tokenizer、
capability、profile policy、health state、cost 字段，以及已配置的 model fallback 行。
OpenAI-compatible profile policy 会说明已解析出的 temperature、reasoning、message、tool
schema 和额外 request body 行为。它不会调用远端 provider，也不会打印 API key。

`provider health` 读取本地 provider health ledger。`provider health --live` 会通过 runtime
`NetworkEgress` 发送一个短请求，并记录结果，不会打印 API key。
`provider matrix` 会输出当前配置的 model、embedding、TTS 和 ASR provider 行，包括
descriptor capability、context、input/output/cache-read/cache-write cost metadata、本地就绪状态、脱敏后的凭证存在性、最近
health status、profile policy 和 cooldown metadata。`provider matrix --live` 会 probe 当前配置的
model、embedding、TTS 和 ASR 行。Model 和远程 embedding probe 走 runtime
`NetworkEgress`；本地 embedding probe 走本地 RAG store，TTS/ASR probe 走已配置的
voice provider。
`provider profiles` 会输出静态 OpenAI-compatible profile catalog，包括 auto-detection
hints、request-shaping policy、context metadata 和 capability flag。

`api serve` 会启动本地 OpenAI-compatible API surface。它只绑定 loopback 地址，提供
`GET /healthz`、`GET /health`、`GET /ready`、`GET /v1/models`、
`GET /v1/ikaros/protocol`、`POST /v1/chat/completions`、`POST /v1/responses`、
`POST /v1/embeddings`、`POST /v1/images/generations`、`POST /v1/audio/speech`
和 `POST /v1/audio/transcriptions`。

Chat 和 Responses 调用通过 runtime `NetworkEgress` 使用当前 agent 配置的模型
provider。Embedding 调用使用当前 RAG embedding provider，远程 embedding 请求也走同一个
execution environment。`/v1/embeddings` 接受 `encoding_format: "float"` 和
`encoding_format: "base64"`；base64 响应会把返回的 `float32` 向量字节按
little-endian 顺序编码。`/v1/models` 会在配置存在时列出 chat、embedding、image、
TTS 和 ASR 行。

Chat request message 可以使用普通字符串 `content`，也可以使用 OpenAI 风格 content
part。当前 multimodal protocol slice 会把 text、image、audio 和 file part 保留为模型
`ContentBlock`。Provider adapter 会按自身 capability 接受或拒绝这些 block。
`image_url` 会转发给 OpenAI-compatible provider，Anthropic 会收到 image content block，
Ollama 会在输入为 `data:image/...;base64,...` 时收到 base64 `images`。

`/v1/responses` 接受字符串 `input` 或 message item 数组，支持 `instructions`、
text/image/audio/file content part、function tool definition、`max_output_tokens`、
`temperature`、`top_p` 和 `stream: true`。非流式响应会返回 Responses 形状的
`response` object，并带 Ikaros session evidence；流式响应会输出 Responses SSE 风格事件。
内置 web/file tool、background mode、持久 response conversation、retrieval 和完整
Responses event parity 属于后续 API 工作。

`/v1/images/generations` 会把 OpenAI-compatible image-generation 请求代理到已配置的
模型 provider base URL。`POST /v1/audio/speech` 和
`POST /v1/audio/transcriptions` 会通过已配置 voice provider 代理 OpenAI-compatible
语音请求。这些 route 和 chat 一样会写 session evidence 与 audit record，但不会执行模型传入的工具。

传入
`--bearer-token` 后，`/v1/*` 路由要求 `Authorization: Bearer ...`；本地 key rotation
期间可以重复传入该参数，让旧 token 和新 token 同时可用。health 和 readiness
路由保持开放，方便本地检查。客户端可以发送 `X-Ikaros-Client-Id`；脱敏后的值会进入
`api_request` audit event，便于本地追踪。服务还会使用 `--rate-limit-per-minute` 做进程内请求限流，
并写入脱敏的 `api_request` audit event。每个 chat 或 embedding 请求也会在 `state.db`
创建 service session turn；响应里的非标准 `ikaros.session_id` 和 `ikaros.turn_id` 可用于
debug/replay 命令。同一组值也会通过 `X-Ikaros-Session-Id`、`X-Ikaros-Turn-Id` 和
`X-Ikaros-Correlation-Id` 响应 header 返回，并写入 audit event。Chat 请求可以带
OpenAI 风格的 function `tools`、assistant `tool_calls` 和 tool-result message；Ikaros 会把
这些转发到 provider 协议，并把 provider 返回的 tool call 投影回 OpenAI-compatible 响应，
但不会执行 API 传入的工具。无效请求体和内部失败会以脱敏 JSON error object 返回，不会直接断开连接。
`stream: true` 会从 Ikaros 归一化 provider stream 输出 OpenAI SSE 形状的 chunk；真正逐字节实时转发、
分布式限流和持久 API credential lifecycle 不属于这一版切片。

`acp serve` 会在 stdio JSON-RPC 上启动 Agent Client Protocol server，面向 IDE client
和其他本地前端。它使用和 CLI 相同的 runtime、session store、harness policy、approval、
audit、workspace scope 和 provider 边界。第一版支持 `initialize`、`initialized`、
`session/new`、`session/prompt`、`session/list`、`session/events`、`session/replay`、
`tools/list`、`approval/list` 和 `shutdown`。它声明的 capability 包括 session
management、streaming events、tool discovery、approval handling 和 session replay。

`mcp serve-stdio` 会启动第一版由 harness 托管的 MCP stdio surface。它在
stdin/stdout 上使用 line-delimited JSON-RPC，目前支持 `initialize`、`ping`、
`tools/list` 和 `tools/call`。暴露的工具来自当前 agent 已启用的 Ikaros skill；
工具调用仍经过 `ExecutionSession`、policy、approval、audit、workspace scope 和
`ExecutionEnv`。

`mcp status` 会输出 `config.yaml` 中配置的外部 MCP server，但不会启动它们。
`mcp probe <id>` 会使用对应配置记录，并把 `include_tools`/`exclude_tools` 精确工具名
过滤应用到发现到的 tool report。未启用的 server 默认会跳过；传 `--force` 后也仍会
经过 policy 和 approval。

`mcp probe-stdio` 会把一个 stdio MCP server 作为 harness 托管进程启动，发送
`initialize` 和 `tools/list`，然后打印解析出的 server/tool report。它现在是一
次性 client probe，不是持久 MCP client 进程池。该命令会被视为任意本地进程启动，
因此仍经过普通 harness process tool 的 policy、approval、audit、workspace scope、
timeout 和 output-limit 路径。
`mcp probe-http` 会通过当前 session `NetworkEgress` 边界向 HTTP MCP endpoint 发送
`initialize` 和 `tools/list` JSON-RPC POST 请求，脱敏响应，并应用 include/exclude
tool 过滤。它同样只是一次性 probe；持久 HTTP MCP session、server-sent event
streaming、生命周期管理和动态工具注册仍是后续 client 工作。
`mcp call-http` 会通过同一个 `NetworkEgress` 边界发送 `initialize` 和一次
`tools/call` 请求。打印前会脱敏 request/response report。这是受控的一次性
client call；持久远端工具注册和长生命周期 MCP session 仍是后续 client 工作。

聊天：

```bash
ikaros
ikaros chat
ikaros chat --message "hello"
ikaros chat --stream --message "hello"
ikaros chat --message "describe this screenshot" --image screenshots/workbench.png
ikaros chat --message "transcribe or inspect this audio" --audio audio/sample.wav
ikaros chat --message "summarize this file" --file docs/zh-CN/architecture.md
ikaros chat --context-token-budget 4000 --message "summarize @file:docs/zh-CN/architecture.md:1-80"
ikaros chat --memory-search-limit 3 --message "include explicit long-term memory search"
ikaros chat --sessions
ikaros chat --history
ikaros chat --history-search "query"
```

不传子命令的 `ikaros`，以及不传 `--message` 的 `ikaros chat` 都会进入
terminal workbench。一次性 CLI 仍保留给脚本和测试；普通交互路径提供 history、
multiline input、cursor-aware line editing、内存输入缓冲区 undo、bracketed paste、
pending input queue、默认 streaming、session resume、timeline/replay/debug、provider/gateway/task
状态、approval overlay、context/memory/RAG 状态和 coding workflow 命令。

常用 workbench slash command 按用途分组：

- 帮助和导航：`/help`、`/commands [query]`、`/queue`、`/agents`、
  `/agent <profile-or-instance>`、`/status` 和
  `/budget [show|set <tokens>|disable]`。
- 屏幕控制：`/screen [--focus status|timeline|main|side]`、
  `/screen [--focus-next|--focus-prev]`、`/screen [--scroll N]`、
  `/screen [--select N|--select-title TEXT|--select-kind KIND]`、
  `/screen [--select-action SELECTOR]`、
  `/screen [--down|--up|--page-down|--page-up|--top]`、
  `/screen [--palette [query]|--palette-query query|--close-palette]`、
  `/screen [--fullscreen|--inline|--raw|--rich]`，以及
  `approve-selected`、`deny-selected`、`cancel-selected`、`clear-selected`、
  `open-selected`、`confirm-selected` 这类选中项动作。
- Session 控制：`/history [limit]`、`/sessions`、
  `/session status|resume|history|timeline|export [path]`、`/resume <session>`、
  `/new` 和 `/fork`。
- Timeline 和 debug：
  `/timeline|/replay|/debug [turn] [--page N] [--kind KIND] [--failed|--approval]`
  以及 `/trace [turn] [--kind KIND] [--failed|--approval]`。
- Context 和 runtime 视图：`/mentions [query]`、`/context`、`/memory`、`/rag`、
  `/tools`、`/model`、
  `/provider [inspect|health [--live]|matrix [--live]|profiles|debug]`、
  `/mcp status`、`/mcp call-http <url> <tool>`、`/api status`、`/gateway`、
  `/tasks`、`/web`、`/browser`、`/vision` 和 `/image`。
- 动作命令：`/approval|/approvals [approve|deny <id>]`、
  `/cancel [all|<continuation-id>]`、`/diff`、`/multi`、`/clear`、
  `/attach`、`/code <plan|apply|test|review|rollback> ...`、`/review` 和
  `/rollback`。
- 退出：`/quit` 或 `/exit`。
`/code` 会路由到同一套受治理 `ikaros code` workflow，并把 coding turn evidence 写入
`state.db`。
`/attach` 会把 image、audio 或 file content block 加到下一次 chat turn。同一条 attachment
路径也能通过一次性 chat 的 `--image`、`--audio` 和 `--file` 使用。本地路径会在
workspace 内解析，受大小限制，并在进入 provider 前转换成 data URL。
`/memory` 会把当前 memory surface 拆成三层显示：projection 文件、pending memory
candidate，以及当前 chat session 的 active working-memory record，后面再显示 memory
lifecycle timeline 和 journal cell。
每个 streamed turn 结束后，workbench 会输出 compact live cell：model stream delta 会压缩成一条
summary cell，tool、context、coding、approval、continuation、audit 和 error 事件仍按 typed cell 展示。
同时会输出 `live_cells_json`，包含 `schema=ikaros-workbench-live-cells-v1`、`version=1`、
event category 计数、被压缩的 model-stream 数量，以及脱敏后的 compact cell 列表，供 async
TUI cell renderer 直接消费。
streaming 完成后还会输出 `rendered_markdown` transcript，让 code fence、diff block、table
和脱敏后的错误文本保持可读，同时不牺牲 live token output。
`/status` 会在 active model 行里直接显示已解析 provider profile、profile source、context
window、默认 output reservation、tokenizer、runtime、transport 和最近 health state，并用
`status_model_policy` 行显示 temperature、reasoning、message、tool-schema、request-body、
prompt-cache 和 retry policy。它还会输出 `status_model_budget`、`status_model_cost`
和 `status_model_fallbacks`，用于解释每日 token budget、今日估算成本、cache read/write
token 统计和 fallback chain readiness。`/sandbox [--probe]` 会输出和
`/debug sandbox` 相同的脱敏 `sandbox_json` report；screen 里的 sandbox cell 默认打开
`/sandbox`，同时保留 `/debug sandbox` 作为更深诊断入口。`/screen` 视图也会包含 gateway、MCP 和 API cell。
Gateway cell 会显示 pending/processing/cancelled 计数和脱敏后的 `message-worker.lock`
状态，因此不用离开 workbench 也能看出是否有重复本地 gateway worker 或被中断的消息。
API cell 会指向 `/api status`，列出本地 OpenAI-compatible chat、Responses、embeddings、
image、audio、protocol、model discovery 和 health 路由，不会启动 server 或发起 live
provider 调用。配置 model fallback 时，这是不发起 live provider 调用就确认
OpenAI-compatible endpoint 是否解析到预期 profile 的最快方式，例如 Moonshot/Kimi、
Qwen、OpenRouter 或本地兼容服务。
`/model` 会从当前 workbench runtime 输出同一份 descriptor（`model_source:
active_runtime`），不会像独立顶层命令一样重新加载 provider 状态，并会输出每个已配置
fallback 的 resolved profile/readiness 摘要。明确想看配置级 provider 解析路径时再用
`/provider inspect`。
Slash command registry 会把命令分成 inspect、action/probe 和 terminal-output，
供 screen model 和后续 TUI routing 使用。默认 `ikaros` 终端路径仍输出人类可读的
命令结果；`ikaros chat`、非 TTY 运行和显式 screen/debug 命令仍保留脚本使用的
确定性 protocol 输出。需要查看 command palette 模型时使用 `/screen --palette`；
`/help` 和 `/commands` 会输出命令帮助，不会打开常驻 palette。
`/provider debug` 会输出与 `ikaros debug provider` 相同的脱敏结构化诊断，包括 profile
source、cache policy、health、fallback row 和 live-smoke readiness hint，不会发起 live
provider 调用。
`/cancel` 会把当前 session 中 queued/running 的 durable continuation 标为 cancelled。
它不会杀任意宿主机进程；实际停止仍依赖 runtime worker 或 provider wait 路径观察到持久化的 cancel 状态。
`/screen` 会渲染确定性的 workbench frame，用于显式检查；不带子命令的 `ikaros`
使用正常终端 scrollback 和 inline composer。非 TTY 运行仍保留脚本和 smoke test
使用的确定性 snapshot/protocol 输出。每个交互 turn 和 slash command 结束后，
terminal UI 会把 human transcript、composer 和确定性的 `/screen` 输出分开。
streamed turn 运行中，cached screen 会先把刚提交的用户输入插成
`user turn=pending`，让显式 screen inspection 在模型 delta 到达前也能看到当前输入。
`--fullscreen` 仍是诊断渲染模式，会把单次刷新包进 alternate-screen terminal envelope，
并包含 hide/show cursor、clear/home 控制序列；`--inline` 会回到适合脚本读取的
行式 frame。`--focus`、`--scroll` 和 `--select` 是 timeline/main/side panel 的
第一版可导航 screen slice；连续 `/screen` 命令会保留当前 workbench session 的
focus、scroll 状态和每个 panel 独立的 selected row。每次刷新都会输出
`screen_selected`，包含当前 focused cell 的 panel、row、kind、title 和 detail，
方便脚本和 replay view 不解析 frame 也能查看选中内容；还会输出
`screen_selected_actions`，当选中的 cell 有足够证据时直接给出 timeline、trace、
debug、approval、cancellation 和 queue 后续命令；
随后输出 `screen_selected_actions_json`，用机器可读 JSON 给出同一份 panel、row、kind
和 command list，供 TUI key binding、ACP consumer 和 replay tooling 使用。
每次刷新还会输出 `screen_json`，这是脱敏后的全屏快照，包含 status cell、
timeline/main/side panel cell、focus/scroll/selection state、当前 selected cell 以及
安全后续命令。payload 会声明 `schema=ikaros-workbench-screen-v1`、`version=1`、
紧凑的 `key_bindings` 数组，以及更完整的 `keymap_model`。`keymap_model`
按 global command、panel navigation、composer editing、command palette、approval、
queue、action menu、timeline tabs 和 raw/rich render mode 分组描述按键。
同一份 payload 还包含 `surface` object，声明
`schema=ikaros-workbench-surface-v1`。它是 terminal UI 的稳定消费模型，包含
`bottom_pane_model`、`input_model`、`input_popup`、`overlay_routing`、
`turn_state_model`、`recovery_model`、`action_menu_model`、timeline grouping、
dashboard panel、provider、context、memory、RAG、coding、approval、queue panel、
readiness 和 debug surface。TUI、ACP 和 replay consumer 应读取这些结构化模型，
不要解析 footer 文本或 cell detail 字符串。
`input_model.context_chips` 会暴露 composer 当前可见的 session、memory 和 context
状态。Command palette item 也会包含 `command_class`、`action`、`action_label` 和
`visible_state`，让 consumer 在执行前就能显示 `/session`、`/context` 或 `/memory`
会检查什么。
fullscreen footer 现在主要显示模型、workspace 和滚动状态。当前 Enter target 应从
`overlay_routing`、`action_menu_model` 和 `Selected` 面板读取，而不是解析 footer 文本。
全屏 frame 还会显示 `Selected` 面板，列出当前可见选中 cell 的 panel、row、kind、title、
primary action 和脱敏后的 detail。如果 selection 已经滚动到当前窗口上方，面板会显示当前没有
可见选中项，而不是把不可见行的内容展示出来。
`open-selected` 会执行当前选中 cell 的第一个安全只读后续命令；patch apply、rollback
或 live provider probe 这类高风险选中动作需要 `confirm-selected`，避免回车静默跨过
mutation 边界。approval、cancellation 和 queue mutation 仍然通过 `approve-selected`、
`deny-selected`、`cancel-selected` 和 `clear-selected` 显式触发。
行式 key alias 也可用：`enter` 对应 `open-selected`，`confirm` 对应
`confirm-selected`，`a` 对应 `approve-selected`，`d` 对应 `deny-selected`，`c`
对应 `cancel-selected`，`x` 对应 `clear-selected`。
pending approval cell 会包含 approval id、call id、tool、risk、scope、reason、脱敏 input
preview 和内联 approve/deny 命令，因此不打开 approval JSON overlay 也能在 side panel
里判断这次审批。
tools cell 会把 `/tools` 作为只读可见性检查打开，用来查看当前 agent 的 direct、deferred
和 disabled 工具面。Live cell 也会保留稳定的 tool/context summary row，避免长 model stream
把当前 tool/context 状态挤出视图。`/tools` 会输出 `tools_status_json`，声明
`schema=ikaros-workbench-tools-status-v1`、`version=1`，包含 active agent、已启用
toolset、direct/deferred/disabled count，以及每个可见工具的脱敏 descriptor 元数据。
main panel 会显示 provider matrix、cost/cache
health/cooldown/error 和 fallback/debug cell，方便不离开 screen 就查看模型后端；provider matrix
cell 会携带 `/provider matrix`、`--live`、health、debug 和 inspect 操作；`open-selected`
默认使用本地只读 provider action，health/fallback 行会打开 `/provider health` 或
`/provider debug`；`/provider matrix --live` 这类 live probe 必须显式输入。coding cell
会从最新 `CodingTurn` replay 投影出 progress、diff、test 和 review 状态，并给出
`/code plan`、`/diff`、`/code apply`、`/code test`、`/code review` 和 `/code rollback`
后续动作。coding cell 的安全默认动作是 `/diff`；patch、test、review 和 rollback 仍必须通过显式
`/code ...` 命令触发。交互式 `/code apply --diff "..."` 会把转义的 `\n` 解码成换行，
因此 unified diff 可以作为一个 quoted argument 粘贴。Coding workflow 的审批请求会持久化到
session store，approve/deny 决策会写成类型化 approval event；`/trace --kind approval`
可以展示 patch 和 rollback 审批的请求与解决过程。
`/diff` 会输出 `diff_status_json`，声明 `schema=ikaros-workbench-diff-status-v1`、
`version=1`，包含 git status code、`has_changes`、脱敏后的 stat/error 行，以及直接 coding
workflow action。
也会把最新
context diff 投影成 budget、section、reference 和 compaction cell；section cell 会展示
source、trust level、freshness、scope、token budget 和 injection reason 等 context
contract 字段，并对疑似 secret 内容脱敏。
line editor 会跟踪 cursor movement、Home/End、delete、backspace、undo 状态，以及
readline 风格的 Ctrl-P/Ctrl-N/Ctrl-B/Ctrl-F/Ctrl-D 快捷键。Ctrl-U 和 Ctrl-K 会删除
光标前后内容，Shift+Enter 或 Alt+Enter 会插入换行，raw-mode line input 会启用
bracketed paste。控制动作会输出 `input_state` 行，包含 cursor 位置、脱敏后的
buffer、cursor view 和 slash completion candidates；这是 ratatui/crossterm 支撑的
第一版，有 raw-mode input、确定性的 screen reducer、单帧真实 TTY 诊断绘制和确定性
非 TTY snapshot，但还不是完整 async TUI 应用。
side panel 会显示 pending approval、queued/running continuation 和内存中的 pending input queue，
并在适用时给出 `/cancel`、`/queue remove N` 和 `/queue clear` 操作。`approve-selected`
和 `deny-selected` 会处理 side panel 当前选中的 pending approval；`cancel-selected`
会取消当前选中的 queued/running continuation；`clear-selected` 会移除当前选中的 pending
input queue 项，然后刷新 screen。
`/queue` 在 list、add、remove 和 clear 操作后都会输出 `pending_inputs_json`。payload 声明
`schema=ikaros-workbench-pending-inputs-v1`、`version=1`、`pending_count`、脱敏后的
input `items`，以及显式 `/queue remove N` 和 `/queue clear` 命令，供 queue/interrupt
panel 直接消费。
`/cancel` 和选中 continuation cancel 会输出 `continuations_json`。payload 声明
`schema=ikaros-workbench-continuations-v1`、`version=1`、queue status count、active
count、lease owner/expiry、attempt count、terminal 状态、脱敏后的 continuation payload，
以及 active queued/running continuation 的显式 cancel 命令。
取消 queued 或 running continuation 时也会记录类型化 `ContinuationCancelled` event。
payload 包含 continuation id、kind、status、脱敏 reason、attempt count，以及存在时的 lease
metadata，因此 `/trace --kind continuation` 和 `/timeline --kind continuation` 可以解释哪条
continuation 被取消以及原因。
交互式 turn 失败时会输出 `chat_turn_error_json`。payload 声明
`schema=ikaros-workbench-chat-turn-error-v1`、`version=1`、失败 session、分类后的
`error_kind`，例如 `budget_exceeded` 和 `provider_error`、脱敏 message，以及 `/status`、
`/budget`、`/budget set <tokens>`、`/budget disable`、`/provider debug` 或
`/provider health --live` 等恢复动作。同一次失败也会持久化为 main panel 里的
`latest error` cell 和 timeline error cell，因此 `/screen`、`/timeline` 和
`/trace --failed` 会在 turn 中止后继续展示恢复路径。
approval overlay 还会输出 `approval_overlay_json`，包含
`schema=ikaros-workbench-approval-overlay-v1`、`version=1`、pending count、脱敏后的 approval
context，以及每个 pending item 的 approve/deny/replay 命令。
全屏模式下，同一条 pending approval 也会渲染成居中的 TUI overlay，并显示 approve/deny/open
快捷键提示；side panel 和 JSON payload 仍是 replay 与自动化使用的结构化来源。
`/approval approve <id>` 或 `/approval deny <id>` 之后，workbench 会输出
`workbench_approval_continue`，包含 replay 状态、剩余 pending approval 数量，以及可直接使用的
`/screen`、`/timeline` 和 `/trace` 后续命令。
`/session status` 会显示当前 session 的 `state.db`、active leaf、active branch 长度、
active branch root/leaf，以及 durable continuation 数量，方便在 workbench 里直接理解当前
replay 分支，不需要先导出 JSON debug。`/session export [path]` 会为 active session 写出脱敏的
`ikaros-session-export-v1` artifact；相对路径会解析到当前 workspace 下，默认路径在
`IKAROS_HOME/exports`。
`/status` 会输出 `workbench_status_json`，声明
`schema=ikaros-workbench-status-v1`、`version=1`，包含 active session 和 state.db 路径、
agent policy、workspace、model profile 和 provider health、budget status、
gateway/approval/continuation count，以及 screen、timeline、trace、provider debug、approval
和 cancellation 的直接导航 action。
`/timeline`、`/replay`、`/debug` 和 `/trace` 可以组合 turn id 和
`--kind session|model|tool|context|memory|coding|audit|continuation|approval|error`，
直接跳到对应 replay cell 或 trace span，不需要导出完整 trace JSON。
timeline/replay/debug 也支持 `--page N`。未带过滤条件的 timeline/replay/debug 页面会使用
`SessionStore` 分页 replay API，并输出 `*_page_source: session_store_page`，正常导航不会先加载完整事件列表再切片。
带 turn、kind、failure 或 approval 过滤时，仍会扫描 session replay 以计算过滤后的 evidence set。
`--failed` 会跳到 error、tool failed、continuation failed 和 provider failure diagnostic；`--approval`
会跳到 approval requested/resolved event。
`/commands [query]` 会同时输出人类可读命令列表和 `commands_json` payload，后者包含每个匹配命令的
name、usage、summary、tags、permissions 和 supported surfaces。Full-screen TUI、
Gateway、ACP 和其他 command palette 应该消费这份 metadata，而不是解析 `/help` 文本。

Chat message 可以包含本地 context reference，例如 `@file:path:line-line`、`@folder:path`、`@git:rev`、`@diff` 和
`@staged`。这些 reference 会在当前 workspace 下解析，并写入 session context diff。`@url:` 会在配置的 network allowlist
允许精确 host 且 URL scheme 是 `http` 或 `https` 时通过 session `NetworkEgress` 抓取；被拒绝的 host 或不支持的 scheme 会使
turn 失败，同时不会泄漏疑似 secret 的 URL 文本。URL 响应如果声明 content type，只接受 plain text、Markdown、JSON、XML 或 YAML；
HTML/二进制响应和超过 64 KiB 的正文会变成 skipped reference notice，而不是 prompt content。
这不是 web search。需要受治理的搜索结果 metadata 时，应该显式调用 `web_search`
skill；需要单页 citation metadata 和 HTML 文本抽取时，应该显式调用 `web_extract`
skill。

`/context` 会输出 `context_status_json`，声明
`schema=ikaros-workbench-context-status-v1`、`version=1`，包含 context option 值、已注册
context engine descriptor、最新 context budget 元数据、section/reference count、脱敏后的
prompt section 元数据和 compaction 状态。它刻意不包含 prompt section 正文。Chat 默认使用
deterministic context engine；`--context-engine llm-summary` 会显式启用 provider-backed
summary compressor，未知 engine 名称会被拒绝。

默认 chat context 使用已接受的 memory projection 和 session working memory。长期 memory search 不会自动执行；如果某个 turn
需要 retrieved memory result，需要传入 `--memory-search-limit N` 或显式使用 `memory_search` 工具。
`/memory` 会输出 `memory_status_json`，声明
`schema=ikaros-workbench-memory-status-v1`、`version=1`，包含 memory backend、policy
threshold、external provider 数量、projection file count、pending candidate count、active
working-memory count、journal entry count，以及直接 memory debug action。它不会嵌入完整
memory record 正文。
`/rag` 会输出 `rag_status_json`，声明 `schema=ikaros-workbench-rag-status-v1`、
`version=1`，包含 RAG backend、embedding provider/model、配置的 `rag_top_k`、普通 chat
是否启用注入、本地 RAG 目录，以及 ingest/search/context 直接 action。

`--context-token-budget 0` 表示让 runtime chat 使用 provider 推导出来的可用
context window，不表示可以绕过模型上下文窗口。
持久化的 context diff 会记录本轮选择的 token estimator adapter，例如
OpenAI-compatible、mock，或 Anthropic/Ollama 的显式 fallback。

调试持久化 session evidence：

```bash
ikaros debug context-diff <session-id>
ikaros debug context-diff <session-id> --turn-id <turn-id>
ikaros debug memory-lifecycle <session-id>
ikaros debug memory-lifecycle <session-id> --turn-id <turn-id>
ikaros debug continuations <session-id>
ikaros debug continuations <session-id> --turn-id <turn-id>
ikaros debug coding-turn <session-id>
ikaros debug coding-turn <session-id> --turn-id <turn-id>
ikaros debug trace <session-id>
ikaros debug trace <session-id> --turn-id <turn-id>
ikaros debug provider
ikaros debug sandbox
ikaros debug insights
ikaros debug logs
ikaros debug logs --source model-usage --page-size 20
ikaros debug logs --source trace --page-size 20
ikaros debug dump --output /tmp/ikaros-debug-dump.json
ikaros debug state-db
ikaros debug state-db --checkpoint
ikaros debug state-db --backup /tmp/ikaros-state-backup.db
ikaros debug state-db --repair /tmp/ikaros-state-repair.db
ikaros debug state-db --restore /tmp/ikaros-state-backup.db
ikaros debug state-db --prune-ended-before 2026-01-01T00:00:00Z --vacuum
```

`context-diff` 读取 `state.db`，报告 estimator、budget、context window、context section token 估算、prompt
section 的 source/priority/token 元数据、added/removed/compressed context、已解析 reference、compaction
summary、continuation prompt、`ContextCompacted` 和 context-limit error。`memory-lifecycle` 读取 session
timeline 和 `memory_journal.jsonl`，查询匹配的 `MemoryLifecycle` event、`MemoryRef::SessionTurn` 关联、skipped
write、redaction 相关 note、action count 和 runtime memory policy action。`continuations` 会报告 durable
continuation queue status、status reason、lease owner、lease expiry、attempt count、terminal summary、
worker lease timeout evidence、error 和已脱敏 payload。按 `--turn-id` 过滤时，
如果 turn 存在但没有 continuation，会返回空结果；
只有 replay 中不存在该 turn 时才报错。
`coding-turn` 会报告 `ikaros code workflow` 持久化的 `CodingTurn` event、coding event kind 计数、review finding
和 custom session entry。
`trace` 会导出脱敏的 `ikaros-trace-v1` JSON，包含 session 或 turn 的 event category
计数、turn span、ordered event summary、entry 和 approval count，不包含 prompt 原文或疑似
secret 的 payload。
`provider` 会导出脱敏的 `ikaros-provider-debug-v1`，包含 active model row、fallback
chain、profile source、cache policy、cost metadata、health 和 readiness hint。`sandbox`
会导出当前本地 sandbox/debug report 和 isolation matrix；启用
`execution.sandbox.backend: docker` 时还会包含配置的 Docker image 和 mount point。
Docker-backed process execution 已经有第一版，但它不是完整 VM 或多租户 sandbox。
`logs` 会导出脱敏的 `ikaros-logs-v1` JSON，从本地 `audit.jsonl`、
`model-usage.jsonl` 和 `logs/trace.jsonl` 读取记录，支持 source 过滤和分页。
CLI 启动会把结构化 tracing event 写入 `logs/trace.jsonl`；`RUST_LOG` 可以缩小或扩大默认
Ikaros-focused filter。
`insights` 会导出脱敏的 `ikaros-debug-insights-v1` 运维摘要，把 config validation、
`state.db` integrity、provider readiness、audit/model-usage/trace 计数、cache token 统计、
近期脱敏日志样本、gateway queue 状态和需要关注的 alert row 合在一个视图里。
`dump` 会写出脱敏的 `ikaros-debug-dump-v1` 排障 artifact，包含 state database
健康状态、近期日志、sandbox 状态、路径和当前 agent identity。
`state-db` 会报告 SQLite 运维状态、WAL checkpoint、integrity check、write policy
和 search index 可用性。`--backup` 写出原始备份 artifact，`--repair` 写出重新整理并
通过 integrity check 的 artifact，`--restore` 会先校验源数据库、为当前数据库写出
pre-restore 安全备份、替换 `state.db`、清理旧 WAL sidecar，然后报告恢复后的 integrity
check。`--prune-ended-before` 只删除 `ended_at` 早于 RFC3339 cutoff 的 session；
需要压缩文件体积时可以同时加 `--vacuum`。

记忆和关系笔记：

```bash
ikaros memory add "note" --kind project --scope ikaros
ikaros memory add \
  --kind relationship \
  --scope default \
  --observer alice \
  --subject bob \
  "Bob likes pancakes"
ikaros memory search "query"
ikaros memory update <id> --content "new note"
ikaros memory delete --id <id>
ikaros memory projection render --scope ikaros
ikaros memory projection show --scope ikaros
ikaros memory candidate list
ikaros memory candidate accept <candidate-id> --reason "explicit user instruction"
ikaros memory candidate accept <candidate-id> \
  --supersedes <memory-id> \
  --reason "user corrected this"
ikaros memory candidate reject <candidate-id> --reason "temporary task scope"
ikaros memory supersession <memory-id>
ikaros memory working list --session <session-id>
ikaros memory working prune
ikaros relationship remember "preference" --scope user
ikaros relationship show --scope user
```

Runtime chat 会把安全的 turn 状态写进 session working memory，而不是长期 `Task`
memory。自动 relationship 观察会先进入 pending candidate；接受后才成为 core
memory。Projection 命令渲染 chat context 使用的已接受长期记忆 surface。`memory
update` 会返回更新后的 record，以及包含 `changed_fields`、`before` 和 `after`
摘要的 `change_report`，用于解释 content 和 tag 字段变化。

RAG：

```bash
ikaros rag ingest docs --scope project
ikaros rag search "harness policy"
ikaros rag stale
ikaros rag reindex docs --scope project
ikaros rag delete-path docs/old.md
ikaros rag delete-scope scratch
```

当 RAG 使用 `openai-compatible` 或 `ollama` embedding 时，`ingest`、`reindex` 和
`search` 可能先返回 approval id。审批后会重放原始 request，并通过当前 session 的
`ExecutionEnv` / `NetworkEgress` 边界执行 provider-backed embedding HTTP。本地
`hash`/`sparse`/`mock` embedding 不需要网络审批。`ikaros doctor` 会报告当前
embedding provider 是否使用 network egress，以及 embedding base URL 是否已经配置。

任务和 agent：

```bash
ikaros task run "summarize the repository" --dry-run
ikaros task run "inspect runtime" --agent-loop
ikaros agent list
ikaros agent show plan
ikaros agent run --profile plan --dry-run "inspect docs"
ikaros agent run --profile plan --agent-loop --parent-session <session-id> "inspect docs"
ikaros agent batch --profile plan --task "inspect docs" --task "inspect runtime"
```

`--parent-session` 会把 delegated agent-loop work 记录成 child session；
如果 parent session 位于同一个 agent store，还会把脱敏后的 `subagent_result`
entry 写回 parent timeline。

策略和审批：

```bash
ikaros policy explain write_note --risk local-write --path note.txt --write
ikaros approval list
ikaros approval approve <approval-id>
ikaros approval deny <approval-id>
```

Gateway 和 schedule：

```bash
ikaros schedule add "summarize status" --at now
ikaros schedule add "summarize status" --at now --delivery gateway-outbox
ikaros schedule add "summarize status" \
  --at now \
  --retry-max-attempts 3 \
  --retry-backoff-seconds 60 \
  --grace-period-seconds 300 \
  --timezone UTC
ikaros schedule run-due --dry-run
ikaros schedule worker --once
ikaros message send "hello" --kind chat
ikaros message send "hello" --kind chat --source telegram --account acct --peer peer --thread thread
ikaros message status
ikaros message cancel <id> --reason "operator requested cancel"
ikaros message delivery claim --limit 5 --owner telegram-adapter
ikaros message delivery ack <delivery-id> --lease-owner telegram-adapter --summary "sent"
ikaros message delivery fail <delivery-id> \
  --lease-owner telegram-adapter \
  --reason "remote timeout" \
  --backoff-seconds 30 \
  --max-attempts 3
ikaros message pairing create --source telegram --account bot-account --peer user-id
ikaros message pairing list
ikaros message drain --dry-run
ikaros message daemon start --interval-seconds 5 --limit 10
ikaros message daemon status
ikaros message daemon stop --reason "maintenance"
ikaros message daemon restart --interval-seconds 5 --limit 10
ikaros message webhook --port 8002
ikaros message webhook --port 8002 --hmac-secret "$IKAROS_WEBHOOK_SECRET"
ikaros message webhook --port 8002 --allow-source telegram --allow-peer user-id
ikaros message webhook --port 8002 --require-pairing
ikaros message webhook --port 8002 --unsafe-tools
```

`message status` 会报告 pending/processing/processed/failed/cancelled/dead-letter 数量，以及 delivery
pending/processing/delivered/dead-letter 数量，并显示脱敏后的 worker snapshot：active lease
owner/expiry/attempt、stale-processing 数量、带 last error 的 retryable message/delivery 数量，以及 dead-letter
terminal evidence。`message cancel` 会把 pending 或 processing message 移到终态 cancelled，避免旧 worker claim
后续投递结果。`message daemon` 负责 start/stop/restart/status 本地长期运行 worker，并复用 `message worker` 相同的 runtime
和 harness 路径。
`message delivery claim|ack|fail` 是 adapter 消费 outbox delivery 的 lease-bound retry/backoff 控制面。
`message webhook --hmac-secret` 要求 `X-Ikaros-Signature: sha256=<hex-hmac>`
覆盖原始 request body，校验通过后才会入队。
`message webhook --allow-source|--allow-account|--allow-peer|--allow-thread`
会在入队前拒绝不匹配的 adapter payload。
`message pairing create` 会给 source/account/peer 生成一次性 code；`message webhook --require-pairing`
要求 peer 已经配对，或接受一次有效 `pairing_code` 完成绑定后再入队。
Webhook 消息默认进入 safe-tools mode，drain 后的 chat/task agent loop 只暴露 `core` toolset；
`--unsafe-tools` 只用于本地可信 adapter 的显式 opt-out。

语音和 body 界面：

```bash
ikaros voice tts "hello" --output speech.wav
ikaros voice asr input.wav --language en
ikaros body status
ikaros body dashboard
ikaros body dashboard --refresh-seconds 5 --snapshot-output previews/frame.json
ikaros body serve --port 8001
ikaros browser launch --headless --url https://example.com
ikaros browser supervisor-status
ikaros browser stop
ikaros browser status --endpoint http://127.0.0.1:9222
ikaros browser list --endpoint http://127.0.0.1:9222
ikaros browser new https://example.com --endpoint http://127.0.0.1:9222
ikaros browser activate <target-id> --endpoint http://127.0.0.1:9222
ikaros browser close <target-id> --endpoint http://127.0.0.1:9222
ikaros browser navigate <target-id> https://example.com --endpoint http://127.0.0.1:9222
ikaros browser snapshot <target-id> --endpoint http://127.0.0.1:9222
ikaros browser click <target-id> 100 200 --endpoint http://127.0.0.1:9222
ikaros browser type <target-id> "hello" --endpoint http://127.0.0.1:9222
ikaros browser scroll <target-id> --y 600 --endpoint http://127.0.0.1:9222
ikaros browser screenshot <target-id> --format png --endpoint http://127.0.0.1:9222
ikaros browser cdp <target-id> Runtime.evaluate --params-json '{"expression":"location.href"}'
```

Cloud TTS 和 ASR 也走同一套审批流程。TTS 输出只渲染字节长度和可选文件路径，不打印原始音频字节。
`browser launch`、`browser supervisor-status` 和 `browser stop` 是第一版本地
browser supervisor。Supervisor state 存在 `IKAROS_HOME/browser/supervisor`；
除非传入 `--user-data-dir`，launch 会使用 profile 专属 browser data directory。

CDP 命令会通过当前 session `NetworkEgress` 边界调用本地或已配置 Chrome DevTools
endpoint，并打印脱敏 JSON。HTTP discovery 命令覆盖 status、target list、新建 target、
activate 和 close。WebSocket CDP 命令覆盖 navigate、snapshot、click、type、scroll、
screenshot 和 raw method call。受治理的是 CDP 控制请求；在更严格的 browser sandbox
完成前，页面网络流量仍由浏览器进程自己执行。

本地文件系统和 git 辅助命令：

```bash
ikaros fs read README.md
ikaros fs list docs
ikaros fs write notes/example.txt "local note"
ikaros git status
ikaros git diff --stat
```

插件：

```bash
ikaros skill list
ikaros skill audit
ikaros skill validate ./plugins/example
ikaros skill install ./plugins/example
ikaros skill inspect example.tool
ikaros skill run example.tool --input-json '{"message":"hello"}'
ikaros skill run web_search --input-json '{"query":"ikaros runtime"}'
ikaros skill run web_extract --input-json '{"url":"https://example.com"}'
```

`ikaros skill list` 会显示每个内置 skill 的 toolset，以及当前 agent profile 下的
model visibility：`direct`、`deferred` 或 `disabled`。Workbench `/tools` 会显示同一套当前
profile 的 direct/deferred/disabled 工具面，并通过 `tools_status_json` 暴露同一份分组结构，
供 workbench 和 ACP 消费。`web_search` 是直接可见的受治理网络 skill，默认使用
DuckDuckGo HTML provider；配置后也可以使用 Brave、Bing、SerpAPI 或 Tavily-compatible
endpoint。配置来源可以是 `providers.search`，也可以是单次调用传入的 override。
它返回结果标题、URL、snippet 和 citation metadata，不会继续抓取结果页面。
`web_extract` 是直接可见的单 URL 网络 skill：只接受 `http`/`https`，请求经过当前
session 的 `NetworkEgress` policy，保留内容有上限，返回 citation metadata，会脱敏疑似
secret 的文本，并在 content type 不支持时跳过而不是返回二进制内容。Deferred 的 RAG、
coding、voice 和 plugin tool 只有在当前
agent 启用对应 toolset 时，才能通过带非空 query 的 `tool_search`
发现、通过 `tool_describe` 查看 schema、通过 `tool_call` 显式调用；`tool_call`
还要求目标 tool 已经在同一 execution session 中被 `tool_search` 或 `tool_describe`
披露。目标工具实际执行仍然经过 harness policy、approval 和 audit。

代码辅助：

```bash
ikaros repo scan
ikaros test infer
ikaros test run --command "cargo test"
ikaros code plan "add focused tests" \
  --diff "<unified diff>" \
  --session-id <session-id> \
  --turn-id <turn-id>
ikaros code apply "apply candidate patch" \
  --diff "<unified diff>" \
  --session-id <session-id> \
  --turn-id <turn-id>
ikaros code test "run focused tests" \
  --test-command "cargo test -p ikaros-coding" \
  --session-id <session-id> \
  --turn-id <turn-id>
ikaros code review --diff "<unified diff>" --session-id <session-id> --turn-id <turn-id>
ikaros code rollback <session-id> --turn-id <turn-id> --rollback-turn-id <rollback-turn-id>
ikaros code workflow "provider loop" \
  --mode edit \
  --model-loop \
  --apply-patch \
  --run-tests \
  --max-iterations 2 \
  --test-command "cargo test"
ikaros code iterate
ikaros code guarded-edit "apply approved patch" --diff "<unified diff>"
```

`code plan`、`code apply`、`code test`、`code review` 和 `code rollback` 是
terminal-first coding 命令。它们只是同一个受治理 `code workflow` turn 的薄路由，
因此共享审批行为、`ExecutionEnv` 写入、test-matrix evidence 和持久化
`CodingTurn` replay。`code rollback` 会从 `state.db` 读取目标 turn 最后一个
`diff_updated` event，构造反向 unified diff，并作为新的审批 edit turn 提交。

`code workflow` 仍是完整底层入口。它会构造 `CodingTurnContext`、repo map、change plan、可选 patch
attempt、turn diff、test-matrix evidence、review、iteration plan、loop report 和
final report。它支持 `--mode plan|edit|review|test|self_modify`。Mode policy 是显式的：
`plan`/`review` 偏只读，`test` 可以运行 test matrix，`edit` 只有在设置
`--apply-patch` 时才会应用候选 patch，`self_modify` 在进入专用 self-modify
审批路径前会被普通 workflow 拒绝。Context 会记录 git baseline，包括 HEAD、
branch/detached 状态、clean/dirty/not-git/unknown 状态，以及
staged/unstaged/untracked 标记。传入 session/turn id 时，coding event 会写入
`state.db`，可用 `ikaros debug coding-turn` 查询。设置 `--model-loop` 时，
workflow 会使用配置的 model provider 请求严格 JSON candidate patch；审批后的执行路径会把
model request/response metadata、token budget stop、cancellation stop、patch
attempt、test evidence、review finding 和 loop termination 都写成可 replay 的
coding event。`--max-iterations` 限制为 `1..=8`；`--model-token-budget` 会在预计
request 超过剩余 coding-loop budget 时于 provider call 前停止。存在 `IKAROS.md` 和
`.ikaros/instructions.md` 时，workspace instruction 会自动进入 coding context。Coding turn
的审批请求会携带 provider、shell/test、workspace write、session 和 replay 的结构化 context；
CLI 会把它显示为 `approval_scope`。执行时会输出 `coding_progress` 和 `coding_result`
摘要；provider-backed coding turn 在等待 provider response 时可以用 Ctrl-C 请求取消。

Service manager 模板：

```bash
ikaros service render --kind schedule-worker --manager systemd
ikaros service render \
  --kind message-worker \
  --manager systemd \
  --output services/ikaros-message-worker.service
ikaros service render --kind message-webhook --manager launchd
```

Systemd `message-worker` 模板使用前台 `message worker` 作为 `ExecStart`，并添加
`ExecStop` hook 写入协作式 `message-worker.stop` 请求。

Self-modify：

```bash
ikaros self-modify propose --kind documentation-patch --target README.md --diff "<unified diff>"
ikaros self-modify request-apply <proposal-id>
ikaros self-modify apply-approved <proposal-id> --approval-id <approval-id>
ikaros self-modify rollback <proposal-id>
```

## 全局选项

`--ikaros-home <path>` 选择本地状态目录。

`--agent <profile>` 为创建 harness session 的命令选择 active profile。它可以放在 subcommand 前后：

```bash
ikaros --agent plan chat --message "read only"
ikaros chat --agent plan --message "read only"
```

## 兼容性

CLI 输出主要面向人类阅读。需要自动化集成时，优先使用已有测试覆盖的结构化 report 字段。

升级 Ikaros 后，应重新运行相关验证命令来确认依赖的输出字段仍符合预期。
