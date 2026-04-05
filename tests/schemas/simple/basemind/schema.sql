-- user
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

-- permission_type
CREATE TYPE access_permission_type AS ENUM (
    'ADMIN',
    'MEMBER'
);

-- project
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
CREATE INDEX idx_project_name ON project (name) WHERE deleted_at IS NULL;

-- user-project many-to-many
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

CREATE INDEX idx_user_project_user_id ON user_project (user_id);
CREATE INDEX idx_user_project_project_id ON user_project (project_id);

-- project-invitation many-to-many
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
CREATE UNIQUE INDEX idx_project_invitation_email_project_id ON project_invitation (email, project_id);


-- application
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

CREATE INDEX idx_application_project_id ON application (project_id) WHERE deleted_at IS NULL;

-- model_vendor
CREATE TYPE model_vendor AS ENUM (
    'OPEN_AI',
    'COHERE'
);

-- model_type
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

-- prompt-config
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

CREATE INDEX idx_prompt_config_application_id ON prompt_config (application_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_prompt_config_is_default ON prompt_config (is_default) WHERE deleted_at IS NULL;
CREATE INDEX idx_prompt_config_created_at ON prompt_config (created_at) WHERE deleted_at IS NULL;

-- provider-model-pricing
-- we intentionally keep this model denormalized because providers can and will change their prices over time.
-- therefore, the pricing of model use are time specific.
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

CREATE INDEX idx_provider_model_pricing_active_from_date ON provider_model_pricing (active_from_date);
CREATE INDEX idx_provider_model_pricing_active_to_date ON provider_model_pricing (active_to_date);
CREATE INDEX idx_provider_model_pricing_model_type ON provider_model_pricing (model_type);
CREATE INDEX idx_provider_model_pricing_model_vendor ON provider_model_pricing (model_vendor);

-- prompt-finish-reason
CREATE TYPE prompt_finish_reason AS ENUM (
    'DONE',
    'ERROR',
    'LIMIT'
);

-- prompt-request-record
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

CREATE INDEX idx_prompt_request_record_prompt_config_id ON prompt_request_record (
    prompt_config_id
) WHERE deleted_at IS NULL;
CREATE INDEX idx_prompt_request_record_pricing_id ON prompt_request_record (
    provider_model_pricing_id
) WHERE deleted_at IS NULL;
CREATE INDEX idx_prompt_request_record_start_time ON prompt_request_record (
    start_time
) WHERE deleted_at IS NULL;
CREATE INDEX idx_prompt_request_record_finish_time ON prompt_request_record (
    finish_time
) WHERE deleted_at IS NULL;

-- prompt-test-record
CREATE TABLE prompt_test_record
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    variable_values json NOT NULL,
    response text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    prompt_request_record_id uuid NOT NULL,
    FOREIGN KEY (prompt_request_record_id) REFERENCES prompt_request_record (id) ON DELETE CASCADE
);

CREATE INDEX idx_prompt_test_record_prompt_request_record_id ON prompt_test_record (
    prompt_request_record_id
);
CREATE INDEX idx_prompt_test_record_created_at ON prompt_test_record (created_at);

-- api-key
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
CREATE INDEX idx_api_key_application_id ON api_key (application_id) WHERE deleted_at IS NULL;

-- provider-key
CREATE TABLE provider_key
(
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    model_vendor model_vendor NOT NULL,
    encrypted_api_key varchar(255) NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    project_id uuid NOT NULL,
    FOREIGN KEY (project_id) REFERENCES project (id) ON DELETE CASCADE
);
CREATE INDEX idx_provider_key_project_id ON provider_key (project_id);
CREATE UNIQUE INDEX idx_provider_key_model_vendor_project_id ON provider_key (model_vendor, project_id);
