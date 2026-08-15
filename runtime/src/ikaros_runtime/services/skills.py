"""Application service for global Skill discovery and enablement."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ..config import ConfigDocumentStore
from ..domain import SkillDescriptor, is_valid_skill_name
from ..errors import ConfigError, InvalidParamsError
from ..skills.catalog import SkillCatalog, SkillDiscovery


@dataclass(frozen=True, slots=True)
class SkillSummary:
    name: str
    description: str
    location: str
    enabled: bool

    @classmethod
    def from_descriptor(cls, descriptor: SkillDescriptor, *, enabled: bool) -> SkillSummary:
        return cls(
            name=descriptor.name,
            description=descriptor.description,
            location=descriptor.location,
            enabled=enabled,
        )

    def to_wire(self) -> dict[str, object]:
        return {
            "name": self.name,
            "description": self.description,
            "location": self.location,
            "enabled": self.enabled,
        }


class SkillService:
    def __init__(self, catalog: SkillCatalog, config: ConfigDocumentStore) -> None:
        self._catalog = catalog
        self._config = config
        self._disabled = _load_disabled(config)

    def list_skills(self, params: dict[str, Any]) -> dict[str, object]:
        if params:
            raise InvalidParamsError("skill.list does not accept parameters")
        discovery = self._catalog.discover()
        return {
            "skills": [
                SkillSummary.from_descriptor(
                    descriptor,
                    enabled=descriptor.name not in self._disabled,
                ).to_wire()
                for descriptor in discovery.skills
            ],
            "diagnostics": [diagnostic.to_wire() for diagnostic in discovery.diagnostics],
        }

    def enabled_descriptors(self) -> tuple[SkillDescriptor, ...]:
        return tuple(
            descriptor
            for descriptor in self._catalog.discover().skills
            if descriptor.name not in self._disabled
        )

    def set_enabled(self, params: dict[str, Any]) -> dict[str, object]:
        if set(params) != {"name", "enabled"}:
            raise InvalidParamsError("skill.set_enabled requires exactly name and enabled")
        name = params["name"]
        enabled = params["enabled"]
        if not is_valid_skill_name(name):
            raise InvalidParamsError("Skill name is invalid")
        if not isinstance(enabled, bool):
            raise InvalidParamsError("Skill enabled must be a boolean")
        discovery = self._catalog.discover()
        descriptor = _find_skill(discovery, name)

        updated = set(self._disabled)
        if enabled:
            updated.discard(name)
        else:
            updated.add(name)
        if updated != self._disabled:
            if updated:
                self._config.replace_section("skills", {"disabled": sorted(updated)})
            else:
                self._config.remove_section("skills")
            self._disabled = updated
        return {
            "skill": SkillSummary.from_descriptor(descriptor, enabled=enabled).to_wire(),
        }


def _load_disabled(config: ConfigDocumentStore) -> set[str]:
    section = config.read_section("skills")
    if section is None:
        return set()
    if set(section) != {"disabled"}:
        raise ConfigError("config.yaml skills has an invalid structure")
    value = section["disabled"]
    if not isinstance(value, list):
        raise ConfigError("config.yaml skills.disabled must be an array")
    if any(not is_valid_skill_name(name) for name in value):
        raise ConfigError("config.yaml skills.disabled contains an invalid Skill name")
    names = [name for name in value if isinstance(name, str)]
    if len(names) != len(set(names)):
        raise ConfigError("config.yaml skills.disabled must contain unique names")
    return set(names)


def _find_skill(discovery: SkillDiscovery, name: str) -> SkillDescriptor:
    descriptor = next((candidate for candidate in discovery.skills if candidate.name == name), None)
    if descriptor is None:
        raise InvalidParamsError("Skill does not exist")
    return descriptor


__all__ = ["SkillService", "SkillSummary"]
