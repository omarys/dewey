pub const CREATE_SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS series (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    sort_title TEXT,
    cover_path TEXT,
    status TEXT,
    fetch_url TEXT,
    metadata_json TEXT
);

CREATE TABLE IF NOT EXISTS chapters (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
    chapter_number REAL NOT NULL,
    file_path TEXT,
    page_count INTEGER,
    fetch_url TEXT,
    UNIQUE(series_id, chapter_number)
);

CREATE TABLE IF NOT EXISTS progress (
    chapter_id INTEGER PRIMARY KEY REFERENCES chapters(id) ON DELETE CASCADE,
    last_page_read INTEGER NOT NULL DEFAULT 0,
    is_completed INTEGER NOT NULL DEFAULT 0,
    last_read_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_chapters_series_id ON chapters(series_id);
CREATE INDEX IF NOT EXISTS idx_progress_last_read ON progress(last_read_at);
"#;
