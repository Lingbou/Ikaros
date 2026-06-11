# Persona 模型

Persona 是 prompt 和上下文输入，不是执行权限。

## 存储

默认路径：

```text
IKAROS_HOME/persona.md
```

Loader 保留 markdown，并解析 identity、traits、tone、relationship stance、boundaries 和 behavior rules 等常见 section。

## 命令

```bash
ikaros persona show
ikaros persona set --name Ikaros --tone "calm, direct"
ikaros persona reset
```

`persona set` 只写入 `IKAROS_HOME/persona.md`，拒绝疑似 secret 值，并记录 audit event。

## Emotion

Runtime emotion state 很小，并由 audit 支撑。当前 signal 会把 task/chat 结果映射到 neutral、focused、curious、concerned、confused、satisfied 等状态。

Body renderer 从 runtime/audit state 读取最新 emotion。Persona 文本不能设置策略或权限。

## 关系记忆

关系记忆作为本地 `Relationship` record 存储，并通过以下命令展示：

```bash
ikaros relationship remember "Prefer short updates" --scope user
ikaros relationship show --scope user
```

Chat 可以在脱敏和去重后学习明确偏好。使用 `--no-relationship-learning` 可在单轮中关闭。

## 边界

Persona 可以影响 tone、context priority 和 prompt wording。它不能授予工具、secret、代码变更、审批或 provider 凭证访问权限。
