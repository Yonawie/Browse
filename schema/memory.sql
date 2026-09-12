-- Browse — memory.db schema (SQLite ≥ 3.45, WAL, foreign_keys=ON)
-- See docs/02-architecture.md §6 and docs/adr/ADR-004-memory-store.md.
-- All timestamps are INTEGER unix milliseconds, UTC.
-- Sensitivity classes: 'public' | 'personal' | 'private'. 'secret' is never persisted.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
  version     INTEGER PRIMARY KEY,
  applied_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  key         TEXT PRIMARY KEY,
  value_json  TEXT NOT NULL,
  updated_at  INTEGER NOT NULL
);

-- ---------------------------------------------------------------------------
-- Profiles: the user's browsing profile(s) and the isolated agent profile(s).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS profiles (
  id          TEXT PRIMARY KEY,                  -- uuid
  kind        TEXT NOT NULL CHECK (kind IN ('user','agent','private')),
  name        TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);

-- ---------------------------------------------------------------------------
-- Tasks: user-level units of work that group tabs, memory and agent sessions.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS tasks (
  id          TEXT PRIMARY KEY,
  title       TEXT NOT NULL,
  status      TEXT NOT NULL CHECK (status IN ('active','paused','done','archived')),
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

-- ---------------------------------------------------------------------------
-- Pages & versions: canonical page identity + content snapshots over time.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pages (
  id            TEXT PRIMARY KEY,
  url           TEXT NOT NULL,
  origin        TEXT NOT NULL,                   -- scheme://host[:port]
  domain        TEXT NOT NULL,                   -- registrable domain (eTLD+1)
  canonical_url TEXT,
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  never_remember INTEGER NOT NULL DEFAULT 0,     -- 1 => content is never stored/indexed
  UNIQUE (url)
);
CREATE INDEX IF NOT EXISTS idx_pages_domain ON pages(domain);
CREATE INDEX IF NOT EXISTS idx_pages_last_seen ON pages(last_seen_at);

CREATE TABLE IF NOT EXISTS page_versions (
  id            TEXT PRIMARY KEY,
  page_id       TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  content_hash  TEXT NOT NULL,                   -- blake3 of extracted main text
  title         TEXT,
  lang          TEXT,
  page_kind     TEXT NOT NULL DEFAULT 'unknown', -- article|product|form|app|search|doc|code|media|unknown
  sensitivity   TEXT NOT NULL CHECK (sensitivity IN ('public','personal','private')),
  main_text_zst BLOB,                            -- zstd-compressed extracted text (NULL if never_remember)
  summary       TEXT,
  captured_at   INTEGER NOT NULL,
  superseded_by TEXT REFERENCES page_versions(id) ON DELETE SET NULL,
  UNIQUE (page_id, content_hash)
);
CREATE INDEX IF NOT EXISTS idx_page_versions_page ON page_versions(page_id, captured_at);

-- ---------------------------------------------------------------------------
-- Visits: history. One row per navigation.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS visits (
  id              TEXT PRIMARY KEY,
  profile_id      TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  page_id         TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  page_version_id TEXT REFERENCES page_versions(id) ON DELETE SET NULL,
  task_id         TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  tab_id          TEXT,
  referrer_visit  TEXT REFERENCES visits(id) ON DELETE SET NULL,
  transition      TEXT NOT NULL,                 -- link|typed|omnibox_nl|agent|reload|back_forward|...
  started_at      INTEGER NOT NULL,
  dwell_ms        INTEGER,
  scroll_depth    REAL                           -- 0..1
);
CREATE INDEX IF NOT EXISTS idx_visits_started ON visits(started_at);
CREATE INDEX IF NOT EXISTS idx_visits_task ON visits(task_id);

-- ---------------------------------------------------------------------------
-- Chunks + vectors + FTS: the retrieval substrate.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS chunks (
  id              TEXT PRIMARY KEY,
  page_version_id TEXT NOT NULL REFERENCES page_versions(id) ON DELETE CASCADE,
  ordinal         INTEGER NOT NULL,
  heading_path    TEXT,                          -- "H1 > H2 > H3"
  text            TEXT NOT NULL,
  char_start      INTEGER NOT NULL,
  char_end        INTEGER NOT NULL,
  token_count     INTEGER NOT NULL,
  sensitivity     TEXT NOT NULL CHECK (sensitivity IN ('public','personal','private')),
  suspect_injection INTEGER NOT NULL DEFAULT 0,  -- signal for the critic; never a hard block
  month_bucket    INTEGER NOT NULL,              -- yyyymm, partition key mirrored into vec0
  domain          TEXT NOT NULL,
  UNIQUE (page_version_id, ordinal)
);
CREATE INDEX IF NOT EXISTS idx_chunks_month_domain ON chunks(month_bucket, domain);

-- Lexical index. External-content FTS5 table over chunks.text.
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
  text,
  heading_path,
  content='chunks',
  content_rowid='rowid',
  tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, text, heading_path) VALUES (new.rowid, new.text, new.heading_path);
