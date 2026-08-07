from __future__ import annotations

from pathlib import Path

import pytest
from scythe_sql import cli
from scythe_sql.checksum import ChecksumMismatchError, MissingChecksumError
from scythe_sql.download import DownloadError
from scythe_sql.errors import ScytheSqlError
from scythe_sql.extract import BinaryNotFoundError
from scythe_sql.platform_resolver import UnsupportedPlatformError
from scythe_sql.version_utils import PlaceholderVersionError

WRAPPER_ERRORS = [
    DownloadError,
    UnsupportedPlatformError,
    MissingChecksumError,
    ChecksumMismatchError,
    BinaryNotFoundError,
    PlaceholderVersionError,
]


@pytest.mark.parametrize("error_type", WRAPPER_ERRORS, ids=lambda error_type: error_type.__name__)
def test_every_wrapper_error_shares_the_package_base(error_type: type[Exception]) -> None:
    assert issubclass(error_type, ScytheSqlError)
    assert issubclass(error_type, RuntimeError)


@pytest.mark.parametrize("error_type", WRAPPER_ERRORS, ids=lambda error_type: error_type.__name__)
def test_main_prints_wrapper_errors_verbatim(
    error_type: type[Exception], monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    """These messages are pre-formatted; re-wrapping them double-prefixes the output."""
    message = "scythe-sql: checksum mismatch for https://example/asset"
    monkeypatch.setattr(cli, "ensure_binary", _raiser(error_type(message)))

    assert cli.main() == 1

    stderr = capsys.readouterr().err
    assert stderr == f"{message}\n"
    assert "failed to resolve the scythe binary" not in stderr


def test_main_wraps_unexpected_errors_with_context(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr(cli, "ensure_binary", _raiser(ValueError("boom")))

    assert cli.main() == 1
    assert capsys.readouterr().err == "scythe-sql: failed to resolve the scythe binary: boom\n"


def test_main_execs_the_resolved_binary_forwarding_argv(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[tuple[str, list[str]]] = []
    monkeypatch.setattr(cli, "ensure_binary", lambda _version: Path("/cache/scythe"))
    monkeypatch.setattr(cli.os, "execv", lambda path, argv: calls.append((path, argv)))
    monkeypatch.setattr(cli.sys, "argv", ["scythe", "generate", "--config", "scythe.toml"])

    cli.main()

    assert calls == [("/cache/scythe", ["/cache/scythe", "generate", "--config", "scythe.toml"])]


def test_main_reports_an_exec_failure(monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]) -> None:
    monkeypatch.setattr(cli, "ensure_binary", lambda _version: Path("/cache/scythe"))
    monkeypatch.setattr(cli.os, "execv", _raiser(OSError(13, "Permission denied")))

    assert cli.main() == 1
    assert "scythe-sql: failed to execute /cache/scythe" in capsys.readouterr().err


def _raiser(error: BaseException):  # noqa: ANN202 -- returns a throwaway stub of varying arity
    def _raise(*_args: object, **_kwargs: object) -> None:
        raise error

    return _raise
