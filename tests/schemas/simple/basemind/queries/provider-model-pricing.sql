-- provider model pricing

-- @name CreateProviderModelPricing
-- @returns :one
INSERT INTO provider_model_pricing (
    model_type,
    model_vendor,
    input_token_price,
    output_token_price,
    token_unit_size,
    active_from_date,
    active_to_date
)
VALUES (
    $1,
    $2, $3, $4, $5, $6, $7
) RETURNING *;

-- @name RetrieveActiveProviderModelPricing
-- @returns :one
SELECT
    id,
    model_type,
    model_vendor,
    input_token_price,
    output_token_price,
    token_unit_size,
    active_from_date,
    active_to_date
FROM provider_model_pricing
WHERE
    model_type = $1
    AND model_vendor = $2
    AND active_from_date <= CURRENT_DATE
    AND active_to_date IS NULL OR active_to_date >= CURRENT_DATE
ORDER BY active_from_date DESC
LIMIT 1;
