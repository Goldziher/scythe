-- Emulator-compatible Snowflake schema (DuckDB-backed)
-- Simplified: no VARIANT, no AUTOINCREMENT, no FOREIGN KEY constraints

CREATE TABLE users (
    id INTEGER NOT NULL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP
);

CREATE TABLE orders (
    id INTEGER NOT NULL PRIMARY KEY,
    user_id INTEGER NOT NULL,
    total NUMBER(10, 2) NOT NULL,
    notes VARCHAR(16777216),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE tags (
    id INTEGER NOT NULL PRIMARY KEY,
    name VARCHAR(255) NOT NULL UNIQUE
);

CREATE TABLE user_tags (
    user_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    PRIMARY KEY (user_id, tag_id)
);
