from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any

from .agent import AgentLoop, AgentScheduler, EventPublisher
from .domain import JournalEvent
from .policy import FullAccessPolicy
from .process_tool import ProcessRunTool
from .providers import ScriptedProvider
from .storage import SqliteRuntimeStore
from .tools import ToolExecutor, ToolRegistry


class InvalidParamsError(ValueError):
    """The client supplied invalid JSON-RPC method parameters."""


def _client_request_id(params: dict[str, Any]) -> str | None:
    value = params.get("clientRequestId")
    if value is None:
        return None
    if not isinstance(value, str) or not value or len(value) > 200:
        raise InvalidParamsError(
            "clientRequestId must be a non-empty string of at most 200 characters"
        )
    return value


@dataclass(frozen=True, slots=True)
class CommandOutcome:
    result: dict[str, Any]
    events_after_ack: tuple[JournalEvent, ...] = ()
    run_after_ack: str | None = None
    cancel_after_ack: str | None = None


class RuntimeKernel:
    def __init__(self, store: SqliteRuntimeStore, publish: EventPublisher) -> None:
        self._store = store
        scripted = ScriptedProvider()
        tools = ToolExecutor(ToolRegistry([ProcessRunTool()]), FullAccessPolicy())
        loop = AgentLoop(store, {scripted.id: scripted}, publish, tools)
        self._scheduler = AgentScheduler(loop)
        self._publish = publish

    def start(self, recovered_run_ids: Sequence[str] = ()) -> None:
        self._scheduler.start(recovered_run_ids)

    async def close(self) -> None:
        await self._scheduler.close()

    async def finish_command(self, outcome: CommandOutcome) -> None:
        for event in outcome.events_after_ack:
            await self._publish(event)
        if outcome.run_after_ack is not None:
            await self._scheduler.activate(outcome.run_after_ack)
        if outcome.cancel_after_ack is not None:
            await self._scheduler.cancel(outcome.cancel_after_ack)

    def create_thread(self, params: dict[str, Any]) -> CommandOutcome:
        unknown = set(params) - {"title", "clientRequestId"}
        if unknown:
            raise InvalidParamsError(f"unknown thread.create parameters: {sorted(unknown)}")
        title = params.get("title")
        if title is not None:
            if not isinstance(title, str):
                raise InvalidParamsError("title must be a string or null")
            title = title.strip()
            if not title:
                title = None
            elif len(title) > 200:
                raise InvalidParamsError("title must not exceed 200 characters")
        client_request_id = _client_request_id(params)
        if client_request_id is None:
            thread, event = self._store.create_thread(title)
            created = True
        else:
            try:
                thread, event, created = self._store.create_thread_once(
                    title,
                    client_request_id,
                )
            except LookupError as error:
                raise InvalidParamsError(str(error)) from error
        return CommandOutcome(
            result={"thread": thread.to_wire(), "event": event.to_wire()},
            events_after_ack=(event,) if created else (),
        )

    def start_turn(self, params: dict[str, Any]) -> CommandOutcome:
        required = {"threadId", "branchId", "content", "providerId", "modelId"}
        allowed = required | {"clientRequestId"}
        if not required <= set(params) or not set(params) <= allowed:
            missing = sorted(required - set(params))
            unknown = sorted(set(params) - allowed)
            raise InvalidParamsError(
                f"turn.start fields mismatch; missing={missing}, unknown={unknown}"
            )
        values = {name: params[name] for name in required}
        if any(not isinstance(value, str) for value in values.values()):
            raise InvalidParamsError("turn.start fields must be strings")
        content = values["content"].strip()
        if not content:
            raise InvalidParamsError("content must not be empty")
        if values["providerId"] != ScriptedProvider.id:
            raise InvalidParamsError("provider is not available")
        if values["modelId"] != ScriptedProvider.model_id:
            raise InvalidParamsError("model is not available")
        client_request_id = _client_request_id(params)
        try:
            prepared = self._store.prepare_turn(
                thread_id=values["threadId"],
                branch_id=values["branchId"],
                content=content,
                provider_id=values["providerId"],
                model_id=values["modelId"],
                client_request_id=client_request_id,
            )
        except LookupError as error:
            raise InvalidParamsError(str(error)) from error
        if prepared.newly_created:
            self._scheduler.reserve(prepared.run_id)
        return CommandOutcome(
            result={
                "turnId": prepared.turn_id,
                "runId": prepared.run_id,
                "threadId": prepared.thread_id,
                "branchId": prepared.branch_id,
            },
            events_after_ack=prepared.initial_events if prepared.newly_created else (),
            run_after_ack=prepared.run_id if prepared.newly_created else None,
        )

    def list_threads(self, params: dict[str, Any]) -> dict[str, Any]:
        if params:
            raise InvalidParamsError("thread.list does not accept parameters")
        return {"threads": [thread.to_wire() for thread in self._store.list_threads()]}

    def cancel_run(self, params: dict[str, Any]) -> CommandOutcome:
        if set(params) != {"runId"}:
            raise InvalidParamsError("run.cancel requires exactly one runId")
        run_id = params["runId"]
        if not isinstance(run_id, str) or not run_id:
            raise InvalidParamsError("runId must be a non-empty string")
        try:
            status = self._store.run_status(run_id)
        except LookupError as error:
            raise InvalidParamsError(str(error)) from error
        if status in {"completed", "failed", "cancelled"}:
            return CommandOutcome(result={"accepted": False, "runId": run_id, "status": status})
        return CommandOutcome(
            result={"accepted": True, "runId": run_id, "status": status},
            cancel_after_ack=run_id,
        )

    def replay_events(self, params: dict[str, Any]) -> dict[str, Any]:
        unknown = set(params) - {"afterSeq", "limit"}
        if unknown:
            raise InvalidParamsError(f"unknown event.replay parameters: {sorted(unknown)}")
        after_seq = params.get("afterSeq", 0)
        limit = params.get("limit", 500)
        if not isinstance(after_seq, int) or isinstance(after_seq, bool) or after_seq < 0:
            raise InvalidParamsError("afterSeq must be a non-negative integer")
        if not isinstance(limit, int) or isinstance(limit, bool) or not 1 <= limit <= 1000:
            raise InvalidParamsError("limit must be an integer between 1 and 1000")
        events, latest_seq = self._store.replay_events(after_seq, limit)
        next_after_seq = events[-1].seq if events else after_seq
        return {
            "events": [event.to_wire() for event in events],
            "latestSeq": latest_seq,
            "nextAfterSeq": next_after_seq,
            "hasMore": next_after_seq < latest_seq,
        }
