from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ModelCapacityDefaults:
    context_window: int


UNKNOWN_MODEL_CAPACITY = ModelCapacityDefaults(32_768)
_DEEPSEEK_V4_CAPACITY = ModelCapacityDefaults(1_000_000)
_DEEPSEEK_V4_MODELS = frozenset(
    {"deepseek-v4-flash", "deepseek-v4-pro", "deepseek-v4-flash-vision-exp"}
)


def default_model_capacity(model_id: str) -> ModelCapacityDefaults:
    """Initial model context window before discovery or explicit configuration."""

    if model_id in _DEEPSEEK_V4_MODELS:
        return _DEEPSEEK_V4_CAPACITY
    return UNKNOWN_MODEL_CAPACITY
