# Message Gateway

The message gateway is a local inbox/outbox and lightweight protocol boundary for external adapters.
Ingestion does not execute work; worker/drain paths later process messages through runtime and
harness flows.

The gateway is deliberately a queue boundary. Adapters may enqueue redacted
requests and read redacted deliveries, but they do not receive extra permission
to run models or tools.

## State

```text
IKAROS_HOME/gateway/inbox.jsonl
IKAROS_HOME/gateway/outbox.jsonl
IKAROS_HOME/gateway/inbox.jsonl.lock
IKAROS_HOME/gateway/outbox.jsonl.lock
IKAROS_HOME/gateway/message-worker.lock
IKAROS_HOME/gateway/message-worker-events.jsonl
IKAROS_HOME/gateway/message-worker.stop
```

Gateway processing also writes high-level evidence into the resolved agent
`state.db` session store. The gateway JSONL files remain the queue and delivery
state; `state.db` is the replay timeline that links the external message to the
runtime turn.

The inbox and outbox JSONL files are protected by portable sibling lock files.
Gateway writes take an exclusive filesystem lock and an in-process guard before
reading and rewriting JSONL state. The lock files may remain on disk after use;
they are coordination files, not stale queue records, and should not be used as
message state.

`message-worker.lock` is different: it is the local worker process lock used to
avoid starting two gateway workers against the same home directory. `message
worker` creates it with the current PID and start time before polling the inbox,
fails fast if it already exists, prints the redacted existing owner, and removes
the lock again when that worker exits normally.
On Linux, if the lock contains a PID that no longer exists, the worker treats it
as stale crash evidence, archives it as `message-worker.lock.stale.<timestamp>`,
and starts with a fresh lock.

`message-worker-events.jsonl` is a small shutdown-forensics log for the local
worker entrypoint. Each run appends a redacted `started` event and a terminal
`stopped` event with `completed`, `failed`, or best-effort `aborted` status. It
does not replace the inbox/outbox queue or the session timeline; it exists so a
local operator can tell whether a service-managed worker exited cleanly.

`message-worker.stop` is a cooperative stop request. `message worker-stop`
writes a redacted reason to this file; the worker consumes it before the next
poll, exits cleanly, removes the file, and records a stopped forensic event.
`message daemon stop` writes the same request. It is a control-plane command,
not a host process killer.

Inbox records include:

- `id`
- `source`
- `kind`
- redacted `content`
- optional `agent`
- `session`
- `client`
- `capabilities`
- `safe_tools`
- `idempotency_key`
- `status`
- `attempt_count`
- optional `lease_owner`
- optional `lease_expires_at`
- optional `last_error`
- optional `dead_lettered_at`
- timestamps and processing summary

Outbox delivery records include:

- `id`
- `message_id`
- `kind`
- redacted `content`
- `status`
- `attempt_count`
- optional `lease_owner`
- optional `lease_expires_at`
- optional `next_attempt_at`
- optional `delivered_at`
- optional `dead_lettered_at`
- optional `last_error`
- optional `summary`
- `created_at`

Enqueueing the same `idempotency_key` reuses the existing record so external
channel retries do not duplicate execution.

Message statuses:

- `Pending`: queued and ready to be claimed.
- `Processing`: claimed by a worker.
- `Processed`: handled successfully and, when applicable, delivered to outbox.
- `Failed`: processing returned an error.
- `Cancelled`: cancelled by the operator or control plane before delivery.
- `DeadLettered`: processing failed after the worker retry budget was exhausted.

`Processed`, `Failed`, `Cancelled`, and `DeadLettered` are terminal states.
Once a message is terminal, later status or failure writes that are not bound to
a live claim are ignored instead of rewriting the record. A cancellation also
prevents a stale worker claim from delivering a late successful result.

Delivery statuses:

- `Pending`: ready for an adapter to claim, unless `next_attempt_at` is still in
  the future.
- `Processing`: claimed by an adapter.
- `Delivered`: adapter delivery succeeded.
- `DeadLettered`: adapter delivery failed after the retry budget was exhausted.

Delivery claims are lease-bound just like inbox message claims. Failed delivery
attempts clear the lease, record a redacted error, and either set
`next_attempt_at` for retry/backoff or move the delivery to `DeadLettered`.
Stale adapter completions cannot overwrite a delivery that has been reclaimed by
another adapter.

