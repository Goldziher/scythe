# AI-RULEZ :: GENERATED FILE — DO NOT EDIT
# Content-Hash: blake3:6493876cbf379aa936215bfc8580764781d62c6f8fa6d45e83f52ee70e7d9dd8
# Source-Hash: blake3:dda6f642ae20dd0ec58dfb83a2337b9997b046a92f543f4dfb9a5ca28f2dd3f7
# Schema-Version: v1

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
