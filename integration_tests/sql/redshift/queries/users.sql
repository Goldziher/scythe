-- @name GetUser
-- @returns :one
SELECT id, name, email, status, metadata, created_at, updated_at
FROM users
WHERE id = $1;

-- @name ListUsers
-- @returns :many
SELECT id, name, email, status, created_at
FROM users
ORDER BY created_at DESC;

-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, email, status)
VALUES ($1, $2, $3);

-- @name UpdateUser
-- @returns :exec
UPDATE users
SET name = $1, email = $2, status = $3, updated_at = GETDATE()
WHERE id = $4;

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = $1;

-- @name SearchUsers
-- @returns :many
SELECT id, name, email, status
FROM users
WHERE status = $1
ORDER BY name;
