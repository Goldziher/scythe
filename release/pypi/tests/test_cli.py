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


def test_main_on_windows_waits_for_the_child_and_returns_its_exit_code(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Windows has no real exec.

    `os.execv` there spawns a new process and kills this one, so the shell sees
    exit code 0 immediately while scythe is still running -- every failure would
    be reported as success and `scythe check` could never fail a Windows CI job.
    The release smoke test only catches this on an actual Windows runner, which
    is how it reached a tagged release.
    """
    import subprocess

    calls: list[list[str]] = []
    binary = Path("/cache/scythe")
    monkeypatch.setattr(cli.os, "name", "nt")
    monkeypatch.setattr(cli, "ensure_binary", lambda _version: binary)
    monkeypatch.setattr(cli.sys, "argv", ["scythe", "this-is-not-a-real-subcommand"])
    monkeypatch.setattr(cli.os, "execv", _raiser(AssertionError("must not exec on Windows")))
    monkeypatch.setattr(
        subprocess,
        "run",
        lambda argv, check: calls.append(argv) or subprocess.CompletedProcess(argv, 2),  # noqa: ARG005
    )

    # The child's status must come back verbatim -- 2, not 0 -- since a wrong
    # exit code here is the entire failure this test guards.
    assert cli.main() == 2
    # ~keep `str(binary)` rather than a literal: patching os.name to "nt" makes pathlib
    # render this path with backslashes even on a POSIX host.
    assert calls == [[str(binary), "this-is-not-a-real-subcommand"]]


def test_main_on_windows_reports_a_spawn_failure(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import subprocess

    binary = Path("/cache/scythe")
    monkeypatch.setattr(cli.os, "name", "nt")
    monkeypatch.setattr(cli, "ensure_binary", lambda _version: binary)
    monkeypatch.setattr(subprocess, "run", _raiser(OSError(13, "Permission denied")))

    assert cli.main() == 1
    assert f"scythe-sql: failed to execute {binary}" in capsys.readouterr().err


def _raiser(error: BaseException):  # noqa: ANN202 -- returns a throwaway stub of varying arity
    def _raise(*_args: object, **_kwargs: object) -> None:
        raise error

    return _raise
