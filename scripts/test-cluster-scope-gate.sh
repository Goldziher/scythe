#!/bin/sh
# Acceptance controls for `scythe audit --cluster-scope` (issue #269).
#
# Usage: test-cluster-scope-gate.sh SCYTHE_BINARY PARSER_DIRECTORY [PG_MAJOR]
#
# Every planted cluster-scoped statement must be refused with a gate code, and
# every database-local statement must be accepted. The counts are asserted so a
# harness that examines nothing cannot pass.
set -eu

if [ "$#" -lt 2 ] || [ -z "$1" ] || [ -z "$2" ]; then
  echo 'usage: test-cluster-scope-gate.sh SCYTHE_BINARY PARSER_DIRECTORY [PG_MAJOR]' >&2
  exit 1
fi

scythe=$1
parser_dir=$2
pg_major=${3:-18}
[ -x "$scythe" ] || {
  echo "scythe binary not executable: $scythe" >&2
  exit 1
}

scratch=$(mktemp -d)
[ -n "$scratch" ] && [ -d "$scratch" ] || exit 1
trap 'rm -rf -- "$scratch"' EXIT HUP INT TERM

refused_count=0
accepted_count=0

run_gate() {
  SCYTHE_PG_PARSER_DIR="$parser_dir" "$scythe" audit --cluster-scope --format json \
    --pg-version "$pg_major" "$1" 2>"$scratch/stderr"
}

assert_refused() {
  label=$1
  statement=$2
  input="$scratch/refused-$label.sql"
  printf '%s\n' "$statement" >"$input"
  if run_gate "$input" >"$scratch/stdout"; then
    echo "cluster-scope gate accepted planted statement ($label): $statement" >&2
    exit 1
  fi
  if ! grep -Eq 'SC-CLUSTER0[1-4]' "$scratch/stdout" "$scratch/stderr"; then
    echo "cluster-scope gate refused '$label' without a gate code: $(cat "$scratch/stderr")" >&2
    exit 1
  fi
  refused_count=$((refused_count + 1))
}

assert_accepted() {
  label=$1
  statement=$2
  input="$scratch/accepted-$label.sql"
  printf '%s\n' "$statement" >"$input"
  if ! run_gate "$input" >"$scratch/stdout"; then
    echo "cluster-scope gate refused database-local statement ($label): $statement" >&2
    cat "$scratch/stderr" >&2
    exit 1
  fi
  accepted_count=$((accepted_count + 1))
}

# --- planted cluster-scoped statements (must be refused) ---
while IFS= read -r row; do
  [ -z "$row" ] && continue
  assert_refused "${row%%|*}" "${row#*|}"
done <<'ROWS'
create-role|CREATE ROLE planted_role;
alter-role|ALTER ROLE planted_role LOGIN;
alter-role-set|ALTER ROLE planted_role SET work_mem = '1MB';
drop-role|DROP ROLE planted_role;
create-user|CREATE USER planted_user;
alter-user|ALTER USER planted_user LOGIN;
drop-user|DROP USER planted_user;
create-group|CREATE GROUP planted_group;
alter-group|ALTER GROUP planted_group ADD USER planted_user;
drop-group|DROP GROUP planted_group;
grant-role|GRANT planted_role TO planted_member;
revoke-role|REVOKE planted_role FROM planted_member;
create-database|CREATE DATABASE planted_database;
alter-database-owner|ALTER DATABASE planted_database OWNER TO planted_role;
drop-database|DROP DATABASE planted_database;
grant-database|GRANT CONNECT, CREATE ON DATABASE planted_database TO planted_role;
revoke-database|REVOKE CONNECT ON DATABASE planted_database FROM PUBLIC;
grant-tablespace|GRANT CREATE ON TABLESPACE planted_tablespace TO planted_role;
revoke-tablespace|REVOKE ALL ON TABLESPACE planted_tablespace FROM planted_role;
grant-parameter-set|GRANT SET ON PARAMETER work_mem TO planted_role;
grant-parameter-alter-system|GRANT ALTER SYSTEM ON PARAMETER work_mem TO planted_role;
drop-owned|DROP OWNED BY planted_role;
reassign-owned|REASSIGN OWNED BY planted_role TO planted_member;
comment-role|COMMENT ON ROLE planted_role IS 'planted';
comment-database|COMMENT ON DATABASE planted_database IS 'planted';
comment-tablespace|COMMENT ON TABLESPACE planted_tablespace IS 'planted';
create-subscription|CREATE SUBSCRIPTION planted CONNECTION 'dbname=planted' PUBLICATION planted;
alter-system|ALTER SYSTEM SET application_name = 'planted';
create-tablespace|CREATE TABLESPACE planted_tablespace LOCATION '/tmp/planted';
drop-tablespace|DROP TABLESPACE planted_tablespace;
alter-owner|ALTER TABLE planted_table OWNER TO planted_role;
top-level-select|SELECT planted_function();
top-level-call|CALL planted_procedure();
psql-gexec|SELECT 'CREATE ROLE planted_role' \gexec
block-comment-split|CREATE/**/ROLE planted_role;
escape-string-smuggle|COMMENT ON TABLE planted_table IS E'\''; CREATE ROLE planted_role; --';
unterminated-string|COMMENT ON TABLE planted_table IS 'planted;
dollar-do|DO $body$ BEGIN ALTER ROLE planted_role LOGIN; END $body$;
dollar-do-grant-role|DO $body$ BEGIN GRANT planted_role TO planted_member; END $body$;
dollar-do-grant-database|DO $body$ BEGIN GRANT ALL ON DATABASE planted_database TO planted_role; END $body$;
dollar-do-grant-parameter|DO $body$ BEGIN GRANT SET ON PARAMETER work_mem TO planted_role; END $body$;
dollar-do-drop-owned|DO $body$ BEGIN DROP OWNED BY planted_role; END $body$;
dollar-do-alter-system|DO $body$ BEGIN ALTER SYSTEM SET work_mem = '1MB'; END $body$;
dollar-do-alter-owner|DO $body$ BEGIN ALTER TABLE planted_table OWNER TO planted_role; END $body$;
quoted-function-body|CREATE FUNCTION planted_function() RETURNS void LANGUAGE sql AS 'GRANT planted_role TO planted_member';
quoted-do-body|DO 'BEGIN GRANT planted_role TO planted_member; END';
dynamic-create-role|DO $body$ BEGIN EXECUTE 'CREATE ROLE planted_role'; END $body$;
dynamic-concatenation|DO $body$ BEGIN EXECUTE 'CREATE ' || 'ROLE planted_concat'; END $body$;
dynamic-format|DO $body$ BEGIN EXECUTE format('%s ROLE %I', 'CREATE', 'planted_format'); END $body$;
dynamic-variable|DO $body$ DECLARE statement text := 'ALTER ROLE planted_variable LOGIN'; BEGIN EXECUTE statement; END $body$;
dynamic-grant-database|DO $body$ BEGIN EXECUTE format('GRANT ALL ON DATABASE %I TO %I', 'planted', 'planted'); END $body$;
dynamic-unsafe-placeholder|DO $body$ BEGIN EXECUTE format('CREATE TABLE safe_%s(id int)', planted); END $body$;
dynamic-trailing-expression|DO $body$ BEGIN EXECUTE format('CREATE TABLE %I(id int)', safe_name) || planted; END $body$;
ROWS

