# Persona 模型

Persona �?prompt 和上下文输入，不是执行权限�?

## 存储

默认路径�?

```text
IKAROS_HOME/persona.md
```

Loader 保留 markdown，并解析 identity、traits、tone、relationship stance、boundaries �?behavior rules 等常�?section�?

## 命令

```bash
ikaros persona show
ikaros persona set --name Ikaros --tone "calm, direct"
ikaros persona reset
```

`persona set` 只写�?`IKAROS_HOME/persona.md`，拒绝疑�?secret 值，并记�?audit event�?

## Emotion

Runtime emotion state 很小，并�?audit 支撑。当�?signal 会把 task/chat 结果映射�?neutral、focused、curious、concerned�?
confused、satisfied 等状态�?

Body renderer �?runtime/audit state 读取最�?emotion。Persona 文本不能设置策略或权限�?

## 关系记忆

关系记忆作为本地 `Relationship` record 存储，并通过以下命令展示�?

```bash
ikaros relationship remember "Prefer short updates" --scope user
ikaros relationship show --scope user
```

Chat 可以在脱敏和去重后提取明确偏好，但自动观察会先进�?memory candidate inbox。使�?`--no-relationship-learning` 可在单轮中关�?candidate 创建�?

## 边界

Persona 可以影响 tone、context priority �?prompt wording。它不能授予工具、secret、代码变更、审批或 provider 凭证访问权限�?
