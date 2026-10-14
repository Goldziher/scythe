"""Register the bundled Scythe skill with Hermes Agent."""

from pathlib import Path
from typing import Protocol


class SkillRegistry(Protocol):
    """The Hermes registration boundary without a dependency on the host SDK."""

    def register_skill(self, name: str, path: Path, description: str = "") -> None: ...


def register(ctx: SkillRegistry) -> None:
    """Expose the installed skill through Hermes' native plugin skill registry."""
    skill = Path(__file__).resolve().parent / "skills/scythe/SKILL.md"
    ctx.register_skill("scythe", skill, description="Generate database access code from annotated SQL with Scythe.")
