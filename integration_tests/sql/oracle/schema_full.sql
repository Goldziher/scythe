-- Full Oracle schema including sequences and triggers.
-- Used by integration tests for actual database setup.

CREATE SEQUENCE users_seq START WITH 1 INCREMENT BY 1;

CREATE TABLE users (
    id NUMBER NOT NULL PRIMARY KEY,
    name VARCHAR2(255) NOT NULL,
    email VARCHAR2(255),
    active NUMBER(1) DEFAULT 1 NOT NULL,
    created_at DATE DEFAULT SYSDATE NOT NULL
);

CREATE OR REPLACE TRIGGER users_bi
BEFORE INSERT ON users
FOR EACH ROW
BEGIN
    IF :NEW.id IS NULL THEN
        :NEW.id := users_seq.NEXTVAL;
    END IF;
END;
/

CREATE SEQUENCE orders_seq START WITH 1 INCREMENT BY 1;

CREATE TABLE orders (
    id NUMBER NOT NULL PRIMARY KEY,
    user_id NUMBER NOT NULL,
    total NUMBER(10, 2) NOT NULL,
    notes CLOB,
    created_at DATE DEFAULT SYSDATE NOT NULL,
    CONSTRAINT fk_orders_users FOREIGN KEY (user_id) REFERENCES users (id)
);

CREATE OR REPLACE TRIGGER orders_bi
BEFORE INSERT ON orders
FOR EACH ROW
BEGIN
    IF :NEW.id IS NULL THEN
        :NEW.id := orders_seq.NEXTVAL;
    END IF;
END;
/

CREATE TABLE tags (
    id NUMBER NOT NULL PRIMARY KEY,
    name VARCHAR2(255) NOT NULL UNIQUE
);

CREATE SEQUENCE tags_seq START WITH 1 INCREMENT BY 1;

CREATE OR REPLACE TRIGGER tags_bi
BEFORE INSERT ON tags
FOR EACH ROW
BEGIN
    IF :NEW.id IS NULL THEN
        :NEW.id := tags_seq.NEXTVAL;
    END IF;
END;
/

CREATE TABLE user_tags (
    user_id NUMBER NOT NULL,
    tag_id NUMBER NOT NULL,
    PRIMARY KEY (user_id, tag_id),
    CONSTRAINT fk_user_tags_users FOREIGN KEY (user_id) REFERENCES users (id),
    CONSTRAINT fk_user_tags_tags FOREIGN KEY (tag_id) REFERENCES tags (id)
);
