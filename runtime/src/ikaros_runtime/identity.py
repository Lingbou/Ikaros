"""Load the immutable Ikaros identity shipped with the Runtime release."""

from __future__ import annotations

from importlib.resources import files as resource_files

from .run_input import (
    IDENTITY_CORE_MAX_CHARACTERS_V1,
    IKAROS_IDENTITY_ID,
    IKAROS_IDENTITY_SOURCE,
    IKAROS_IDENTITY_VERSION,
    InstructionBlockV1,
)

_RESOURCE_PACKAGE = "ikaros_runtime.resources"
_RESOURCE_NAME = "IKAROS.md"


class IdentityResourceError(RuntimeError):
    """The packaged Runtime identity is missing or violates its release contract."""


def load_identity_core() -> InstructionBlockV1:
    """Return the canonical, release-scoped Ikaros identity instruction."""

    try:
        raw = resource_files(_RESOURCE_PACKAGE).joinpath(_RESOURCE_NAME).read_bytes()
    except (ModuleNotFoundError, OSError) as error:
        raise IdentityResourceError("Ikaros identity resource is unavailable") from error

    try:
        decoded = raw.decode("utf-8", errors="strict")
    except UnicodeDecodeError as error:
        raise IdentityResourceError("Ikaros identity resource is not valid UTF-8") from error

    if decoded.startswith("\ufeff"):
        raise IdentityResourceError("Ikaros identity resource must not contain a UTF-8 BOM")

    content = decoded.replace("\r\n", "\n")
    if "\r" in content:
        raise IdentityResourceError("Ikaros identity resource contains invalid line endings")
    content = content.removesuffix("\n")

    if not content.strip():
        raise IdentityResourceError("Ikaros identity resource must not be blank")
    if len(content) > IDENTITY_CORE_MAX_CHARACTERS_V1:
        raise IdentityResourceError("Ikaros identity resource exceeds its character limit")

    return InstructionBlockV1(
        id=IKAROS_IDENTITY_ID,
        version=IKAROS_IDENTITY_VERSION,
        source=IKAROS_IDENTITY_SOURCE,
        authority="runtime_identity",
        scope="global",
        lifetime="release",
        content=content,
    )


__all__ = ["IdentityResourceError", "load_identity_core"]
