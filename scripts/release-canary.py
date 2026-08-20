#!/usr/bin/env python3

import argparse
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
SIZE_CONFIG = ROOT / "release" / "duckdb-binary-sizes.json"
GORELEASER_CONFIG = ROOT / ".goreleaser.yaml"
EXPECTED_TARGET_COUNT = 5


def load_config() -> dict[str, Any]:
    return json.loads(SIZE_CONFIG.read_text(encoding="utf-8"))


def goreleaser_targets() -> list[str]:
    targets: list[str] = []
    in_targets = False
    targets_indent = 0
    for line in GORELEASER_CONFIG.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        indent = len(line) - len(line.lstrip())
        if stripped == "targets:":
            in_targets = True
            targets_indent = indent
            continue
        if not in_targets:
            continue
        if stripped and indent <= targets_indent:
            break
        if stripped.startswith("- "):
            targets.append(stripped.removeprefix("- "))
    return targets


def configured_targets(config: dict[str, Any]) -> list[dict[str, Any]]:
    targets = config.get("targets")
    if not isinstance(targets, list):
        raise ValueError("release size configuration must contain a targets list")
    return targets


def validate(config: dict[str, Any], selected_target: str | None = None) -> list[dict[str, Any]]:
    minimum_open_files = config.get("minimum_open_files")
    if not isinstance(minimum_open_files, int) or minimum_open_files < 4096:
        raise ValueError("minimum_open_files must be at least 4096")

    targets = configured_targets(config)
    triples = [target.get("triple") for target in targets]
    if len(targets) != EXPECTED_TARGET_COUNT or len(set(triples)) != EXPECTED_TARGET_COUNT:
        raise ValueError("release canary must define exactly five unique targets")
    if triples != goreleaser_targets():
        raise ValueError("release canary targets must match .goreleaser.yaml in order")

    for target in targets:
        triple = target.get("triple")
        binary = target.get("binary")
        baseline = target.get("baseline_bytes")
        observed = target.get("observed_bundled_bytes")
        maximum = target.get("max_bytes")
        if not isinstance(triple, str) or not isinstance(binary, str):
            raise ValueError("each release target requires string triple and binary fields")
        if not all(isinstance(value, int) and value > 0 for value in (baseline, observed, maximum)):
            raise ValueError(f"{triple} size fields must be positive integers")
        if not baseline < observed <= maximum:
            raise ValueError(f"{triple} sizes must satisfy baseline < observed <= maximum")

    if selected_target is not None and selected_target not in triples:
        raise ValueError(f"unsupported release target: {selected_target}")
    return targets


def target_config(targets: list[dict[str, Any]], triple: str) -> dict[str, Any]:
    return next(target for target in targets if target["triple"] == triple)


def check_binary(target: dict[str, Any], target_dir: Path) -> None:
    triple = target["triple"]
    binary = target_dir / triple / "release" / target["binary"]
    if not binary.is_file():
        raise ValueError(f"release binary does not exist: {binary}")
    actual = binary.stat().st_size
    maximum = target["max_bytes"]
    baseline = target["baseline_bytes"]
    if actual > maximum:
        raise ValueError(f"{triple} binary is {actual} bytes; maximum is {maximum} bytes")
    delta = actual - baseline
    percent = delta * 100 / baseline
    print(f"{triple}: {actual} bytes, {delta:+d} bytes ({percent:+.1f}%) from the pre-DuckDB baseline")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate bundled-DuckDB release binaries")
    subparsers = parser.add_subparsers(dest="command", required=True)

    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("--target")
    subparsers.add_parser("matrix")
    subparsers.add_parser("targets")

    check_parser = subparsers.add_parser("check")
    check_parser.add_argument("--target", required=True)
    check_parser.add_argument("--target-dir", type=Path, required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    config = load_config()
    selected_target = getattr(args, "target", None)
    targets = validate(config, selected_target)

    if args.command == "validate":
        print("release canary configuration matches all five GoReleaser targets")
    elif args.command == "matrix":
        print(json.dumps([target["triple"] for target in targets], separators=(",", ":")))
    elif args.command == "targets":
        print("\n".join(target["triple"] for target in targets))
    else:
        check_binary(target_config(targets, args.target), args.target_dir)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"release canary: {error}") from error
