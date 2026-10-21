-- Maya Mini v0.1: Initial Database Schema (30 tables)

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK(version > 0),
    sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
    applied_at INTEGER NOT NULL
) STRICT;

CREATE TABLE settings (
    key TEXT PRIMARY KEY CHECK(key IN ('identity', 'voice', 'assistant', 'security', 'browser', 'policy_version')),
    value_json TEXT NOT NULL CHECK(json_valid(value_json)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE providers (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL CHECK(length(label) BETWEEN 1 AND 80),
    kind TEXT NOT NULL CHECK(kind IN ('openai_responses', 'openai_compatible_chat', 'gemini', 'anthropic')),
    protocol TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    local_endpoint_allowed INTEGER NOT NULL DEFAULT 0 CHECK(local_endpoint_allowed IN (0, 1)),
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    billing_mode TEXT NOT NULL CHECK(billing_mode IN ('api', 'subscription_cli', 'unknown')),
    secret_ref TEXT,
    credential_generation INTEGER NOT NULL DEFAULT 0 CHECK(credential_generation >= 0),
    timeout_ms INTEGER NOT NULL CHECK(timeout_ms BETWEEN 10000 AND 300000),
    capability_json TEXT NOT NULL CHECK(json_valid(capability_json)),
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE models (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    model_id TEXT NOT NULL CHECK(length(model_id) BETWEEN 1 AND 200),
    label TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    capabilities_json TEXT NOT NULL CHECK(json_valid(capabilities_json)),
    context_limit_bytes INTEGER NOT NULL CHECK(context_limit_bytes BETWEEN 1024 AND 131072),
    revision INTEGER NOT NULL CHECK(revision > 0),
    UNIQUE(provider_id, model_id)
) STRICT;

CREATE TABLE tools (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('codex', 'compatible_cli', 'editor', 'chromium', 'pdf', 'clipboard', 'tts')),
    executable TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    adapter_version TEXT NOT NULL,
    executable_sha256 TEXT,
    capability_json TEXT NOT NULL CHECK(json_valid(capability_json)),
    revision INTEGER NOT NULL CHECK(revision > 0),
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE model_roles (
    role TEXT PRIMARY KEY CHECK(role IN ('chat', 'planning', 'review', 'coding', 'fallback', 'stt', 'tts')),
    model_id TEXT REFERENCES models(id) ON DELETE RESTRICT,
    tool_id TEXT REFERENCES tools(id) ON DELETE RESTRICT,
    fallback_json TEXT NOT NULL CHECK(json_valid(fallback_json)),
    allow_paid_fallback INTEGER NOT NULL CHECK(allow_paid_fallback IN (0, 1)),
    timeout_ms INTEGER NOT NULL CHECK(timeout_ms BETWEEN 10000 AND 300000),
    revision INTEGER NOT NULL CHECK(revision > 0),
    CHECK(role = 'coding' OR NOT(model_id IS NOT NULL AND tool_id IS NOT NULL))
) STRICT;

CREATE TABLE chains (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 80),
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE chain_steps (
    chain_id TEXT NOT NULL REFERENCES chains(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK(position BETWEEN 1 AND 8),
    purpose TEXT NOT NULL CHECK(purpose IN ('research', 'review', 'plan', 'code')),
    model_id TEXT REFERENCES models(id) ON DELETE RESTRICT,
    tool_id TEXT REFERENCES tools(id) ON DELETE RESTRICT,
    max_output_bytes INTEGER NOT NULL CHECK(max_output_bytes BETWEEN 1024 AND 65536),
    PRIMARY KEY(chain_id, position),
    CHECK(model_id IS NOT NULL OR tool_id IS NOT NULL)
) STRICT;

CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 120),
    root TEXT NOT NULL UNIQUE,
    root_device INTEGER NOT NULL,
    root_inode INTEGER NOT NULL,
    allowed_paths_json TEXT NOT NULL CHECK(json_valid(allowed_paths_json)),
    excluded_paths_json TEXT NOT NULL CHECK(json_valid(excluded_paths_json)),
    notes TEXT NOT NULL CHECK(length(notes) <= 8192),
    preferred_model_id TEXT REFERENCES models(id) ON DELETE RESTRICT,
    preferred_tool_id TEXT REFERENCES tools(id) ON DELETE RESTRICT,
    archived INTEGER NOT NULL CHECK(archived IN (0, 1)),
    revision INTEGER NOT NULL CHECK(revision > 0),
    mutation_job_id TEXT UNIQUE REFERENCES jobs(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE command_specs (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    label TEXT NOT NULL,
    executable TEXT NOT NULL,
    args_json TEXT NOT NULL CHECK(json_valid(args_json)),
    kind TEXT NOT NULL CHECK(kind IN ('test', 'build', 'read', 'custom')),
    timeout_ms INTEGER NOT NULL CHECK(timeout_ms BETWEEN 1000 AND 1200000),
    network INTEGER NOT NULL CHECK(network = 0),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0, 1)),
    revision INTEGER NOT NULL CHECK(revision > 0)
) STRICT;

CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT,
    model_override_id TEXT REFERENCES models(id) ON DELETE RESTRICT,
    current_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    summary TEXT NOT NULL DEFAULT '' CHECK(length(summary) <= 8192),
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('owner', 'assistant', 'system')),
    text TEXT NOT NULL CHECK(length(text) <= 65536),
    model_id TEXT REFERENCES models(id) ON DELETE SET NULL,
    job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    intent TEXT CHECK(intent IN ('question', 'task', 'reminder', 'project_work', 'system_action', 'clarify')),
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE memory_items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('preference', 'decision', 'note', 'summary')),
    project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    text TEXT NOT NULL CHECK(length(text) BETWEEN 1 AND 8192),
    source TEXT NOT NULL CHECK(source IN ('owner', 'conversation_summary', 'task_event')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    parent_task_id TEXT REFERENCES tasks(id) ON DELETE RESTRICT,
    project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 200),
    description TEXT NOT NULL CHECK(length(description) <= 8192),
    priority TEXT NOT NULL CHECK(priority IN ('low', 'normal', 'high', 'urgent')),
    status TEXT NOT NULL CHECK(status IN ('todo', 'in_progress', 'blocked', 'done', 'cancelled')),
    progress_pct INTEGER CHECK(progress_pct BETWEEN 0 AND 100),
    progress_source TEXT CHECK(progress_source IN ('manual', 'observed', 'verified')),
    due_at INTEGER,
    timezone TEXT NOT NULL,
    estimated_minutes INTEGER CHECK(estimated_minutes BETWEEN 1 AND 525600),
    completed_at INTEGER,
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK(parent_task_id IS NULL OR parent_task_id <> id),
    CHECK((progress_pct IS NULL) = (progress_source IS NULL)),
    CHECK((status = 'done') = (completed_at IS NOT NULL))
) STRICT;

