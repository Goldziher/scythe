---- user_account

-- @name CheckUserAccountExists
-- @returns :one
SELECT EXISTS(SELECT 1 FROM user_account WHERE firebase_id = $1);

-- @name RetrieveUserAccountByFirebaseID
-- @returns :one
SELECT
    id,
    display_name,
    email,
    firebase_id,
    phone_number,
    photo_url,
    created_at
FROM user_account
WHERE firebase_id = $1;

-- @name RetrieveUserAccountByID
-- @returns :one
SELECT
    id,
    display_name,
    email,
    firebase_id,
    phone_number,
    photo_url,
    created_at
FROM user_account
WHERE id = $1;

-- @name RetrieveUserAccountByEmail
-- @returns :one
SELECT
    id,
    display_name,
    email,
    firebase_id,
    phone_number,
    photo_url,
    created_at
FROM user_account
WHERE email = $1;

-- @name CreateUserAccount
-- @returns :one
INSERT INTO user_account (
    display_name,
    email,
    firebase_id,
    phone_number,
    photo_url
)
VALUES ($1, $2, $3, $4, $5)
RETURNING *;

-- @name UpdateUserAccount
-- @returns :one
UPDATE user_account
SET
    email = $2,
    display_name = $3,
    firebase_id = $4,
    phone_number = $5,
    photo_url = $6
WHERE id = $1
RETURNING *;

-- @name DeleteUserAccount
-- @returns :exec
DELETE FROM user_account WHERE id = $1;

-- @name CheckUserIsSoleAdminInAnyProject
-- @returns :one
SELECT EXISTS(
    SELECT 1
    FROM project AS p
    LEFT JOIN user_project AS up ON p.id = up.project_id
    WHERE
        up.user_id = $1 AND up.permission = 'ADMIN' AND p.deleted_at IS NULL
        AND NOT EXISTS (
            SELECT 1
            FROM user_project AS up2
            WHERE up2.project_id = p.id AND up2.user_id != $1 AND up2.permission = 'ADMIN'
        )
);

-- @name RetrieveProjectUserAccounts
-- @returns :many
SELECT
    user_account.id,
    user_account.display_name,
    user_account.email,
    user_account.firebase_id,
    user_account.phone_number,
    user_account.photo_url,
    user_account.created_at,
    up.permission
FROM user_account
LEFT JOIN user_project AS up ON user_account.id = up.user_id
LEFT JOIN project AS p ON up.project_id = p.id
WHERE p.id = $1
ORDER BY user_account.display_name;
