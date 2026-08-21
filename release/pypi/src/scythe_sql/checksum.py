"""Checksum parsing and verification for goreleaser checksum files."""

import hashlib
import re

from scythe_sql.errors import ScytheSqlError

_ROW_RE = re.compile(r"^([0-9a-fA-F]{64})\s+\*?(\S+)$")


def parse_checksums(contents: str) -> dict[str, str]:
    """Parses a `scythe_<version>_checksums.txt` file into filename -> lowercase hex sha256."""
    result: dict[str, str] = {}
    for raw_line in contents.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        match = _ROW_RE.match(line)
        if not match:
            continue
        digest, filename = match.groups()
        result[filename] = digest.lower()
    return result


class MissingChecksumError(ScytheSqlError):
    """Raised when the checksums file has no row for the requested asset."""


def expected_checksum(checksums: dict[str, str], asset_filename: str, checksums_url: str) -> str:
    """Looks up the expected checksum for `asset_filename`.

    Raises :class:`MissingChecksumError` when the row is missing -- a
    missing row means the release is malformed and verification must not be
    silently skipped.
    """
    expected = checksums.get(asset_filename)
    if expected is None:
        raise MissingChecksumError(
            f"scythe-sql: no checksum entry for '{asset_filename}' in {checksums_url}. "
            "The release appears malformed; refusing to install without verification."
        )
    return expected


def sha256_hex(data: bytes) -> str:
    """Computes the lowercase hex sha256 digest of `data`."""
    return hashlib.sha256(data).hexdigest()


class ChecksumMismatchError(ScytheSqlError):
    """Raised when a downloaded asset's checksum does not match the expected value."""


def verify_checksum(data: bytes, expected_hex: str, asset_url: str) -> None:
    """Verifies `data` against `expected_hex`, case-insensitively.

    Raises :class:`ChecksumMismatchError` naming both hashes and the source
    URL on mismatch.
    """
    actual = sha256_hex(data)
    if actual.lower() != expected_hex.lower():
        raise ChecksumMismatchError(
            f"scythe-sql: checksum mismatch for {asset_url}\n  expected: {expected_hex.lower()}\n  actual:   {actual}"
        )
