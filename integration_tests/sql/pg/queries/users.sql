-- @name GetUserById
-- @returns :one
SELECT id, name, email, status, created_at FROM users WHERE id = $1;

-- @name ListActiveUsers
-- @returns :many
SELECT id, name, email FROM users WHERE status = $1;

-- @name CreateUser
-- @returns :one
INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at;

-- @name UpdateUserEmail
-- @returns :exec
UPDATE users SET email = $1 WHERE id = $2;

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = $1;

-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;

-- @name CountUsersByStatus
-- @returns :one
SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1;

-- @name GetUserWithTags
-- @returns :many
SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = $1;

-- @name SearchUsers
-- @returns :many
SELECT id, name, email FROM users WHERE name LIKE $1;
