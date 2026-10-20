# 参考仓库调研与下一步方向

> 日期：2026-10-20 ｜ 范围：`.temp/` 下 12 个本地参考仓库（只读调研）+ Ikaros 现状核对
> 方法：3 个并行调研任务分别精读「编码 Agent 生态 / 桌面与助理产品 / Agent 基础设施」；结论均出自代码与文档实际内容，关键点附仓库内路径。Ikaros 现状由 README、源码结构与 git 历史核对。
> 用途：为下一步方向选择提供依据；方向确定后应转为对应 Issue 实施。

## 一、Ikaros 现状核对

### 1.1 时间线与规模

- 2026-03-25 初始提交（Rust 时代：gateway/harness/fs4；遗留 Issue #10 仍引用当时的 Rust 代码）→ 06-12 起转向 Python Runtime 重写。
- 提交节奏：6 月 47、7 月 3、8 月 38、9 月 42、10 月 5（文档与流程为主）。
- Runtime 源码约 846 KB；Desktop 约 1.53 MB；测试约 880 KB（README §10：pytest 781 / vitest 528 通过）。
- 最大文件：storage/store.py 105 KB、storage/projections.py 73 KB、run_input.py 61 KB、agent/loop.py 50 KB。

### 1.2 能力面

- 8 个 provider 工具（protocol/spec.py）：process_start / process_read / process_wait / process_stop / history_read / read / write / edit。
  - 无 grep / glob / 目录列举工具：文件发现依赖 shell 命令；无 PTY、无 MCP（全仓 0 处提及）。
- 协议 7（32 方法 / 15 事件）由 spec.py 单一来源生成 + golden trace 回归；state schema 14 / memory schema 1。
- state.db append-only Journal + 可重建投影；memory.db 确定性关键词召回（无向量、无自动写入器）。
- 安全：full_access 单档（无审批流）；凭据保护（精确值 + ≥8 字符嵌入检测）；回环 WS + 每次启动 token；渲染进程按不可信处理。
- 流程与工具链：AGENTS.md 规范（Issue → worktree → PR → 清理）；`-s -S` 签名提交（GPG，已验证）；SSH 远程；本地按需验证（无 CI）。

### 1.3 判断

**强项**

1. 事件溯源内核（Journal + 投影 + clientRequestId 幂等）与四家编码 Agent 的会话模型同构——方向正确。
2. 协议单一来源 + 生成物检查，工程纪律在个人项目中罕见。
3. 边界写得极细（上限表、恢复语义、超时/清理/幂等），为长任务打底。
4. 长任务内核已成型：上下文预算/压缩、有界完成检查、进程树清理与输出 head/tail 保留。

**短板（按严重度）**

1. 进程不能跨 Run / 跨重启存活，无 PTY——长任务承诺的最大缺口（参考实现全部把进程当作独立基础设施）。
2. 无权限/审批层：四家编码 Agent 已收敛为「规则集 → 判定 → 上限」三层，Ikaros 仅 full_access。
3. 工具面偏窄：无检索类工具；工具结果超限直接截断，而非落盘 + 预览 + 续读。
4. Runtime 未打包：安装包不含 Python，无法分发给他人（README §4.11/§11）。
5. 记忆只有确定性关键词召回，无后台精炼/自动提取。
6. 维护面：README 112 KB 单文件；storage 单文件 105 KB。

## 二、参考仓库调研

### 2.1 编码 Agent 生态（codex / opencode / claude-code / openclaw）

**codex（openai/codex，Rust 单二进制，npm 分发壳）**

- 权限：execpolicy 前缀规则引擎（`codex-rs/execpolicy/src/rule.rs` PrefixPattern、`decision.rs` allow/ask/deny、`amend.rs` 让模型提议新规则）+ Guardian 审查 Agent（`codex-rs/core/src/guardian/`：独立 reviewer 会话裁决待批动作，带 token 预算）。
- 终端：统一 PTY（`codex-rs/core/src/unified_exec/`：process_manager 并发上限、head_tail_buffer 首尾截断、stdin_approval 写 stdin 需二次审批）。
- 上下文：`core/src/session/context_window.rs` 把 active_context_tokens 与 auto_compact_scope_tokens 分开计数。
- 启示：审查式审批可低成本替代 full_access-only；前缀规则 + 模型提议规则易落地。

