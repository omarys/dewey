#![allow(dead_code)]

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tracing::info;

use crate::db::Database;

#[derive(Debug, Default, Clone)]
pub struct ScanSummary {
    pub series_found: usize,
    pub chapters_found: usize,
    pub new_chapters_added: usize,
}

pub struct LibraryScanner;

impl LibraryScanner {
    /// Scans the designated library directory and synchronizes it with SQLite
    pub fn scan_directory(db: &Database, library_dir: &Path) -> Result<ScanSummary> {
        if !library_dir.exists() {
            fs::create_dir_all(library_dir)?;
            info!(path = ?library_dir, "Created empty library directory");
            return Ok(ScanSummary::default());
        }

        info!(path = ?library_dir, "Scanning library directory for manga entries");

        let mut summary = ScanSummary::default();
        let entries = fs::read_dir(library_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !Self::is_special_dir(&path) {
                summary.series_found += 1;

                // 1. Read series.json if present for richer metadata
                let series_meta = Self::read_series_json(&path);

                let folder_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().replace('_', " "))
                    .unwrap_or_else(|| "Unknown Series".to_string());

                let series_title = series_meta
                    .as_ref()
                    .and_then(|m| m.name.clone())
                    .unwrap_or(folder_name);

                let cover_path = Self::find_cover_image(&path);
                let series_id = db.insert_or_get_series_with_cover(
                    &series_title,
                    cover_path.as_deref().and_then(|p| p.to_str()),
                )?;

                // Update series metadata / status / fetch_url if found in series.json
                if let Some(meta) = &series_meta {
                    if let Some(status) = &meta.status {
                        let _ = db.update_series_status(series_id, status);
                    }
                    if let Some(json_raw) = &meta.raw_json {
                        let _ = db.update_series_metadata(series_id, json_raw);
                    }
                    if let Some(url) = &meta.fetch_url {
                        let _ = db.update_series_fetch_url(series_id, url);
                    }
                }

                // 2. Scan chapter files inside the series directory
                let (ch_count, new_count) = Self::scan_series_directory(db, series_id, &path)?;
                summary.chapters_found += ch_count;
                summary.new_chapters_added += new_count;
            }
        }

        info!(
            series = summary.series_found,
            chapters = summary.chapters_found,
            new = summary.new_chapters_added,
            "Library directory scan completed"
        );

