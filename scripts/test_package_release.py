#!/usr/bin/env python3
"""Tests for scripts/package-release.sh.

No compiler, network or real release build is needed: the packager is driven through
--helpers-from with fixture helpers. Run with `python3 scripts/test_package_release.py`.
"""

import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
PACKAGER = SCRIPTS / "package-release.sh"

POSIX_MEMBERS = {
    "scythe",
    "scythe-pg15-parser",
    "scythe-pg18-parser",
    "pg-parser-manifest.json",
    "THIRD_PARTY_LICENSES.txt",
    "PARSER_PROTOCOL.md",
    "CHANGELOG.md",
    "LICENSE",
    "README.md",
}
WINDOWS_MEMBERS = {
    "scythe.exe",
    "PARSER_HELPERS_UNAVAILABLE.txt",
    "PARSER_PROTOCOL.md",
    "CHANGELOG.md",
    "LICENSE",
    "README.md",
}

DARWIN_ARM = "aarch64-apple-darwin"
DARWIN_X64 = "x86_64-apple-darwin"
WINDOWS = "x86_64-pc-windows-msvc"


def run(script: Path, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    merged = dict(os.environ)
    if env:
        merged.update(env)
    return subprocess.run(
        ["bash", str(script), *args],
        capture_output=True,
        text=True,
        check=False,
        env=merged,
    )


def has_zip_writer() -> bool:
    return any(shutil.which(tool) for tool in ("7z", "zip", "powershell"))


class PackageReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.work = Path(self._tmp.name)
        self.target_dir = self.work / "target"
        self.out_dir = self.work / "out"
        self.out_dir.mkdir()

    def _write_binary(self, triple: str, name: str) -> Path:
        binary = self.target_dir / triple / "release" / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"#!/bin/sh\nexit 0\n")
        binary.chmod(0o755)
        return binary

    def _write_helpers(self, *, omit: str = "") -> Path:
        helpers = self.work / "helpers"
        helpers.mkdir(exist_ok=True)
        for name in ("scythe-pg15-parser", "scythe-pg18-parser"):
            if name == omit:
                continue
            binary = helpers / name
            binary.write_bytes(b"#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
        if omit != "THIRD_PARTY_LICENSES.txt":
            (helpers / "THIRD_PARTY_LICENSES.txt").write_text("libpg_query notice\n", encoding="utf-8")
        if omit != "pg-parser-manifest.json":
            manifest = {"protocol": 1, "target": DARWIN_ARM, "helpers": []}
            (helpers / "pg-parser-manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        return helpers

    def _env(self) -> dict[str, str]:
        return {"CARGO_TARGET_DIR": str(self.target_dir)}

    def test_packages_one_flat_posix_archive(self) -> None:
        binary = self._write_binary(DARWIN_ARM, "scythe")
        helpers = self._write_helpers()

        result = run(
            PACKAGER,
            DARWIN_ARM,
            "--helpers-from",
            str(helpers),
            "--out-dir",
            str(self.out_dir),
            env=self._env(),
        )

        self.assertEqual(0, result.returncode, result.stderr)
        archive = self.out_dir / f"scythe-{DARWIN_ARM}.tar.gz"
        self.assertEqual([archive], sorted(self.out_dir.iterdir()))

        with tarfile.open(archive) as tar:
            names = tar.getnames()
            self.assertEqual(POSIX_MEMBERS, set(names))
            for name in names:
                self.assertNotIn("/", name, f"{name} is not flat at the archive root")
            self.assertEqual(binary.read_bytes(), tar.extractfile("scythe").read())

    def test_refuses_to_ship_an_archive_missing_a_helper(self) -> None:
        self._write_binary(DARWIN_ARM, "scythe")
        helpers = self._write_helpers(omit="pg-parser-manifest.json")

        result = run(
            PACKAGER,
            DARWIN_ARM,
            "--helpers-from",
            str(helpers),
            "--out-dir",
            str(self.out_dir),
            env=self._env(),
        )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("pg-parser-manifest.json", result.stderr)
        self.assertEqual([], list(self.out_dir.iterdir()))

    def test_refuses_to_package_a_missing_binary(self) -> None:
        helpers = self._write_helpers()

        result = run(
            PACKAGER,
            DARWIN_X64,
            "--helpers-from",
            str(helpers),
            "--out-dir",
            str(self.out_dir),
            env=self._env(),
        )

        self.assertNotEqual(0, result.returncode)
        self.assertIn(str(self.target_dir / DARWIN_X64 / "release" / "scythe"), result.stderr)
        self.assertEqual([], list(self.out_dir.iterdir()))

    def test_rejects_an_unknown_triple(self) -> None:
        result = run(PACKAGER, "sparc-unknown-solaris", "--out-dir", str(self.out_dir), env=self._env())

        self.assertNotEqual(0, result.returncode)
        self.assertIn("unsupported target triple", result.stderr)

    @unittest.skipUnless(has_zip_writer(), "no zip writer (7z, zip or powershell) on PATH")
    def test_windows_archive_carries_the_unavailable_note(self) -> None:
        self._write_binary(WINDOWS, "scythe.exe")

        result = run(PACKAGER, WINDOWS, "--out-dir", str(self.out_dir), env=self._env())

        self.assertEqual(0, result.returncode, result.stderr)
        archive = self.out_dir / f"scythe-{WINDOWS}.zip"
        self.assertEqual([archive], sorted(self.out_dir.iterdir()))

        with zipfile.ZipFile(archive) as zipped:
            self.assertEqual(WINDOWS_MEMBERS, set(zipped.namelist()))
            note = zipped.read("PARSER_HELPERS_UNAVAILABLE.txt").decode()
        self.assertIn(WINDOWS, note)
        self.assertIn("libpg_query", note)
        self.assertIn("POSIX socket headers", note)


if __name__ == "__main__":
    unittest.main()