# --- database-local statements (must be accepted) ---
while IFS= read -r row; do
  [ -z "$row" ] && continue
  assert_accepted "${row%%|*}" "${row#*|}"
done <<'ROWS'
create-table|CREATE TABLE planted_table(id integer PRIMARY KEY, role text, database text);
create-unique-index|CREATE UNIQUE INDEX planted_index ON planted_table(id);
alter-table-rls|ALTER TABLE planted_table ENABLE ROW LEVEL SECURITY;
create-policy|CREATE POLICY planted_policy ON planted_table FOR SELECT TO planted_role USING (true);
grant-table|GRANT SELECT, INSERT ON TABLE planted_table TO planted_role;
revoke-function|REVOKE EXECUTE ON FUNCTION planted_function() FROM PUBLIC;
comment-table-literal|COMMENT ON TABLE planted_table IS 'GRANT ALL ON DATABASE planted TO planted';
comment-escape-string|COMMENT ON COLUMN planted_table.id IS E'it\'s local';
create-function|CREATE OR REPLACE FUNCTION planted_function() RETURNS integer LANGUAGE sql AS $$ SELECT 1 $$;
create-trigger|CREATE TRIGGER planted_trigger BEFORE UPDATE ON planted_table FOR EACH ROW EXECUTE FUNCTION planted_function();
create-event-trigger|CREATE EVENT TRIGGER planted_trigger ON ddl_command_end EXECUTE PROCEDURE planted_function();
create-enum|CREATE TYPE planted_state AS ENUM ('database', 'role');
alter-sequence-owned-by|ALTER SEQUENCE planted_sequence OWNED BY planted_table.id;
create-user-mapping|CREATE USER MAPPING FOR planted_role SERVER planted_server;
alter-user-mapping|ALTER USER MAPPING FOR planted_role SERVER planted_server OPTIONS (SET password 'planted');
dollar-do-local|DO $body$ BEGIN RAISE NOTICE 'planted'; END $body$;
dynamic-safe-revoke|DO $body$ DECLARE target record; BEGIN FOR target IN SELECT oid FROM pg_proc LOOP EXECUTE format('REVOKE EXECUTE ON FUNCTION %s FROM PUBLIC', target.oid::regprocedure); END LOOP; END $body$;
ROWS

# --- empty input is refused ---
: >"$scratch/empty.sql"
if run_gate "$scratch/empty.sql" >"$scratch/stdout"; then
  echo 'cluster-scope gate accepted an empty planned schema input' >&2
  exit 1
fi

# ~keep Assert the harness did real work: a matrix that silently ran zero rows
# ~keep must not report success.
if [ "$refused_count" -lt 45 ] || [ "$accepted_count" -lt 17 ]; then
  echo "cluster-scope controls ran too few cases: refused=$refused_count accepted=$accepted_count" >&2
  exit 1
fi

echo "cluster-scope gate: refused $refused_count planted statements, accepted $accepted_count local statements, refused empty input"