Workers record a redacted lease owner, lease deadline, and attempt count when
they claim a message. Failed processing clears the lease and either returns the
message to `Pending` for retry or moves it to `DeadLettered` when the retry
budget is exhausted. Workers may also reclaim stale processing messages. This
makes local retry possible without treating every adapter retry as a new task.
Stale-processing reclaim is based on `lease_expires_at` for new records and
falls back to timestamps for old records; it is not based on deleting lock files.
Worker completion and failure writes are lease-bound: a worker must write back
with the claim it received. If another worker has already reclaimed the message,
the stale worker update is ignored instead of overwriting the new owner.

## Protocol Types

Gateway's local inbox/outbox frame model lives in `ikaros-gateway`. Stable
cross-surface wire shapes that need to be shared by API, TUI, replay, and future
adapters belong in `ikaros-protocol`.

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
- `GatewayProtocolPolicy`

Frames support `connect`, `request`, `response`, and `event`. `GatewaySessionSource` describes
channel/account/peer/thread/message_id style origins. Frames and routes are redacted before storage.

`GatewayProtocolPolicy` validates frame protocol version, allowed client ids,
allowed channels, and required client capabilities before a daemon or adapter
accepts a frame. This is the local protocol hardening layer used before any
long-running multi-client gateway daemon is exposed.

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

`message send` can attach structured `GatewaySessionSource` fields with
`--account`, `--peer`, `--thread`, and `--message-id`; `--idempotency-key`
deduplicates adapter retries. `message status` summarizes queue counts,
delivery pending/processing/delivered/dead-letter counts, recent
gateway-derived session ids, and a CLI resume hint without printing
secret-looking content. Its worker snapshot also reports
the current `message-worker.lock` owner and stale flag, latest worker forensic event,
processing, stale-processing, retryable, and dead-letter counts, then shows a
small redacted sample of active leases, retryable messages with their last
error, retryable deliveries, and dead-lettered messages/deliveries with
terminal evidence.
`message cancel` moves a pending or processing message to `Cancelled`, clears
its lease, stores the redacted reason, and makes any later worker completion for
the old claim a no-op.
`message delivery claim` is the adapter-facing outbox lease command. Adapters
must later call `message delivery ack` or `message delivery fail` with the same
redacted lease owner; mismatched owners are rejected, and stale adapter updates
cannot overwrite a reclaimed delivery. `fail` records a redacted reason and
either schedules the next retry with `next_attempt_at` or moves the delivery to
`DeadLettered`.
`message adapter list` prints the built-in adapter descriptors and the matching
enqueue/render commands for each platform. `message adapter enqueue` maps a
platform-shaped inbound envelope into the normal local inbox. `message adapter
render-delivery` renders an outbox delivery as a redacted platform-shaped
outbound envelope for an adapter to send. It does not acknowledge delivery and
does not bypass the lease path; adapters still use `message delivery claim`,
`ack`, and `fail` for delivery state.
`message pairing create` creates a one-time code for a source/account/peer. The
code is printed once at creation time. `message pairing list` redacts stored
codes and shows pairing status for local operators.

`message worker` is still the foreground process entrypoint. `message daemon`
is the local daemon control surface for the same worker path: `start` launches
the current `ikaros` binary in the background, waits until `message-worker.lock`
is acquired, and writes stdout/stderr to
`IKAROS_HOME/gateway/message-worker-daemon.log`; `status` reports the lock,
stop request, and latest forensic event; `stop` writes the cooperative stop
request; `restart` waits for the current worker to release its lock before
starting a new one. This is a local long-running worker, not the future
multi-client platform daemon.
`message worker-stop` requests cooperative shutdown by writing
`message-worker.stop`; it does not kill arbitrary host processes.

## Processing

`message drain` and `message worker` process pending inbox records through `ikaros-runtime`.

- `chat` messages use the same governed chat path as `ikaros chat --message`.
- `task` messages use the session-aware task agent-loop path with a
  gateway-derived session id, turn id, and session source, so their typed events
  can share the same `state.db` timeline as gateway request/result/delivery
  evidence.

Successful outputs are written to the local outbox. The gateway does not grant additional
permissions; agent/profile data only affects runtime context and policy overlay.

