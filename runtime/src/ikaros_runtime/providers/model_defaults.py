from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ModelCapacityDefaults:
    context_window: int
    max_output_tokens: int


UNKNOWN_MODEL_CAPACITY = ModelCapacityDefaults(32_768, 4_096)
_DEEPSEEK_V4_CAPACITY = ModelCapacityDefaults(1_000_000, 64_000)
_DEEPSEEK_V4_MODELS = frozenset(
    {"deepseek-v4-flash", "deepseek-v4-pro", "deepseek-v4-flash-vision-exp"}
)


def default_model_capacity(model_id: str) -> ModelCapacityDefaults:
    """Initial input window and per-request output allowance, before user overrides.

    DeepSeek publishes a 1M context window and a 384K maximum output capability.
    The 64K allowance is Ikaros's initial output reservation, not DeepSeek's default.
    See runtime/MODEL_CAPACITY_REFERENCES.md for the dated source.
    """

    if model_id in _DEEPSEEK_V4_MODELS:
        return _DEEPSEEK_V4_CAPACITY
    return UNKNOWN_MODEL_CAPACITY
