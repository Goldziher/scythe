-- PG-compatible version of the Redshift schema for CI testing
-- Substitutions: IDENTITY -> SERIAL, GETDATE() -> NOW()

CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255),
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id),
    total DECIMAL(10, 2) NOT NULL,
    notes VARCHAR(4000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE tags (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL UNIQUE
);

CREATE TABLE user_tags (
    user_id INTEGER NOT NULL REFERENCES users (id),
    tag_id INTEGER NOT NULL REFERENCES tags (id),
    PRIMARY KEY (user_id, tag_id)
);
