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
    pub is_hidden: bool,
    pub category: Option<String>,
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

    /// Extracts the chapter title / subtitle from the archive filename.
    /// E.g. "Berserk - Chapter 6 - Master of the Sword (1).cbz" -> Some("Master of the Sword (1)")
    pub fn chapter_subtitle(&self) -> Option<String> {
        let fp = self.chapter.file_path.as_deref()?;
        let stem = Path::new(fp).file_stem()?.to_str()?;

        let parts: Vec<&str> = stem.split(" - ").collect();
        if parts.len() >= 3 {
            let sub = parts[2..].join(" - ").trim().to_string();
            if !sub.is_empty() {
                return Some(sub);
            }
        }

        if parts.len() == 2 {
            let p0_lower = parts[0].to_lowercase();
            if p0_lower.starts_with("chapter")
                || p0_lower.starts_with("episode")
                || p0_lower.starts_with("ch.")
                || p0_lower.starts_with("ep.")
                || p0_lower.starts_with("vol")
            {
                let sub = parts[1].trim().to_string();
                if !sub.is_empty() {
                    return Some(sub);
                }
            }
        }

        None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chapter_subtitle_extraction() {
        let ch = ChapterWithProgress {
            chapter: Chapter {
                id: 1,
                series_id: 1,
                chapter_number: 6.0,
                file_path: Some(
                    "/library/Berserk/Berserk - Chapter 6 - Master of the Sword (1).cbz"
                        .to_string(),
                ),
                page_count: Some(25),
                fetch_url: None,
            },
            progress: None,
        };
        assert_eq!(
            ch.chapter_subtitle(),
            Some("Master of the Sword (1)".to_string())
        );

        let ch2 = ChapterWithProgress {
            chapter: Chapter {
                id: 2,
                series_id: 1,
                chapter_number: 0.1,
                file_path: Some(
                    "/library/Berserk/Berserk - Chapter 0.1 - The Black Swordsman.cbz".to_string(),
                ),
                page_count: Some(52),
                fetch_url: None,
            },
            progress: None,
        };
        assert_eq!(
            ch2.chapter_subtitle(),
            Some("The Black Swordsman".to_string())
        );

        let ch_no_sub = ChapterWithProgress {
            chapter: Chapter {
                id: 3,
                series_id: 2,
                chapter_number: 192.0,
                file_path: Some("/library/Chainsaw_Man/Chainsaw Man - Chapter 192.cbz".to_string()),
                page_count: Some(19),
                fetch_url: None,
            },
            progress: None,
        };
        assert_eq!(ch_no_sub.chapter_subtitle(), None);

        let ch_multiple_sep = ChapterWithProgress {
            chapter: Chapter {
                id: 4,
                series_id: 3,
                chapter_number: 1.0,
                file_path: Some(
                    "/library/Manga/Series - Chapter 1 - Part 1 - The Beginning.cbz".to_string(),
                ),
                page_count: Some(30),
                fetch_url: None,
            },
            progress: None,
        };
        assert_eq!(
            ch_multiple_sep.chapter_subtitle(),
            Some("Part 1 - The Beginning".to_string())
        );
    }
}
