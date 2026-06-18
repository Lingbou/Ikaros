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
ikaros doctor
```

聊天：

```bash
ikaros chat
ikaros chat --message "hello"
ikaros chat --stream --message "hello"
ikaros chat --context-token-budget 4000 --message "summarize @file:docs/zh-CN/architecture.md:1-80"
ikaros chat --sessions
ikaros chat --history
ikaros chat --history-search "query"
```

不传 `--message` 时，`ikaros chat` 会进入当前交互式 chat REPL。它支持 `/help`、
`/agents`、`/agent <profile>`、`/status`、`/code <plan|apply|test|review|rollback> ...`
和 `/quit` 这类 slash command。`/code` 是同一套受控 `ikaros code` workflow 的薄包装，
也会把 coding turn evidence 写入 `state.db`。

Chat message 可以包含本地 context reference，例如 `@file:path:line-line`、`@folder:path`、`@git:rev`、`@diff` 和 `@staged`。这些 reference 会在当前 workspace 下解析，并写入 session context diff。`@url:` 只解析，不抓取。

`--context-token-budget 0` 表示让 runtime chat 使用 provider 推导出来的可用 context window，不表示可以绕过模型上下文窗口。
持久化的 context diff 会记录本轮选择的 token estimator adapter，例如 OpenAI-compatible、mock，或 Anthropic/Ollama 的显式 fallback。

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
```

`context-diff` 读取 `state.db`，报告 estimator、budget、context window、section token 估算、added/removed/compressed context、已解析 reference、compaction summary、continuation prompt、`ContextCompacted` 和 context-limit error。`memory-lifecycle` 读取 session timeline 和 `memory_journal.jsonl`，查询匹配的 `MemoryLifecycle` event、`MemoryRef::SessionTurn` 关联、skipped write、redaction 相关 note、action count 和 runtime memory policy action。`continuations` 会报告 durable continuation queue status、status reason、lease owner、lease expiry、attempt count、terminal summary、worker lease timeout evidence、error 和已脱敏 payload。按 `--turn-id` 过滤时，如果 turn 存在但没有 continuation，会返回空结果；只有 replay 中不存在该 turn 时才报错。
`coding-turn` 会报告 `ikaros code workflow` 持久化的 `CodingTurn` event、coding event kind 计数、review finding 和 custom session entry。

记忆和关系笔记：

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

Runtime chat 会把安全的 turn 状态写进 session working memory，而不是长期 `Task`
memory。自动 relationship 观察会先进入 pending candidate；接受后才成为 core
memory。Projection 命令渲染 chat context 使用的已接受长期记忆 surface。

RAG：

```bash
ikaros rag ingest docs --scope project
ikaros rag search "harness policy"
ikaros rag stale
ikaros rag reindex docs --scope project
ikaros rag delete-path docs/old.md
ikaros rag delete-scope scratch
```

当 RAG 使用 cloud embedding provider 时，`ingest`、`reindex` 和 `search` 可能先返回 approval id。执行 `ikaros approval approve <approval-id>` 后，才会重放并执行原始 approved request。

任务和 agent：

```bash
ikaros task run "summarize the repository" --dry-run
ikaros task run "inspect runtime" --agent-loop
ikaros agent list
ikaros agent show plan
ikaros agent run --profile plan --dry-run "inspect docs"
ikaros agent batch --profile plan --task "inspect docs" --task "inspect runtime"
```

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
ikaros schedule run-due --dry-run
ikaros schedule worker --once
ikaros message send "hello" --kind chat
ikaros message drain --dry-run
ikaros message webhook --port 8002
```

语音和 body 界面：

```bash
ikaros voice tts "hello" --output speech.wav
ikaros voice asr input.wav --language en
ikaros body status
ikaros body dashboard
ikaros body dashboard --refresh-seconds 5 --snapshot-output previews/frame.json
ikaros body serve --port 8001
```

Cloud TTS 和 ASR 也走同一套审批流程。TTS 输出只渲染字节长度和可选文件路径，不打印原始音频字节。

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
```

代码辅助：

```bash
ikaros repo scan
ikaros test infer
ikaros test run --command "cargo test"
ikaros code plan "add focused tests" --diff "<unified diff>" --session-id <session-id> --turn-id <turn-id>
ikaros code apply "apply candidate patch" --diff "<unified diff>" --session-id <session-id> --turn-id <turn-id>
ikaros code test "run focused tests" --test-command "cargo test -p ikaros-coding" --session-id <session-id> --turn-id <turn-id>
ikaros code review --diff "<unified diff>" --session-id <session-id> --turn-id <turn-id>
ikaros code rollback <session-id> --turn-id <turn-id> --rollback-turn-id <rollback-turn-id>
ikaros code workflow "provider loop" --mode edit --model-loop --apply-patch --run-tests --max-iterations 2 --test-command "cargo test"
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
`.ikaros/instructions.md` 时，workspace instruction 会自动进入 coding context。

Service manager 模板：

```bash
ikaros service render --kind schedule-worker --manager systemd
ikaros service render --kind message-worker --manager systemd --output services/ikaros-message-worker.service
ikaros service render --kind message-webhook --manager launchd
```

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
