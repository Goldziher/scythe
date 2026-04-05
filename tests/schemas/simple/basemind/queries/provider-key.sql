---- provider key

-- @name CreateProviderKey
-- @returns :one
INSERT INTO provider_key (model_vendor, encrypted_api_key, project_id)
VALUES ($1, $2, $3)
RETURNING *;

-- @name RetrieveProviderKey
-- @returns :one
SELECT
    id,
    model_vendor,
    encrypted_api_key
FROM provider_key WHERE project_id = $1 AND model_vendor = $2;

-- @name CheckProviderKeyExists
-- @returns :one
SELECT EXISTS(SELECT 1 FROM provider_key WHERE id = $1);

-- @name DeleteProviderKey
-- @returns :exec
DELETE FROM provider_key WHERE id = $1;

-- @name RetrieveProjectProviderKeys
-- @returns :many
SELECT
    id,
    model_vendor,
    created_at
FROM provider_key WHERE project_id = $1;
