//! Turns a decoded Komikku backup into Dewey series, chapters, progress and
//! bookmarks.
//!
//! The work is split in two so it can be tested without a database: [`plan`] is
//! pure (it takes the backup plus the rows currently in the database and returns
//! what *would* be written) and [`apply`] performs the writes. Both share
//! [`ImportReport`], which is what the CLI prints.
//!
//! Rules encoded here (see `docs/adr/0004-komikku-backup-import.md`):
//!
//! * Only favorited series are imported. Un-favorited twins exist because the
//!   user migrated sources inside Komikku; their *reading state* is still unioned
//!   in, matched by chapter number.
//! * Dewey's category comes from Komikku's type tags: `Manga`, `Manhwa`,
//!   `Comics` → `Comicbook`. Everything else (`H`, `Best`, `Good`, `Okay`,
//!   `Unrated`, `Partial`) is a rating bucket: `H` becomes a hidden `Manga`,
//!   anything untagged a hidden `Manhwa`.
//! * A series is visible only when it carries a type tag.
//! * Existing rows are never downgraded: `category`, `is_hidden` and
//!   `reading_mode` are written for new rows only, and reading progress is a
//!   monotone union unless `--replace` is given.
//! * Relative chapter URLs are not stored as `fetch_url` (Labrador needs an
//!   absolute URL); they are resolved only when `[komikku.sources]` supplies a
//!   base URL for that source.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::json;

use super::model::{Backup, BackupChapter, BackupManga, BackupSource};
use crate::config::Config;
use crate::db::models::SeriesWithStats;
use crate::db::{Database, ImportChapterState, RemoteChapterInfo};

/// Dewey's library types. `Comicbook` keeps its name deliberately: the category
/// column is a path fragment and "Comic" would be ambiguous in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeweyType {
    Manga,
    Manhwa,
    Comicbook,
}

impl DeweyType {
    pub fn category(self) -> &'static str {
        match self {
            DeweyType::Manga => "Manga",
            DeweyType::Manhwa => "Manhwa",
            DeweyType::Comicbook => "Comicbook",
        }
    }

    pub fn reading_mode(self) -> &'static str {
        match self {
            DeweyType::Manhwa => "webtoon",
            DeweyType::Manga | DeweyType::Comicbook => "manga",
        }
    }
}

/// One chapter to be created (or to have its fetch URL refreshed).
#[derive(Debug, Clone)]
pub struct PlannedChapter {
    pub number: f64,
    /// Chapter name as Komikku stored it; `None` when it had none.
    pub title: Option<String>,
    pub fetch_url: Option<String>,
}

/// Reading state to apply after the chapter exists in the database.
#[derive(Debug, Clone)]
pub struct PlannedChapterState {
    pub number: f64,
    pub last_page_read: i64,
    pub completed: bool,
    pub bookmarked: bool,
    pub last_read_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlannedSeries {
    /// Set when the series already exists locally (matched by normalized title).
    pub existing_id: Option<i64>,
    pub title: String,
    pub category: &'static str,
    pub hidden: bool,
    pub reading_mode: &'static str,
    pub status: Option<&'static str>,
    pub metadata_json: String,
    pub write_status: bool,
    pub write_metadata: bool,
    pub write_reading_mode: bool,
    pub chapters: Vec<PlannedChapter>,
    pub states: Vec<PlannedChapterState>,
}

#[derive(Debug, Default, Clone)]
pub struct ImportPlan {
    pub series: Vec<PlannedSeries>,
    pub report: ImportReport,
}

#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub mangas_in_backup: usize,
    pub favorites: usize,
    /// Favorited copies merged into another favorited copy (same title).
    pub duplicates_merged: usize,
    /// Un-favorited copies merged into a favorite: how many contributed state.
    pub twins_merged: usize,
    /// Un-favorited copies ignored entirely (no reading state to contribute).
    pub twins_skipped: usize,
    pub series_created: usize,
    pub series_matched: usize,
    pub series_hidden: usize,
    pub chapters: usize,
    pub completed_markers: usize,
    pub partial_markers: usize,
    pub bookmarks: usize,
    pub unioned_read: usize,
    pub unrecoverable_read: usize,
    pub urls_absolute: usize,
    pub urls_resolved: usize,
    pub urls_relative: usize,
    /// Sources whose relative links could not be resolved, with how many
    /// chapters that left without a fetch url. This is the config TODO list.
    pub missing_base_url: BTreeMap<String, usize>,
}

