use anyhow::{Context, Result};
use ratatui::layout::Rect;
use ratatui::widgets::{ListState, TableState};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, info};

use crate::config::Config;
use crate::db::models::{ChapterWithProgress, SeriesWithStats};
use crate::db::Database;
use crate::event::{AppEvent, DownloadSuccessPayload};
use crate::runner::{ContinuumRunner, LabradorRunner};
use crate::scanner::LibraryScanner;
use crate::terminal::Tui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    SeriesList,
    ChaptersList,
    ActiveDownloads,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    SearchInput,
    CategoryPicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterMode {
    #[default]
    All,
    Unread,
    Ongoing,
    Completed,
}

impl FilterMode {
    pub fn next(self) -> Self {
        match self {
            FilterMode::All => FilterMode::Unread,
            FilterMode::Unread => FilterMode::Ongoing,
            FilterMode::Ongoing => FilterMode::Completed,
            FilterMode::Completed => FilterMode::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FilterMode::All => "All",
            FilterMode::Unread => "Unread",
            FilterMode::Ongoing => "Ongoing",
            FilterMode::Completed => "Completed",
        }
    }
}

/// One-shot actions exposed as tappable buttons in the footer action bar
/// (touchscreen-friendly mirror of the keyboard shortcuts).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    Open,
    Fetch,
    FetchNext,
    Mode,
    MarkRead,
    Scan,
    Reset,
    Delete,
    SwitchPane,
    Search,
    Filter,
    ToggleHidden,
    TagCategory,
    Quit,
}

#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub task_id: u64,
    pub series_id: i64,
    pub series_title: String,
    pub chapter_number: f64,
    pub started_at: Instant,
}

pub struct App {
    pub config: Config,
    pub db: Database,
    pub continuum_runner: ContinuumRunner,
    pub labrador_runner: LabradorRunner,

    pub series_list: Vec<SeriesWithStats>,
    pub selected_series_idx: usize,
    pub series_state: ListState,

    pub chapters_list: Vec<ChapterWithProgress>,
    pub selected_chapter_idx: usize,
    pub chapters_state: TableState,

    pub input_mode: InputMode,
    pub search_query: String,
    pub filter_mode: FilterMode,
    pub filtered_indices: Vec<usize>,
    pub show_hidden: bool,

    pub available_categories: Vec<String>,
    pub category_input: String,
    pub category_selected_idx: usize,

    pub active_pane: ActivePane,
    pub download_jobs: Vec<DownloadJob>,
    pub tick_count: usize,
    pub is_scanning: bool,

