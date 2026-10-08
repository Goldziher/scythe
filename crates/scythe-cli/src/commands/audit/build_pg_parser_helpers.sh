#!/bin/sh
set -eu

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo 'usage: build_pg_parser_helpers.sh OUTPUT_DIRECTORY' >&2
  exit 1
fi

output_dir=$1
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
work_dir=$(mktemp -d)
[ -n "$work_dir" ] && [ -d "$work_dir" ] || exit 1
trap 'rm -rf -- "$work_dir"' EXIT HUP INT TERM
mkdir -p -- "$output_dir"

# ~keep Cross-compilation drivers are overridable so the release pipeline can target each
# ~keep GoReleaser triple; the defaults are the host tools used by local development.
: "${SCYTHE_PG_CC:=cc}"
: "${SCYTHE_PG_MAKE:=make}"
export SCYTHE_PG_CC SCYTHE_PG_MAKE

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

# A pinned patch is applied and its markers asserted, so a patch that silently fails to
# apply (fuzzy match, reversed, wrong file) aborts the build instead of producing an
# unpatched helper that then fails the release controls.
build_one() {
  major=$1
  release=$2
  expected_sha=$3
  patch_file=$4
  patch_sha=""
  archive="$work_dir/libpg_query-$release.tar.gz"
  source_dir="$work_dir/libpg_query-$release"
  url="https://github.com/pganalyze/libpg_query/archive/refs/tags/$release.tar.gz"

  curl --fail --location --silent --show-error "$url" --output "$archive"
  actual_sha=$(sha256_file "$archive")
  if [ "$actual_sha" != "$expected_sha" ]; then
    echo "libpg_query $release SHA256 mismatch" >&2
    exit 1
  fi

  tar -xzf "$archive" -C "$work_dir"
  if [ -n "$patch_file" ]; then
    patch_sha=$(sha256_file "$patch_file")
    (cd "$source_dir" && patch -p1 --batch --forward <"$patch_file")
    if ! grep -q 'SCYTHE PATCH' "$source_dir/src/postgres/src_backend_catalog_namespace.c" ||
      ! grep -q 'PLPGSQL_DTYPE_PROMISE' "$source_dir/src/pg_query_json_plpgsql.c"; then
      echo "libpg_query $release patch did not apply" >&2
      exit 1
    fi
  fi

  if [ "$major" = 15 ] && [ "$(uname -s)" = Darwin ]; then
    "$SCYTHE_PG_MAKE" -C "$source_dir" CFLAGS=-DHAVE_STRCHRNUL libpg_query.a
  else
    "$SCYTHE_PG_MAKE" -C "$source_dir" libpg_query.a
  fi
  "$SCYTHE_PG_CC" -std=c11 -Wall -Wextra -Werror -I "$source_dir" \
    "-DSCYTHE_LIBPG_QUERY_RELEASE=\"$release\"" \
    "$script_dir/pg_parser_helper.c" "$source_dir/libpg_query.a" \
    -pthread -lm -o "$output_dir/scythe-pg$major-parser"

  # ~keep Emitted, not global: the manifest below must record the patch actually used.
  printf '%s %s\n' "$patch_sha" "$major" >>"$work_dir/patches.txt"
}

: >"$work_dir/patches.txt"
build_one 15 15-4.2.4 d0ace0bff40e5daa99e32753a22a7558f9b2207e89260f00171fc50723b831dd ""
build_one 18 18.1.0 2d3486cf6a9d3955b53e66235db39d62b54216c820cd392ab66dc842c5b1316d \
  "$script_dir/patches/libpg_query-18.1.0-plpgsql-json.patch"

pg15_binary_sha=$(sha256_file "$output_dir/scythe-pg15-parser")
pg18_binary_sha=$(sha256_file "$output_dir/scythe-pg18-parser")
pg18_patch_sha=$(awk '$2 == 18 { print $1 }' "$work_dir/patches.txt")
printf '{"protocol":1,"helpers":[{"pg_major":15,"pg_version":"15.1","parser_release":"15-4.2.4","source_sha256":"d0ace0bff40e5daa99e32753a22a7558f9b2207e89260f00171fc50723b831dd","patch_sha256":null,"binary_sha256":"%s"},{"pg_major":18,"pg_version":"18.6","parser_release":"18.1.0","source_sha256":"2d3486cf6a9d3955b53e66235db39d62b54216c820cd392ab66dc842c5b1316d","patch_sha256":"%s","binary_sha256":"%s"}]}\n' \
  "$pg15_binary_sha" "$pg18_patch_sha" "$pg18_binary_sha" >"$output_dir/pg-parser-manifest.json"
