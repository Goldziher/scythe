from __future__ import annotations

from pathlib import Path

import pytest
from scythe_sql.platform_resolver import (
    TRIPLES,
    ResolvedTarget,
    UnsupportedPlatformError,
    is_musl_linux,
    musl_loader_present,
    normalize_machine,
    normalize_platform,
    resolve_target,
)


def _always() -> bool:
    return True


def _never() -> bool:
    return False


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


def test_is_musl_linux_trusts_an_explicit_musl_report() -> None:
    """CPython 3.14+ names musl directly, so no on-disk probe should be consulted."""
    assert is_musl_linux(("musl", "1.2.5"), loader_present=_never) is True


def test_is_musl_linux_trusts_an_explicit_glibc_report() -> None:
    assert is_musl_linux(("glibc", "2.35"), loader_present=_always) is False
    assert is_musl_linux(("libc", "6"), loader_present=_always) is False


def test_is_musl_linux_falls_back_to_the_loader_probe_when_libc_ver_is_empty() -> None:
    """On 3.9-3.13 musl reports ("", ""), but so does a glibc host whose lookup failed."""
    assert is_musl_linux(("", ""), loader_present=_always) is True
    assert is_musl_linux(("", ""), loader_present=_never) is False


def test_musl_loader_present_detects_the_arch_suffixed_loader(tmp_path: Path) -> None:
    (tmp_path / "ld-musl-x86_64.so.1").touch()
    assert musl_loader_present([str(tmp_path)]) is True


def test_musl_loader_present_is_false_for_missing_dirs_and_glibc_layouts(tmp_path: Path) -> None:
    (tmp_path / "ld-linux-x86-64.so.2").touch()
    assert musl_loader_present([str(tmp_path), str(tmp_path / "does-not-exist")]) is False


def test_triples_has_no_musl_or_windows_arm64_entries() -> None:
    assert not any("musl" in target for target in TRIPLES.values())
    assert ("win32", "arm64") not in TRIPLES