CREATE TABLE task_checklist (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    text TEXT NOT NULL CHECK(length(text) BETWEEN 1 AND 500),
    done INTEGER NOT NULL CHECK(done IN (0, 1)),
    position INTEGER NOT NULL CHECK(position BETWEEN 1 AND 100),
    UNIQUE(task_id, position)
) STRICT;

CREATE TABLE task_events (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('note', 'decision', 'blocker', 'status', 'progress', 'reschedule', 'evidence', 'checkin')),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    evidence_artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    actor_json TEXT NOT NULL CHECK(json_valid(actor_json)),
    at INTEGER NOT NULL
) STRICT;

CREATE TABLE schedules (
    id TEXT PRIMARY KEY,
    task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('reminder', 'checkin', 'daily_review')),
    label TEXT NOT NULL CHECK(length(label) BETWEEN 1 AND 200),
    first_at INTEGER NOT NULL,
    timezone TEXT NOT NULL,
    recurrence TEXT NOT NULL CHECK(recurrence IN ('once', 'daily', 'weekdays', 'weekly')),
    weekdays_json TEXT NOT NULL CHECK(json_valid(weekdays_json)),
    local_time TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    next_at INTEGER,
    revision INTEGER NOT NULL CHECK(revision > 0),
    CHECK(enabled = 0 OR next_at IS NOT NULL)
) STRICT;

