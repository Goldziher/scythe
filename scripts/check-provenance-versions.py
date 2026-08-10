#!/usr/bin/env python3
"""Assert every committed `scythe:provenance v=X.Y.Z` header matches the
version in crates/scythe-cli/Cargo.toml.

Why this exists (issue #129): the inline check in .github/workflows/ci.yml
scans `integration_tests/` only, but 28 generated snippets under
`website/src/content/docs/` carry the same header. A release that bumps the
version and regenerates integration_tests but not the docs ships a
documentation site whose every sample claims the previous release — and the
CI check passes, because it never looked there.

Two properties this deliberately keeps, because both are how a version check
degrades into a no-op:

  * Per-root vacuity. The workflow already errors when the *total* header
    count is zero. That is not enough once there is more than one root: adding
    a root that matches nothing is then invisible, which is exactly the blind
    spot being closed. So each root must independently yield at least one
    header, and a root that yields none is an error naming that root.

  * No denominator games. Coverage here is not a ratio and there is nothing to
    take a percentage of — every header either agrees or the run fails. A
    shrinking set of scanned files cannot improve the outcome; it fails the
    vacuity check instead.

Usage: scripts/check-provenance-versions.py
Exit status: 0 if every header agrees, 1 otherwise.
"""

from __future__ import annotations

import os
import re
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Every tree that contains generated artifacts carrying a provenance header.
# Adding a root here is safe: if it contains no headers the run fails loudly
# rather than silently widening the scan by nothing.
SCAN_ROOTS = (
    "integration_tests",
    "website/src/content/docs",
)

PROVENANCE_RE = re.compile(r"scythe:provenance v=(\d+\.\d+\.\d+)")

# Directories never worth walking: build output and dependency trees, which
# can contain vendored copies of generated files and are not committed.
SKIP_DIRS = frozenset(
    {
        ".git",
        ".gradle",
        ".venv",
        "_build",
        "bin",
        "build",
        "deps",
        "node_modules",
        "obj",
        "target",
        "vendor",
    }
)


def expected_version() -> str:
    """The version in crates/scythe-cli/Cargo.toml — the single source of truth."""
    path = os.path.join(ROOT, "crates", "scythe-cli", "Cargo.toml")
    with open(path, "rb") as handle:
        return tomllib.load(handle)["package"]["version"]


def scan_root(root: str) -> list[tuple[str, str]]:
    """Return [(relative_path, version)] for every file under `root` whose
    first provenance header was found. Unreadable or binary files are skipped;
    they cannot carry a header this generator wrote."""
    found: list[tuple[str, str]] = []
    absolute_root = os.path.join(ROOT, root)
    if not os.path.isdir(absolute_root):
        print(f"error: scan root '{root}' does not exist", file=sys.stderr)
        sys.exit(1)
    for dirpath, dirnames, filenames in os.walk(absolute_root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for filename in filenames:
            path = os.path.join(dirpath, filename)
            try:
                with open(path, encoding="utf-8") as handle:
                    contents = handle.read()
            except (OSError, UnicodeDecodeError):
                continue
            match = PROVENANCE_RE.search(contents)
            if match:
                found.append((os.path.relpath(path, ROOT), match.group(1)))
    return found


def main() -> int:
    expected = expected_version()
    offenders: list[str] = []
    empty_roots: list[str] = []
    total = 0

    for root in SCAN_ROOTS:
        headers = scan_root(root)
        print(f"{root}: {len(headers)} provenance header(s)")
        if not headers:
            empty_roots.append(root)
            continue
        total += len(headers)
        offenders.extend(
            f"  {path}: v={version} (expected v={expected})" for path, version in sorted(headers) if version != expected
        )

    for root in empty_roots:
        print(
            f"error: found no scythe:provenance headers under '{root}'. Generated artifacts "
            f"there are expected to carry one, so this check would pass vacuously for that root. "
            f"Either regenerate them, or delete the root from SCAN_ROOTS in this script.",
            file=sys.stderr,
        )

    if offenders:
        print(
            f"error: {len(offenders)} provenance header(s) disagree with v={expected} "
            f"(from crates/scythe-cli/Cargo.toml):",
            file=sys.stderr,
        )
        print("\n".join(offenders), file=sys.stderr)
        print(
            f"Run 'task version:sync VERSION={expected}', regenerate the website snippets, and commit the result.",
            file=sys.stderr,
        )

    if offenders or empty_roots:
        return 1

    print(f"All {total} provenance header(s) across {len(SCAN_ROOTS)} root(s) agree on v={expected}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
