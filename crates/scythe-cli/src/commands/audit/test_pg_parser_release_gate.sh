#!/bin/sh
set -eu

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo 'usage: test_pg_parser_release_gate.sh BINARY_DIRECTORY' >&2
  exit 1
fi

failures=0
for major in 15 18; do
  binary="$1/scythe-pg$major-parser"
  [ -x "$binary" ] || exit 1
  trigger_ast=$(
    "$binary" --plpgsql <<'SQL'
CREATE FUNCTION audit_trigger() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  NEW.updated_at := now();
  RETURN NEW;
END;
$$;
SQL
  )
  if ! printf '%s\n' "$trigger_ast" | jq -e \
    '.ast[0].PLpgSQL_function.action.PLpgSQL_stmt_block.body | length > 0' >/dev/null; then
    echo "PG$major helper emitted invalid or incomplete PL/pgSQL trigger AST" >&2
    failures=$((failures + 1))
  fi
done

pg18_binary="$1/scythe-pg18-parser"
return_ast=$(
  "$pg18_binary" --plpgsql <<'SQL'
CREATE FUNCTION audit_return_var() RETURNS integer LANGUAGE plpgsql AS $$
DECLARE result integer := 1;
BEGIN
  RETURN result;
END;
$$;
SQL
)
if ! printf '%s\n' "$return_ast" | jq -e '
    .ast[0].PLpgSQL_function.action.PLpgSQL_stmt_block.body[-1].PLpgSQL_stmt_return |
    (has("expr") or (.retvarno | type == "number"))
' >/dev/null; then
  echo 'PG18 helper dropped a PL/pgSQL return-variable expression' >&2
  failures=$((failures + 1))
fi

# ~keep PG18 canonicalizes PL/pgSQL type names (int -> int4, and an unknown schema falls
# ~keep through to RECORDOID), so the exact "my_schema".users typname is not recoverable
# ~keep here. The cluster-scope gate classifies statement kinds, not variable types, so the
# ~keep property under test is that the declaration compiles instead of being refused --
# ~keep the unpatched 18.1.0 errors with "Not implemented", which this control still catches.
if qualified_ast=$(
  "$pg18_binary" --plpgsql <<'SQL'
CREATE FUNCTION audit_qualified_type() RETURNS void LANGUAGE plpgsql AS $$
DECLARE value "my_schema".users;
BEGIN
  RETURN;
END;
$$;
SQL
); then
  if ! printf '%s\n' "$qualified_ast" | jq -e '
        (.ast[0].PLpgSQL_function.action.PLpgSQL_stmt_block.body | length > 0) and
        any(.ast[0].PLpgSQL_function.datums[]?; .PLpgSQL_var.refname == "value" or .PLpgSQL_rec.refname == "value")
    ' >/dev/null; then
    echo 'PG18 helper did not compile a schema-qualified PL/pgSQL type declaration' >&2
    failures=$((failures + 1))
  fi
else
  echo 'PG18 helper refused a schema-qualified PL/pgSQL type' >&2
  failures=$((failures + 1))
fi

[ "$failures" -eq 0 ] || {
  echo "$failures parser release controls failed" >&2
  exit 1
}
echo 'parser release gate passed for PG15 and PG18'
