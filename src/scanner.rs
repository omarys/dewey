#![allow(dead_code)]

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    LazyLock,
};
use tracing::info;
use tracing::warn;

use crate::db::{ChapterScanEntry, Database};

static RE_EXPLICIT_CHAPTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:chapter|chap|episode|ep|\bch|\bc|\b#)[_\s\.]*([0-9]+(?:\.[0-9]+)?)"#)
        .expect("invalid regex")
});
static RE_BRACKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\[([0-9]+(?:\.[0-9]+)?)\]"#).expect("invalid regex"));
static RE_SEP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:[-_]\s*)([0-9]+(?:\.[0-9]+)?)(?:$|\s|\.)"#).expect("invalid regex")
});
static RE_PURE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*([0-9]+(?:\.[0-9]+)?)\s*$"#).expect("invalid regex"));
static RE_ANY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\b([0-9]+(?:\.[0-9]+)?)\b"#).expect("invalid regex"));

#[derive(Debug, Default, Clone)]
pub struct ScanSummary {
    pub series_found: usize,
    pub chapters_found: usize,
    pub new_chapters_added: usize,
}

pub struct LibraryScanner;

impl LibraryScanner {
    /// Scans the designated library directory and synchronizes it with SQLite.
    /// Series are processed in parallel across a bounded worker pool so ZIP
    /// page-count reads (the slow part) overlap; SQLite writes stay serialized
    /// behind the shared connection mutex.
    pub fn scan_directory(db: &Database, library_dir: &Path) -> Result<ScanSummary> {
        Self::scan_directory_with_profile(
            db,
            library_dir,
            crate::config::StorageProfile::Fast,
            None,
        )
    }

    /// Scans the designated library directory and synchronizes it with SQLite.
    /// Worker pool concurrency is scaled based on the storage profile (fast vs usb).
    pub fn scan_directory_with_profile(
        db: &Database,
        library_dir: &Path,
        profile: crate::config::StorageProfile,
        max_concurrency: Option<usize>,
    ) -> Result<ScanSummary> {
        if !library_dir.exists() {
            fs::create_dir_all(library_dir)?;
            info!(path = ?library_dir, "Created empty library directory");
            return Ok(ScanSummary::default());
        }

        info!(
            path = ?library_dir,
            profile = profile.as_str(),
            "Scanning library directory for manga entries"
        );

        // Collect series directories before fanning out work.
        let series_dirs: Vec<PathBuf> = fs::read_dir(library_dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !Self::is_special_dir(p))
            .collect();

        if series_dirs.is_empty() {
            return Ok(ScanSummary::default());
        }

        let default_workers = match profile {
            crate::config::StorageProfile::Fast => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            crate::config::StorageProfile::Usb => 2,
        };

        let workers = max_concurrency
            .unwrap_or(default_workers)
            .max(1)
            .min(series_dirs.len());
        let next = AtomicUsize::new(0);

        // Each worker claims the next unprocessed series by index and reports a
        // local summary; the parent merges them all.
        let summary = std::thread::scope(|s| {
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    s.spawn(|| {
                        let mut local = ScanSummary::default();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            if i >= series_dirs.len() {
                                break;
                            }
                            match Self::scan_one_series(db, &series_dirs[i]) {
                                Ok(one) => {
                                    local.series_found += one.series_found;
                                    local.chapters_found += one.chapters_found;
                                    local.new_chapters_added += one.new_chapters_added;
                                }
                                Err(err) => warn!(
                                    series = ?series_dirs[i],
                                    error = %err,
                                    "Failed to scan series; continuing"
                                ),
                            }
                        }
                        local
                    })
                })
                .collect();