**opencode（anomalyco/opencode，Bun/TS + Effect）**

- 会话事件溯源：`packages/core/src/session/event.ts`（durable/ephemeral 分离）+ `store.ts` + `projector.ts`（防重投影）+ `inbox.ts` + `run-coordinator.ts`。
- 权限规则集：`packages/schema/src/permission.ts` 的 `{action, resource, effect: allow|deny|ask}`；回复 once/always/reject；`permission-saved.ts` 按 projectID 持久化，缺省 deny。
- 持久 PTY 守护进程：`packages/core/src/persistent-pty/daemon.ts`（自带协议版本、Unix socket + token、WireTerminal、Handoff 跨重启接管）。
- 会话回滚：`snapshot.ts`（capture/diff/restore）+ `session/revert.ts`。
- 启示：projector + inbox + run-coordinator 组合可解多 Run 并发；Ruleset 是 Ikaros 权限的目标形态；persistent-pty 直接对应「进程跨 Run 存活」。

**claude-code（anthropics/claude-code，CLI 发行/扩展仓库）**

- 插件双层清单（`.claude-plugin/marketplace.json` + 各 `plugin.json`，能力面 commands/agents/skills/hooks）；hook 是外部进程协议（`plugins/hookify/hooks/hooks.json` 声明 PreToolUse/PostToolUse/Stop/UserPromptSubmit，stdin/stdout JSON + timeout）。
- 权限模式分层 + 自动审批（`--permission-mode auto` 交 classifier 判定；OTel `tool_decision` 带 classifier_verdict/permission_mode）。
- 沙箱以文件 glob 表达（`sandbox.filesystem.denyRead/allowRead/allowWrite`）；托管设置是不可越过的上限（`examples/mdm/managed-settings.json` 的 `permissions.disableBypassPermissionsMode`）。
- 启示：hook 事件面可先于完整插件系统落地；托管设置是最省的安全阀；沙箱先做文件规则层。

**openclaw（openclaw/openclaw，巨型多包 + 原生应用）**

- 进程体系独立（`src/process/` 103 文件）：`command-process-custody.ts`（start-time 身份 + 进程组，重启后仍能结算遗留进程）、`child-process-tree`/`kill-tree`、容量组队列、后台 lane 预算=3。
- Agent loop 语义：RPC 立即返回 runId，`agent.wait` 取终态（terminalReply / terminalReceipt，sourceReplyDelivered 才算送达）。
- MCP 双向：`src/mcp/channel-server.ts` 以 stdio 把自身暴露为 MCP server。
- 启示：process custody 是长任务最稳的存活方案；runId + wait 终态 + 投递回执语义可映射到 Ikaros Run/Turn。

### 2.2 桌面与助理产品（airi / AstrBot / N.E.K.O / Open-LLM-VTuber）

**airi（moeru-ai/airi，Vue3 多端角色容器）**

- 后台任务跑隐藏 session、结果以 notice 回注（ADR 2026-10-10）；`compaction.ts` 以 turn 为保留单位（recentTurnLimit + 摘要回调，块标记后跳过二次压缩）。
- `context-prompt.ts` 用扁平 [Context] 列表而非 XML（弱模型会照抄回回复），只取 text 保 KV-cache。
- `spark-notify/types.ts` 的单回合 `prepare()` 钩子（为单回合贡献 systemInstructions/tools）。
- 启示：单回合插件钩子；「申请 ∩ 授权」（`permissions.ts`）+ manifest 字段级诊断支撑交互式权限。

**AstrBot（AstrBotDevs/AstrBot，IM 机器人平台）**

