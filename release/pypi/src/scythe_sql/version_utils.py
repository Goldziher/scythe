"""Version string parsing and the 0.0.0 placeholder guard."""

from __future__ import annotations

import re

from scythe_sql.errors import ScytheSqlError

_VERSION_RE = re.compile(r"(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)")


def extract_version(output: str) -> str | None:
    """Permissively extracts a semver-ish version token from CLI output.

    The exact output format of the real ``scythe --version`` binary is not
    pinned by design, so this finds the first ``MAJOR.MINOR.PATCH[-pre][+build]``
    token rather than anchoring to the full line.
    """
    match = _VERSION_RE.search(output)
    return match.group(1) if match else None


class PlaceholderVersionError(ScytheSqlError):
    """Raised when the package still carries the unbuilt 0.0.0 placeholder version."""


def assert_real_version(version: str) -> None:
    """Refuses to proceed when `version` is the unbuilt `0.0.0` placeholder.

    A wrapper built from a dirty/unversioned checkout would otherwise
    construct a nonsense download URL.
    """
    if version == "0.0.0":
        raise PlaceholderVersionError(
            "scythe-sql: this package was built incorrectly -- it still carries the placeholder "
            "version '0.0.0'. The publish workflow must inject the real release version (RELEASE_VERSION) "
            "before building. Please report this at https://github.com/Goldziher/scythe/issues."
        )
