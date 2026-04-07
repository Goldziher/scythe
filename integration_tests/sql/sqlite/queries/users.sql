-- @name GetUserById
-- @returns :one
SELECT id, name, email, status, created_at FROM users WHERE id = ?;

-- @name ListActiveUsers
-- @returns :many
SELECT id, name, email FROM users WHERE status = ?;

-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, email, status) VALUES (?, ?, ?);

-- @name UpdateUserEmail
-- @returns :exec
UPDATE users SET email = ? WHERE id = ?;

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = ?;

-- @name SearchUsers
-- @returns :many
SELECT id, name, email FROM users WHERE name LIKE ?;
