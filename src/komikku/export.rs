//! Writes a Komikku/Tachiyomi backup from Dewey's library.
//!
//! The mirror of [`crate::komikku::import`]: every series in the database is
//! emitted as a favorite (in Komikku a favorite *is* library membership), with
//! its Komikku source binding, categories, chapters, reading progress, bookmarks
//! and history restored from what the import recorded. See
//! `docs/adr/0004-komikku-backup-import.md`.
//!
//! Deliberate losses, because Dewey never stored the data:
//!
//! * Chapters with no `fetch_url` are skipped — a Komikku chapter without a URL
//!   cannot be opened, and inventing one would produce a backup that fails
//!   later rather than now. The count is reported.
//! * `viewer` is left at Komikku's default instead of being guessed from
//!   Dewey's reading mode, so a restore never overrides the user's own viewer
//!   setting with a fabricated one.
//! * Chapter `sourceOrder`, upload/fetch dates and scanlators are not restored.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::Value;

use crate::db::models::{ChapterWithProgress, Series};
use crate::db::Database;
use crate::komikku::proto::Writer;

/// Komikku category names for Dewey's three library types.
const CATEGORY_MANGA: &str = "Manga";
const CATEGORY_MANHWA: &str = "Manhwa";
const CATEGORY_COMICS: &str = "Comics";

/// Komikku's `Status.COMPLETED`, used when the recorded value is unusable.
const STATUS_COMPLETED: i32 = 2;

#[derive(Debug, Default, Clone)]
pub struct ExportReport {
    pub series: usize,
    /// Series that could not be written: no Komikku source binding, or no URL.
    pub series_skipped: usize,
    pub chapters: usize,
    /// Chapters dropped because they have no `fetch_url`.
    pub chapters_without_url: usize,
    pub history: usize,
    pub categories: usize,
    pub bytes: usize,
}

impl std::fmt::Display for ExportReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Komikku backup export:")?;
        writeln!(f, "  series: {}", self.series)?;
        if self.series_skipped > 0 {
            writeln!(
                f,
                "  skipped series: {} (no Komikku source binding)",
                self.series_skipped
            )?;
        }
        writeln!(f, "  chapters: {}", self.chapters)?;
        if self.chapters_without_url > 0 {
            writeln!(
                f,
                "  skipped chapters: {} (no fetch url)",
                self.chapters_without_url
            )?;
        }
        writeln!(f, "  history entries: {}", self.history)?;
        writeln!(f, "  categories: {}", self.categories)?;
        writeln!(f, "  size: {} bytes", self.bytes)
    }
}

/// Encodes the whole library and writes it to `path` unless `dry_run`.
pub fn run(db: &Database, path: &Path, dry_run: bool) -> Result<ExportReport> {
    let (bytes, report) = build(db)?;
    if !dry_run {
        std::fs::write(path, &bytes)
            .with_context(|| format!("writing backup file {}", path.display()))?;
    }
    Ok(report)
}

