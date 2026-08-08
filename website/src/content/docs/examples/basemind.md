---
title: BaseMind.AI
description: Real-world example from the basemind.ai open-source project.
---

Real-world example from the [basemind.ai](https://github.com/basemind-ai/monorepo) open-source project (MIT license). This mirrors the `tests/schemas/simple/basemind` fixture in the scythe repository.

## Schema overview

11 tables with enums, arrays, JSON, and soft-delete patterns. Table names are singular (`project`, `application`, `api_key`, `user_project`), matching the source project's convention:

```sql
CREATE TABLE user_account
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    display_name varchar(255) NOT NULL,
    email varchar(320) NOT NULL,
    firebase_id varchar(128) NOT NULL,
    phone_number varchar(255) NOT NULL,
    photo_url text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (firebase_id),
    UNIQUE (email)
);

CREATE TYPE access_permission_type AS ENUM (
    'ADMIN',
    'MEMBER'
);

CREATE TABLE project
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name varchar(255) NOT NULL,
    description text NOT NULL,
    credits decimal NOT NULL DEFAULT 1.0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz NULL
);

CREATE TABLE user_project
(
    user_id uuid NOT NULL,
    project_id uuid NOT NULL,
    permission access_permission_type NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, project_id),
    FOREIGN KEY (user_id) REFERENCES user_account (id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES project (id) ON DELETE CASCADE
);

CREATE TABLE project_invitation
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email varchar(320) NOT NULL,
    project_id uuid NOT NULL,
    permission access_permission_type NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (project_id) REFERENCES project (id) ON DELETE CASCADE
);

CREATE TABLE application
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    description text NOT NULL,
    name varchar(255) NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz NULL,
    project_id uuid NOT NULL,
    FOREIGN KEY (project_id) REFERENCES project (id) ON DELETE CASCADE
);

CREATE TYPE model_vendor AS ENUM (
    'OPEN_AI',
    'COHERE'
);

CREATE TYPE model_type AS ENUM (
    'gpt-3.5-turbo',
    'gpt-3.5-turbo-16k',
    'gpt-4',
    'gpt-4-32k',
    'command',
    'command-light',
    'command-nightly',
    'command-light-nightly'
);

CREATE TABLE prompt_config
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name varchar(255) NOT NULL,
    model_parameters json NOT NULL,
    model_type model_type NOT NULL,
    model_vendor model_vendor NOT NULL,
    provider_prompt_messages json NOT NULL,
    expected_template_variables varchar(255) [] NOT NULL,
    is_default boolean NOT NULL DEFAULT TRUE,
    is_test_config boolean NOT NULL DEFAULT FALSE,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz NULL,
    application_id uuid NOT NULL,
    FOREIGN KEY (application_id) REFERENCES application (id) ON DELETE CASCADE,
    UNIQUE (name, application_id)
);

CREATE TABLE provider_model_pricing
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    model_type model_type NOT NULL,
    model_vendor model_vendor NOT NULL,
    input_token_price numeric NOT NULL,
    output_token_price numeric NOT NULL,
    token_unit_size int NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    active_from_date date NOT NULL DEFAULT current_date,
    active_to_date date NULL
);

CREATE TYPE prompt_finish_reason AS ENUM (
    'DONE',
    'ERROR',
    'LIMIT'
);

CREATE TABLE prompt_request_record
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    is_stream_response boolean NOT NULL DEFAULT FALSE,
    request_tokens int NOT NULL,
    response_tokens int NOT NULL,
    request_tokens_cost numeric NOT NULL,
    response_tokens_cost numeric NOT NULL,
    start_time timestamptz NOT NULL,
    finish_time timestamptz NOT NULL,
    finish_reason prompt_finish_reason NOT NULL DEFAULT 'DONE',
    duration_ms int NULL,
    prompt_config_id uuid NULL,
    error_log text NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz NULL,
    provider_model_pricing_id uuid NULL,
    FOREIGN KEY (provider_model_pricing_id) REFERENCES provider_model_pricing (id) ON DELETE CASCADE,
    FOREIGN KEY (prompt_config_id) REFERENCES prompt_config (id) ON DELETE CASCADE
);

CREATE TABLE prompt_test_record
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    variable_values json NOT NULL,
    response text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    prompt_request_record_id uuid NOT NULL,
    FOREIGN KEY (prompt_request_record_id) REFERENCES prompt_request_record (id) ON DELETE CASCADE
);

CREATE TABLE api_key
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name varchar(255) NOT NULL,
    is_internal boolean NOT NULL DEFAULT FALSE,
    created_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz NULL,
    application_id uuid NOT NULL,
    FOREIGN KEY (application_id) REFERENCES application (id) ON DELETE CASCADE
);

CREATE TABLE provider_key
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    model_vendor model_vendor NOT NULL,
    encrypted_api_key varchar(255) NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    project_id uuid NOT NULL,
    FOREIGN KEY (project_id) REFERENCES project (id) ON DELETE CASCADE
);
```

## Representative queries

### 1. Soft-delete pattern

```sql
-- @name DeleteProject
-- @returns :exec
UPDATE project
SET deleted_at = NOW()
WHERE id = $1;
```

### 2. Temporal query with soft-delete filter

```sql
-- @name RetrieveApplications
-- @returns :many
SELECT
    id,
    description,
    name,
    created_at,
    updated_at,
    project_id
FROM application
WHERE
    project_id = $1
    AND deleted_at IS NULL;
```

### 3. Aggregation with COALESCE

```sql
-- @name RetrieveApplicationTokensTotalCost
-- @returns :one
SELECT COALESCE(SUM(prr.request_tokens_cost + prr.response_tokens_cost), 0)
FROM application AS app
LEFT JOIN prompt_config AS pc ON app.id = pc.application_id
LEFT JOIN prompt_request_record AS prr ON pc.id = prr.prompt_config_id
WHERE
    app.id = $1
    AND prr.created_at BETWEEN $2 AND $3;
```

### 4. UPSERT with ON CONFLICT

```sql
-- @name UpsertProjectInvitationAdvanced
-- @returns :one
INSERT INTO project_invitation (email, project_id, permission)
VALUES ($1, $2, $3)
ON CONFLICT (email, project_id)
DO UPDATE SET permission = EXCLUDED.permission, updated_at = NOW()
RETURNING id, email, project_id, permission, created_at, updated_at;
```

### 5. Array column query

```sql
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
```

## Highlights

- **Soft-delete**: `deleted_at TIMESTAMPTZ` columns filtered with `AND deleted_at IS NULL`
- **Temporal queries**: `created_at` / `updated_at` for audit trails
- **Aggregations**: `COALESCE(SUM(...), 0)` over a `LEFT JOIN` chain so a project with no usage yet returns `0`, not `NULL`
- **UPSERT**: `ON CONFLICT ... DO UPDATE SET ... = EXCLUDED...` for idempotent invitations
- **JSON**: `model_parameters json` and `provider_prompt_messages json` for flexible configuration
- **Arrays**: `expected_template_variables varchar(255)[]` for list data without join tables
- **Enums**: `access_permission_type`, `model_vendor`, and `model_type` for type-safe status values
