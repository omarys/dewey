use crate::db::Database;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugHistoryEntry {
    pub series: String,
    pub chapter: f64,
    pub path: String,
    pub last_page: i64,
    pub opened_at: String,
}

pub struct DebugHistory;

impl DebugHistory {
    pub const MAX_ENTRIES: usize = 3;

    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(format!("{}/.local/state/dewey/debug_history.json", home))
    }

    pub fn load(path: &Path) -> Vec<DebugHistoryEntry> {
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(entries) = serde_json::from_str::<Vec<DebugHistoryEntry>>(&data) {
                return entries;
            }
        }
        Vec::new()
    }

    pub fn record(path: &Path, series: &str, chapter: f64, file_path: &Path, last_page: i64) {
        let mut entries = Self::load(path);
        let path_str = file_path.to_string_lossy().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let new_entry = DebugHistoryEntry {
            series: series.to_string(),
            chapter,
            path: path_str.clone(),
            last_page,
            opened_at: now,
        };

        // Remove any prior entry for the same file so the newest read is placed at the top
        entries.retain(|e| e.path != path_str);
        entries.insert(0, new_entry);
        entries.truncate(Self::MAX_ENTRIES);

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if let Ok(json) = serde_json::to_string_pretty(&entries) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load_or_fallback(path: &Path, db: &Database) -> Vec<DebugHistoryEntry> {
        let existing = Self::load(path);
        if !existing.is_empty() {
            return existing;
        }

        // Fall back to database query if history file has not been populated yet
        if let Ok(recent) = db.get_recent_read_history(Self::MAX_ENTRIES) {
            let entries: Vec<DebugHistoryEntry> = recent
                .into_iter()
                .map(|r| DebugHistoryEntry {
                    series: r.series_title,
                    chapter: r.chapter_number,
                    path: r.file_path,
                    last_page: r.last_page_read,
                    opened_at: r
                        .last_read_at
                        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                })
                .collect();

            if !entries.is_empty() {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(json) = serde_json::to_string_pretty(&entries) {
                    let _ = std::fs::write(path, json);
                }
            }
            return entries;
        }

        Vec::new()
    }
}
