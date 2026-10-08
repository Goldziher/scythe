#!/bin/sh
set -eu

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo 'usage: test_pg_parser_helpers.sh BINARY_DIRECTORY' >&2
  exit 1
fi

for major in 15 18; do
  binary="$1/scythe-pg$major-parser"
  [ -x "$binary" ] || exit 1
  "$binary" --version | grep -F "scythe-pg$major-parser protocol/1" >/dev/null

  local_ast=$(printf '%s\n' 'CREATE TABLE local_table (id integer);' | "$binary")
  printf '%s\n' "$local_ast" | jq -e --argjson major "$major" \
    '.protocol == 1 and .pg_major == $major and
         (.ast.stmts | length) == 1 and
         (.ast.stmts[0].stmt | has("CreateStmt"))' >/dev/null

  role_ast=$(printf '%s\n' 'CREATE ROLE planted_role;' | "$binary")
  printf '%s\n' "$role_ast" | jq -e \
    '(.ast.stmts | length) == 1 and
         (.ast.stmts[0].stmt | has("CreateRoleStmt"))' >/dev/null

  grant_ast=$(printf '%s\n' 'GRANT planted_role TO planted_member;' | "$binary")
  printf '%s\n' "$grant_ast" | jq -e \
    '(.ast.stmts | length) == 1 and
         (.ast.stmts[0].stmt | has("GrantRoleStmt"))' >/dev/null

  nested_ast=$(
    "$binary" --plpgsql <<'SQL'
DO $body$ BEGIN CREATE ROLE planted_nested; END $body$;
SQL
  )
  printf '%s\n' "$nested_ast" | jq -e \
    '.ast[0].PLpgSQL_function.action.PLpgSQL_stmt_block.body[0].PLpgSQL_stmt_execsql.sqlstmt.PLpgSQL_expr.query == "CREATE ROLE planted_nested"' >/dev/null

  dynamic_ast=$(
    "$binary" --plpgsql <<'SQL'
DO $body$ BEGIN EXECUTE format('CREATE ROLE %I', 'planted_dynamic'); END $body$;
SQL
  )
  printf '%s\n' "$dynamic_ast" | jq -e \
    '.ast[0].PLpgSQL_function.action.PLpgSQL_stmt_block.body[0].PLpgSQL_stmt_dynexecute.query.PLpgSQL_expr.query == "format(\u0027CREATE ROLE %I\u0027, \u0027planted_dynamic\u0027)"' >/dev/null

  if printf '%s\n' 'SELECT FROM;' | "$binary" >/dev/null 2>&1; then
    echo "PG$major helper accepted invalid SQL" >&2
    exit 1
  fi

  for sql in '' '-- comment only'; do
    if empty_error=$(printf '%s\n' "$sql" | "$binary" 2>&1 >/dev/null); then
      echo "PG$major helper accepted zero SQL statements" >&2
      exit 1
    fi
    case "$empty_error" in
    *'zero statements'*) ;;
    *)
      echo "PG$major helper failed for the wrong reason: $empty_error" >&2
      exit 1
      ;;
    esac
  done

  if cap_error=$(dd if=/dev/zero bs=1048576 count=65 2>/dev/null | "$binary" 2>&1 >/dev/null); then
    echo "PG$major helper accepted input above 64 MiB" >&2
    exit 1
  fi
  case "$cap_error" in
  *'exceeds 64 MiB'*) ;;
  *)
    echo "PG$major helper failed for the wrong reason: $cap_error" >&2
    exit 1
    ;;
  esac
done

manifest="$1/pg-parser-manifest.json"
[ -f "$manifest" ] || {
  echo 'parser helper package has no version manifest' >&2
  exit 1
}
jq -e '
    .protocol == 1 and
    (.helpers | length) == 2 and
    .helpers[0].pg_major == 15 and
    .helpers[0].pg_version == "15.1" and
    .helpers[0].parser_release == "15-4.2.4" and
    .helpers[0].source_sha256 == "d0ace0bff40e5daa99e32753a22a7558f9b2207e89260f00171fc50723b831dd" and
    .helpers[0].patch_sha256 == null and
    .helpers[1].pg_major == 18 and
    .helpers[1].pg_version == "18.6" and
    .helpers[1].parser_release == "18.1.0" and
    .helpers[1].source_sha256 == "2d3486cf6a9d3955b53e66235db39d62b54216c820cd392ab66dc842c5b1316d" and
    (.helpers[1].patch_sha256 | test("^[a-f0-9]{64}$")) and
    all(.helpers[]; (.binary_sha256 | test("^[a-f0-9]{64}$")))
' "$manifest" >/dev/null || {
  echo 'parser helper package manifest is incomplete' >&2
  exit 1
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

for major in 15 18; do
  expected=$(jq -r --argjson major "$major" '.helpers[] | select(.pg_major == $major) | .binary_sha256' "$manifest")
  actual=$(sha256_file "$1/scythe-pg$major-parser")
  [ "$actual" = "$expected" ] || {
    echo "PG$major helper checksum disagrees with package manifest" >&2
    exit 1
  }
  pg_version=$(jq -r --argjson major "$major" '.helpers[] | select(.pg_major == $major) | .pg_version' "$manifest")
  release=$(jq -r --argjson major "$major" '.helpers[] | select(.pg_major == $major) | .parser_release' "$manifest")
  actual_version=$("$1/scythe-pg$major-parser" --version)
  [ "$actual_version" = "scythe-pg$major-parser protocol/1 libpg_query/$release PostgreSQL/$pg_version" ] || {
    echo "PG$major helper version disagrees with package manifest" >&2
    exit 1
  }
done

echo 'parser helper controls passed for PG15 and PG18'
