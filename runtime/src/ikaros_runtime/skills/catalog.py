"""Safe one-level discovery for ``~/.ikaros/skills/*/SKILL.md``."""

from __future__ import annotations

import os
import stat
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

from ..domain import SkillDescriptor, is_valid_skill_description, is_valid_skill_name

MAX_SKILL_BYTES = 256 * 1024
MAX_FRONTMATTER_BYTES = 16 * 1024

type ProtectedValuesSource = Callable[[], Sequence[str]]


class _UniqueKeyLoader(yaml.SafeLoader):
    """Safe YAML loader that refuses silent mapping-key replacement."""


def _construct_unique_mapping(
    loader: _UniqueKeyLoader,
    node: yaml.nodes.MappingNode,
    deep: bool = False,
) -> dict[object, object]:
    loader.flatten_mapping(node)
    mapping: dict[object, object] = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        try:
            duplicate = key in mapping
        except TypeError as error:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                "found an unhashable key",
                key_node.start_mark,
            ) from error
        if duplicate:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                "found a duplicate key",
                key_node.start_mark,
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


_UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    _construct_unique_mapping,
)


@dataclass(frozen=True, slots=True)
class SkillDiagnostic:
    entry: str
    code: str
    message: str

    def to_wire(self) -> dict[str, str]:
        return {"entry": self.entry, "code": self.code, "message": self.message}


@dataclass(frozen=True, slots=True)
class SkillDiscovery:
    skills: tuple[SkillDescriptor, ...]
    diagnostics: tuple[SkillDiagnostic, ...]


class SkillCatalog:
    """Discover Skills without importing or executing any Skill-owned code."""

    def __init__(
        self,
        root: Path,
        protected_values: ProtectedValuesSource | None = None,
    ) -> None:
        self._root = root
        self._protected_values = protected_values or _no_protected_values

    @property
    def root(self) -> Path:
        return self._root

    def discover(self) -> SkillDiscovery:
        protected = tuple(dict.fromkeys(value for value in self._protected_values() if value))
        if not self._root.exists():
            return SkillDiscovery((), ())
        if _is_link_like(self._root) or not _is_directory(self._root):
            return SkillDiscovery((), (_diagnostic("<skills>", "unsafe_root"),))
        try:
            root = self._root.resolve(strict=True)
            entries = sorted(self._root.iterdir(), key=lambda value: value.name)
        except OSError:
            return SkillDiscovery((), (_diagnostic("<skills>", "catalog_unavailable"),))

        skills: list[SkillDescriptor] = []
        diagnostics: list[SkillDiagnostic] = []
        for entry in entries:
            entry_name = _public_entry(entry.name, protected)
            if _is_link_like(entry):
                diagnostics.append(_diagnostic(entry_name, "unsafe_path"))
                continue
            if not _is_directory(entry):
                continue
            if not is_valid_skill_name(entry.name):
                diagnostics.append(_diagnostic(entry_name, "invalid_name"))
                continue
            skill_file = entry / "SKILL.md"
            if not skill_file.exists():
                diagnostics.append(_diagnostic(entry_name, "missing_file"))
                continue
            if _is_link_like(skill_file) or not _is_regular_file(skill_file):
                diagnostics.append(_diagnostic(entry_name, "unsafe_path"))
                continue
            try:
                resolved = skill_file.resolve(strict=True)
                if os.path.commonpath((str(root), str(resolved))) != str(root):
                    diagnostics.append(_diagnostic(entry_name, "unsafe_path"))
                    continue
                source = _read_bounded(skill_file)
                descriptor = _parse_descriptor(entry.name, resolved, source)
            except _SkillReadError as error:
                diagnostics.append(_diagnostic(entry_name, error.code))
                continue
            except OSError:
                diagnostics.append(_diagnostic(entry_name, "read_failed"))
                continue
            if _descriptor_is_protected(descriptor, protected):
                diagnostics.append(_diagnostic("<redacted>", "protected_value"))
                continue
            skills.append(descriptor)
        return SkillDiscovery(tuple(skills), tuple(diagnostics))