impl fmt::Display for ImportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = format!(
            "Komikku backup: {} series, {} favorited\n",
            self.mangas_in_backup, self.favorites
        );
        out.push_str(&format!(
            "  merged: {} favorited duplicates, {} un-favorited twins\n",
            self.duplicates_merged, self.twins_merged
        ));
        if self.twins_skipped > 0 {
            out.push_str(&format!(
                "  ignored: {} un-favorited series with no reading state\n",
                self.twins_skipped
            ));
        }
        out.push_str(&format!(
            "  series: {} created ({} hidden), {} matched existing rows\n",
            self.series_created, self.series_hidden, self.series_matched
        ));
        out.push_str(&format!("  chapters: {}\n", self.chapters));
        out.push_str(&format!(
            "  progress: {} completed, {} partial, {} bookmarks\n",
            self.completed_markers, self.partial_markers, self.bookmarks
        ));
        if self.unioned_read > 0 || self.unrecoverable_read > 0 {
            out.push_str(&format!(
                "  union from twins: {} read markers recovered, {} unrecoverable\n",
                self.unioned_read, self.unrecoverable_read
            ));
        }
        out.push_str(&format!(
            "  chapter urls: {} absolute, {} resolved from config, {} relative (no fetch_url)\n",
            self.urls_absolute, self.urls_resolved, self.urls_relative
        ));
        if !self.missing_base_url.is_empty() {
            let mut sources: Vec<(&String, &usize)> = self.missing_base_url.iter().collect();
            sources.sort_by(|left, right| right.1.cmp(left.1).then(left.0.cmp(right.0)));
            out.push_str("  sources needing a base URL (chapters imported without fetch_url):\n");
            for (name, count) in sources {
                out.push_str(&format!("    {name}: {count}\n"));
            }
            out.push_str(
                "  TODO: add these to [komikku.sources] in the config and re-import with\n        --replace. Until then their chapters cannot be fetched or exported.\n",
            );
        }
        write!(f, "{out}")
    }
}

/// Normalizes a title for identity matching: lowercase, punctuation collapsed to
/// single spaces. `"GOSU"`, `"Gosu"` and `"Gosu!"` all match.
pub fn normalize_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut pending_space = false;
    for ch in title.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(ch);
        } else {
            pending_space = true;
        }
    }
    out
}

/// Maps Komikku category names to a Dewey type plus visibility.
///
/// A type tag outranks the rating buckets; `H` alone becomes a hidden manga and
/// everything untagged a hidden manhwa.
pub fn classify(category_names: &[String]) -> (DeweyType, bool) {
    let has = |want: &str| category_names.iter().any(|n| n.eq_ignore_ascii_case(want));

    if has("Manga") {
        return (DeweyType::Manga, true);
    }
    if has("Manhwa") {
        return (DeweyType::Manhwa, true);
    }
    if has("Comics") {
        return (DeweyType::Comicbook, true);
    }
    if has("H") {
        return (DeweyType::Manga, false);
    }
    (DeweyType::Manhwa, false)
}

