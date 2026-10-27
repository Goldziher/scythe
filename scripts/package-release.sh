#!/usr/bin/env bash
# Build the archive one release target publishes, and nothing else.
#
# ~keep The members sit at the ARCHIVE ROOT (no leading directory), because that is what
# ~keep every consumer already expects: the Homebrew formula installs `scythe` straight out
# ~keep of the extracted archive and the pre-0.20.0 tarballs were laid out the same way.
# ~keep A Windows archive must never be produced without PARSER_HELPERS_UNAVAILABLE.txt: a
# ~keep user who extracts a zip with no helpers and no note reads the missing gate as a bug.
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: package-release.sh <target-triple> [--helpers-from <dir>] [--out-dir <dir>]

  <target-triple>       x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu
                        x86_64-apple-darwin | aarch64-apple-darwin
                        x86_64-pc-windows-msvc | x86_64-pc-windows-gnu
  --helpers-from <dir>  package a prepared parser-helper directory instead of building it
  --out-dir <dir>       write the archive here (default: the repository root)
EOF
  exit 2
}

fail() {
  echo "package-release: $*" >&2
  exit 1
}

[ "$#" -ge 1 ] || usage
triple=$1
shift

helpers_from=""
out_dir=""
while [ "$#" -gt 0 ]; do
  case "$1" in
  --helpers-from)
    [ "$#" -ge 2 ] || usage
    helpers_from=$2
    shift 2
    ;;
  --out-dir)
    [ "$#" -ge 2 ] || usage
    out_dir=$2
    shift 2
    ;;
  *)
    echo "package-release: unknown argument: $1" >&2
    usage
    ;;
  esac
done

bin_ext=""
archive_ext="tar.gz"
is_windows=false
case "$triple" in
x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu | x86_64-apple-darwin | aarch64-apple-darwin) ;;
# ~keep windows-gnu is what the release pipeline built before the msvc move; both triples
# ~keep package identically, and keeping the gnu arm working means a re-run of an older tag
# ~keep still produces an archive.
x86_64-pc-windows-msvc | x86_64-pc-windows-gnu)
  bin_ext=".exe"
  archive_ext="zip"
  is_windows=true
  ;;
*) fail "unsupported target triple: $triple" ;;
esac

script_dir=$(cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
parser_audit_dir="$repo_root/crates/scythe-cli/src/commands/audit"

cargo_target_dir=${CARGO_TARGET_DIR:-target}
case "$cargo_target_dir" in
/*) ;;
*) cargo_target_dir="$repo_root/$cargo_target_dir" ;;
esac
binary="$cargo_target_dir/$triple/release/scythe$bin_ext"
[ -f "$binary" ] || fail "release binary not found at $binary (build $triple first, or set CARGO_TARGET_DIR)"

if [ -z "$out_dir" ]; then
  out_dir="$repo_root"
fi
mkdir -p -- "$out_dir"
# ~keep Resolved against $PWD rather than `cd`-ing, so a relative --out-dir is unambiguous in
# ~keep the message printed at the end and in the zip writers that run from the staging dir.
case "$out_dir" in
/*) ;;
*) out_dir="$PWD/$out_dir" ;;
esac
archive="$out_dir/scythe-$triple.$archive_ext"

members=()
if [ "$is_windows" = true ]; then
  [ -z "$helpers_from" ] ||
    echo "package-release: ignoring --helpers-from for $triple: this archive carries no parser helpers" >&2
  members=("scythe.exe" "PARSER_HELPERS_UNAVAILABLE.txt")
else
  members=("scythe" "scythe-pg15-parser" "scythe-pg18-parser" "pg-parser-manifest.json" "THIRD_PARTY_LICENSES.txt")
fi
members+=("PARSER_PROTOCOL.md" "CHANGELOG.md" "LICENSE" "README.md")

work_dir=$(mktemp -d)
[ -n "$work_dir" ] && [ -d "$work_dir" ] || fail "could not create a staging directory"
trap 'rm -rf -- "$work_dir"' EXIT HUP INT TERM
staging="$work_dir/staging"
mkdir -p -- "$staging"

cp -- "$binary" "$staging/scythe$bin_ext"

copy_member() {
  [ -f "$1" ] || fail "missing required source file: $1"
  cp -- "$1" "$staging/$2"
}

copy_member "$parser_audit_dir/PARSER_PROTOCOL.md" "PARSER_PROTOCOL.md"
copy_member "$repo_root/CHANGELOG.md" "CHANGELOG.md"
copy_member "$repo_root/LICENSE" "LICENSE"
copy_member "$repo_root/README.md" "README.md"

if [ "$is_windows" = true ]; then
  # ~keep Verbatim the note `task pg:parsers:all` writes, so the archive and the build tree
  # ~keep explain the absent helpers the same way. Only the target name is filled in.
  {
    printf '%s\n' \
      "The PostgreSQL parser helpers are not built for $triple: upstream" \
      'libpg_query 15 and 18 include POSIX socket headers the Windows libc does' \
      'not provide. scythe audit --cluster-scope reports the helpers as' \
      'unavailable on this platform; run the gate on Linux or macOS.'
  } >"$staging/PARSER_HELPERS_UNAVAILABLE.txt"
elif [ -n "$helpers_from" ]; then
  [ -d "$helpers_from" ] || fail "helpers directory not found: $helpers_from"
  cp -R -- "$helpers_from/." "$staging/"
else
  # ~keep No triple argument: a native build uses the host toolchain, and passing a triple
  # ~keep would send the script down its zig cross-compile path and produce foreign helpers.
  SCYTHE_PARSER_HELPER_TARGET="$triple" \
    sh "$parser_audit_dir/build_pg_parser_helpers.sh" "$staging"
fi

missing=()
for member in "${members[@]}"; do
  [ -f "$staging/$member" ] || missing+=("$member")
done
if [ "${#missing[@]}" -ne 0 ]; then
  fail "refusing to ship a partial archive, missing: ${missing[*]}"
fi

create_zip() {
  if command -v 7z >/dev/null 2>&1; then
    (cd "$staging" && 7z a -tzip "$archive" "${members[@]}" >/dev/null)
  elif command -v zip >/dev/null 2>&1; then
    (cd "$staging" && zip -q "$archive" "${members[@]}")
  elif command -v powershell >/dev/null 2>&1; then
    local member_list
    member_list=$(printf "'%s'," "${members[@]}")
    member_list=${member_list%,}
    (cd "$staging" && powershell -NoProfile -Command \
      "Compress-Archive -Path $member_list -DestinationPath '$archive' -Force")
  else
    fail "no zip writer available (need 7z, zip or powershell)"
  fi
}

# ~keep bsdtar (macOS) emits an AppleDouble `._name` member for every file that carries
# ~keep extended attributes, which would drop junk next to the binary in the tarball. GNU tar
# ~keep ignores this variable, so setting it is correct for every release runner.
COPYFILE_DISABLE=1
export COPYFILE_DISABLE

rm -f -- "$archive"
if [ "$archive_ext" = "zip" ]; then
  create_zip
else
  tar -czf "$archive" -C "$staging" "${members[@]}"
fi

echo "created $archive"
