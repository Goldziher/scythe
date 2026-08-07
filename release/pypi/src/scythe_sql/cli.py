"""Entry point installed as the `scythe` console script.

`pip install` does not run post-install code for a wheel, so there is no way
to download the binary at install time. Instead, each invocation ensures the
cached binary is present (downloading + verifying on first run only, which
prints one line to stderr) and then execs it, forwarding argv and letting the
kernel propagate the exit code. Never writes into site-packages at runtime --
that breaks read-only installs, containers, and `pip install --user`.
"""

from __future__ import annotations

import os
import sys

from scythe_sql import __version__
from scythe_sql.download import ensure_binary
from scythe_sql.errors import ScytheSqlError


def main() -> int:
    """Resolves the pinned scythe binary and execs it with the current argv."""
    try:
        binary_path = ensure_binary(__version__)
    except ScytheSqlError as exc:
        # Every error in this hierarchy already carries the `scythe-sql:` prefix
        # and its own remediation advice; re-wrapping it below would double the
        # prefix. Catching the shared base rather than naming subclasses is what
        # keeps checksum and platform failures on this branch as they are added.
        print(str(exc), file=sys.stderr)
        return 1
    except Exception as exc:  # noqa: BLE001 -- surface any unexpected failure with context
        print(f"scythe-sql: failed to resolve the scythe binary: {exc}", file=sys.stderr)
        return 1

    argv = [str(binary_path), *sys.argv[1:]]
    try:
        os.execv(str(binary_path), argv)  # noqa: S606 -- resolved via our own cache/PATH logic
    except OSError as exc:
        print(f"scythe-sql: failed to execute {binary_path}: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
