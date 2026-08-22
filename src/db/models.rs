use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    pub id: i64,
    pub title: String,
    pub sort_title: Option<String>,
    pub cover_path: Option<String>,
    pub status: Option<String>,
    pub fetch_url: Option<String>,
    pub metadata_json: Option<String>,
    pub reading_mode: Option<String>,
}

impl Series {
    pub fn reading_mode(&self) -> &str {
        self.reading_mode.as_deref().unwrap_or("webtoon")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: i64,
    pub series_id: i64,
    pub chapter_number: f64,
    pub file_path: Option<String>,
    pub page_count: Option<i64>,
    pub fetch_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub chapter_id: i64,
    pub last_page_read: i64,
    pub is_completed: bool,
    pub last_read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterWithProgress {
    pub chapter: Chapter,
    pub progress: Option<Progress>,
}

impl ChapterWithProgress {
    pub fn is_downloaded(&self) -> bool {
        if let Some(path_str) = &self.chapter.file_path {
            Path::new(path_str).exists()
        } else {
            false
        }
    }

    pub fn last_page(&self) -> i64 {
        self.progress
            .as_ref()
            .map(|p| p.last_page_read)
            .unwrap_or(0)
    }

    pub fn is_completed(&self) -> bool {
        self.progress
            .as_ref()
            .map(|p| p.is_completed)
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesStats {
    pub total_chapters: usize,
    pub downloaded_chapters: usize,
    pub completed_chapters: usize,
    pub latest_read_chapter: Option<f64>,
    pub last_read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesWithStats {
    pub series: Series,
    pub stats: SeriesStats,
}
