#!/usr/bin/env bash
#MISE description="Build and size-check one bundled-DuckDB release target on its own platform"
#USAGE arg "<target>" help="Rust target triple, e.g. x86_64-unknown-linux-gnu"
# shellcheck disable=SC2154 # usage_* are set by mise from the #USAGE lines
set -euo pipefail

export RELEASE_CANARY_TARGET="$usage_target"
export CARGO_TARGET_DIR=target/release-canary

python3 scripts/release-canary.py validate --target "$RELEASE_CANARY_TARGET"
# ~keep Native build per target, matching the publish workflow: each release
# ~keep archive is built on its own runner, so the canary must not depend on a
# ~keep cross-compiler the release no longer uses.
cargo build --release --package scythe-cli --target "$RELEASE_CANARY_TARGET"
python3 scripts/release-canary.py check --target "$RELEASE_CANARY_TARGET" --target-dir "$CARGO_TARGET_DIR"
# ~keep The gate's parser helpers ship in every POSIX archive, so the canary
# ~keep builds them for the target too; a break fails CI rather than the tag.
# ~keep windows-msvc carries the documented unavailable marker instead.
case "$RELEASE_CANARY_TARGET" in
*windows*) echo "release-canary: skipping parser helpers for $RELEASE_CANARY_TARGET" >&2 ;;
*)
  # ~keep No triple argument: the canary runs on the target's own platform now,
  # ~keep so the helpers build with the host compiler. Passing the triple would
  # ~keep select the script's zig cross-compilation path and fail.
  SCYTHE_PARSER_HELPER_TARGET="$RELEASE_CANARY_TARGET" \
    sh crates/scythe-cli/src/commands/audit/build_pg_parser_helpers.sh \
    "$CARGO_TARGET_DIR/pg-parsers/$RELEASE_CANARY_TARGET"
  ;;
esac
