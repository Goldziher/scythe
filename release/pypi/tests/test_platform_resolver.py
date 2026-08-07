from __future__ import annotations

import pytest
from scythe_sql.platform_resolver import (
    TRIPLES,
    ResolvedTarget,
    UnsupportedPlatformError,
    is_musl_linux,
    normalize_machine,
    normalize_platform,
    resolve_target,
)


def test_resolve_target_maps_all_five_known_platform_arch_pairs() -> None:
    assert resolve_target(sys_platform="linux", machine="x86_64").target == "x86_64-unknown-linux-gnu"
    assert resolve_target(sys_platform="linux", machine="aarch64").target == "aarch64-unknown-linux-gnu"
    assert resolve_target(sys_platform="darwin", machine="x86_64").target == "x86_64-apple-darwin"
    assert resolve_target(sys_platform="darwin", machine="arm64").target == "aarch64-apple-darwin"
    assert resolve_target(sys_platform="win32", machine="AMD64").target == "x86_64-pc-windows-gnu"


def test_resolve_target_picks_tar_gz_for_unix_and_zip_for_windows() -> None:
    assert resolve_target(sys_platform="linux", machine="x86_64") == ResolvedTarget(
        target="x86_64-unknown-linux-gnu", archive_ext="tar.gz", binary_name="scythe"
    )
    assert resolve_target(sys_platform="win32", machine="AMD64") == ResolvedTarget(
        target="x86_64-pc-windows-gnu", archive_ext="zip", binary_name="scythe.exe"
    )


def test_resolve_target_hard_fails_on_musl_linux() -> None:
    with pytest.raises(UnsupportedPlatformError, match=r"musl libc.*cargo install scythe-cli"):
        resolve_target(sys_platform="linux", machine="x86_64", is_musl=True)


def test_resolve_target_falls_back_to_x64_with_warning_on_win32_arm64() -> None:
    warnings: list[str] = []
    result = resolve_target(sys_platform="win32", machine="ARM64", warn=warnings.append)
    assert result == ResolvedTarget(target="x86_64-pc-windows-gnu", archive_ext="zip", binary_name="scythe.exe")
    assert len(warnings) == 1
    assert "ARM64" in warnings[0]


def test_resolve_target_hard_errors_on_unknown_platform_arch() -> None:
    with pytest.raises(UnsupportedPlatformError, match="freebsd/x64"):
        resolve_target(sys_platform="freebsd", machine="x86_64")
    with pytest.raises(UnsupportedPlatformError, match="linux/i686"):
        resolve_target(sys_platform="linux", machine="i686")


def test_normalize_platform() -> None:
    assert normalize_platform("linux") == "linux"
    assert normalize_platform("linux2") == "linux"
    assert normalize_platform("darwin") == "darwin"
    assert normalize_platform("win32") == "win32"


def test_normalize_machine() -> None:
    assert normalize_machine("x86_64") == "x64"
    assert normalize_machine("AMD64") == "x64"
    assert normalize_machine("aarch64") == "arm64"
    assert normalize_machine("arm64") == "arm64"
    assert normalize_machine("i686") == "i686"


def test_is_musl_linux_detects_empty_libc_ver() -> None:
    assert is_musl_linux(("", "")) is True
    assert is_musl_linux(("glibc", "2.35")) is False


def test_triples_has_no_musl_or_windows_arm64_entries() -> None:
    assert not any("musl" in target for target in TRIPLES.values())
    assert ("win32", "arm64") not in TRIPLES
