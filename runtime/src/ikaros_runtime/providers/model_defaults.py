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
_VISION_MODEL_IDS = frozenset({"deepseek-v4-flash-vision-exp"})
_VISION_NAME_MARKERS = frozenset({"vision", "vl", "multimodal"})


def model_supports_vision(model_id: str) -> bool:
    """Whether a model id is known to accept image content parts.

    Unknown ids are treated as text-only so an unsupported model never
    receives an image block; the Runtime then degrades to a path note.
    """

    if model_id in _VISION_MODEL_IDS:
        return True
    tokens = model_id.lower().replace("_", "-").replace(".", "-").split("-")
    return any(token in _VISION_NAME_MARKERS for token in tokens)


def default_model_capacity(model_id: str) -> ModelCapacityDefaults:
    """Initial model context window before discovery or explicit configuration."""

    if model_id in _DEEPSEEK_V4_MODELS:
        return _DEEPSEEK_V4_CAPACITY
    return UNKNOWN_MODEL_CAPACITY
