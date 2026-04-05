---- api-key

-- @name CreateAPIKey
-- @returns :one
INSERT INTO api_key (application_id, name, is_internal)
VALUES ($1, $2, $3)
RETURNING *;

-- @name RetrieveAPIKeys
-- @returns :many
SELECT
    t.id,
    t.name,
    t.created_at
FROM api_key AS t
LEFT JOIN application AS app ON t.application_id = app.id
WHERE
    app.id = $1
    AND t.deleted_at IS NULL AND app.deleted_at IS NULL AND t.is_internal = FALSE
ORDER BY t.created_at;

-- @name DeleteAPIKey
-- @returns :exec
UPDATE api_key
SET deleted_at = NOW()
WHERE id = $1;

-- @name RetrieveApplicationDataForAPIKey
-- @returns :one
SELECT
    app.id AS application_id,
    app.project_id
FROM api_key AS t
LEFT JOIN application AS app ON t.application_id = app.id
WHERE
    t.id = $1
    AND t.deleted_at IS NULL
    AND app.deleted_at IS NULL;

-- @name RetrieveApplicationInternalAPIKeyID
-- @returns :one
SELECT t.id
FROM api_key AS t
LEFT JOIN application AS app ON t.application_id = app.id
WHERE
    app.id = $1
    AND t.deleted_at IS NULL
    AND app.deleted_at IS NULL
    AND t.is_internal = TRUE;