CREATE TABLE occurrences (
    id TEXT PRIMARY KEY,
    schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    scheduled_at INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending', 'delivered', 'missed', 'failed')),
    notification_status TEXT NOT NULL CHECK(notification_status IN ('not_attempted', 'accepted', 'unavailable', 'failed', 'unknown')),
    attempted_at INTEGER,
    delivered_at INTEGER,
    UNIQUE(schedule_id, scheduled_at)
) STRICT;

CREATE TABLE checkins (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    occurrence_id TEXT UNIQUE REFERENCES occurrences(id) ON DELETE SET NULL,
    requested_at INTEGER NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    responded_at INTEGER,
    percent INTEGER CHECK(percent BETWEEN 0 AND 100),
    blocker TEXT CHECK(length(blocker) <= 2000),
    note TEXT CHECK(length(note) <= 4000),
    CHECK(responded_at IS NOT NULL OR (percent IS NULL AND blocker IS NULL AND note IS NULL))
) STRICT;

CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('coding', 'chain', 'chat', 'operation')),
    task_id TEXT REFERENCES tasks(id) ON DELETE RESTRICT,
    project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    goal TEXT NOT NULL CHECK(length(goal) BETWEEN 1 AND 8192),
    state TEXT NOT NULL CHECK(state IN ('created', 'inspecting', 'reviewing', 'planned', 'awaiting_approval', 'approved', 'checkpointing', 'executing', 'verifying', 'promoting', 'reporting', 'completed', 'completed_with_issues', 'blocked', 'paused', 'failed', 'cancelling', 'cancelled', 'needs_reconciliation')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    plan_id TEXT REFERENCES plans(id) ON DELETE RESTRICT,
    checkpoint_id TEXT REFERENCES artifacts(id) ON DELETE RESTRICT,
    verification TEXT NOT NULL CHECK(verification IN ('not_run', 'passed', 'failed', 'not_configured', 'not_applicable')),
    paused_from TEXT,
    error_code TEXT,
    input_json TEXT NOT NULL CHECK(json_valid(input_json)),
    cancellation_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancellation_requested IN (0, 1)),
    pause_requested INTEGER NOT NULL DEFAULT 0 CHECK(pause_requested IN (0, 1)),
    continuation TEXT CHECK(continuation IN ('review', 'checkpoint', 'promote', 'operation', 'voice_receive', 'report')),
    approval_purpose TEXT CHECK(approval_purpose IN ('context', 'implementation', 'destructive', 'verification_waiver', 'operation', 'voice')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE job_steps (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK(position > 0),
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending', 'running', 'succeeded', 'failed', 'cancelled', 'uncertain', 'skipped')),
    attempt INTEGER NOT NULL CHECK(attempt BETWEEN 0 AND 3),
    idempotent INTEGER NOT NULL CHECK(idempotent IN (0, 1)),
    started_at INTEGER,
    finished_at INTEGER,
    artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    error_code TEXT,
    UNIQUE(job_id, position, attempt)
) STRICT;

CREATE TABLE plans (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK(revision > 0),
    body_json TEXT NOT NULL CHECK(json_valid(body_json)),
    digest TEXT NOT NULL CHECK(length(digest) = 64),
    project_revision INTEGER NOT NULL,
    baseline_digest TEXT NOT NULL CHECK(length(baseline_digest) = 64),
    created_at INTEGER NOT NULL,
    UNIQUE(job_id, revision)
) STRICT;

CREATE TABLE approvals (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE RESTRICT,
    plan_id TEXT REFERENCES plans(id) ON DELETE RESTRICT,
    action TEXT NOT NULL,
    request_digest TEXT NOT NULL CHECK(length(request_digest) = 64),
    scope_json TEXT NOT NULL CHECK(json_valid(scope_json)),
    policy_version INTEGER NOT NULL,
    decision TEXT NOT NULL CHECK(decision IN ('pending', 'approved', 'denied', 'revoked', 'consumed')),
    requested_at INTEGER NOT NULL,
    decided_at INTEGER,
    expires_at INTEGER NOT NULL,
    deciding_session_id TEXT,
    revision INTEGER NOT NULL CHECK(revision > 0),
    CHECK(expires_at > requested_at),
    CHECK(decision = 'pending' OR decided_at IS NOT NULL)
) STRICT;

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY,
    job_id TEXT REFERENCES jobs(id) ON DELETE RESTRICT,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK(kind IN ('scan', 'context', 'plan', 'review', 'handoff', 'checkpoint', 'changes', 'diff', 'test', 'log', 'report', 'export', 'audio')),
    storage_name TEXT NOT NULL UNIQUE,
    sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    mime TEXT NOT NULL,
    provenance_json TEXT NOT NULL CHECK(json_valid(provenance_json)),
    redacted INTEGER NOT NULL CHECK(redacted IN (0, 1)),
    expires_at INTEGER,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE audit_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    actor_json TEXT NOT NULL CHECK(json_valid(actor_json)),
    action TEXT NOT NULL,
    resource_kind TEXT NOT NULL,
    resource_id TEXT,
    decision TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json) AND length(metadata_json) <= 16384),
    at INTEGER NOT NULL
) STRICT;

