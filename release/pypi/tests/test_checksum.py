from __future__ import annotations

import pytest
from scythe_sql.checksum import (
    ChecksumMismatchError,
    MissingChecksumError,
    expected_checksum,
    parse_checksums,
    sha256_hex,
    verify_checksum,
)

SAMPLE = (
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  "
    "scythe-x86_64-unknown-linux-gnu.tar.gz\n"
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  "
    "scythe-aarch64-apple-darwin.tar.gz\n"
    "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC  "
    "scythe-x86_64-pc-windows-gnu.zip\n"
)


def test_parse_checksums_parses_filename_to_lowercase_hash_rows() -> None:
    result = parse_checksums(SAMPLE)
    assert result["scythe-x86_64-unknown-linux-gnu.tar.gz"] == "a" * 64
    assert result["scythe-aarch64-apple-darwin.tar.gz"] == "b" * 64
    assert result["scythe-x86_64-pc-windows-gnu.zip"] == "c" * 64


def test_parse_checksums_skips_blank_lines() -> None:
    assert len(parse_checksums(f"\n{SAMPLE}\n\n")) == 3


def test_expected_checksum_returns_hash_for_known_asset() -> None:
    checksums = parse_checksums(SAMPLE)
    assert expected_checksum(checksums, "scythe-x86_64-unknown-linux-gnu.tar.gz", "url") == "a" * 64


def test_expected_checksum_raises_on_missing_row() -> None:
    checksums = parse_checksums(SAMPLE)
    with pytest.raises(MissingChecksumError, match=r"no checksum entry.*https://example/checksums\.txt"):
        expected_checksum(checksums, "scythe-riscv64-unknown-linux-gnu.tar.gz", "https://example/checksums.txt")


def test_sha256_hex_matches_known_digest() -> None:
    assert sha256_hex(b"") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"


def test_verify_checksum_passes_case_insensitively() -> None:
    data = b"hello"
    digest = sha256_hex(data)
    verify_checksum(data, digest.upper(), "https://example/asset.tar.gz")


def test_verify_checksum_raises_naming_both_hashes_and_url() -> None:
    data = b"hello"
    with pytest.raises(ChecksumMismatchError) as exc_info:
        verify_checksum(data, "0" * 64, "https://example/asset.tar.gz")
    message = str(exc_info.value)
    assert "https://example/asset.tar.gz" in message
    assert f"expected: {'0' * 64}" in message
    assert f"actual:   {sha256_hex(data)}" in message