/// Encodes the library into a gzip-compressed `Backup` message.
pub fn build(db: &Database) -> Result<(Vec<u8>, ExportReport)> {
    let all = db.get_all_series()?;
    let mut report = ExportReport::default();

    // Source bindings we could borrow for a series that has none of its own,
    // keyed by normalized title: a scan-made series usually has a twin that the
    // Komikku import created.
    let mut by_title: BTreeMap<String, (i64, String)> = BTreeMap::new();
    for row in &all {
        if let Some(binding) = info(&row.series) {
            by_title
                .entry(normalize(&row.series.title))
                .or_insert((binding.source_id, binding.source_name));
        }
    }

    // Category ids are allocated here; Komikku only needs them to be consistent
    // within the file, so sorted names give a stable, diffable result.
    let mut category_names: Vec<String> = Vec::new();
    let mut prepared = Vec::new();

    for row in &all {
        let series = &row.series;
        let Some(binding) = binding_for(series, &by_title) else {
            report.series_skipped += 1;
            continue;
        };
        for name in categories_for(series, &binding) {
            if !category_names.contains(&name) {
                category_names.push(name);
            }
        }
        prepared.push((series.clone(), binding));
    }
    category_names.sort();
    let category_ids: BTreeMap<&str, i64> = category_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), index as i64 + 1))
        .collect();
    report.categories = category_ids.len();

    let mut sources: Vec<(String, i64)> = Vec::new();
    let mut backup = Writer::new();

    for (series, binding) in &prepared {
        if !sources
            .iter()
            .any(|(name, id)| name == &binding.source_name && *id == binding.source_id)
        {
            sources.push((binding.source_name.clone(), binding.source_id));
        }

        let chapters = db.get_chapters_for_series(series.id)?;
        let mut chapter_msgs = Vec::new();
        let mut history_msgs = Vec::new();

        for (index, row) in chapters.iter().enumerate() {
            let url = row
                .chapter
                .fetch_url
                .as_deref()
                .map(str::trim)
                .filter(|url| !url.is_empty());
            let Some(url) = url else {
                report.chapters_without_url += 1;
                continue;
            };

            chapter_msgs.push(chapter_message(url, &chapter_name(row), row, index as i64));
            if let Some(written) = history_message(row) {
                history_msgs.push(written);
                report.history += 1;
            }
            report.chapters += 1;
        }

        let mut manga = Writer::new();
        let mut category_ids_out: Vec<i64> = Vec::new();
        manga.int(1, binding.source_id);
        manga.string(2, &binding.url);
        manga.string(3, &series.title);
        if let Some(genre) = &binding.genre {
            for entry in genre {
                manga.string(7, entry);
            }
        }
        if let Some(status) = binding.status {
            manga.int(8, i64::from(status));
        }
        if let Some(thumbnail) = &binding.thumbnail_url {
            manga.string(9, thumbnail);
        }
        if let Some(author) = &binding.author {
            manga.string(5, author);
        }
        if let Some(artist) = &binding.artist {
            manga.string(4, artist);
        }
        if let Some(description) = &binding.description {
            manga.string(6, description);
        }
        for id in categories_for(series, binding)
            .iter()
            .filter_map(|name| category_ids.get(name.as_str()).copied())
        {
            category_ids_out.push(id);
        }
        if !category_ids_out.is_empty() {
            manga.packed_int(17, &category_ids_out);
        }
        for message in &chapter_msgs {
            manga.message(16, message);
        }
        for message in &history_msgs {
            manga.message(104, message);
        }
        // In Komikku a favorite is a library entry; every Dewey series is one.
        manga.boolean(100, true);

        // Each manga is its own field-1 entry on the backup, not a nested batch.
        backup.message(1, &manga);
        report.series += 1;
    }

    // Categories come after the mangas here; field order is not significant.
    for (order, name) in category_names.iter().enumerate() {
        let id = category_ids[name.as_str()];
        let mut category = Writer::new();
        category.string(1, name);
        category.int(2, order as i64);
        category.int(3, id);
        backup.message(2, &category);
    }
    for (name, id) in &sources {
        let mut source = Writer::new();
        source.string(1, name);
        source.int(2, *id);
        backup.message(101, &source);
    }

    let raw = backup.into_vec();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw).context("compressing backup")?;
    let bytes = encoder.finish().context("compressing backup")?;
    report.bytes = bytes.len();

    Ok((bytes, report))
}

/// Everything Komikku needs to place a series back into a source.
struct Binding {
    source_id: i64,
    source_name: String,
    url: String,
    thumbnail_url: Option<String>,
    status: Option<i32>,
    categories: Vec<String>,
    author: Option<String>,
    artist: Option<String>,
    description: Option<String>,
    genre: Option<Vec<String>>,
}

