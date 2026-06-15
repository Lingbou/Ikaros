# 消息网关

消息网关是外部 adapter 使用的本地 inbox/outbox 和轻量协议边界。消息进入时不会执行工作；worker/drain 再通过 runtime 和 harness 路径处理。

Gateway 刻意保持为队列边界。Adapter 可以入队脱敏请求并读取脱敏 delivery，但不会因此获得运行模型或工具的额外权限。

## 状态

```text
IKAROS_HOME/gateway/inbox.jsonl
IKAROS_HOME/gateway/outbox.jsonl
IKAROS_HOME/gateway/inbox.jsonl.lock
IKAROS_HOME/gateway/outbox.jsonl.lock
```

Gateway 处理时还会把高层 evidence 写入解析后的 agent `state.db` session store。Gateway JSONL 文件仍然是队列和 delivery 状态；`state.db` 是把外部消息和 runtime turn 串起来的 replay timeline。

Inbox 和 outbox JSONL 文件会用同目录 sibling lock file 保护。Gateway 写入时会先拿独占 filesystem lock 和同进程 guard，再读取并重写 JSONL 状态。Lock file 使用后可能继续留在磁盘上；它们只是协调文件，不是 stale queue record，也不应被当成 message state。

Inbox record 包含：

- `id`
- `source`
- `kind`
- 脱敏后的 `content`
- 可选 `agent`
- `session`
- `client`
- `capabilities`
- `idempotency_key`
- `status`
- timestamp 和 processing summary

相同 `idempotency_key` 的入队会复用已有记录，避免外部 channel 重试时重复执行。

消息状态：

- `Pending`：已入队，等待 claim。
- `Processing`：已被 worker claim。
- `Processed`：处理成功，并在需要时写入 outbox。
- `Failed`：处理返回错误。

Worker 可以回收 stale processing message。这样本地 retry 不会把 adapter 的每次重试都当成新任务。
这个 stale-processing 回收基于 message timestamp，不通过删除 lock file 判断。

## 协议类型

Gateway protocol 类型放在 `ikaros-gateway` 内部，不单独建 crate。

核心类型：

- `GatewayFrame`
- `GatewayFramePayload`
- `GatewayConnect`
- `GatewayRequest`
- `GatewayResponse`
- `GatewayEvent`
- `GatewaySessionSource`
- `GatewayClientIdentity`
- `GatewayCapability`

Frame 支持 `connect`、`request`、`response` 和 `event`。`GatewaySessionSource` 用于描述 channel/account/peer/thread/message_id 这类会话来源。所有 frame/route 入库前都会脱敏。

协议 frame 使用 `ikaros.gateway.v1`。Request frame 示例：

```json
{
  "protocol": "ikaros.gateway.v1",
  "source": {
    "channel": "local",
    "account": "acct",
    "peer": "peer",
    "thread": "thread",
    "message_id": "msg"
  },
  "payload": {
    "type": "request",
    "kind": "chat",
    "content": "hello",
    "agent": "build"
  }
}
```

所有可能含 secret 的字段在 frame 转换为持久 route/message 时都会脱敏。`idempotency_key` 入库前也会脱敏。

## 命令

```bash
ikaros message send "hello" --kind chat
ikaros message send "summarize project" --kind task --profile plan
ikaros message list
ikaros message drain --dry-run
ikaros message drain --limit 5
ikaros message worker --once
ikaros message outbox
ikaros message delete <id>
ikaros message webhook --port 8002
```

## 处理

`message drain` 和 `message worker` 通过 `ikaros-runtime` 处理 pending inbox record。

- `chat` 消息使用 `ikaros chat --message` 相同的 governed chat path。
- `task` 消息使用确定性的 harness task runner。

成功输出写入本地 outbox。Gateway 自身不授予额外权限，agent/profile 只影响 runtime 上下文和 policy overlay。

处理上下文：

1. Worker claim 一个 pending inbox record。
2. Runtime 解析请求的 agent/profile；缺省时回退到配置默认值。
3. Chat 或 task execution 创建普通 harness session。
4. Policy、approval、audit、memory 和 provider governance 与直接 CLI 命令保持一致。
5. Inbox record 标记为 processed 或 failed。
6. 成功结果创建 `GatewayDelivery` 写入 outbox。
7. Session store 写入脱敏后的 request/result/delivery evidence。

`chat` 消息复用由 gateway channel、account、peer、thread/message id 派生出的 chat session id。`task` 消息会写入 gateway-scoped user entry、runtime result entry，以及 typed start/end/error event。Delivery record 只记录 outbox delivery id 和 kind，不复制未脱敏的完整输出。

如果消息需要审批，worker 会在 processing summary 中记录等待审批状态；它不会自动审批，也不会用更宽权限重试。

## Webhook

Webhook 默认绑定 loopback，并在 `POST /message` 接受 JSON：

```json
{"content":"hello","kind":"chat","source":"local-webhook","profile":"plan"}
```

Webhook 只入队脱敏消息，不调用模型、工具、插件或任务 runner。

## 不变量

- Ingestion 不执行工作。
- Gateway record 是 `IKAROS_HOME/gateway` 下的本地状态。
- 修改 inbox/outbox 必须持有 gateway JSONL lock；adapter 不应直接编辑 JSONL 文件。
- 协议类型位于 `ikaros-gateway` 内部；adapter 应依赖 frame shape，而不是 runtime internal。
- Session source 标识外部 conversation；`agent` 选择 runtime 上下文，但不授予权限。
- Outbox delivery content 入库前会脱敏。
- Session replay 是 evidence，不是第二套队列。Adapter 仍应使用 inbox/outbox 判断 delivery 状态。
