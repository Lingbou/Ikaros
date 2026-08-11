from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any

from .agent import AgentLoop, AgentScheduler, EventPublisher
from .config import ConfigError, ConfigStore, ModelInput
from .domain import JournalEvent
from .policy import FullAccessPolicy
from .process_tool import ProcessRunTool
from .provider_registry import RuntimeProviderRegistry
from .providers import ScriptedProvider
from .security import (
    json_contains_protected_value,
    response_values_contain_protected_value,
    rpc_request_values_contain_protected_value,
)
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
    def __init__(
        self,
        store: SqliteRuntimeStore,
        publish: EventPublisher,
        *,
        config_store: ConfigStore | None = None,
    ) -> None:
        self._store = store
        self._config = config_store or ConfigStore(store.database_path.parent)
        if self._store.journal_contains_protected_values(self._config.protected_values()):
            raise ConfigError("configured credentials conflict with persisted Runtime data")
        self._providers = RuntimeProviderRegistry(self._config)
        tools = ToolExecutor(ToolRegistry([ProcessRunTool()]), FullAccessPolicy())
        loop = AgentLoop(
            store,
            self._providers,
            publish,
            tools,
            protected_values=self._config.protected_values,
        )
        self._scheduler = AgentScheduler(loop)
        self._publish = publish

    def start(self, recovered_run_ids: Sequence[str] = ()) -> None:
        self._scheduler.start(recovered_run_ids)

    async def close(self) -> None:
        await self._scheduler.close()
        await self._providers.close()

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
            raise InvalidParamsError("thread.create contains unsupported parameters")
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
        self._assert_request_values_safe((title, client_request_id))
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
            raise InvalidParamsError("turn.start fields do not match the required schema")
        values = {name: params[name] for name in required}
        if any(not isinstance(value, str) for value in values.values()):
            raise InvalidParamsError("turn.start fields must be strings")
        content = values["content"].strip()
        if not content:
            raise InvalidParamsError("content must not be empty")
        client_request_id = _client_request_id(params)
        self._assert_request_values_safe((*values.values(), content, client_request_id))
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
        else:
            try:
                self._config.resolve_model(values["providerId"], values["modelId"])
            except ConfigError as error:
                raise InvalidParamsError(str(error)) from None
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

    def list_providers(self, params: dict[str, Any]) -> dict[str, Any]:
        if params:
            raise InvalidParamsError("provider.list does not accept parameters")
        return {"providers": [summary.to_wire() for summary in self._config.provider_summaries()]}

    def list_models(self, params: dict[str, Any]) -> dict[str, Any]:
        if params:
            raise InvalidParamsError("model.list does not accept parameters")
        return {"models": [summary.to_wire() for summary in self._config.model_summaries()]}

    def configure_provider(self, params: dict[str, Any]) -> dict[str, Any]:
        kind = params.get("kind")
        if kind == "deepseek":
            if set(params) != {"kind", "apiKey", "models"}:
                raise InvalidParamsError(
                    "DeepSeek provider.configure requires kind, apiKey, and models"
                )
            self._assert_provider_mutable("deepseek")
            api_key = params["apiKey"]
            if not isinstance(api_key, str):
                raise InvalidParamsError("apiKey must be a string")
            models = _model_inputs(params["models"])
            self._assert_credentials_not_in_journal((api_key,))
            try:
                summary = self._config.configure_deepseek(api_key=api_key, models=models)
            except ConfigError as error:
                raise InvalidParamsError(str(error)) from None
            self._providers.configuration_changed("deepseek")
            return {"provider": summary.to_wire()}
        if kind == "custom":
            required = {"kind", "providerId", "displayName", "baseUrl", "models"}
            allowed = required | {"apiKey", "headers"}
            if not required <= set(params) or not set(params) <= allowed:
                raise InvalidParamsError(
                    "Custom provider.configure requires kind, providerId, displayName, "
                    "baseUrl, and models"
                )
            provider_id = params["providerId"]
            display_name = params["displayName"]
            base_url = params["baseUrl"]
            if not all(isinstance(value, str) for value in (provider_id, display_name, base_url)):
                raise InvalidParamsError("Custom provider string fields must be strings")
            normalized_provider_id = provider_id.strip().lower()
            self._assert_provider_mutable(normalized_provider_id)
            api_key = params.get("apiKey")
            if api_key is not None and not isinstance(api_key, str):
                raise InvalidParamsError("apiKey must be a string or null")
            headers = params.get("headers")
            if headers is not None and not isinstance(headers, dict):
                raise InvalidParamsError("headers must be an object or null")
            models = _model_inputs(params["models"])
            proposed_credentials = [api_key]
            if headers is not None:
                proposed_credentials.extend(
                    value for value in headers.values() if isinstance(value, str)
                )
            self._assert_credentials_not_in_journal(proposed_credentials)
            try:
                summary = self._config.configure_custom(
                    provider_id=provider_id,
                    display_name=display_name,
                    base_url=base_url,
                    api_key=api_key,
                    headers=headers,
                    models=models,
                )
            except ConfigError as error:
                raise InvalidParamsError(str(error)) from None
            self._providers.configuration_changed(summary.id)
            return {"provider": summary.to_wire()}
        raise InvalidParamsError("provider.configure kind must be deepseek or custom")

    def disconnect_provider(self, params: dict[str, Any]) -> dict[str, Any]:
        if set(params) != {"providerId"} or params.get("providerId") != "deepseek":
            raise InvalidParamsError("provider.disconnect is only available for DeepSeek")
        self._assert_provider_mutable("deepseek")
        try:
            summary = self._config.disconnect_deepseek()
        except ConfigError as error:
            raise InvalidParamsError(str(error)) from None
        self._providers.configuration_changed("deepseek")
        return {"provider": summary.to_wire()}

    def remove_provider(self, params: dict[str, Any]) -> dict[str, Any]:
        if set(params) != {"providerId"}:
            raise InvalidParamsError("provider.remove requires exactly one providerId")
        provider_id = params["providerId"]
        if not isinstance(provider_id, str):
            raise InvalidParamsError("providerId must be a string")
        normalized_provider_id = provider_id.strip().lower()
        self._assert_provider_mutable(normalized_provider_id)
        try:
            self._config.remove_custom(provider_id)
        except ConfigError as error:
            raise InvalidParamsError(str(error)) from None
        self._providers.configuration_changed(normalized_provider_id)
        return {"removed": True, "providerId": normalized_provider_id}

    def set_model_enabled(self, params: dict[str, Any]) -> dict[str, Any]:
        if set(params) != {"providerId", "modelId", "enabled"}:
            raise InvalidParamsError("model.set_enabled requires providerId, modelId, and enabled")
        provider_id = params["providerId"]
        model_id = params["modelId"]
        enabled = params["enabled"]
        if not isinstance(provider_id, str) or not isinstance(model_id, str):
            raise InvalidParamsError("providerId and modelId must be strings")
        if not isinstance(enabled, bool):
            raise InvalidParamsError("enabled must be a boolean")
        self._assert_provider_mutable(provider_id)
        try:
            summary = self._config.set_model_enabled(provider_id, model_id, enabled)
        except ConfigError as error:
            raise InvalidParamsError(str(error)) from None
        self._providers.configuration_changed(provider_id)
        return {"model": summary.to_wire()}

    def _assert_provider_mutable(self, provider_id: str) -> None:
        del provider_id
        if self._store.has_active_runs():
            raise InvalidParamsError("provider configuration cannot change while a Run is active")

    def _assert_request_values_safe(self, values: object) -> None:
        protected_values = self._config.protected_values()
        if protected_values and json_contains_protected_value(values, protected_values):
            raise InvalidParamsError("request contains protected configuration data")

    def rpc_request_values_contain_protected_value(
        self,
        value: object,
        additional_values: Sequence[str] = (),
    ) -> bool:
        protected_values = (*self._config.protected_values(), *additional_values)
        return bool(protected_values) and rpc_request_values_contain_protected_value(
            value,
            protected_values,
        )

    def protected_values(self) -> tuple[str, ...]:
        return self._config.protected_values()

    def response_contains_protected_value(
        self,
        value: object,
        additional_values: Sequence[str] = (),
    ) -> bool:
        protected_values = (*self._config.protected_values(), *additional_values)
        return bool(protected_values) and response_values_contain_protected_value(
            value,
            protected_values,
        )

    def _assert_credentials_not_in_journal(self, values: Sequence[object]) -> None:
        protected_values = tuple(value for value in values if isinstance(value, str) and value)
        if self._store.journal_contains_protected_values(protected_values):
            raise InvalidParamsError("credentials conflict with persisted Runtime data")

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


def _model_inputs(value: Any) -> tuple[ModelInput, ...]:
    if not isinstance(value, list):
        raise InvalidParamsError("models must be an array")
    models: list[ModelInput] = []
    for item in value:
        if not isinstance(item, dict) or set(item) != {"id", "displayName"}:
            raise InvalidParamsError("each model requires exactly id and displayName")
        model_id = item["id"]
        display_name = item["displayName"]
        if not isinstance(model_id, str) or not isinstance(display_name, str):
            raise InvalidParamsError("model id and displayName must be strings")
        models.append(ModelInput(model_id, display_name))
    return tuple(models)
