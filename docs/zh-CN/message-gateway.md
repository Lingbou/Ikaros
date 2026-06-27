# Message Gateway

Message gateway 是本地 inbox/outbox 与外部消息 adapter 边界。入站只入队，不执行工作；worker/drain 后续通过 `ikaros-agent::gateway_drain` 处理消息，并使用 CLI/host 边界装配好的依赖。

## 所有权

- 稳定跨 surface wire shape 放在 `ikaros-protocol`。
- 本地 inbox/outbox、lease、retry、dead-letter、worker lock state 放在 `ikaros-state::gateway`。
- adapter mapping、webhook ingress、admin operation 和 gateway-local policy checks 放在 `ikaros-surfaces::gateway`。

## 状态文件

```text
IKAROS_HOME/gateway/inbox.jsonl
IKAROS_HOME/gateway/outbox.jsonl
IKAROS_HOME/gateway/inbox.jsonl.lock
IKAROS_HOME/gateway/outbox.jsonl.lock
IKAROS_HOME/gateway/message-worker.lock
IKAROS_HOME/gateway/message-worker-events.jsonl
IKAROS_HOME/gateway/message-worker.stop
```

Gateway processing 还会把高层 request/result/delivery evidence 写入解析后的 agent `state.db` session store。gateway JSONL 文件仍是队列和投递状态；`state.db` 是把外部消息连接到 agent turn 的 replay timeline。

## Processing

`chat` 消息走和 `ikaros chat --message` 相同的治理路径。`task` 消息走 session-aware task agent-loop path，并携带 gateway 派生的 session id、turn id 和 session source。

Gateway 不授予额外权限；`agent` 只选择 agent context 和 policy overlay。

## Invariants

- Ingestion 不执行模型、工具、插件或任务。
- Inbox/outbox mutation 必须持有 gateway JSONL lock。
- 同一 `IKAROS_HOME` 只应有一个本地 `message worker`。
- Worker completion/failure 必须绑定 claimed lease owner 和 attempt count。
- Stable gateway/session wire shape 属于 `ikaros-protocol`。
- Adapter 应 target inbox/outbox queue，不应调用 agent internals。