        Ok(summary)
    }

    fn scan_series_directory(
        db: &Database,
        series_id: i64,
        series_dir: &Path,
    ) -> Result<(usize, usize)> {
        let entries = fs::read_dir(series_dir)?;
        let mut chapters_found = 0;
        let mut new_chapters_added = 0;

        for entry in entries.flatten() {
            let path = entry.path();
            if Self::is_chapter_file(&path) || (path.is_dir() && !Self::is_special_dir(&path)) {
                if let Some(chap_num) = Self::parse_chapter_number(&path) {
                    chapters_found += 1;
                    let page_count = Self::detect_page_count(&path);
                    let path_str = path.to_string_lossy().to_string();

                    db.record_chapter_download(series_id, chap_num, &path_str, page_count, None)?;
                    new_chapters_added += 1;
                }
            }
        }

        Ok((chapters_found, new_chapters_added))
    }

    pub fn is_chapter_file(path: &Path) -> bool {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext_lower = ext.to_lowercase();
            matches!(ext_lower.as_str(), "cbz" | "zip" | "epub" | "pdf" | "cbr")
        } else {
            false
        }
    }

    fn is_special_dir(path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            name.starts_with('.')
                || name.eq_ignore_ascii_case("covers")
                || name.eq_ignore_ascii_case("metadata")
        } else {
            true
        }
    }

    pub fn find_cover_image(series_dir: &Path) -> Option<PathBuf> {
        let candidates = [
            "cover.jpg",
            "cover.png",
            "cover.webp",
            "cover.jpeg",
            "folder.jpg",
            "folder.png",
            "poster.jpg",
            "poster.png",
        ];

        for name in &candidates {
            let p = series_dir.join(name);
            if p.exists() {
                return Some(p);
            }
        }

        // Search for any .jpg / .png named cover*
        if let Ok(entries) = fs::read_dir(series_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let file_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    if file_name.starts_with("cover") || file_name.starts_with("folder") {
                        return Some(path);
                    }
                }
            }
        }

        None
    }

    fn read_series_json(series_dir: &Path) -> Option<SeriesMetadataParsed> {
        let json_path = series_dir.join("series.json");
        if !json_path.exists() {
            return None;
        }

        let content = fs::read_to_string(&json_path).ok()?;
        let parsed: Value = serde_json::from_str(&content).ok()?;

        let meta_obj = parsed.get("metadata").unwrap_or(&parsed);

        let name = meta_obj
            .get("name")
            .or_else(|| meta_obj.get("title"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let status = meta_obj
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| match s {
                "Continuing" => "Ongoing".to_string(),
                "Ended" => "Completed".to_string(),
                other => other.to_string(),
            });

        let fetch_url = meta_obj
            .get("fetch_url")
            .or_else(|| meta_obj.get("url"))
            .or_else(|| meta_obj.get("source_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Some(SeriesMetadataParsed {
            name,
            status,
            fetch_url,
            raw_json: Some(content),
        })
    }

    /// Robust chapter number parsing supporting:
    /// - "[0001]_Chapter_1.cbz" -> 1.0
    /// - "[0105]_Chapter_105.cbz" -> 105.0
    /// - "Solo Leveling - c105.cbz" -> 105.0
    /// - "Ch. 12.5.zip" -> 12.5
    /// - "Chapter_045.cbz" -> 45.0
    /// - "v02_c015.cbz" -> 15.0
    /// - "105.cbz" -> 105.0
    pub fn parse_chapter_number(path: &Path) -> Option<f64> {
        let file_stem = path.file_stem()?.to_string_lossy();

        // Pattern 1: Look explicitly for "chapter_105" or "chapter 105" or "c105"
        let re_chapter =
            Regex::new(r#"(?i)(?:chapter|chap|ch|\bc|#)[_\s\.]*([0-9]+(?:\.[0-9]+)?)"#).ok()?;
        if let Some(caps) = re_chapter.captures(&file_stem) {
            if let Some(m) = caps.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    return Some(num);
                }
            }
        }

        // Pattern 2: Bracket prefix index like [0045]
        let re_bracket = Regex::new(r#"\[([0-9]+(?:\.[0-9]+)?)\]"#).ok()?;
        if let Some(caps) = re_bracket.captures(&file_stem) {
            if let Some(m) = caps.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    return Some(num);
                }
            }
        }

        // Pattern 3: Separator then number: " - 105", "_105"
        let re_sep = Regex::new(r#"(?:[-_]\s*)([0-9]+(?:\.[0-9]+)?)(?:$|\s|\.)"#).ok()?;
        if let Some(caps) = re_sep.captures(&file_stem) {
            if let Some(m) = caps.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    return Some(num);
                }
            }
        }

        // Pattern 4: Pure numeric name: "0105", "105.5"
        let re_pure = Regex::new(r#"^\s*([0-9]+(?:\.[0-9]+)?)\s*$"#).ok()?;
        if let Some(caps) = re_pure.captures(&file_stem) {
            if let Some(m) = caps.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    return Some(num);
                }
            }
        }

        // Pattern 5: Last standalone number
        let re_any = Regex::new(r#"\b([0-9]+(?:\.[0-9]+)?)\b"#).ok()?;
        let mut last_num = None;
        for cap in re_any.captures_iter(&file_stem) {
            if let Some(m) = cap.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    last_num = Some(num);
                }
            }
        }

        last_num
    }

    /// Detect page count from .cbz/.zip archives or folder
    pub fn detect_page_count(path: &Path) -> Option<i64> {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("cbz") || ext.eq_ignore_ascii_case("zip") {
                if let Ok(file) = File::open(path) {
                    if let Ok(archive) = zip::ZipArchive::new(file) {
                        let count = archive
                            .file_names()
                            .filter(|name| {
                                let lower = name.to_lowercase();
                                lower.ends_with(".jpg")
                                    || lower.ends_with(".jpeg")
                                    || lower.ends_with(".png")
                                    || lower.ends_with(".webp")
                                    || lower.ends_with(".avif")
                            })
                            .count();
                        if count > 0 {
                            return Some(count as i64);
                        }
                    }
                }
            }
        } else if path.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                let count = entries
                    .flatten()
                    .filter(|e| {
                        if let Some(ext) = e.path().extension().and_then(|x| x.to_str()) {
                            let l = ext.to_lowercase();
                            matches!(l.as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif")
                        } else {
                            false
                        }
                    })
                    .count();
                if count > 0 {
                    return Some(count as i64);
                }
            }
        }

        None
    }
}

struct SeriesMetadataParsed {
    name: Option<String>,
    status: Option<String>,
    fetch_url: Option<String>,
    raw_json: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bracketed_chapter_names() {
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("[0001]_Chapter_1.cbz")),
            Some(1.0)
        );
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("[0105]_Chapter_105.cbz")),
            Some(105.0)
        );
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("[0012]_Chapter_12.5.cbz")),
            Some(12.5)
        );
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("Solo Leveling - c105.cbz")),
            Some(105.0)
        );
    }
}
