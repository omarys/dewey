#![allow(dead_code)]

pub mod models;
pub mod schema;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use models::{Chapter, ChapterWithProgress, Progress, Series, SeriesStats, SeriesWithStats};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct ChapterScanEntry {
    pub chapter_number: f64,
    pub file_path: String,
    pub page_count: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ExistingChapterInfo {
    pub id: i64,
    pub chapter_number: f64,
    pub page_count: Option<i64>,
}

pub type ExistingChaptersMap = HashMap<String, ExistingChapterInfo>;

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_profile(path, crate::config::StorageProfile::Fast)
    }

    pub fn open_with_profile(path: &Path, profile: crate::config::StorageProfile) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create DB directory: {:?}", parent))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open SQLite database at {:?}", path))?;

        // Configure high-performance SQLite pragmas tailored to the storage medium
        let pragmas = match profile {
            crate::config::StorageProfile::Fast => {
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA cache_size = -64000;
                 PRAGMA mmap_size = 268435456;
                 PRAGMA temp_store = MEMORY;"
            }
            crate::config::StorageProfile::Usb => {
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA cache_size = -32000;
                 PRAGMA mmap_size = 0;
                 PRAGMA temp_store = MEMORY;"
            }
        };

        conn.execute_batch(pragmas)?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.init_schema()?;
        Ok(db)
    }

    pub fn in_memory() -> Result<Self> {
        let conn =
            Connection::open_in_memory().context("Failed to open in-memory SQLite database")?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(schema::CREATE_SCHEMA)
            .context("Failed to initialize database schema")?;

        // Run non-destructive column migrations if existing database is missing columns
        let _ = conn.execute("ALTER TABLE series ADD COLUMN fetch_url TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE series ADD COLUMN reading_mode TEXT DEFAULT 'webtoon'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE series ADD COLUMN is_hidden INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE series ADD COLUMN category TEXT", []);
        let _ = conn.execute("ALTER TABLE chapters ADD COLUMN fetch_url TEXT", []);
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_chapters_file_path ON chapters(file_path)",
            [],
        );

        // Repair corrupted progress: clamp last_page_read to the chapter's
        // page count where one is known (e.g. a reader once reported a global
        // page number far beyond a single chapter). Idempotent.
        let _ = conn.execute_batch(
            "UPDATE progress SET last_page_read = c.page_count
             FROM chapters c
             WHERE c.id = progress.chapter_id
               AND c.page_count IS NOT NULL AND c.page_count > 0
               AND progress.last_page_read > c.page_count;",
        );
        Ok(())
    }

    /// Single grouped SQL query retrieving all series and their computed statistics in ONE roundtrip
    pub fn get_all_series(&self) -> Result<Vec<SeriesWithStats>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT
                s.id, s.title, s.sort_title, s.cover_path, s.status, s.fetch_url, s.metadata_json, s.reading_mode, s.is_hidden, s.category,
                COUNT(c.id) AS total_count,
                SUM(CASE WHEN c.file_path IS NOT NULL AND c.file_path != '' THEN 1 ELSE 0 END) AS downloaded_count,
                SUM(CASE WHEN p.is_completed = 1 THEN 1 ELSE 0 END) AS completed_count,
                MAX(CASE WHEN p.last_read_at IS NOT NULL THEN c.chapter_number ELSE NULL END) AS latest_read_chap,
                MAX(p.last_read_at) AS latest_read_time
             FROM series s
             LEFT JOIN chapters c ON s.id = c.series_id
             LEFT JOIN progress p ON c.id = p.chapter_id
             GROUP BY s.id
             ORDER BY COALESCE(s.sort_title, s.title) ASC",
        )?;

        let series_list = stmt
            .query_map([], |row| {
                let is_hidden_int: i64 = row.get(8).unwrap_or(0);
                let series = Series {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    sort_title: row.get(2)?,
                    cover_path: row.get(3)?,
                    status: row.get(4)?,
                    fetch_url: row.get(5)?,
                    metadata_json: row.get(6)?,
                    reading_mode: row.get(7)?,
                    is_hidden: is_hidden_int != 0,
                    category: row.get(9)?,
                };

                let total_count: i64 = row.get(10).unwrap_or(0);
                let downloaded_count: i64 = row.get(11).unwrap_or(0);
                let completed_count: i64 = row.get(12).unwrap_or(0);
                let latest_read_chap: Option<f64> = row.get(13)?;
                let latest_read_time_str: Option<String> = row.get(14)?;

                let last_read_at = latest_read_time_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });

                let stats = SeriesStats {
                    total_chapters: total_count as usize,
                    downloaded_chapters: downloaded_count as usize,
                    completed_chapters: completed_count as usize,
                    latest_read_chapter: latest_read_chap,
                    last_read_at,
                };

                Ok(SeriesWithStats { series, stats })
            })?
            .collect::<Result<Vec<SeriesWithStats>, _>>()?;

        Ok(series_list)
    }

    pub fn get_chapters_for_series(&self, series_id: i64) -> Result<Vec<ChapterWithProgress>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT
                c.id, c.series_id, c.chapter_number, c.file_path, c.page_count, c.fetch_url,
                p.last_page_read, p.is_completed, p.last_read_at
             FROM chapters c
             LEFT JOIN progress p ON c.id = p.chapter_id
             WHERE c.series_id = ?1
             ORDER BY c.chapter_number ASC, c.file_path ASC",
        )?;

        let chapters = stmt
            .query_map(params![series_id], |row| {
                let chapter = Chapter {
                    id: row.get(0)?,
                    series_id: row.get(1)?,
                    chapter_number: row.get(2)?,
                    file_path: row.get(3)?,
                    page_count: row.get(4)?,
                    fetch_url: row.get(5)?,
                };

                let last_page_read: Option<i64> = row.get(6)?;
                let progress = if let Some(last_page) = last_page_read {
                    let is_completed_int: i64 = row.get(7).unwrap_or(0);
                    let last_read_at_str: Option<String> = row.get(8)?;
                    let last_read_at = last_read_at_str.and_then(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .map(|dt| dt.with_timezone(&Utc))
                            .ok()
                    });

                    Some(Progress {
                        chapter_id: chapter.id,
                        last_page_read: last_page,
                        is_completed: is_completed_int != 0,
                        last_read_at,
                    })
                } else {
                    None
                };

                Ok(ChapterWithProgress { chapter, progress })
            })?
            .collect::<Result<Vec<ChapterWithProgress>, _>>()?;

        Ok(chapters)
    }

    pub fn upsert_progress(&self, chapter_id: i64, last_page: i64, completed: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Enforce bounds: progress beyond a known page count is data
        // corruption (e.g. a reader report spanning several chapters).
        let page_count: Option<i64> = conn
            .query_row(
                "SELECT page_count FROM chapters WHERE id = ?1",
                params![chapter_id],
                |row| row.get(0),
            )
            .ok();
        let bounded = match page_count {
            Some(n) if n > 0 => last_page.clamp(0, n),
            _ => last_page.max(0),
        };

        let now_str = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO progress (chapter_id, last_page_read, is_completed, last_read_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chapter_id) DO UPDATE SET
                last_page_read = ?2,
                is_completed = ?3,
                last_read_at = ?4",
            params![chapter_id, bounded, if completed { 1 } else { 0 }, now_str],
        )?;

        Ok(())
    }

    /// Removes any progress row for the chapter, making it read as unread
    /// (page 0, not completed).
    pub fn delete_progress(&self, chapter_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM progress WHERE chapter_id = ?1",
            params![chapter_id],
        )?;
        Ok(())
    }

    /// Removes a series and, via FK ON DELETE CASCADE, all of its chapters
    /// and progress records.
    pub fn delete_series(&self, series_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM series WHERE id = ?1", params![series_id])?;
        Ok(())
    }

    /// Removes a single chapter and its progress record.
    pub fn delete_chapter(&self, chapter_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM chapters WHERE id = ?1", params![chapter_id])?;
        Ok(())
    }

    /// Wipes the on-disk SQLite database so the next open starts fresh.
    /// Also removes the WAL and SHM sidecar files SQLite creates alongside it.
    pub fn reset(path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        for file in [
            path_str.clone(),
            format!("{}-wal", path_str),
            format!("{}-shm", path_str),
        ] {
            if Path::new(&file).exists() {
                std::fs::remove_file(&file)?;
            }
        }
        Ok(())
    }

    pub fn toggle_completed(&self, chapter_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let current_completed: Option<i64> = conn
            .query_row(
                "SELECT is_completed FROM progress WHERE chapter_id = ?1",
                params![chapter_id],
                |r| r.get(0),
            )
            .ok();

        let new_completed = !matches!(current_completed, Some(1));

        let now_str = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO progress (chapter_id, last_page_read, is_completed, last_read_at)
             VALUES (?1, 0, ?2, ?3)
             ON CONFLICT(chapter_id) DO UPDATE SET
                is_completed = ?2,
                last_read_at = ?3",
            params![chapter_id, if new_completed { 1 } else { 0 }, now_str],
        )?;

        Ok(new_completed)
    }

    pub fn record_chapter_download(
        &self,
        series_id: i64,
        chapter_number: f64,
        file_path: &str,
        page_count: Option<i64>,
        fetch_url: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO chapters (series_id, chapter_number, file_path, page_count, fetch_url)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(series_id, chapter_number) DO UPDATE SET
                file_path = ?3,
                page_count = COALESCE(?4, page_count),
                fetch_url = COALESCE(?5, fetch_url)",
            params![series_id, chapter_number, file_path, page_count, fetch_url],
        )?;

        let chapter_id: i64 = conn.query_row(
            "SELECT id FROM chapters WHERE series_id = ?1 AND chapter_number = ?2",
            params![series_id, chapter_number],
            |r| r.get(0),
        )?;

        Ok(chapter_id)
    }

    /// Fast indexed lookup of known chapters for diffing during scans
    pub fn get_existing_chapters_by_path(&self, series_id: i64) -> Result<ExistingChaptersMap> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, chapter_number, file_path, page_count FROM chapters WHERE series_id = ?1 AND file_path IS NOT NULL",
        )?;

        let mut map = HashMap::new();
        let rows = stmt.query_map(params![series_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;

        for row in rows.flatten() {
            map.insert(
                row.2,
                ExistingChapterInfo {
                    id: row.0,
                    chapter_number: row.1,
                    page_count: row.3,
                },
            );
        }

        Ok(map)
    }

    pub fn update_chapter_number(&self, chapter_id: i64, chapter_number: f64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE chapters SET chapter_number = ?1 WHERE id = ?2",
            params![chapter_number, chapter_id],
        )?;
        Ok(())
    }

    /// Batch insert or update scanned chapters in a single atomic transaction.
    /// Reduces 1,000 separate disk syncs to 1 single transaction on slow storage.
    pub fn batch_record_chapters(
        &self,
        series_id: i64,
        chapters: &[ChapterScanEntry],
    ) -> Result<usize> {
        if chapters.is_empty() {
            return Ok(0);
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut count = 0;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO chapters (series_id, chapter_number, file_path, page_count, fetch_url)
                 VALUES (?1, ?2, ?3, ?4, NULL)
                 ON CONFLICT(series_id, chapter_number) DO UPDATE SET
                    file_path = ?3,
                    page_count = COALESCE(?4, page_count)",
            )?;

            for entry in chapters {
                stmt.execute(params![
                    series_id,
                    entry.chapter_number,
                    entry.file_path,
                    entry.page_count,
                ])?;
                count += 1;
            }
        }

        tx.commit()?;
        Ok(count)
    }

    pub fn update_series_fetch_url(&self, series_id: i64, fetch_url: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE series SET fetch_url = ?1 WHERE id = ?2",
            params![fetch_url, series_id],
        )?;
        Ok(())
    }

    pub fn update_chapter_fetch_url(&self, chapter_id: i64, fetch_url: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE chapters SET fetch_url = ?1 WHERE id = ?2",
            params![fetch_url, chapter_id],
        )?;
        Ok(())
    }

    pub fn update_series_cover(&self, series_id: i64, cover_path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE series SET cover_path = ?1 WHERE id = ?2",
            params![cover_path, series_id],
        )?;
        Ok(())
    }

    pub fn update_series_reading_mode(&self, series_id: i64, reading_mode: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE series SET reading_mode = ?1 WHERE id = ?2",
            params![reading_mode, series_id],
        )?;
        Ok(())
    }

    pub fn update_series_hidden(&self, series_id: i64, is_hidden: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE series SET is_hidden = ?1 WHERE id = ?2",
            params![if is_hidden { 1 } else { 0 }, series_id],
        )?;
        Ok(())
    }

    pub fn update_series_category(&self, series_id: i64, category: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE series SET category = ?1 WHERE id = ?2",
            params![category, series_id],
        )?;
        Ok(())
    }

    pub fn get_all_categories(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT category FROM series WHERE category IS NOT NULL AND category != '' ORDER BY category ASC",
        )?;
        let categories = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(categories)
    }

    pub fn rename_series_directory(
        &self,
        series_id: i64,
        old_dir: &Path,
        new_dir: &Path,
        is_hidden: bool,
    ) -> Result<()> {
        self.rename_series_directory_with_category(series_id, old_dir, new_dir, is_hidden, None)
    }

    pub fn rename_series_directory_with_category(
        &self,
        series_id: i64,
        old_dir: &Path,
        new_dir: &Path,
        is_hidden: bool,
        new_category: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let old_dir_str = old_dir.to_string_lossy().to_string();
        let new_dir_str = new_dir.to_string_lossy().to_string();

        // 1. Update series cover_path if it begins with old_dir_str
        let existing_cover: Option<String> = conn
            .query_row(
                "SELECT cover_path FROM series WHERE id = ?1",
                params![series_id],
                |r| r.get(0),
            )
            .ok()
            .flatten();

        if let Some(cover) = existing_cover {
            if cover.starts_with(&old_dir_str) {
                let updated_cover = cover.replacen(&old_dir_str, &new_dir_str, 1);
                let _ = conn.execute(
                    "UPDATE series SET cover_path = ?1 WHERE id = ?2",
                    params![updated_cover, series_id],
                );
            }
        }

        // 2. Update all chapter file_paths for this series that begin with old_dir_str
        let mut stmt = conn.prepare("SELECT id, file_path FROM chapters WHERE series_id = ?1")?;
        let rows = stmt
            .query_map(params![series_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        for (ch_id, file_path_opt) in rows {
            if let Some(fp) = file_path_opt {
                if fp.starts_with(&old_dir_str) {
                    let updated_fp = fp.replacen(&old_dir_str, &new_dir_str, 1);
                    let _ = conn.execute(
                        "UPDATE chapters SET file_path = ?1 WHERE id = ?2",
                        params![updated_fp, ch_id],
                    );
                }
            }
        }

        // 3. Update is_hidden flag and optionally category
        if let Some(cat) = new_category {
            conn.execute(
                "UPDATE series SET is_hidden = ?1, category = ?2 WHERE id = ?3",
                params![if is_hidden { 1 } else { 0 }, cat, series_id],
            )?;
        } else {
            conn.execute(
                "UPDATE series SET is_hidden = ?1 WHERE id = ?2",
                params![if is_hidden { 1 } else { 0 }, series_id],
            )?;
        }

        Ok(())
    }

    pub fn insert_or_get_series(&self, title: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        self.insert_or_get_series_inner(&conn, title, None, false, None)
    }

    pub fn insert_or_get_series_with_cover(
        &self,
        title: &str,
        cover_path: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        self.insert_or_get_series_inner(&conn, title, cover_path, false, None)
    }

    pub fn insert_or_get_series_with_cover_and_hidden(
        &self,
        title: &str,
        cover_path: Option<&str>,
        is_hidden: bool,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        self.insert_or_get_series_inner(&conn, title, cover_path, is_hidden, None)
    }

    pub fn insert_or_get_series_full(
        &self,
        title: &str,
        cover_path: Option<&str>,
        is_hidden: bool,
        category: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        self.insert_or_get_series_inner(&conn, title, cover_path, is_hidden, category)
    }

    fn insert_or_get_series_inner(
        &self,
        conn: &Connection,
        title: &str,
        cover_path: Option<&str>,
        is_hidden: bool,
        category: Option<&str>,
    ) -> Result<i64> {
        let existing: Option<(i64, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT id, cover_path, category FROM series WHERE title = ?1",
                params![title],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();

        if let Some((id, existing_cover, existing_cat)) = existing {
            if existing_cover.is_none() && cover_path.is_some() {
                let _ = conn.execute(
                    "UPDATE series SET cover_path = ?1 WHERE id = ?2",
                    params![cover_path, id],
                );
            }
            if existing_cat.is_none() && category.is_some() {
                let _ = conn.execute(
                    "UPDATE series SET category = ?1 WHERE id = ?2",
                    params![category, id],
                );
            }
            let _ = conn.execute(
                "UPDATE series SET is_hidden = ?1 WHERE id = ?2",
                params![if is_hidden { 1 } else { 0 }, id],
            );
            Ok(id)
        } else {
            conn.execute(
                "INSERT INTO series (title, sort_title, status, cover_path, is_hidden, category) VALUES (?1, ?1, 'Ongoing', ?2, ?3, ?4)",
                params![title, cover_path, if is_hidden { 1 } else { 0 }, category],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }

    pub fn seed_sample_data_if_empty(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM series", [], |r| r.get(0))?;

        if count == 0 {
            let series_id = self.insert_or_get_series_inner(
                &conn,
                "Solo Leveling",
                None,
                false,
                Some("Manhwa/Action"),
            )?;
            conn.execute(
                "UPDATE series SET
                    sort_title = 'Solo Leveling',
                    status = 'Completed',
                    fetch_url = 'https://example.com/manga/solo-leveling',
                    metadata_json = '{\"author\": \"Chugong\", \"artist\": \"DUBU\", \"genre\": [\"Action\", \"Fantasy\", \"Supernatural\"]}'
                 WHERE id = ?1",
                params![series_id],
            )?;

            for chap in 100..=108 {
                let chap_num = chap as f64;
                let file_path = if chap == 100 || chap == 101 {
                    Some(format!("/tmp/manga/solo_leveling_c{:03}.cbz", chap))
                } else {
                    None
                };
                let page_count = if file_path.is_some() { Some(48) } else { None };
                let fetch_url = if chap > 101 {
                    Some(format!(
                        "https://example.com/manga/solo-leveling/chapter-{}",
                        chap
                    ))
                } else {
                    None
                };

                conn.execute(
                    "INSERT INTO chapters (series_id, chapter_number, file_path, page_count, fetch_url)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![series_id, chap_num, file_path, page_count, fetch_url],
                )?;
            }

            let s2_id = self.insert_or_get_series_inner(
                &conn,
                "Chainsaw Man",
                None,
                false,
                Some("Manga/Action"),
            )?;
            conn.execute(
                "UPDATE series SET
                    sort_title = 'Chainsaw Man',
                    status = 'Ongoing',
                    fetch_url = NULL,
                    metadata_json = '{\"author\": \"Tatsuki Fujimoto\", \"genre\": [\"Action\", \"Horror\", \"Comedy\"]}'
                 WHERE id = ?1",
                params![s2_id],
            )?;

            for chap in 1..=6 {
                let chap_num = chap as f64;
                conn.execute(
                    "INSERT INTO chapters (series_id, chapter_number, file_path, page_count, fetch_url)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![s2_id, chap_num, None::<String>, None::<i64>, None::<String>],
                )?;
            }
        }

        Ok(())
    }

    pub fn update_series_status(&self, series_id: i64, status: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE series SET status = ?1 WHERE id = ?2",
            params![status, series_id],
        )?;
        Ok(())
    }

    pub fn update_series_metadata(&self, series_id: i64, metadata_json: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE series SET metadata_json = ?1 WHERE id = ?2",
            params![metadata_json, series_id],
        )?;
        Ok(())
    }

    /// Retrieves chapter ID and reading progress for a given file path if present
    pub fn get_progress_for_file(&self, file_path: &Path) -> Result<Option<(i64, Progress)>> {
        let conn = self.conn.lock().unwrap();
        let path_str = file_path.to_string_lossy();
        let mut stmt = conn.prepare(
            "SELECT c.id, p.last_page_read, p.is_completed, p.last_read_at
             FROM chapters c
             LEFT JOIN progress p ON c.id = p.chapter_id
             WHERE c.file_path = ?1",
        )?;

        let result = stmt
            .query_row(params![path_str.as_ref()], |row| {
                let chapter_id: i64 = row.get(0)?;
                let last_page: Option<i64> = row.get(1)?;
                let is_completed_int: Option<i64> = row.get(2)?;
                let last_read_at_str: Option<String> = row.get(3)?;

                let last_read_at = last_read_at_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });

                let progress = Progress {
                    chapter_id,
                    last_page_read: last_page.unwrap_or(0),
                    is_completed: is_completed_int.unwrap_or(0) != 0,
                    last_read_at,
                };

                Ok((chapter_id, progress))
            })
            .ok();

        Ok(result)
    }

    /// Gets or creates a chapter record for a standalone file path
    pub fn get_or_create_chapter_for_file(&self, file_path: &Path) -> Result<i64> {
        if let Ok(Some((chapter_id, _))) = self.get_progress_for_file(file_path) {
            return Ok(chapter_id);
        }

        let series_title = file_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().replace('_', " "))
            .unwrap_or_else(|| "Unknown Series".to_string());

        let chapter_number =
            crate::scanner::LibraryScanner::parse_chapter_number(file_path).unwrap_or(1.0);
        let page_count = crate::scanner::LibraryScanner::detect_page_count(file_path);

        let series_id = self.insert_or_get_series(&series_title)?;
        let path_str = file_path.to_string_lossy().to_string();

        self.record_chapter_download(series_id, chapter_number, &path_str, page_count, None)
    }

    /// Applies the reader's reported progress for one chapter file: resolves
    /// the chapter record (creating one if the library hasn't scanned it yet),
    /// persists the clamped progress, and returns its chapter id.
    pub fn apply_chapter_progress(
        &self,
        file_path: &Path,
        last_page: i64,
        completed: bool,
    ) -> Result<i64> {
        let chapter_id = match self.get_progress_for_file(file_path)? {
            Some((id, _)) => id,
            None => self.get_or_create_chapter_for_file(file_path)?,
        };
        self.upsert_progress(chapter_id, last_page, completed)?;
        Ok(chapter_id)
    }

    pub fn get_series_id_for_chapter(&self, chapter_id: i64) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        let sid = conn
            .query_row(
                "SELECT series_id FROM chapters WHERE id = ?1",
                params![chapter_id],
                |r| r.get(0),
            )
            .ok();
        Ok(sid)
    }

    pub fn get_series_reading_mode_for_chapter(&self, chapter_id: i64) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let mode: Option<String> = conn
            .query_row(
                "SELECT s.reading_mode FROM series s JOIN chapters c ON s.id = c.series_id WHERE c.id = ?1",
                params![chapter_id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        Ok(mode.unwrap_or_else(|| "webtoon".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_db_and_schema() {
        let db = Database::in_memory().unwrap();
        db.seed_sample_data_if_empty().unwrap();

        let series = db.get_all_series().unwrap();
        assert_eq!(series.len(), 2);

        let solo = series
            .iter()
            .find(|s| s.series.title == "Solo Leveling")
            .unwrap();
        assert_eq!(solo.stats.total_chapters, 9);
        assert_eq!(solo.stats.downloaded_chapters, 2);
        assert_eq!(
            solo.series.fetch_url,
            Some("https://example.com/manga/solo-leveling".to_string())
        );

        let chapters = db.get_chapters_for_series(solo.series.id).unwrap();
        assert_eq!(chapters.len(), 9);

        // Update progress
        let first_chap_id = chapters[0].chapter.id;
        db.upsert_progress(first_chap_id, 25, false).unwrap();

        let updated_chapters = db.get_chapters_for_series(solo.series.id).unwrap();
        let prog = updated_chapters[0].progress.as_ref().unwrap();
        assert_eq!(prog.last_page_read, 25);
        assert!(!prog.is_completed);

        // Toggle completed
        let is_completed = db.toggle_completed(first_chap_id).unwrap();
        assert!(is_completed);

        let reloaded_series = db.get_all_series().unwrap();
        let solo_updated = reloaded_series
            .iter()
            .find(|s| s.series.title == "Solo Leveling")
            .unwrap();
        assert_eq!(solo_updated.stats.completed_chapters, 1);

        // Test updating fetch url on series & chapter
        db.update_series_fetch_url(solo.series.id, "https://solo.example.com")
            .unwrap();
        db.update_chapter_fetch_url(first_chap_id, "https://solo.example.com/c100")
            .unwrap();

        let reloaded_series2 = db.get_all_series().unwrap();
        let solo_updated2 = reloaded_series2
            .iter()
            .find(|s| s.series.title == "Solo Leveling")
            .unwrap();
        assert_eq!(
            solo_updated2.series.fetch_url,
            Some("https://solo.example.com".to_string())
        );

        // Test updating reading mode
        assert_eq!(solo.series.reading_mode(), "webtoon");
        db.update_series_reading_mode(solo.series.id, "manga")
            .unwrap();

        let reloaded_series3 = db.get_all_series().unwrap();
        let solo_updated3 = reloaded_series3
            .iter()
            .find(|s| s.series.title == "Solo Leveling")
            .unwrap();
        assert_eq!(solo_updated3.series.reading_mode(), "manga");
    }

    #[test]
    fn test_file_progress_lookup_and_update() {
        let db = Database::in_memory().unwrap();
        let test_file = Path::new("/storage/manga/test_ch01.cbz");

        // First lookup on unregistered file -> returns None
        assert!(db.get_progress_for_file(test_file).unwrap().is_none());

        // Register chapter
        let chapter_id = db.get_or_create_chapter_for_file(test_file).unwrap();

        // Check progress before reading -> page 0, not completed
        let (id, prog) = db.get_progress_for_file(test_file).unwrap().unwrap();
        assert_eq!(id, chapter_id);
        assert_eq!(prog.last_page_read, 0);
        assert!(!prog.is_completed);

        // Update progress (e.g. read up to page 35)
        db.upsert_progress(chapter_id, 35, false).unwrap();

        // Query again -> returns page 35
        let (_, prog2) = db.get_progress_for_file(test_file).unwrap().unwrap();
        assert_eq!(prog2.last_page_read, 35);
        assert!(!prog2.is_completed);
    }

    #[test]
    fn test_progress_bounds_are_enforced() {
        let db = Database::in_memory().unwrap();
        let series_id = db.insert_or_get_series("Bounds Test").unwrap();
        db.record_chapter_download(series_id, 1.0, "/manga/c01.cbz", Some(15), None)
            .unwrap();
        let chapter_id = db
            .get_chapters_for_series(series_id)
            .unwrap()
            .remove(0)
            .chapter
            .id;

        // Absurd payload (global page across chapters) -> clamped to page count
        db.upsert_progress(chapter_id, 1446, false).unwrap();
        let (_, prog) = db
            .get_progress_for_file(Path::new("/manga/c01.cbz"))
            .unwrap()
            .unwrap();
        assert_eq!(prog.last_page_read, 15);

        // Negative -> clamped to 0
        db.upsert_progress(chapter_id, -5, false).unwrap();
        let (_, prog) = db
            .get_progress_for_file(Path::new("/manga/c01.cbz"))
            .unwrap()
            .unwrap();
        assert_eq!(prog.last_page_read, 0);

        // In-range value passes through untouched
        db.upsert_progress(chapter_id, 7, false).unwrap();
        let (_, prog) = db
            .get_progress_for_file(Path::new("/manga/c01.cbz"))
            .unwrap()
            .unwrap();
        assert_eq!(prog.last_page_read, 7);

        // Mark unread: progress row deleted -> reads as page 0, uncompleted
        db.delete_progress(chapter_id).unwrap();
        let (_, prog) = db
            .get_progress_for_file(Path::new("/manga/c01.cbz"))
            .unwrap()
            .unwrap();
        assert_eq!(prog.last_page_read, 0);
        assert!(!prog.is_completed);
    }

    #[test]
    fn test_init_schema_repairs_out_of_bounds_progress() {
        let db = Database::in_memory().unwrap();
        let series_id = db.insert_or_get_series("Repair Test").unwrap();
        db.record_chapter_download(series_id, 1.0, "/manga/r01.cbz", Some(15), None)
            .unwrap();
        let chapter_id = db
            .get_chapters_for_series(series_id)
            .unwrap()
            .remove(0)
            .chapter
            .id;

        // Simulate the corrupted row the old global-page payload produced.
        db.upsert_progress(chapter_id, 0, false).unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE progress SET last_page_read = 1446 WHERE chapter_id = ?1",
            params![chapter_id],
        )
        .unwrap();
        drop(conn);

        // Re-running schema init (as on next launch) clamps it.
        db.init_schema().unwrap();
        let (_, prog) = db
            .get_progress_for_file(Path::new("/manga/r01.cbz"))
            .unwrap()
            .unwrap();
        assert_eq!(prog.last_page_read, 15);
    }

    #[test]
    fn test_apply_chapter_progress() {
        let db = Database::in_memory().unwrap();
        db.seed_sample_data_if_empty().unwrap();

        // Known seeded chapter (page_count 48): large page is clamped.
        db.apply_chapter_progress(Path::new("/tmp/manga/solo_leveling_c100.cbz"), 500, false)
            .unwrap();
        let (_, prog) = db
            .get_progress_for_file(Path::new("/tmp/manga/solo_leveling_c100.cbz"))
            .unwrap()
            .unwrap();
        assert_eq!(prog.last_page_read, 48);
        assert!(!prog.is_completed);

        // Unknown path: chapter (and series) are created and progress set.
        let id = db
            .apply_chapter_progress(Path::new("/tmp/manga/never_seen.cbz"), 5, true)
            .unwrap();
        assert!(id > 0);
        let (_, prog2) = db
            .get_progress_for_file(Path::new("/tmp/manga/never_seen.cbz"))
            .unwrap()
            .unwrap();
        assert_eq!(prog2.last_page_read, 5);
        assert!(prog2.is_completed);
        let _ = id;
    }

    #[test]
    fn test_delete_series_cascades() {
        let db = Database::in_memory().unwrap();
        db.seed_sample_data_if_empty().unwrap();

        let series = db.get_all_series().unwrap();
        let solo = series
            .iter()
            .find(|s| s.series.title == "Solo Leveling")
            .unwrap();

        db.delete_series(solo.series.id).unwrap();

        let after = db.get_all_series().unwrap();
        assert_eq!(after.len(), 1);
        assert!(db
            .get_chapters_for_series(solo.series.id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_reset_removes_db_and_sidecar_files() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("dewey_reset_test_{}.db", std::process::id()));
        let path_str = path.to_string_lossy().to_string();
        std::fs::write(&path, b"fake").unwrap();
        std::fs::write(format!("{}-wal", path_str), b"wal").unwrap();
        std::fs::write(format!("{}-shm", path_str), b"shm").unwrap();

        Database::reset(&path).unwrap();

        assert!(!path.exists());
        assert!(!Path::new(&format!("{}-wal", path_str)).exists());
        assert!(!Path::new(&format!("{}-shm", path_str)).exists());
    }

    #[test]
    fn test_batch_record_chapters_and_diff() {
        let db = Database::in_memory().unwrap();
        let series_id = db.insert_or_get_series("Diff Test Series").unwrap();

        let entries = vec![
            ChapterScanEntry {
                chapter_number: 1.0,
                file_path: "/manga/ch01.cbz".to_string(),
                page_count: Some(45),
            },
            ChapterScanEntry {
                chapter_number: 2.0,
                file_path: "/manga/ch02.cbz".to_string(),
                page_count: Some(50),
            },
        ];

        let recorded = db.batch_record_chapters(series_id, &entries).unwrap();
        assert_eq!(recorded, 2);

        let map = db.get_existing_chapters_by_path(series_id).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("/manga/ch01.cbz").unwrap().page_count, Some(45));
    }

    #[test]
    fn test_database_open_with_profile() {
        let dir = std::env::temp_dir();
        let path_fast = dir.join(format!("dewey_test_fast_{}.db", std::process::id()));
        let path_usb = dir.join(format!("dewey_test_usb_{}.db", std::process::id()));

        let _db_fast =
            Database::open_with_profile(&path_fast, crate::config::StorageProfile::Fast).unwrap();
        let _db_usb =
            Database::open_with_profile(&path_usb, crate::config::StorageProfile::Usb).unwrap();

        assert!(path_fast.exists());
        assert!(path_usb.exists());

        let _ = Database::reset(&path_fast);
        let _ = Database::reset(&path_usb);
    }

    #[test]
    fn test_update_series_hidden_and_rename_directory() {
        let db = Database::in_memory().unwrap();
        let series_id = db
            .insert_or_get_series_with_cover_and_hidden(
                "Solo Leveling",
                Some("/manga/Solo/cover.jpg"),
                false,
            )
            .unwrap();

        db.record_chapter_download(series_id, 1.0, "/manga/Solo/c001.cbz", Some(20), None)
            .unwrap();

        let series = db.get_all_series().unwrap();
        assert!(!series[0].series.is_hidden);

        // Test update_series_hidden
        db.update_series_hidden(series_id, true).unwrap();
        let series = db.get_all_series().unwrap();
        assert!(series[0].series.is_hidden);

        // Test rename_series_directory
        db.rename_series_directory(
            series_id,
            Path::new("/manga/Solo"),
            Path::new("/manga/.Solo"),
            true,
        )
        .unwrap();

        let series = db.get_all_series().unwrap();
        assert_eq!(
            series[0].series.cover_path.as_deref(),
            Some("/manga/.Solo/cover.jpg")
        );
        assert!(series[0].series.is_hidden);

        let chapters = db.get_chapters_for_series(series_id).unwrap();
        assert_eq!(
            chapters[0].chapter.file_path.as_deref(),
            Some("/manga/.Solo/c001.cbz")
        );
    }
}
