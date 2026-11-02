"""Single owner for the versioned ``~/.ikaros/config.yaml`` document."""

from __future__ import annotations

from collections.abc import Mapping
from copy import deepcopy
from pathlib import Path
from typing import Any, Literal

import yaml

from .configuration_io import (
    atomic_write_file,
    load_unique_yaml_document,
    sync_directory_best_effort,
)
from .errors import ConfigError
from .paths import RuntimePaths

CONFIG_VERSION = 2
MAX_CONFIG_BYTES = 256 * 1024

type ConfigSectionName = Literal["providers", "skills"]

_SECTION_NAMES: tuple[ConfigSectionName, ...] = ("providers", "skills")
_TOP_LEVEL_KEYS = frozenset({"version", *_SECTION_NAMES})


class ConfigDocumentStore:
    """Load and atomically replace known sections of the Runtime configuration.

    Section readers receive isolated snapshots. A writer can therefore update one
    section without gaining ownership of, or accidentally erasing, another one.
    Provider and Skill semantics deliberately live in their respective facades.
    """

    def __init__(self, runtime_home: Path) -> None:
        paths = RuntimePaths.from_home(runtime_home)
        self._runtime_home = paths.home
        self._path = paths.config
        self._sections = self._load()

    @property
    def path(self) -> Path:
        return self._path

    def read_section(self, name: ConfigSectionName) -> Mapping[str, object] | None:
        """Return an isolated, read-only-by-contract snapshot of a known section."""

        _require_section_name(name)
        section = self._sections.get(name)
        return deepcopy(section) if section is not None else None

    def replace_section(
        self,
        name: ConfigSectionName,
        section: Mapping[str, object],
    ) -> None:
        """Atomically replace one section while preserving every other known section."""

        _require_section_name(name)
        snapshot = _snapshot_section(name, section)
        if not snapshot:
            self.remove_section(name)
            return
        updated = deepcopy(self._sections)
        updated[name] = snapshot
        self._persist(updated)
        self._sections = updated

    def remove_section(self, name: ConfigSectionName) -> None:
        """Remove one section, deleting the document only when no non-empty section remains."""

        _require_section_name(name)
        if name not in self._sections:
            return
        updated = deepcopy(self._sections)
        del updated[name]
        self._persist(updated)
        self._sections = updated

    def _load(self) -> dict[ConfigSectionName, dict[str, object]]:
        if not self._path.exists():
            return {}

        read_failed = False
        oversized = False
        source = b""
        try:
            with self._path.open("rb") as handle:
                source = handle.read(MAX_CONFIG_BYTES + 1)
            oversized = len(source) > MAX_CONFIG_BYTES
        except OSError:
            read_failed = True
        if read_failed:
            raise ConfigError("config.yaml could not be read or parsed")
        if oversized:
            raise ConfigError("config.yaml exceeds the supported size")

        parse_failed = False
        document: Any = None
        try:
            document = load_unique_yaml_document(source.decode("utf-8"))
        except (UnicodeError, yaml.YAMLError):
            parse_failed = True
        if parse_failed:
            raise ConfigError("config.yaml could not be read or parsed")
        return _parse_document(document)

    def _persist(self, sections: Mapping[ConfigSectionName, Mapping[str, object]]) -> None:
        non_empty = {name: section for name, section in sections.items() if section}
        if not non_empty:
            self._delete_document()
            return

        document: dict[str, object] = {"version": CONFIG_VERSION}
        for name in _SECTION_NAMES:
            section = non_empty.get(name)
            if section:
                document[name] = section
        serialized = _serialize(document)
        write_failed = False
        try:
            atomic_write_file(
                self._path,
                serialized,
                temporary_prefix=".config-",
            )
        except OSError:
            write_failed = True
        if write_failed:
            raise ConfigError("config.yaml could not be updated")

    def _delete_document(self) -> None:
        delete_failed = False
        try:
            self._path.unlink(missing_ok=True)
        except OSError:
            delete_failed = True
        if delete_failed:
            raise ConfigError("config.yaml could not be updated")
        if self._runtime_home.exists():
            sync_directory_best_effort(self._runtime_home)


def _parse_document(value: Any) -> dict[ConfigSectionName, dict[str, object]]:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise ConfigError("config.yaml has an invalid top-level structure")
    if "version" not in value or set(value) - _TOP_LEVEL_KEYS:
        raise ConfigError("config.yaml has an invalid top-level structure")
    version = value["version"]
    if version != CONFIG_VERSION or isinstance(version, bool):
        raise ConfigError("config.yaml uses an unsupported version")

    sections: dict[ConfigSectionName, dict[str, object]] = {}
    for name in _SECTION_NAMES:
        if name not in value:
            continue
        section = value[name]
        if not isinstance(section, dict):
            raise ConfigError(f"config.yaml {name} must be a mapping")
        if not all(isinstance(key, str) for key in section):
            raise ConfigError(f"config.yaml {name} contains an invalid section key")
        if section:
            sections[name] = deepcopy(section)
    return sections


def _snapshot_section(
    name: ConfigSectionName,
    section: Mapping[str, object],
) -> dict[str, object]:
    snapshot_failed = False
    snapshot: dict[str, object] = {}
    try:
        snapshot = deepcopy(dict(section))
    except Exception:
        snapshot_failed = True
    if snapshot_failed:
        raise ConfigError(f"config.yaml {name} could not be updated")
    if not all(isinstance(key, str) for key in snapshot):
        raise ConfigError(f"config.yaml {name} contains an invalid section key")
    return snapshot


def _require_section_name(name: object) -> None:
    if name not in _SECTION_NAMES:
        raise ConfigError("config.yaml section is not supported")


def _serialize(document: Mapping[str, object]) -> bytes:
    serialization_failed = False
    serialized = ""
    try:
        serialized = yaml.safe_dump(
            dict(document),
            allow_unicode=True,
            default_flow_style=False,
            sort_keys=False,
        )
        encoded = serialized.encode("utf-8")
    except (UnicodeError, yaml.YAMLError):
        serialization_failed = True
        encoded = b""
    if serialization_failed:
        raise ConfigError("config.yaml could not be serialized")
    if len(encoded) > MAX_CONFIG_BYTES:
        raise ConfigError("config.yaml exceeds the supported size")
    return encoded


__all__ = [
    "CONFIG_VERSION",
    "ConfigDocumentStore",
    "ConfigSectionName",
    "MAX_CONFIG_BYTES",
]
