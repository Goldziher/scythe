---- prompt config

-- @name CreatePromptConfig
-- @returns :one
INSERT INTO prompt_config (
    name,
    model_parameters,
    model_type,
    model_vendor,
    provider_prompt_messages,
    expected_template_variables,
    is_default,
    application_id,
    is_test_config
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
RETURNING *;

-- @name CheckDefaultPromptConfigExists
-- @returns :one
SELECT EXISTS(
    SELECT 1
    FROM prompt_config
    WHERE
        application_id = $1
        AND deleted_at IS NULL
        AND is_default = TRUE
);

-- @name UpdateDefaultPromptConfig
-- @returns :exec
UPDATE prompt_config
SET
    is_default = $2,
    updated_at = NOW()
WHERE
    id = $1
    AND deleted_at IS NULL
    AND is_test_config = FALSE;

-- @name UpdatePromptConfig
-- @returns :one
UPDATE prompt_config
SET
    name = $2,
    model_parameters = $3,
    model_type = $4,
    model_vendor = $5,
    provider_prompt_messages = $6,
    expected_template_variables = $7,
    is_test_config = $8,
    updated_at = NOW()
WHERE
    id = $1
    AND deleted_at IS NULL
RETURNING *;

-- @name DeletePromptConfig
-- @returns :exec
UPDATE prompt_config
SET deleted_at = NOW()
WHERE id = $1;

-- @name RetrievePromptConfig
-- @returns :one
SELECT
    id,
    name,
    model_parameters,
    model_type,
    model_vendor,
    provider_prompt_messages,
    expected_template_variables,
    is_default,
    created_at,
    updated_at,
    application_id,
    is_test_config
FROM prompt_config
WHERE
    id = $1
    AND deleted_at IS NULL;

-- @name RetrievePromptConfigs
-- @returns :many
SELECT
    id,
    name,
    model_parameters,
    model_type,
    model_vendor,
    provider_prompt_messages,
    expected_template_variables,
    is_default,
    created_at,
    updated_at,
    application_id
FROM prompt_config
WHERE
    application_id = $1
    AND deleted_at IS NULL
    AND is_test_config = FALSE;

-- @name RetrieveDefaultPromptConfig
-- @returns :one
SELECT
    id,
    name,
    model_parameters,
    model_type,
    model_vendor,
    provider_prompt_messages,
    expected_template_variables,
    is_default,
    created_at,
    updated_at,
    application_id
FROM prompt_config
WHERE
    application_id = $1
    AND deleted_at IS NULL
    AND is_default = TRUE
    AND is_test_config = FALSE;

-- @name RetrievePromptConfigAPIRequestCount
-- @returns :one
SELECT COUNT(prr.id) AS total_requests
FROM prompt_config AS pc
LEFT JOIN prompt_request_record AS prr ON pc.id = prr.prompt_config_id
WHERE
    pc.id = $1
    AND prr.created_at BETWEEN $2 AND $3;

-- @name RetrievePromptConfigTokensTotalCost
-- @returns :one
SELECT COALESCE(SUM(prr.request_tokens_cost + prr.response_tokens_cost), 0)
FROM prompt_config AS pc
LEFT JOIN prompt_request_record AS prr ON pc.id = prr.prompt_config_id
WHERE
    pc.id = $1
    AND prr.created_at BETWEEN $2 AND $3;