/// Reads the Komikku provenance the import stored in `metadata_json`.
fn info(series: &Series) -> Option<Binding> {
    let metadata: Value = series
        .metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())?;
    let komikku = metadata.get("komikku")?;

    let source_id = komikku.get("id").and_then(Value::as_i64)?;
    let source_name = komikku.get("source").and_then(Value::as_str)?.to_string();
    let url = komikku
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| series.fetch_url.clone())
        .filter(|url| !url.trim().is_empty())?;

    Some(Binding {
        source_id,
        source_name,
        url,
        thumbnail_url: string_of(metadata.get("komikku")?.get("thumbnail_url")),
        status: komikku
            .get("status")
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        categories: komikku
            .get("categories")
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        author: string_of(metadata.get("author")),
        artist: string_of(metadata.get("artist")),
        description: string_of(metadata.get("description")),
        genre: metadata
            .get("genre")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            }),
    })
}

/// Falls back to a series with the same title that does have a Komikku binding.
fn binding_for(series: &Series, by_title: &BTreeMap<String, (i64, String)>) -> Option<Binding> {
    if let Some(binding) = info(series) {
        return Some(binding);
    }

    let (source_id, source_name) = by_title.get(&normalize(&series.title))?.clone();
    let url = series
        .fetch_url
        .clone()
        .filter(|url| !url.trim().is_empty())?;

    Some(Binding {
        source_id,
        source_name,
        url,
        thumbnail_url: None,
        status: status_id(series.status.as_deref()),
        categories: Vec::new(),
        author: None,
        artist: None,
        description: None,
        genre: None,
    })
}

/// Komikku category names for a series.
///
/// The import recorded the categories the series actually carried, so those are
/// replayed verbatim: that is what makes a re-import land on the same side of
/// the hidden/visible line, since visibility is decided by whether a type tag is
/// present. Only a series the import never saw falls back to Dewey's type.
fn categories_for(series: &Series, binding: &Binding) -> Vec<String> {
    if let Some(names) = recorded_categories(binding) {
        return names;
    }

    match series.category.as_deref() {
        Some("Comicbook") => vec![CATEGORY_COMICS.to_string()],
        Some("Manhwa") => vec![CATEGORY_MANHWA.to_string()],
        Some("Manga") => vec![CATEGORY_MANGA.to_string()],
        _ => Vec::new(),
    }
}

/// The Komikku categories the import saw, without duplicates.
fn recorded_categories(binding: &Binding) -> Option<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    for name in &binding.categories {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    (!names.is_empty()).then_some(names)
}

fn chapter_name(row: &ChapterWithProgress) -> String {
    match row.chapter.title.as_deref().map(str::trim) {
        Some(title) if !title.is_empty() => title.to_string(),
        // Dewey's own fallback: the number is the only name it has.
        _ => format!("Chapter {}", trim_number(row.chapter.chapter_number)),
    }
}

fn chapter_message(url: &str, name: &str, row: &ChapterWithProgress, source_order: i64) -> Writer {
    let mut chapter = Writer::new();
    chapter.string(1, url);
    chapter.string(2, name);
    if row.chapter.is_bookmarked {
        chapter.boolean(5, true);
    }
    if let Some(progress) = &row.progress {
        chapter.boolean(4, progress.is_completed);
        if progress.last_page_read > 0 {
            chapter.int(6, progress.last_page_read);
        }
    }
    chapter.float(9, row.chapter.chapter_number as f32);
    chapter.int(10, source_order);
    chapter
}

fn history_message(row: &ChapterWithProgress) -> Option<Writer> {
    let progress = row.progress.as_ref()?;
    let last_read_at = progress.last_read_at?;
    let url = row.chapter.fetch_url.as_deref()?.trim();
    if url.is_empty() {
        return None;
    }

    let mut history = Writer::new();
    history.string(1, url);
    history.int(2, last_read_at.timestamp_millis());
    Some(history)
}

/// Maps Dewey's status strings back to Komikku's enum, for series whose
/// provenance only left a text status behind.
fn status_id(status: Option<&str>) -> Option<i32> {
    let status = status?.to_ascii_lowercase();
    Some(match status.as_str() {
        "ongoing" => 1,
        "completed" | "finished" | "ended" => STATUS_COMPLETED,
        "licensed" => 3,
        "publishing finished" => 4,
        "cancelled" | "canceled" => 5,
        "on hiatus" | "hiatus" => 6,
        _ => return None,
    })
}