CREATE TABLE usage_records (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    model_id TEXT REFERENCES models(id) ON DELETE SET NULL,
    job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    request_id TEXT NOT NULL UNIQUE,
    input_tokens INTEGER CHECK(input_tokens >= 0),
    output_tokens INTEGER CHECK(output_tokens >= 0),
    cost_coefficient INTEGER CHECK(cost_coefficient >= 0),
    cost_scale INTEGER CHECK(cost_scale BETWEEN 0 AND 9),
    currency TEXT,
    cost_source TEXT CHECK(cost_source IN ('reported', 'user_rate_estimate')),
    billing_mode TEXT NOT NULL CHECK(billing_mode IN ('api', 'subscription_cli', 'unknown')),
    status TEXT NOT NULL CHECK(status IN ('succeeded', 'failed', 'cancelled', 'unknown')),
    at INTEGER NOT NULL,
    CHECK((cost_coefficient IS NULL AND cost_scale IS NULL AND currency IS NULL AND cost_source IS NULL) OR
          (cost_coefficient IS NOT NULL AND cost_scale IS NOT NULL AND currency IS NOT NULL AND cost_source IS NOT NULL))
) STRICT;

CREATE TABLE process_leases (
    id TEXT PRIMARY KEY,
    job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    category TEXT NOT NULL CHECK(category IN ('core', 'coding', 'browser', 'editor', 'voice', 'helper')),
    pid INTEGER CHECK(pid > 0),
    start_ticks INTEGER,
    pgid INTEGER,
    boot_id TEXT NOT NULL,
    owned INTEGER NOT NULL CHECK(owned IN (0, 1)),
    state TEXT NOT NULL CHECK(state IN ('starting', 'running', 'exited', 'unknown')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(boot_id, pid, start_ticks),
    CHECK(state = 'starting' OR (pid IS NOT NULL AND start_ticks IS NOT NULL AND pgid IS NOT NULL))
) STRICT;

CREATE TABLE workspace_changes (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE RESTRICT,
    checkpoint_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK(ordinal > 0),
    path TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('create', 'modify', 'delete')),
    preimage TEXT,
    postimage TEXT,
    content_artifact_id TEXT REFERENCES artifacts(id) ON DELETE RESTRICT,
    recovery_path TEXT,
    displaced_sha256 TEXT,
    displaced_artifact_id TEXT REFERENCES artifacts(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK(state IN ('prepared', 'applying', 'applied', 'uncertain', 'restored', 'conflict')),
    updated_at INTEGER NOT NULL,
    UNIQUE(job_id, ordinal),
    UNIQUE(job_id, path),
    CHECK((kind = 'create' AND preimage IS NULL AND postimage IS NOT NULL) OR
          (kind = 'modify' AND preimage IS NOT NULL AND postimage IS NOT NULL) OR
          (kind = 'delete' AND preimage IS NOT NULL AND postimage IS NULL))
) STRICT;

CREATE TABLE idempotency_keys (
    session_id TEXT NOT NULL,
    key TEXT NOT NULL CHECK(length(key) BETWEEN 16 AND 128),
    request_digest TEXT NOT NULL CHECK(length(request_digest) = 64),
    state TEXT NOT NULL CHECK(state IN ('pending', 'complete', 'uncertain')),
    response_json TEXT CHECK(response_json IS NULL OR json_valid(response_json)),
    status_code INTEGER,
    resource_id TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY(session_id, key)
) STRICT;

CREATE TABLE backups (
    id TEXT PRIMARY KEY,
    storage_name TEXT NOT NULL UNIQUE,
    schema_version INTEGER NOT NULL,
    manifest_sha256 TEXT NOT NULL CHECK(length(manifest_sha256) = 64),
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    state TEXT NOT NULL CHECK(state IN ('creating', 'verified', 'failed')),
    created_at INTEGER NOT NULL,
    verified_at INTEGER
) STRICT;

-- Indexes
CREATE INDEX idx_models_provider ON models(provider_id);
CREATE INDEX idx_tasks_status_due ON tasks(status, due_at, id);
CREATE INDEX idx_tasks_project_parent ON tasks(project_id, parent_task_id);
CREATE INDEX idx_task_events_time ON task_events(task_id, at, id);
CREATE INDEX idx_schedules_next ON schedules(enabled, next_at);
CREATE INDEX idx_occurrences_state ON occurrences(state, scheduled_at);
CREATE INDEX idx_checkins_task ON checkins(task_id, requested_at);
CREATE INDEX idx_jobs_state_time ON jobs(state, updated_at, id);
CREATE UNIQUE INDEX idx_one_project_writer ON jobs(project_id) WHERE project_id IS NOT NULL AND state IN ('checkpointing', 'executing', 'verifying', 'promoting');
CREATE INDEX idx_messages_cursor ON messages(conversation_id, created_at, id);
CREATE INDEX idx_memory_scope ON memory_items(project_id, kind, updated_at, id);
CREATE INDEX idx_approvals_job ON approvals(job_id, decision, expires_at);
CREATE INDEX idx_artifacts_job ON artifacts(job_id, created_at);
CREATE INDEX idx_audit_job_seq ON audit_events(job_id, seq);
CREATE INDEX idx_audit_time ON audit_events(at, seq);
CREATE INDEX idx_usage_time ON usage_records(provider_id, at);
CREATE INDEX idx_idempotency_expiry ON idempotency_keys(expires_at);
CREATE INDEX idx_changes_state ON workspace_changes(job_id, state);

-- Triggers
CREATE TRIGGER plans_no_update BEFORE UPDATE ON plans BEGIN
    SELECT RAISE(ABORT, 'immutable plan');
END;

CREATE TRIGGER approvals_no_rebind BEFORE UPDATE OF job_id, plan_id, action, request_digest, scope_json, policy_version, requested_at, expires_at ON approvals BEGIN
    SELECT RAISE(ABORT, 'immutable approval request');
END;

-- Seed default settings and initial model roles
INSERT INTO settings (key, value_json, revision, updated_at) VALUES
('identity', '{"name":"Maya","greeting":"Hello, I am Maya Mini. How can I help you today?","language":"english","tone":"concise"}', 1, 0),
('voice', '{"stt":"disabled","stt_model_id":null,"tts":"disabled","tts_model_id":null,"voice":null,"input_device":null,"output_device":null,"rate":1.0,"muted":false,"language":"english"}', 1, 0),
('assistant', '{"timezone":"Asia/Kolkata","work_days":[1,2,3,4,5],"work_start":"09:00","work_end":"18:00","checkin_minutes":120,"daily_review_time":"17:30","notifications":true,"report_delivery":"in_app"}', 1, 0),
('security', '{"require_plan_approval":true,"critical_always_confirm":true,"chat_retention_days":90,"event_retention_days":365,"artifact_retention_days":30,"backup_keep":7,"storage_cap_bytes":1000000000}', 1, 0),
('browser', '{"enabled":false,"executable":null,"profile_dir":null,"cdp_mode":"disabled","cdp_url":null}', 1, 0),
('policy_version', '{"config_revision":1,"policy_version":1}', 1, 0);

INSERT INTO model_roles (role, model_id, tool_id, fallback_json, allow_paid_fallback, timeout_ms, revision) VALUES
('chat', null, null, '[]', 0, 60000, 1),
('planning', null, null, '[]', 0, 120000, 1),
('review', null, null, '[]', 0, 120000, 1),
('coding', null, null, '[]', 0, 300000, 1),
('fallback', null, null, '[]', 0, 60000, 1),
('stt', null, null, '[]', 0, 60000, 1),
('tts', null, null, '[]', 0, 60000, 1);
