PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS contacts (
  wa_id         TEXT PRIMARY KEY,            -- número en formato internacional sin '+'
  profile_name  TEXT,                        -- nombre que expone WhatsApp
  display_name  TEXT,                        -- nombre que edita el usuario (manda sobre profile_name)
  notes         TEXT,
  is_blocked    INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS conversations (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  wa_id           TEXT NOT NULL UNIQUE REFERENCES contacts(wa_id) ON DELETE CASCADE,
  last_message_at INTEGER,
  last_inbound_at INTEGER,                   -- base de la ventana de 24 h
  unread_count    INTEGER NOT NULL DEFAULT 0,
  is_archived     INTEGER NOT NULL DEFAULT 0,
  is_pinned       INTEGER NOT NULL DEFAULT 0,
  is_muted        INTEGER NOT NULL DEFAULT 0,
  created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_conv_last ON conversations(last_message_at DESC);

CREATE TABLE IF NOT EXISTS messages (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  wam_id          TEXT UNIQUE,               -- id de WhatsApp (wamid....)
  conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  wa_id           TEXT NOT NULL,
  direction       TEXT NOT NULL CHECK (direction IN ('in','out')),
  type            TEXT NOT NULL,             -- text|image|video|audio|document|sticker|location|contacts|template|interactive|reaction|order|system|unsupported
  body            TEXT,                      -- texto o caption
  status          TEXT NOT NULL DEFAULT 'pending', -- pending|sent|delivered|read|failed|received
  error_code      TEXT,
  error_message   TEXT,
  timestamp       INTEGER NOT NULL,          -- epoch ms
  context_wam_id  TEXT,                      -- mensaje citado
  reaction_emoji  TEXT,
  reaction_target TEXT,
  media_id        TEXT,                      -- id de media en Meta
  media_path      TEXT,                      -- ruta local relativa a MEDIA_DIR
  media_mime      TEXT,
  media_filename  TEXT,
  media_size      INTEGER,
  media_state     TEXT,                      -- pending|ready|error
  payload         TEXT,                      -- JSON original (location, contacts, order, interactive...)
  created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_msg_conv ON messages(conversation_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_msg_wam ON messages(wam_id);

CREATE TABLE IF NOT EXISTS labels (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT NOT NULL UNIQUE,
  color      TEXT NOT NULL DEFAULT '#25d366',
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS contact_labels (
  wa_id    TEXT NOT NULL REFERENCES contacts(wa_id) ON DELETE CASCADE,
  label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  PRIMARY KEY (wa_id, label_id)
);

-- Sustituto de los grupos: la Cloud API no soporta grupos de WhatsApp.
CREATE TABLE IF NOT EXISTS broadcast_lists (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS broadcast_members (
  list_id INTEGER NOT NULL REFERENCES broadcast_lists(id) ON DELETE CASCADE,
  wa_id   TEXT NOT NULL REFERENCES contacts(wa_id) ON DELETE CASCADE,
  PRIMARY KEY (list_id, wa_id)
);

CREATE TABLE IF NOT EXISTS auto_replies (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('welcome','away','keyword')),
  enabled     INTEGER NOT NULL DEFAULT 1,
  keyword     TEXT,                          -- para kind='keyword'
  match_type  TEXT NOT NULL DEFAULT 'contains' CHECK (match_type IN ('contains','equals','starts_with')),
  reply_text  TEXT NOT NULL,
  -- kind='away': minutos desde medianoche. Si start<=end la regla aplica
  -- dentro del rango; si start>end, aplica cruzando medianoche.
  active_from INTEGER,
  active_to   INTEGER,
  priority    INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Idempotencia de webhooks: Meta reintenta las entregas.
CREATE TABLE IF NOT EXISTS processed_events (
  key        TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL
);
