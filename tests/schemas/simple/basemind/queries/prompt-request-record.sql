---- prompt request record

-- @name CreatePromptRequestRecord
-- @returns :one
INSERT INTO prompt_request_record (
    is_stream_response,
    request_tokens,
    response_tokens,
    request_tokens_cost,
    response_tokens_cost,
    start_time,
    finish_time,
    duration_ms,
    prompt_config_id,
    provider_model_pricing_id,
    error_log,
    finish_reason
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
RETURNING *;
