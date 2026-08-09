-- Queries against sql/torture/schema.sql. See that file's header for why
-- this schema and these queries exist and what they are (and are not) used
-- for.

-- @name GetWidget
-- @returns :one
SELECT "widgetId", "type", "class", "fn", "end", children, tags, statuses, home_address, metadata, external_id, status, scheduled_at
FROM "torture_widgets"
WHERE "widgetId" = $1;

-- @name ListWidgetsByStatus
-- @returns :many
SELECT "widgetId", "type", tags, statuses, status FROM "torture_widgets" WHERE status = $1;

-- @name CreateWidget
-- @returns :one
INSERT INTO "torture_widgets" ("type", "class", "fn", "end", children, tags, statuses, home_address, metadata, external_id, status)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
RETURNING "widgetId", scheduled_at;

-- @name DeleteWidget
-- @returns :exec_rows
DELETE FROM "torture_widgets" WHERE "widgetId" = $1;