/// Komikku's `SManga` status constants → Dewey's free-text status.
pub fn status_label(raw: i32) -> Option<&'static str> {
    match raw {
        1 => Some("Ongoing"),
        2 | 4 => Some("Completed"),
        5 => Some("Cancelled"),
        6 => Some("On Hiatus"),
        // 0 = unknown, 3 = licensed: no Dewey equivalent, keep the column default
        _ => None,
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct ChapterState {
    read: bool,
    bookmarked: bool,
    last_page: i64,
}

impl ChapterState {
    fn merge(&mut self, other: ChapterState) {
        self.read |= other.read;
        self.bookmarked |= other.bookmarked;
        self.last_page = self.last_page.max(other.last_page);
    }

    fn is_empty(&self) -> bool {
        !self.read && !self.bookmarked && self.last_page == 0
    }
}

fn chapter_state(chapter: &BackupChapter) -> ChapterState {
    ChapterState {
        read: chapter.read,
        bookmarked: chapter.bookmark,
        last_page: chapter.last_page_read.max(0),
    }
}

/// Ranks a favorited copy: the copy the user actually read wins.
fn read_score(manga: &BackupManga) -> (usize, usize, usize) {
    let read = manga.chapters.iter().filter(|c| c.read).count();
    let bookmarks = manga.chapters.iter().filter(|c| c.bookmark).count();
    (read, bookmarks, manga.chapters.len())
}

fn rfc3339_from_millis(millis: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(millis).map(|dt| dt.to_rfc3339())
}

fn join_url(base: &str, raw: &str) -> String {
    let base = base.trim_end_matches('/');
    if raw.starts_with('/') {
        format!("{base}{raw}")
    } else {
        format!("{base}/{raw}")
    }
}

fn chapter_number(chapter: &BackupChapter) -> Option<f64> {
    let number = f64::from(chapter.chapter_number);
    if number > 0.0 && number.is_finite() {
        Some(number)
    } else {
        None
    }
}

fn resolve_fetch_url(
    raw: &str,
    source_name: Option<&str>,
    config: &Config,
    report: &mut ImportReport,
) -> Option<String> {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        report.urls_absolute += 1;
        return Some(raw.to_string());
    }

    if let Some(base) = source_name.and_then(|name| config.komikku.sources.get(name)) {
        report.urls_resolved += 1;
        return Some(join_url(base, raw));
    }

    report.urls_relative += 1;
    // TODO(komikku-sources): this chapter keeps no link at all because its
    // source has no `[komikku.sources]` base URL, so it can neither be fetched
    // by Labrador nor written back by `--export-komikku`. The fix is config, not
    // code: add the base URL for this source and re-import with `--replace`,
    // which re-resolves every link through this function.
    *report
        .missing_base_url
        .entry(source_name.unwrap_or("(source not in backup)").to_string())
        .or_default() += 1;
    None
}

