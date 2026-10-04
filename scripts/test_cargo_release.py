import os
import subprocess
import tempfile
import unittest
from pathlib import Path

WRAPPER = Path(__file__).with_name("cargo-release")


class CargoReleaseTests(unittest.TestCase):
    def assert_dispatch(self, platform: str, arguments: list[str], expected: list[str]) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            log = bin_dir / "cargo-arguments"
            cargo = bin_dir / "cargo"
            cargo.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$CARGO_TEST_LOG"\n')
            cargo.chmod(0o755)
            uname = bin_dir / "uname"
            uname.write_text(f'#!/bin/sh\nprintf "%s\\n" "{platform}"\n')
            uname.chmod(0o755)
            environment = os.environ.copy()
            environment["PATH"] = f"{bin_dir}:{environment['PATH']}"
            environment["CARGO_TEST_LOG"] = str(log)

            subprocess.run([str(WRAPPER), *arguments], check=True, env=environment)

            self.assertEqual(expected, log.read_text().splitlines())

    def test_darwin_zigbuild_with_equals_target_uses_cargo_build(self) -> None:
        self.assert_dispatch(
            "Darwin",
            ["zigbuild", "--release", "--target=aarch64-apple-darwin"],
            ["build", "--release", "--target=aarch64-apple-darwin"],
        )

    def test_darwin_zigbuild_with_separate_target_uses_cargo_build(self) -> None:
        self.assert_dispatch(
            "Darwin",
            ["zigbuild", "--release", "--target", "x86_64-apple-darwin"],
            ["build", "--release", "--target", "x86_64-apple-darwin"],
        )

    def test_linux_zigbuild_is_unchanged(self) -> None:
        self.assert_dispatch(
            "Linux",
            ["zigbuild", "--release", "--target", "aarch64-apple-darwin"],
            ["zigbuild", "--release", "--target", "aarch64-apple-darwin"],
        )

    def test_darwin_non_apple_target_is_unchanged(self) -> None:
        self.assert_dispatch(
            "Darwin",
            ["zigbuild", "--release", "--target=x86_64-unknown-linux-gnu"],
            ["zigbuild", "--release", "--target=x86_64-unknown-linux-gnu"],
        )


if __name__ == "__main__":
    unittest.main()
