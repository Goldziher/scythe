import pytest
from scythe_sql.version_utils import (
    PlaceholderVersionError,
    assert_real_version,
    extract_version,
)


def test_extract_version_parses_real_scythe_version_output() -> None:
    assert extract_version("scythe 0.12.0\n") == "0.12.0"


def test_extract_version_handles_prerelease_and_build_metadata() -> None:
    assert extract_version("scythe 0.13.0-rc.1") == "0.13.0-rc.1"
    assert extract_version("scythe 0.13.0+abcdef") == "0.13.0+abcdef"


def test_extract_version_returns_none_when_no_version_token_present() -> None:
    assert extract_version("command not found") is None


def test_assert_real_version_raises_on_placeholder() -> None:
    with pytest.raises(PlaceholderVersionError, match="built incorrectly"):
        assert_real_version("0.0.0")


def test_assert_real_version_accepts_real_version() -> None:
    assert_real_version("0.13.0")