class _SkillReadError(Exception):
    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


def _read_bounded(path: Path) -> bytes:
    try:
        with path.open("rb") as handle:
            source = handle.read(MAX_SKILL_BYTES + 1)
    except OSError:
        raise
    if len(source) > MAX_SKILL_BYTES:
        raise _SkillReadError("file_too_large")
    return source


def _parse_descriptor(directory_name: str, location: Path, source: bytes) -> SkillDescriptor:
    try:
        text = source.decode("utf-8")
    except UnicodeError:
        raise _SkillReadError("invalid_utf8") from None
    lines = text.splitlines()
    if not lines or lines[0] != "---":
        raise _SkillReadError("invalid_frontmatter")
    closing = next((index for index in range(1, len(lines)) if lines[index] == "---"), None)
    if closing is None:
        raise _SkillReadError("invalid_frontmatter")
    frontmatter = "\n".join(lines[1:closing])
    if len(frontmatter.encode("utf-8")) > MAX_FRONTMATTER_BYTES:
        raise _SkillReadError("frontmatter_too_large")
    try:
        metadata: Any = yaml.load(frontmatter, Loader=_UniqueKeyLoader)
    except yaml.YAMLError:
        raise _SkillReadError("invalid_frontmatter") from None
    if not isinstance(metadata, Mapping):
        raise _SkillReadError("invalid_frontmatter")
    name = metadata.get("name")
    description = metadata.get("description")
    if name != directory_name or not is_valid_skill_name(name):
        raise _SkillReadError("name_mismatch")
    if not is_valid_skill_description(description):
        raise _SkillReadError("invalid_description")
    assert isinstance(name, str)
    assert isinstance(description, str)
    return SkillDescriptor(name=name, description=description, location=str(location))


def _is_link_like(path: Path) -> bool:
    try:
        if path.is_symlink() or path.is_junction():
            return True
        attributes = getattr(path.lstat(), "st_file_attributes", 0)
        return bool(attributes & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0))
    except OSError:
        return True


def _is_directory(path: Path) -> bool:
    try:
        return path.is_dir()
    except OSError:
        return False


def _is_regular_file(path: Path) -> bool:
    try:
        return path.is_file()
    except OSError:
        return False


def _descriptor_is_protected(
    descriptor: SkillDescriptor,
    protected_values: Sequence[str],
) -> bool:
    return any(
        _matches_protected(value, protected_values)
        for value in (descriptor.name, descriptor.description, descriptor.location)
    )


def _public_entry(value: str, protected_values: Sequence[str]) -> str:
    return "<redacted>" if _matches_protected(value, protected_values) else value


def _matches_protected(value: str, protected_values: Sequence[str]) -> bool:
    return any(
        value == protected or (len(protected) >= 8 and protected in value)
        for protected in protected_values
    )


def _diagnostic(entry: str, code: str) -> SkillDiagnostic:
    messages = {
        "catalog_unavailable": "The Skills directory could not be read.",
        "file_too_large": "SKILL.md exceeds the supported size.",
        "frontmatter_too_large": "SKILL.md frontmatter exceeds the supported size.",
        "invalid_description": "Skill description is invalid.",
        "invalid_frontmatter": "SKILL.md frontmatter is invalid.",
        "invalid_name": "Skill directory name is invalid.",
        "invalid_utf8": "SKILL.md is not valid UTF-8 text.",
        "missing_file": "SKILL.md is missing.",
        "name_mismatch": "Skill name must match its directory name.",
        "protected_value": "Skill metadata contains protected configuration data.",
        "read_failed": "SKILL.md could not be read.",
        "unsafe_path": "Skill path is not a safe local path.",
        "unsafe_root": "The Skills directory is not a safe local directory.",
    }
    return SkillDiagnostic(entry=entry, code=code, message=messages[code])


def _no_protected_values() -> tuple[str, ...]:
    return ()


__all__ = [
    "MAX_FRONTMATTER_BYTES",
    "MAX_SKILL_BYTES",
    "SkillCatalog",
    "SkillDiagnostic",
    "SkillDiscovery",
]
