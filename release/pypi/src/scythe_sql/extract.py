"""Archive extraction: locate and unpack the scythe binary by basename.

The archive also contains LICENSE/README (goreleaser default), so callers
must not assume a fixed path inside the archive -- search by basename.
"""

from __future__ import annotations

import io
import stat
import tarfile
import zipfile
from pathlib import Path


class BinaryNotFoundError(RuntimeError):
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


def extract_tar_gz(data: bytes, binary_name: str, dest_path: Path) -> None:
    """Extracts the scythe binary from a `.tar.gz` archive to `dest_path`."""
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        names = [member.name for member in tar.getmembers() if member.isfile()]
        entry_name = find_binary_entry(names, binary_name)
        member = tar.getmember(entry_name)
        extracted = tar.extractfile(member)
        if extracted is None:
            raise BinaryNotFoundError(f"scythe-sql: archive entry '{entry_name}' is not a regular file")
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        dest_path.write_bytes(extracted.read())


def extract_zip(data: bytes, binary_name: str, dest_path: Path) -> None:
    """Extracts the scythe binary from a `.zip` archive to `dest_path`."""
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        names = [info.filename for info in zf.infolist() if not info.is_dir()]
        entry_name = find_binary_entry(names, binary_name)
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        dest_path.write_bytes(zf.read(entry_name))


def make_executable(path: Path) -> None:
    """chmod 0o755 on POSIX; a no-op is not needed on Windows since .exe requires no bit."""
    path.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH)
