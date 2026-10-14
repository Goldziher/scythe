#!/usr/bin/env python3
"""Sync the coding-agent plugin version to the CLI crate version.

``ai-rulez generate --plugin`` renders every per-runtime plugin manifest under
``plugin/`` from ``plugin/.ai-rulez/config.plugins.toml``'s ``[plugin].version``. This script
keeps that single source pinned to the CLI crate version in ``crates/scythe-cli/Cargo.toml`` (the repo's
version source of truth), alongside the existing crate version synchronization. Run via ``task plugin:generate`` (which then regenerates the bundles);
freshness of the generated bundles themselves is enforced by ``ai-rulez verify
--plugin`` in CI, so this script only touches the config source.

Usage:
    python3 scripts/sync_plugin_version.py                 # apply
    python3 scripts/sync_plugin_version.py --check         # verify against crates/scythe-cli/Cargo.toml
    python3 scripts/sync_plugin_version.py --expect X.Y.Z  # verify both equal X.Y.Z

``--check`` answers "does plugin/ track core?" and is the gate CI runs on `main`.
``--expect`` additionally pins both to the version actually being released, so a
release cannot ship a plugin bundle that lags the tag it is built from. Both exit 1
on drift.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLUGIN_CONFIG = ROOT / "plugin" / ".ai-rulez" / "config.plugins.toml"
if not PLUGIN_CONFIG.exists():
    PLUGIN_CONFIG = ROOT / "plugin" / ".ai-rulez" / "config.toml"
# ~keep Matches the `version` key of the `[plugin]` table only. The file also has a
# top-level ai-rulez schema `version = "5.0"` that must never be rewritten.
PLUGIN_VERSION_RE = re.compile(r'(\[plugin\][^\[]*?\nversion = ")([^"]*)(")', re.DOTALL)
SYNC_HINT = "run `task plugin:generate` to re-pin the plugin and regenerate its bundles"


def core_version() -> str:
    """Read the workspace crate version from ``Cargo.toml``."""
    text = (ROOT / "crates/scythe-cli/Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'(?m)^version = "([^"]+)"', text)
    if not match:
        sys.exit("could not read version from Cargo.toml")
    return match.group(1)


def plugin_version() -> str:
    """Read ``[plugin].version`` from the plugin config."""
    match = PLUGIN_VERSION_RE.search(PLUGIN_CONFIG.read_text(encoding="utf-8"))
    if not match:
        sys.exit(f"could not read [plugin].version from {PLUGIN_CONFIG.relative_to(ROOT)}")
    return match.group(2)


def transform(version: str) -> tuple[str, str]:
    """Rewrite ``version`` inside the ``[plugin]`` table (not the top-level ai-rulez
    schema ``version = "5.0"``) of the plugin config. Uses the native/semver form,
    which is also what ai-rulez expects; ai-rulez renders the PEP 440 form for the
    Hermes wheel.
    """
    original = PLUGIN_CONFIG.read_text(encoding="utf-8")
    updated = PLUGIN_VERSION_RE.sub(rf"\g<1>{version}\g<3>", original, count=1)
    return original, updated


def parse_expect(argv: list[str]) -> str | None:
    """Return the version passed to ``--expect``, or ``None`` when absent."""
    for index, arg in enumerate(argv):
        if arg == "--expect":
            if index + 1 >= len(argv):
                sys.exit("--expect requires a version argument")
            return argv[index + 1]
        if arg.startswith("--expect="):
            return arg.split("=", 1)[1]
    return None


def report_drift(expected: str, core: str, plugin: str) -> int:
    """Print every version that disagrees with ``expected`` and fail."""
    sys.stdout.write(f"plugin/core version drift — expected {expected}" + "\n")
    if core != expected:
        sys.stdout.write(f"  crates/scythe-cli/Cargo.toml [package].version -> {core}" + "\n")
    if plugin != expected:
        sys.stdout.write(f"  {PLUGIN_CONFIG.relative_to(ROOT)} [plugin].version -> {plugin}" + "\n")
    sys.stdout.write(SYNC_HINT + "\n")
    return 1


def main() -> int:
    argv = sys.argv[1:]
    expect = parse_expect(argv)
    core = core_version()

    if expect is not None:
        plugin = plugin_version()
        if core == expect and plugin == expect:
            sys.stdout.write(f"plugin and core both at {expect}" + "\n")
            return 0
        return report_drift(expect, core, plugin)

    original, updated = transform(core)
    if original == updated:
        return 0
    if "--check" in argv:
        return report_drift(core, core, plugin_version())
    PLUGIN_CONFIG.write_text(updated, encoding="utf-8")
    sys.stdout.write(f"synced plugin version -> {core}" + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