- 消息管道洋葱模型（`scheduler.py`：阶段返回 async generator 即包住后续阶段并逐级短路）。
- FollowUpTicket（`tool_loop_agent_runner.py`）：运行中新消息排队，于下一工具边界并入 [SYSTEM NOTICE]。
- `_materialize_large_tool_result`：>27.5k token 的结果写文件、只回填 ~7k preview、由 read_tool 续读。
- cron 触发 CronMessageEvent 复用完整 agent 路径；future_task 做成内建工具并按 sender 校验归属。
- 启示：工具边界合并插话；大结果溢写文件 + preview + 续读；session_booter + 空闲回收 = 跨 Run 存活的现成结构。

**N.E.K.O（Project-N-E-K-O，桌面数字生命）**

- 记忆子系统：facts（SHA-256 去重 + FTS5）、reflections、persona 三层；scope 缺字段解释为 legacy_private 而非全局。
- `hybrid_recall.py`：BM25 池（含 archive）+ 向量池 + RRF(k=60)，不做 LLM 精排（工具在等人）。
- reflection 的 temporal_scope（pattern 永不过期 / state / episode）驱动衰减；`temporal.py` 让 LLM 只输出相对时间、系统换算 ISO。
- `proactive_delivery.py`：优先级 + coalesce_key 合并 + 门控 + min-gap + 超 ttl_s 丢弃。
- 启示：记忆分层 + scope + 时间衰减 + 后台精炼；主动唤醒需要同款优先级/合并/门控/TTL 节流。

**Open-LLM-VTuber（离线语音伴侣）**

- 每连接一个 DI 容器（`service_context.py` 装配各引擎）；Agent = 无状态 LLM + 记忆包装（`agents/` 提供 basic_memory/letta/mem0）。
- `json_detector.py` 的 StreamJSONDetector 在增量文本里边流边试解析完整 JSON，为无原生 function calling 的模型兜底。
- 启示：每会话一个装配根；流式 JSON 工具调用探测兼容弱 Provider。

### 2.3 Agent 基础设施（honcho / hermes-agent / nanobot / pi / demiurge-agent）

**honcho（plastic-labs/honcho，记忆服务）**

- 后台 deriver 消费队列（`src/deriver/queue_manager.py`：work_unit 键保证同会话顺序 + 3 次重试预算）；结论分 explicit/deductive/inductive/contradiction 四级，带可引用 id（`src/utils/memory_brief.py`）。
- dreamer 离线周期（`src/dreamer/`：搜索—写入专家、几何 surprisal 采样、working notes 按「梦次数」过期、新消息取消未跑 dream）。
- dialectic 检索 agent 有工具调用预算与「下次调用就该作答」提示（`src/dialectic/core.py`）。
- 启示：记忆做成后台派生投影而非主链路内联摘要；结论级 provenance；检索预算提示。

**hermes-agent（NousResearch/hermes-agent，Python 单体全家桶）**

- MemoryManager 每轮把钩子扇出到 provider（内置 + 最多一个外部插件，`agent/memory_manager.py`）；provider v2 要求压缩前 checkpoint fail-closed。
- 压缩分层：保护 head/tail、先剪工具输出、micro 与 batch 两级摘要（`agent/context_compressor.py`、`agent/micro_compaction.py`）。
- 自研 FTS5 CJK bigram 分词器（`hermes_state_fts.py`：1–2 字中文查询不再退化为全表 LIKE）；回合级 rewind（持久历史为权威、warm history 只是投影）。
- 终端工具 notify_on_complete / watch_patterns；异步委托结果作为新回合投递（`tools/async_delegation.py`，迭代预算父 500 / 子 50）。
- 启示：压缩前 checkpoint；中文 FTS；后台完成通知；迭代预算。

**nanobot（HKUDS/nanobot，轻量 asyncio agent）**

