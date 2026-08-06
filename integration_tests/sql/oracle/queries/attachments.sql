-- @name CreateAttachment
-- @returns :one
INSERT INTO attachments (order_id, filename, payload, description) VALUES (:1, :2, :3, :4) RETURNING id, order_id, filename INTO :5, :6, :7;

-- @name GetAttachmentsByOrder
-- @returns :many
SELECT id, order_id, filename, payload, description FROM attachments WHERE order_id = :1 ORDER BY id;

-- @name GetAttachmentById
-- @returns :opt
SELECT id, order_id, filename, payload, description FROM attachments WHERE id = :1;

-- @name DeleteAttachmentsByOrder
-- @returns :exec_rows
DELETE FROM attachments WHERE order_id = :1;
