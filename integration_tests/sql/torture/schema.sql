-- Torture schema for the generated-code compile gate (issue #146).
--
-- sql/pg/schema.sql (the schema every committed postgresql-engine
-- integration project builds against) is 29 lines: one enum, a handful of
-- scalar columns, two foreign keys. It cannot reach the type/identifier
-- shapes several known codegen defects live in -- an array column, an enum
-- inside an array, a composite type, a quoted mixed-case identifier as a
-- primary key, a non-ASCII identifier, columns named for words reserved in
-- one or more target languages, and a column name that collides with a
-- name scythe itself might synthesize. This schema exists only to give the
-- compile gate (scripts/check-generated-backends.py) something with teeth;
-- it is never migrated into the live database used by the `task test:*`
-- integration tests, and no `scythe.toml` under integration_tests/ other
-- than the ones the gate writes into a scratch copy points at it.
--
-- Every construct below is independently proven to parse under scythe-core
-- by an existing testing_data/ fixture, so a failure downstream is a
-- codegen-backend defect, not a gap in what scythe-core can analyze:
--   - array column:            testing_data/types/arrays/01_integer_array.json
--   - enum:                    testing_data/types/enums/01_select_enum_column.json
--   - composite type:          testing_data/types/composite/01_composite_column.json
--   - uuid:                    testing_data/types/uuid/01_uuid_column.json
--   - jsonb:                   testing_data/types/json_jsonb_advanced/01_jsonb_contains.json
-- Enum-inside-array has no matching fixture (arrays and enums are each
-- proven individually, not composed) -- if scythe-core itself rejects
-- `torture_status[]`, that is itself a finding this gate should surface,
-- not something to route around.
--
-- The non-ASCII identifier issue #181 asks for is deliberately NOT a column
-- here: `CREATE TABLE t ("café" TEXT)` plus `SELECT "café" FROM t` makes
-- `scythe generate` fail outright with `UNKNOWN_COLUMN: column "café" does
-- not exist` -- scythe-core itself cannot resolve a byte-identical quoted
-- non-ASCII identifier between DDL and a query, before any backend runs. A
-- parse-time failure here would abort generation for the whole file, taking
-- down every other case in this schema for every backend and reporting one
-- uniform failure that says nothing about codegen. See
-- sql/torture/nonascii/schema.sql for that defect isolated on its own.

CREATE TYPE torture_status AS ENUM ('active', 'inactive', 'archived');

CREATE TYPE torture_address AS (
    street TEXT,
    city TEXT,
    zip TEXT
);

CREATE TABLE "torture_widgets" (
    -- Quoted, mixed-case identifier as a PRIMARY KEY (issue #178).
    "widgetId" SERIAL PRIMARY KEY,
    -- Reserved (or contextually reserved) words in one or more target
    -- languages: `type`/`fn` in Rust, `class` in Java/C#/Kotlin/Python,
    -- `end` in Ruby/Elixir, `type` as a soft keyword in modern Python.
    "type" TEXT NOT NULL,
    "class" TEXT,
    "fn" TEXT,
    "end" TEXT,
    -- Collides with a name scythe itself might synthesize for a nested or
    -- aggregated relation (e.g. a "children" field on a parent row struct).
    children INT NOT NULL DEFAULT 0,
    tags TEXT[] NOT NULL,                      -- array column (issue #200)
    statuses torture_status[] NOT NULL,        -- enum inside an array (issue #187)
    home_address torture_address,              -- composite type
    metadata JSONB,                            -- jsonb / json_nested (issue #147)
    external_id UUID NOT NULL DEFAULT gen_random_uuid(),
    status torture_status NOT NULL DEFAULT 'active',
    scheduled_at TIMESTAMP NOT NULL DEFAULT NOW() -- naive-datetime column (issue #192)
);