- Dream（`nanobot/agent/memory.py`）：按 cursor 增量取未处理 history，用受限工具集改写 SOUL.md/USER.md/MEMORY.md，git diff 作提交依据。
- exec_session（`nanobot/agent/tools/exec_session.py`：yield/wait/until-exit、idle_timeout、输出上限截断）；ManagedProcessRuntime（`nanobot/process_runtime.py`：跨平台分离进程 + 日志轮转）。
- cron 不另开执行路径，落成普通会话回合（`nanobot/cron/session_turns.py`）。
- 启示：Dream/cron 表达为普通 session turn 以复用恢复与投递；exec_session 语义；记忆文件 git 版本化。

**pi（earendil-works/pi，TS 极简 harness）**

- 会话是 JSONL 树，entry 带 id/parentId 可就地分支（`packages/coding-agent/docs/session-format.md`）。
- 压缩写 CompactionEntry + firstKeptEntryId，尾部保留 keepRecentTokens；/tree 切分支另写 BranchSummaryEntry（`docs/compaction.md`）。
- pi-env 用长度前缀帧协议把 exec 与 pty 分开（`packages/env/daemon/src/pty.rs`：ptyWrite/ptyResize/ptyForeground；输出超阈值 spill 到文件）。
- durable 规范：Session 原子提交 entry/task/document，只有已提交状态可观测（`packages/durable/docs/spec.md`）。
- 启示：会话树 + entry 级压缩/分支摘要可映射 Journal + 投影；pty/exec 分协议是 PTY 缺口的现成设计。

**demiurge-agent（AllenReder/demiurge-agent，文件化 Agent Core）**

- 槽位（slot.yaml）声明 capability / failure_policy / history_policy；host 独占调度、审批、投递、状态（`docs/reference/slots-yaml.md`）。
- 审批预览裁剪 + 命令中密钥重写（`demiurge/security/approval.py`）；RuntimeTask 状态机含 blocked_needs_user/lost（`demiurge/runtime/tasks.py`）。
- 子 agent 带 scope、槽位与工具白名单（`runtime/child_agents.py`）。
- 注意：其 `gates/`、`evolution/` 目录实际只有 `__init__.py`，文档承诺大于实现。
- 启示：声明式槽位边界（失败/历史策略）可用于上下文装配；子 agent 工具白名单。

## 三、跨仓共识（12 个仓库）

1. **权限三层**：规则集（opencode Ruleset / codex execpolicy）→ 判定（claude classifier / codex Guardian）→ 上限（claude 托管设置 / openclaw delegated authority）。Ikaros 仅 full_access 是最大安全落差；共识是先做规则集 + 审批点，不必先做内核级沙箱。
2. **会话事件化 + 可重建**：codex rollout 重建、opencode projector/revert/snapshot、pi durable 原子提交、openclaw versioned state。Ikaros 的 Journal + 投影方向正确；下一步补「投影幂等重建」与「重启后进程结算」。
3. **进程/终端是独立基础设施**：codex unified_exec、opencode persistent-pty daemon、openclaw process custody、nanobot ManagedProcess、pi env 守护进程。跨 Run / 跨重启存活是长任务产品的分水岭。
4. **记忆 = 后台派生 + 分层 + 混合检索**：honcho 四级结论 + dreamer；hermes provider 钩子 + FTS5；nanobot Dream 写文件；N.E.K.O BM25+向量+RRF。Ikaros 的 Journal 是天然派生源，起步用 FTS5 + RRF 即可。
5. **长任务 = 节流 + 可插话 + 大结果落盘 + 完成通知**：FollowUpTicket、notify_on_complete、exec_session、输出 spill 到文件。
6. **扩展面必须设预算**：openclaw shrink-only 表面预算 + CI import guard；claude hook timeout / 托管上限。上插件/MCP 前先定 manifest 与权限契约，否则复杂度爆炸。

## 四、建议方向

### 方向 A：权限/审批层（规则集 + 审批点）

