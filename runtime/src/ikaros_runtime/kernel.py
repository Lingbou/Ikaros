from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from .agent import AgentLoop, AgentScheduler, EventPublisher
from .domain import JournalEvent
from .providers import ScriptedProvider
from .storage import SqliteRuntimeStore


class InvalidParamsError(ValueError):
    """The client supplied invalid JSON-RPC method parameters."""


@dataclass(frozen=True, slots=True)
class CommandOutcome:
    result: dict[str, Any]
    events_after_ack: tuple[JournalEvent, ...] = ()
    run_after_ack: str | None = None


class RuntimeKernel:
    def __init__(self, store: SqliteRuntimeStore, publish: EventPublisher) -> None:
        self._store = store
        scripted = ScriptedProvider()
        loop = AgentLoop(store, {scripted.id: scripted}, publish)
        self._scheduler = AgentScheduler(loop)
        self._publish = publish

    def start(self) -> None:
        self._scheduler.start()

    async def close(self) -> None:
        await self._scheduler.close()

    async def finish_command(self, outcome: CommandOutcome) -> None:
        for event in outcome.events_after_ack:
            await self._publish(event)
        if outcome.run_after_ack is not None:
            await self._scheduler.enqueue(outcome.run_after_ack)

    def create_thread(self, params: dict[str, Any]) -> CommandOutcome:
        unknown = set(params) - {"title"}
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
        thread, event = self._store.create_thread(title)
        return CommandOutcome(
            result={"thread": thread.to_wire(), "event": event.to_wire()},
            events_after_ack=(event,),
        )

    def start_turn(self, params: dict[str, Any]) -> CommandOutcome:
        required = {"threadId", "branchId", "content", "providerId", "modelId"}
        if set(params) != required:
            missing = sorted(required - set(params))
            unknown = sorted(set(params) - required)
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
        try:
            prepared = self._store.prepare_turn(
                thread_id=values["threadId"],
                branch_id=values["branchId"],
                content=content,
                provider_id=values["providerId"],
                model_id=values["modelId"],
            )
        except LookupError as error:
            raise InvalidParamsError(str(error)) from error
        return CommandOutcome(
            result={
                "turnId": prepared.turn_id,
                "runId": prepared.run_id,
                "threadId": prepared.thread_id,
                "branchId": prepared.branch_id,
            },
            events_after_ack=prepared.initial_events,
            run_after_ack=prepared.run_id,
        )

    def list_threads(self, params: dict[str, Any]) -> dict[str, Any]:
        if params:
            raise InvalidParamsError("thread.list does not accept parameters")
        return {"threads": [thread.to_wire() for thread in self._store.list_threads()]}

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
