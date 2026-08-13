"""OpenAI-compatible Provider implementation."""

from ...errors import ProviderFailure, ProviderFailureCategory
from .adapter import OpenAICompatibleAdapter, ProviderTimeouts
from .discovery import discover_openai_compatible_models

__all__ = [
    "OpenAICompatibleAdapter",
    "ProviderFailure",
    "ProviderFailureCategory",
    "ProviderTimeouts",
    "discover_openai_compatible_models",
]