END;
CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, text, heading_path) VALUES ('delete', old.rowid, old.text, old.heading_path);
END;
CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, text, heading_path) VALUES ('delete', old.rowid, old.text, old.heading_path);
  INSERT INTO chunks_fts(rowid, text, heading_path) VALUES (new.rowid, new.text, new.heading_path);
END;

-- Vector index (requires the sqlite-vec extension). Created by the memory crate
-- only when the extension is loaded; kept here for reference and for `sqlite3 -cmd '.load vec0'`.
--
-- CREATE VIRTUAL TABLE IF NOT EXISTS chunk_vectors USING vec0(
--   chunk_id      TEXT PRIMARY KEY,
--   month_bucket  INTEGER PARTITION KEY,
--   domain        TEXT,
--   task_id       TEXT,
--   embedding     FLOAT[256] distance_metric=cosine
-- );
-- Cascade for the virtual table (no FK support in vec0):
-- CREATE TRIGGER IF NOT EXISTS chunks_ad_vec AFTER DELETE ON chunks BEGIN
--   DELETE FROM chunk_vectors WHERE chunk_id = old.id;
-- END;

-- Fallback vector storage when sqlite-vec is unavailable (e.g. tests): brute force in Rust.
CREATE TABLE IF NOT EXISTS chunk_vectors_raw (
  chunk_id     TEXT PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
  model        TEXT NOT NULL,                    -- e.g. 'embeddinggemma-300m@256'
  dims         INTEGER NOT NULL,
  embedding    BLOB NOT NULL                     -- little-endian f32[dims]
);

-- ---------------------------------------------------------------------------
-- Knowledge graph: entities mentioned in chunks, and edges between them.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS entities (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL,                     -- person|org|project|repo|product|topic|place|other
  name        TEXT NOT NULL,
  normalized  TEXT NOT NULL,                     -- lowercased/normalized for dedup
  created_at  INTEGER NOT NULL,
  UNIQUE (kind, normalized)
);

CREATE TABLE IF NOT EXISTS entity_mentions (
  entity_id   TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  chunk_id    TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
  confidence  REAL NOT NULL,
  PRIMARY KEY (entity_id, chunk_id)
);

CREATE TABLE IF NOT EXISTS edges (
  id          TEXT PRIMARY KEY,
  src_id      TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  dst_id      TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  relation    TEXT NOT NULL,                     -- seen_on|compared_with|part_of|authored_by|...
  weight      REAL NOT NULL DEFAULT 1.0,
  evidence_chunk_id TEXT REFERENCES chunks(id) ON DELETE SET NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src_id);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst_id);

