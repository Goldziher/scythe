-- @name CreateOrder
-- @returns :one
INSERT INTO orders (id, user_id, total, notes)
OUTPUT INSERTED.id, INSERTED.user_id, INSERTED.total, INSERTED.notes, INSERTED.created_at
VALUES (@p1, @p2, @p3, @p4);

-- @name GetOrdersByUser
-- @returns :many
SELECT id, total, notes, created_at FROM orders WHERE user_id = @p1 ORDER BY created_at DESC;

-- @name GetOrderTotal
-- @returns :one
SELECT SUM(total) AS total_sum FROM orders WHERE user_id = @p1;

-- @name DeleteOrdersByUser
-- @returns :exec_rows
DELETE FROM orders WHERE user_id = @p1;
