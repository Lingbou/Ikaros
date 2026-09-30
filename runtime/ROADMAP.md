# Ikaros Roadmap

更新日期：2026-10-01。

本文描述当前基线和后续实施顺序。当前架构见 [DESIGN.md](DESIGN.md)，
模型输入与 Memory 设计见 [MODEL_INPUT_AND_MEMORY_DESIGN.md](MODEL_INPUT_AND_MEMORY_DESIGN.md)，
验证状态见 [LIVE_VALIDATION.md](LIVE_VALIDATION.md)。

## 当前基线

长任务执行内核已经实现：

- Thread、Branch、Turn、Run、Item 的当前唯一数据结构；
- 无总模型调用次数和总运行时长上限的 Run；
- Provider 流式响应、请求重试、流重连和超时；
- `process_start/read/wait/stop` 托管命令，单 Run 最多 64 个活动进程；
- 单进程 1 MiB head/tail 输出，模型每次最多读取 16 KiB；
- 运行中补充、取消、进程清理和重启后的未知状态恢复；
- ContextRevision、StepInput、Memory 冻结和可重建 Journal 投影；
- 95% 输入预算、90% 自动压缩、可重复压缩和 `history_read`；
- 有界完成检查，复用当前上下文并禁止工具调用；
- DeepSeek 和 OpenAI-compatible Provider；
- Runtime-backed Desktop、设置、Memory 管理、文件查看和 usage 展示。

当前不提供旧格式迁移。开发数据不兼容时通过显式重置处理。

## P0：长任务验收与可靠性

这是下一阶段的阻塞项。以下验收完成前，不进入大规模能力扩张：

1. **真实多压缩任务**
   - 使用真实 Provider 跑至少 30 个主模型步骤。
   - 触发至少 2 次自动压缩。
   - 验证用户要求、决策、工具事实和未完成工作没有丢失。
   - 验证 `history_read` 能找回被压缩的原始证据。

2. **大输出与并发进程**
   - 验证超过 1 MiB 的命令输出保留 head/tail 和省略标记。
   - 验证 64 个并发进程和第 65 个进程的配额行为。
   - 验证模型工具结果只包含有界分页，不携带完整 stdout/stderr。

3. **进程输出持久化**
   - 当前运行中仍会周期性写完整 bounded snapshot。
   - 改为增量输出事件或 checkpoint/终态快照，避免大输出造成 SQLite 写放大。

4. **网络恢复**
   - 补流在未产生模型内容前断开并重连的测试。
   - 验证重连不重复文本、tool call、usage 或取消状态。

5. **Windows/Linux 实机验收**
   - Windows 进程树、Job Object、输出截断和取消。
   - Linux 长命令、补充、崩溃恢复和 schema reset。
   - 两个平台分别使用真实 Provider 完成任务。

6. **发布流程**
   - 开发改动使用独立分支和 PR。
   - 提交使用 `-s -S`，不再直接 force push `main`。
   - 远程 CI、签名和分支保护必须成为常规门禁。

## P1：任务控制与安全模型

长任务内核稳定后，下一步是定义用户如何控制工作：

1. **Branch / Retry / Resume**
   - Branch 是编辑历史还是只做并行探索；
   - Retry 是重跑一个 Run 还是从失败 Step 继续；
   - Resume 恢复的是上下文、工具进度还是外部进程。

2. **权限系统**
   - 只读、自动批准、逐次确认和全权限模式；
   - 沙箱边界与操作系统权限说明；
   - 命令、文件写入、网络和 Skill 脚本的审批策略。

3. **检查点与任务计划**
   - 用户可见的任务计划和进度；
   - 可恢复检查点、artifact 和未完成事项；
   - 不把模型摘要当作唯一的恢复来源。

4. **进程体验**
   - 完整日志面板；
   - PTY 交互；
   - 更清晰的停止、等待、补发和进程重连体验。

## P2：平台能力

在单 Agent、单活动 Run 的可靠性达标后，再扩展并发和外部能力：

- 多个同时运行的 Run；
- Multi-Agent 和子 Agent 生命周期；
- 跨 Run/重启保留进程；
- cron 和主动唤醒任务；
- 附件、Web、浏览器和桌面操作；
- MCP、插件和第三方工具生态；
- 自动 Memory 提取、Skill 学习和语义检索。

这些能力涉及新的权限、所有权和恢复边界，不应在 P0/P1 之前并行堆叠。

## P3：发布与运维

- 打包 Python Runtime；
- 安装包、自动更新和回滚；
- 崩溃报告、诊断包和日志导出；
- 数据备份、恢复、迁移和发布策略；
- 多平台签名与供应链安全。

## 每阶段完成标准

每个阶段至少满足：

- 有明确的用户可见交付，不只是内部抽象；
- 有确定性的回归测试；
- 有真实 Provider 或真实 OS 的验证记录；
- 失败、取消、重启和未知状态有明确行为；
- 不引入第二份事实来源或兼容分支；
- 文档、协议生成物和迁移策略同步更新。