-- ---------------------------------------------------------------------------
-- Tabs and tab groups (current state; history lives in visits).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS tab_groups (
  id          TEXT PRIMARY KEY,
  task_id     TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  title       TEXT NOT NULL,
  auto        INTEGER NOT NULL DEFAULT 0,        -- 1 => created by attention layer
  created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tabs (
  id            TEXT PRIMARY KEY,
  profile_id    TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  group_id      TEXT REFERENCES tab_groups(id) ON DELETE SET NULL,
  page_id       TEXT REFERENCES pages(id) ON DELETE SET NULL,
  window_id     TEXT NOT NULL,
  position      INTEGER NOT NULL,
  pinned        INTEGER NOT NULL DEFAULT 0,
  last_active_at INTEGER NOT NULL,
  opened_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tabs_group ON tabs(group_id);

-- ---------------------------------------------------------------------------
-- User memory: facts/preferences. Provenance is restricted by CHECK.
-- Observations from pages live in chunks, never here (anti-poisoning).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS memories (
  id            TEXT PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN ('fact','preference','instruction','contact','credential_hint')),
  text          TEXT NOT NULL,
  provenance    TEXT NOT NULL CHECK (provenance IN ('user','confirmed')),
  source_chunk_id TEXT REFERENCES chunks(id) ON DELETE SET NULL, -- for 'confirmed' extracts
  sensitivity   TEXT NOT NULL CHECK (sensitivity IN ('public','personal','private')),
  created_at    INTEGER NOT NULL,
  superseded_by TEXT REFERENCES memories(id) ON DELETE SET NULL,
  deleted_at    INTEGER
);

-- ---------------------------------------------------------------------------
-- Agent: sessions, steps, actions, approvals, grants, policy decisions.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS agent_sessions (
  id            TEXT PRIMARY KEY,
  task_id       TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  profile_id    TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, -- normally kind='agent'
  request       TEXT NOT NULL,                   -- user's verbatim request
  scope_json    TEXT NOT NULL,                   -- TaskScope (schemas/task-scope.schema.json)
  mode          TEXT NOT NULL CHECK (mode IN ('live','dry_run')),
  status        TEXT NOT NULL CHECK (status IN ('planning','running','waiting_confirmation','paused','done','failed','cancelled')),
  planner_model TEXT,
  critic_model  TEXT,
  started_at    INTEGER NOT NULL,
  ended_at      INTEGER,
  outcome       TEXT
);

CREATE TABLE IF NOT EXISTS agent_steps (
  id            TEXT PRIMARY KEY,
  session_id    TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
  ordinal       INTEGER NOT NULL,
  observation_hash TEXT,                         -- hash of Observation the planner saw
  observation_tokens INTEGER,
  thought       TEXT,                            -- planner's visible reasoning summary (never page text)
  created_at    INTEGER NOT NULL,
  UNIQUE (session_id, ordinal)
);

-- Append-only journal of tool calls. UPDATE/DELETE are forbidden by triggers.
CREATE TABLE IF NOT EXISTS agent_actions (
  id              TEXT PRIMARY KEY,
  step_id         TEXT NOT NULL REFERENCES agent_steps(id) ON DELETE CASCADE,
  tool            TEXT NOT NULL,
  args_json       TEXT NOT NULL,                 -- arguments with provenance labels
  target_origin   TEXT,
  consequential   INTEGER NOT NULL DEFAULT 0,
  policy_verdict  TEXT NOT NULL CHECK (policy_verdict IN ('allow','confirm','deny')),
  policy_reason   TEXT,
  critic_verdict  TEXT CHECK (critic_verdict IN ('allow','confirm','deny')),
  critic_reason   TEXT,
  approval_id     TEXT,                          -- set when a confirmation was requested
  executed        INTEGER NOT NULL DEFAULT 0,
  result_json     TEXT,
  error           TEXT,
  before_hash     TEXT,
  after_hash      TEXT,
  created_at      INTEGER NOT NULL,
  duration_ms     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_agent_actions_step ON agent_actions(step_id);
CREATE TRIGGER IF NOT EXISTS agent_actions_no_update BEFORE UPDATE ON agent_actions BEGIN
  SELECT RAISE(ABORT, 'agent_actions is append-only');
END;
CREATE TRIGGER IF NOT EXISTS agent_actions_no_delete BEFORE DELETE ON agent_actions
WHEN (SELECT value_json FROM settings WHERE key = 'journal.allow_purge') IS NOT 'true' BEGIN
  SELECT RAISE(ABORT, 'agent_actions is append-only (set journal.allow_purge to purge)');
END;

CREATE TABLE IF NOT EXISTS approvals (
  id            TEXT PRIMARY KEY,
  session_id    TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
  kind          TEXT NOT NULL,                   -- consequential|cross_origin_flow|out_of_scope|session_import|tool_enable
  preview_json  TEXT NOT NULL,                   -- what the user was shown
  decision      TEXT CHECK (decision IN ('approved','rejected','expired')),
  requested_at  INTEGER NOT NULL,
  decided_at    INTEGER
);

-- Time-limited permissions granted by the user to a session/tool.
CREATE TABLE IF NOT EXISTS grants (
  id            TEXT PRIMARY KEY,
  session_id    TEXT REFERENCES agent_sessions(id) ON DELETE CASCADE,
  kind          TEXT NOT NULL,                   -- session_import|cross_origin_flow|origin_scope|tool|iframe_observe|profile_readonly
  subject       TEXT NOT NULL,                   -- e.g. tool name or 'agent'
  object        TEXT NOT NULL,                   -- e.g. origin, "originA->originB", capability
  granted_at    INTEGER NOT NULL,
  expires_at    INTEGER NOT NULL,
  revoked_at    INTEGER
);
CREATE INDEX IF NOT EXISTS idx_grants_active ON grants(kind, object, expires_at);

CREATE TABLE IF NOT EXISTS policy_decisions (
  id            TEXT PRIMARY KEY,
  session_id    TEXT REFERENCES agent_sessions(id) ON DELETE CASCADE,
  action_id     TEXT REFERENCES agent_actions(id) ON DELETE SET NULL,
  rule          TEXT NOT NULL,                   -- which rule fired
  verdict       TEXT NOT NULL CHECK (verdict IN ('allow','confirm','deny')),
  details_json  TEXT,
  created_at    INTEGER NOT NULL
);

-- ---------------------------------------------------------------------------
-- Recorded scenarios (macros) by semantic paths, parameterized.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS scenarios (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  description   TEXT,
  params_schema_json TEXT,
  scope_json    TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS scenario_steps (
  id            TEXT PRIMARY KEY,
  scenario_id   TEXT NOT NULL REFERENCES scenarios(id) ON DELETE CASCADE,
  ordinal       INTEGER NOT NULL,
  tool          TEXT NOT NULL,
  args_template_json TEXT NOT NULL,              -- semantic paths + {{param}} placeholders
  consequential INTEGER NOT NULL DEFAULT 0,
  UNIQUE (scenario_id, ordinal)
);

-- ---------------------------------------------------------------------------
-- Local audit of model usage (never leaves the device).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS model_calls (
  id            TEXT PRIMARY KEY,
  session_id    TEXT REFERENCES agent_sessions(id) ON DELETE SET NULL,
  purpose       TEXT NOT NULL,                   -- summarize|extract|plan|critic|embed|intent|qa|...
  provider      TEXT NOT NULL,                   -- llama-server|ollama|openai|anthropic|gemini|apple-fm|onnx
  model         TEXT NOT NULL,
  locality      TEXT NOT NULL CHECK (locality IN ('local','cloud')),
  sensitivity   TEXT NOT NULL CHECK (sensitivity IN ('public','personal','private')),
  input_tokens  INTEGER,
  output_tokens INTEGER,
  latency_ms    INTEGER,
  cost_usd      REAL,
  cache_hit     INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_model_calls_created ON model_calls(created_at);
