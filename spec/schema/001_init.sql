-- PinShot local storage. This file is the single source of truth: the Rust crate
-- compiles it in with `include_str!`, so the spec and the schema cannot drift.
--
-- Conventions
--   * ids are 128-bit stable ids, hex-encoded (see `pinshot-store::stable_id`).
--   * times are Unix seconds (UTC). `*_at` is always seconds unless suffixed `_ms`.
--   * every geometry value is a physical pixel, never a dip.
--   * blobs live outside SQLite, addressed by content hash: blobs/xx/<hash>.<ext>.

PRAGMA foreign_keys = ON;

-- A capture is one successful snip, scroll composite or imported image.
CREATE TABLE IF NOT EXISTS captures (
    id            TEXT PRIMARY KEY,
    created_at    INTEGER NOT NULL,
    kind          TEXT NOT NULL,           -- snip | scroll | pin | import
    width         INTEGER NOT NULL,        -- physical pixels
    height        INTEGER NOT NULL,
    scale         REAL NOT NULL DEFAULT 1.0,
    blob_hash     TEXT,                    -- lossless PNG of the final image
    thumb_hash    TEXT,                    -- longest side <= 320
    app_name      TEXT,                    -- frontmost app when captured
    bundle_id     TEXT,
    window_title  TEXT,
    display_id    TEXT,
    frame_count   INTEGER NOT NULL DEFAULT 1,
    favorite      INTEGER NOT NULL DEFAULT 0,
    deleted_at    INTEGER,
    CHECK (width > 0 AND height > 0),
    CHECK (kind IN ('snip', 'scroll', 'pin', 'import'))
);

CREATE INDEX IF NOT EXISTS captures_recent ON captures (created_at DESC);
CREATE INDEX IF NOT EXISTS captures_cleanup ON captures (favorite, created_at);
CREATE INDEX IF NOT EXISTS captures_app ON captures (app_name, created_at DESC);

-- Raw frames kept for scroll captures: reprocessing and "keep originals" recovery.
CREATE TABLE IF NOT EXISTS capture_frames (
    capture_id  TEXT NOT NULL REFERENCES captures (id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    offset_y    INTEGER NOT NULL DEFAULT 0,   -- accepted displacement from the previous frame
    blob_hash   TEXT NOT NULL,
    PRIMARY KEY (capture_id, seq)
);

-- Where a long capture was stitched. Useful to explain seams and to re-run a job.
CREATE TABLE IF NOT EXISTS scroll_jobs (
    id            TEXT PRIMARY KEY,
    capture_id    TEXT REFERENCES captures (id) ON DELETE SET NULL,
    started_at    INTEGER NOT NULL,
    finished_at   INTEGER,
    mode          TEXT NOT NULL,           -- auto | manual | window
    state         TEXT NOT NULL,           -- running | done | degraded | cancelled | failed
    container_app TEXT,
    step_px       INTEGER,
    confidence    REAL,
    seams         TEXT                     -- JSON array of { y, score }
);

CREATE TABLE IF NOT EXISTS pin_groups (
    name        TEXT PRIMARY KEY,
    created_at  INTEGER NOT NULL,
    sort        INTEGER NOT NULL DEFAULT 0
);

-- Live and recoverable pinned windows. A pin outlives its window while `state` is
-- 'closed'; rows are deleted when destroyed (or at exit for closed-but-unshown pins).
CREATE TABLE IF NOT EXISTS pins (
    id            TEXT PRIMARY KEY,
    capture_id    TEXT REFERENCES captures (id) ON DELETE SET NULL,
    group_name    TEXT REFERENCES pin_groups (name) ON DELETE SET NULL,
    created_at    INTEGER NOT NULL,
    state         TEXT NOT NULL,           -- visible | hidden | closed
    x             INTEGER NOT NULL DEFAULT 0,
    y             INTEGER NOT NULL DEFAULT 0,
    width         INTEGER NOT NULL,
    height        INTEGER NOT NULL,
    scale         REAL NOT NULL DEFAULT 1.0,
    opacity       REAL NOT NULL DEFAULT 1.0,
    rotation      INTEGER NOT NULL DEFAULT 0,   -- 0 | 90 | 180 | 270
    flip_h        INTEGER NOT NULL DEFAULT 0,
    flip_v        INTEGER NOT NULL DEFAULT 0,
    click_through INTEGER NOT NULL DEFAULT 0,
    thumbnail     INTEGER NOT NULL DEFAULT 0,
    display_id    TEXT,
    CHECK (state IN ('visible', 'hidden', 'closed')),
    CHECK (rotation IN (0, 90, 180, 270)),
    CHECK (opacity >= 0.1 AND opacity <= 1.0)
);

CREATE INDEX IF NOT EXISTS pins_state ON pins (state, created_at);
CREATE INDEX IF NOT EXISTS pins_group ON pins (group_name, created_at);

CREATE TABLE IF NOT EXISTS tags (
    name TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS capture_tags (
    capture_id  TEXT NOT NULL REFERENCES captures (id) ON DELETE CASCADE,
    tag         TEXT NOT NULL REFERENCES tags (name) ON DELETE CASCADE,
    PRIMARY KEY (capture_id, tag)
);

-- Settings are a key/value mirror of config.toml for things the UI changes at
-- runtime. config.toml wins on startup; the table records what the app actually ran
-- with, which makes bug reports reproducible.
CREATE TABLE IF NOT EXISTS settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- OCR text (P2). Separate from captures so the hot path never touches it, and so a
-- re-OCR can replace rows without rewriting the capture.
CREATE TABLE IF NOT EXISTS ocr_text (
    capture_id  TEXT PRIMARY KEY REFERENCES captures (id) ON DELETE CASCADE,
    text        TEXT NOT NULL,
    language    TEXT,
    engine      TEXT,
    created_at  INTEGER NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS ocr_fts USING fts5(
    text,
    content = 'ocr_text',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

PRAGMA user_version = 1;
