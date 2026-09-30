# Validation Status

更新日期：2026-10-01。

本文记录离线门禁、真实 Provider 验证和仍未完成的验收。没有运行记录的能力一律标为待验证，不把自动化测试或历史运行描述成当前版本已验证。

## 当前结论

- 离线确定性测试在当前长任务内核上通过。
- 当前开发数据中已有真实 DeepSeek 简单任务完成记录。
- 真实多步骤、多次上下文压缩、大输出、64 并发进程和 Windows 实机验收尚未完成。
- 本次文档清理前的最后一次完整离线验证基于 2026-09-18 代码状态。

## 离线基线

最近一次完整门禁结果：

| 检查 | 结果 |
| --- | --- |
| Runtime pytest | 781 passed, 2 skipped |
| Desktop Vitest | 528 passed, 1 skipped |
| Ruff | passed |
| strict mypy（src + tests） | passed |
| Protocol generation check | passed |
| Golden Trace check | passed |
| Desktop typecheck | passed |
| Desktop production build | passed |

两个 Runtime skip 分别是 Windows 专属进程启动测试和显式 opt-in 的真实 Provider 测试。Desktop skip 是 live DeepSeek smoke。

这些结果证明当前协议、存储、Agent loop、工具、恢复、压缩和 UI 的确定性行为；它们不等于真实模型质量和真实操作系统验收。

## 真实 DeepSeek 记录

2026-09-18 在当前 Linux 开发环境中，使用真实 DeepSeek 完成了 3 个简单 Run：

- 3 个 Run 全部 completed；
- 其中一个 Run 执行了 3 次主模型调用；
- 0 次自动压缩；
- 没有覆盖长命令、大输出、并发进程或 Windows 原生进程树。

这证明当前主请求、流式响应、工具调用和状态落盘至少完成了基础真实链路，但还不能替代长任务验收。

## 待完成的真实验收

| 场景 | 当前状态 |
| --- | --- |
| 真实 Provider 连续 30+ Steps | 待验证 |
| 同一 Run 触发 2 次以上自动压缩 | 待验证 |
| 压缩后通过 `history_read` 找回原始证据 | 待验证 |
| 单进程超过 1 MiB head/tail | 待验证 |
| 64 个并发进程和配额边界 | 待验证 |
| 网络断开但尚未产生模型内容时重连 | 待验证 |
| Windows 进程树、Job Object 和取消 | 待验证 |
| Linux 长命令、补充、崩溃恢复 | 部分自动化覆盖，真实 Provider 待验证 |
| Python Runtime 打包和安装包 | 未实现 |

## 历史验证

2026-08 至 2026-09 期间完成过 Windows 上的早期 live smoke、Identity A/B、部分 Memory 和文件工具验证。这些记录对应旧的 `process_run` 和旧协议，不能直接证明当前版本。

旧的历史记录已删除，保留当前结论和待办。需要具体历史证据时从 Git 历史读取对应旧版本文档。

## 如何运行 live smoke

Desktop 端：

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

普通 `pnpm test` 和 `pytest` 默认跳过这些测试。

## 凭据边界

Live smoke 使用临时 Runtime home，并读取显式指定的 key file。凭据不应出现在命令行参数、事件、SQLite、日志、UI projection 或诊断输出中。验证完成后临时 Runtime home 应被清理。