            let mut total = ScanSummary::default();
            for h in handles {
                if let Ok(part) = h.join() {
                    total.series_found += part.series_found;
                    total.chapters_found += part.chapters_found;
                    total.new_chapters_added += part.new_chapters_added;
                }
            }
            total
        });

        info!(
            series = summary.series_found,
            chapters = summary.chapters_found,
            new = summary.new_chapters_added,
            "Library directory scan completed"
        );

        Ok(summary)
    }

    /// Scans a single series directory and returns its contribution to the summary.
    fn scan_one_series(db: &Database, path: &Path) -> Result<ScanSummary> {
        let mut summary = ScanSummary::default();
        summary.series_found += 1;

        // Read series directory entries once in a single pass
        let entries: Vec<PathBuf> = fs::read_dir(path)?.flatten().map(|e| e.path()).collect();

        // 1. Read series.json if present for richer metadata
        let series_meta = Self::read_series_json_from_entries(&entries);

        let folder_name = path
            .file_name()
            .map(|n| n.to_string_lossy().replace('_', " "))
            .unwrap_or_else(|| "Unknown Series".to_string());

        let series_title = series_meta
            .as_ref()
            .and_then(|m| m.name.clone())
            .unwrap_or(folder_name);

        let cover_path = Self::find_cover_image_from_entries(&entries);
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

        // 2. Scan chapter files inside the series directory using the already read entries
        let (ch_count, new_count) = Self::scan_series_entries(db, series_id, &entries)?;
        summary.chapters_found += ch_count;
        summary.new_chapters_added += new_count;

        Ok(summary)
    }

    pub fn scan_series_directory(
        db: &Database,
        series_id: i64,
        series_dir: &Path,
    ) -> Result<(usize, usize)> {
        let entries: Vec<PathBuf> = fs::read_dir(series_dir)?
            .flatten()
            .map(|e| e.path())
            .collect();
        Self::scan_series_entries(db, series_id, &entries)
    }

    fn scan_series_entries(
        db: &Database,
        series_id: i64,
        entries: &[PathBuf],
    ) -> Result<(usize, usize)> {
        let existing_map = db.get_existing_chapters_by_path(series_id)?;
        let mut chapter_paths: Vec<PathBuf> = entries
            .iter()
            .filter(|p| Self::is_chapter_file(p) || (p.is_dir() && !Self::is_special_dir(p)))
            .cloned()
            .collect();

        chapter_paths.sort_by(|a, b| natord::compare(&a.to_string_lossy(), &b.to_string_lossy()));

        let has_explicit = chapter_paths
            .iter()
            .any(|p| Self::parse_explicit_chapter_number(p).is_some());

        let mut chapters_found = 0;
        let mut to_insert = Vec::new();
        let mut last_known_chap = 0.0;
        let mut bonus_offset = 0.0;

        for path in chapter_paths {
            let explicit_chap = Self::parse_explicit_chapter_number(&path);
            let chap_num = if let Some(num) = explicit_chap {
                last_known_chap = num;
                bonus_offset = 0.0;
                num
            } else if !has_explicit {
                if let Some(fallback_num) = Self::parse_chapter_number(&path) {
                    last_known_chap = fallback_num;
                    bonus_offset = 0.0;
                    fallback_num
                } else {
                    bonus_offset += 0.1;
                    ((last_known_chap + bonus_offset) * 100.0).round() / 100.0
                }
            } else {
                bonus_offset += 0.1;
                ((last_known_chap + bonus_offset) * 100.0).round() / 100.0
            };

            chapters_found += 1;
            let path_str = path.to_string_lossy().to_string();

            // Diff check: if file is already indexed with page_count, skip opening zip
            if let Some(info) = existing_map.get(&path_str) {
                if (info.chapter_number - chap_num).abs() > 0.001 {
                    let _ = db.update_chapter_number(info.id, chap_num);
                }
                if info.page_count.is_some() {
                    continue;
                }
            }

            // New or uncounted chapter -> open archive once
            let page_count = Self::detect_page_count(&path);
            to_insert.push(ChapterScanEntry {
                chapter_number: chap_num,
                file_path: path_str,
                page_count,
            });
        }

        let new_chapters_added = db.batch_record_chapters(series_id, &to_insert)?;
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

    /// True if `dir` (recursively) contains any chapter archive
    /// (.cbz/.zip/.epub/.pdf/.cbr). Used to refuse initializing an empty
    /// library with no comic content.
    pub fn has_chapter_files(dir: &Path) -> bool {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(entries) = fs::read_dir(&d) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if Self::is_chapter_file(&p) {
                        return true;
                    }
                }
            }
        }
        false
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
        let entries: Vec<PathBuf> = fs::read_dir(series_dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        Self::find_cover_image_from_entries(&entries)
    }

    pub fn find_cover_image_from_entries(entries: &[PathBuf]) -> Option<PathBuf> {
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
            if let Some(p) = entries.iter().find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.eq_ignore_ascii_case(name))
                    .unwrap_or(false)
            }) {
                return Some(p.clone());
            }
        }

        None
    }

    fn read_series_json(series_dir: &Path) -> Option<SeriesMetadataParsed> {
        let entries: Vec<PathBuf> = fs::read_dir(series_dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        Self::read_series_json_from_entries(&entries)
    }

    fn read_series_json_from_entries(entries: &[PathBuf]) -> Option<SeriesMetadataParsed> {
        let json_path = entries.iter().find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.eq_ignore_ascii_case("series.json"))
                .unwrap_or(false)
        })?;

        let content = fs::read_to_string(json_path).ok()?;
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

    /// Attempts to extract an explicit chapter number (e.g. Chapter 5, Episode 2, Ch. 12, #40)
    pub fn parse_explicit_chapter_number(path: &Path) -> Option<f64> {
        let file_stem = path.file_stem()?.to_string_lossy();
        if let Some(caps) = RE_EXPLICIT_CHAPTER.captures(&file_stem) {
            if let Some(m) = caps.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    return Some(num);
                }
            }
        }
        None
    }

    /// Robust chapter number parsing using static precompiled regexes
    pub fn parse_chapter_number(path: &Path) -> Option<f64> {
        let file_stem = path.file_stem()?.to_string_lossy();

        // Pattern 1: Look explicitly for "chapter_105" or "chapter 105" or "ch105" or "episode 1"
        if let Some(num) = Self::parse_explicit_chapter_number(path) {
            return Some(num);
        }

        // Pattern 2: Bracket prefix index like [0045]
        if let Some(caps) = RE_BRACKET.captures(&file_stem) {
            if let Some(m) = caps.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    return Some(num);
                }
            }
        }

        // Pattern 3: Separator then number: " - 105", "_105"
        if let Some(caps) = RE_SEP.captures(&file_stem) {
            if let Some(m) = caps.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    return Some(num);
                }
            }
        }

        // Pattern 4: Pure numeric name: "0105", "105.5"
        if let Some(caps) = RE_PURE.captures(&file_stem) {
            if let Some(m) = caps.get(1) {
                if let Ok(num) = m.as_str().parse::<f64>() {
                    return Some(num);
                }
            }
        }

        // Pattern 5: Last standalone number
        let mut last_num = None;
        for cap in RE_ANY.captures_iter(&file_stem) {
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
    use crate::db::Database;
    use std::io::Write;

    #[test]
    fn test_parse_episode_names() {
        // Real-world conventions: release-index bracket + episode number.
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("[0000]_Episode_209_Aug_15.cbz")),
            Some(209.0)
        );
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("[0001]_Episode_208_Aug_7.cbz")),
            Some(208.0)
        );
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("[0208]_Episode_1_Sep_7_2024.cbz")),
            Some(1.0)
        );
        // Chapter-style names are unaffected by the episode keywords.
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("[0000]_Chapter_40.6_Apr_4.cbz")),
            Some(40.6)
        );
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("[0001]_Chapter_1.cbz")),
            Some(1.0)
        );
        // Episode/volume shorthand.
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("Ep. 5.cbz")),
            Some(5.0)
        );
        assert_eq!(
            LibraryScanner::parse_chapter_number(Path::new("Vol 3.cbz")),
            Some(3.0)
        );
    }

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
        assert_eq!(
            LibraryScanner::parse_explicit_chapter_number(Path::new(
                "[0006]_Vol.1_Bonus_Material.cbz"
            )),
            None
        );
        assert_eq!(
            LibraryScanner::parse_explicit_chapter_number(Path::new("[0001]_Chapter_1.cbz")),
            Some(1.0)
        );
    }

    #[test]
    fn test_has_chapter_files_detects_archives() {
        let root = std::env::temp_dir().join(format!("dewey_empty_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Series")).unwrap();

        // Empty library (even with subfolders) -> false
        assert!(!LibraryScanner::has_chapter_files(&root));

        // One cbz in root -> true
        std::fs::write(root.join("c001.cbz"), b"x").unwrap();
        assert!(LibraryScanner::has_chapter_files(&root));

        // Only archives in a nested subfolder -> true
        std::fs::remove_file(root.join("c001.cbz")).unwrap();
        std::fs::create_dir_all(root.join("Series").join("deep")).unwrap();
        std::fs::write(root.join("Series").join("deep").join("c002.zip"), b"x").unwrap();
        assert!(LibraryScanner::has_chapter_files(&root));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Exercises the parallel scan path: builds a temp library with several
    /// series of cbz files and verifies the DB is hydrated correctly.
    #[test]
    fn test_parallel_scan_hydrates_library() {
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let root = std::env::temp_dir().join(format!("dewey_scan_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        // 4 series x 3 chapters, each cbz with 5 image entries.
        for s in 0..4 {
            let dir = root.join(format!("Series_{}", s));
            std::fs::create_dir_all(&dir).unwrap();
            for c in 0..3 {
                let zp = dir.join(format!("c{:03}.cbz", c + 1));
                let file = std::fs::File::create(&zp).unwrap();
                let mut z = ZipWriter::new(file);
                let opts = SimpleFileOptions::default();
                for p in 0..5 {
                    z.start_file(format!("{}.jpg", p), opts).unwrap();
                    z.write_all(b"x").unwrap();
                }
                z.finish().unwrap();
            }
        }

        let db = Database::in_memory().unwrap();
        let summary = LibraryScanner::scan_directory(&db, &root).unwrap();

        assert_eq!(summary.series_found, 4);
        assert_eq!(summary.chapters_found, 12);
        assert_eq!(summary.new_chapters_added, 12);

        let series = db.get_all_series().unwrap();
        assert_eq!(series.len(), 4);
        assert_eq!(series[0].stats.total_chapters, 3);
        assert_eq!(series[0].stats.downloaded_chapters, 3);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_usb_scan_profile_with_concurrency_limit() {
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let root = std::env::temp_dir().join(format!("dewey_usb_scan_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let dir = root.join("UsbSeries");
        std::fs::create_dir_all(&dir).unwrap();
        let zp = dir.join("c001.cbz");
        let file = std::fs::File::create(&zp).unwrap();
        let mut z = ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        z.start_file("1.jpg", opts).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();

        let db = Database::in_memory().unwrap();
        let summary = LibraryScanner::scan_directory_with_profile(
            &db,
            &root,
            crate::config::StorageProfile::Usb,
            Some(1),
        )
        .unwrap();

        assert_eq!(summary.series_found, 1);
        assert_eq!(summary.chapters_found, 1);
        assert_eq!(summary.new_chapters_added, 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_find_cover_image_from_entries() {
        let entries = vec![
            PathBuf::from("/manga/Solo/ch01.cbz"),
            PathBuf::from("/manga/Solo/folder.jpg"),
            PathBuf::from("/manga/Solo/series.json"),
        ];
        let cover = LibraryScanner::find_cover_image_from_entries(&entries);
        assert_eq!(cover, Some(PathBuf::from("/manga/Solo/folder.jpg")));
    }
}
