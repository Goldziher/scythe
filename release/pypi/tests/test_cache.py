from __future__ import annotations

from pathlib import Path

from scythe_sql.cache import cached_binary_path, resolve_cache_root


def test_resolve_cache_root_honours_scythe_cache_dir_override() -> None:
    assert resolve_cache_root(env={"SCYTHE_CACHE_DIR": "/custom"}, sys_platform="linux", home=Path("/home/u")) == Path(
        "/custom"
    )


def test_resolve_cache_root_uses_xdg_cache_home_on_linux() -> None:
    assert resolve_cache_root(env={"XDG_CACHE_HOME": "/xdg"}, sys_platform="linux", home=Path("/home/u")) == Path(
        "/xdg/scythe"
    )


def test_resolve_cache_root_defaults_to_dot_cache_on_linux_and_darwin() -> None:
    assert resolve_cache_root(env={}, sys_platform="linux", home=Path("/home/u")) == Path("/home/u/.cache/scythe")
    assert resolve_cache_root(env={}, sys_platform="darwin", home=Path("/Users/u")) == Path("/Users/u/.cache/scythe")


def test_resolve_cache_root_uses_localappdata_on_windows() -> None:
    assert resolve_cache_root(
        env={"LOCALAPPDATA": "C:/Users/u/AppData/Local"}, sys_platform="win32", home=Path("C:/Users/u")
    ) == Path("C:/Users/u/AppData/Local/scythe/cache")


def test_resolve_cache_root_falls_back_on_windows_without_localappdata() -> None:
    assert resolve_cache_root(env={}, sys_platform="win32", home=Path("C:/Users/u")) == Path(
        "C:/Users/u/AppData/Local/scythe/cache"
    )


def test_cached_binary_path_nests_by_version_and_binary_name() -> None:
    assert cached_binary_path(
        env={}, sys_platform="linux", home=Path("/home/u"), version="0.13.0", binary_name="scythe"
    ) == Path("/home/u/.cache/scythe/0.13.0/scythe")