    pub toast: Option<(String, bool, Instant)>,
    pub show_help_modal: bool,
    pub pending_delete_id: Option<i64>,
    /// Last-frame render areas, used to hit-test taps.
    pub series_rect: Option<Rect>,
    pub chapters_rect: Option<Rect>,
    /// Tappable tab navigation buttons in portrait mode.
    pub tab_rects: Vec<(Rect, ActivePane)>,
    /// Tappable action-bar buttons rendered by the footer / touch bar.
    pub action_rects: Vec<(Rect, AppAction)>,
    /// (time, pane, index) of the previous left-click, for double-tap open.
    pub last_tap: Option<(Instant, ActivePane, usize)>,
    pub event_tx: UnboundedSender<AppEvent>,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config, db: Database, event_tx: UnboundedSender<AppEvent>) -> Result<Self> {
        let continuum_runner = ContinuumRunner::new(&config.continuum_bin)
            .with_storage_profile(config.storage_profile.as_str());
        let labrador_runner = LabradorRunner::new(&config.labrador_bin);

        let mut series_state = ListState::default();
        series_state.select(Some(0));
        let mut chapters_state = TableState::default();
        chapters_state.select(Some(0));

        let mut app = Self {
            config,
            db,
            continuum_runner,
            labrador_runner,
            series_list: Vec::new(),
            selected_series_idx: 0,
            series_state,
            chapters_list: Vec::new(),
            selected_chapter_idx: 0,
            chapters_state,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            filter_mode: FilterMode::All,
            filtered_indices: Vec::new(),
            show_hidden: false,
            available_categories: Vec::new(),
            category_input: String::new(),
            category_selected_idx: 0,
            active_pane: ActivePane::SeriesList,
            download_jobs: Vec::new(),
            tick_count: 0,
            is_scanning: false,
            toast: None,
            show_help_modal: false,
            pending_delete_id: None,
            series_rect: None,
            chapters_rect: None,
            tab_rects: Vec::new(),
            action_rects: Vec::new(),
            last_tap: None,
            event_tx,
            should_quit: false,
        };

        // 1. Instant startup: load existing SQLite database immediately (< 2ms)
        app.reload_series()?;
        app.reload_chapters()?;

        // 2. Spawn non-blocking background scan if auto_scan_on_startup is true
        if app.config.auto_scan_on_startup && app.config.library_dir.exists() {
            app.spawn_background_scan();
        }

        Ok(app)
    }

    /// Spawns a background thread to scan the library without blocking UI startup or navigation
    pub fn spawn_background_scan(&mut self) {
        if self.is_scanning {
            return;
        }
        self.is_scanning = true;
        let db = self.db.clone();
        let lib_dir = self.config.library_dir.clone();
        let profile = self.config.storage_profile;
        let max_concurrency = self.config.max_scan_concurrency;
        let event_tx = self.event_tx.clone();

        tokio::task::spawn_blocking(move || {
            match LibraryScanner::scan_directory_with_profile(
                &db,
                &lib_dir,
                profile,
                max_concurrency,
            ) {
                Ok(summary) => {
                    let _ = event_tx.send(AppEvent::ScanCompleted(summary));
                }
                Err(err) => {
                    let _ = event_tx.send(AppEvent::Toast {
                        message: format!("Scan error: {}", err),
                        is_error: true,
                    });
                }
            }
        });
    }

    pub fn on_scan_completed(&mut self, summary: crate::scanner::ScanSummary) {
        self.is_scanning = false;
        let _ = self.reload_series();
        let _ = self.reload_chapters();

        if summary.new_chapters_added > 0 {
            self.set_toast(
                format!(
                    "Scanned library: {} series, {} chapters ({} added)",
                    summary.series_found, summary.chapters_found, summary.new_chapters_added
                ),
                false,
            );
        }
    }

    pub fn reload_series(&mut self) -> Result<()> {
        self.series_list = self.db.get_all_series()?;
        self.apply_filter();
        Ok(())
    }

    pub fn apply_filter(&mut self) {
        let query = self.search_query.trim().to_lowercase();
        let query_tokens: Vec<&str> = query.split_whitespace().collect();

        self.filtered_indices = self
            .series_list
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                // 1. Hidden filter: exclude hidden series unless show_hidden is true
                if s.series.is_hidden && !self.show_hidden {
                    return false;
                }

                // 2. Status Filter
                let matches_status = match self.filter_mode {
                    FilterMode::All => true,
                    FilterMode::Unread => {
                        s.stats.completed_chapters < s.stats.total_chapters
                            || s.stats.total_chapters == 0
                    }
                    FilterMode::Ongoing => s
                        .series
                        .status
                        .as_deref()
                        .map(|st| {
                            st.eq_ignore_ascii_case("ongoing")
                                || st.eq_ignore_ascii_case("continuing")
                        })
                        .unwrap_or(false),
                    FilterMode::Completed => s
                        .series
                        .status
                        .as_deref()
                        .map(|st| st.eq_ignore_ascii_case("completed"))
                        .unwrap_or(false),
                };

                if !matches_status {
                    return false;
                }

                // 3. Search Query Tokens
                if query_tokens.is_empty() {
                    return true;
                }

                let title = s.series.title.to_lowercase();
                let sort_title = s.series.sort_title.as_deref().unwrap_or("").to_lowercase();
                let meta = s
                    .series
                    .metadata_json
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase();
                let cat = s.series.category.as_deref().unwrap_or("").to_lowercase();

                query_tokens.iter().all(|&token| {
                    title.contains(token)
                        || sort_title.contains(token)
                        || meta.contains(token)
                        || cat.contains(token)
                })
            })
            .map(|(i, _)| i)
            .collect();

        if self.filtered_indices.is_empty() {
            self.selected_series_idx = 0;
            self.series_state.select(None);
        } else {
            if self.selected_series_idx >= self.filtered_indices.len() {
                self.selected_series_idx = self.filtered_indices.len() - 1;
            }
            self.series_state.select(Some(self.selected_series_idx));
        }

        let _ = self.reload_chapters();
    }

    pub fn open_category_modal(&mut self) {
        if self.current_series().is_none() {
            self.set_toast("No series selected", true);
            return;
        }

        let mut cats = vec![
            "Action".to_string(),
            "Comedy".to_string(),
            "Romance".to_string(),
            "Manga".to_string(),
            "Manga/Action".to_string(),
            "Manga/Comedy".to_string(),
            "Manga/Romance".to_string(),
            "Manhwa".to_string(),
            "Manhwa/Action".to_string(),
            "Manhwa/Comedy".to_string(),
            "Manhwa/Romance".to_string(),
            "Uncategorized".to_string(),
        ];

        if let Ok(db_cats) = self.db.get_all_categories() {
            for c in db_cats {
                if !cats.contains(&c) {
                    cats.push(c);
                }
            }
        }

        self.available_categories = cats;
        self.category_input = self
            .current_series()
            .and_then(|s| s.series.category.clone())
            .unwrap_or_default();
        self.category_selected_idx = 0;
        self.input_mode = InputMode::CategoryPicker;
    }

    pub fn close_category_modal(&mut self) {
        self.input_mode = InputMode::Normal;
    }

    pub fn category_modal_select_next(&mut self) {
        if !self.available_categories.is_empty() {
            self.category_selected_idx =
                (self.category_selected_idx + 1) % self.available_categories.len();
            self.category_input = self.available_categories[self.category_selected_idx].clone();
        }
    }

    pub fn category_modal_select_prev(&mut self) {
        if !self.available_categories.is_empty() {
            if self.category_selected_idx == 0 {
                self.category_selected_idx = self.available_categories.len() - 1;
            } else {
                self.category_selected_idx -= 1;
            }
            self.category_input = self.available_categories[self.category_selected_idx].clone();
        }
    }

    pub fn category_modal_push_char(&mut self, c: char) {
        self.category_input.push(c);
    }

    pub fn category_modal_pop_char(&mut self) {
        self.category_input.pop();
    }

    pub fn confirm_category_selection(&mut self) -> Result<()> {
        let cat = self.category_input.trim().to_string();
        self.close_category_modal();
        self.apply_category_to_selected(&cat)
    }

    pub fn apply_category_to_selected(&mut self, new_category: &str) -> Result<()> {
        let current = match self.current_series() {
            Some(s) => s.clone(),
            None => {
                self.set_toast("No series selected", true);
                return Ok(());
            }
        };

        let series_id = current.series.id;
        let title = &current.series.title;
        let is_hidden = current.series.is_hidden;

        // 1. Locate current series directory
        let chapters = self.db.get_chapters_for_series(series_id)?;
        let existing_file_dir = chapters
            .iter()
            .find_map(|c| c.chapter.file_path.as_deref())
            .map(Path::new)
            .filter(|p| p.exists())
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        let cover_dir = current
            .series
            .cover_path
            .as_deref()
            .map(Path::new)
            .filter(|p| p.exists())
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        let folder_name = existing_file_dir
            .as_ref()
            .or(cover_dir.as_ref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| {
                let safe_title = title.replace(' ', "_");
                if is_hidden {
                    format!(".{}", safe_title)
                } else {
                    safe_title
                }
            });

        let current_dir = existing_file_dir.or(cover_dir);

        // 2. Compute target parent directory
        let clean_cat = new_category.trim().trim_matches('/');
        let is_uncat = clean_cat.is_empty() || clean_cat.eq_ignore_ascii_case("uncategorized");

        let target_parent = if is_uncat {
            if is_hidden {
                self.config.library_dir.join(".Other")
            } else {
                self.config.library_dir.clone()
            }
        } else if is_hidden {
            self.config.library_dir.join(".Other").join(clean_cat)
        } else {
            self.config.library_dir.join(clean_cat)
        };

        let target_dir = target_parent.join(&folder_name);
        let cat_value = if is_uncat { None } else { Some(clean_cat) };

        if let Some(src_dir) = &current_dir {
            if src_dir.exists() && src_dir != &target_dir {
                if target_dir.exists() {
                    self.set_toast(
                        format!("Target directory '{:?}' already exists", target_dir),
                        true,
                    );
                    return Ok(());
                }
                std::fs::create_dir_all(&target_parent).with_context(|| {
                    format!("Failed to create directory {:?}", target_parent)
                })?;
                std::fs::rename(src_dir, &target_dir)
                    .with_context(|| format!("Failed to move {:?} to {:?}", src_dir, target_dir))?;

                // Clean up old parent directory if empty
                if let Some(old_parent) = src_dir.parent() {
                    if old_parent != self.config.library_dir
                        && old_parent != self.config.library_dir.join(".Other")
                    {
                        let _ = std::fs::remove_dir(old_parent);
                    }
                }

                self.db.rename_series_directory_with_category(
                    series_id,
                    src_dir,
                    &target_dir,
                    is_hidden,
                    cat_value,
                )?;
            } else {
                self.db.update_series_category(series_id, cat_value)?;
            }
        } else {
            self.db.update_series_category(series_id, cat_value)?;
        }

        self.reload_series()?;
        let cat_label = cat_value.unwrap_or("Uncategorized");
        self.set_toast(
            format!("Series '{}' moved to [{}]", title, cat_label),
            false,
        );
        Ok(())
    }

    pub fn toggle_show_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.apply_filter();
        self.set_toast(
            if self.show_hidden {
                "Showing hidden series"
            } else {
                "Hiding hidden series"
            },
            false,
        );
    }

    pub fn toggle_selected_series_hidden(&mut self) -> Result<()> {
        let current = match self.current_series() {
            Some(s) => s.clone(),
            None => {
                self.set_toast("No series selected", true);
                return Ok(());
            }
        };

        let series_id = current.series.id;
        let title = &current.series.title;

        // Find existing series directory from chapters or cover
        let chapters = self.db.get_chapters_for_series(series_id)?;
        let existing_file_dir = chapters
            .iter()
            .find_map(|c| c.chapter.file_path.as_deref())
            .map(Path::new)
            .filter(|p| p.exists())
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        let cover_dir = current
            .series
            .cover_path
            .as_deref()
            .map(Path::new)
            .filter(|p| p.exists())
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        let series_dir = existing_file_dir.or(cover_dir).unwrap_or_else(|| {
            let safe_name = title.replace(' ', "_");
            if current.series.is_hidden {
                self.config.library_dir.join(format!(".{}", safe_name))
            } else {
                self.config.library_dir.join(safe_name)
            }
        });

        let folder_name = series_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let parent_dir = series_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.config.library_dir.clone());

        let (new_dir, new_is_hidden) = if folder_name.starts_with('.') {
            let clean_name = folder_name.trim_start_matches('.');
            (parent_dir.join(clean_name), false)
        } else {
            let dot_name = format!(".{}", folder_name);
            (parent_dir.join(dot_name), true)
        };

        if series_dir.exists() {
            if new_dir.exists() {
                let msg = format!("Cannot rename: directory '{:?}' already exists", new_dir);
                self.set_toast(msg, true);
                return Ok(());
            }
            std::fs::rename(&series_dir, &new_dir)
                .with_context(|| format!("Failed to rename {:?} to {:?}", series_dir, new_dir))?;
            self.db
                .rename_series_directory(series_id, &series_dir, &new_dir, new_is_hidden)?;
        } else {
            self.db.update_series_hidden(series_id, new_is_hidden)?;
        }

        self.reload_series()?;
        let status_label = if new_is_hidden { "hidden" } else { "unhidden" };
        self.set_toast(
            format!("Series '{}' marked as {}", title, status_label),
            false,
        );
        Ok(())
    }

    pub fn enter_search_mode(&mut self) {
        self.input_mode = InputMode::SearchInput;
        self.active_pane = ActivePane::SeriesList;
    }

    pub fn exit_search_mode(&mut self, clear: bool) {
        self.input_mode = InputMode::Normal;
        if clear {
            self.search_query.clear();
            self.apply_filter();
        }
    }

    pub fn search_push_char(&mut self, c: char) {
        self.search_query.push(c);
        self.apply_filter();
    }

    pub fn search_pop_char(&mut self) {
        self.search_query.pop();
        self.apply_filter();
    }

    pub fn toggle_filter_mode(&mut self) {
        self.filter_mode = self.filter_mode.next();
        self.apply_filter();
        self.set_toast(format!("Filter: {}", self.filter_mode.label()), false);
    }

    pub fn clear_search_and_filters(&mut self) {
        let had_filters = !self.search_query.is_empty() || self.filter_mode != FilterMode::All;
        self.search_query.clear();
        self.filter_mode = FilterMode::All;
        self.input_mode = InputMode::Normal;
        self.apply_filter();
        if had_filters {
            self.set_toast("Filters cleared", false);
        }
    }

    pub fn reload_chapters(&mut self) -> Result<()> {
        if let Some(current_series) = self.current_series() {
            self.chapters_list = self.db.get_chapters_for_series(current_series.series.id)?;
            if self.selected_chapter_idx >= self.chapters_list.len()
                && !self.chapters_list.is_empty()
            {
                self.selected_chapter_idx = self.chapters_list.len() - 1;
            }
        } else {
            self.chapters_list.clear();
            self.selected_chapter_idx = 0;
        }

        if self.chapters_list.is_empty() {
            self.chapters_state.select(None);
        } else {
            self.chapters_state.select(Some(self.selected_chapter_idx));
        }
        Ok(())
    }

    pub fn current_series(&self) -> Option<&SeriesWithStats> {
        self.filtered_indices
            .get(self.selected_series_idx)
            .and_then(|&idx| self.series_list.get(idx))
    }

    pub fn current_chapter(&self) -> Option<&ChapterWithProgress> {
        self.chapters_list.get(self.selected_chapter_idx)
    }

    /// Selects the series at `idx` in filtered view (tap). Focuses the series pane,
    /// reloads the chapter list for it, and cancels any pending delete confirmation.
    pub fn select_series_index(&mut self, idx: usize) {
        if idx >= self.filtered_indices.len() {
            return;
        }
        self.active_pane = ActivePane::SeriesList;
        self.selected_series_idx = idx;
        self.series_state.select(Some(idx));
        self.selected_chapter_idx = 0;
        let _ = self.reload_chapters();
        self.pending_delete_id = None;
    }

    /// Selects the chapter at `idx` (tap).
    pub fn select_chapter_index(&mut self, idx: usize) {
        if idx >= self.chapters_list.len() {
            return;
        }
        self.active_pane = ActivePane::ChaptersList;
        self.selected_chapter_idx = idx;
        self.chapters_state.select(Some(idx));
        self.pending_delete_id = None;
    }

    /// Registers a tap on (pane, idx); returns true when it is a double-tap on
    /// the same item (i.e. should trigger the open action).
    pub fn handle_tap(&mut self, pane: ActivePane, idx: usize) -> bool {
        let now = Instant::now();
        let is_double = self.last_tap.is_some_and(|(t, p, i)| {
            p == pane && i == idx && now.duration_since(t) < Duration::from_millis(400)
        });
        self.last_tap = Some((now, pane, idx));
        is_double
    }

    pub fn set_toast(&mut self, message: impl Into<String>, is_error: bool) {
        self.toast = Some((message.into(), is_error, Instant::now()));
    }

    pub fn check_toast_expiry(&mut self) {
        if let Some((_, _, time)) = self.toast {
            if time.elapsed() > Duration::from_secs(4) {
                self.toast = None;
            }
        }
    }

    pub fn next_item(&mut self) {
        self.pending_delete_id = None;
        match self.active_pane {
            ActivePane::SeriesList => {
                if !self.filtered_indices.is_empty() {
                    self.selected_series_idx =
                        (self.selected_series_idx + 1) % self.filtered_indices.len();
                    self.series_state.select(Some(self.selected_series_idx));
                    self.selected_chapter_idx = 0;
                    let _ = self.reload_chapters();
                }
            }
            ActivePane::ChaptersList => {
                if !self.chapters_list.is_empty() {
                    self.selected_chapter_idx =
                        (self.selected_chapter_idx + 1) % self.chapters_list.len();
                    self.chapters_state.select(Some(self.selected_chapter_idx));
                }
            }
            ActivePane::ActiveDownloads => {}
        }
    }

    /// `g`: jump to the top (first item) of the active list.
    pub fn jump_list_top(&mut self) {
        self.pending_delete_id = None;
        match self.active_pane {
            ActivePane::SeriesList => self.select_series_index(0),
            ActivePane::ChaptersList => self.select_chapter_index(0),
            ActivePane::ActiveDownloads => {}
        }
    }

    /// `G`: jump to the bottom (last item) of the active list.
    pub fn jump_list_bottom(&mut self) {
        self.pending_delete_id = None;
        match self.active_pane {
            ActivePane::SeriesList => {
                if !self.filtered_indices.is_empty() {
                    self.select_series_index(self.filtered_indices.len() - 1);
                }
            }
            ActivePane::ChaptersList => {
                if !self.chapters_list.is_empty() {
                    self.select_chapter_index(self.chapters_list.len() - 1);
                }
            }
            ActivePane::ActiveDownloads => {}
        }
    }

    pub fn prev_item(&mut self) {
        self.pending_delete_id = None;
        match self.active_pane {
            ActivePane::SeriesList => {
                if !self.filtered_indices.is_empty() {
                    if self.selected_series_idx == 0 {
                        self.selected_series_idx = self.filtered_indices.len() - 1;
                    } else {
                        self.selected_series_idx -= 1;
                    }
                    self.series_state.select(Some(self.selected_series_idx));
                    self.selected_chapter_idx = 0;
                    let _ = self.reload_chapters();
                }
            }
            ActivePane::ChaptersList => {
                if !self.chapters_list.is_empty() {
                    if self.selected_chapter_idx == 0 {
                        self.selected_chapter_idx = self.chapters_list.len() - 1;
                    } else {
                        self.selected_chapter_idx -= 1;
                    }
                    self.chapters_state.select(Some(self.selected_chapter_idx));
                }
            }
            ActivePane::ActiveDownloads => {}
        }
    }

    pub fn switch_pane_forward(&mut self) {
        self.pending_delete_id = None;
        self.active_pane = match self.active_pane {
            ActivePane::SeriesList => ActivePane::ChaptersList,
            ActivePane::ChaptersList => {
                if !self.download_jobs.is_empty() {
                    ActivePane::ActiveDownloads
                } else {
                    ActivePane::SeriesList
                }
            }
            ActivePane::ActiveDownloads => ActivePane::SeriesList,
        };
    }

    pub fn switch_pane_backward(&mut self) {
        self.pending_delete_id = None;
        self.active_pane = match self.active_pane {
            ActivePane::SeriesList => {
                if !self.download_jobs.is_empty() {
                    ActivePane::ActiveDownloads
                } else {
                    ActivePane::ChaptersList
                }
            }
            ActivePane::ChaptersList => ActivePane::SeriesList,
            ActivePane::ActiveDownloads => ActivePane::ChaptersList,
        };
    }

    /// Primary action on selected item (e.g. Enter key)
    pub fn handle_enter_action(&mut self, tui: &mut Tui) -> Result<()> {
        match self.active_pane {
            ActivePane::SeriesList => {
                self.active_pane = ActivePane::ChaptersList;
            }
            ActivePane::ChaptersList => {
                self.read_or_fetch_selected(tui)?;
            }
            ActivePane::ActiveDownloads => {}
        }
        Ok(())
    }

    /// Reading Loop execution with Continuum integration.
    /// If the chapter is not downloaded, triggers Labrador fetch automatically.
    pub fn read_or_fetch_selected(&mut self, tui: &mut Tui) -> Result<()> {
        let current_chap = match self.current_chapter() {
            Some(chap) => chap.clone(),
            None => {
                self.set_toast("No chapter selected", false);
                return Ok(());
            }
        };

        let file_exists = current_chap
            .chapter
            .file_path
            .as_ref()
            .map(|p| Path::new(p).exists())
            .unwrap_or(false);

        if !file_exists {
            // Chapter is not downloaded yet -> Trigger Labrador fetching loop
            self.download_selected_chapter();
            return Ok(());
        }

        let file_path = PathBuf::from(current_chap.chapter.file_path.as_ref().unwrap());
        let last_page = current_chap.last_page();
        let chapter_id = current_chap.chapter.id;
        let chapter_num = current_chap.chapter.chapter_number;

        let (series_id, series_mode) = self
            .current_series()
            .map(|s| (Some(s.series.id), s.series.reading_mode().to_string()))
            .unwrap_or((None, "webtoon".to_string()));

        info!(
            chapter_id,
            chapter_num,
            file = ?file_path,
            last_page,
            mode = %series_mode,
            "Suspending TUI to spawn Continuum reader"
        );

        self.set_toast(
            format!(
                "Reading Chapter {:.1} in Continuum ({} mode)...",
                chapter_num, series_mode
            ),
            false,
        );

        // 1. Suspend TUI raw mode and alternate screen
        tui.suspend()?;

        // 2. Launch Continuum child process & wait for stdout exit payload
        let result =
            self.continuum_runner
                .spawn_and_wait(&file_path, last_page, Some(&series_mode));

        // 3. Resume TUI terminal mode
        tui.resume()?;

        // 4. Handle exit payload, update SQLite progress, and display completion message
        match result {
            Ok(payload) => {
                info!(
                    chapter_id,
                    last_page = payload.last_page,
                    completed = payload.completed,
                    mode = ?payload.mode,
                    "Continuum closed. Updating SQLite progress table."
                );

                // Update series reading mode if returned from Continuum
                if let Some(new_mode) = &payload.mode {
                    if let Some(sid) = series_id {
                        let _ = self.db.update_series_reading_mode(sid, new_mode);
                    }
                }

                // Persist progress for every chapter the reader actually
                // touched; fall back to the legacy single-chapter contract
                // when chapters is absent.
                let updated = match payload.chapters.as_ref().filter(|c| !c.is_empty()) {
                    Some(chapters) => {
                        let mut n = 0usize;
                        for entry in chapters {
                            self.db.apply_chapter_progress(
                                Path::new(&entry.file),
                                entry.last_page,
                                entry.completed,
                            )?;
                            n += 1;
                        }
                        n
                    }
                    None => {
                        self.db.upsert_progress(
                            chapter_id,
                            payload.last_page,
                            payload.completed,
                        )?;
                        1
                    }
                };
                self.reload_chapters()?;
                self.reload_series()?;

                let mut status_msg = payload.completion_message(chapter_num);
                if updated > 1 {
                    status_msg = format!("{} · {} chapters updated", status_msg, updated);
                }
                self.set_toast(status_msg, false);
            }
            Err(err) => {
                error!(error = %err, "Error executing Continuum");
                self.set_toast(format!("Continuum error: {}", err), true);
            }
        }

        Ok(())
    }

    /// Spawns an asynchronous Labrador fetch for the selected chapter.
    /// Passes the known fetch_url if available, or lets Labrador discover it.
    pub fn download_selected_chapter(&mut self) {
        let (series_id, series_title, series_url) = match self.current_series() {
            Some(s) => (
                s.series.id,
                s.series.title.clone(),
                s.series.fetch_url.clone(),
            ),
            None => {
                self.set_toast("No series selected", true);
                return;
            }
        };

        let (chapter_number, chap_url) = match self.current_chapter() {
            Some(c) => (c.chapter.chapter_number, c.chapter.fetch_url.clone()),
            None => {
                self.set_toast("No chapter selected to download", true);
                return;
            }
        };

        // Check if already downloading
        if self.download_jobs.iter().any(|j| {
            j.series_id == series_id && (j.chapter_number - chapter_number).abs() < f64::EPSILON
        }) {
            self.set_toast(
                format!("Chapter {:.1} is already being fetched", chapter_number),
                false,
            );
            return;
        }

        let effective_url = chap_url.or(series_url);

        let task_id = self.labrador_runner.spawn_fetch(
            self.event_tx.clone(),
            series_id,
            series_title.clone(),
            chapter_number,
            effective_url.clone(),
        );

        self.download_jobs.push(DownloadJob {
            task_id,
            series_id,
            series_title: series_title.clone(),
            chapter_number,
            started_at: Instant::now(),
        });

        let msg = if effective_url.is_some() {
            format!(
                "Fetching '{}' Ch. {:.1} from URL...",
                series_title, chapter_number
            )
        } else {
            format!(
                "Fetching '{}' Ch. {:.1} with Labrador (resolving URL)...",
                series_title, chapter_number
            )
        };

        self.set_toast(msg, false);
    }

    /// Triggers download for the next un-downloaded chapter in current series
    pub fn download_next_unread_chapter(&mut self) {
        let (series_id, series_title, series_url) = match self.current_series() {
            Some(s) => (
                s.series.id,
                s.series.title.clone(),
                s.series.fetch_url.clone(),
            ),
            None => {
                self.set_toast("No series selected", true);
                return;
            }
        };

        let next_target = self.chapters_list.iter().find(|c| !c.is_downloaded());
        if let Some(target) = next_target {
            let chapter_number = target.chapter.chapter_number;
            let effective_url = target.chapter.fetch_url.clone().or(series_url);

            let task_id = self.labrador_runner.spawn_fetch(
                self.event_tx.clone(),
                series_id,
                series_title.clone(),
                chapter_number,
                effective_url,
            );

            self.download_jobs.push(DownloadJob {
                task_id,
                series_id,
                series_title: series_title.clone(),
                chapter_number,
                started_at: Instant::now(),
            });

            self.set_toast(
                format!("Fetching Next Chapter: Ch. {:.1}", chapter_number),
                false,
            );
        } else {
            self.set_toast("All chapters in this series are already downloaded", false);
        }
    }

    /// Deletes the selected series. Requires a second `x` press on the same
    /// series to confirm; any navigation clears the pending confirmation.
    pub fn request_delete_selected(&mut self) {
        let current_id = self.current_series().map(|s| s.series.id);

        // Second press on the same series -> confirm and delete.
        if self.pending_delete_id.is_some() && self.pending_delete_id == current_id {
            self.pending_delete_id = None;
            match self.db.delete_series(current_id.unwrap()) {
                Ok(()) => {
                    let _ = self.reload_series();
                    let _ = self.reload_chapters();
                    self.set_toast("Series removed from library", false);
                }
                Err(err) => self.set_toast(format!("Failed to delete series: {}", err), true),
            }
        } else {
            self.pending_delete_id = current_id;
            if current_id.is_some() {
                self.set_toast("Press x again to delete this series", false);
            }
        }
    }

    /// Toggles reading mode (Webtoon <-> Manga) for the selected series.
    pub fn toggle_reading_mode_selected(&mut self) -> Result<()> {
        if let Some(s) = self.current_series() {
            let series_id = s.series.id;
            let current_mode = s.series.reading_mode();
            let new_mode = if current_mode == "manga" {
                "webtoon"
            } else {
                "manga"
            };
            self.db.update_series_reading_mode(series_id, new_mode)?;
            self.reload_series()?;

            let display_name = match new_mode {
                "manga" => "Manga (Horizontal)",
                _ => "Webtoon (Vertical)",
            };
            self.set_toast(format!("Reading mode set to {}", display_name), false);
        } else {
            self.set_toast("No series selected", true);
        }
        Ok(())
    }

    /// Toggle chapter completed / uncompleted status for the selected chapter.
    pub fn toggle_completed_selected(&mut self) -> Result<()> {
        if let Some(chap) = self.current_chapter() {
            let chapter_id = chap.chapter.id;
            let chapter_num = chap.chapter.chapter_number;
            let is_now_completed = self.db.toggle_completed(chapter_id)?;
            self.reload_chapters()?;
            self.reload_series()?;

            let msg = if is_now_completed {
                format!("Chapter {:.1} marked completed [✓]", chapter_num)
            } else {
                format!("Chapter {:.1} marked uncompleted", chapter_num)
            };
            self.set_toast(msg, false);
        }
        Ok(())
    }

    /// Clears the selected chapter's reading progress (page 0, uncompleted).
    pub fn clear_progress_selected(&mut self) {
        if let Some(chap) = self.current_chapter() {
            let chapter_id = chap.chapter.id;
            let chapter_num = chap.chapter.chapter_number;
            match self.db.delete_progress(chapter_id) {
                Ok(()) => {
                    let _ = self.reload_chapters();
                    let _ = self.reload_series();
                    self.set_toast(
                        format!("Chapter {:.1} marked as unread", chapter_num),
                        false,
                    );
                }
                Err(err) => self.set_toast(format!("Failed to clear progress: {}", err), true),
            }
        }
    }

    pub fn on_download_started(
        &mut self,
        task_id: u64,
        series_id: i64,
        series_title: String,
        chapter_number: f64,
    ) {
        if !self.download_jobs.iter().any(|j| j.task_id == task_id) {
            self.download_jobs.push(DownloadJob {
                task_id,
                series_id,
                series_title,
                chapter_number,
                started_at: Instant::now(),
            });
        }
    }

    pub fn on_download_success(&mut self, payload: DownloadSuccessPayload) -> Result<()> {
        self.download_jobs.retain(|j| j.task_id != payload.task_id);

        let path_str = payload.file_path.to_string_lossy().to_string();
        let chapter_id = self.db.record_chapter_download(
            payload.series_id,
            payload.chapter_number,
            &path_str,
            payload.page_count,
            payload.fetch_url.as_deref(),
        )?;

        // If Labrador returned an updated chapter fetch_url, update it
        if let Some(url) = &payload.fetch_url {
            let _ = self.db.update_chapter_fetch_url(chapter_id, url);
        }

        // If Labrador returned an updated series-level fetch_url, update it
        if let Some(series_url) = &payload.series_fetch_url {
            let _ = self
                .db
                .update_series_fetch_url(payload.series_id, series_url);
        }

        self.reload_chapters()?;
        self.reload_series()?;

        let msg = match (payload.fetch_url, payload.series_fetch_url) {
            (Some(url), _) => format!(
                "Downloaded Ch. {:.1} (Saved source URL: {})",
                payload.chapter_number, url
            ),
            _ => format!(
                "Downloaded Chapter {:.1} successfully!",
                payload.chapter_number
            ),
        };

        self.set_toast(msg, false);
        Ok(())
    }

    pub fn on_download_failed(
        &mut self,
        task_id: u64,
        series_title: String,
        chapter_number: f64,
        error: String,
    ) {
        self.download_jobs.retain(|j| j.task_id != task_id);
        self.set_toast(
            format!(
                "Download failed for {} Ch. {:.1}: {}",
                series_title, chapter_number, error
            ),
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Database;
    use tokio::sync::mpsc;

    fn test_app() -> App {
        let db = Database::in_memory().unwrap();
        db.seed_sample_data_if_empty().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut cfg = Config::default();
        cfg.auto_scan_on_startup = false; // no tokio runtime in tests
        App::new(cfg, db, tx).unwrap()
    }

    #[test]
    fn tap_selects_and_double_tap_detects() {
        let mut app = test_app();
        assert!(app.series_list.len() >= 2);

        app.select_series_index(1);
        assert_eq!(app.selected_series_idx, 1);
        assert_eq!(app.active_pane, ActivePane::SeriesList);

        // Out-of-range tap is ignored.
        app.select_series_index(999);
        assert_eq!(app.selected_series_idx, 1);

        // Chapter selection works and updates the active pane.
        app.select_chapter_index(0);
        assert_eq!(app.selected_chapter_idx, 0);
        assert_eq!(app.active_pane, ActivePane::ChaptersList);

        // First tap: not a double-tap; immediate repeat on same item: double.
        assert!(!app.handle_tap(ActivePane::ChaptersList, 0));
        assert!(app.handle_tap(ActivePane::ChaptersList, 0));
        // Different item (or pane) resets the window.
        assert!(!app.handle_tap(ActivePane::ChaptersList, 1));
    }

    #[test]
    fn jump_list_top_and_bottom() {
        let mut app = test_app();
        assert!(app.series_list.len() >= 2);

        app.jump_list_bottom();
        assert_eq!(app.selected_series_idx, app.series_list.len() - 1);
        app.jump_list_top();
        assert_eq!(app.selected_series_idx, 0);

        // Same for the chapters pane.
        app.select_series_index(1);
        assert!(!app.chapters_list.is_empty());
        app.active_pane = ActivePane::ChaptersList;

        app.jump_list_bottom();
        assert_eq!(app.selected_chapter_idx, app.chapters_list.len() - 1);
        app.jump_list_top();
        assert_eq!(app.selected_chapter_idx, 0);
    }

    #[test]
    fn test_toggle_reading_mode_selected() {
        let mut app = test_app();
        assert_eq!(
            app.current_series().unwrap().series.reading_mode(),
            "webtoon"
        );

        app.toggle_reading_mode_selected().unwrap();
        assert_eq!(app.current_series().unwrap().series.reading_mode(), "manga");

        app.toggle_reading_mode_selected().unwrap();
        assert_eq!(
            app.current_series().unwrap().series.reading_mode(),
            "webtoon"
        );
    }

    #[test]
    fn test_search_query_filtering() {
        let mut app = test_app();
        let total = app.series_list.len();
        assert!(total >= 2);
        assert_eq!(app.filtered_indices.len(), total);

        // Enter search mode and type query
        app.enter_search_mode();
        assert_eq!(app.input_mode, InputMode::SearchInput);

        app.search_push_char('s');
        app.search_push_char('o');
        app.search_push_char('l');
        app.search_push_char('o');
        assert_eq!(app.search_query, "solo");
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.current_series().unwrap().series.title, "Solo Leveling");

        // Backspace
        app.search_pop_char();
        app.search_pop_char();
        app.search_pop_char();
        app.search_pop_char();
        assert_eq!(app.search_query, "");
        assert_eq!(app.filtered_indices.len(), total);

        // Exit search mode with clear
        app.search_push_char('z');
        app.search_push_char('z');
        assert_eq!(app.filtered_indices.len(), 0);
        assert!(app.current_series().is_none());

        app.exit_search_mode(true);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.search_query, "");
        assert_eq!(app.filtered_indices.len(), total);
    }

    #[test]
    fn test_filter_modes_cycle_and_clear() {
        let mut app = test_app();
        let total = app.series_list.len();

        assert_eq!(app.filter_mode, FilterMode::All);
        app.toggle_filter_mode();
        assert_eq!(app.filter_mode, FilterMode::Unread);

        app.toggle_filter_mode();
        assert_eq!(app.filter_mode, FilterMode::Ongoing);

        app.toggle_filter_mode();
        assert_eq!(app.filter_mode, FilterMode::Completed);

        app.toggle_filter_mode();
        assert_eq!(app.filter_mode, FilterMode::All);
        assert_eq!(app.filtered_indices.len(), total);

        // Test clear_search_and_filters
        app.search_push_char('t');
        app.toggle_filter_mode();
        assert!(!app.search_query.is_empty());
        assert_ne!(app.filter_mode, FilterMode::All);

        app.clear_search_and_filters();
        assert_eq!(app.search_query, "");
        assert_eq!(app.filter_mode, FilterMode::All);
        assert_eq!(app.filtered_indices.len(), total);
    }

    #[test]
    fn test_hidden_series_filter_and_toggle() {
        let mut app = test_app();
        let total = app.series_list.len();

        // Mark first series as hidden in DB
        let first_id = app.series_list[0].series.id;
        app.db.update_series_hidden(first_id, true).unwrap();
        app.reload_series().unwrap();

        // By default show_hidden is false, so 1 fewer series in filtered_indices
        assert!(!app.show_hidden);
        assert_eq!(app.filtered_indices.len(), total - 1);

        // Toggle show_hidden -> now all series are visible
        app.toggle_show_hidden();
        assert!(app.show_hidden);
        assert_eq!(app.filtered_indices.len(), total);

        // Toggle show_hidden back -> hidden series is filtered out again
        app.toggle_show_hidden();
        assert!(!app.show_hidden);
        assert_eq!(app.filtered_indices.len(), total - 1);
    }

    #[test]
    fn test_toggle_selected_series_hidden_directory_rename() {
        let temp_lib = std::env::temp_dir().join(format!("dewey_app_rename_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_lib);
        std::fs::create_dir_all(&temp_lib).unwrap();

        let series_dir = temp_lib.join("My_Manga");
        std::fs::create_dir_all(&series_dir).unwrap();
        let ch_path = series_dir.join("c001.cbz");
        std::fs::write(&ch_path, b"dummy").unwrap();

        let db = Database::in_memory().unwrap();
        let series_id = db.insert_or_get_series_with_cover_and_hidden("My Manga", None, false).unwrap();
        db.record_chapter_download(series_id, 1.0, ch_path.to_str().unwrap(), Some(10), None).unwrap();

        let (tx, _rx) = mpsc::unbounded_channel();
        let mut cfg = Config::default();
        cfg.library_dir = temp_lib.clone();
        cfg.auto_scan_on_startup = false;
        let mut app = App::new(cfg, db, tx).unwrap();
        app.show_hidden = true;
        app.reload_series().unwrap();

        assert_eq!(app.series_list.len(), 1);
        assert!(!app.series_list[0].series.is_hidden);
        assert!(series_dir.exists());

        // Toggle to hidden
        app.toggle_selected_series_hidden().unwrap();
        let hidden_dir = temp_lib.join(".My_Manga");
        assert!(hidden_dir.exists());
        assert!(!series_dir.exists());
        assert!(app.series_list[0].series.is_hidden);

        // Chapter file path should be updated in DB
        let chapters = app.db.get_chapters_for_series(series_id).unwrap();
        assert_eq!(
            chapters[0].chapter.file_path.as_deref(),
            Some(hidden_dir.join("c001.cbz").to_str().unwrap())
        );

        // Toggle back to unhidden
        app.toggle_selected_series_hidden().unwrap();
        assert!(series_dir.exists());
        assert!(!hidden_dir.exists());
        assert!(!app.series_list[0].series.is_hidden);

        let _ = std::fs::remove_dir_all(&temp_lib);
    }

    #[test]
    fn test_apply_category_moves_directory_and_updates_db() {
        let temp_lib = std::env::temp_dir().join(format!("dewey_app_cat_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_lib);
        std::fs::create_dir_all(&temp_lib).unwrap();

        let initial_series_dir = temp_lib.join("Solo_Leveling");
        std::fs::create_dir_all(&initial_series_dir).unwrap();
        let ch_path = initial_series_dir.join("c001.cbz");
        std::fs::write(&ch_path, b"dummy").unwrap();

        let db = Database::in_memory().unwrap();
        let series_id = db
            .insert_or_get_series_full("Solo Leveling", None, false, None)
            .unwrap();
        db.record_chapter_download(series_id, 1.0, ch_path.to_str().unwrap(), Some(10), None)
            .unwrap();

        let (tx, _rx) = mpsc::unbounded_channel();
        let mut cfg = Config::default();
        cfg.library_dir = temp_lib.clone();
        cfg.auto_scan_on_startup = false;
        let mut app = App::new(cfg, db, tx).unwrap();

        assert_eq!(app.series_list.len(), 1);
        assert_eq!(app.series_list[0].series.category, None);
        assert!(initial_series_dir.exists());

        // 1. Move to "Manhwa/Action"
        app.apply_category_to_selected("Manhwa/Action").unwrap();

        let target_dir = temp_lib.join("Manhwa").join("Action").join("Solo_Leveling");
        assert!(target_dir.exists());
        assert!(!initial_series_dir.exists());

        let series = app.db.get_all_series().unwrap();
        assert_eq!(series[0].series.category.as_deref(), Some("Manhwa/Action"));

        let chapters = app.db.get_chapters_for_series(series_id).unwrap();
        assert_eq!(
            chapters[0].chapter.file_path.as_deref(),
            Some(target_dir.join("c001.cbz").to_str().unwrap())
        );

        // 2. Search matches category "action"
        app.search_query = "action".to_string();
        app.apply_filter();
        assert_eq!(app.filtered_indices.len(), 1);

        app.search_query = "romance".to_string();
        app.apply_filter();
        assert_eq!(app.filtered_indices.len(), 0);

        let _ = std::fs::remove_dir_all(&temp_lib);
    }
}
