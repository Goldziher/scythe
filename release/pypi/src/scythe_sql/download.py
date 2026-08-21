"""Download and cache the platform-specific scythe binary."""

import os
import platform
import stat
import sys
import urllib.error
import urllib.request
from pathlib import Path

from scythe_sql.cache import cached_binary_path, default_home
from scythe_sql.checksum import expected_checksum, parse_checksums, verify_checksum
from scythe_sql.errors import ScytheSqlError
from scythe_sql.extract import extract_tar_gz, extract_zip
from scythe_sql.platform_resolver import UnsupportedPlatformError, is_musl_linux, resolve_target
from scythe_sql.proxy import resolve_ca_file
from scythe_sql.version_utils import assert_real_version

REPO = "https://github.com/Goldziher/scythe"


class DownloadError(ScytheSqlError):
    """Raised when fetching a release asset fails."""


def _fetch(url: str) -> bytes:
    ca_file = resolve_ca_file(dict(os.environ))
    context = None
    if ca_file:
        import ssl

        context = ssl.create_default_context(cafile=ca_file)

    # urllib.request honours HTTPS_PROXY/HTTP_PROXY/NO_PROXY natively via
    # the environment (ProxyHandler is installed by default in build_opener()).
    request = urllib.request.Request(url, headers={"User-Agent": "scythe-sql-installer"})  # noqa: S310
    try:
        with urllib.request.urlopen(request, context=context) as response:  # noqa: S310
            if response.status != 200:
                raise DownloadError(f"scythe-sql: failed to download {url}: HTTP {response.status}")
            return response.read()
    except urllib.error.URLError as exc:
        raise DownloadError(f"scythe-sql: failed to download {url}: {exc}") from exc


def is_usable_cached_binary(path: Path) -> bool:
    """Reports whether `path` holds a complete, runnable cached binary.

    Existence is not enough. A cache entry written by an older, non-atomic
    version of this package -- or by any interrupted write -- can be an empty or
    truncated file, and returning it would fail at exec time with an opaque
    error on every subsequent run. A zero-length file is never a valid binary,
    so treat it as absent and re-download over it.
    """
    try:
        stat_result = path.stat()
    except OSError:
        return False
    return stat.S_ISREG(stat_result.st_mode) and stat_result.st_size > 0


def ensure_binary(version: str, *, env: dict[str, str] | None = None) -> Path:
    """Ensures the pinned scythe binary is present locally, downloading it if needed.

    Returns the path to a usable `scythe` binary.
    """
    env = dict(os.environ) if env is None else env
    assert_real_version(version)

    if env.get("SCYTHE_BINARY"):
        return Path(env["SCYTHE_BINARY"])

    resolved = resolve_target(
        sys_platform=sys.platform,
        machine=platform.machine(),
        is_musl=sys.platform.startswith("linux") and is_musl_linux(platform.libc_ver()),
    )

    dest_path = cached_binary_path(
        env=env,
        sys_platform=sys.platform,
        home=default_home(),
        version=version,
        binary_name=resolved.binary_name,
    )

    if is_usable_cached_binary(dest_path):
        return dest_path

    from scythe_sql.preinstalled import has_matching_path_binary

    if has_matching_path_binary(version):
        return Path("scythe")

    if env.get("SCYTHE_SKIP_DOWNLOAD") == "1":
        raise DownloadError(
            f"scythe-sql: SCYTHE_SKIP_DOWNLOAD=1 set but no cached or PATH binary matching {version} found."
        )

    asset_name = f"scythe-{resolved.target}.{resolved.archive_ext}"
    checksums_name = f"scythe_{version}_checksums.txt"
    base_url = f"{REPO}/releases/download/v{version}"
    asset_url = f"{base_url}/{asset_name}"
    checksums_url = f"{base_url}/{checksums_name}"

    print(f"scythe-sql: downloading scythe {version} for {resolved.target}...", file=sys.stderr)  # noqa: T201

    checksums_text = _fetch(checksums_url).decode("utf-8")
    checksums = parse_checksums(checksums_text)
    expected = expected_checksum(checksums, asset_name, checksums_url)

    asset_bytes = _fetch(asset_url)
    verify_checksum(asset_bytes, expected, asset_url)

    # Both extractors stage into a temp file, set the mode, then rename, so
    # dest_path only ever appears complete and executable.
    if resolved.archive_ext == "zip":
        extract_zip(asset_bytes, resolved.binary_name, dest_path)
    else:
        extract_tar_gz(asset_bytes, resolved.binary_name, dest_path)

    print(f"scythe-sql: installed scythe {version} to {dest_path}", file=sys.stderr)  # noqa: T201
    return dest_path


__all__ = ["DownloadError", "ScytheSqlError", "UnsupportedPlatformError", "ensure_binary", "is_usable_cached_binary"]
