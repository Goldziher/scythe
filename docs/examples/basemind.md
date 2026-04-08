# BaseMind.AI

Real-world example from the [basemind.ai](https://github.com/basemind-ai/monorepo) open-source project (MIT license).

## Schema overview

13 tables with enums, arrays, JSONB, and soft-delete patterns:

```sql
CREATE TYPE model_vendor AS ENUM ('OPEN_AI', 'COHERE', 'ANTHROPIC');
CREATE TYPE access_permission AS ENUM ('ADMIN', 'MEMBER');

CREATE TABLE projects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);

CREATE TABLE applications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL,
    model_vendor model_vendor NOT NULL,
    model_parameters JSONB NOT NULL DEFAULT '{}',
    prompt_config_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);

CREATE TABLE prompt_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id UUID NOT NULL REFERENCES applications(id),
    name TEXT NOT NULL,
    model_parameters JSONB NOT NULL DEFAULT '{}',
    model_type TEXT NOT NULL,
    template_variables TEXT[] NOT NULL DEFAULT '{}',
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id UUID NOT NULL REFERENCES applications(id),
    name TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    is_internal BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);

CREATE TABLE project_members (
    project_id UUID NOT NULL REFERENCES projects(id),
    user_id UUID NOT NULL,
    permission access_permission NOT NULL DEFAULT 'MEMBER',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (project_id, user_id)
);
```

## Representative queries

### 1. Soft-delete pattern

```sql
-- @name SoftDeleteProject
-- @returns :exec
UPDATE projects SET deleted_at = NOW() WHERE id = $1 AND deleted_at IS NULL;
```

### 2. Temporal query with soft-delete filter

```sql
-- @name ListActiveApplications
-- @returns :many
SELECT a.id, a.name, a.model_vendor, a.created_at
FROM applications a
JOIN projects p ON p.id = a.project_id
WHERE a.project_id = $1
  AND a.deleted_at IS NULL
  AND p.deleted_at IS NULL
ORDER BY a.created_at DESC;
```

### 3. Aggregation with JSONB

```sql
-- @name CountApplicationsByVendor
-- @returns :many
SELECT model_vendor, COUNT(*) AS total
FROM applications
WHERE project_id = $1 AND deleted_at IS NULL
GROUP BY model_vendor
ORDER BY total DESC;
```

### 4. UPSERT with ON CONFLICT

```sql
-- @name UpsertProjectMember
-- @returns :one
INSERT INTO project_members (project_id, user_id, permission)
VALUES ($1, $2, $3)
ON CONFLICT (project_id, user_id) DO UPDATE SET permission = $3
RETURNING project_id, user_id, permission, created_at;
```

### 5. Array column query

```sql
-- @name GetPromptConfig
-- @returns :one
SELECT id, name, model_parameters, model_type, template_variables, is_default, created_at
FROM prompt_configs
WHERE id = $1;
```

## Highlights

- **Soft-delete**: `deleted_at TIMESTAMPTZ` columns filtered with `AND deleted_at IS NULL`
- **Temporal queries**: `created_at` / `updated_at` for audit trails
- **Aggregations**: `GROUP BY` with `COUNT(*)` for analytics
- **UPSERT**: `ON CONFLICT ... DO UPDATE` for idempotent operations
- **JSONB**: `model_parameters JSONB` for flexible configuration
- **Arrays**: `template_variables TEXT[]` for list data without join tables
- **Enums**: `model_vendor` and `access_permission` for type-safe status values
