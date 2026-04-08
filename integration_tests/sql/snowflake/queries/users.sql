-- @name GetUserById
-- @returns :one
SELECT id, name, email, active, metadata, created_at, updated_at FROM users WHERE id = ?;

-- @name ListActiveUsers
-- @returns :many
SELECT id, name, email FROM users WHERE active = TRUE;

-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, email, active, metadata) VALUES (?, ?, ?, PARSE_JSON(?));

-- @name UpdateUserEmail
-- @returns :exec
UPDATE users SET email = ?, updated_at = CURRENT_TIMESTAMP() WHERE id = ?;

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = ?;

-- @name SearchUsers
-- @returns :many
SELECT id, name, email FROM users WHERE name LIKE ?;
