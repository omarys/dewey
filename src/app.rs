use anyhow::Result;
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

/// One-shot actions exposed as tappable buttons in the footer action bar
/// (touchscreen-friendly mirror of the keyboard shortcuts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    Open,
    Fetch,
    FetchNext,
    Scan,
    Reset,
    Delete,
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
    /// Tappable action-bar buttons rendered by the footer.
    pub action_rects: Vec<(Rect, AppAction)>,
    /// (time, pane, index) of the previous left-click, for double-tap open.
    pub last_tap: Option<(Instant, ActivePane, usize)>,
    pub event_tx: UnboundedSender<AppEvent>,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config, db: Database, event_tx: UnboundedSender<AppEvent>) -> Result<Self> {
        let continuum_runner = ContinuumRunner::new(&config.continuum_bin);
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
            active_pane: ActivePane::SeriesList,
            download_jobs: Vec::new(),
            tick_count: 0,
            is_scanning: false,
            toast: None,
            show_help_modal: false,
            pending_delete_id: None,
            series_rect: None,
            chapters_rect: None,
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
        let event_tx = self.event_tx.clone();

        tokio::task::spawn_blocking(
            move || match LibraryScanner::scan_directory(&db, &lib_dir) {
                Ok(summary) => {
                    let _ = event_tx.send(AppEvent::ScanCompleted(summary));
                }
                Err(err) => {
                    let _ = event_tx.send(AppEvent::Toast {
                        message: format!("Scan error: {}", err),
                        is_error: true,
                    });
                }
            },
        );
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
        if self.selected_series_idx >= self.series_list.len() && !self.series_list.is_empty() {
            self.selected_series_idx = self.series_list.len() - 1;
        }
        if self.series_list.is_empty() {
            self.series_state.select(None);
        } else {
            self.series_state.select(Some(self.selected_series_idx));
        }
        Ok(())
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
        self.series_list.get(self.selected_series_idx)
    }

    pub fn current_chapter(&self) -> Option<&ChapterWithProgress> {
        self.chapters_list.get(self.selected_chapter_idx)
    }

    /// Selects the series at `idx` (tap). Focuses the series pane, reloads the
    /// chapter list for it, and cancels any pending delete confirmation.
    pub fn select_series_index(&mut self, idx: usize) {
        if idx >= self.series_list.len() {
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
                if !self.series_list.is_empty() {
                    self.selected_series_idx =
                        (self.selected_series_idx + 1) % self.series_list.len();
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

    pub fn prev_item(&mut self) {
        self.pending_delete_id = None;
        match self.active_pane {
            ActivePane::SeriesList => {
                if !self.series_list.is_empty() {
                    if self.selected_series_idx == 0 {
                        self.selected_series_idx = self.series_list.len() - 1;
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

        info!(
            chapter_id,
            chapter_num,
            file = ?file_path,
            last_page,
            "Suspending TUI to spawn Continuum reader"
        );

        self.set_toast(
            format!("Reading Chapter {:.1} in Continuum...", chapter_num),
            false,
        );

        // 1. Suspend TUI raw mode and alternate screen
        tui.suspend()?;

        // 2. Launch Continuum child process & wait for stdout exit payload
        let result = self.continuum_runner.spawn_and_wait(&file_path, last_page);

        // 3. Resume TUI terminal mode
        tui.resume()?;

        // 4. Handle exit payload, update SQLite progress, and display completion message
        match result {
            Ok(payload) => {
                info!(
                    chapter_id,
                    last_page = payload.last_page,
                    completed = payload.completed,
                    "Continuum closed. Updating SQLite progress table."
                );

                self.db
                    .upsert_progress(chapter_id, payload.last_page, payload.completed)?;
                self.reload_chapters()?;
                self.reload_series()?;

                let status_msg = payload.completion_message(chapter_num);
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
}
