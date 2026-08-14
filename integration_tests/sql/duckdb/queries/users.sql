-- @name GetUserById
-- @returns :one
SELECT id, name, email, status, created_at FROM users WHERE id = $1;

-- @name ListActiveUsers
-- @returns :many
SELECT id, name, email FROM users WHERE status = $1;

-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, email, status) VALUES ($1, $2, $3);

-- @name UpdateUserEmail
-- @returns :exec
UPDATE users SET email = $1 WHERE id = $2;

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = $1;

-- @name SearchUsers
-- @returns :many
SELECT id, name, email FROM users WHERE name LIKE $1;