- 内容：`{action, resource, effect: allow|deny|ask}` 规则集（按 Thread/Project 持久化）；命令前缀规则（execpolicy 风格）；Desktop 审批 UI（once/always/reject）；协议/事件/schema 扩展 + golden trace 更新。
- 参考：opencode `schema/src/permission.ts`、`permission-saved.ts`；codex `execpolicy/`；claude managed settings。
- 收益：修复最大安全缺口；为 MCP/分发建立信任基础；中等工作量。
- 风险：资源粒度/通配/继承语义要一次设计清楚，避免反复改协议。

### 方向 B：进程持久化/PTY + 长任务双平台验收

- 内容：PTY 会话（交互式 stdin/resize）；进程跨 Run 存活；Runtime 重启后对遗留进程「结算」（身份 + 进程组 custody 语义）；随后执行 README §10 的长任务验收（30+ 步、2+ 压缩、1 MiB 输出、64 并发、断线重连、Win/Linux）。
- 参考：opencode `persistent-pty/daemon.ts`；openclaw `command-process-custody.ts`；codex `unified_exec`；pi pty 帧协议。
- 收益：兑现「长任务执行内核」的产品承诺，对齐 README 已声明的验收里程碑。
- 风险：工作量最大；双平台 PTY 差异需要真实机器验证。

### 方向 C：工具面升级（检索工具 + 大结果落盘）

- 内容：新增 grep / glob / list 类原生工具（不依赖 shell 差异）；工具结果超限改为「落盘 + preview + 续读」（复用 read 的翻页语义与 process 的 head/tail 思路）。
- 参考：AstrBot `_materialize_large_tool_result`；codex/claude 检索工具面。
- 收益：日常可用性立刻提升；为长任务（大日志/大文件）减负；工作量中等。

### 方向 D：Runtime 打包 + 分发

- 内容：把 Python Runtime 打进安装包（Windows/Linux）；签名、自动更新可分期（先打包，后签名）。
- 参考：codex npm 分发壳（按平台映射二进制）；claude-code 安装渠道（curl/irm/brew/winget）。
- 收益：从「自己用」到「能给别人装」；README §4.11 明确缺口。
- 风险：PyInstaller/嵌入式 Python 与 uv 生态的兼容成本；体积。

### 方向 E：MCP client（生态接入）

- 内容：实现 MCP client（stdio/HTTP），把外部 MCP server 工具并入工具面；配套工具审批与命名空间隔离。
- 参考：openclaw `src/mcp/channel-server.ts`；codex `mcp_tool_approval_templates.rs`。
- 收益：以最低成本接入生态（不自建插件 SDK）；中等偏上工作量。
- 前提：先做方向 A（MCP 工具必须有审批归属）。

### 方向 F：记忆升级（FTS5 + RRF + 后台精炼）

- 内容：检索改为 FTS5(BM25) + 向量混合 + RRF；后台精炼（Dream/curator 式：从 Journal 派生候选事实，确认后写入）。
- 参考：N.E.K.O `hybrid_recall.py`；hermes FTS5 CJK bigram；honcho dreamer；nanobot Dream。
- 收益：「助理会记事」的质量跃迁；中等偏上工作量。
- 前提：中文分词方案先定（hermes bigram 可直接借鉴）。

### 建议排序

1. **C（工具面）**——最小成本、立刻改善日常使用，并为 B 的大输出场景减负；
2. **A（权限）**——安全缺口，且是 MCP/分发的前置；
3. **B（进程持久化/PTY + 验收）**——旗舰承诺，工作量最大，放在 C/A 之后心智负担更小；
4. **D（打包）**——准备对外分发时做；
5. **E（MCP）**——A 完成后接入生态；
6. **F（记忆）**——体验型投入，可并行评估。

若只推进一件「有里程碑意义」的事，选 B；若先要日常体验收益，选 C。

## 五、杂项

- Issue #10（2026-06，引用 Rust 时代 `LocalExecutionEnv::run_process`）已不适用于当前 Python Runtime，建议关闭。
- README 112 KB 单文件偏重，可考虑后续拆分（低优先）。
- `.temp/` 参考仓库均处于活跃开发（2026-09~10），后续实现方向 B/C/E 时应重新拉取对应仓库核对最新实现。