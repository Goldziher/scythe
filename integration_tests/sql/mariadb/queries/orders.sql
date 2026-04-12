-- @name CreateOrder
-- @returns :one
INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at;

-- @name GetOrdersByUser
-- @returns :many
SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC;

-- @name GetOrderTotal
-- @returns :one
SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?;

-- @name DeleteOrdersByUser
-- @returns :exec_rows
DELETE FROM orders WHERE user_id = ?;
