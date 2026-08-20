import copy
import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("release-canary.py")
SPEC = importlib.util.spec_from_file_location("release_canary", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"could not load {SCRIPT}")
release_canary = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_canary)


class ReleaseCanaryTests(unittest.TestCase):
    def test_repository_configuration_matches_goreleaser(self) -> None:
        targets = release_canary.validate(release_canary.load_config())
        self.assertEqual(5, len(targets))

    def test_rejects_a_maximum_below_the_observed_size(self) -> None:
        config = copy.deepcopy(release_canary.load_config())
        config["targets"][0]["max_bytes"] = config["targets"][0]["observed_bundled_bytes"] - 1

        with self.assertRaisesRegex(ValueError, "baseline < observed <= maximum"):
            release_canary.validate(config)

    def test_rejects_a_binary_above_the_target_maximum(self) -> None:
        target = {
            "triple": "test-target",
            "binary": "scythe",
            "baseline_bytes": 2,
            "observed_bundled_bytes": 3,
            "max_bytes": 4,
        }
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "test-target" / "release" / "scythe"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"12345")

            with self.assertRaisesRegex(ValueError, "maximum is 4 bytes"):
                release_canary.check_binary(target, Path(directory))


if __name__ == "__main__":
    unittest.main()
