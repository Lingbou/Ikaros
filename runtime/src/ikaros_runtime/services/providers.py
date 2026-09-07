from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from typing import Any

from ..errors import ConfigError, InvalidParamsError
from ..providers.registry import (
    ConfigStore,
    ModelInput,
    ProviderConfig,
    RuntimeProviderRegistry,
    deepseek_discovery_provider,
)
from ..storage import SqliteRuntimeStore

ModelDiscovery = Callable[[ProviderConfig], Awaitable[tuple[ModelInput, ...]]]
CredentialSafetyCheck = Callable[[Sequence[object]], None]


class ProviderService:
    """Provider and model configuration use cases exposed by Runtime RPCs."""

    def __init__(
        self,
        config_store: ConfigStore,
        provider_registry: RuntimeProviderRegistry,
        store: SqliteRuntimeStore,
        model_discovery: ModelDiscovery,
        assert_credentials_safe: CredentialSafetyCheck,
    ) -> None:
        self._config = config_store
        self._providers = provider_registry
        self._store = store
        self._model_discovery = model_discovery
        self._assert_credentials_safe = assert_credentials_safe

    def list_providers(self, params: dict[str, Any]) -> dict[str, Any]:
        if params:
            raise InvalidParamsError("provider.list does not accept parameters")
        return {"providers": [summary.to_wire() for summary in self._config.provider_summaries()]}

    def list_models(self, params: dict[str, Any]) -> dict[str, Any]:
        if params:
            raise InvalidParamsError("model.list does not accept parameters")
        return {"models": [summary.to_wire() for summary in self._config.model_summaries()]}

    async def discover_provider_models(self, params: dict[str, Any]) -> dict[str, Any]:
        if set(params) != {"kind", "apiKey"} or params.get("kind") != "deepseek":
            raise InvalidParamsError(
                "DeepSeek provider.discover_models requires exactly kind and apiKey"
            )
        api_key = params["apiKey"]
        if not isinstance(api_key, str):
            raise InvalidParamsError("apiKey must be a string")
        self._assert_credentials_safe((api_key,))
        try:
            provider = deepseek_discovery_provider(api_key)
        except ConfigError as error:
            raise InvalidParamsError(str(error)) from None
        models = await self._model_discovery(provider)
        existing_provider = self._config.get_provider("deepseek")
        existing_models = (
            {model.id: model for model in existing_provider.models}
            if existing_provider is not None
            else {}
        )
        return {
            "models": [
                {
                    "id": model.id,
                    "displayName": model.display_name,
                    "contextWindow": existing_models.get(model.id, model).context_window,
                    "maxOutputTokens": existing_models.get(model.id, model).max_output_tokens,
                }
                for model in models
            ]
        }

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
            self._assert_credentials_safe((api_key,))
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
            self._assert_credentials_safe(proposed_credentials)
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

    def set_model_limits(self, params: dict[str, Any]) -> dict[str, Any]:
        if set(params) != {"providerId", "modelId", "contextWindow", "maxOutputTokens"}:
            raise InvalidParamsError(
                "model.set_limits requires providerId, modelId, contextWindow, maxOutputTokens"
            )
        provider_id, model_id = params["providerId"], params["modelId"]
        if not isinstance(provider_id, str) or not isinstance(model_id, str):
            raise InvalidParamsError("providerId and modelId must be strings")
        try:
            model = self._config.set_model_limits(
                provider_id, model_id, params["contextWindow"], params["maxOutputTokens"]
            )
        except ConfigError as error:
            raise InvalidParamsError(str(error)) from None
        # Capacity settings affect future requests; keep an in-flight stream alive.
        return {"model": model.to_wire()}

    def _assert_provider_mutable(self, provider_id: str) -> None:
        del provider_id
        if self._store.has_active_runs():
            raise InvalidParamsError("provider configuration cannot change while a Run is active")


def _model_inputs(value: Any) -> tuple[ModelInput, ...]:
    if not isinstance(value, list):
        raise InvalidParamsError("models must be an array")
    models: list[ModelInput] = []
    for item in value:
        if not isinstance(item, dict) or set(item) != {
            "id",
            "displayName",
            "contextWindow",
            "maxOutputTokens",
        }:
            raise InvalidParamsError(
                "each model requires id, displayName, contextWindow, and maxOutputTokens"
            )
        model_id = item["id"]
        display_name = item["displayName"]
        if not isinstance(model_id, str) or not isinstance(display_name, str):
            raise InvalidParamsError("model id and displayName must be strings")
        context_window = item["contextWindow"]
        max_output_tokens = item["maxOutputTokens"]
        if (
            not isinstance(context_window, int)
            or isinstance(context_window, bool)
            or not isinstance(max_output_tokens, int)
            or isinstance(max_output_tokens, bool)
            or not 0 < max_output_tokens < context_window <= 9007199254740991
        ):
            raise InvalidParamsError(
                "model token limits must be positive integers with output below context window"
            )
        models.append(ModelInput(model_id, display_name, context_window, max_output_tokens))
    return tuple(models)
