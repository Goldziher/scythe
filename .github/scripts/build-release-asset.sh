#!/usr/bin/env bash
# Package, smoke-test and upload one release archive.
#
# Usage: build-release-asset.sh TRIPLE RELEASE_ID
#
# Runs on the target's own runner after `cargo build --release --target TRIPLE`.
# Packaging (including the pinned parser helpers) is delegated to
# scripts/package-release.sh, which refuses to produce a partial archive.
set -euo pipefail

if [ "$#" -ne 2 ] || [ -z "$1" ] || [ -z "$2" ]; then
  echo "usage: build-release-asset.sh TRIPLE RELEASE_ID" >&2
  exit 1
fi

triple=$1
release_id=$2
repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

case "$triple" in
*windows*)
  archive="scythe-${triple}.zip"
  content_type="application/zip"
  ;;
*)
  archive="scythe-${triple}.tar.gz"
  content_type="application/gzip"
  ;;
esac

./scripts/package-release.sh "$triple"

[ -s "$archive" ] || {
  echo "packaging did not produce $archive" >&2
  exit 1
}

# ~keep The archive is what users run, so smoke-test the extracted copy rather than the
# ~keep build-tree binary: it proves the members landed where the wrappers expect them.
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT HUP INT TERM
case "$archive" in
*.zip)
  # ~keep python3 -m zipfile, not unzip: the Windows runner's bash provides no
  # ~keep unzip, and bsdtar is not what `tar` resolves to there.
  python3 -m zipfile -e "$archive" "$scratch"
  binary="$scratch/scythe.exe"
  ;;
*)
  tar -xzf "$archive" -C "$scratch"
  binary="$scratch/scythe"
  ;;
esac

[ -x "$binary" ] || {
  echo "$archive does not contain an executable binary" >&2
  exit 1
}
"$binary" --version >/dev/null

case "$triple" in
*windows*)
  [ -f "$scratch/PARSER_HELPERS_UNAVAILABLE.txt" ] || {
    echo "$archive is missing PARSER_HELPERS_UNAVAILABLE.txt" >&2
    exit 1
  }
  ;;
*)
  for helper in scythe-pg15-parser scythe-pg18-parser; do
    [ -x "$scratch/$helper" ] || {
      echo "$archive is missing $helper" >&2
      exit 1
    }
    "$scratch/$helper" --version >/dev/null
  done
  [ -s "$scratch/pg-parser-manifest.json" ] || {
    echo "$archive is missing pg-parser-manifest.json" >&2
    exit 1
  }
  ;;
esac

bash .github/scripts/release-asset.sh upload "$release_id" "$archive" "$content_type"
echo "uploaded $archive to release $release_id"
