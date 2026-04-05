---- application

-- @name CreateApplication
-- @returns :one
INSERT INTO application (
    project_id,
    name,
    description
)
VALUES ($1, $2, $3)
RETURNING *;

-- @name UpdateApplication
-- @returns :one
UPDATE application
SET
    name = $2,
    description = $3,
    updated_at = NOW()
WHERE
    id = $1
    AND deleted_at IS NULL
RETURNING *;

-- @name DeleteApplication
-- @returns :exec
UPDATE application
SET deleted_at = NOW()
WHERE id = $1;

-- @name RetrieveApplication
-- @returns :one
SELECT
    id,
    description,
    name,
    created_at,
    updated_at,
    project_id
FROM application
WHERE
    id = $1
    AND deleted_at IS NULL;

-- @name RetrieveApplications
-- @returns :many
SELECT
    id,
    description,
    name,
    created_at,
    updated_at,
    project_id
FROM application
WHERE
    project_id = $1
    AND deleted_at IS NULL;

-- @name RetrieveApplicationAPIRequestCount
-- @returns :one
SELECT COUNT(prr.id) AS total_requests
FROM application AS a
INNER JOIN prompt_config AS pc ON a.id = pc.application_id
INNER JOIN prompt_request_record AS prr ON pc.id = prr.prompt_config_id
WHERE
    a.id = $1
    AND prr.created_at BETWEEN $2 AND $3;

-- @name RetrieveApplicationTokensTotalCost
-- @returns :one
SELECT COALESCE(SUM(prr.request_tokens_cost + prr.response_tokens_cost), 0)
FROM application AS app
LEFT JOIN prompt_config AS pc ON app.id = pc.application_id
LEFT JOIN prompt_request_record AS prr ON pc.id = prr.prompt_config_id
WHERE
    app.id = $1
    AND prr.created_at BETWEEN $2 AND $3;
