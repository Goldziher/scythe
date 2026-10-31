# AI-RULEZ :: GENERATED FILE — DO NOT EDIT
# Content-Hash: blake3:6493876cbf379aa936215bfc8580764781d62c6f8fa6d45e83f52ee70e7d9dd8
# Source-Hash: blake3:bcc197f0dde443cc11b753bd6bb5d9992793f20ef5b75fe76a8a69c6e29e2384
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
