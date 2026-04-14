-- @name CreateOrder
-- @returns :exec
INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?);

-- @name GetOrdersByUser
-- @returns :many
SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC;

-- @name GetOrderTotal
-- @returns :one
SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?;

-- @name DeleteOrdersByUser
-- @returns :exec_rows
DELETE FROM orders WHERE id IN (SELECT id FROM orders WHERE user_id = ?);
