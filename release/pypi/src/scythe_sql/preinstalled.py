"""Detects a matching scythe binary already on PATH."""

from __future__ import annotations

import subprocess
from collections.abc import Callable

from scythe_sql.version_utils import extract_version

_ExecFn = Callable[[], str]


def _default_exec() -> str:
    result = subprocess.run(  # noqa: S603
        ["scythe", "--version"],  # noqa: S607
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout


def has_matching_path_binary(want_version: str, exec_fn: _ExecFn = _default_exec) -> bool:
    """Checks whether a `scythe` binary already on PATH matches `want_version` exactly.

    Exact equality only: a newer binary on PATH does not satisfy a pin, since
    silently upgrading defeats the point of pinning the dependency version.
    """
    try:
        output = exec_fn()
    except (OSError, subprocess.SubprocessError):
        return False
    return extract_version(output) == want_version