/// Builds the plan without touching the database.
pub fn plan(backup: &Backup, existing: &[SeriesWithStats], config: &Config) -> ImportPlan {
    let category_names: HashMap<i64, String> = backup
        .categories
        .iter()
        .map(|c| (c.id, c.name.clone()))
        .collect();
    let source_names: HashMap<i64, &str> = backup
        .sources
        .iter()
        .map(|s: &BackupSource| (s.id, s.name.as_str()))
        .collect();

    let mut report = ImportReport {
        mangas_in_backup: backup.mangas.len(),
        favorites: backup.mangas.iter().filter(|m| m.favorite).count(),
        ..Default::default()
    };

    let mut existing_by_title: HashMap<String, i64> = HashMap::new();
    for row in existing {
        existing_by_title
            .entry(normalize_title(&row.series.title))
            .or_insert(row.series.id);
    }

    // Group every backup entry (favorited or not) by normalized title.
    let mut groups: HashMap<String, Vec<&BackupManga>> = HashMap::new();
    for manga in &backup.mangas {
        groups
            .entry(normalize_title(&manga.title))
            .or_default()
            .push(manga);
    }
    let mut titles: Vec<&String> = groups.keys().collect();
    titles.sort();

    let mut planned = Vec::new();

    for title_key in titles {
        let members = &groups[title_key];

        let mut favorites: Vec<&BackupManga> =
            members.iter().copied().filter(|m| m.favorite).collect();
        if favorites.is_empty() {
            report.twins_skipped += members.len();
            continue;
        }

        // Deterministic: most-read copy wins, ties broken by source id.
        favorites.sort_by(|a, b| {
            read_score(b)
                .cmp(&read_score(a))
                .then_with(|| a.source.cmp(&b.source))
                .then_with(|| a.url.cmp(&b.url))
        });
        let primary = favorites[0];
        let others: Vec<&BackupManga> = members
            .iter()
            .copied()
            .filter(|m| !std::ptr::eq(*m, primary))
            .collect();
        report.duplicates_merged += favorites.len() - 1;

        let category_names_of = |manga: &BackupManga| -> Vec<String> {
            manga
                .category_ids
                .iter()
                .filter_map(|id| category_names.get(id).cloned())
                .collect()
        };

        // Chapter state from the primary copy, keyed by chapter number bits.
        let mut order: Vec<f64> = Vec::new();
        let mut states: HashMap<u64, ChapterState> = HashMap::new();
        let mut raw_urls: HashMap<u64, String> = HashMap::new();
        let mut names: HashMap<u64, String> = HashMap::new();
        let mut unnumbered = 0usize;

        for chapter in &primary.chapters {
            match chapter_number(chapter) {
                Some(number) => {
                    let key = number.to_bits();
                    if let Some(existing_state) = states.get_mut(&key) {
                        existing_state.merge(chapter_state(chapter));
                    } else {
                        states.insert(key, chapter_state(chapter));
                        raw_urls.insert(key, chapter.url.clone());
                        if !chapter.name.is_empty() {
                            names.insert(key, chapter.name.clone());
                        }
                        order.push(number);
                    }
                }
                None => unnumbered += 1,
            }
        }

        // Union reading state from every other copy (favorited twins and
        // un-favorited migration leftovers alike), matched by chapter number.
        for other in &others {
            let mut contributed = false;
            for chapter in &other.chapters {
                let state = chapter_state(chapter);
                if state.is_empty() {
                    continue;
                }
                let number = match chapter_number(chapter) {
                    Some(number) => number,
                    None => {
                        if state.read {
                            report.unrecoverable_read += 1;
                        }
                        continue;
                    }
                };
                match states.get_mut(&number.to_bits()) {
                    Some(existing_state) => {
                        if !existing_state.read && state.read {
                            report.unioned_read += 1;
                        }
                        existing_state.merge(state);
                        contributed = true;
                    }
                    None => {
                        if state.read {
                            report.unrecoverable_read += 1;
                        }
                    }
                }
            }
            if contributed {
                report.twins_merged += 1;
            }
        }

        // Timestamps come from the history lists, which are keyed by chapter URL.
        let mut last_read: HashMap<u64, i64> = HashMap::new();
        let mut all_copies: Vec<&BackupManga> = Vec::with_capacity(1 + others.len());
        all_copies.push(primary);
        all_copies.extend(others.iter().copied());
        for member in all_copies {
            let url_to_number: HashMap<&str, f64> = member
                .chapters
                .iter()
                .filter_map(|c| chapter_number(c).map(|n| (c.url.as_str(), n)))
                .collect();
            for entry in &member.history {
                if let Some(number) = url_to_number.get(entry.url.as_str()) {
                    let slot = last_read.entry(number.to_bits()).or_insert(entry.last_read);
                    *slot = (*slot).max(entry.last_read);
                }
            }
        }

        // Unnumbered chapters sort after everything else, in backup order.
        let mut next_number = order.iter().copied().fold(0.0_f64, f64::max);
        for _ in 0..unnumbered {
            next_number += 0.001;
            order.push(next_number);
        }

        let primary_categories = category_names_of(primary);
        let (dewey_type, visible) = classify(&primary_categories);
        let existing_id = existing_by_title.get(title_key).copied();
        let status = status_label(primary.status);
        let source_name = source_names.get(&primary.source).copied();

        let mut chapters = Vec::new();
        let mut chapter_states = Vec::new();
        let mut completed = 0;
        let mut partial = 0;
        let mut bookmarks = 0;

        for number in &order {
            let key = number.to_bits();
            let state = states.get(&key).copied().unwrap_or_default();
            let raw_url = raw_urls.get(&key).map(String::as_str).unwrap_or("");

            chapters.push(PlannedChapter {
                number: *number,
                title: names.get(&number.to_bits()).cloned(),
                fetch_url: resolve_fetch_url(raw_url, source_name, config, &mut report),
            });

            if state.is_empty() {
                continue;
            }
            if state.bookmarked {
                bookmarks += 1;
            }
            if state.read {
                completed += 1;
            } else if state.last_page > 0 {
                partial += 1;
            }
            chapter_states.push(PlannedChapterState {
                number: *number,
                last_page_read: state.last_page,
                completed: state.read,
                bookmarked: state.bookmarked,
                last_read_at: last_read
                    .get(&key)
                    .and_then(|millis| rfc3339_from_millis(*millis)),
            });
        }

        report.chapters += chapters.len();
        report.completed_markers += completed;
        report.partial_markers += partial;
        report.bookmarks += bookmarks;

        match existing_id {
            None => {
                report.series_created += 1;
                if !visible {
                    report.series_hidden += 1;
                }
            }
            Some(_) => report.series_matched += 1,
        }

        let metadata = json!({
            "author": primary.author,
            "artist": primary.artist,
            "description": primary.description,
            "genre": primary.genre,
            "komikku": {
                "id": primary.source,
                "source": source_name,
                "url": primary.url,
                "thumbnail_url": primary.thumbnail_url,
                "categories": primary_categories,
                "status": primary.status,
                "chapters": primary.chapters.len(),
                "merged_from": others.iter().map(|m| m.source).collect::<Vec<_>>(),
            }
        });

        planned.push(PlannedSeries {
            existing_id,
            title: primary.title.clone(),
            category: dewey_type.category(),
            hidden: !visible,
            reading_mode: dewey_type.reading_mode(),
            status,
            metadata_json: metadata.to_string(),
            write_status: status.is_some() && existing_id.is_none(),
            write_metadata: existing_id.is_none(),
            write_reading_mode: existing_id.is_none(),
            chapters,
            states: chapter_states,
        });
    }

    ImportPlan {
        series: planned,
        report,
    }
}

