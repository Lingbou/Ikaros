# 消息网关

消息网关是外部 adapter 使用的本地 inbox/outbox 和轻量协议边界。消息进入时不会执行工作；worker/drain 再通过 runtime 和 harness 路径处理。

Gateway 刻意保持为队列边界。Adapter 可以入队脱敏请求并读取脱敏 delivery，但不会因此获得运行模型或工具的额外权限。

## 状态

```text
IKAROS_HOME/gateway/inbox.jsonl
IKAROS_HOME/gateway/outbox.jsonl
IKAROS_HOME/gateway/inbox.jsonl.lock
IKAROS_HOME/gateway/outbox.jsonl.lock
IKAROS_HOME/gateway/message-worker.lock
IKAROS_HOME/gateway/message-worker-events.jsonl
IKAROS_HOME/gateway/message-worker.stop
```

Gateway 处理时还会把高层 evidence 写入解析后的 agent `state.db` session store。Gateway JSONL 文件仍然是队列和 delivery 状态；
`state.db` 是把外部消息和 runtime turn 串起来的 replay timeline。

Inbox 和 outbox JSONL 文件会用同目录 sibling lock file 保护。Gateway 写入时会先拿独占 filesystem lock 和同进程 guard，再读取并重写
JSONL 状态。Lock file 使用后可能继续留在磁盘上；它们只是协调文件，不是 stale queue record，也不应被当成 message state。

`message-worker.lock` 的语义不同：它是本地 worker 进程锁，用来避免同一个
`IKAROS_HOME` 下同时启动两个 gateway worker。`message worker` 在轮询 inbox 前会写入当前
PID 和启动时间；如果 lock 已存在，会 fail-fast 并打印脱敏后的已有 owner；worker 正常退出时会释放自己的 lock。
在 Linux 上，如果 lock 内的 PID 已不存在，worker 会把它当作 stale crash evidence，归档为
`message-worker.lock.stale.<timestamp>`，然后用新的 lock 启动。

`message-worker-events.jsonl` 是本地 worker 入口的小型 shutdown-forensics 日志。每次运行会追加一条已脱敏的
`started` event 和一条终态 `stopped` event，状态可能是 `completed`、`failed` 或尽力写入的
`aborted`。它不替代 inbox/outbox 队列或 session timeline，只用于让本地 operator 判断
service-managed worker 是否干净退出。

`message-worker.stop` 是协作式停止请求。`message worker-stop` 会把脱敏后的 reason 写入这个文件；
worker 在下一次 poll 前消费它、干净退出、删除 stop 文件，并写入 stopped forensic event。
`message daemon stop` 写入同一类请求。它是控制面命令，不会 kill 任意宿主机进程。

Inbox record 包含：

- `id`
- `source`
- `kind`
- 脱敏后的 `content`
- 可选 `agent`
- `session`
- `client`
- `capabilities`
- `safe_tools`
- `idempotency_key`
- `status`
- `attempt_count`
- 可选 `lease_owner`
- 可选 `lease_expires_at`
- 可选 `last_error`
- 可选 `dead_lettered_at`
- timestamp 和 processing summary

Outbox delivery record 包含：

- `id`
- `message_id`
- `kind`
- 脱敏后的 `content`
- `status`
- `attempt_count`
- 可选 `lease_owner`
- 可选 `lease_expires_at`
- 可选 `next_attempt_at`
- 可选 `delivered_at`
- 可选 `dead_lettered_at`
- 可选 `last_error`
- 可选 `summary`
- `created_at`

相同 `idempotency_key` 的入队会复用已有记录，避免外部 channel 重试时重复执行。

消息状态：

- `Pending`：已入队，等待 claim。
- `Processing`：已被 worker claim。
- `Processed`：处理成功，并在需要时写入 outbox。
- `Failed`：处理返回错误。
- `Cancelled`：被 operator 或控制面在 delivery 前取消。
- `DeadLettered`：处理失败且 worker retry budget 已耗尽。