Processing context:

1. The worker claims a pending inbox record.
2. Runtime resolves the requested agent/profile or falls back to the configured
   default.
3. Chat or task execution builds a normal harness session.
4. Policy, approvals, audit, memory, and provider governance apply exactly as
   they do for direct CLI commands.
5. The inbox record is marked processed, returned to pending for retry, or moved
   to dead-lettered.
6. Successful results create a `GatewayDelivery` in the outbox.
7. The session store receives redacted request/result/delivery evidence.

`chat` and `task` messages derive their durable session id from a versioned
digest of gateway channel, account, peer, and thread. The raw channel/account/
peer/thread values are not embedded in the session id. `message_id` is stored as
redacted source evidence for the individual message; it does not decide the
conversation identity when a structured thread is present. `task` messages write
a gateway-scoped user entry, a runtime result entry, and typed start/end/error
events. Delivery records include the outbox delivery id and kind, not the full
unredacted output.

If a message requires approval, the worker records the pending approval state in
the processing summary; it does not auto-approve or retry with broader
permissions.

## Webhook

The webhook binds to loopback by default and accepts JSON at `POST /message`:

```json
{"content":"hello","kind":"chat","source":"local-webhook","profile":"plan"}
```

Adapter payloads may also include structured routing fields:

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

`source` becomes the gateway channel, `account`/`peer`/`thread`/`message_id`
become the structured `GatewaySessionSource`, and `idempotency_key` deduplicates
adapter retries before execution.

`--allow-source`, `--allow-account`, `--allow-peer`, and `--allow-thread`
provide a local adapter ACL. When any allowlist is configured, the corresponding
payload field must match before the message is enqueued. Rejected requests
return `403 Forbidden` with the rejected field name and do not create inbox
records.

When `--require-pairing` is set, the route's source/account/peer must already
be paired. A request may include `pairing_code` once to bind a pending pairing
created by `message pairing create`; later requests from that same peer do not
need to repeat the code. Unpaired requests return `403 Forbidden` before
enqueueing.

Webhook messages are marked `safe_tools` by default. During runtime drain, chat
and task agent loops for those messages are limited to the `core` toolset, so
remote platform users cannot directly expose workspace, coding, RAG, voice, or
plugin tools through the model. `--unsafe-tools` disables this marker and should
only be used for local, trusted adapters with HMAC, ACL, and pairing controls.

When `--hmac-secret` is set, message requests must include
`X-Ikaros-Signature: sha256=<hex-hmac>`, where the HMAC-SHA256 input is the raw
request body bytes and the key is the configured secret. Missing, malformed, or
invalid signatures return `401 Unauthorized` before payload parsing or queue
writes. The signature is only required for `POST /message`; health checks remain
unsigned so local service managers can probe the process.

The webhook only enqueues a redacted message. It does not call models, tools,
plugins, or task runners.

## Invariants

- Ingestion is non-executing.
- Gateway records are local state under `IKAROS_HOME/gateway`.
- Inbox and outbox mutation must hold the gateway JSONL lock; adapters should
  not edit the JSONL files directly.
- Only one local `message worker` should run for a given `IKAROS_HOME`; the
  process must hold `message-worker.lock` while polling.
- Worker completion/failure updates must be bound to the claimed lease owner and
  attempt count, so stale workers cannot complete a message after reclaim.
- Terminal inbox states (`Processed`, `Failed`, `DeadLettered`) are immutable
  for later unclaimed status/failure writes.
- The protocol lives inside `ikaros-gateway`; adapters should depend on the
  frame shape, not on runtime internals.
- Session source identifies the external conversation; `agent` selects runtime
  context but does not grant permissions.
- Outbox delivery content is redacted before storage.
- Webhook HMAC verification happens before enqueueing; rejected requests do not
  create inbox records.
- Webhook ACL checks happen before enqueueing; rejected source/account/peer/thread
  values are not written to the gateway queue.
- Webhook pairing checks happen before enqueueing when `--require-pairing` is
  enabled; pairing codes are not copied into inbox messages.
- Webhook messages default to `safe_tools`; disabling it is an explicit local
  operator choice.
- Session replay is evidence, not a second queue. Adapters should continue to
  use inbox/outbox for delivery state.
