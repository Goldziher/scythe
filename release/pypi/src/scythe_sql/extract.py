"""Archive extraction: locate and unpack the scythe binary by basename.

The archive also contains LICENSE/README (goreleaser default), so callers
must not assume a fixed path inside the archive -- search by basename.
"""

from __future__ import annotations

import io
import os
import stat
import tarfile
import zipfile
from pathlib import Path

from scythe_sql.errors import ScytheSqlError


class BinaryNotFoundError(ScytheSqlError):
    """Raised when the archive does not contain an entry matching the binary name."""


def find_binary_entry(entry_names: list[str], binary_name: str) -> str:
    """Finds the archive entry whose basename matches `binary_name`."""
    for name in entry_names:
        if name.rstrip("/").split("/")[-1] == binary_name:
            return name
    listing = ", ".join(entry_names) if entry_names else "(empty)"
    raise BinaryNotFoundError(
        f"scythe-sql: could not find '{binary_name}' inside the downloaded archive. Archive contained: {listing}"
    )


def write_binary_atomically(dest_path: Path, data: bytes) -> None:
    """Writes `data` to `dest_path` atomically, already marked executable.

    Writing straight to the cached path is not safe: a process killed mid-write,
    an OOM kill, or a full disk leaves a truncated file that the cache -- which
    keys only on the path -- then trusts forever, so every later run fails with
    an opaque exec error. Two concurrent installs sharing a cache can also
    interleave their writes into the same file.

    Staging into a sibling temp file and renaming makes the cached path either
    absent or complete, never partial, because `os.replace` is atomic within a
    filesystem (a sibling guarantees the same filesystem). The pid in the temp
    name keeps concurrent writers off each other's staging file; the rename
    itself makes the last writer win harmlessly, since both wrote verified
    bytes. The mode is set on the temp file so the binary is never observable
    at its final path without the executable bit.
    """
    dest_path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = dest_path.with_name(f".{dest_path.name}.{os.getpid()}.tmp")
    try:
        with tmp_path.open("wb") as handle:
            handle.write(data)
            handle.flush()
            # Without fsync the rename can land before the data on a crash,
            # recreating the truncated-file problem the staging file prevents.
            os.fsync(handle.fileno())
        if os.name != "nt":
            make_executable(tmp_path)
        os.replace(tmp_path, dest_path)
    finally:
        tmp_path.unlink(missing_ok=True)


def extract_tar_gz(data: bytes, binary_name: str, dest_path: Path) -> None:
    """Extracts the scythe binary from a `.tar.gz` archive to `dest_path`."""
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        names = [member.name for member in tar.getmembers() if member.isfile()]
        entry_name = find_binary_entry(names, binary_name)
        member = tar.getmember(entry_name)
        extracted = tar.extractfile(member)
        if extracted is None:
            raise BinaryNotFoundError(f"scythe-sql: archive entry '{entry_name}' is not a regular file")
        write_binary_atomically(dest_path, extracted.read())


def extract_zip(data: bytes, binary_name: str, dest_path: Path) -> None:
    """Extracts the scythe binary from a `.zip` archive to `dest_path`."""
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        names = [info.filename for info in zf.infolist() if not info.is_dir()]
        entry_name = find_binary_entry(names, binary_name)
        write_binary_atomically(dest_path, zf.read(entry_name))


def make_executable(path: Path) -> None:
    """chmod 0o755 on POSIX; a no-op is not needed on Windows since .exe requires no bit."""
    path.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH)
