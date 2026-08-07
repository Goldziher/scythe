"""End-to-end tests for `ensure_binary`, driven against a local http.server.

The real download path is the one piece that wires checksum verification,
extraction and caching together, so it is exercised over a loopback HTTP server
serving a synthetic release rather than mocked out -- hermetic and offline, but
still through `urllib`.
"""

from __future__ import annotations

import http.server
import io
import os
import platform
import sys
import tarfile
import threading
import zipfile
from collections.abc import Iterator
from pathlib import Path

import pytest
from scythe_sql import download, extract
from scythe_sql.checksum import ChecksumMismatchError, MissingChecksumError, sha256_hex
from scythe_sql.download import DownloadError, ensure_binary
from scythe_sql.platform_resolver import ResolvedTarget, resolve_target
from scythe_sql.version_utils import PlaceholderVersionError

VERSION = "9.9.9"
PAYLOAD = b"#!/bin/sh\necho scythe 9.9.9\n"


def current_target() -> ResolvedTarget:
    """Resolves the target for the machine running the tests, so archives match it."""
    return resolve_target(
        sys_platform=sys.platform,
        machine=platform.machine(),
        is_musl=False,
        warn=lambda _message: None,
    )


def build_archive(resolved: ResolvedTarget, payload: bytes = PAYLOAD) -> bytes:
    """Builds a release archive holding the binary alongside goreleaser's LICENSE/README."""
    buffer = io.BytesIO()
    if resolved.archive_ext == "zip":
        with zipfile.ZipFile(buffer, "w") as archive:
            archive.writestr("LICENSE", "MIT")
            archive.writestr(f"scythe-{resolved.target}/{resolved.binary_name}", payload)
        return buffer.getvalue()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        info = tarfile.TarInfo(f"scythe-{resolved.target}/{resolved.binary_name}")
        info.size = len(payload)
        archive.addfile(info, io.BytesIO(payload))
    return buffer.getvalue()


class ReleaseServer:
    """A loopback HTTP server serving a fixed route table and recording requests."""

    def __init__(self, routes: dict[str, tuple[int, bytes]]) -> None:
        self.requests: list[str] = []
        recorded = self.requests

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802 -- BaseHTTPRequestHandler dispatch name
                recorded.append(self.path)
                status, body = routes.get(self.path, (404, b"Not Found"))
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, format: str, *args: object) -> None:  # noqa: A002 -- stdlib signature
                """Silences the per-request stderr logging."""

        self._server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        # The default 0.5s poll interval is paid back on every shutdown().
        self._thread = threading.Thread(target=self._server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)

    def __enter__(self) -> ReleaseServer:
        self._thread.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self._server.server_port}"


def release_routes(
    resolved: ResolvedTarget,
    *,
    archive: bytes,
    checksum: str | None = None,
    serve_asset: bool = True,
) -> dict[str, tuple[int, bytes]]:
    """Builds the two-route release layout: a checksums file and the asset itself."""
    asset_name = f"scythe-{resolved.target}.{resolved.archive_ext}"
    prefix = f"/releases/download/v{VERSION}"
    digest = sha256_hex(archive) if checksum is None else checksum
    routes: dict[str, tuple[int, bytes]] = {
        f"{prefix}/scythe_{VERSION}_checksums.txt": (200, f"{digest}  {asset_name}\n".encode()),
    }
    if serve_asset:
        routes[f"{prefix}/{asset_name}"] = (200, archive)
    return routes


@pytest.fixture
def cache_env(tmp_path: Path) -> dict[str, str]:
    return {"SCYTHE_CACHE_DIR": str(tmp_path / "cache")}


@pytest.fixture(autouse=True)
def no_path_binary(monkeypatch: pytest.MonkeyPatch) -> None:
    """Keeps a real `scythe` on the developer's PATH from short-circuiting the download."""
    monkeypatch.setattr("scythe_sql.preinstalled.has_matching_path_binary", lambda _version: False)


@pytest.fixture
def served(monkeypatch: pytest.MonkeyPatch) -> Iterator[object]:
    """Points `download.REPO` at a loopback server built from caller-supplied routes."""

    def _serve(routes: dict[str, tuple[int, bytes]]) -> ReleaseServer:
        server = ReleaseServer(routes)
        stack.append(server)
        server.__enter__()
        monkeypatch.setattr(download, "REPO", server.base_url)
        return server

    stack: list[ReleaseServer] = []
    try:
        yield _serve
    finally:
        for server in stack:
            server.__exit__()


def cached_path(cache_env: dict[str, str], resolved: ResolvedTarget) -> Path:
    return Path(cache_env["SCYTHE_CACHE_DIR"]) / VERSION / resolved.binary_name


def test_ensure_binary_downloads_verifies_extracts_and_caches(cache_env: dict[str, str], served: object) -> None:
    resolved = current_target()
    archive = build_archive(resolved)
    served(release_routes(resolved, archive=archive))

    result = ensure_binary(VERSION, env=cache_env)

    assert result == cached_path(cache_env, resolved)
    assert result.read_bytes() == PAYLOAD


@pytest.mark.skipif(os.name == "nt", reason="POSIX permission bits are meaningless on Windows")
def test_ensure_binary_marks_the_cached_binary_executable(cache_env: dict[str, str], served: object) -> None:
    resolved = current_target()
    served(release_routes(resolved, archive=build_archive(resolved)))

    result = ensure_binary(VERSION, env=cache_env)

    assert result.stat().st_mode & 0o111 == 0o111


