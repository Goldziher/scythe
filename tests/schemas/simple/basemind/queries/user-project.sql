---- user_project

-- @name CheckUserProjectExists
-- @returns :one
SELECT EXISTS(
    SELECT 1
    FROM user_project AS up
    LEFT JOIN project AS p ON up.project_id = p.id
    LEFT JOIN user_account AS ua ON up.user_id = ua.id
    WHERE
        ua.email = $1
        AND up.project_id = $2
        AND p.deleted_at IS NULL
);

-- @name CreateUserProject
-- @returns :one
INSERT INTO user_project (user_id, project_id, permission)
VALUES ($1, $2, $3)
RETURNING *;

-- @name UpdateUserProjectPermission
-- @returns :one
UPDATE user_project
SET
    permission = $3,
    updated_at = NOW()
WHERE
    user_id = $1
    AND project_id = $2
RETURNING *;

-- @name RetrieveUserProject
-- @returns :one
SELECT
    up.user_id,
    up.project_id,
    up.permission,
    up.created_at,
    up.updated_at
FROM user_project AS up
LEFT JOIN user_account AS ua ON up.user_id = ua.id
LEFT JOIN project AS p ON up.project_id = p.id
WHERE
    ua.firebase_id = $1
    AND up.project_id = $2 AND p.deleted_at IS NULL;

-- @name DeleteUserProject
-- @returns :exec
DELETE FROM user_project
WHERE
    user_id = $1
    AND project_id = $2;
