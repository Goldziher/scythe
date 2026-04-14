-- Redshift schema for integration tests
-- Uses Redshift-compatible types: IDENTITY, SUPER, VARCHAR, TIMESTAMPTZ

CREATE TABLE users (
    id INTEGER IDENTITY(1,1) NOT NULL,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255),
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT GETDATE()
);

CREATE TABLE orders (
    id INTEGER IDENTITY(1,1) NOT NULL,
    user_id INTEGER NOT NULL,
    total DECIMAL(10, 2) NOT NULL,
    notes VARCHAR(4000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT GETDATE()
);

CREATE TABLE tags (
    id INTEGER IDENTITY(1,1) NOT NULL,
    name VARCHAR(255) NOT NULL
);

CREATE TABLE user_tags (
    user_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL
);
