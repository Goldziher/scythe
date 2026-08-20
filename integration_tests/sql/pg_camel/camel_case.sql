-- @name GetMultipleUnderscoreAlias
-- @returns :one
SELECT id AS multiple_underscore_alias FROM users WHERE id = $1;
