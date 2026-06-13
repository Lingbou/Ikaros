# Self-Modify

Self-modify 是一个很窄的 proposal/apply 工作流。它不是让 agent 任意改写自己的通用权限。

## 流程

```bash
ikaros self-modify propose --kind documentation-patch --target README.md --diff "<unified diff>"
ikaros self-modify request-apply <proposal-id>
ikaros approval approve <approval-id>
ikaros self-modify apply-approved <proposal-id> --approval-id <approval-id>
```

回滚：

```bash
ikaros self-modify rollback <proposal-id>
```

检查：

```bash
ikaros self-modify list
ikaros self-modify operations
ikaros self-modify heartbeat
```

## 保证

- `RiskLevel::SelfModify` 对普通 tool dispatch 是拒绝的。
- Proposal 保存脱敏 diff summary 和 rollback snapshot。
- Apply 需要专用 approval id。
- Approval 必须匹配 proposal 和 workspace。
- Target 不能相对 snapshot 发生 drift。
- Apply 前后会运行受限 check command。
- Pre-check 失败会在 mutation 前停止。
- Post-check 失败会触发 rollback。
- Operation 记录在本地 self-modify state。

## 状态

```text
IKAROS_HOME/self-modify/proposals.jsonl
IKAROS_HOME/self-modify/operations.jsonl
IKAROS_HOME/self-modify/rollback/<proposal-id>/target.snapshot
```

## Check Profile

内置 profile 使用受限 test/check/lint/build 命令。配置可以覆盖：

```yaml
self_modify:
  check_profiles:
    runtime_patch:
      commands:
        - cargo check --workspace --all-features
      reason: "Runtime patches must keep the workspace compiling."
```

允许的 kind：

- `skill_patch`
- `persona_patch`
- `config_patch`
- `runtime_patch`
- `documentation_patch`

Shell chaining、redirection、command substitution、publishing 和 git commit/push/tag 仍然拒绝。

## 未实现

自动 apply 不属于当前合约。当前版本只支持显式 proposal、审批、apply-approved 和 rollback 流程。
