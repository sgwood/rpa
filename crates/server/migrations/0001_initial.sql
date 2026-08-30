CREATE TABLE IF NOT EXISTS enrollment_codes (
  code_hash TEXT PRIMARY KEY,
  expires_at TIMESTAMPTZ NOT NULL,
  max_uses INTEGER NOT NULL DEFAULT 1,
  uses INTEGER NOT NULL DEFAULT 0,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS devices (
  id TEXT PRIMARY KEY,
  alias TEXT NOT NULL,
  os TEXT NOT NULL,
  arch TEXT NOT NULL,
  hostname TEXT NOT NULL,
  logical_environment TEXT NOT NULL,
  node_version TEXT NOT NULL,
  token_hash TEXT NOT NULL,
  last_seen_at TIMESTAMPTZ NOT NULL,
  connected_at TIMESTAMPTZ,
  revoked_at TIMESTAMPTZ,
  record_json JSONB NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_devices_token_hash ON devices(token_hash);
CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen_at DESC);

CREATE TABLE IF NOT EXISTS adapters (
  device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  provider TEXT NOT NULL,
  last_event_at TIMESTAMPTZ,
  record_json JSONB NOT NULL,
  PRIMARY KEY(device_id, provider)
);

CREATE TABLE IF NOT EXISTS tasks (
  id UUID PRIMARY KEY,
  provider TEXT NOT NULL,
  device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  session_id TEXT NOT NULL,
  state TEXT NOT NULL,
  project TEXT,
  updated_at TIMESTAMPTZ NOT NULL,
  snapshot_json JSONB NOT NULL,
  UNIQUE(provider, device_id, session_id)
);
CREATE INDEX IF NOT EXISTS idx_central_tasks_updated ON tasks(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_central_tasks_state ON tasks(state, updated_at DESC);

CREATE TABLE IF NOT EXISTS events (
  event_id UUID PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  occurred_at TIMESTAMPTZ NOT NULL,
  event_type TEXT NOT NULL,
  event_json JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_central_events_task_time ON events(task_id, occurred_at);

CREATE TABLE IF NOT EXISTS commands (
  id UUID PRIMARY KEY,
  task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  action TEXT NOT NULL,
  state TEXT NOT NULL,
  body_ciphertext TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  record_json JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_central_commands_device ON commands(device_id, state, created_at);

CREATE TABLE IF NOT EXISTS audit_log (
  id UUID PRIMARY KEY,
  task_id UUID,
  command_id UUID,
  action TEXT NOT NULL,
  actor TEXT NOT NULL,
  summary TEXT NOT NULL,
  occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_central_audit_task ON audit_log(task_id, occurred_at);
