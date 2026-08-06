-- Oracle schema for scythe integration tests.
-- Note: sequences and triggers are created by the integration test setup,
-- not parsed by scythe (sqlparser does not support Oracle PL/SQL blocks).

CREATE TABLE users (
    id NUMBER NOT NULL PRIMARY KEY,
    name VARCHAR2(255) NOT NULL,
    email VARCHAR2(255),
    active NUMBER(1) DEFAULT 1 NOT NULL,
    created_at DATE DEFAULT SYSDATE NOT NULL
);

CREATE TABLE orders (
    id NUMBER NOT NULL PRIMARY KEY,
    user_id NUMBER NOT NULL,
    total NUMBER(10, 2) NOT NULL,
    notes CLOB,
    created_at DATE DEFAULT SYSDATE NOT NULL,
    CONSTRAINT fk_orders_users FOREIGN KEY (user_id) REFERENCES users (id)
);

CREATE TABLE tags (
    id NUMBER NOT NULL PRIMARY KEY,
    name VARCHAR2(255) NOT NULL UNIQUE
);

CREATE TABLE attachments (
    id NUMBER NOT NULL PRIMARY KEY,
    order_id NUMBER NOT NULL,
    filename VARCHAR2(255) NOT NULL,
    payload BLOB NOT NULL,
    description NCLOB,
    CONSTRAINT fk_attachments_orders FOREIGN KEY (order_id) REFERENCES orders (id)
);

CREATE TABLE user_tags (
    user_id NUMBER NOT NULL,
    tag_id NUMBER NOT NULL,
    PRIMARY KEY (user_id, tag_id),
    CONSTRAINT fk_user_tags_users FOREIGN KEY (user_id) REFERENCES users (id),
    CONSTRAINT fk_user_tags_tags FOREIGN KEY (tag_id) REFERENCES tags (id)
);
