-- @name CreateOrder
-- @returns :exec
INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?);

-- @name GetLastInsertOrder
-- @returns :one
SELECT id, user_id, total, notes, created_at FROM orders WHERE id = LAST_INSERT_ID();

-- @name GetOrdersByUser
-- @returns :many
SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC;

-- @name GetOrderTotal
-- @returns :one
SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?;

-- @name DeleteOrdersByUser
-- @returns :exec_rows
DELETE FROM orders WHERE user_id = ?;
