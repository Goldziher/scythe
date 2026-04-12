CREATE TABLE users (
    id INT PRIMARY KEY,
    name NVARCHAR(255) NOT NULL,
    email NVARCHAR(255),
    active BIT NOT NULL DEFAULT 1,
    external_id UNIQUEIDENTIFIER,
    created_at DATETIME2 NOT NULL DEFAULT GETDATE()
);

CREATE TABLE orders (
    id INT PRIMARY KEY,
    user_id INT NOT NULL,
    total DECIMAL(10, 2) NOT NULL,
    notes NVARCHAR(4000),
    created_at DATETIME2 NOT NULL DEFAULT GETDATE(),
    CONSTRAINT FK_orders_users FOREIGN KEY (user_id) REFERENCES users (id)
);

CREATE TABLE tags (
    id INT PRIMARY KEY,
    name NVARCHAR(255) NOT NULL UNIQUE
);

CREATE TABLE user_tags (
    user_id INT NOT NULL,
    tag_id INT NOT NULL,
    PRIMARY KEY (user_id, tag_id),
    CONSTRAINT FK_user_tags_users FOREIGN KEY (user_id) REFERENCES users (id),
    CONSTRAINT FK_user_tags_tags FOREIGN KEY (tag_id) REFERENCES tags (id)
);
