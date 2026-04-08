-- Redshift schema for integration tests
-- Uses Redshift-compatible types: IDENTITY, SUPER, GEOMETRY, VARCHAR, TIMESTAMPTZ

CREATE TABLE users (
    id INTEGER IDENTITY(1,1) NOT NULL,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    metadata SUPER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT GETDATE(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT GETDATE()
);

CREATE TABLE locations (
    id INTEGER IDENTITY(1,1) NOT NULL,
    name VARCHAR(255) NOT NULL,
    geo GEOMETRY,
    attributes SUPER
);