def test_ensure_binary_rejects_a_checksum_mismatch(cache_env: dict[str, str], served: object) -> None:
    resolved = current_target()
    archive = build_archive(resolved)
    served(release_routes(resolved, archive=archive, checksum="00" * 32))

    with pytest.raises(ChecksumMismatchError, match="checksum mismatch"):
        ensure_binary(VERSION, env=cache_env)


def test_ensure_binary_leaves_no_cache_entry_after_a_checksum_mismatch(
    cache_env: dict[str, str], served: object
) -> None:
    """A rejected asset must not be reachable by the next run, in whole or in part."""
    resolved = current_target()
    served(release_routes(resolved, archive=build_archive(resolved), checksum="00" * 32))

    with pytest.raises(ChecksumMismatchError):
        ensure_binary(VERSION, env=cache_env)

    cache_root = Path(cache_env["SCYTHE_CACHE_DIR"])
    assert not cached_path(cache_env, resolved).exists()
    assert [path for path in cache_root.rglob("*") if path.is_file()] == []


def test_ensure_binary_raises_when_the_checksums_file_has_no_row_for_the_asset(
    cache_env: dict[str, str], served: object
) -> None:
    prefix = f"/releases/download/v{VERSION}"
    served({f"{prefix}/scythe_{VERSION}_checksums.txt": (200, b"%s  scythe-other-target.tar.gz\n" % (b"aa" * 32))})

    with pytest.raises(MissingChecksumError, match="no checksum entry"):
        ensure_binary(VERSION, env=cache_env)


def test_ensure_binary_reports_a_missing_asset_as_a_download_error(cache_env: dict[str, str], served: object) -> None:
    resolved = current_target()
    served(release_routes(resolved, archive=build_archive(resolved), serve_asset=False))

    with pytest.raises(DownloadError, match="failed to download.*404"):
        ensure_binary(VERSION, env=cache_env)


def test_ensure_binary_reports_a_missing_release_as_a_download_error(cache_env: dict[str, str], served: object) -> None:
    served({})

    with pytest.raises(DownloadError, match="failed to download.*404"):
        ensure_binary(VERSION, env=cache_env)


def test_ensure_binary_short_circuits_on_an_already_cached_binary(cache_env: dict[str, str], served: object) -> None:
    resolved = current_target()
    server = served(release_routes(resolved, archive=build_archive(resolved)))
    dest = cached_path(cache_env, resolved)
    dest.parent.mkdir(parents=True)
    dest.write_bytes(b"cached-binary")

    result = ensure_binary(VERSION, env=cache_env)

    assert result == dest
    assert result.read_bytes() == b"cached-binary"
    assert server.requests == []


def test_ensure_binary_replaces_a_zero_length_cached_binary(cache_env: dict[str, str], served: object) -> None:
    """A truncated cache entry is not a binary; trusting it fails at exec time forever."""
    resolved = current_target()
    server = served(release_routes(resolved, archive=build_archive(resolved)))
    dest = cached_path(cache_env, resolved)
    dest.parent.mkdir(parents=True)
    dest.touch()

    result = ensure_binary(VERSION, env=cache_env)

    assert result.read_bytes() == PAYLOAD
    assert server.requests != []


@pytest.mark.skipif(os.name == "nt", reason="the mode is not set on Windows, so this step cannot fail")
def test_ensure_binary_leaves_nothing_behind_when_finalizing_the_write_fails(
    cache_env: dict[str, str], served: object, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Stands in for an OOM kill or full disk part-way through installing the binary."""
    resolved = current_target()
    served(release_routes(resolved, archive=build_archive(resolved)))

    def boom(_path: Path) -> None:
        raise OSError(28, "No space left on device")

    monkeypatch.setattr(extract, "make_executable", boom)

    with pytest.raises(OSError, match="No space left on device"):
        ensure_binary(VERSION, env=cache_env)

    cache_root = Path(cache_env["SCYTHE_CACHE_DIR"])
    assert not cached_path(cache_env, resolved).exists()
    assert [path for path in cache_root.rglob("*") if path.is_file()] == []


def test_ensure_binary_honours_the_scythe_binary_override() -> None:
    result = ensure_binary(VERSION, env={"SCYTHE_BINARY": "/opt/scythe"})

    assert result == Path("/opt/scythe")


def test_ensure_binary_refuses_the_placeholder_version(cache_env: dict[str, str]) -> None:
    with pytest.raises(PlaceholderVersionError, match="placeholder"):
        ensure_binary("0.0.0", env=cache_env)


def test_ensure_binary_refuses_to_download_when_skip_download_is_set(cache_env: dict[str, str]) -> None:
    env = {**cache_env, "SCYTHE_SKIP_DOWNLOAD": "1"}

    with pytest.raises(DownloadError, match="SCYTHE_SKIP_DOWNLOAD=1"):
        ensure_binary(VERSION, env=env)


def test_ensure_binary_serves_a_skip_download_run_from_the_cache(cache_env: dict[str, str]) -> None:
    resolved = current_target()
    dest = cached_path(cache_env, resolved)
    dest.parent.mkdir(parents=True)
    dest.write_bytes(b"cached-binary")

    assert ensure_binary(VERSION, env={**cache_env, "SCYTHE_SKIP_DOWNLOAD": "1"}) == dest
