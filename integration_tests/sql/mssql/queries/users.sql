-- @name GetUserById
-- @returns :one
SELECT id, name, email, active, external_id, created_at FROM users WHERE id = @p1;

-- @name ListActiveUsers
-- @returns :many
SELECT id, name, email FROM users WHERE active = CAST(1 AS BIT);

-- @name CreateUser
-- @returns :one
INSERT INTO users (id, name, email, active)
OUTPUT INSERTED.id, INSERTED.name, INSERTED.email, INSERTED.active, INSERTED.created_at
VALUES (@p1, @p2, @p3, @p4);

-- @name UpdateUserEmail
-- @returns :exec
UPDATE users SET email = @p1 WHERE id = @p2;

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = @p1;

-- @name SearchUsers
-- @returns :many
SELECT id, name, email FROM users WHERE name LIKE @p1;
