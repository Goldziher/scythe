#!/usr/bin/env python3
"""Injects the release version into the committed 0.0.0 placeholders.

Committed manifests always carry `0.0.0` -- never a real version in git.
The publish workflow calls this with `RELEASE_VERSION` set (or a positional
argument) right before `uv build`, so the built wheel/sdist carry the real
version. Never run this against a checkout you intend to commit.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
PYPROJECT = PACKAGE_ROOT / "pyproject.toml"
INIT_FILE = PACKAGE_ROOT / "src" / "scythe_sql" / "__init__.py"


def main() -> int:
    version = sys.argv[1] if len(sys.argv) > 1 else __import__("os").environ.get("RELEASE_VERSION")
    if not version:
        print("set_version.py: pass the version as an argument or set RELEASE_VERSION", file=sys.stderr)
        return 1
    if version == "0.0.0":
        print("set_version.py: refusing to set the placeholder version 0.0.0", file=sys.stderr)
        return 1

    pyproject_text = PYPROJECT.read_text()
    new_pyproject_text, count = re.subn(
        r'^version = "0\.0\.0"$', f'version = "{version}"', pyproject_text, count=1, flags=re.MULTILINE
    )
    if count != 1:
        print(f'set_version.py: could not find version = "0.0.0" in {PYPROJECT}', file=sys.stderr)
        return 1
    PYPROJECT.write_text(new_pyproject_text)

    init_text = INIT_FILE.read_text()
    new_init_text, count = re.subn(
        r'^__version__ = "0\.0\.0"$', f'__version__ = "{version}"', init_text, count=1, flags=re.MULTILINE
    )
    if count != 1:
        print(f'set_version.py: could not find __version__ = "0.0.0" in {INIT_FILE}', file=sys.stderr)
        return 1
    INIT_FILE.write_text(new_init_text)

    print(f"set_version.py: set version to {version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
