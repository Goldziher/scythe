-- DuckDB integration schema (issue #126).
--
-- Modelled on sql/sqlite/schema.sql rather than sql/pg/schema.sql: DuckDB is
-- embedded, so like SQLite it needs no service container, and its type system
-- is closer to PostgreSQL's than SQLite's while its DDL is closer to SQLite's.
-- Where the two disagree, this follows what DuckDB actually accepts:
--
--   * no AUTOINCREMENT. DuckDB has no such keyword -- identity comes from a
--     sequence plus a DEFAULT, which is the documented idiom.
--   * real types rather than SQLite's affinities: DuckDB has VARCHAR, INTEGER,
--     DECIMAL and TIMESTAMP as distinct types, so the generated code exercises
--     genuine type mapping instead of everything collapsing to TEXT/REAL.
--   * DECIMAL(10, 2) for money rather than SQLite's REAL, so `total` resolves
--     to a decimal in every target language rather than a float.
--
-- Keep every comment in this file free of semicolons. Each generated harness
-- executes the schema by splitting it into single statements on the semicolon
-- character, and that split is not comment-aware, so one inside a comment ends
-- the fragment early and leaves the rest of the comment line as bare SQL.
-- Guarded by schema_sql_comments_contain_no_semicolon in the generator tests.

CREATE SEQUENCE users_id_seq;
CREATE SEQUENCE orders_id_seq;
CREATE SEQUENCE tags_id_seq;

CREATE TABLE users (
    id INTEGER PRIMARY KEY DEFAULT nextval('users_id_seq'),
    name VARCHAR NOT NULL,
    email VARCHAR,
    status VARCHAR NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'inactive', 'banned')),
    created_at TIMESTAMP NOT NULL DEFAULT current_timestamp
);

CREATE TABLE orders (
    id INTEGER PRIMARY KEY DEFAULT nextval('orders_id_seq'),
    user_id INTEGER NOT NULL REFERENCES users (id),
    total DECIMAL(10, 2) NOT NULL,
    notes VARCHAR,
    created_at TIMESTAMP NOT NULL DEFAULT current_timestamp
);

CREATE TABLE tags (
    id INTEGER PRIMARY KEY DEFAULT nextval('tags_id_seq'),
    name VARCHAR NOT NULL UNIQUE
);

CREATE TABLE user_tags (
    user_id INTEGER NOT NULL REFERENCES users (id),
    tag_id INTEGER NOT NULL REFERENCES tags (id),
    PRIMARY KEY (user_id, tag_id)
);
