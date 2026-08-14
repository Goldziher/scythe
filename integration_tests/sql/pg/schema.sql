CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');

-- Nullable composite column target (board #197): a top-level composite must
-- decode a present value AND a SQL NULL. See sql/torture/schema.sql's
-- torture_address for the same shape under compile-only coverage. This one
-- is exercised live by the integration harnesses.
--
-- Keep every comment in this file free of semicolons. Each generated harness
-- executes the schema by splitting it into single statements on the semicolon
-- character, and that split is not comment-aware, so one inside a comment ends
-- the fragment early and leaves the rest of the comment line as bare SQL.
-- Guarded by schema_sql_comments_contain_no_semicolon in the generator tests.
CREATE TYPE user_address AS (
    street TEXT,
    city TEXT,
    zip TEXT
);

CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT,
    status user_status NOT NULL DEFAULT 'active',
    -- Nullable enum column target (board #197), distinct from the NOT NULL
    -- `status` column above: a reader that zero-decodes a null enum instead
    -- of reporting it as absent would pass every existing test here.
    secondary_status user_status,
    address user_address,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INT NOT NULL REFERENCES users (id),
    total NUMERIC(10, 2) NOT NULL,
    weight_kg DOUBLE PRECISION,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE tags (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE user_tags (
    user_id INT NOT NULL REFERENCES users (id),
    tag_id INT NOT NULL REFERENCES tags (id),
    PRIMARY KEY (user_id, tag_id)
);
