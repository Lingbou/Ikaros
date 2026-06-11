# 威胁模型

本文描述当前本地 MVP 的威胁模型。它不足以覆盖托管或多用户部署。

## 受保护资产

- 存在本地 `IKAROS_HOME/config.toml` `[providers.*]` 表里的 API key。
- 用户记忆和关系笔记。
- 聊天历史。
- 项目文件。
- RAG 索引。
- 审计日志和审批记录。
- Self-modify proposal 和 rollback snapshot。

## 信任边界

- 工具执行前的 harness policy。
- 需要用户审批写入时的 approval replay。
- 写入 audit/model/RAG/provider 存储前的脱敏。
- `IKAROS_HOME` 下的本地状态。
- Cloud model、embedding、TTS 和 ASR 的 provider adapter。
- Plugin manifest 和 command-backed plugin execution。

## 当前控制

- 默认拒绝 destructive action、direct secret access、publish/commit action、workspace-external write 和普通 self-modify。
- 默认本地优先存储。
- 默认是协议级 provider 配置；远程调用前要求本地 key、base URL 和模型名齐全。
- 本地 provider 设置在写入日志和审计输出前脱敏。
- 为 policy decision 和 tool result 写 audit log。
- 拒绝疑似 secret 的 memory 内容。
- 校验 plugin command path。

## 已知限制

- Redaction 是启发式的，可能漏掉 secret。
- Shell/test skill 使用结构化 allowlist command，但这仍不是 process sandbox。
- 这不是 process sandbox 或 VM 边界。
- 没有多租户隔离。
- Browser/dashboard hardening 仅按本地 preview 假设。
- 远程部署仍是手动测试环境事项，不是生产 hardening。

## 托管使用前的阻塞项

任何托管或多用户部署前，都需要更强 sandbox、认证、网络暴露审查、secret storage 集成、依赖审查和运维事故流程。
