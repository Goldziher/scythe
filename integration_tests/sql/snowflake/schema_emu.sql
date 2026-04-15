-- Emulator-compatible Snowflake schema (DuckDB-backed)
-- Simplified: no VARIANT, no FOREIGN KEY constraints, uses AUTOINCREMENT

CREATE TABLE users (
    id INTEGER NOT NULL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata VARCHAR,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP(),
    updated_at TIMESTAMP
);

CREATE TABLE orders (
    id INTEGER NOT NULL PRIMARY KEY,
    user_id INTEGER NOT NULL,
    total FLOAT NOT NULL,
    notes VARCHAR,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP()
);

CREATE TABLE tags (
    id INTEGER NOT NULL PRIMARY KEY,
    name VARCHAR NOT NULL
);

CREATE TABLE user_tags (
    user_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    PRIMARY KEY (user_id, tag_id)
);
