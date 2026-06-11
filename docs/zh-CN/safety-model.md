# 安全模型

Ikaros 把工具使用视为受控操作。Harness 是模型、persona、命令或调度器请求工作和 runtime 真正执行工作之间的边界。

## 默认策略

默认策略偏保守：

- workspace 内的安全读取允许。
- workspace 写入通常需要审批。
- workspace 外写入拒绝。
- workspace 外读取需要审批。
- 疑似 secret 路径需要审批或专用 secret adapter。
- 疑似 secret 的记忆内容会被拒绝。
- 破坏性 shell 命令拒绝。
- git commit、push、tag、release 和 package publish 动作拒绝。
- 直接 secret 访问拒绝。
- 普通 self-modify 拒绝。

Agent profile 可以改变普通策略选择，例如 workspace 写入、shell 动作或网络动作是允许、拒绝还是需要审批。Profile 不能削弱上面的硬性拒绝。

## 审计和审批

通过 harness 执行的技能会记录：

- `tool_call`
- `policy_decision`
- `tool_result`

审批请求保存在本地 audit 状态下，并绑定创建请求时的 workspace。审批重放会验证这个绑定。

Dry-run 仍然评估策略并写审计事件，但允许的技能返回 dry-run 结果，不修改本地状态。

审计日志默认写入 `audit.jsonl`，并在文件超过 16 MiB 或事件日期变化时轮转；旧 JSONL 文件会压缩成 `.gz` 归档，避免单个审计文件无限增长。

## Provider 安全

模型、RAG embedding 和语音 provider 都通过 adapter 访问。当前实现支持的路径会在 provider 调用前脱敏。用量日志只存 provider、model、token 等元数据，不存 prompt。

Provider key 和 base URL 从本地 `IKAROS_HOME/config.toml` 的 `[providers.*]` 表读取。不要把 key 写入仓库文件、审计日志、记忆或 RAG 索引。

## 本地自动化

计划任务和 gateway 消息只是在请求工作，不授予权限。计划任务或 gateway task 被处理时，仍然走显式 CLI task 相同的 runtime 和 harness 路径。

Loopback message webhook 只写入脱敏后的 inbox 记录，不直接执行工具或调用模型。

## 插件

命令型插件只通过内置 plugin runner skill 执行。插件 manifest 声明 risk 和 command 元数据，但运行时仍会做策略评估。插件 stdin/stdout/stderr 会在 harness 输出中脱敏。

## Self-Modify

Self-modify 有单独命令：

```bash
ikaros self-modify propose
ikaros self-modify request-apply
ikaros self-modify apply-approved
ikaros self-modify rollback
```

它需要已存储的 proposal、rollback snapshot、明确 approval id、drift check 和受限 check command。这个路径用于让变更可审查、可回滚，不代表开放自动自修改。
