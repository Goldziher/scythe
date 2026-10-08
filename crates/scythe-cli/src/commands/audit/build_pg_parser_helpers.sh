#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ] || [ -z "$1" ]; then
  echo 'usage: build_pg_parser_helpers.sh OUTPUT_DIRECTORY [RUST_TARGET_TRIPLE]' >&2
  exit 1
fi

output_dir=$1
rust_target=${2:-}
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
work_dir=$(mktemp -d)
[ -n "$work_dir" ] && [ -d "$work_dir" ] || exit 1
trap 'rm -rf -- "$work_dir"' EXIT HUP INT TERM
mkdir -p -- "$output_dir"

# ~keep The helpers are C programs built per release target. Native builds use the
# ~keep host toolchain; cross builds use zig (zig cc / zig ar), which is why the
# ~keep compiler and archiver are overridable here.
target_os=host
zig_target=""
if [ -n "$rust_target" ]; then
  case "$rust_target" in
  x86_64-unknown-linux-gnu) zig_target=x86_64-linux-gnu; target_os=linux ;;
  aarch64-unknown-linux-gnu) zig_target=aarch64-linux-gnu; target_os=linux ;;
  x86_64-apple-darwin) zig_target=x86_64-macos; target_os=macos ;;
  aarch64-apple-darwin) zig_target=aarch64-macos; target_os=macos ;;
  x86_64-pc-windows-gnu)
    # ~keep Upstream libpg_query 15/18 does not build for windows-gnu: its vendored
    # ~keep PostgreSQL headers include POSIX socket headers the Windows libc lacks.
    # ~keep Fail loudly rather than emit an archive with no helpers.
    echo "parser helpers cannot be built for $rust_target (unsupported by libpg_query)" >&2
    exit 1
    ;;
  *)
    echo "unsupported parser-helper target: $rust_target" >&2
    exit 1
    ;;
  esac
fi

if [ -n "$zig_target" ]; then
  # ~keep Unquoted on purpose: the compiler command is two words (`zig cc -target X`).
  cc_command="zig cc -target $zig_target"
else
  cc_command="cc"
fi

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

# ~keep Binary distribution must carry the copyright and license text of every
# ~keep component linked into the helper; the tarballs are downloaded, not vendored,
# ~keep so the notices are collected here for the release materials.
licenses_file="$output_dir/THIRD_PARTY_LICENSES.txt"
append_license() {
  label=$1
  file=$2
  [ -f "$file" ] || return 0
  {
    printf '\n===== %s =====\n\n' "$label"
    if [ "$(basename "$file")" = "xxhash.h" ]; then
      # ~keep xxHash ships its BSD-2 notice in the header comment, not a LICENSE file.
      sed -n '1,32p' "$file"
    else
      cat "$file"
    fi
  } >>"$licenses_file"
}

collect_licenses() {
  release=$1
  source_dir=$2
  printf '### libpg_query %s (PostgreSQL %s)\n' "$release" "$3" >>"$licenses_file"
  append_license "libpg_query" "$source_dir/LICENSE"
  append_license "PostgreSQL" "$source_dir/src/postgres/COPYRIGHT"
  append_license "upb" "$source_dir/vendor/upb/LICENSE"
  append_license "utf8_range" "$source_dir/vendor/upb/third_party/utf8_range/LICENSE"
  append_license "protobuf-c" "$source_dir/vendor/protobuf-c/LICENSE"
  append_license "xxHash" "$source_dir/vendor/xxhash/xxhash.h"
}