/// Applies a plan to the database. `replace` makes imported reading progress
/// overwrite local progress instead of unioning into it.
pub fn apply(db: &Database, plan: &ImportPlan, replace: bool) -> Result<ImportReport> {
    for series in &plan.series {
        let series_id = match series.existing_id {
            Some(id) => id,
            None => db.insert_or_get_series_full(
                &series.title,
                None,
                series.hidden,
                Some(series.category),
            )?,
        };

        if series.write_status {
            if let Some(status) = series.status {
                db.update_series_status(series_id, status)?;
            }
        }
        if series.write_metadata {
            db.update_series_metadata(series_id, &series.metadata_json)?;
        }
        if series.write_reading_mode {
            db.update_series_reading_mode(series_id, series.reading_mode)?;
        }

        let remote: Vec<RemoteChapterInfo> = series
            .chapters
            .iter()
            .map(|chapter| RemoteChapterInfo {
                chapter_number: chapter.number,
                title: chapter.title.clone(),
                fetch_url: chapter.fetch_url.clone(),
            })
            .collect();
        db.batch_insert_placeholder_chapters(series_id, &remote)?;

        if series.states.is_empty() {
            continue;
        }

        let chapter_ids: HashMap<u64, i64> = db
            .get_chapters_for_series(series_id)?
            .into_iter()
            .map(|row| (row.chapter.chapter_number.to_bits(), row.chapter.id))
            .collect();

        let entries: Vec<ImportChapterState> = series
            .states
            .iter()
            .filter_map(|state| {
                chapter_ids
                    .get(&state.number.to_bits())
                    .map(|id| ImportChapterState {
                        chapter_id: *id,
                        last_page_read: state.last_page_read,
                        completed: state.completed,
                        bookmarked: state.bookmarked,
                        last_read_at: state.last_read_at.clone(),
                    })
            })
            .collect();
        db.import_chapter_states(&entries, replace)?;
    }

    Ok(plan.report.clone())
}

