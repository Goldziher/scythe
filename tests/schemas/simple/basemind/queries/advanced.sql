-- @name GetProjectWithUserCount
-- @returns :one
WITH project_users AS (
    SELECT project_id, COUNT(*) as user_count
    FROM user_project
    GROUP BY project_id
)
SELECT p.id, p.name, COALESCE(pu.user_count, 0) as user_count
FROM project p
LEFT JOIN project_users pu ON p.id = pu.project_id
WHERE p.id = $1 AND p.deleted_at IS NULL;

-- @name GetApplicationRankings
-- @returns :many
SELECT
    a.id,
    a.name,
    COUNT(prr.id) as request_count,
    ROW_NUMBER() OVER (ORDER BY COUNT(prr.id) DESC) as rank
FROM application a
LEFT JOIN prompt_config pc ON pc.application_id = a.id AND pc.deleted_at IS NULL
LEFT JOIN prompt_request_record prr ON prr.prompt_config_id = pc.id AND prr.deleted_at IS NULL
WHERE a.project_id = $1 AND a.deleted_at IS NULL
GROUP BY a.id, a.name
ORDER BY request_count DESC;

-- @name GetRecentRequestsWithLag
-- @returns :many
SELECT
    id,
    request_tokens,
    response_tokens,
    duration_ms,
    LAG(duration_ms) OVER (ORDER BY created_at) as prev_duration_ms,
    created_at
FROM prompt_request_record
WHERE prompt_config_id = $1 AND deleted_at IS NULL
ORDER BY created_at DESC
LIMIT $2;

-- @name GetProjectStats
-- @returns :one
SELECT
    COUNT(DISTINCT a.id) as app_count,
    COUNT(DISTINCT pc.id) as config_count,
    COALESCE(SUM(prr.request_tokens_cost + prr.response_tokens_cost), 0) as total_cost
FROM project p
LEFT JOIN application a ON a.project_id = p.id AND a.deleted_at IS NULL
LEFT JOIN prompt_config pc ON pc.application_id = a.id AND pc.deleted_at IS NULL
LEFT JOIN prompt_request_record prr ON prr.prompt_config_id = pc.id AND prr.deleted_at IS NULL
WHERE p.id = $1 AND p.deleted_at IS NULL;

-- @name SearchApplicationsByName
-- @returns :many
SELECT id, name, description, created_at
FROM application
WHERE project_id = $1 AND deleted_at IS NULL AND name LIKE $2
ORDER BY name;

-- @name GetActiveModelPricing
-- @returns :many
SELECT id, model_type, model_vendor, input_token_price, output_token_price, token_unit_size
FROM provider_model_pricing
WHERE active_from_date <= CURRENT_DATE
  AND (active_to_date IS NULL OR active_to_date >= CURRENT_DATE)
ORDER BY model_vendor, model_type;

-- @name UpsertProjectInvitationAdvanced
-- @returns :one
INSERT INTO project_invitation (email, project_id, permission)
VALUES ($1, $2, $3)
ON CONFLICT (email, project_id)
DO UPDATE SET permission = EXCLUDED.permission, updated_at = NOW()
RETURNING id, email, project_id, permission, created_at, updated_at;