# A pinned patch is applied and its markers asserted, so a patch that silently
# fails to apply (fuzzy match, reversed, wrong file) aborts the build instead of
# producing an unpatched helper that then fails the release controls.
build_one() {
  major=$1
  release=$2
  expected_sha=$3
  patch_file=$4
  pg_version=$5
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

  # ~keep libpg_query 15 compiles its own strchrnul fallback unless HAVE_STRCHRNUL is
  # ~keep defined. glibc and modern macOS libc provide it (defining the macro avoids a
  # ~keep duplicate-symbol clash); the windows-gnu libc does not, so it needs the fallback.
  define_strchrnul=0
  if [ "$major" = 15 ]; then
    case "$target_os" in
    windows) define_strchrnul=0 ;;
    host)
      case "$(uname -s)" in
      MINGW* | MSYS* | CYGWIN*) define_strchrnul=0 ;;
      *) define_strchrnul=1 ;;
      esac
      ;;
    *) define_strchrnul=1 ;;
    esac
  fi
  # ~keep 15 and 18 spell the archiver differently: 15 hardcodes `AR := $(AR) rs`
  # ~keep (which a command-line AR overrides, dropping the `rs`), 18 uses a separate
  # ~keep ARFLAGS. Override AR only for zig, with the operation the Makefile expects.
  make_ar=""
  if [ -n "$zig_target" ]; then
    if [ "$major" = 15 ]; then
      make_ar="zig ar rs"
    else
      make_ar="zig ar"
    fi
  fi
  if [ -n "$make_ar" ]; then
    if [ "$define_strchrnul" = 1 ]; then
      make -C "$source_dir" CC="$cc_command" AR="$make_ar" CFLAGS=-DHAVE_STRCHRNUL libpg_query.a
    else
      make -C "$source_dir" CC="$cc_command" AR="$make_ar" libpg_query.a
    fi
  elif [ "$define_strchrnul" = 1 ]; then
    make -C "$source_dir" CC="$cc_command" CFLAGS=-DHAVE_STRCHRNUL libpg_query.a
  else
    make -C "$source_dir" CC="$cc_command" libpg_query.a
  fi
  # shellcheck disable=SC2086
  $cc_command -std=c11 -Wall -Wextra -Werror -I "$source_dir" \
    "-DSCYTHE_LIBPG_QUERY_RELEASE=\"$release\"" \
    "$script_dir/pg_parser_helper.c" "$source_dir/libpg_query.a" \
    -pthread -lm -o "$output_dir/scythe-pg$major-parser"

  # ~keep Emitted, not global: the manifest below must record the patch actually used.
  printf '%s %s\n' "$patch_sha" "$major" >>"$work_dir/patches.txt"
  collect_licenses "$release" "$source_dir" "$pg_version"
}

: >"$work_dir/patches.txt"
: >"$licenses_file"
build_one 15 15-4.2.4 d0ace0bff40e5daa99e32753a22a7558f9b2207e89260f00171fc50723b831dd "" 15.1
build_one 18 18.1.0 2d3486cf6a9d3955b53e66235db39d62b54216c820cd392ab66dc842c5b1316d \
  "$script_dir/patches/libpg_query-18.1.0-plpgsql-json.patch" 18.6

pg15_binary_sha=$(sha256_file "$output_dir/scythe-pg15-parser")
pg18_binary_sha=$(sha256_file "$output_dir/scythe-pg18-parser")
pg18_patch_sha=$(awk '$2 == 18 { print $1 }' "$work_dir/patches.txt")
printf '{"protocol":1,"target":"%s","helpers":[{"pg_major":15,"pg_version":"15.1","parser_release":"15-4.2.4","source_sha256":"d0ace0bff40e5daa99e32753a22a7558f9b2207e89260f00171fc50723b831dd","patch_sha256":null,"binary_sha256":"%s"},{"pg_major":18,"pg_version":"18.6","parser_release":"18.1.0","source_sha256":"2d3486cf6a9d3955b53e66235db39d62b54216c820cd392ab66dc842c5b1316d","patch_sha256":"%s","binary_sha256":"%s"}]}\n' \
  "${rust_target:-native}" "$pg15_binary_sha" "$pg18_patch_sha" "$pg18_binary_sha" >"$output_dir/pg-parser-manifest.json"