`Processed`、`Failed`、`Cancelled` 和 `DeadLettered` 是终态。消息进入终态后，后续没有绑定
live claim 的状态或失败回写会被忽略，不会重写这条记录。取消还会阻止旧 worker claim
在稍后成功完成时继续投递结果。

Delivery 状态：

- `Pending`：等待 adapter claim；如果 `next_attempt_at` 还在未来，则暂时不会被 claim。
- `Processing`：已被 adapter claim。
- `Delivered`：adapter delivery 成功。
- `DeadLettered`：adapter delivery 失败且 retry budget 已耗尽。

Delivery claim 和 inbox message claim 一样绑定 lease。Delivery 失败会清除 lease、记录脱敏错误，并根据 retry budget
设置 `next_attempt_at` 等待 retry/backoff，或移动到 `DeadLettered`。如果 delivery 已被另一个
adapter 回收，旧 adapter 的 completion 不能覆盖新 owner。

Worker claim 消息时会记录脱敏后的 lease owner、lease deadline 和 attempt count。处理失败会清除 lease，并根据 retry budget
把消息放回 `Pending` 等待重试，或移动到 `DeadLettered`。Worker 也可以回收 stale processing message。这样本地 retry 不会把
adapter 的每次重试都当成新任务。新记录用 `lease_expires_at` 判断 stale processing，旧记录回退到 message timestamp；不会通过删除 lock
file 判断。
Worker 的完成和失败回写必须绑定它 claim 到的 lease。也就是说回写时要携带 claim 中的
lease owner 和 attempt count；如果消息已经被另一个 worker 回收并重新 claim，旧 worker 的回写会被忽略，而不会覆盖新的 owner。

## 协议类型

Gateway 的本地 inbox/outbox frame 模型放在 `ikaros-gateway`。需要被 API、TUI、
replay 和后续 adapter 共享的稳定跨界 wire shape 属于 `ikaros-protocol`。

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
- `GatewayProtocolPolicy`

Frame 支持 `connect`、`request`、`response` 和 `event`。`GatewaySessionSource` 用于描述
channel/account/peer/thread/message_id 这类会话来源。所有 frame/route 入库前都会脱敏。

`GatewayProtocolPolicy` 会在 daemon 或 adapter 接受 frame 前校验协议版本、允许的 client id、允许的 channel 和必需
capability。这是长期多客户端 gateway daemon 暴露前的本地协议 hardening 层。

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
ikaros message send "hello" \
  --kind chat \
  --source telegram \
  --account acct \
  --peer peer \
  --thread thread \
  --message-id msg
ikaros message status
ikaros message list
ikaros message drain --dry-run
ikaros message drain --limit 5
ikaros message cancel <id> --reason "operator requested cancel"
ikaros message delivery claim --limit 5 --owner telegram-adapter
ikaros message delivery ack <delivery-id> --lease-owner telegram-adapter --summary "sent"
ikaros message delivery fail <delivery-id> \
  --lease-owner telegram-adapter \
  --reason "remote timeout" \
  --backoff-seconds 30 \
  --max-attempts 3
