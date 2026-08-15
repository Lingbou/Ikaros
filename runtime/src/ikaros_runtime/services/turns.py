from __future__ import annotations

from collections.abc import Callable
from typing import Any

from ..agent.scheduler import AgentScheduler
from ..domain import CommandOutcome, SkillDescriptor
from ..errors import ConfigError, InvalidParamsError
from ..providers.registry import ConfigStore
from ..providers.scripted import ScriptedProvider
from ..run_input import ProviderExecutionSnapshotV1, SubmissionFrameTemplateV1
from ..storage import SqliteRuntimeStore
from ..storage.thread_history import TURN_HISTORY_DEFAULT_LIMIT, TURN_HISTORY_MAX_LIMIT
from ..tools.core import ToolDefinition
from .threads import RequestSafetyCheck, client_request_id_from, record_id_from

type SkillSnapshotSource = Callable[[], tuple[SkillDescriptor, ...]]
type ToolDefinitionSource = Callable[[], tuple[ToolDefinition, ...]]


class TurnService:
    def __init__(
        self,
        store: SqliteRuntimeStore,
        scheduler: AgentScheduler,
        config_store: ConfigStore,
        assert_request_safe: RequestSafetyCheck,
        skill_snapshot: SkillSnapshotSource | None = None,
        tool_definitions: ToolDefinitionSource | None = None,
        execution_policy: str = "full_access",
        max_steps: int = 16,
    ) -> None:
        self._store = store
        self._scheduler = scheduler
        self._config = config_store
        self._assert_request_safe = assert_request_safe
        self._skill_snapshot = skill_snapshot or _empty_skill_snapshot
        self._tool_definitions = tool_definitions or _empty_tool_definitions
        self._execution_policy = execution_policy
        self._max_steps = max_steps

    def start_turn(self, params: dict[str, Any]) -> CommandOutcome:
        required = {"threadId", "branchId", "content", "providerId", "modelId"}
        allowed = required | {"clientRequestId"}
        if not required <= set(params) or not set(params) <= allowed:
            raise InvalidParamsError("turn.start fields do not match the required schema")
        values = {name: params[name] for name in required}
        if any(not isinstance(value, str) for value in values.values()):
            raise InvalidParamsError("turn.start fields must be strings")
        content = values["content"]
        if not content.strip():
            raise InvalidParamsError("content must not be empty")
        client_request_id = client_request_id_from(params)
        self._assert_request_safe((*values.values(), content, client_request_id))
        if client_request_id is not None:
            try:
                existing = self._store.find_turn_by_client_request_id(
                    thread_id=values["threadId"],
                    branch_id=values["branchId"],
                    content=content,
                    provider_id=values["providerId"],
                    model_id=values["modelId"],
                    client_request_id=client_request_id,
                )
            except LookupError as error:
                raise InvalidParamsError(str(error)) from error
            if existing is not None:
                return CommandOutcome(
                    result={
                        "turnId": existing.turn_id,
                        "runId": existing.run_id,
                        "threadId": existing.thread_id,
                        "branchId": existing.branch_id,
                    }
                )
        if values["providerId"] == ScriptedProvider.id:
            if values["modelId"] != ScriptedProvider.model_id:
                raise InvalidParamsError("model is not available")
            provider_snapshot = ProviderExecutionSnapshotV1(
                provider_id=ScriptedProvider.id,
                origin="scripted",
                base_url=None,
                model_id=ScriptedProvider.model_id,
                supports_tools=True,
            )
        else:
            try:
                provider_snapshot = self._config.execution_snapshot(
                    values["providerId"],
                    values["modelId"],
                )
            except ConfigError as error:
                raise InvalidParamsError(str(error)) from None
        skills = self._skill_snapshot()
        frame_template = SubmissionFrameTemplateV1.create(
            provider=provider_snapshot,
            execution_policy=self._execution_policy,
            skills=skills,
            tools=self._tool_definitions(),
            max_steps=self._max_steps,
        )
        try:
            prepared = self._store.prepare_turn(
                thread_id=values["threadId"],
                branch_id=values["branchId"],
                content=content,
                frame_template=frame_template,
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
            raise InvalidParamsError("event.replay contains unsupported parameters")
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

    def list(self, params: dict[str, Any]) -> dict[str, Any]:
        required = {"threadId", "branchId"}
        allowed = required | {"cursor", "limit"}
        if not required <= set(params) or not set(params) <= allowed:
            raise InvalidParamsError("turn.list fields do not match the required schema")
        thread_id = record_id_from(params["threadId"], name="threadId")
        branch_id = record_id_from(params["branchId"], name="branchId")
        cursor: str | None = None
        if "cursor" in params:
            raw_cursor = params["cursor"]
            if not isinstance(raw_cursor, str) or not raw_cursor:
                raise InvalidParamsError("turn.list cursor must be a non-empty string")
            cursor = raw_cursor
        limit = params.get("limit", TURN_HISTORY_DEFAULT_LIMIT)
        if isinstance(limit, bool) or not isinstance(limit, int):
            raise InvalidParamsError("turn.list limit must be an integer")
        if limit < 1 or limit > TURN_HISTORY_MAX_LIMIT:
            raise InvalidParamsError(
                f"turn.list limit must be between 1 and {TURN_HISTORY_MAX_LIMIT}"
            )
        self._assert_request_safe((thread_id, branch_id, cursor))
        try:
            return self._store.list_turn_page(
                thread_id=thread_id,
                branch_id=branch_id,
                cursor=cursor,
                limit=limit,
            ).to_wire()
        except (LookupError, ValueError) as error:
            raise InvalidParamsError(str(error)) from error


def _empty_skill_snapshot() -> tuple[SkillDescriptor, ...]:
    return ()


def _empty_tool_definitions() -> tuple[ToolDefinition, ...]:
    return ()


__all__ = ["TurnService"]
