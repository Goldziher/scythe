-- @name GetUserById
-- @returns :one
SELECT id, name, email, active, created_at FROM users WHERE id = :1;

-- @name ListActiveUsers
-- @returns :many
SELECT id, name, email FROM users WHERE active = 1;

-- @name CreateUser
-- @returns :one
INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :4, :5, :6, :7, :8;

-- @name UpdateUserEmail
-- @returns :exec
UPDATE users SET email = :1 WHERE id = :2;

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = :1;

-- @name SearchUsers
-- @returns :many
SELECT id, name, email FROM users WHERE name LIKE :1;