ikaros message adapter list
ikaros message adapter enqueue "hello" --platform telegram --kind chat
ikaros message adapter render-delivery <delivery-id> --platform telegram
ikaros message pairing create --source telegram --account bot-account --peer user-id
ikaros message pairing list
ikaros message worker --once
ikaros message worker-stop --reason "maintenance"
ikaros message daemon start --interval-seconds 5 --limit 10
ikaros message daemon status
ikaros message daemon stop --reason "maintenance"
ikaros message daemon restart --interval-seconds 5 --limit 10
ikaros message outbox
ikaros message delete <id>
ikaros message webhook --port 8002
ikaros message webhook --port 8002 --hmac-secret "$IKAROS_WEBHOOK_SECRET"
ikaros message webhook --port 8002 --allow-source telegram --allow-peer user-id
ikaros message webhook --port 8002 --require-pairing
ikaros message webhook --port 8002 --unsafe-tools
```

`message send` 可以通过 `--account`、`--peer`、`--thread` 和 `--message-id` 附带结构化
`GatewaySessionSource`；`--idempotency-key` 用于去重 adapter 重试。`message status`
会汇总队列数量、delivery pending/processing/delivered/dead-letter 数量、最近的 gateway-derived session id 和 CLI
resume 提示，同时不打印疑似 secret 的内容。它的 worker snapshot 还会报告当前
`message-worker.lock` owner 和 stale 标记、最新 worker forensic event、processing、stale-processing、
retryable 和 dead-letter 数量，并展示少量已脱敏的 active lease、带 last error 的 retryable
message、retryable delivery，以及带终态 evidence 的 dead-lettered message/delivery。
`message cancel` 会把 pending 或 processing message 移到 `Cancelled`，清除 lease，保存脱敏
reason，并让旧 claim 的后续 worker 完成回写变成 no-op。
`message delivery claim` 是 adapter 消费 outbox 的 lease 命令。Adapter 后续必须用同一个脱敏后的
lease owner 调用 `message delivery ack` 或 `message delivery fail`；owner 不匹配会被拒绝，旧
adapter 也不能覆盖已被回收的 delivery。`fail` 会记录脱敏 reason，并根据 retry budget 设置
`next_attempt_at` 或移动到 `DeadLettered`。
`message adapter list` 会输出内置 adapter descriptor，以及每个平台对应的 enqueue/render
命令。`message adapter enqueue` 会把平台形状的 inbound envelope 转成普通本地 inbox
记录。`message adapter render-delivery` 会把 outbox delivery 渲染成脱敏后的平台形状
outbound envelope，供 adapter 发送。它不会确认投递，也不会绕过 lease 路径；adapter
仍然需要使用 `message delivery claim`、`ack` 和 `fail` 维护 delivery 状态。
`message pairing create` 会为 source/account/peer 创建一次性配对码；code 只在创建时输出一次。
`message pairing list` 会脱敏已存储的 code，只展示本地 operator 需要的配对状态。

`message worker` 仍然是前台进程入口。`message daemon` 是同一条 worker 路径的本地 daemon
控制面：`start` 会在后台启动当前 `ikaros` 二进制，等待 `message-worker.lock` 建立，并把
stdout/stderr 写入 `IKAROS_HOME/gateway/message-worker-daemon.log`；`status` 报告 lock、
stop request 和最新 forensic event；`stop` 写入协作式停止请求；`restart` 会等待当前 worker
释放 lock 后再启动新的 worker。这是本地长期运行 worker，不是后续完整的多客户端平台 daemon。
`message worker-stop` 通过写入 `message-worker.stop` 请求协作式关闭；它不会杀任意宿主机进程。

## 处理

`message drain` 和 `message worker` 通过 `ikaros-runtime` 处理 pending inbox record。

- `chat` 消息使用 `ikaros chat --message` 相同的 governed chat path。
- `task` 消息使用 session-aware task agent-loop path，并携带 gateway 派生的 session
  id、turn id 和 session source，因此 typed event 可以和 gateway request/result/delivery
  evidence 落在同一个 `state.db` timeline 中。

成功输出写入本地 outbox。Gateway 自身不授予额外权限，agent/profile 只影响 runtime 上下文和 policy overlay。

处理上下文：

1. Worker claim 一个 pending inbox record。
2. Runtime 解析请求的 agent/profile；缺省时回退到配置默认值。
3. Chat 或 task execution 创建普通 harness session。
4. Policy、approval、audit、memory 和 provider governance 与直接 CLI 命令保持一致。
5. Inbox record 标记为 processed、放回 pending 等待 retry，或移动到 dead-lettered。
6. 成功结果创建 `GatewayDelivery` 写入 outbox。
7. Session store 写入脱敏后的 request/result/delivery evidence。

`chat` 和 `task` 消息的持久 session id 来自 gateway channel、account、peer 和 thread
的带版本 digest。原始 channel/account/peer/thread 不会嵌入 session id。`message_id`
作为单条消息的脱敏 source evidence 保存；当存在结构化 thread 时，它不决定 conversation
identity。`task` 消息会写入 gateway-scoped user entry、runtime result entry，以及 typed
start/end/error event。Delivery record 只记录 outbox delivery id 和 kind，不复制未脱敏的完整输出。

如果消息需要审批，worker 会在 processing summary 中记录等待审批状态；它不会自动审批，也不会用更宽权限重试。

## Webhook

Webhook 默认绑定 loopback，并在 `POST /message` 接受 JSON：

```json
{"content":"hello","kind":"chat","source":"local-webhook","profile":"plan"}
```

Adapter payload 也可以携带结构化路由字段：

```json
{
  "content": "hello",
  "kind": "chat",
  "source": "telegram",
  "account": "bot-account",
  "peer": "user-id",
  "thread": "chat-or-thread-id",
  "message_id": "platform-message-id",
  "idempotency_key": "telegram:chat-or-thread-id:platform-message-id",
  "pairing_code": "one-time-code-from-message-pairing-create",
  "profile": "plan"
}
```

`source` 会成为 gateway channel，`account`/`peer`/`thread`/`message_id`
会成为结构化 `GatewaySessionSource`，`idempotency_key` 会在执行前去重 adapter 重试。

`--allow-source`、`--allow-account`、`--allow-peer` 和 `--allow-thread`
提供本地 adapter ACL。配置任意 allowlist 后，对应 payload 字段必须匹配才会入队。
被拒绝的请求返回
`403 Forbidden` 和被拒绝字段名，不会创建 inbox record。

设置 `--require-pairing` 后，route 的 source/account/peer 必须已经配对。请求可以携带一次
`pairing_code` 来绑定 `message pairing create` 创建的 pending pairing；之后同一个
peer 的请求不需要重复携带 code。未配对请求会在入队前返回 `403 Forbidden`。

Webhook 消息默认会标记 `safe_tools`。Runtime drain 处理这类 chat/task agent loop 时只暴露
`core` toolset，远程平台用户不能通过模型直接拿到 workspace、coding、RAG、voice 或 plugin 工具。
`--unsafe-tools` 会关闭这个标记，只应在本地可信 adapter 且已有 HMAC、ACL、pairing 控制时使用。

设置 `--hmac-secret` 后，`POST /message` 必须携带
`X-Ikaros-Signature: sha256=<hex-hmac>`。HMAC-SHA256 的输入是原始 request body
bytes，key 是配置的 secret。缺失、格式错误或校验失败会在 payload 解析和入队前返回
`401 Unauthorized`。签名只要求 `POST /message`，`/healthz` 仍然不需要签名，方便本地 service manager 探活。

Webhook 只入队脱敏消息，不调用模型、工具、插件或任务 runner。

## 不变量

- Ingestion 不执行工作。
- Gateway record 是 `IKAROS_HOME/gateway` 下的本地状态。
- 修改 inbox/outbox 必须持有 gateway JSONL lock；adapter 不应直接编辑 JSONL 文件。
- 同一个 `IKAROS_HOME` 应只运行一个本地 `message worker`；worker 轮询期间必须持有
  `message-worker.lock`。
- Worker 完成/失败回写必须绑定 claim 的 lease owner 和 attempt count，防止 stale worker 在消息被回收后继续完成旧 claim。
- Inbox 终态（`Processed`、`Failed`、`DeadLettered`）对后续无 claim 的状态/失败回写不可变。
- 协议类型位于 `ikaros-gateway` 内部；adapter 应依赖 frame shape，而不是 runtime internal。
- Session source 标识外部 conversation；`agent` 选择 runtime 上下文，但不授予权限。
- Outbox delivery content 入库前会脱敏。
- Webhook HMAC 校验发生在入队前；被拒绝的请求不会创建 inbox record。
- Webhook ACL 校验也发生在入队前；被拒绝的 source/account/peer/thread 不会写入 gateway queue。
- 启用 `--require-pairing` 时，Webhook pairing 校验也发生在入队前；pairing code 不会复制进 inbox message。
- Webhook 消息默认进入 `safe_tools`；关闭它必须是本地 operator 的显式选择。
- Session replay 是 evidence，不是第二套队列。Adapter 仍应使用 inbox/outbox 判断 delivery 状态。
