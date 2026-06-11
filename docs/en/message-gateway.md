# Message Gateway

The message gateway is a local inbox/outbox and lightweight protocol boundary for external adapters. Ingestion does not execute work; worker/drain paths later process messages through runtime and harness flows.

The gateway is deliberately a queue boundary. Adapters may enqueue redacted
requests and read redacted deliveries, but they do not receive extra permission
to run models or tools.

## State

```text
IKAROS_HOME/gateway/inbox.jsonl
IKAROS_HOME/gateway/outbox.jsonl
```

Inbox records include:

- `id`
- `source`
- `kind`
- redacted `content`
- optional `agent`
- `session`
- `client`
- `capabilities`
- `idempotency_key`
- `status`
- timestamps and processing summary

Enqueueing the same `idempotency_key` reuses the existing record so external channel retries do not duplicate execution.

Message statuses:

- `Pending`: queued and ready to be claimed.
- `Processing`: claimed by a worker.
- `Processed`: handled successfully and, when applicable, delivered to outbox.
- `Failed`: processing returned an error.

Workers may reclaim stale processing messages. This makes local retry possible
without treating every adapter retry as a new task.

## Protocol Types

Gateway protocol types live inside `ikaros-gateway`; there is no separate protocol crate.

Core types:

- `GatewayFrame`
- `GatewayFramePayload`
- `GatewayConnect`
- `GatewayRequest`
- `GatewayResponse`
- `GatewayEvent`
- `GatewaySessionSource`
- `GatewayClientIdentity`
- `GatewayCapability`

Frames support `connect`, `request`, `response`, and `event`. `GatewaySessionSource` describes channel/account/peer/thread/message_id style origins. Frames and routes are redacted before storage.

Protocol frames use `ikaros.gateway.v1`. A request frame contains:

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

All fields that may contain secrets are redacted when a frame becomes a stored
route or message. The `idempotency_key` is also redacted before storage.

## Commands

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

## Processing

`message drain` and `message worker` process pending inbox records through `ikaros-runtime`.

- `chat` messages use the same governed chat path as `ikaros chat --message`.
- `task` messages use the deterministic harness task runner.

Successful outputs are written to the local outbox. The gateway does not grant additional permissions; agent/profile data only affects runtime context and policy overlay.

Processing context:

1. The worker claims a pending inbox record.
2. Runtime resolves the requested agent/profile or falls back to the configured
   default.
3. Chat or task execution builds a normal harness session.
4. Policy, approvals, audit, memory, and provider governance apply exactly as
   they do for direct CLI commands.
5. The inbox record is marked processed or failed.
6. Successful results create a `GatewayDelivery` in the outbox.

If a message requires approval, the worker records the pending approval state in
the processing summary; it does not auto-approve or retry with broader
permissions.

## Webhook

The webhook binds to loopback by default and accepts JSON at `POST /message`:

```json
{"content":"hello","kind":"chat","source":"local-webhook","profile":"plan"}
```

The webhook only enqueues a redacted message. It does not call models, tools, plugins, or task runners.

## Invariants

- Ingestion is non-executing.
- Gateway records are local state under `IKAROS_HOME/gateway`.
- The protocol lives inside `ikaros-gateway`; adapters should depend on the
  frame shape, not on runtime internals.
- Session source identifies the external conversation; `agent` selects runtime
  context but does not grant permissions.
- Outbox delivery content is redacted before storage.
