"""Cache directory resolution for the downloaded scythe binary."""

import os
from pathlib import Path


def resolve_cache_root(*, env: dict[str, str], sys_platform: str, home: Path) -> Path:
    """Resolves the root cache directory, honouring `SCYTHE_CACHE_DIR` and XDG conventions."""
    override = env.get("SCYTHE_CACHE_DIR")
    if override:
        return Path(override)

    if sys_platform.startswith("win"):
        base = env.get("LOCALAPPDATA")
        base_path = Path(base) if base else home / "AppData" / "Local"
        return base_path / "scythe" / "cache"

    xdg_cache_home = env.get("XDG_CACHE_HOME")
    if xdg_cache_home:
        return Path(xdg_cache_home) / "scythe"

    return home / ".cache" / "scythe"


def cached_binary_path(*, env: dict[str, str], sys_platform: str, home: Path, version: str, binary_name: str) -> Path:
    """Resolves the cached binary path for a given version."""
    return resolve_cache_root(env=env, sys_platform=sys_platform, home=home) / version / binary_name


def default_home() -> Path:
    """Returns the current user's home directory."""
    return Path(os.path.expanduser("~"))
