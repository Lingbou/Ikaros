"""Provider-neutral prompt fragment for a frozen Skill descriptor snapshot."""

from __future__ import annotations

from collections.abc import Sequence
from xml.sax.saxutils import escape

from ..domain import SkillDescriptor


def build_skill_prompt(skills: Sequence[SkillDescriptor]) -> str | None:
    if not skills:
        return None
    rows = [
        "Skills provide optional instructions for specialized tasks. When a Skill is relevant, "
        "use the read tool to load its SKILL.md from the listed location before following it.",
        "<available_skills>",
    ]
    for skill in skills:
        rows.extend(
            (
                "  <skill>",
                f"    <name>{escape(skill.name)}</name>",
                f"    <description>{escape(skill.description)}</description>",
                f"    <location>{escape(skill.location)}</location>",
                "  </skill>",
            )
        )
    rows.append("</available_skills>")
    return "\n".join(rows)


__all__ = ["build_skill_prompt"]
