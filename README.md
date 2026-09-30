# Ikaros 详细说明

Ikaros 是一个本地通用 Agent 应用：Electron 桌面客户端 + 常驻 Python Runtime，
支持 Windows 和 Linux。面向多轮对话、长任务执行、托管命令、文件读写、Memory、
Skill 扩展和多 Provider 模型接入，不局限于编码场景。

当前状态：可日常使用的 Alpha + 长任务执行内核（无总调用次数/时长上限、自动
上下文压缩、完成检查）。真实 Provider 的双平台验收仍在进行。

> 本文按模块描述"现状"，记录实现细节、重要常量和参数，用于快速索引；不是设计
> 契约，具体行为以代码为准。核对日期：2026-10-01。
> 阅读顺序：架构与技术栈（第 1–2 节）→ 逐文件结构（第 3 节）→ 桌面端与界面交互
> （第 4–5 节）→ Runtime 与接口参考（第 6–8 节）→ 数据目录、验证与速查（第 9–13 节）→ 故障排查（第 14 节）。

## 目录

- [1. 架构总览](#1-架构总览)
- [2. 技术栈与版本](#2-技术栈与版本)
- [3. 项目文件结构（逐文件）](#3-项目文件结构逐文件)
- [4. 桌面客户端](#4-桌面客户端)
- [5. 界面交互细节](#5-界面交互细节)
- [6. Runtime](#6-runtime)
- [7. RPC 接口参考](#7-rpc-接口参考协议-7共-32-个方法)
- [8. 数据库表结构（列级）](#8-数据库表结构列级)
- [9. 数据目录](#9-数据目录)
- [10. 验证与测试现状](#10-验证与测试现状)
- [11. 当前边界](#11-当前边界)
- [12. 常用命令速查](#12-常用命令速查)
- [13. 关键常量速查](#13-关键常量速查)
- [14. 故障排查](#14-故障排查)
- [15. 许可证](#15-许可证)

## 仓库结构

- `ikaros/desktop/`：桌面客户端（React renderer + Electron main）
- `runtime/`：Python Runtime
  - `src/ikaros_runtime/agent/`：Agent 循环、调度、上下文选择与压缩
  - `src/ikaros_runtime/providers/`：Provider 注册表与 OpenAI 兼容适配器
  - `src/ikaros_runtime/tools/`：8 个内建工具与进程管理
  - `src/ikaros_runtime/storage/`：SQLite 存储、Journal、投影、维护命令
  - `src/ikaros_runtime/memory/`：Memory 存储与召回
  - `src/ikaros_runtime/skills/`：Skill 目录扫描与提示词
  - `src/ikaros_runtime/server/`：WebSocket 服务、连接与事件广播
  - `src/ikaros_runtime/protocol/`：协议规范与生成器
- `runtime/protocol/`：生成物 `runtime-protocol.json`、`golden-trace.json`

## 快速开始

环境要求：Node.js 22.12+、pnpm 11.9、uv、Python 3.12 或 3.13。

```shell
uv sync --project runtime --locked
cd ikaros/desktop
pnpm install
pnpm dev
```

验证与打包（`ikaros/desktop/` 下）：

```shell
pnpm check          # typecheck + vitest + build
pnpm package:dir    # 目录形式打包
pnpm package:linux  # AppImage / DEB / RPM（x64，未签名）
pnpm package:win    # NSIS 安装器（x64，未签名）
```

Runtime 校验（仓库根目录）：

```powershell
uv run --project runtime ruff check runtime
uv run --project runtime mypy runtime/src runtime/tests
uv run --project runtime pytest runtime/tests
uv run --project runtime python -m ikaros_runtime.protocol.generate --check
```

运行数据默认位于 `~/.ikaros/`（Windows：`%USERPROFILE%\.ikaros`）。

## 1. 架构总览

### 1.1 分层与进程模型

整个应用由两个进程域组成：

```text
┌─ Electron 主进程（TypeScript / Node）───────────────────────────────┐
│  窗口与标题栏 · 原生偏好(ui-preferences.json) · 窄 IPC 桥           │
│  RuntimeHost：启动/认证/监督 Python Runtime，WS 重连与进程重启      │
└──────────────────────────────┬──────────────────────────────────────┘
                               │ 类型化 preload API（contextBridge）
┌──────────────────────────────┴──────────────────────────────────────┐
│  Electron 渲染进程（React）：纯呈现层                               │
│  store.ts 状态 · runtimeClient RPC 封装 · 事件投影 · i18n           │
│  不直接接触 WebSocket / IPC 原生对象 / 文件系统                     │
└─────────────────────────────────────────────────────────────────────┘
                               │ 回环 WebSocket JSON-RPC（Bearer token）
┌──────────────────────────────┴──────────────────────────────────────┐
│  Python Runtime（单进程，asyncio）                                  │
│  server ── protocol.router ── services（threads/turns/provider/…）  │
│  agent.loop + scheduler ── providers（OpenAI 兼容适配器）           │
│  tools（8 个）+ ProcessManager ── storage（SQLite/Journal/投影）    │
│  memory（memory.db）· skills（目录扫描）· config（config.yaml）     │
└─────────────────────────────────────────────────────────────────────┘
```

进程与并发模型：

- 每个应用生命周期只有 1 个 Python Runtime 进程；通过 `runtime.lock` 独占
  `~/.ikaros`，启动第二个实例会失败而不共享状态。
- Runtime 内部是单事件循环（asyncio）；调度器只有 1 个 worker，同一时刻只运行
  1 个 Run；同一 Run 内工具串行执行；每个 Branch 拥有串行队列。
- 数据库访问在单个连接上串行化（SQLite WAL）；`state.db` 与 `memory.db` 是两个
  独立数据库，只共享 Runtime-home 锁。
- Desktop 与 Runtime 的生命周期绑定：关闭应用会先请求 Runtime 干净关闭。

### 1.2 一次 Turn 的完整链路

1. **提交**：Composer → 渲染 store → preload → Electron IPC → RuntimeHost →
   回环 WS `turn.start`（携带 threadId / branchId / content / providerId /
   modelId / clientRequestId）。
2. **受理**：`protocol.router` 派发到 `services.turns`；在一个 SQLite 事务里追加
   用户 Item + Turn + `queued` Run + 不可变 RunConfig；先回 ACK，再发布初始事件并
   激活 Run（ACK 优先边界，见 6.2）。
3. **准备输入**：Scheduler 预留并启动 Run → AgentLoop 在事务外选择 Memory
   （冻结 revision）→ HistorySelector 选择历史（32 Turn/页、连续后缀、工具成对）
   → 生成 ContextRevision 与本次调用的 StepInput → 发布 `model.input_prepared`。
4. **模型调用**：ContextBuilder 把指令、身份、Skill 目录、历史、Memory、状态块
   组装为 ProviderRequest → OpenAI 兼容适配器发起流式请求（连接 10s / 响应头
   30s / 流空闲 300s；请求重试 ≤4、流重试 ≤5）。
5. **流式落盘**：第一个安全文本 delta 立即持久化；后续 delta 合并到 256 字符或
   50ms；工具调用参数分片按 index/ID 累积，完成后解析并校验 schema。
6. **工具执行**：ToolExecutor 以可信上下文（Run/Step/Item/Thread/默认 cwd）串行
   执行；进程与文件工具分别走 ProcessManager 与文件路径锁；工具结果与终态 Item
   同事务提交；`process.recorded` / `file.change_recorded` 独立记录操作事实。
7. **继续或终结**：若模型请求了工具或需要完成检查，进入下一个 Step；否则候选
   答案在有未完成工作时执行一次有界完成检查，然后以 `run.settled` 终结
   （completed / failed / cancelled + reasonCode）。
8. **界面投影**：EventHub 按 `seq` 扇出 → WebSocket `event` 通知 → RuntimeHost
   校验 → 渲染 store 投影；历史页与实时事件按 `snapshotSeq`/`seq` 去重合并。

### 1.3 事件、存储与恢复模型

- 单一事实来源：`state.db` 的 `events` 表是 append-only Journal，`seq` 全局单调
  递增；Thread/Turn/Run/Item 查询表都是可重建投影。
- 恢复：异常退出后 `running` → `failed(runtime_interrupted)`，`queued` 按原顺序
  重排，进程记录 → `unknown`；终态 Run 永不重跑。
- 重放：客户端从最后连续 `seq` 调用 `event.replay` 追赶，遇到缺口不推进游标；
  目录/历史页返回 `snapshotSeq` 水位线用于对齐。
- 三个独立数据域：`state.db`（会话与执行）、`memory.db`（长期记忆）、
  `config.yaml`（Provider/Skill 配置）；互不迁移，Session 重置不影响后两者。
- 无旧格式迁移：schema/协议变更通过显式重置处理，启动不会自动删除数据。

### 1.4 信任与安全边界

- 渲染进程视为不受信：只能通过 preload 暴露的窄 API 访问 Runtime；窗口安全策略
  在 `main/security.ts` 中集中配置。
- Runtime 只绑定回环地址；连接需要每次启动生成的随机 Bearer token；stdio 不承载
  业务流量。
- 凭据保护：精确值匹配 + ≥8 字符嵌入检测；凭据不进入 Journal、事件、日志、
  列表响应、异常或 UI 投影。
- 执行权限：当前固定 `full_access`——工具以当前操作系统用户权限运行，无审批流，
  不是沙箱；超时、输出上限、环境白名单和进程树清理仍然生效。
- 输出防护：进程输出、文件捕获、事件负载、配置文档都有明确的字节/行数上限。

### 1.5 数据所有权

| 数据 | 所有者 |
| --- | --- |
| Threads / Turns / Runs / Items、工具执行、执行策略、技能目录、Memory、Provider 配置 | Runtime |
| 主题、语言、字体、侧边栏状态、用户名、草稿、所选 Model、展开分组、设置导航 | Desktop |
| 文件系统内容 | 用户（工具以用户权限访问） |
| 会话 SQLite、Memory SQLite、配置文档 | `~/.ikaros`（Runtime 独占管理） |

## 2. 技术栈与版本

- Desktop：Electron 43.3.0、React 19.2.8、TypeScript 7.0.2、Vite 7.3.6、
  electron-vite 5.0.0、Vitest 4.1.10、Tailwind CSS 4.3.3、Radix UI、lucide-react、
  react-markdown、@tanstack/react-virtual、ws 8.18.3、electron-builder 26.15.3。
- Runtime：Python 3.12–3.13（`>=3.12,<3.14`）、httpx 0.28.x、websockets 15.x、
  PyYAML 6.x；构建后端 Hatchling。
- 包管理与锁：uv（`runtime/uv.lock`）、pnpm 11.9（`pnpm-lock.yaml`，CI 使用
  frozen lockfile）。
- Desktop 脚本：`dev` / `build` / `preview` / `typecheck` / `test` /
  `test:live:deepseek` / `check` / `package:dir` / `package:linux` / `package:win`。

## 3. 项目文件结构（逐文件）

> 不包含 `.temp/`（本地参考仓库克隆）、`node_modules/`、构建产物（`out/`、`dist/`）
> 与锁文件内容。每个文件后面是一句话作用说明。

### 3.1 根目录

- `.gitattributes` — 行尾与文本属性规则。
- `.gitignore` — 忽略构建产物、依赖与本地状态。
- `LICENSE` — GPL-3.0-only 许可证全文。
- `README.md` — 本文档（架构 + 功能 + 参数 + 文件结构 + 接口参考）。
- `.github/workflows/check.yml` — GitHub Actions 门禁：Runtime 矩阵（ubuntu +
  windows × Python 3.12/3.13：ruff、mypy、协议生成检查、pytest）与 Desktop 矩阵
  （ubuntu + windows：typecheck、Vitest、生产构建）。

### 3.2 runtime/（Python Runtime）

配置与协议文件：

- `pyproject.toml` — 包元数据（`ikaros-runtime` 0.1.0）、依赖（httpx / websockets /
  PyYAML）、Python 版本约束（>=3.12,<3.14）、脚本入口 `ikaros-runtime`。
- `uv.lock` — uv 锁定依赖。
- `protocol/runtime-protocol.json` — 生成的机器可读协议清单：协议版本、RPC 方法、
  事件类型、provider 工具 ID、错误定义。
- `protocol/golden-trace.json` — 跨语言协议回放用例（Python 与 Desktop 测试共用）。

`src/ikaros_runtime/` 顶层：

- `__init__.py` — 包标记。
- `__main__.py` — CLI 入口：`serve`（WebSocket 服务）与 `storage check / backup /
  repair-projections` 子命令；输出机器可读 JSON。
- `bootstrap.py` — 唯一生产组合根：取锁 → 路径 → 身份 → 配置 → 存储 → Memory →
  服务装配 → 启动 server；关闭时按序释放。
- `cancellation.py` — 协作取消令牌（CancellationToken），供循环与工具共享。
- `config.py` — `config.yaml` 单一所有者：版本 2、256 KiB 上限、原子替换、Provider/
  model/Skill 禁用列表读写与凭据校验。
- `domain.py` — 领域 DTO：WorkspaceSummary、ThreadSummary、JournalEvent、
  SkillDescriptor、RunDescriptor、ContextItem、Usage 等。
- `errors.py` — 稳定错误类型与原因码：ProviderFailureCategory、Memory 操作/召回
  原因码、RunInputDriftError、ContextBudgetExceededError 等。
- `file_changes.py` — 文件变更捕获模型：256 KiB / 5,000 行上限、事件序列化上限。
- `file_preview.py` — 只读文件预览分页：50 KiB / 2,000 行、8 MiB 扫描上限、
  不可用原因枚举。
- `history_status.py` — 无正文的冻结状态块，描述未完成/失败的历史 Run。
- `identity.py` — 加载打包资源 `IKAROS.md` 并提供身份 id/version/source。
- `json_codec.py` — 严格 JSON 编解码：拒绝非有限数、超 JS 安全整数、过深嵌套。
- `maintenance.py` — 维护命令应用边界：与 server 共用 Runtime-home 锁执行
  check/backup/repair。
- `paths.py` — Runtime home 的规范路径集合（config/state/memory/lock/backups/
  skills/logs）。
- `run_input.py` — 输入快照与预算核心：RunConfig / ContextRevision / StepInput /
  ModelInputPlan、95% 预算、90% 压缩阈值、Memory 选择上限等常量。
- `security.py` — 凭据保护：精确值匹配与 ≥8 字符嵌入检测，覆盖请求、Journal、
  日志与输出。

`agent/`（Agent 执行）：

- `__init__.py` — 导出 Agent 循环与调度器。
- `compaction.py` — 确定性裁剪（保留工具单元、700 字节记录截断）与压缩源构建。
- `context.py` — 上下文构建：指令、历史、Memory 包装、状态块到消息的装配。
- `history.py` — 历史选择：32 Turn/页、连续后缀、工具成对、省略边界。
- `loop.py` — Agent 主循环（1,219 行）：执行 Run、流式处理、工具调度、压缩、
  完成检查与终结。
- `model_input.py` — 单步模型输入计划（ModelInputPlan）与校验。
- `scheduler.py` — 单 worker 调度：预留/激活/取消/steer 与关闭顺序。

`memory/`（长期记忆）：

- `__init__.py` — 导出 Memory 存储与召回。
- `domain.py` — Memory 领域类型：kind/state/scope、来源、revision、内容上限。
- `retrieval.py` — 确定性关键词召回：候选 ≤2,000、选中 ≤8 条 / 6,000 字符。
- `schema.py` — memory.db schema 1 的 DDL 与逐字节校验。
- `store.py` — memory.db SQLite 所有者（870 行）：CRUD、revision、墓碑、操作回执。

`protocol/`：

- `__init__.py` — 包标记。
- `generate.py` — 生成 `runtime-protocol.json` 与 Desktop 侧 TS 常量；支持 `--check`。
- `jsonrpc.py` — JSON-RPC 信封与错误对象构造。
- `router.py` — 方法派发：32 个 RPC → 各服务；错误码映射（-32602 / -32010 / -32020）。
- `spec.py` — 协议注册表：版本 7、32 方法、15 事件、8 工具 ID（手工维护唯一来源）。

`providers/`：

- `__init__.py` — Provider 契约导出。
- `base.py` — ProviderAdapter 协议、ProviderRequest 与内部事件词汇。
- `model_defaults.py` — 默认上下文窗口：DeepSeek V4 三个模型 1,000,000；未知 32,768。
- `registry.py` — Provider/model 配置解析、注册表、DeepSeek 发现、密钥保护
  （752 行）。
- `scripted.py` — 测试用确定性 Provider（`/process.run`、`/skill.run` 约定）。
- `openai_compatible/__init__.py` — 适配器导出。
- `openai_compatible/adapter.py` — 流式请求、重试（4/5）、超时（10/30/300 秒）、
  错误归一化（581 行）。
- `openai_compatible/discovery.py` — `/models` 发现：响应 ≤1 MB、≤1,000 个模型。
- `openai_compatible/streaming.py` — SSE 解析、工具参数分片、usage 装配。

`server/`：

- `__init__.py` — 服务导出。
- `connection.py` — 单条已认证连接：`initialize` 握手、方法准入、事件订阅、ACK 优先
  发送与发送超时（5 秒）。
- `event_hub.py` — 有序事件扇出：把 Journal 事件按连续 `seq` 通知各连接。
- `host.py` — WebSocket 服务与 Runtime-home 锁获取（超时 10 秒、轮询 50 ms）。

`services/`（RPC 应用服务）：

- `__init__.py` — 服务导出。
- `files.py` — `file.preview` / `file.change.get`：线程内只读文件访问与变更查询。
- `memories.py` — `memory.*`：幂等创建/纠正/遗忘、分页列表、来源校验。
- `processes.py` — `process.read` / `process.stop`：在线进程查询与重启后的历史回退。
- `providers.py` — `provider.*` / `model.*`：配置、发现、断开、移除、启用与容量设置。
- `skills.py` — `skill.list` / `skill.set_enabled`：全局技能发现与启用状态。
- `threads.py` — `thread.*`：创建（幂等）、改名、归档/取消归档、目录分页、详情。
- `turns.py` — `turn.start/list`、`run.cancel`、`run.steer`、`event.replay`。
- `usage.py` — `usage.read`：只读 usage 聚合。

`skills/`：

- `__init__.py` — Skills 导出。
- `catalog.py` — 单层目录扫描与校验：名称规则、256 KiB 文件、16 KiB frontmatter、
  越界路径拒绝、有界诊断。
- `prompt.py` — 冻结 Skill 描述符的 Provider 提示词片段。

`storage/`：

- `__init__.py` — 存储门面导出。
- `context_history.py` — 模型可见历史的分页 SQLite 读取与装配。
- `file_changes.py` — 按 Tool Call 查询不可变文件变更捕获。
- `history_read.py` — `history_read` 的存储实现（41 条 / 24,000 字符边界）。
- `journal.py` — Journal 追加与读取、事件 schema 版本校验。
- `maintenance.py` — check / backup / repair 的完整实现与校验流程。
- `processes.py` — 进程事实读取；重启恢复只回放观察，永不启动命令。
- `projections.py` — 投影重建与一致性校验（1,894 行）。
- `schema.py` — state.db schema 14 的规范 DDL。
- `session_provenance.py` — Memory 来源 Item 的只读校验（schema 1）。
- `store.py` — 主 SQLite 存储（2,750 行）：Threads/Turns/Runs/Items/Journal/
  进程/文件/usage 的全部查询与事务。
- `thread_catalog.py` — 目录 keyset 分页（默认 50、最大 100、作用域绑定游标）。
- `thread_history.py` — 回合分页与子记录装配（默认 50、最大 100）。
- `usage.py` — usage 聚合：生命周期/单日峰值/最长任务/连续天数/每日桶。

`tools/`：

- `__init__.py` — 工具注册与导出。
- `core.py` — Tool 契约、ToolExecutionContext、参数精确校验原语。
- `edit.py` — `edit`：精确匹配、唯一匹配默认、replaceAll、stale_content 检测。
- `file_common.py` — 文件工具共享行为：路径校验（≤4,096）、二进制嗅探、按路径锁、
  原子写。
- `history_read.py` — `history_read` 工具：itemId 周边的有界切片。
- `policy.py` — 执行策略：当前固定 FullAccessPolicy。
- `process_manager.py` — 托管进程管理器：64 并发、1 MiB 保留、4 条/秒快照、
  终止 6 秒 / 排空 2 秒。
- `process_platform.py` — 平台原语：Windows 挂起创建 + Job Object；POSIX 进程组；
  环境白名单 + UTF-8 变量。
- `process.py` — 4 个进程工具（start/read/wait/stop）的模型侧定义与参数校验。
- `read.py` — `read`：≤2,000 行 / 50 KiB、行截断 2,000 字符、nextOffset 翻页。
- `write.py` — `write`：临时文件 + SHA-256 校验 + fsync + 原子替换。

`tests/`（Runtime 测试，共 37 个 Python 文件）：

- `__init__.py` — 测试包标记。
- `golden_trace.py` — golden trace 加载与回放辅助。
- `helpers.py` — 测试辅助：临时 Runtime home、fixture 构造。
- `test_agent_context.py` — 上下文构建测试。
- `test_agent.py` — Agent 循环行为测试。
- `test_compaction.py` — 裁剪与压缩测试。
- `test_config_document.py` — config 文档读写与原子性测试。
- `test_config.py` — 配置解析、版本与凭据测试。
- `test_file_changes.py` — 文件变更捕获测试。
- `test_file_preview.py` — 预览分页与不可用状态测试。
- `test_history_read.py` — `history_read` 边界测试。
- `test_history_v2.py` — 失败后继续的整链路测试（真实 provider 请求序列）。
- `test_history.py` — 历史选择规则测试。
- `test_identity.py` — 身份加载与冻结测试。
- `test_json_codec.py` — 严格 JSON 规则测试。
- `test_live_identity.py` — 真实 DeepSeek 身份验收（opt-in，需 key file）。
- `test_long_task_kernel.py` — 进程事实、重启边界与取消测试。
- `test_maintenance.py` — check/backup/repair 测试。
- `test_memory_recall.py` — Memory 召回整链路测试。
- `test_memory_retrieval.py` — 召回算法与预算测试。
- `test_memory.py` — Memory 存储、幂等、墓碑测试。
- `test_model_defaults.py` — 默认上下文窗口测试。
- `test_model_input_pipeline.py` — RunConfig → StepInput 流水线测试。
- `test_model_input.py` — 输入快照校验测试。
- `test_openai_compatible.py` — 适配器、流式、重试与错误归一测试。
- `test_paths.py` — Runtime home 路径测试。
- `test_process_manager.py` — 进程管理器与平台行为测试。
- `test_protocol_contract.py` — 协议契约与生成物测试。
- `test_protocol_router.py` — 方法派发与错误码测试。
- `test_runtime_lock.py` — 单实例锁测试。
- `test_schema.py` — schema 14 校验测试。
- `test_server.py` — WebSocket 服务与连接测试。
- `test_skills.py` — Skill 目录扫描与诊断测试。
- `test_storage.py` — 存储、Journal 与投影测试。
- `test_thread_catalog.py` — 目录分页与游标测试。
- `test_thread_history.py` — 回合历史分页测试。
- `test_tools.py` — 文件工具（read/write/edit）测试。

### 3.3 ikaros/desktop/（Electron 桌面客户端）

配置与脚本：

- `electron-builder.config.ts` — 打包配置：Linux（AppImage/DEB/RPM，x64）、Windows
  （NSIS，x64）、应用 ID、图标与安装项。
- `electron.vite.config.ts` — electron-vite 构建配置（main / preload / renderer）。
- `package.json` — 脚本与依赖（Electron 43、React 19、Vite 7、Vitest 4 等）。
- `pnpm-lock.yaml` / `pnpm-workspace.yaml` — 依赖锁与工作区定义。
- `tsconfig.json` / `tsconfig.node.json` / `tsconfig.web.json` / `tsconfig.live.json`
  — TypeScript 项目引用与三套构建/测试配置。
- `vitest.config.ts` — 测试环境配置。
- `scripts/run-live-deepseek.mjs` — live smoke 运行器：读取 key file、执行真实
  DeepSeek 用例并扫描输出中的凭据（PASS/FAIL）。

`src/main/`（Electron 主进程）：

- `index.ts` — 主进程入口：装配窗口、IPC、RuntimeHost 与应用生命周期。
- `ipc.ts` — 类型化 IPC 注册与转发（`registerDesktopIpc`，559 行）。
- `lifecycle.ts` — 退出前等待 Runtime 干净关闭。
- `preferences.ts` — `ui-preferences.json` 读写、校验、原生主题同步（330 行）。
- `runtime/wire.ts` — 线上 DTO 解析与校验：事件、目录页、历史页、Memory、文件、
  进程（3,019 行）。
- `runtimeHost.ts` — Runtime 子进程监督：启动/认证/WS 重连/进程重启/状态通知
  （1,156 行）。
- `security.ts` — 渲染信任策略与窗口安全设置。
- `window.ts` — 创建主窗口、标题栏选项、开发/生产渲染地址。

`src/preload/`：

- `index.ts` — contextBridge 暴露的类型化 API：runtime / workspace / preferences /
  windowControls。

`src/shared/`（主/渲染共用）：

- `generated/runtimeProtocol.ts` — 由 Runtime 生成的协议常量与类型（版本、方法、
  事件、工具 ID、错误定义）。
- `platform-global.d.ts` — `window.ikaros` 全局类型声明。
- `platform.ts` — 偏好类型与默认值：语言、字体、侧边栏 220–520、用户名 ≤32、主题
  结构与桌面 API 接口。
- `runtime.ts` — 全部 Runtime DTO 类型：Thread/Turn/Run/Item、进程、文件、Memory、
  Provider/Model、usage（733 行）。
- `theme.ts` — 颜色工具：十六进制规范化、混色、可读文字色。

`src/renderer/`（React 渲染层）：

- `index.html` / `main.tsx` — 渲染入口 HTML 与 React 挂载。
- `App.tsx` — 根组件：Runtime 初始化与重试、全局快捷键（Ctrl/Cmd+K 搜索、
  Ctrl/Cmd+N 新对话）、设置页与对话页切换。
- `store.ts` — 全局状态与动作（3,653 行）：Thread/详情缓存、Run 活动投影、草稿、
  设置导航、文件面板等。
- `runtimeClient.ts` — WebSocket JSON-RPC 客户端封装与错误类型。
- `runtimeProjection.ts` — 事件 → UI 投影：目录、Thread、历史、Projects。
- `runtimeHistory.ts` — 历史分页加载（每页 ≤100）。
- `runtimeStore.liveEvidence.ts` — live 证据校验（工具配对完整性）。
- `domain.ts` — 渲染领域类型与查找辅助（threads/projects/事件文本）。
- `eventCopy.ts` — 事件与中断原因的本地化文案解析。
- `i18n.ts` — en / zh-CN 双语文案表与 `useTranslation`（1,115 行）。
- `platformPreferences.ts` — 偏好读取 hook。
- `applyUiPreferences.ts` — 把偏好应用到文档（主题、字体、动效、颜色变量）。
- `profileUsage.ts` — Profile 图表数据：周/日网格、月份标签、紧凑数字格式。
- `localProfile.ts` — 本地用户名默认值与首字母。
- `mockAgentClient.ts` — 测试/非 Electron 开发用 mock 客户端（690 行）。
- `styles.css` — Tailwind 入口与全局样式（695 行）。

`src/renderer/components/`（界面组件）：

- `AppearanceSettings.tsx` — 外观设置：主题、颜色、字体、侧边栏、动效。
- `ArchivedThreadsDialog.tsx` — 归档对话管理对话框（取消归档、空态、错误态）。
- `Composer.tsx` — 输入框：发送/停止、草稿、slash 命令菜单、权限提示（415 行）。
- `ConversationHeader.tsx` — 会话标题栏：重命名/归档菜单、Model 标识。
- `CreateProjectDialog.tsx` — 新建项目（选择工作区文件夹）。
- `EditMessageDialog.tsx` — 编辑消息对话框（mock 功能）。
- `EventCard.tsx` — 事件/工具卡片：命令、文件、状态、diff 入口（674 行）。
- `EventFeed.tsx` — 对话事件流：虚拟滚动、占位、回合导航快捷键（539 行）。
- `FilePanel.tsx` — 右侧文件查看器：当前内容/操作 diff、分页与刷新。
- `MemorySettings.tsx` — Memory 管理页：筛选、分页、创建/纠正/遗忘（928 行）。
- `ProcessSessionCard.tsx` — 进程会话卡片：状态、日志、停止。
- `ProfileSettings.tsx` — Profile 页：Token 活动图与统计（440 行）。
- `ProviderModelSettings.tsx` — Provider/Model 设置：配置、发现、启用、容量
  （1,327 行）。
- `RuntimeRunProgress.tsx` — Run 进度：调用次数、耗时、压缩/补充状态。
- `RuntimeStatusBanner.tsx` — 连接/重连/离线状态横幅。
- `RuntimeTurnOutcome.tsx` — 回合结果：失败/取消说明与原因码。
- `SearchDialog.tsx` — 对话搜索对话框（标题过滤、键盘打开）。
- `SettingsPage.tsx` — 设置页容器与导航（Provider/Model/Skills/Memory/Profile/
  外观/通用）。
- `Sidebar.tsx` — 侧边栏：对话列表、Projects、归档入口、搜索按钮（539 行）。
- `SkillsSettings.tsx` — Skills 设置：列表、诊断、启用开关。
- `SlashCommandMenu.tsx` — slash 命令菜单与命令表。
- `ThemePreview.tsx` — 主题预览卡与代码 diff 预览。
- `TitleBar.tsx` — 自定义标题栏与窗口控制（最小化/最大化/关闭）。
- `TurnNavigator.tsx` — 回合导航条：锚点计算与键盘导航。
- `ui.tsx` — 基础 UI 原语：cx、IconButton、Markdown、MenuItem。

`src/renderer/` 与 `src/main/`、`src/preload/`、`src/shared/` 的测试文件：

- `main/ipc.test.ts` / `main/lifecycle.test.ts` / `main/window.test.ts` —
  IPC、生命周期与窗口单元测试。
- `main/linuxPackaging.test.ts` — Linux 打包配置断言。
- `main/preferences.test.ts` — 偏好校验与持久化测试。
- `main/protocolContract.test.ts` — 协议契约与 golden trace 校验（1,331 行）。
- `main/runtime/runOutcome.test.ts` — 回合结果解析测试。
- `main/runtimeHost.test.ts` — Runtime 监督、重连与重启测试（1,304 行）。
- `preload/index.test.ts` — preload API 转发测试。
- `renderer/App.test.tsx` / `renderer/store.test.ts` — 根组件与状态 store 测试。
- `renderer/applyUiPreferences.test.ts` / `platformPreferences.test.tsx` —
  偏好应用与 hook 测试。
- `renderer/i18n.test.tsx` — 双语文案完整性测试。
- `renderer/mockAgentClient.test.ts` — mock 客户端场景测试。
- `renderer/profileUsage.test.ts` — usage 图表数据测试。
- `renderer/runtimeClient.test.ts` / `runtimeHistory.test.ts` /
  `runtimeProjection.test.ts` — RPC 客户端、历史加载与投影测试。
- `renderer/runtimeStore.integration.test.ts` — 端到端集成测试（4,539 行）。
- `renderer/runtimeStore.live.test.ts` / `runtimeStore.liveEvidence.test.ts` —
  真实 DeepSeek live smoke 与证据校验。
- `shared/theme.test.ts` — 颜色工具测试。
- `renderer/components/*.test.tsx` — 各界面组件测试：AppearanceSettings、
  ArchivedThreadsDialog、Composer、ConversationHeader、CreateProjectDialog、
  EventCard、EventFeed、FilePanel、MemorySettings、ProfileSettings、
  RuntimeRunProgress、RuntimeStatusBanner、RuntimeTurnOutcome、SearchDialog、
  SettingsPage、Sidebar、SkillsSettings、TitleBar、TurnNavigator、ui。

## 4. 桌面客户端

### 4.1 对话与回合

- 多 Turn Thread 持久化：对话、工具调用、结果与状态写入 Runtime 的 SQLite，
  应用重启后可恢复。
- 流式渲染：assistant 文本与工具调用增量实时投影；工具卡片在调用开始、结束和
  结果落盘时更新。
- 停止：运行中的 Run 可手动停止（`run.cancel`）；完成、失败、取消各有明确终态。
- 无总上限：Run 没有总模型调用次数或总时长上限；界面显示累计调用次数与已耗时，
  是否停止由用户决定。
- 运行中补充（steering）：Run 执行期间可继续输入（`run.steer`），补充内容进入
  当前 Run 的后续模型调用。
- 失败后继续：失败/取消的 Run 保留已结算的工具结果，并给出本地化说明和有界的
  原因码；新一轮对话不会重放旧命令，也不会回滚可能已生效的文件或命令操作。
- 草稿保护：没有可用 Model 时，Composer 直接打开 Provider/Model 设置，并保留
  草稿、当前 Thread 和已选 Workspace。
- 历史分页：目录与回合历史默认每页 50 条、最大 100 条；过期页与实时事件按 `seq`
  去重合并；已加载 Thread 有详情缓存；后台 Run 活动状态独立于导航。
- Turn Navigator：以当前 Turn/Item 记录做导航，不做状态权威。

实现位置：`ikaros/desktop/src/main/runtimeHost.ts`、`src/main/runtime/wire.ts`、
`runtime/src/ikaros_runtime/agent/loop.py`。

### 4.2 Thread 管理

- 支持新建、重命名、归档、取消归档；生命周期事件带序号并持久化。
- 侧边栏只显示活动 Thread；归档目录在通用设置中按需加载与管理。
- 归档规则：存在 queued/running Run 的 Thread 拒绝归档；已归档 Thread 不能发起
  新 Turn；取消归档后恢复。
- 搜索：先加载活动 Thread 目录，再在渲染端按标题过滤；不搜索消息正文。
- 冷启动：只读活动目录与其事件水位线，然后增量追赶；不重放整个 Journal。

### 4.3 Projects 与 Workspace

- Project 不是 Runtime 资源，而是按 Thread 的可选 workspace 归并出的桌面分组。
- 选择文件夹即生成 workspace 快照；该目录成为托管命令的默认工作目录
  （`process_start` 的默认 `cwd`）和相对文件路径（`read`/`write`/`edit`）的基准。
- 尚未产生消息的已选文件夹只作为临时桌面状态存在。
- 详情读取：`thread.get` + 分页 `turn.list`（默认 50、最大 100），按 Turn 组装
  Run/Item 完整序列；页水位线与实时事件缓冲做去重合并。

### 4.4 模型与 Provider

- 内置 DeepSeek profile：provider ID `deepseek`，默认 Base URL
  `https://api.deepseek.com/v1`。
- Custom Provider：任意 OpenAI Chat Completions 兼容端点（Base URL、可选 API key、
  可选自定义 header、手动录入 model）。
- Model 发现：DeepSeek 支持在线发现（请求上游 `/models`，响应体上限 1 MB，
  最多解析 1,000 个 model）；Custom Provider 手动录入。
- Model 启用/禁用；新对话选择第一个启用 Model；打开旧对话优先恢复其最近使用的
  Model。
- 上下文窗口：可编辑（有效范围 1 到 9,007,199,254,740,991）。默认值：
  - `deepseek-v4-flash`、`deepseek-v4-pro`、`deepseek-v4-flash-vision-exp`：
    1,000,000 tokens
  - 其他未知 model：32,768 tokens
- Ikaros 不发送用户配置的输出 token 上限（不设置 `max_tokens`）。
- 凭据：API key 明文存于 `~/.ikaros/config.yaml`；只在写配置命令中跨桥；界面、
  日志、事件、SQLite 与投影只出现脱敏状态。
- 使用统计只来自 Provider 上报的 usage，不做 token 文本估算。

### 4.5 Skills 界面

- Skill 目录：`~/.ikaros/skills/<name>/SKILL.md`，只扫描一层目录。
- 名称规则：`^[a-z0-9][a-z0-9-]{0,63}$`（小写字母/数字/连字符，1–64 字符）。
- 设置页展示有效 Skill 与有界诊断；支持全局启用/禁用（禁用列表持久化在
  `config.yaml` 的 `skills.disabled`）。
- 每次 Run 冻结已启用描述符快照（名称、描述、位置）；模型需要时用 `read` 工具
  加载 SKILL.md 正文，不做预注入。

### 4.6 Memory 界面

- 独立数据库 `~/.ikaros/memory.db`；记录字段包含 kind（`fact` / `preference` /
  `relationship` / `project`）、state（`active` / `forgotten`）、scope
  （`global` / 精确 workspace）和 revision。
- 能力：创建（可附带一个 Session Item 来源）、纠正、遗忘（内容抹除的墓碑版本）、
  分页列表（默认 50、最大 100）、按 ID 详情。
- 内容限制：单条正文最多 2,048 字符；列表预览最多 160 字符。
- 召回：关键词相关性（`deterministic-keywords-v1`），候选上限 2,000 条，选中
  上限 8 条 / 6,000 字符；每个 Run 冻结精确 revision。
- 语义：Run 执行中的纠正只影响之后的新 Run；遗忘阻止继续发送已不可用版本；
  没有自动写入或自动学习。

### 4.7 文件与命令界面

- 命令卡片：`process_start` / `process_read` / `process_wait` / `process_stop`
  显示有界日志、展开更多与停止操作。
- 文件卡片：`read` / `write` / `edit` 显示路径、行范围、字节数、替换次数等有界
  元数据；不显示完整写入内容。
- 文件预览面板（只读）：按 revision 绑定分页，每页最多 50 KiB / 2,000 行，
  单次扫描上限 8 MiB；二进制、编码不支持或不可访问时明确标注。
- 操作差异：`write`/`edit` 保存实际 before/after 捕获（各上限 256 KiB /
  5,000 行；序列化事件上限 256 KiB），面板中可查看；命令创建的文件可预览，但
  命令产生的变化不做自动 diff。
- 引用文件：在对话中引用文件会把路径加入草稿，便于下一轮编辑。

### 4.8 Profile 与 Usage

数据只来自 Provider 上报结果（`model.response_finished`）并重建到 `model_usages`
投影：

- 生命周期 Token 总量、单日峰值 Token；
- 最长完成任务的时长（秒）——排除 `runtime_interrupted` 的 Run；
- 当前与最长连续使用天数（按本地时区日期计算）；
- 每日活动分桶（本地日期）。

### 4.9 本地偏好（`ui-preferences.json`）

- 主题（含 accent/background/foreground 颜色）、界面语言、字体、减少动态效果、
  侧边栏状态、本地用户名。
- 侧边栏宽度：整数，220–520 px；用户名：去空格后 1–32 字符。
- 文件只支持当前格式；不完整或不支持时重置为默认值，不迁移。

### 4.10 连接、重连与启动超时

- Runtime 启动超时默认 15 秒；停止（shutdown）超时默认 5 秒。
- WebSocket 断线重连退避序列：50 / 100 / 200 / 400 / 800 ms（共 5 次尝试）。
- Runtime 进程异常退出后的重启退避：100 / 200 / 400 ms（共 3 次）。
- 重连成功后按最后确认的 `seq` 做增量追赶；不重放全量事件。

### 4.11 打包与发布

- Linux（x64，未签名）：AppImage、DEB、RPM；安装 `io.github.lingbou.ikaros.desktop`
  启动器、应用图标和 `ikaros` 可执行文件。
- Windows（x64，未签名）：NSIS 开发安装器。
- Fedora 构建需要 `libxcrypt-compat`、`rpm-build`；Debian/Ubuntu 生成 RPM 需要
  `rpm` 工具。
- Python Runtime 打包尚未包含在安装包内；开发与验证使用 checkout 加本地
  Python 3.12/3.13。

### 4.12 界面语言与 i18n

- 界面语言：`en`、`zh-CN`（偏好字段 `language`）。
- 主题：`system` / `light` / `dark` 三种色彩模式，深浅两套主题各含
  accent/background/foreground 颜色、UI 字体（system / inter / sego-ui）、
  代码字体（system-mono / cascadia-code / consolas）、半透明侧边栏与对比度。
- 固定短语（工具状态、Run 状态、文件查看器、错误提示）由渲染端 i18n 翻译；
  用户与模型的消息内容不做翻译。
- 实现位置：`ikaros/desktop/src/renderer/i18n.ts`、`src/shared/platform.ts`。

### 4.13 IPC 与 preload 通道

渲染进程不直接接触 Electron IPC；`preload/index.ts` 通过 contextBridge 暴露
`window.ikaros`（runtime / workspace / preferences / windowControls 四组 API），
Electron main 在 `ipc.ts` 中注册全部通道：

- `runtime.*`（invoke，共 31 条）：thread-create / thread-rename / thread-archive /
  thread-unarchive / thread-get / thread-list / turn-list / turn-start /
  run-cancel / run-steer / event-replay / process-read / process-stop /
  provider-list / provider-configure / provider-discover-models /
  provider-disconnect / provider-remove / model-list / model-set-enabled /
  model-set-limits / skill-list / skill-set-enabled / memory-create /
  memory-correct / memory-forget / memory-list / memory-get / file-preview /
  file-change-get / usage-read。
- `runtime.event` / `runtime.status`（push）：Runtime 事件通知与连接状态。
- `workspace.choose-directory`：系统目录选择器，返回 workspace 快照。
- `preferences.get` / `preferences.update` / `preferences.changed`：UI 偏好
  读写与变更推送。
- `window.close` / `window.minimize` / `window.toggle-maximize`：窗口控制。

渲染进程按不受信处理：所有 IPC 参数在主进程侧校验后才转发到 Runtime；安全策略
集中在 `src/main/security.ts`。

## 5. 界面交互细节

### 5.1 快捷键

| 位置 | 按键 | 行为 |
| --- | --- | --- |
| 全局 | Ctrl/Cmd + K | 打开搜索（设置页关闭时；设置页打开时不触发） |
| 全局 | Ctrl/Cmd + N | 新建对话 |
| Composer | Enter | 发送消息（无 Shift、非 IME 合成中） |
| Composer | Shift + Enter | 换行 |
| Composer | ↑ / ↓ | slash 命令菜单上下选择 |
| Composer | Tab / Enter | 补全当前 slash 命令 |
| Composer | Esc | 关闭 slash 命令菜单 |
| 对话区 | Alt + ↑ / Alt + ↓ | 跳到上一个 / 下一个回合（输入框聚焦、对话框打开或 IME 合成时不触发） |
| 会话标题菜单 | Esc | 关闭菜单 |
| 搜索对话框 | Enter | 打开选中的结果 |
| 侧边栏 | ↑ / ↓ / ← / → / Home / End | 会话列表键盘导航 |
| 回合导航条 | ↑ / ↓ / Home / End | 锚点键盘导航 |
| Profile 图表 | ← / → / ↑ / ↓ / Home / End | 数据点键盘导航 |

### 5.2 窗口与标题栏

- 使用自定义标题栏（`TitleBar` 组件），窗口控制（最小化 / 最大化 / 关闭）通过
  preload 的 `windowControls` API 调用 Electron main。
- `usesCustomTitleBar` 决定渲染层是否绘制窗口按钮；窗口创建时通过
  `updateWindowChrome` 应用标题栏 / 边框选项。
- 开发模式：渲染进程从 `ELECTRON_RENDERER_URL` 加载（electron-vite dev server）；
  生产模式加载打包后的本地资源。

### 5.3 状态反馈

没有独立 toast 系统，反馈由三类界面承担：

1. 连接状态横幅 `RuntimeStatusBanner`：starting / connected / reconnecting /
   offline 四态，带重连 / 重载操作；
2. 对话框与设置页内的 `role="alert"` / `role="status"` 区域：错误、加载中、空态；
3. 卡片级状态：Run 进度（调用次数 / 耗时 / 压缩 / 补充）、回合结果（失败 / 取消
   说明 + reasonCode）、进程卡片（状态 / 退出码 / 停止）。

### 5.4 空状态与占位

- 空对话：标题“我们接下来做什么？”，副标题“开始一段对话，让 Ikaros 帮你规划、
  探索、创作或完成任务。”（i18n 键 `empty.heading` / `empty.description`）。
- 文件为空：“此文件为空。”；二进制、编码不支持、过大、扫描超限、版本变化等
  都有专门的不可用文案。
- 归档对话、搜索结果、Memory 列表、Skills 列表均有各自空态与错误态。
- `/mock-1`、`/mock-2`、`/mock-3` 只在 mock 模式插入占位文本；真实对话路径由
  Runtime 驱动。

### 5.5 输入法与无障碍

- IME 合成期间（`isComposing`）不触发快捷指令（slash 菜单、回合跳转）。
- 主要交互元素带 `aria-label` 与 `role`（dialog / alert / status），菜单与列表
  支持方向键和 Home / End。

## 6. Runtime

### 6.1 进程、启动与单实例

- 每个应用启动一个长驻 Python Runtime；Desktop 主进程通过回环 WebSocket
  JSON-RPC 连接（`Authorization: Bearer <每次启动随机 token>`）。
- 启动参数：`--host 127.0.0.1`、`--port 0`（临时端口）、`--token`、
  `--parent-pid`。标准输出只有一条机器可读就绪记录（含选定端口）；日志走标准错误。
- 单实例：通过 `runtime.lock` 独占 Runtime home；等锁默认超时 10 秒、轮询间隔
  50 ms。崩溃后操作系统句柄释放即恢复。
- 连接服务：每个连接的事件队列上限 1,024 条；单条消息发送超时 5 秒。
- Runtime 生命周期与应用绑定：关闭应用会干净停止 Runtime；切换页面/Thread
  不会重启 Runtime，也不会取消活动 Run。

### 6.2 协议与 RPC

- 协议版本 7，JSON-RPC 2.0，服务名 `ikaros-runtime`；初始化方法 `initialize`，
  事件通知方法 `event`。
- 规模：32 个 post-initialize RPC 方法、15 种持久化事件类型、8 个 provider
  工具 ID；由 `protocol/spec.py` 生成 `runtime-protocol.json` 与 Desktop 字面量
  联合类型；两端共用 `golden-trace.json`（约 139 KB）做协议回归。
- RPC 方法清单：
  `thread.create` / `thread.rename` / `thread.archive` / `thread.unarchive` /
  `thread.get` / `thread.list` / `turn.start` / `turn.list` / `run.cancel` /
  `run.steer` / `event.replay` / `provider.list` / `provider.configure` /
  `provider.discover_models` / `provider.disconnect` / `provider.remove` /
  `model.list` / `model.set_enabled` / `model.set_limits` / `skill.list` /
  `skill.set_enabled` / `memory.create` / `memory.correct` / `memory.forget` /
  `memory.list` / `memory.get` / `file.preview` / `file.change.get` /
  `process.read` / `process.stop` / `usage.read` / `runtime.shutdown`。
- 持久化事件类型（15）：`thread.created`、`thread.renamed`、`thread.archived`、
  `thread.unarchived`、`run.state_changed`、`item.started`、`item.delta`、
  `item.completed`、`file.change_recorded`、`process.recorded`、
  `model.input_prepared`、`context.compacted`、`model.response_finished`、
  `run.settled`、`run.steered`。
- 事件模型：全局单调递增 `seq`；客户端从游标调用 `event.replay` 追赶，重复序列
  忽略、缺口不跨过；目录/历史页返回 `snapshotSeq` 水位线用于对齐。
- 幂等：`thread.create` 与 `turn.start` 使用稳定 `clientRequestId`（唯一索引），
  传输失败后重试不会产生重复 Thread/Turn。
- 文本增量：第一个安全 delta 立即持久化发布；之后合并到 256 字符或 50 ms，并在
  Provider 完成、工具调用边界、取消、失败时同步刷出。
- 数值规则：严格 JSON；拒绝非有限数、超出 ±(2^53−1) 的整数和过深嵌套；
  SQLite 整数上限 2^63−1。

### 6.3 存储与数据

- `state.db`：SQLite，schema 14；Journal Event schema 10。保存 Threads/Branches/
  Turns/Runs/Items、事件 Journal、RunConfig、ContextRevision、StepInput 审计、
  usage、进程与文件操作事实；投影可从 Journal 重建。
- `memory.db`：独立 schema 1；有自己的连接、事务、revision 与操作回执；与
  `state.db` 无跨库事务/外键，只共享 Runtime-home 锁。
- `config.yaml`：version 2，上限 256 KiB（读/写都校验），原子替换写入；保存
  Provider/model 配置和 `skills.disabled`。
- `runtime.lock`：单进程独占；崩溃后可恢复；释放时保留文件（POSIX 不换 inode）。
- 离线维护：
  - `storage check`：校验 schema、Journal 顺序、投影可重建性；
  - `storage backup`：SQLite Backup API（包含已提交 WAL），拒绝覆盖目标；
  - `storage repair-projections`：先产出修复前备份，再在单事务里重建派生表。
- 数据策略：只支持当前格式（config 2 / schema 14 / Journal 10 / protocol 7）；
  不兼容的开发数据以"需要重置"错误失败，需要显式重建；更新代码不会动配置、
  Skills、桌面偏好与 `memory.db`。

### 6.4 崩溃恢复语义

- 干净关闭：Scheduler 停止接收新工作 → 协作取消活动 Run → 排队/预留 Run 标记
  `cancelled` → 等待 worker → 关闭数据库。
- 异常退出后启动：`running` Run → `failed`（原因码 `runtime_interrupted`，保留
  部分 assistant 文本）；`queued` Run 按原 Journal 顺序重新调度；终态 Run 永不
  重跑。
- 进程记录：重启把 active/start-pending 命令标为 `unknown`；不重挂 PID、不重放
  命令；继续工作需要在新的 Turn 里先检查当前状态。

### 6.5 Agent 执行模型

- 层级：Thread → Branch → Turn → Run → Item；Run 内部按 Step 组织（一次模型请求
  + 其工具调用与结果）。
- 提交时冻结 RunConfig：Provider/Model、workspace、工具定义（含 schema 哈希）、
  身份（IKAROS.md）、已启用 Skill 描述符、模型上下文窗口。
- 调度：全局 1 个 scheduler worker（同时只跑一个 Run）；同一 Run 内工具串行
  执行；每个 Branch 有串行队列。
- 无总模型调用次数/时长上限；可手动取消；单次 Provider 请求有超时；资源清理
  有界。
- 失败/取消的 Run 向后续对话提供：已持久化的工具结果 + 独立的 Runtime 状态块
  （解释已完成、未完成、未知影响），历史工具调用永不自动重放。
- 隔离规则：模型不能提供所有权 ID 来启动或接管其他 Run 的命令；工具执行上下文
  （Run/Step/Item/Thread/默认 cwd）由 Runtime 提供。

### 6.6 上下文预算与自动压缩

- 输入预算：模型窗口的 95%（`USABLE_CONTEXT_WINDOW_PERCENT = 95`）。
- 自动压缩触发阈值：模型窗口的 90%（`AUTO_COMPACT_TOKEN_LIMIT_PERCENT = 90`）。
- token 估算方式：保守 UTF-8 上界（`conservative-utf8-upper-bound`），只用于
  输入安全，不用于计费；计费只信 Provider 上报 usage。
- 历史选择（`HistorySelector`）：每页 32 个 Turn，从新到旧取连续后缀；工具调用
  与结果必须成对；第一个放不下的 Turn 成为省略边界；选择结果对 Run 冻结。
- 压缩流程：
  1) 确定性 head/tail 裁剪（`trim_context_records`），保留完整工具单元；
  2) 用一次无工具 Provider 请求生成不可信摘要（前缀
     `Prior context summary (untrusted; verify):`），输出预算 4,096 tokens，
     提示词开销预留 256 tokens；
  3) 摘要以新的 ContextRevision 追加，最多 64,000 字符
     （`COMPACTION_SUMMARY_MAX_CHARACTERS_V1`）。
- 压缩源与保留预算：单个裁剪单元最多 24,000 字节；单条记录/工具参数各截断到
  700 字节；近期用户消息保留预算 20,000 tokens；压缩后上下文尾部保留预算
  64,000 tokens（`_RETAINED_CONTEXT_TOKEN_BUDGET`）。
- Memory 预算：最多使用剩余空间的一半，同时至少保留 4,096 tokens 给历史和失败
  说明；小窗口下可完全省略 Memory。
- 原始 Items 永不改写；被省略内容可通过 `history_read` 找回；同一 Run 可多次
  压缩，每次追加新 revision。
- 计费口径：Token 总量、每日分桶、最长任务、连续天数都来自 Provider 上报。

实现位置：`runtime/src/ikaros_runtime/run_input.py`、
`agent/compaction.py`、`agent/history.py`、`storage/store.py`。

### 6.7 完成检查（bounded completion check）

- 触发条件：候选最终答案存在尚未完成、未验证或未结算的持久化工具工作时。
- 行为：复用当前 Run 的模型与上下文计划，不携带工具，执行一次有界验证调用，
  输出预算 256 tokens（`_COMPLETION_CHECK_OUTPUT_TOKENS`），不循环。

### 6.8 Provider 适配

- 结构：`ProviderRegistry` + 单一 OpenAI 兼容流式适配器；DeepSeek 为内置 profile，
  Custom Provider 复用的同一适配器；测试用 `ScriptedProvider`。
- 流式事件：`response.started`、`text.delta`、`reasoning.delta`、
  `tool_call.arguments_delta`、`tool_call.completed`、`usage.updated`、
  `response.completed`。
- 解析限制：单个 SSE 事件 ≤ 1,000,000 字符；工具参数累计 ≤ 1,000,000 字符；
  reasoning 累计 ≤ 1,000,000 字符。
- 超时（`ProviderTimeouts`）：连接 10 秒、响应头 30 秒、流空闲 300 秒。
- 重试（`ProviderRetryPolicy`）：请求失败最多重试 4 次、流失败最多重试 5 次；
  基础退避 0.2 秒，指数增长 ×2、抖动 0.9–1.1，单次上限 5 秒；尊重
  `retry-after`（封顶 5 秒）；一旦已产生模型内容就停止透明重试。
- 错误分类：authentication、rate limit、context overflow、invalid request、
  timeout、network、server、cancelled、protocol、unknown；附带安全可选的
  status / request ID / retryable / retry-after。
- 凭据保护：任何已配置凭据的精确值（或 ≥8 字符嵌入）会被拦截，不进入 Journal、
  事件、日志、列表响应、异常与 UI 投影；轮换/删除后仍对公开字段做校验。
- Model 发现：`/models` 响应体 ≤ 1 MB、最多解析 1,000 条；发现请求也使用相同的
  超时与错误分类。

### 6.9 内建工具（8 个）

| 工具 | 参数 | 限制与行为 |
| --- | --- | --- |
| `process_start` | `command`（≤32,768 字符）、可选 `cwd`（≤4,096 字符） | 返回稳定 `processId`；同 Item 同参数幂等，改参数报错；每 Run 最多 64 个运行中命令 |
| `process_read` | `processId`、可选 `cursor` | 返回状态 + 最多 16 KiB 输出页；稳定字节游标 |
| `process_wait` | `processId`、可选 `cursor`、`timeoutMs`（1–60,000，默认 1,000） | 等待退出；超时返回 `running` 且不杀进程 |
| `process_stop` | `processId`、可选 `cursor` | 停止进程树；重复调用无害 |
| `history_read` | `itemId`、`before`/`after`（0–41）、`maxChars`（256–24,000） | 读取已结算对话 Item 的有界切片 |
| `read` | `filePath`、`offset`、`limit`（1–2,000，默认 2,000） | UTF-8；每次 ≤50 KiB；单行超 2,000 字符截断；拒绝目录/二进制/非法编码；`nextOffset` 翻页 |
| `write` | `filePath`、`content` | 建父目录；同目录临时文件 + SHA-256 校验 + fsync + 原子替换；保留 BOM/换行/文件模式 |
| `edit` | `filePath`、`oldString`、`newString`、`replaceAll` | 精确匹配；默认要求唯一匹配；CRLF/孤立 CR 规范化为 LF；变更前检测 `stale_content` |

进程与输出细节：

- 每进程保留总量 1 MiB（head/tail 各半）；stdout/stderr 各自最多保留 256 KiB，
  超出持续排空丢弃。
- 输出快照最多每 0.25 秒一条（4 条/秒）；终态事实立即记录；缓冲区不变时不重复
  写入。
- 进程状态：`running` / `exited` / `terminated` / `unknown`；只有观察到成功退出
  才算成功。
- 平台行为：Windows 挂起创建 + kill-on-close Job Object + 恢复；POSIX 新会话/
  进程组；根进程退出后清理残留后代。
- 清理超时：终止进程树 6 秒、排空管道 2 秒、Run 关闭清理等待 10 秒。
- 环境：子进程只继承白名单环境变量，并强制 `PYTHONIOENCODING=utf-8`、
  `PYTHONUTF8=1`。
- 文件路径锁：按解析后的绝对路径加锁；符号链接别名解析到目标后共享同一把锁。
- 文件读取辅助常量：路径上限 4,096 字符；二进制嗅探采样 4,096 字节；常见二进制
  扩展名（.exe/.dll/.zip/.docx/.png 等）直接拒绝文本读取。

### 6.10 文件预览与变更捕获

- `file.preview`：读取当前 UTF-8 文本，绑定 revision 分页，每页 ≤50 KiB / 2,000 行，
  单次扫描上限 8 MiB，带行号；磁盘变化需要刷新；后续页必须使用同一 revision。
- 不可用状态显式返回：二进制、编码不支持、不可访问；工作区外路径需要线程内匹配
  的文件工具记录。
- 预览文本本身不会成为模型输入，除非模型主动调用工具读取。
- `file.change.get`：按 Thread + Tool Call Item 返回不可变的 write/edit 捕获；
  before/after 各 ≤256 KiB / 5,000 行；序列化事件 ≤256 KiB（超大捕获只保留
  元数据）；缺失捕获显式不可用。
- 提交语义：工具结算与 `file.change_recorded` 事件同事务提交；diff 正文与模型
  工具输出分离存储。

### 6.11 Memory 子系统

- 存储：`memory.db`（schema 1）。记录包含 id、kind、state、scope、正文、来源
  引用、revision、内容完整性摘要（摘要私有，Forget 时清除）。
- 操作：create / correct / forget / list / get；创建与纠正幂等；遗忘产生内容抹除
  的墓碑 revision；冲突返回有界 `reasonCode`（协议错误码 `-32020`
  `memory operation failed`）。
- 溯源：可选的来源必须指向本 Session 已存在的 Item；Session 重置只会让软来源
  退化，不删除 Memory。
- 召回（`MemoryRetrieverV1`）：确定性关键词相关性；候选 ≤2,000；选中 ≤8 条且
  ≤6,000 字符；每个 Run 冻结精确 revision；读取与物化在 `state.db` 事务之外。
- 输入包装：Memory 正文以 contextual-data 包装进入模型，不改变工具权限、身份、
  执行策略或用户请求；审计只记录 ID/revision/scope/字符数。
- 边界：没有自动 Memory 写入器、没有学习循环；Memory 不保存运行进度、命令日志
  或文件差异。

### 6.12 Skills 子系统

- 目录：`~/.ikaros/skills/<name>/SKILL.md`，只扫描一层；名字规则
  `^[a-z0-9][a-z0-9-]{0,63}$`；描述 ≤1,024 字符；规范位置 ≤32,767 字符。
- 文件限制：SKILL.md ≤256 KiB；YAML frontmatter ≤16 KiB；UTF-8 校验；拒绝
  符号链接类或越界路径；无效项以诊断形式返回而不中断目录。
- 运行机制：`turn.start` 冻结已启用描述符；每个 Provider Step 收到冻结目录和
  "按需读取 SKILL.md"的指令；正文、引用与脚本不预注入。
- 脚本：通过通用命令工具以子进程运行，复用同一套超时、输出限制、取消与清理。
- 不做：任务相关选择、总目录预算、自动全量加载、专用 Skill Item/统计。

### 6.13 身份（Identity）

- `IKAROS.md` 打包在 Runtime 资源内，作为 Agent 身份提示词（英文原文）。
- 身份记录：id `ikaros-identity`、version 1、source `ikaros-runtime:identity`，
  身份核心预算 ≤2,048 字符。
- 每个 RunConfig 冻结身份内容与哈希；Memory、历史摘要或工具输出不能重定义身份。

### 6.14 安全模型

- 执行策略：固定 `FullAccessPolicy`——工具执行不需要审批、不产生权限请求事件；
  命令行使当前操作系统用户权限，不是沙箱。
- 子进程环境：白名单继承 + 强制 UTF-8 变量（见 6.9）。
- 凭据防护：精确值匹配 + ≥8 字符嵌入检测；覆盖请求参数、Journal、事件、日志、
  Provider 流内容与工具输出。
- 写入安全：`config.yaml` 256 KiB 上限 + 原子替换；`write` 用临时文件 + SHA-256
  + fsync + `os.replace`；`edit` 使用 stale 检测。
- 资源边界：输出/日志/捕获均有字节与行数上限；取消与关闭进行有界清理。

### 6.15 审计与可观测性

- `model.input_prepared`：首次使用时携带完整 ContextRevision；后续以 `null` +
  StepInput 序号引用，避免重复正文。
- `StepInput`：记录该次模型调用使用的 revision、完整 Item 引用范围、Memory 与
  状态元数据、预算。
- `usage`：Token 总量/每日分桶/最长任务/连续天数，全部来自 Provider 上报。
- 协议校验：`protocol.generate --check` 与 `golden-trace` 在 Python 与 Desktop
  测试中共同运行；`storage check` 校验 Journal 顺序与投影可重建性。

### 6.16 数据模型与枚举

- ThreadSummary 字段：`id`、`title`、`defaultBranchId`、`workspace`（可空）、
  `createdAt`、`updatedAt`、`archivedAt`（可空）。
- WorkspaceSummary 字段：`id`、`name`、`rootUri`（可空）。
- JournalEvent 信封字段：`seq`、`schemaVersion`、`type`、`threadId`、`branchId`、
  `turnId`、`runId`、`itemId`、`timestamp`、`payload`（作用域 ID 可为空）。
- RunDescriptor 字段：`id`、`turnId`、`threadId`、`branchId`、`providerId`、
  `modelId`、`executionPolicy`、`workspace`、`skills`。
- 状态枚举：
  - Turn / Run：`queued`、`running`、`completed`、`failed`、`cancelled`；
  - Item：`streaming`、`running`、`completed`、`failed`、`cancelled`；
  - Item 类型：`message`、`tool_call`、`tool_result`。
- 稳定 ID 格式：`thread_` / `turn_` / `item_` / `memory_` + 32 位十六进制；
  Desktop 线上校验的通用标识符上限 200 字符。
- Run 失败原因码（`reasonCode`）：`cancelled`、`context_budget_exceeded`、
  `model_input_unavailable`、`protected_value`、`agent_error`、
  `provider_protocol`、`provider_unavailable`、`runtime_interrupted`（重启恢复）、
  按类别生成的 `provider_<category>`（如 `provider_timeout`、
  `provider_rate_limit`），以及冻结配置漂移类如 `provider_configuration_changed`、
  `execution_policy_changed`。
- Memory 原因码：
  - 操作：`memory_not_found`、`memory_revision_conflict`、`memory_forgotten`、
    `memory_idempotency_conflict`、`memory_source_unavailable`；
  - 召回：`memory_retrieval_overflow`、`memory_snapshot_unavailable`。
- Provider 失败类别：`authentication`、`rate_limit`、`context_overflow`、
  `invalid_request`、`timeout`、`network`、`server`、`cancelled`、`protocol`、
  `unknown`。

### 6.17 数据库实现细节

- SQLite 打开参数：`journal_mode = WAL`、`synchronous = NORMAL`、
  `foreign_keys = ON`；`memory.db` 额外启用 `secure_delete = ON`。
- `state.db` 表：`threads`、`branches`、`turns`、`runs`、`items`、`events`、
  `run_configs`、`context_revisions`、`process_sessions`、`model_calls`、
  `model_usages`、`file_changes`。
- 关键索引/约束：
  - `events_one_settled_per_run`：每个 Run 至多一个 `run.settled`（部分唯一索引）；
  - `threads_client_request_id_idx` / `runs_client_request_id_idx`：提交幂等；
  - `threads_active_catalog_order_idx` / `threads_archived_catalog_order_idx`：
    目录按 `updated_at DESC, id ASC` 排序；
  - `branches_default_thread_idx`：每 Thread 一个默认 Branch；
  - `runs_turn_history_idx`：回合历史子记录装配；
  - `model_calls_execution_step_idx`：模型调用步骤唯一；
  - `model_usages_completed_at_idx` / `model_usages_activity_date_idx`：usage 聚合。
- 事务要点：`turn.start` 原子追加用户 Item + Turn + Run + RunConfig；工具结算与
  `file.change_recorded` 同事务；Run 终结（Item/Turn/Run 更新 + `run.settled`）
  单事务且幂等。
- 数值上限：SQLite 整数 2^63−1；跨协议 JSON 安全整数 ±(2^53−1)。
- 维护语义：备份包含已提交 WAL（Backup API）且拒绝覆盖；修复先 `quick_check`、
  外键检查、`wal_checkpoint(TRUNCATE)`、产物重放校验，再原子替换投影。

### 6.18 配置与环境变量

- `IKAROS_HOME`：Runtime home 覆盖（默认 `~/.ikaros`）；Desktop 启动 Runtime 时
  会显式传递（可选 `runtimeHome`）。
- `IKAROS_RUNTIME_PYTHON`：Desktop 指定 Runtime 的 Python 可执行文件。
- `ELECTRON_RENDERER_URL`：开发模式下 Electron 加载的渲染地址（仅开发）。
- live smoke：`IKAROS_LIVE_DEEPSEEK_SMOKE`、`IKAROS_LIVE_DEEPSEEK_KEY_FILE`。
- `config.yaml`（version 2）结构概要：

```yaml
version: 2
providers:
  <provider-id>:
    type: openai_compatible
    preset: deepseek            # 仅内置 DeepSeek profile
    base_url: <url>
    api_key: <plain-text-key>   # 可省略（本地无鉴权端点）
    headers: {}                 # 可选
    models:
      <model-id>:
        display_name: <label>
        enabled: true
        supports_tools: true
        context_window: 32768
skills:
  disabled:
    - <skill-name>
```

- 没有全局默认 Provider/Model；每次 `turn.start` 显式携带 provider/model。
- 明文存储：API key 与 header 值不做静态加密，靠"不打印、不落库、不投影"的
  运行时约束保护。

配置示例。

DeepSeek：

```yaml
version: 2
providers:
  deepseek:
    type: openai_compatible
    preset: deepseek
    base_url: https://api.deepseek.com/v1
    api_key: sk-...
    models:
      deepseek-v4-flash:
        display_name: DeepSeek V4 Flash
        enabled: true
        supports_tools: true
        context_window: 1000000
      deepseek-v4-pro:
        display_name: DeepSeek V4 Pro
        enabled: false
        supports_tools: true
        context_window: 1000000
```

本地 OpenAI 兼容端点（例如 Ollama / vLLM，无鉴权）：

```yaml
version: 2
providers:
  local:
    type: openai_compatible
    display_name: Local
    base_url: http://127.0.0.1:11434/v1
    models:
      qwen3:32b:
        display_name: Qwen3 32B
        enabled: true
        supports_tools: true
        context_window: 32768
```

### 6.19 日志与诊断

- 日志：`logging.basicConfig(level=INFO, stream=stderr)`；标准输出只用于
  serve 就绪记录和 storage 命令的机器可读 JSON 结果。
- `~/.ikaros/logs/` 仅在启用持久文件日志时创建（当前实现默认只输出到 stderr）。
- storage 命令输出 JSON（如 `{"operation":"storage.check", ...}`），可在脚本中
  直接解析。
- 诊断手段：`storage check`（Journal 顺序 + 投影重建校验）、`golden-trace` 协议
  回放、`protocol.generate --check`（生成物一致性）。

### 6.20 状态机（Turn / Run / Item / Process）

```text
Turn / Run
  queued ──激活──▶ running ──┬──▶ completed
                             ├──▶ failed（含 runtime_interrupted）
                             └──▶ cancelled

Item（message）      streaming ──▶ completed / failed / cancelled
Item（tool_call）    running   ──▶ completed / failed / cancelled（与 tool_result 同事务提交）
Item（tool_result）  写入即终态：completed / failed / cancelled

Process              running ──▶ exited / terminated
                        └─（Runtime 重启）──▶ unknown
```

状态转换规则：

| 对象 | 转换 | 触发 |
| --- | --- | --- |
| Run | `queued` → `running` | Scheduler 激活（发布 `run.state_changed`） |
| Run | `running` → `completed` | Agent 循环正常结束 + 有界完成检查通过 |
| Run | `running` → `failed` | Provider 失败、预算超限、输入漂移、内部错误等（带 reasonCode） |
| Run | `queued/running` → `cancelled` | `run.cancel` 被接受并生效，或干净关闭 |
| Run | `running` → `failed(runtime_interrupted)` | 异常退出后启动恢复 |
| Run | `queued`（重启后） | 保持 queued，按原始 Journal 顺序重新调度 |
| Turn | 与其实时 Run 状态同步 | V1 每个 Turn 只有一个 Run；Run 终结时 Turn 一并终结 |
| Item | message：`streaming` → 终态 | `item.started` / `item.delta` / `item.completed` 序列 |
| Item | tool_call：`running` → 终态 | 终态快照与配对的 `tool_result` 同事务提交 |
| Process | `running` → `exited/terminated` | 观察到退出/停止；只有观察到成功退出才算成功 |
| Process | 任意 → `unknown` | Runtime 重启；不重挂 PID、不重放命令 |

不变量：

- 每个 Run 恰好一个 `run.settled`（部分唯一索引强制）；终结事务幂等，允许完成、
  取消、关闭与恢复竞争。
- 终结时同一事务更新所有活跃 Item、Turn、Run；部分文本不会以完成态存活。
- `run.cancel` 的 `accepted: true` 只表示请求被受理，不承诺结果一定是 `cancelled`
  （Provider 可能先完成）。
- 历史 Tool 调用永不自动重放；`unknown` 进程必须在新 Turn 中重新检查。

## 7. RPC 接口参考（协议 7，共 32 个方法）

通用约定：

- 传输：回环 WebSocket，JSON-RPC 2.0；连接后必须先调用 `initialize`，之后才允许
  其他方法；未初始化调用返回错误。
- 连接鉴权：HTTP `Authorization: Bearer <启动 token>`；stdout 就绪记录提供端口。
- 错误码：`-32000` 未初始化；`-32001` 协议版本不支持；`-32002` 重复初始化；
  `-32601` 方法不存在；`-32602` 参数无效；`-32603` 内部错误；
  `-32010` Provider 失败；`-32020` Memory 操作失败（`data.reasonCode`）。
- 命令类方法（thread.create / turn.start / run.cancel 等）先回 ACK，再发布事件；
  带 `clientRequestId` 的提交在超时重试时保持幂等。
- 事件通过 `event` 通知推送；重连后用 `event.replay` 追赶。

### 7.1 会话与目录

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `thread.create` | `title: string\|null`；`workspace: {id≤200, name≤200, rootUri≤4096}\|null`；`clientRequestId?: string` | `{ thread, event }`（幂等时返回既有 Thread） |
| `thread.rename` | `threadId`；`title: string\|null` | `{ thread, changed, event\|null }` |
| `thread.archive` | `threadId` | `{ thread, changed, event\|null }` |
| `thread.unarchive` | `threadId` | `{ thread, changed, event\|null }` |
| `thread.get` | `threadId` | `{ thread, snapshotSeq }` |
| `thread.list` | `cursor?`；`limit?`（1–100，默认 50）；`archived?`（默认 false） | `{ threads, nextCursor, hasMore, snapshotSeq }` |

### 7.2 回合、Run 与事件

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `turn.start` | `threadId`；`branchId`；`content: string`；`providerId`；`modelId`；`clientRequestId?` | `{ threadId, branchId, turnId, runId }` |
| `turn.list` | `threadId`；`branchId`；`cursor?`；`limit?`（1–100，默认 50） | `{ turns, nextCursor, hasMore, snapshotSeq }`（完整 Item 序列） |
| `run.cancel` | `runId` | `{ accepted, runId, status }`；`accepted: true` 不承诺最终为 cancelled |
| `run.steer` | `runId`；`content`（非空）；`expectedRunId`（必须等于 runId）；`clientRequestId` | `{ accepted, runId, clientRequestId, status: received\|processed\|unprocessed }` |
| `event.replay` | `afterSeq?`（≥0，默认 0）；`limit?`（1–1000，默认 500） | `{ events, latestSeq, nextAfterSeq, hasMore }` |

### 7.3 Provider 与 Model

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `provider.list` | 无 | `{ providers: [{id, displayName, origin, configured, credentialConfigured, health}] }` |
| `provider.configure` | DeepSeek：`{kind:"deepseek", apiKey, models}`；Custom：`{kind:"custom", providerId, displayName, baseUrl, apiKey?, headers?, models}`（model 含 id/displayName/contextWindow） | `{ provider }` |
| `provider.discover_models` | `{kind:"deepseek", apiKey}`（临时使用，不持久化） | `{ models: [{id, displayName, contextWindow}] }`（≤1,000 条） |
| `provider.disconnect` | `{providerId:"deepseek"}` | `{ provider }`（删除已保存配置与 model 记录） |
| `provider.remove` | `{providerId}`（Custom） | `{ removed, providerId }` |
| `model.list` | 无 | `{ models: [{contextWindow, providerId, id, displayName, enabled}] }` |
| `model.set_enabled` | `providerId`；`modelId`；`enabled` | `{ model }` |
| `model.set_limits` | `providerId`；`modelId`；`contextWindow`（1–9,007,199,254,740,991） | `{ model }` |

### 7.4 Skills、Memory、文件与进程

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `skill.list` | 无 | `{ skills: [{name, description, location, enabled}], diagnostics: [{entry, code, message}] }` |
| `skill.set_enabled` | `name`；`enabled` | `{ skill }` |
| `memory.create` | `kind`（fact/preference/relationship/project）；`scope`（global 或 workspace{key≤200}）；`content`（≤2,048 字符）；`clientRequestId`；`source?: {type:"session_item", itemId}` | `{ memoryId, resultingRevision, created }` |
| `memory.correct` | `memoryId`；`expectedRevision`；`content`；`clientRequestId` | `{ memoryId, resultingRevision, created:false }` |
| `memory.forget` | `memoryId`；`expectedRevision`；`clientRequestId` | `{ memoryId, resultingRevision, created:false }`（墓碑 revision） |
| `memory.list` | `cursor?`；`limit?`（默认 50、最大 100）；`scope?`；`kind?`；`state?` | `{ memories, nextCursor, hasMore }` |
| `memory.get` | `memoryId` | `{ memory }`（含正文、来源、revision；forgotten 时正文为 null） |
| `file.preview` | `threadId`；`path`；`sourceToolCallItemId?`；`offset?`（≥1，默认 1）；`expectedRevision?` | 文本页 `{status:"text", revision, encoding:"utf-8", bom, content, lineStart, lineEnd, nextOffset, truncated, truncationReason}` 或 `{status:"unavailable", reason}` |
| `file.change.get` | `threadId`；`toolCallItemId` | `{status:"recorded", path, operation, before, after, diff, additions, deletions}` 或 `{status:"unavailable", reason}` |
| `process.read` | `threadId`；`processId`；`cursor?`（≥0，默认 0） | `{state, exitCode, cwd, pid, startedAt, finishedAt, output(≤16 KiB), truncated, errorCode, cursor, nextCursor, hasMore}` |
| `process.stop` | `threadId`；`processId` | 同 `process.read` 的结果形状 |

### 7.5 系统

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `usage.read` | 无 | `{ summary: {lifetimeTokens, peakDailyTokens, longestRunningTurnSec, currentStreakDays, longestStreakDays}, dailyUsageBuckets: [{startDate, tokens}] }` |
| `runtime.shutdown` | 无 | `{ accepted: true }`（ACK 后干净关闭） |
| `initialize`（连接首帧） | 无参数 | `{ protocolVersion: 7, server: {name:"ikaros-runtime", version:"0.1.0"} }` |

补充说明：

- `turn.list` 每页返回完整物化序列：Turn → 全部 Run → 全部 Item（含 `data`）。
- `file.preview` 的 `revision` 是不透明指纹；后续页必须携带同页首用的
  `expectedRevision`，磁盘变化返回 `revision_changed`。
- `process.read` 在 Runtime 重启后从 SQLite 历史事实回退返回，`state` 变为
  `unknown`，不会重挂 PID。
- Memory 变更冲突以稳定 `reasonCode` 返回（见 6.16），不返回任意 JSON 数据。

### 7.6 事件类型与载荷

事件信封（所有事件）：`seq`（≥1，单调递增）、`schemaVersion`（=10）、`type`、
`threadId` / `branchId` / `turnId` / `runId` / `itemId`（按作用域可空）、
`timestamp`、`payload`。非空的 `turnId` / `runId` / `itemId` 必须在 `payload`
中以同值镜像出现；为空的键不得出现在 `payload` 中。

| 事件 | 作用域 | payload 关键字段 |
| --- | --- | --- |
| `thread.created` | Thread | `thread`（完整摘要）、`branch`（id/threadId/createdAt/isDefault）、`clientRequestId?` |
| `thread.renamed` | Thread | `thread`（updatedAt 与事件时间一致） |
| `thread.archived` | Thread | `thread`（archivedAt 等于事件时间） |
| `thread.unarchived` | Thread | `thread`（archivedAt 为 null） |
| `run.state_changed` | Run | `status`：queued / running |
| `item.started` | Run + Item | `item`（该 Item 快照） |
| `item.delta` | Run + Item | `delta`（非空增量文本） |
| `item.completed` | Run + Item | `item`（终态快照）；首个用户消息的 completed 可附带 `turn` / `run`（提交事件） |
| `model.input_prepared` | Run | execution：`stepOrdinal`、`preparedAt`、`contextRevision`（step 1 为完整 revision，之后为 null）、`stepInput`；compression / completion_check：`purpose`、`callOrdinal`、`preparedAt`、`input` |
| `context.compacted` | Run | `revision`、`droppedTurns`、`contextRevision`、`omittedItemIds`、`trigger`（budget_exceeded / threshold）、`targetTokens` |
| `model.response_finished` | Run | `stepOrdinal`、`providerId`、`modelId`、`outcome`（completed/failed/cancelled）、`reasonCode`、`responseModelId`、`requestId`、`usage`、`activityDate`、`finishedAt` |
| `process.recorded` | Run + Item | `process`：processId、threadId、runId、stepOrdinal、itemId、command、cwd、state、exitCode、pid、startedAt、finishedAt、stdout、stderr、output、truncated、errorCode |
| `file.change_recorded` | Run + Item | 变更记录：path、operation、before/after、diff、additions/deletions、recordedAt（reason 不得为 not_recorded） |
| `run.settled` | Run | `status`（completed/failed/cancelled）、`settledAt`、`reasonCode?`（completed 时省略） |
| `run.steered` | Run + Item | `content`、`clientRequestId`、`status`（received/processed/unprocessed）、`item?` |

### 7.7 请求 / 响应示例

除 `initialize` 外，所有请求都必须先完成握手。示例省略标准信封（`jsonrpc` /
`id`），只展示 `params` 与 `result`；`//` 是说明注释，实际传输不含注释；ID、
时间戳与数值均为示意。

initialize（完整信封）：

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":7}}
```

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":7,"server":{"name":"ikaros-runtime","version":"0.1.0"}}}
```

会话与目录：

```jsonc
// thread.create
{"title":"新对话","workspace":null,"clientRequestId":"req-1"}
→ {"thread":{"id":"thread_0123…cdef","title":"新对话","defaultBranchId":"branch_0123…cdef",
            "workspace":null,"createdAt":"2026-10-01T10:00:00.000Z","updatedAt":"2026-10-01T10:00:00.000Z","archivedAt":null},
   "event":{"seq":1,"schemaVersion":10,"type":"thread.created","threadId":"thread_0123…cdef","branchId":"branch_0123…cdef",
            "turnId":null,"runId":null,"itemId":null,"timestamp":"2026-10-01T10:00:00.000Z",
            "payload":{"thread":"(同上)","branch":{"id":"branch_0123…cdef","threadId":"thread_0123…cdef",
                      "createdAt":"2026-10-01T10:00:00.000Z","isDefault":true},"clientRequestId":"req-1"}}}

// thread.rename（archive / unarchive 仅需 threadId）
{"threadId":"thread_0123…cdef","title":"改个名字"}
→ {"thread":{…},"changed":true,"event":{"seq":2,"schemaVersion":10,"type":"thread.renamed",…}}

// thread.get
{"threadId":"thread_0123…cdef"}
→ {"thread":{…},"snapshotSeq":42}

// thread.list
{"cursor":null,"limit":50,"archived":false}
→ {"threads":[{…}],"nextCursor":null,"hasMore":false,"snapshotSeq":42}
```

回合、Run 与事件：

```jsonc
// turn.start
{"threadId":"thread_0123…cdef","branchId":"branch_0123…cdef","content":"帮我写一篇周报",
 "providerId":"deepseek","modelId":"deepseek-v4-flash","clientRequestId":"req-2"}
→ {"threadId":"thread_0123…cdef","branchId":"branch_0123…cdef","turnId":"turn_0123…cdef","runId":"run_0123…cdef"}

// turn.list（每页返回完整 Turn → Run → Item 序列，此处折叠）
{"threadId":"thread_0123…cdef","branchId":"branch_0123…cdef","cursor":null,"limit":50}
→ {"turns":[{"id":"turn_0123…cdef","threadId":"…","branchId":"…","ordinal":1,"status":"completed",
             "createdAt":"…","updatedAt":"…",
             "runs":[{"id":"run_0123…cdef","providerId":"deepseek","modelId":"deepseek-v4-flash",
                      "executionPolicy":"full_access","status":"completed","reasonCode":null,
                      "modelCalls":3,"compactions":0,
                      "items":[{"id":"item_0123…cdef","kind":"message","role":"user","status":"completed",
                                "content":"帮我写一篇周报","data":{},"createdAt":"…","updatedAt":"…"},…]}]}],
   "nextCursor":null,"hasMore":false,"snapshotSeq":57}

// run.cancel
{"runId":"run_0123…cdef"}
→ {"accepted":true,"runId":"run_0123…cdef","status":"running"}

// run.steer
{"runId":"run_0123…cdef","content":"补充一点数据","expectedRunId":"run_0123…cdef","clientRequestId":"req-3"}
→ {"accepted":true,"runId":"run_0123…cdef","clientRequestId":"req-3","status":"received"}

// event.replay
{"afterSeq":100,"limit":500}
→ {"events":[{"seq":101,"schemaVersion":10,"type":"item.delta","threadId":"…","branchId":"…",
              "turnId":"…","runId":"…","itemId":"item_0123…cdef","timestamp":"…",
              "payload":{"delta":"你"}}],
   "latestSeq":101,"nextAfterSeq":101,"hasMore":false}
```

Provider、Model 与 Skills：

```jsonc
// provider.list
{}
→ {"providers":[{"id":"deepseek","displayName":"DeepSeek","origin":"builtin","configured":true,
                "credentialConfigured":true,"health":"unknown"}]}

// provider.configure（DeepSeek）
{"kind":"deepseek","apiKey":"sk-…",
 "models":[{"id":"deepseek-v4-flash","displayName":"DeepSeek V4 Flash","contextWindow":1000000}]}
→ {"provider":{"id":"deepseek","displayName":"DeepSeek","origin":"builtin","configured":true,
               "credentialConfigured":true,"health":"unknown"}}

// provider.configure（Custom）
{"kind":"custom","providerId":"local","displayName":"Local","baseUrl":"http://127.0.0.1:11434/v1",
 "headers":{},"models":[{"id":"qwen3:32b","displayName":"Qwen3 32B","contextWindow":32768}]}
→ {"provider":{"id":"local","displayName":"Local","origin":"custom","configured":true,
               "credentialConfigured":false,"health":"unknown"}}

// provider.discover_models（临时使用 API key，不持久化）
{"kind":"deepseek","apiKey":"sk-…"}
→ {"models":[{"id":"deepseek-v4-flash","displayName":"DeepSeek V4 Flash","contextWindow":1000000}, …]}

// provider.disconnect（内置 DeepSeek：删除已保存配置）
{"providerId":"deepseek"}
→ {"provider":{"id":"deepseek","origin":"builtin","configured":false,"credentialConfigured":false,…}}

// provider.remove（Custom）
{"providerId":"local"}
→ {"removed":true,"providerId":"local"}

// model.list
{}
→ {"models":[{"providerId":"deepseek","id":"deepseek-v4-flash","displayName":"DeepSeek V4 Flash",
              "enabled":true,"contextWindow":1000000}]}

// model.set_enabled
{"providerId":"deepseek","modelId":"deepseek-v4-pro","enabled":false}
→ {"model":{"providerId":"deepseek","id":"deepseek-v4-pro","displayName":"DeepSeek V4 Pro",
            "enabled":false,"contextWindow":1000000}}

// model.set_limits
{"providerId":"deepseek","modelId":"deepseek-v4-flash","contextWindow":500000}
→ {"model":{"providerId":"deepseek","id":"deepseek-v4-flash","enabled":true,"contextWindow":500000}}

// skill.list
{}
→ {"skills":[{"name":"weekly-report","description":"生成周报",
              "location":"C:\\Users\\me\\.ikaros\\skills\\weekly-report\\SKILL.md","enabled":true}],
   "diagnostics":[{"entry":"broken-skill","code":"invalid_frontmatter","message":"…"}]}

// skill.set_enabled
{"name":"weekly-report","enabled":false}
→ {"skill":{"name":"weekly-report","description":"生成周报","location":"…","enabled":false}}
```

Memory、文件、进程与系统：

```jsonc
// memory.create
{"kind":"preference","scope":{"type":"global","key":null},"content":"回复尽量简洁",
 "clientRequestId":"req-4","source":{"type":"session_item","itemId":"item_0123…cdef"}}
→ {"memoryId":"memory_0123…cdef","resultingRevision":1,"created":true}

// memory.correct
{"memoryId":"memory_0123…cdef","expectedRevision":1,"content":"回复尽量简洁，用中文","clientRequestId":"req-5"}
→ {"memoryId":"memory_0123…cdef","resultingRevision":2,"created":false}

// memory.forget（生成内容抹除的墓碑 revision）
{"memoryId":"memory_0123…cdef","expectedRevision":2,"clientRequestId":"req-6"}
→ {"memoryId":"memory_0123…cdef","resultingRevision":3,"created":false}

// memory.list
{"cursor":null,"limit":50,"state":"active","scope":{"type":"global","key":null}}
→ {"memories":[{"id":"memory_0123…cdef","kind":"preference","scope":{"type":"global","key":null},
                "revision":2,"state":"active","preview":"回复尽量简洁，用中文",
                "createdAt":"…","updatedAt":"…","forgottenAt":null}],
   "nextCursor":null,"hasMore":false}

// memory.get
{"memoryId":"memory_0123…cdef"}
→ {"memory":{"id":"memory_0123…cdef","kind":"preference","scope":{"type":"global","key":null},
             "revision":2,"state":"active","content":"回复尽量简洁，用中文",
             "createdAt":"…","updatedAt":"…","forgottenAt":null,
             "provenance":{"sourceKind":"session_item","threadId":"thread_0123…cdef",
                           "turnId":"turn_0123…cdef","itemId":"item_0123…cdef","status":"available"}}}

// file.preview（文本页）
{"threadId":"thread_0123…cdef","path":"report.md","offset":1}
→ {"threadId":"thread_0123…cdef","path":"report.md","status":"text","revision":"rev-1",
   "encoding":"utf-8","bom":false,"content":"# 周报\n…","lineStart":1,"lineEnd":120,
   "nextOffset":121,"truncated":false,"truncationReason":null}

// file.preview（不可用）
{"threadId":"thread_0123…cdef","path":"image.png"}
→ {"threadId":"thread_0123…cdef","path":"image.png","status":"unavailable","reason":"binary_file"}

// file.change.get（已记录）
{"threadId":"thread_0123…cdef","toolCallItemId":"item_0123…cdef"}
→ {"threadId":"thread_0123…cdef","toolCallItemId":"item_0123…cdef","path":"report.md",
   "operation":"write","recordedAt":"…",
   "before":{"exists":false,"byteCount":null,"revision":null,"encoding":null,"bom":null,"newline":null,"lineCount":null},
   "after":{"exists":true,"byteCount":2048,"revision":"sha256:…","encoding":"utf-8","bom":false,"newline":"lf","lineCount":42},
   "status":"recorded","diff":"@@ -0,0 +1,42 @@\n+# 周报\n…","additions":42,"deletions":0}

// process.read
{"threadId":"thread_0123…cdef","processId":"process_0123…cdef","cursor":0}
→ {"processId":"process_0123…cdef","state":"running","exitCode":null,"cwd":"C:\\work",
   "pid":12345,"startedAt":"…","finishedAt":null,"output":"第一段输出…","truncated":false,
   "errorCode":null,"cursor":0,"nextCursor":4096,"hasMore":true}

// process.stop
{"threadId":"thread_0123…cdef","processId":"process_0123…cdef"}
→ {"processId":"process_0123…cdef","state":"terminated","exitCode":null,"cwd":"C:\\work",
   "pid":12345,"startedAt":"…","finishedAt":"…","output":"","truncated":false,
   "errorCode":"terminated","cursor":0,"nextCursor":0,"hasMore":false}

// usage.read
{}
→ {"summary":{"lifetimeTokens":123456,"peakDailyTokens":20000,"longestRunningTurnSec":754,
              "currentStreakDays":3,"longestStreakDays":9},
   "dailyUsageBuckets":[{"startDate":"2026-09-30","tokens":8000},
                        {"startDate":"2026-10-01","tokens":4500}]}

// runtime.shutdown
{}
→ {"accepted":true}
```

## 8. 数据库表结构（列级）

### 8.1 state.db（schema 14）

`events` — 追加型事件 Journal（唯一事实来源）：

| 列 | 类型/约束 |
| --- | --- |
| `seq` | INTEGER PRIMARY KEY AUTOINCREMENT（全局单调递增） |
| `schema_version` | INTEGER NOT NULL CHECK(≥1) |
| `event_type` | TEXT NOT NULL |
| `thread_id` / `branch_id` / `turn_id` / `run_id` / `item_id` | TEXT（可空） |
| `created_at` | TEXT NOT NULL |
| `payload_json` | TEXT NOT NULL |

索引：`events_thread_seq_idx(thread_id, seq)`、`events_run_seq_idx(run_id, seq)`、
`events_one_settled_per_run`（唯一部分索引：每 Run 至多一个 `run.settled`）。

`threads`：

| 列 | 类型/约束 |
| --- | --- |
| `id` | TEXT PRIMARY KEY |
| `title` | TEXT（可空） |
| `default_branch_id` | TEXT NOT NULL UNIQUE |
| `created_at` / `updated_at` | TEXT NOT NULL |
| `archived_at` | TEXT（可空） |
| `client_request_id` | TEXT（可空，部分唯一索引） |
| `workspace_json` | TEXT（可空） |

索引：`threads_client_request_id_idx`（唯一部分）、`threads_active_catalog_order_idx`、
`threads_archived_catalog_order_idx`（按 `updated_at DESC, id ASC`）。

`branches`：`id` PK；`thread_id` → threads（CASCADE）；`created_at`；
`is_default` 0/1。唯一部分索引 `branches_default_thread_idx`（每 Thread 一个默认）。

`turns`：`id` PK；`thread_id` → threads（CASCADE）；`branch_id` → branches
（CASCADE）；`ordinal`；`status`；`created_at`；`updated_at`；
`UNIQUE(branch_id, ordinal)`。

`runs`：`id` PK；`turn_id` → turns（CASCADE）；`provider_id`；`model_id`；
`status`；`created_at`；`started_at?`；`settled_at?`；`reason_code?`；
`client_request_id?`（唯一部分索引）；`execution_policy`（默认 `full_access`）。
索引 `runs_turn_history_idx(turn_id, created_at, id)`。

`run_configs`：`run_id` PK → runs（CASCADE）；`config_json`（冻结配置全文）。

`context_revisions`：`run_id` → runs（CASCADE）；`revision`（≥1）；`record_json`；
`PRIMARY KEY(run_id, revision)`。

`process_sessions`：`process_id` PK；`run_id` → runs（CASCADE）；`item_id` → items
（CASCADE）；`record_json`；`UNIQUE(run_id, item_id)`。

`model_calls`：`run_id` → runs（CASCADE）；`call_ordinal`（≥1）；`step_ordinal?`；
`input_json`；`prepared_at`；`purpose`（execution / compression / completion_check）；
`outcome?`（completed / failed / cancelled）；`reason_code?`；`response_model_id?`；
`request_id?`；`usage_json?`；`activity_date?`；`finished_at?`；
`PRIMARY KEY(run_id, call_ordinal)`。约束：execution 必须带 step_ordinal，其余
purpose 必须为 NULL；usage 只在 completed 出现；唯一索引
`model_calls_execution_step_idx(run_id, step_ordinal) WHERE purpose='execution'`。

`model_usages`：`thread_id` / `turn_id` / `run_id` 外键；`call_ordinal`；
`step_ordinal?`；`provider_id`；`model_id`；`input_tokens`（≥0）；
`cached_input_tokens?`（≥0）；`output_tokens`（≥0）；`reasoning_output_tokens?`
（≥0）；`total_tokens`（≥0）；`activity_date`；`completed_at`；
`PRIMARY KEY(run_id, call_ordinal)` 且外键指向 `model_calls`（CASCADE）。
索引：`model_usages_completed_at_idx`、`model_usages_activity_date_idx`。

`items`：`id` PK；`turn_id` → turns（CASCADE）；`run_id` → runs（CASCADE）；
`ordinal`；`kind`（message / tool_call / tool_result）；`role?`（user / assistant /
tool）；`status`；`content`；`created_at`；`updated_at`；`data_json`（默认 `{}`）；
`UNIQUE(run_id, ordinal)`。索引 `items_turn_context_idx(turn_id, ordinal)`。

`file_changes`：`tool_call_item_id` PK → items（CASCADE）；`record_json`
（不可变 before/after 捕获与 diff）。

### 8.2 memory.db（schema 1）

`memory_records`：

| 列 | 类型/约束 |
| --- | --- |
| `id` | TEXT PRIMARY KEY |
| `kind` | fact / preference / relationship / project |
| `scope_type` | global / workspace |
| `scope_key` | workspace 时必填且长度 1–200；global 时必须为空 |
| `current_revision` | ≥1，外键指向 `memory_revisions`（延迟约束） |
| `state` | active / forgotten（与 `forgotten_at` 组合校验） |
| `created_at` / `updated_at` | TEXT NOT NULL |
| `forgotten_at` | TEXT（可空） |

索引：state+updated_at、state+scope、state+scope+kind 三个排序索引。

`memory_revisions`：`memory_id` → memory_records（RESTRICT）；`revision`（≥1）；
`operation`（create / correct / forget）；`content?`；`content_digest?`；
`content_redacted_at?`；`source_kind`（user_explicit / session_item）；
`source_thread_id?` / `source_turn_id?` / `source_item_id?` / `source_item_digest?`；
`created_at`；`PRIMARY KEY(memory_id, revision)`。约束保证：create/correct 要么
有正文+摘要、要么是已抹除；forget 无正文；来源字段要么全空（用户显式），要么
全齐（Session Item）。

`memory_operations`（幂等回执）：`client_request_id` PK；
`method`（memory.create / correct / forget）；`non_content_fingerprint`；
`memory_id`；`resulting_revision`（≥1）；`created_at`；
`UNIQUE(memory_id, resulting_revision)`；外键指向对应 revision（RESTRICT）。

### 8.3 打开参数与数值上限

- `state.db` 与 `memory.db` 均使用：`journal_mode=WAL`、`synchronous=NORMAL`、
  `foreign_keys=ON`；`memory.db` 额外 `secure_delete=ON`。
- SQLite 整数上限 2^63−1；跨协议 JSON 安全整数 ±(2^53−1)。
- 未知表/索引或结构漂移会被拒绝：state.db 报“需要重置”，memory.db 报
  “schema 不兼容，请恢复兼容备份”。

### 8.4 完整 DDL

`state.db`（schema 14）的完整建表语句（含 `file_changes`）：

```sql
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    event_type TEXT NOT NULL,
    thread_id TEXT,
    branch_id TEXT,
    turn_id TEXT,
    run_id TEXT,
    item_id TEXT,
    created_at TEXT NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX events_thread_seq_idx ON events(thread_id, seq);
CREATE INDEX events_run_seq_idx ON events(run_id, seq);
CREATE UNIQUE INDEX events_one_settled_per_run
ON events(run_id) WHERE event_type = 'run.settled';

CREATE TABLE threads (
    id TEXT PRIMARY KEY,
    title TEXT,
    default_branch_id TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    archived_at TEXT,
    client_request_id TEXT,
    workspace_json TEXT
);

CREATE UNIQUE INDEX threads_client_request_id_idx
ON threads(client_request_id) WHERE client_request_id IS NOT NULL;
CREATE INDEX threads_active_catalog_order_idx
ON threads(updated_at DESC, id ASC) WHERE archived_at IS NULL;
CREATE INDEX threads_archived_catalog_order_idx
ON threads(updated_at DESC, id ASC) WHERE archived_at IS NOT NULL;

CREATE TABLE branches (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    is_default INTEGER NOT NULL CHECK (is_default IN (0, 1))
);

CREATE UNIQUE INDEX branches_default_thread_idx
ON branches(thread_id) WHERE is_default = 1;

CREATE TABLE turns (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    branch_id TEXT NOT NULL REFERENCES branches(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(branch_id, ordinal)
);

CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    started_at TEXT,
    settled_at TEXT,
    reason_code TEXT,
    client_request_id TEXT,
    execution_policy TEXT NOT NULL DEFAULT 'full_access'
);

CREATE UNIQUE INDEX runs_client_request_id_idx
ON runs(client_request_id) WHERE client_request_id IS NOT NULL;
CREATE INDEX runs_turn_history_idx
ON runs(turn_id, created_at ASC, id ASC);

CREATE TABLE run_configs (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    config_json TEXT NOT NULL
);

CREATE TABLE context_revisions (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    record_json TEXT NOT NULL,
    PRIMARY KEY(run_id, revision)
);

CREATE TABLE process_sessions (
    process_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    record_json TEXT NOT NULL,
    UNIQUE(run_id, item_id)
);

CREATE TABLE model_calls (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    call_ordinal INTEGER NOT NULL CHECK (call_ordinal >= 1),
    step_ordinal INTEGER CHECK (step_ordinal IS NULL OR step_ordinal >= 1),
    input_json TEXT NOT NULL,
    prepared_at TEXT NOT NULL,
    purpose TEXT NOT NULL DEFAULT 'execution'
        CHECK (purpose IN ('execution', 'compression', 'completion_check')),
    outcome TEXT CHECK (outcome IN ('completed', 'failed', 'cancelled')),
    reason_code TEXT,
    response_model_id TEXT,
    request_id TEXT,
    usage_json TEXT,
    activity_date TEXT,
    finished_at TEXT,
    PRIMARY KEY(run_id, call_ordinal),
    CHECK ((purpose = 'execution' AND step_ordinal IS NOT NULL)
           OR (purpose IN ('compression', 'completion_check') AND step_ordinal IS NULL)),
    CHECK (
        (outcome IS NULL AND reason_code IS NULL AND response_model_id IS NULL
         AND request_id IS NULL AND usage_json IS NULL AND activity_date IS NULL
         AND finished_at IS NULL)
        OR
        (outcome = 'completed' AND reason_code IS NULL AND finished_at IS NOT NULL)
        OR
        (outcome IN ('failed', 'cancelled') AND reason_code IS NOT NULL
         AND finished_at IS NOT NULL)
    ),
    CHECK (
        (usage_json IS NULL AND activity_date IS NULL)
        OR
        (outcome = 'completed' AND usage_json IS NOT NULL AND activity_date IS NOT NULL)
    )
);

CREATE TABLE model_usages (
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    call_ordinal INTEGER NOT NULL CHECK (call_ordinal >= 1),
    step_ordinal INTEGER CHECK (step_ordinal IS NULL OR step_ordinal >= 1),
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    input_tokens INTEGER NOT NULL CHECK (input_tokens >= 0),
    cached_input_tokens INTEGER CHECK (cached_input_tokens >= 0),
    output_tokens INTEGER NOT NULL CHECK (output_tokens >= 0),
    reasoning_output_tokens INTEGER CHECK (reasoning_output_tokens >= 0),
    total_tokens INTEGER NOT NULL CHECK (total_tokens >= 0),
    activity_date TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    PRIMARY KEY(run_id, call_ordinal),
    FOREIGN KEY(run_id, call_ordinal)
        REFERENCES model_calls(run_id, call_ordinal) ON DELETE CASCADE
);

CREATE INDEX model_usages_completed_at_idx
ON model_usages(completed_at ASC, run_id ASC, call_ordinal ASC);
CREATE INDEX model_usages_activity_date_idx
ON model_usages(activity_date ASC, run_id ASC, call_ordinal ASC);

CREATE UNIQUE INDEX model_calls_execution_step_idx
ON model_calls(run_id, step_ordinal) WHERE purpose = 'execution';

CREATE TABLE items (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    kind TEXT NOT NULL,
    role TEXT,
    status TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    data_json TEXT NOT NULL DEFAULT '{}',
    UNIQUE(run_id, ordinal)
);

CREATE INDEX items_turn_context_idx
ON items(turn_id, ordinal ASC);

CREATE TABLE file_changes (
    tool_call_item_id TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
    record_json TEXT NOT NULL
);
```

`memory.db`（schema 1）的完整建表语句：

```sql
CREATE TABLE memory_records (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('fact', 'preference', 'relationship', 'project')),
    scope_type TEXT NOT NULL CHECK (scope_type IN ('global', 'workspace')),
    scope_key TEXT,
    current_revision INTEGER NOT NULL CHECK (current_revision >= 1),
    state TEXT NOT NULL CHECK (state IN ('active', 'forgotten')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    forgotten_at TEXT,
    CHECK (
        (scope_type = 'global' AND scope_key IS NULL)
        OR
        (scope_type = 'workspace' AND scope_key IS NOT NULL AND length(scope_key) BETWEEN 1 AND 200)
    ),
    CHECK (
        (state = 'active' AND forgotten_at IS NULL)
        OR
        (state = 'forgotten' AND forgotten_at IS NOT NULL)
    ),
    FOREIGN KEY(id, current_revision)
        REFERENCES memory_revisions(memory_id, revision)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE memory_revisions (
    memory_id TEXT NOT NULL REFERENCES memory_records(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    operation TEXT NOT NULL CHECK (operation IN ('create', 'correct', 'forget')),
    content TEXT,
    content_digest TEXT,
    content_redacted_at TEXT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('user_explicit', 'session_item')),
    source_thread_id TEXT,
    source_turn_id TEXT,
    source_item_id TEXT,
    source_item_digest TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY(memory_id, revision),
    CHECK (
        (operation IN ('create', 'correct') AND (
            (content IS NOT NULL AND content_digest IS NOT NULL AND content_redacted_at IS NULL)
            OR
            (content IS NULL AND content_digest IS NULL AND content_redacted_at IS NOT NULL)
        ))
        OR
        (operation = 'forget'
         AND content IS NULL
         AND content_digest IS NULL
         AND content_redacted_at IS NULL)
    ),
    CHECK (
        (source_kind = 'user_explicit'
         AND source_thread_id IS NULL
         AND source_turn_id IS NULL
         AND source_item_id IS NULL
         AND source_item_digest IS NULL)
        OR
        (source_kind = 'session_item'
         AND source_thread_id IS NOT NULL
         AND source_turn_id IS NOT NULL
         AND source_item_id IS NOT NULL
         AND (
             (operation IN ('create', 'correct')
              AND content_redacted_at IS NULL
              AND source_item_digest IS NOT NULL)
             OR
             ((operation = 'forget' OR content_redacted_at IS NOT NULL)
              AND source_item_digest IS NULL)
         ))
    ),
    CHECK (operation != 'forget' OR source_item_digest IS NULL)
);

CREATE TABLE memory_operations (
    client_request_id TEXT PRIMARY KEY,
    method TEXT NOT NULL CHECK (method IN ('memory.create', 'memory.correct', 'memory.forget')),
    non_content_fingerprint TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    resulting_revision INTEGER NOT NULL CHECK (resulting_revision >= 1),
    created_at TEXT NOT NULL,
    UNIQUE(memory_id, resulting_revision),
    FOREIGN KEY(memory_id, resulting_revision)
        REFERENCES memory_revisions(memory_id, revision)
        ON DELETE RESTRICT
);

CREATE INDEX memory_records_state_order_idx
ON memory_records(state, updated_at DESC, id ASC);

CREATE INDEX memory_records_scope_order_idx
ON memory_records(state, scope_type, scope_key, updated_at DESC, id ASC);

CREATE INDEX memory_records_scope_kind_order_idx
ON memory_records(state, scope_type, scope_key, kind, updated_at DESC, id ASC);
```

## 9. 数据目录

```text
~/.ikaros/
  config.yaml          provider/model 配置、Skill 禁用列表（version 2，≤256 KiB）
  state.db             权威会话与执行记录（SQLite，schema 14）
  memory.db            独立长期 Memory（schema 1）
  runtime.lock         单进程独占锁
  backups/             离线备份（按需创建，永不覆盖）
  skills/              用户 Skills：<name>/SKILL.md
  logs/                持久文件日志（启用时创建）
```

## 10. 验证与测试现状

| 项目 | 状态 |
| --- | --- |
| Runtime 离线门禁 | pytest 781 passed / 2 skipped |
| Desktop 离线门禁 | Vitest 528 passed / 1 skipped |
| 静态检查 | Ruff、strict mypy（src+tests）、协议生成检查、Golden Trace、Desktop typecheck、生产构建 全部通过 |
| 真实 DeepSeek | 3 个简单 Run 跑通（含工具调用与状态落盘，0 次压缩） |
| 长任务验收 | 30+ 步、2+ 次压缩、1 MiB 输出、64 并发、断线重连、Windows/Linux 实机：待完成 |

离线基线对应 2026-09-18 代码状态（此后仅文档变化）。

### 10.1 测试布局

- Runtime：`runtime/tests/` 共 37 个测试文件，覆盖 agent 循环、上下文与压缩、
  存储与 schema、协议契约与路由、Provider 适配、进程管理、工具、Memory 召回、
  Skills、服务器与运行时锁；测试使用隔离的临时 Runtime home。
- Desktop：Vitest（`ikaros/desktop/src` 内 `*.test.ts(x)`），包含协议解析、
  IPC、偏好、渲染 store 与 live smoke（`runtimeStore.live.test.ts`）。

### 10.2 live smoke 运行方式

Desktop 端（真实 DeepSeek，脚本会扫描输出中的凭据并打印
`LIVE_SMOKE_OUTPUT_SECRET_SCAN=PASS/FAIL`）：

```powershell
$env:IKAROS_LIVE_DEEPSEEK_KEY_FILE = "<path-to-key-file>"
pnpm run test:live:deepseek
```

Runtime Identity A/B：

```powershell
$env:IKAROS_LIVE_DEEPSEEK_SMOKE = "1"
$env:IKAROS_LIVE_DEEPSEEK_KEY_FILE = "<path-to-key-file>"
uv run --frozen pytest tests/test_live_identity.py -q -s
```

普通 `pnpm test` 与 `pytest` 默认跳过这些用例；凭据只从显式指定的 key file
读取，且不应出现在命令行、事件、SQLite、日志或诊断输出中。

### 10.3 CI 门禁

`.github/workflows/check.yml`（push / PR / 手动触发）：


- Runtime 任务：ubuntu + windows × Python 3.12/3.13；`uv sync --locked` → Ruff →
  strict mypy → 协议生成检查 → pytest；单任务超时 20 分钟。
- Desktop 任务：ubuntu + windows；Node 22 + pnpm 11.9（frozen lockfile）→
  typecheck → Vitest（`--maxWorkers=2`）→ 生产构建；单任务超时 20 分钟。

### 10.4 开发工作流（改协议 / 改 schema / 加能力）

1. **改协议**：编辑 `protocol/spec.py`（方法 / 事件 / 工具 ID）→ 运行
   `uv run --project runtime python -m ikaros_runtime.protocol.generate` 更新
   `runtime-protocol.json` 与 Desktop 生成的 TS 常量 → 更新 `golden-trace.json`
   用例 → 跑两侧测试。
2. **一致性检查**：`protocol.generate --check`（生成物必须与 `spec.py` 一致）
   与 `pytest runtime/tests/test_protocol_contract.py`。
3. **改数据库**：修改 `storage/schema.py` 或 `memory/schema.py` 的 DDL → 提升
   对应 schema 常量 → 更新测试；旧开发数据通过显式重置处理（无迁移路径）。
4. **加/改工具**：修改 `tools/` 实现与注册装配；若是新的 provider 工具 ID，
   同步 `spec.py`、golden trace 与 Desktop 的工具卡片解析。
5. **改界面文案**：在 `renderer/i18n.ts` 同时添加 en 与 zh-CN；`i18n.test.tsx`
   校验两套键集合一致。
6. **改 Provider 行为**：修改 `providers/openai_compatible/`（adapter /
   streaming / discovery）并更新 `test_openai_compatible.py`。
7. **提交前全量门禁**（与 CI 相同）：Runtime 四连（ruff / mypy / pytest /
   protocol check）+ Desktop 三连（typecheck / vitest / build）。

## 11. 当前边界

尚未实现或未接通：

- PTY 交互、跨 Run/重启的进程存活；
- 多个 Run 并发、Multi-Agent 与子 Agent；
- cron 与主动唤醒任务；
- 附件、Web/浏览器/桌面操作工具；
- MCP、插件与第三方工具生态；
- 自动 Memory 提取与 Skill 学习；
- Python Runtime 打包与正式安装包、自动更新、代码签名；
- 交互式权限模式（当前只有 full_access）。

## 12. 常用命令速查

```powershell
# 开发运行
uv sync --project runtime --locked
cd ikaros/desktop; pnpm install; pnpm dev

# 检查与打包（ikaros/desktop）
pnpm check; pnpm package:dir; pnpm package:linux; pnpm package:win

# Runtime 校验（仓库根目录）
uv run --project runtime ruff check runtime
uv run --project runtime mypy runtime/src runtime/tests
uv run --project runtime pytest runtime/tests
uv run --project runtime python -m ikaros_runtime.protocol.generate --check

# 离线维护
uv run --project runtime python -m ikaros_runtime storage check
uv run --project runtime python -m ikaros_runtime storage backup --output C:\Backups\ikaros-state.db
uv run --project runtime python -m ikaros_runtime storage repair-projections
```

## 13. 关键常量速查

| 类别 | 参数 | 值 |
| --- | --- | --- |
| 上下文 | 输入预算 | 模型窗口 95% |
| 上下文 | 自动压缩阈值 | 模型窗口 90% |
| 上下文 | 历史选择分页 | 32 Turns/页 |
| 上下文 | 压缩摘要上限 | 64,000 字符 |
| 上下文 | 摘要输出预算 | 4,096 tokens（提示开销预留 256） |
| 上下文 | 近期用户消息保留 | 20,000 tokens |
| 上下文 | 压缩后尾部保留 | 64,000 tokens |
| 上下文 | 裁剪单元/记录/参数 | 24,000 字节 / 700 字节 / 700 字节 |
| 上下文 | Memory 预留 | 至少 4,096 tokens；Memory 最多占剩余一半 |
| Memory | 单条正文 / 预览 | 2,048 字符 / 160 字符 |
| Memory | 召回候选 / 选中 | 2,000 条 / 8 条、6,000 字符 |
| Memory | 列表分页 | 默认 50、最大 100 |
| 文件 | read | ≤2,000 行、≤50 KiB/次、行截断 2,000 字符、分块 64 KiB |
| 文件 | preview | ≤50 KiB、2,000 行/页；扫描 ≤8 MiB |
| 文件 | write/edit 捕获 | 各 ≤256 KiB / 5,000 行；事件 ≤256 KiB |
| 文件 | history_read | before/after ≤41；maxChars 256–24,000 |
| 文件 | 路径长度 | ≤4,096 字符 |
| 进程 | 并发上限 | 64/ Run |
| 进程 | 输出保留 | 总量 1 MiB（stdout/stderr 各 256 KiB）；分页 16 KiB |
| 进程 | 快照频率 | ≤4 条/秒（0.25 秒间隔） |
| 进程 | wait | 默认 1,000 ms、上限 60,000 ms |
| 进程 | 清理超时 | 终止 6 秒、排空 2 秒、Run 关闭 10 秒 |
| 进程 | 命令/cwd 长度 | ≤32,768 / ≤4,096 字符 |
| Provider | 超时 | 连接 10 秒、响应头 30 秒、流空闲 300 秒 |
| Provider | 重试 | 请求 ≤4 次、流 ≤5 次；基础退避 0.2 秒、×2、抖动 0.9–1.1、上限 5 秒 |
| Provider | 解析上限 | SSE 事件/工具参数/reasoning 各 ≤1,000,000 字符 |
| Provider | 模型发现 | 响应 ≤1 MB、≤1,000 个 model |
| 协议 | 版本/规模 | protocol 7；32 方法；15 事件；8 工具 |
| 存储 | 版本 | state schema 14；Journal 10；memory 1；config 2 |
| 存储 | config 上限 | 256 KiB |
| 服务 | 事件队列 / 发送超时 | 1,024 条 / 5 秒 |
| 服务 | 锁超时 / 轮询 | 10 秒 / 50 ms |
| 桌面 | 启动/停止超时 | 15 秒 / 5 秒 |
| 桌面 | 重连退避 | 50/100/200/400/800 ms；进程重启 100/200/400 ms |
| 桌面 | 分页 | 目录与回合 ≤100/页 |
| 桌面 | 偏好 | 侧边栏 220–520 px；用户名 ≤32 字符 |
| Skills | 名称/描述/位置 | `^[a-z0-9][a-z0-9-]{0,63}$` / ≤1,024 / ≤32,767 字符 |
| Skills | 文件 | SKILL.md ≤256 KiB；frontmatter ≤16 KiB |
| 身份 | 核心预算 | ≤2,048 字符；id `ikaros-identity` v1 |
| 界面 | 语言 | en / zh-CN |
| 数据库 | SQLite 模式 | WAL；synchronous=NORMAL；foreign_keys=ON（memory.db 另有 secure_delete=ON） |
| 日志 | 级别 / 输出 | INFO → stderr |

## 14. 故障排查

### 14.1 Runtime 起不来

- Runtime home 被占用：`runtime.lock` 等锁最多 10 秒；关闭其它 Ikaros / Runtime
  进程，或（仅开发）改用其它 `IKAROS_HOME`。
- Python 版本或路径不对：需要 3.12/3.13；`IKAROS_RUNTIME_PYTHON` 必须指向有效
  解释器。
- 启动即退出：查看 stderr（INFO 日志）；stdout 只会输出就绪记录或 storage 命令
  的 JSON 结果。

### 14.2 数据库被拒绝

- `state database schema is incompatible; reset required`：开发数据版本不符。
  停止 Runtime → 备份或移走 `state.db`（连同 `-wal` / `-shm`）→ 重启自动重建；
  `memory.db`、`config.yaml`、`skills/` 不受影响。
- `memory database schema is incompatible; restore a compatible backup`：恢复
  `memory.db` 备份，或移走后重建（会丢失记忆）。
- 同版本但结构漂移（表 / 索引变化）同样会被拒绝，不会被静默接受。

### 14.3 连接与运行状态

- 横幅显示 reconnecting：WS 按 50/100/200/400/800 ms 退避重连；事件不丢，
  重连后按 `seq` 追赶。
- Runtime 进程意外退出：按 100/200/400 ms 尝试重启；仍失败则显示 offline，
  需要手动重载。
- 长时间无响应：检查流空闲超时（300 秒）与 Provider 状态。

### 14.4 Provider 错误

- authentication：检查 API key、Base URL 与代理设置。
- rate limit：Ikaros 尊重 `retry-after`（封顶 5 秒）并做有界重试。
- context overflow：检查 `model.set_limits` 配置的上下文窗口与压缩阈值。
- timeout / network：连接 10 秒、响应头 30 秒、流空闲 300 秒。
- `model_not_configured` / `model_selection_required`：到设置页配置并启用模型。

### 14.5 文件与命令

- 文件预览不可用原因：`file_not_found`、`not_a_file`、`binary_file`、
  `unsupported_encoding`、`too_large`、`scan_limit`、`revision_changed`、
  `protected_content`；按原因处理（刷新、换文件、检查编码）。
- 命令输出不全：每进程只保留 1 MiB head/tail；用 `process.read` 游标翻页
  （每页 16 KiB），被省略的中间部分不会恢复。
- 命令状态为 `unknown`：Runtime 重启过；不会重挂 PID，先检查实际状态再决定
  新的操作。

### 14.6 live smoke

- 需要 `IKAROS_LIVE_DEEPSEEK_SMOKE=1` 且 `IKAROS_LIVE_DEEPSEEK_KEY_FILE` 指向
  存在的 key 文件；密钥只从文件读取，不要放进命令行参数。
- Desktop 运行器会扫描输出并打印 `LIVE_SMOKE_OUTPUT_SECRET_SCAN=PASS/FAIL`。
- 普通 `pnpm test` / `pytest` 默认跳过这些用例。

## 15. 许可证

GPL-3.0-only，见 `LICENSE`。
