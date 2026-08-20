-- Nested-aggregate queries (#78). These exist so the structs scythe
-- synthesizes for json_agg/row_to_json are compiled and round-tripped
-- against a real PostgreSQL server, not merely rendered as text.

-- @name GetUsersWithOrders
-- @returns :many
SELECT u.id, u.name, json_agg(o.*) AS orders
FROM users u
JOIN orders o ON o.user_id = u.id
GROUP BY u.id, u.name;

-- json_agg over a LEFT JOIN emits [null] for a user with no orders, which is
-- why the element type has to be optional.
-- @name GetUsersWithOrdersOuter
-- @returns :many
SELECT u.id, json_agg(o.*) AS orders
FROM users u
LEFT JOIN orders o ON o.user_id = u.id
GROUP BY u.id;

-- users.status is a PostgreSQL enum, so this nested struct forces the enum
-- definition to be emitted with serde derives it would not otherwise carry.
-- @name GetUserAsJson
-- @returns :many
SELECT row_to_json(u.*) AS payload FROM users u;

-- @name RoundTripUserAddress
-- @returns :one
INSERT INTO users (name, status, address)
VALUES ('Composite Parameter Round Trip', 'active', $1)
RETURNING address;
