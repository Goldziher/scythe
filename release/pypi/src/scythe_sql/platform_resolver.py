"""Platform+arch -> Rust target triple resolution for scythe release assets.

Release assets published by goreleaser (verified against the v0.12.0 release):
    scythe-x86_64-unknown-linux-gnu.tar.gz
    scythe-aarch64-unknown-linux-gnu.tar.gz
    scythe-x86_64-apple-darwin.tar.gz
    scythe-aarch64-apple-darwin.tar.gz
    scythe-x86_64-pc-windows-gnu.zip

There is no musl target and no aarch64 Windows asset.
"""

from __future__ import annotations

import sys
from collections.abc import Callable
from dataclasses import dataclass

TRIPLES: dict[tuple[str, str], str] = {
    ("linux", "x64"): "x86_64-unknown-linux-gnu",
    ("linux", "arm64"): "aarch64-unknown-linux-gnu",
    ("darwin", "x64"): "x86_64-apple-darwin",
    ("darwin", "arm64"): "aarch64-apple-darwin",
    ("win32", "x64"): "x86_64-pc-windows-gnu",
}

_MACHINE_ALIASES: dict[str, str] = {
    "x86_64": "x64",
    "amd64": "x64",
    "aarch64": "arm64",
    "arm64": "arm64",
}


@dataclass(frozen=True)
class ResolvedTarget:
    """Resolved Rust target triple, archive extension, and binary filename."""

    target: str
    archive_ext: str
    binary_name: str


class UnsupportedPlatformError(RuntimeError):
    """Raised when no release asset exists for the current platform/arch."""


def normalize_platform(sys_platform: str) -> str:
    """Maps ``sys.platform`` values to the npm-style keys used by ``TRIPLES``."""
    if sys_platform.startswith("linux"):
        return "linux"
    if sys_platform == "darwin":
        return "darwin"
    if sys_platform.startswith("win"):
        return "win32"
    return sys_platform


def normalize_machine(machine: str) -> str:
    """Maps ``platform.machine()`` values to the npm-style arch keys."""
    return _MACHINE_ALIASES.get(machine.lower(), machine.lower())


def is_musl_linux(libc_ver: tuple[str, str]) -> bool:
    """Detects musl libc (e.g. Alpine) from :func:`platform.libc_ver` output.

    On musl systems, ``platform.libc_ver()`` (which only recognizes glibc)
    returns ``("", "")`` instead of a glibc version tuple.
    """
    return libc_ver == ("", "")


def resolve_target(
    *,
    sys_platform: str = sys.platform,
    machine: str,
    is_musl: bool = False,
    warn: Callable[[str], None] | None = None,
) -> ResolvedTarget:
    """Resolves the release target for the current platform/arch.

    Raises :class:`UnsupportedPlatformError` naming the platform and arch,
    or the musl-specific guidance, when no asset is published.
    """
    platform_key = normalize_platform(sys_platform)
    arch_key = normalize_machine(machine)
    warn_fn = warn or (lambda message: print(message, file=sys.stderr))  # noqa: T201

    if platform_key == "linux" and is_musl:
        raise UnsupportedPlatformError(
            f"scythe-sql: unsupported platform 'linux/{arch_key}' (musl libc, e.g. Alpine). "
            "No musl build is published. Install a glibc-based image, or install scythe via "
            "'cargo install scythe-cli' instead."
        )

    if platform_key == "win32" and arch_key == "arm64":
        warn_fn(
            "scythe-sql: no native Windows ARM64 build is published; falling back to the "
            "x86_64-pc-windows-gnu binary, which runs under Windows 11 ARM64 x64 emulation."
        )
        return ResolvedTarget(target=TRIPLES[("win32", "x64")], archive_ext="zip", binary_name="scythe.exe")

    target = TRIPLES.get((platform_key, arch_key))
    if target is None:
        supported = ", ".join(f"{p}/{a}" for p, a in TRIPLES)
        raise UnsupportedPlatformError(
            f"scythe-sql: unsupported platform '{platform_key}/{arch_key}'. Supported platforms: "
            f"{supported} (plus win32/arm64 via x64 emulation). Install scythe via "
            "'cargo install scythe-cli' instead."
        )

    archive_ext = "zip" if platform_key == "win32" else "tar.gz"
    binary_name = "scythe.exe" if platform_key == "win32" else "scythe"
    return ResolvedTarget(target=target, archive_ext=archive_ext, binary_name=binary_name)