/// Reads, plans and (unless `dry_run`) applies an import.
pub fn run(
    db: &Database,
    path: &std::path::Path,
    config: &Config,
    dry_run: bool,
    replace: bool,
) -> Result<ImportReport> {
    let backup = Backup::from_file(path)?;
    let existing = db.get_all_series()?;
    let plan = plan(&backup, &existing, config);

    if dry_run {
        return Ok(plan.report);
    }

    apply(db, &plan, replace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{Series, SeriesStats};

    fn chapter(
        number: f32,
        read: bool,
        bookmark: bool,
        last_page: i64,
        url: &str,
    ) -> BackupChapter {
        BackupChapter {
            url: url.to_string(),
            name: format!("Chapter {number}"),
            scanlator: None,
            read,
            bookmark,
            last_page_read: last_page,
            chapter_number: number,
            source_order: 0,
        }
    }

    fn manga(title: &str, source: i64, favorite: bool, cats: Vec<i64>) -> BackupManga {
        BackupManga {
            source,
            url: format!("/series/{title}"),
            title: title.to_string(),
            artist: None,
            author: Some("Author".into()),
            description: Some("Desc".into()),
            genre: vec!["Action".into()],
            status: 1,
            thumbnail_url: Some("https://img.test/t.jpg".into()),
            viewer: 0,
            category_ids: cats,
            favorite,
            chapters: Vec::new(),
            history: Vec::new(),
        }
    }

    const CATS: &[(i64, &str)] = &[
        (1, "Manga"),
        (2, "Comics"),
        (3, "Manhwa"),
        (4, "H"),
        (5, "Best"),
        (8, "Unrated"),
        (9, "Partial"),
    ];

    fn backup(mangas: Vec<BackupManga>) -> Backup {
        Backup {
            categories: CATS
                .iter()
                .map(|(id, name)| crate::komikku::model::BackupCategory {
                    id: *id,
                    name: (*name).to_string(),
                    hidden: false,
                })
                .collect(),
            sources: vec![BackupSource {
                id: 1,
                name: "Test Source".into(),
            }],
            mangas,
        }
    }

    fn config() -> Config {
        Config::default()
    }

    fn existing_row(id: i64, title: &str, category: Option<&str>) -> SeriesWithStats {
        SeriesWithStats {
            series: Series {
                id,
                title: title.to_string(),
                sort_title: Some(title.to_string()),
                cover_path: None,
                status: None,
                fetch_url: None,
                metadata_json: None,
                reading_mode: Some("webtoon".into()),
                is_hidden: false,
                category: category.map(str::to_string),
                chapters_checked_at: None,
            },
            stats: SeriesStats {
                total_chapters: 0,
                downloaded_chapters: 0,
                completed_chapters: 0,
                latest_read_chapter: None,
                last_read_at: None,
            },
        }
    }

    #[test]
    fn reports_sources_that_need_a_base_url() {
        // Source id 1 is "Test Source" in the fixture backup.
        let mut fav = manga("No Base", 1, true, vec![3]);
        fav.chapters = vec![
            chapter(1.0, false, false, 0, "/chapters/relative"),
            chapter(2.0, false, false, 0, "https://absolute.test/2"),
        ];

        let plan = plan(&backup(vec![fav]), &[], &config());

        assert_eq!(plan.report.urls_relative, 1);
        assert_eq!(plan.report.urls_absolute, 1);
        assert_eq!(
            plan.report.missing_base_url.get("Test Source").copied(),
            Some(1),
            "the source is named in the report so it can be configured"
        );
        let printed = plan.report.to_string();
        assert!(printed.contains("sources needing a base URL"));
        assert!(printed.contains("Test Source: 1"));
        assert!(printed.contains("--replace"));
    }

    #[test]
    fn normalize_title_ignores_punctuation_and_case() {
        assert_eq!(normalize_title("GOSU"), "gosu");
        assert_eq!(normalize_title("Gosu!"), "gosu");
        assert_eq!(
            normalize_title("  The  Solo   Leveling "),
            "the solo leveling"
        );
        assert_eq!(normalize_title("Re:Monster"), "re monster");
    }

    #[test]
    fn classify_follows_the_taxonomy() {
        let names = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert_eq!(classify(&names(&["Manga"])), (DeweyType::Manga, true));
        assert_eq!(classify(&names(&["Manhwa"])), (DeweyType::Manhwa, true));
        assert_eq!(classify(&names(&["Comics"])), (DeweyType::Comicbook, true));
        assert_eq!(classify(&names(&["H"])), (DeweyType::Manga, false));
        assert_eq!(classify(&names(&["Best"])), (DeweyType::Manhwa, false));
        assert_eq!(classify(&names(&[])), (DeweyType::Manhwa, false));
        assert_eq!(
            classify(&names(&["Manhwa", "Partial"])),
            (DeweyType::Manhwa, true)
        );
    }

    #[test]
    fn status_mapping_matches_smanga_constants() {
        assert_eq!(status_label(1), Some("Ongoing"));
        assert_eq!(status_label(2), Some("Completed"));
        assert_eq!(status_label(4), Some("Completed"));
        assert_eq!(status_label(5), Some("Cancelled"));
        assert_eq!(status_label(6), Some("On Hiatus"));
        assert_eq!(status_label(0), None);
        assert_eq!(status_label(3), None);
    }

    #[test]
    fn imports_favorites_only_and_unions_twins() {
        let mut fav = manga("Solo Leveling", 10, true, vec![3]);
        fav.chapters = vec![
            chapter(1.0, true, false, 10, "https://a.test/1"),
            chapter(2.0, false, false, 0, "https://a.test/2"),
            chapter(3.0, false, false, 0, "https://a.test/3"),
        ];
        let mut twin = manga("Solo Leveling", 20, false, vec![3]);
        twin.chapters = vec![
            chapter(2.0, true, true, 5, "/b/2"),
            // chapter 9 does not exist in the favorite copy
            chapter(9.0, true, false, 1, "/b/9"),
        ];

        let plan = plan(&backup(vec![fav, twin]), &[], &config());

        assert_eq!(plan.series.len(), 1);
        let series = &plan.series[0];
        assert_eq!(series.title, "Solo Leveling");
        assert_eq!(series.category, "Manhwa");
        assert!(!series.hidden);
        assert_eq!(series.chapters.len(), 3);
        assert_eq!(plan.report.twins_merged, 1);
        assert_eq!(plan.report.unioned_read, 1);
        assert_eq!(plan.report.unrecoverable_read, 1);

        let state2 = series
            .states
            .iter()
            .find(|s| s.number == 2.0)
            .expect("chapter 2 has state");
        assert!(state2.completed);
        assert!(state2.bookmarked);
        assert_eq!(state2.last_page_read, 5);
    }

    #[test]
    fn most_read_copy_wins_among_favorites() {
        let mut read_copy = manga("sexual exploits", 1, true, vec![8]);
        read_copy.chapters = vec![chapter(1.0, true, true, 3, "https://webtoon.test/1")];
        let mut unread_copy = manga("sexual exploits", 2, true, vec![8]);
        unread_copy.chapters = vec![
            chapter(1.0, false, false, 0, "chapter-1"),
            chapter(2.0, false, false, 0, "chapter-2"),
        ];

        let plan = plan(&backup(vec![unread_copy, read_copy]), &[], &config());

        assert_eq!(plan.series.len(), 1);
        assert_eq!(plan.report.duplicates_merged, 1);
        assert_eq!(plan.series[0].chapters.len(), 1);
        assert_eq!(
            plan.series[0].chapters[0].fetch_url.as_deref(),
            Some("https://webtoon.test/1")
        );
        assert_eq!(plan.report.completed_markers, 1);
    }

    #[test]
    fn relative_urls_are_not_stored_as_fetch_urls() {
        let mut m = manga("Chainsaw Man", 1, true, vec![1]);
        m.chapters = vec![
            chapter(1.0, false, false, 0, "/chapters/01J76X"),
            chapter(2.0, false, false, 0, "https://weeb.test/2"),
        ];

        let plan = plan(&backup(vec![m]), &[], &config());
        assert_eq!(plan.series[0].chapters[0].fetch_url, None);
        assert_eq!(
            plan.series[0].chapters[1].fetch_url.as_deref(),
            Some("https://weeb.test/2")
        );
        assert_eq!(plan.report.urls_relative, 1);
        assert_eq!(plan.report.urls_absolute, 1);
    }

    #[test]
    fn configured_source_base_resolves_relative_urls() {
        let mut m = manga("Weeb Thing", 1, true, vec![1]);
        m.chapters = vec![chapter(1.0, false, false, 0, "/chapters/01J76X")];

        let mut cfg = config();
        cfg.komikku.sources.insert(
            "Test Source".to_string(),
            "https://weebcentral.com/".to_string(),
        );

        let plan = plan(&backup(vec![m]), &[], &cfg);
        assert_eq!(
            plan.series[0].chapters[0].fetch_url.as_deref(),
            Some("https://weebcentral.com/chapters/01J76X")
        );
        assert_eq!(plan.report.urls_resolved, 1);
        assert_eq!(plan.report.urls_relative, 0);
    }

    #[test]
    fn hidden_rules_follow_the_category_tag() {
        let mut h_only = manga("Adult Thing", 1, true, vec![4, 8]);
        h_only.chapters = vec![chapter(1.0, false, false, 0, "https://a/1")];
        let mut untagged = manga("Unknown Thing", 1, true, vec![5]);
        untagged.chapters = vec![chapter(1.0, false, false, 0, "https://a/1")];
        let mut typed = manga("Comic Thing", 1, true, vec![2, 9]);
        typed.chapters = vec![chapter(1.0, false, false, 0, "https://a/1")];

        let plan = plan(&backup(vec![h_only, untagged, typed]), &[], &config());

        let find = |title: &str| {
            plan.series
                .iter()
                .find(|s| s.title == title)
                .expect("series planned")
        };
        assert_eq!(find("Adult Thing").category, "Manga");
        assert!(find("Adult Thing").hidden);
        assert_eq!(find("Unknown Thing").category, "Manhwa");
        assert!(find("Unknown Thing").hidden);
        assert_eq!(find("Comic Thing").category, "Comicbook");
        assert!(!find("Comic Thing").hidden);
        assert_eq!(plan.report.series_hidden, 2);
    }

    #[test]
    fn reading_mode_follows_the_type() {
        let mut c = manga("Comic Thing", 1, true, vec![2]);
        c.chapters = vec![chapter(1.0, false, false, 0, "https://a/1")];
        let mut w = manga("Manhwa Thing", 1, true, vec![3]);
        w.chapters = vec![chapter(1.0, false, false, 0, "https://a/1")];

        let plan = plan(&backup(vec![c, w]), &[], &config());
        let find = |title: &str| {
            plan.series
                .iter()
                .find(|s| s.title == title)
                .expect("series planned")
        };
        assert_eq!(find("Comic Thing").reading_mode, "manga");
        assert_eq!(find("Manhwa Thing").reading_mode, "webtoon");
    }

    #[test]
    fn unnumbered_chapters_get_sequential_numbers() {
        let mut m = manga("Nameless", 1, true, vec![3]);
        m.chapters = vec![
            chapter(5.0, false, false, 0, "https://a/5"),
            chapter(0.0, false, false, 0, "https://a/x"),
            chapter(0.0, false, false, 0, "https://a/y"),
        ];

        let plan = plan(&backup(vec![m]), &[], &config());
        let numbers: Vec<f64> = plan.series[0].chapters.iter().map(|c| c.number).collect();
        assert_eq!(numbers.len(), 3);
        assert_eq!(numbers[0], 5.0);
        assert!((numbers[1] - 5.001).abs() < 1e-9);
        assert!((numbers[2] - 5.002).abs() < 1e-9);
    }

    #[test]
    fn history_supplies_last_read_timestamps() {
        let mut m = manga("Timed", 1, true, vec![3]);
        m.chapters = vec![
            chapter(1.0, true, false, 4, "https://a/1"),
            chapter(2.0, false, false, 0, "https://a/2"),
        ];
        m.history = vec![crate::komikku::model::BackupHistory {
            url: "https://a/1".into(),
            last_read: 1_760_000_000_000,
        }];

        let plan = plan(&backup(vec![m]), &[], &config());
        let state = plan.series[0]
            .states
            .iter()
            .find(|s| s.number == 1.0)
            .expect("state");
        let stamp = state.last_read_at.as_deref().expect("timestamp");
        assert!(stamp.starts_with("2025-10-09"), "got {stamp}");
    }

    #[test]
    fn existing_rows_keep_their_category_and_visibility() {
        let mut m = manga("Chainsaw Man", 1, true, vec![3]);
        m.chapters = vec![chapter(1.0, true, false, 4, "https://a/1")];

        let existing = vec![existing_row(7, "Chainsaw Man", Some("Manga/Action"))];
        let plan = plan(&backup(vec![m]), &existing, &config());

        assert_eq!(plan.report.series_matched, 1);
        assert_eq!(plan.report.series_created, 0);
        let planned = &plan.series[0];
        assert_eq!(planned.existing_id, Some(7));
        assert!(
            !planned.write_reading_mode,
            "existing rows keep their reading mode"
        );
        assert!(
            !planned.write_metadata,
            "existing metadata is not clobbered"
        );
        assert!(!planned.write_status);
    }

    #[test]
    fn series_without_favorites_are_skipped() {
        let mut m = manga("Orphan", 1, false, vec![3]);
        m.chapters = vec![chapter(1.0, true, false, 4, "https://a/1")];

        let plan = plan(&backup(vec![m]), &[], &config());
        assert!(plan.series.is_empty());
        assert_eq!(plan.report.twins_skipped, 1);
    }
}
