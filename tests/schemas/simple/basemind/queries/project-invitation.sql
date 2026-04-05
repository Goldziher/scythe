-- @name RetrieveProjectInvitations
-- @returns :many
SELECT
    up.id,
    up.email,
    up.permission,
    up.created_at,
    up.updated_at
FROM project_invitation AS up
LEFT JOIN project AS p ON up.project_id = p.id
WHERE
    up.project_id = $1 AND p.deleted_at IS NULL
ORDER BY up.created_at;

-- @name RetrieveProjectInvitationByID
-- @returns :one
SELECT
    up.id,
    up.email,
    up.permission,
    up.project_id
FROM project_invitation AS up
LEFT JOIN project AS p ON up.project_id = p.id
WHERE up.id = $1 AND p.deleted_at IS NULL;

-- @name DeleteProjectInvitation
-- @returns :exec
DELETE FROM project_invitation
WHERE id = $1;

-- @name UpsertProjectInvitation
-- @returns :one
INSERT INTO project_invitation (
    email, project_id, permission
) VALUES (
    $1, $2, $3
)
ON CONFLICT (email, project_id) DO UPDATE
SET permission = $3
RETURNING *;