fn string_of(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Drops trailing `.0` so 12.0 renders as "12" but 12.5 survives.
fn trim_number(number: f64) -> String {
    if (number.fract()).abs() < f64::EPSILON {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

/// Same normalization the importer uses to match titles across sources.
fn normalize(title: &str) -> String {
    let lowered: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect();
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ImportChapterState, RemoteChapterInfo};
    use crate::komikku::model::Backup;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "dewey_komikku_export_{tag}_{}.tachibk",
            std::process::id()
        ))
    }

    /// One series with a Komikku binding, two usable chapters and one that has
    /// no fetch url (a chapter Dewey can only see on disk).
    fn library() -> Database {
        let db = Database::in_memory().unwrap();
        let series_id = db
            .insert_or_get_series_full("Solo Leveling", None, false, Some("Manhwa"))
            .unwrap();
        db.update_series_fetch_url(series_id, "https://weeb.test/series/abc")
            .unwrap();
        db.update_series_metadata(
            series_id,
            &serde_json::json!({
                "author": "Chugong",
                "artist": "Dubu",
                "description": "Desc",
                "genre": ["Action", "Fantasy"],
                "komikku": {
                    "id": 2499283573021220255i64,
                    "source": "Weeb Central",
                    "url": "/series/abc/Solo-Leveling",
                    "thumbnail_url": "https://img.test/t.jpg",
                    "status": 1,
                    "categories": ["Manhwa", "Best"],
                },
            })
            .to_string(),
        )
        .unwrap();

        db.batch_insert_placeholder_chapters(
            series_id,
            &[
                RemoteChapterInfo {
                    chapter_number: 1.0,
                    title: None,
                    fetch_url: Some("https://weeb.test/c/1".into()),
                },
                RemoteChapterInfo {
                    chapter_number: 2.0,
                    title: Some("The Real Start".into()),
                    fetch_url: Some("https://weeb.test/c/2".into()),
                },
                RemoteChapterInfo {
                    chapter_number: 3.0,
                    title: None,
                    fetch_url: None,
                },
            ],
        )
        .unwrap();

        let chapters = db.get_chapters_for_series(series_id).unwrap();
        let id_of = |number: f64| {
            chapters
                .iter()
                .find(|row| row.chapter.chapter_number == number)
                .unwrap()
                .chapter
                .id
        };
        db.import_chapter_states(
            &[
                ImportChapterState {
                    chapter_id: id_of(1.0),
                    last_page_read: 41,
                    completed: true,
                    bookmarked: false,
                    last_read_at: Some("2026-11-05T10:00:00Z".to_string()),
                },
                ImportChapterState {
                    chapter_id: id_of(2.0),
                    last_page_read: 7,
                    completed: false,
                    bookmarked: true,
                    last_read_at: None,
                },
            ],
            true,
        )
        .unwrap();

        db
    }

    fn read_back(bytes: &[u8], tag: &str) -> Backup {
        let path = temp_path(tag);
        std::fs::write(&path, bytes).unwrap();
        let backup = Backup::from_file(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        backup
    }

    #[test]
    fn round_trips_the_library_through_a_backup_file() {
        let db = library();
        let (bytes, report) = build(&db).unwrap();
        let backup = read_back(&bytes, "roundtrip");

        assert_eq!(report.series, 1);
        assert_eq!(report.chapters, 2);
        assert_eq!(report.chapters_without_url, 1);
        assert_eq!(report.history, 1);
        assert_eq!(backup.mangas.len(), 1);

        let manga = &backup.mangas[0];
        assert_eq!(manga.title, "Solo Leveling");
        assert_eq!(manga.source, 2499283573021220255);
        assert_eq!(manga.url, "/series/abc/Solo-Leveling");
        assert!(manga.favorite, "a Dewey series is a Komikku favorite");
        assert_eq!(manga.status, 1);
        assert_eq!(manga.author.as_deref(), Some("Chugong"));
        assert_eq!(manga.genre, vec!["Action", "Fantasy"]);
        assert_eq!(
            manga.thumbnail_url.as_deref(),
            Some("https://img.test/t.jpg")
        );

        // Categories: the type, then the rating buckets the import recorded.
        let names: Vec<&str> = backup
            .categories
            .iter()
            .map(|category| category.name.as_str())
            .collect();
        assert_eq!(names, vec!["Best", "Manhwa"]);
        let manhwa_id = backup.categories[1].id;
        let best_id = backup.categories[0].id;
        assert_eq!(manga.category_ids, vec![manhwa_id, best_id]);

        // The source list lets Komikku resolve the extension on restore.
        assert_eq!(backup.sources.len(), 1);
        assert_eq!(backup.sources[0].name, "Weeb Central");
        assert_eq!(backup.sources[0].id, 2499283573021220255);

        assert_eq!(manga.chapters.len(), 2);
        let first = &manga.chapters[0];
        assert_eq!(first.url, "https://weeb.test/c/1");
        assert_eq!(first.name, "Chapter 1");
        assert!(first.read);
        assert_eq!(first.last_page_read, 41);
        assert_eq!(first.chapter_number, 1.0);

        let second = &manga.chapters[1];
        assert_eq!(second.name, "The Real Start");
        assert!(!second.read);
        assert!(second.bookmark);
        assert_eq!(second.last_page_read, 7);

        assert_eq!(manga.history.len(), 1);
        assert_eq!(manga.history[0].url, "https://weeb.test/c/1");
        // 2026-11-05T10:00:00Z, in the millis Komikku stores.
        assert_eq!(manga.history[0].last_read, 1_793_872_800_000);
    }

    #[test]
    fn borrowed_binding_exports_but_unbound_series_does_not() {
        let db = library();

        // A scanned series with the same title borrows the Komikku binding.
        let twin = db
            .insert_or_get_series_full("Solo  Leveling!", None, false, Some("Manhwa"))
            .unwrap();
        db.update_series_fetch_url(twin, "https://weeb.test/series/abc")
            .unwrap();

        // A series with no provenance and no twin has nowhere to go.
        let orphan = db
            .insert_or_get_series_full("Local Only", None, false, Some("Comicbook"))
            .unwrap();
        db.update_series_metadata(orphan, "{}").unwrap();

        let (bytes, report) = build(&db).unwrap();
        let backup = read_back(&bytes, "borrowed");

        assert_eq!(report.series, 2);
        assert_eq!(report.series_skipped, 1);
        assert!(backup
            .mangas
            .iter()
            .all(|manga| manga.title != "Local Only"));
        assert_eq!(backup.sources.len(), 1, "one source, not one per series");
    }

    #[test]
    fn comicbook_categories_become_comics() {
        let db = Database::in_memory().unwrap();
        let id = db
            .insert_or_get_series_full("Bound", None, false, Some("Comicbook"))
            .unwrap();
        db.update_series_metadata(
            id,
            &serde_json::json!({
                "komikku": {
                    "id": 7,
                    "source": "Comics Source",
                    "url": "https://comics.test/s/1",
                    "status": 0,
                    "categories": ["Comics", "Good"],
                },
            })
            .to_string(),
        )
        .unwrap();

        let (bytes, _) = build(&db).unwrap();
        let backup = read_back(&bytes, "categories");

        let names: Vec<&str> = backup
            .categories
            .iter()
            .map(|category| category.name.as_str())
            .collect();
        assert_eq!(names, vec!["Comics", "Good"], "no duplicate type entries");
        assert_eq!(
            backup.mangas[0].status, 0,
            "an unknown status stays unknown"
        );
    }

    #[test]
    fn status_ids_match_komikku_constants() {
        assert_eq!(status_id(Some("Ongoing")), Some(1));
        assert_eq!(status_id(Some("Completed")), Some(2));
        assert_eq!(status_id(Some("Cancelled")), Some(5));
        assert_eq!(status_id(Some("On Hiatus")), Some(6));
        assert_eq!(status_id(Some("weird")), None);
        assert_eq!(status_id(None), None);
    }
}
