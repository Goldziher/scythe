CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');

-- Nullable composite column target (board #197): a top-level composite must
-- decode a present value AND a SQL NULL. See sql/torture/schema.sql's
-- torture_address for the same shape under compile-only coverage; this one
-- is exercised live by the integration harnesses.
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
