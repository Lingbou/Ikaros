# 日常可用 Alpha 验收记录

日期：2026-09-05。分支：`codex/daily-alpha`。本轮按四阶段实施；未打包 Python Runtime。

## 已交付

| 阶段 | 结果 | 提交 |
| --- | --- | --- |
| 工程基线 | 修复路径/JSON/子进程 fixture、严格 Windows 类型边界；Windows/Linux × Python 3.12/3.13 CI，Node 22 与锁定 pnpm/依赖 | `3c95a989` |
| 配置与失败信息 | Composer 直接打开 Provider/Model 设置并保留草稿、会话与项目；区分加载/未连接/无配置/失败；历史与实时 reasonCode 一致 | `a53978b2` |
| 普通对话继续 | 默认 bounded-history-v2；持久工具配对、失败/取消终结说明、预算降级；v1 queued 与混合历史兼容；无旧调用重放 | `63b613d2` |
| 文件查看与持久差异 | 只读右面板、当前文本分页刷新/行号/引用、独立操作差异、原子结算记录、备份后 schema 8→9 迁移、Journal 5/6 共存 | 本记录所在提交 |

## 自动化验证

- 最终 Runtime 全量：**739 passed，3 skipped**，Python 3.12 / Linux。跳过为既有 Windows 专属与显式关闭的 live Provider 测试，未新增跳过来消除失败。
- 最终 Desktop 全量：**506 passed，1 skipped**。Runtime 子进程、IPC、重连与实际协议解析均包含在内；live Provider 不调用。
- Ruff、严格 mypy（Linux / win32 目标）、协议生成检查、Desktop 三套 TypeScript 配置、生产构建通过。
- 两平台 CI 已配置，但本次未推送/触发远程 CI，不能把配置存在视为 native Windows 或 Python 3.13 已通过。

| 验收重点 | 自动化证据 |
| --- | --- |
| 首次配置、失败状态、草稿与项目保留 | Desktop Composer/Settings/RuntimeStore 测试，真实 Electron 首次使用 |
| 写后 Provider 失败、取消/崩溃、连续失败、无关新问题 | `tests/test_history_v2.py` 使用真实 AgentLoop、OpenAI Adapter/MockTransport、文件工具、SQLite；命令取消实际启动子进程并等待 PID 就绪 |
| 预算与输入事实 | 21 个失败历史/边界场景及命令取消场景；完整历史/省略说明恰好达到上限与超限；Memory 独立计量、Step 冻结、原工具不重放 |
| 重复点击、断线、ACK 丢失 | 既有 `test_server.py` 与 Desktop `runtimeStore.integration.test.ts` 的提交 receipt、lost-ACK、重复提交回归继续通过 |
| 内容与差异 | `test_file_preview.py`、`test_file_changes.py`、`test_tools.py`：新建/覆盖/多处替换/中文、CRLF/BOM、无末尾换行、删除、大文件、读取和翻页期间变化 |
| 原子性、未知结果、保护值 | 事务失败回滚、写后取消/task.cancel、崩溃未提交无差异、旧正文凭据过滤、单次事件上限；未知结果不伪装未执行 |
| 兼容与重建 | 真实冻结 schema8 SQL/WAL 备份与失败回滚；混合 schema5/6、v1/v2 Golden Trace 重建；旧 queued v1 执行；磁盘删除/改变后旧差异不变 |
| 模型输入边界 | 实际 Adapter 请求包含失败工具事实；diff 私有字段不进入 to_wire/to_model_content；预览无 Journal 副作用 |

## Linux 桌面实测

使用生产 Electron 构建、独立 `/tmp` Runtime/配置/缓存/普通目录和本地 HTTP/SSE 模型桩。未读取个人 `~/.ikaros`、未使用远程模型或付费请求。

1. **配置并查看文件**：无配置入口进入真实设置表单，保存 Custom Provider 后草稿保留，发送后真实 `read` 执行；在右面板查看行号并把路径引用到已有草稿。
2. **修改项目并运行验证**：真实 `write → read → edit → process.run → read`；当前内容是 omega，而首次 write 差异仍是 alpha，edit 差异是 alpha→omega。另一个普通 Turn 用实际 Python 命令断言最终文件内容，输出 PASS，记录 `exitCode: 0`。脚本生成文件也可通过路径打开。
3. **失败、重启后继续**：实际 write 后模型桩返回 401；UI 保留成功写入及失败原因。退出并重启同一测试数据，历史原因与旧差异保持；发送普通新 Turn 后请求包含原写入配对与失败说明，没有重新写文件。用户主动取消由自动化测试覆盖，此次未在桌面 UI 实测。

最终记录 **5 个 Runs、13 个模型请求、3 条 file.change_recorded**，未出现额外旧操作。1440×900 与 1000×760 视口截图检查通过。隔离 Electron、Runtime 与模型桩均已退出。

人工/驱动介入：原生目录选择器不能通过 Renderer CDP 操作，测试通过真实 preload API 创建带普通工作目录的 Threads，其余配置、发送、预览、引用与历史操作通过 UI；重启由测试驱动终止再启动 Electron。本地连接测试需要允许 loopback 的沙箱权限。

本机详细步骤与截图位于仓库忽略目录 `.temp/daily-alpha-e2e-report.md`、`.temp/daily-alpha-e2e/`；这些含临时绝对路径的调试资料未作为产品数据提交。

## 未完成的外部验收与已知限制

- **Native Windows 的三项桌面任务、原生目录选择器、Python 3.13 实跑尚未完成**。发布跨平台 Alpha 前应运行配置好的 CI，并在 Windows 重做上述三项任务。
- 模型桩验证了 Ikaros 的链路与输入事实，未验证真实模型判断质量、外部 Provider 可用性或真实凭据配置错误修正的完整桌面流程。
- 文本仅支持 UTF-8；每页最多 50 KiB / 2,000 行，每次最多扫描 8 MiB。超长单行和过远偏移明确不可预览，不承诺遍历任意大小文件。
- before/after 各限 256 KiB / 5,000 行，完整序列化事件限 256 KiB；超限保留元数据，旧操作无快照显示未记录。命令造成的文件变化只可预览，不追踪差异。
- 没有手动编辑器、Git 暂存/提交 UI、附件、联网、自动 Memory、多 Agent 或 Python Runtime 安装包。

本轮后续优先完成 Windows 与真实 Provider 的验收，再依据日常使用中实际发生的阻碍选择下一里程碑。
