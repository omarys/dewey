mod app;
mod config;
mod db;
mod event;
mod runner;
mod scanner;
mod terminal;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use app::{ActivePane, App, AppAction};
use config::Config;
use db::Database;
use event::{AppEvent, EventHandler};
use runner::ContinuumRunner;
use terminal::Tui;

#[derive(Parser, Debug)]
#[command(
    name = "dewey",
    author,
    version,
    about = "TUI Library Manager for Continuum and Labrador"
)]
struct Cli {
    /// Optional comic/chapter file (.cbz, .zip) to launch directly in Continuum with progress tracking
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Path to config file (dewey.toml or ~/.config/dewey/config.toml)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Designated library directory to scan for manga/manhwa (e.g. ~/Documents/Books)
    #[arg(short, long)]
    library_dir: Option<PathBuf>,

    /// Path to SQLite database file
    #[arg(short, long)]
    db_path: Option<PathBuf>,

    /// Log file path
    #[arg(long)]
    log_file: Option<PathBuf>,

    /// Seed sample data if library and database are empty
    #[arg(long)]
    seed: Option<bool>,

    /// Reinitialize the SQLite database from scratch (deletes all data)
    #[arg(long)]
    init: bool,

    /// Storage optimization profile ('fast' for internal NVMe/SSD, 'usb' for flash thumb drives / slow media)
    #[arg(long, value_name = "PROFILE")]
    storage_profile: Option<String>,

    /// Quick flag to enable USB/slow-storage optimization profile
    #[arg(short = 'u', long)]
    usb: bool,

    /// Override max concurrent worker threads for library scanning
    #[arg(long, value_name = "N")]
    scan_concurrency: Option<usize>,
}

fn init_logging(log_path: &Path) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_appender = tracing_appender::rolling::never(
        log_path.parent().unwrap_or_else(|| Path::new(".")),
        log_path.file_name().unwrap_or_default(),
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false),
        )
        .init();

    Ok(guard)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // 1. Load config file and apply CLI overrides
    let mut config = Config::load_or_create(cli.config.as_deref())?;

    // A directory positional argument (e.g. `dewey .`) is a library directory,
    // not a direct-launch file. It takes effect unless -l/--library-dir is given.
    let positional_dir = cli.file.as_deref().filter(|p| p.is_dir());
    let explicit_library = cli
        .library_dir
        .clone()
        .or_else(|| positional_dir.map(Path::to_path_buf));

    // Per-directory DB: when a library dir is explicitly set (-l or `.`), keep
    // the database alongside that dir (one DB per library) unless --db-path is
    // given explicitly.
    if let Some(dir) = &explicit_library {
        config.library_dir = dir.clone();
        if cli.db_path.is_none() {
            config.db_path = dir.join(".dewey.db");
        }
    }
    if let Some(db) = cli.db_path {
        config.db_path = db;
    }
    if let Some(log) = cli.log_file {
        config.log_file = log;
    }
    if let Some(seed) = cli.seed {
        config.seed_sample_data = seed;
    }
    if cli.usb {
        config.storage_profile = config::StorageProfile::Usb;
    } else if let Some(prof_str) = &cli.storage_profile {
        config.storage_profile = config::StorageProfile::from_str_loose(prof_str);
    } else if config.storage_profile == config::StorageProfile::Fast {
        // Auto-detect storage medium from library location
        config.storage_profile = Config::auto_detect_storage_profile(&config.library_dir);
    }
    if let Some(concurrency) = cli.scan_concurrency {
        config.max_scan_concurrency = Some(concurrency);
    }

    let _guard = init_logging(&config.log_file).context("Failed to initialize file logger")?;

    info!("Starting Dewey TUI Library Manager");
    info!(
        db = ?config.db_path,
        library = ?config.library_dir,
        profile = config.storage_profile.as_str(),
        "Connecting to SQLite database"
    );

    // Start over: wipe the database (and WAL/SHM) so the next open recreates it fresh.
    if cli.init {
        Database::reset(&config.db_path)?;
        println!(
            "Database reset at {:?}. A fresh database will be created on next launch.",
            config.db_path
        );
        return Ok(());
    }

    // Refuse to initialize a brand-new library that contains no comic
    // archives anywhere — there is nothing to track yet.
    if let (Some(dir), false) = (&explicit_library, config.db_path.exists()) {
        if !crate::scanner::LibraryScanner::has_chapter_files(dir) {
            println!(
                "No .cbz files found in {:?} — skipping library initialization.",
                dir
            );
            return Ok(());
        }
    }

    let db = Database::open_with_profile(&config.db_path, config.storage_profile)?;
    if config.seed_sample_data {
        db.seed_sample_data_if_empty()?;
    }

    // Direct file launch mode (e.g. `dewey path/to/chapter.cbz`).
    // A directory argument is a library directory, not a file to launch.
    if let Some(target_file) = cli.file.as_ref().filter(|p| !p.is_dir()) {
        let abs_path = std::fs::canonicalize(target_file).unwrap_or(target_file.clone());
        let chapter_id = db.get_or_create_chapter_for_file(&abs_path)?;

        let (series_id, last_page, chapter_num, series_mode) = {
            let conn = db.get_progress_for_file(&abs_path)?;
            let p = conn
                .as_ref()
                .map(|(_, prog)| prog.last_page_read)
                .unwrap_or(0);
            let num =
                crate::scanner::LibraryScanner::parse_chapter_number(&abs_path).unwrap_or(1.0);
            let sid = db.get_series_id_for_chapter(chapter_id).ok().flatten();
            let mode = db
                .get_series_reading_mode_for_chapter(chapter_id)
                .unwrap_or_else(|_| "webtoon".to_string());
            (sid, p, num, mode)
        };

        println!(
            "📖 Opening {:?} in Continuum ({} mode, {} profile) at page {}...",
            abs_path.file_name().unwrap_or_default(),
            series_mode,
            config.storage_profile.as_str(),
            last_page
        );

        let runner = ContinuumRunner::new(&config.continuum_bin)
            .with_storage_profile(config.storage_profile.as_str());
        let result = runner.spawn_and_wait(&abs_path, last_page, Some(&series_mode))?;

        if let Some(new_mode) = &result.mode {
            if let Some(sid) = series_id {
                let _ = db.update_series_reading_mode(sid, new_mode);
            }
        }

        // Persist progress for every chapter the reader actually touched;
        // fall back to the legacy single-chapter contract when absent.
        let updated = match result.chapters.as_ref().filter(|c| !c.is_empty()) {
            Some(chapters) => {
                let mut n = 0usize;
                for entry in chapters {
                    db.apply_chapter_progress(
                        Path::new(&entry.file),
                        entry.last_page,
                        entry.completed,
                    )?;
                    n += 1;
                }
                n
            }
            None => {
                db.upsert_progress(chapter_id, result.last_page, result.completed)?;
                1
            }
        };

        let mut msg = result.completion_message(chapter_num);
        if updated > 1 {
            msg = format!("{} · {} chapters updated", msg, updated);
        }
        println!("{}", msg);
        return Ok(());
    }

    // 2. Set up TUI and Event loop
    let mut tui = Tui::new()?;
    tui.init()?;

    let (mut event_handler, event_tx) = EventHandler::new(Duration::from_millis(80));
    let mut app = App::new(config, db, event_tx)?;

    // 3. Main Ratatui event loop
    while !app.should_quit {
        // Render UI frame with stateful widget support
        tui.terminal_mut().draw(|f| ui::render(f, &mut app))?;

        // Await next event from input stream or background async tasks
        if let Some(event) = event_handler.next().await {
            match event {
                AppEvent::Tick => {
                    app.tick_count = app.tick_count.wrapping_add(1);
                    app.check_toast_expiry();
                }

                AppEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.show_help_modal {
                        if key.code == KeyCode::Esc
                            || key.code == KeyCode::Char('?')
                            || key.code == KeyCode::Char('q')
                        {
                            app.show_help_modal = false;
                        }
                        continue;
                    }

                    if app.input_mode == app::InputMode::SearchInput {
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                app.should_quit = true;
                            }
                            (KeyCode::Esc, _) => {
                                app.exit_search_mode(true);
                            }
                            (KeyCode::Enter, _)
                            | (KeyCode::Down, _)
                            | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                                app.exit_search_mode(false);
                            }
                            (KeyCode::Backspace, _) => {
                                app.search_pop_char();
                            }
                            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                                app.search_push_char(c);
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if app.input_mode == app::InputMode::CategoryPicker {
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                app.should_quit = true;
                            }
                            (KeyCode::Esc, _) => {
                                app.close_category_modal();
                            }
                            (KeyCode::Enter, _) => {
                                if let Err(err) = app.confirm_category_selection() {
                                    app.set_toast(format!("Failed to move series: {}", err), true);
                                }
                            }
                            (KeyCode::Down, _)
                            | (KeyCode::Tab, _)
                            | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                                app.category_modal_select_next();
                            }
                            (KeyCode::Up, _)
                            | (KeyCode::BackTab, _)
                            | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                                app.category_modal_select_prev();
                            }
                            (KeyCode::Backspace, _) => {
                                app.category_modal_pop_char();
                            }
                            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                                app.category_modal_push_char(c);
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if app.input_mode == app::InputMode::EditSeries {
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                app.should_quit = true;
                            }
                            (KeyCode::Esc, _) => {
                                app.close_edit_series_modal();
                            }
                            (KeyCode::Enter, _) => {
                                if let Err(err) = app.save_edit_series_modal() {
                                    app.set_toast(format!("Failed to save series: {}", err), true);
                                }
                            }
                            (KeyCode::Tab, _) | (KeyCode::Down, _) => {
                                app.edit_series_next_field();
                            }
                            (KeyCode::BackTab, _) | (KeyCode::Up, _) => {
                                app.edit_series_prev_field();
                            }
                            (KeyCode::Left, _) => {
                                app.edit_series_cycle_left();
                            }
                            (KeyCode::Right, _) => {
                                app.edit_series_cycle_right();
                            }
                            (KeyCode::Char(' '), KeyModifiers::NONE) => {
                                app.edit_series_toggle_active_option();
                            }
                            (KeyCode::Backspace, _) => {
                                app.edit_series_pop_char();
                            }
                            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                                app.edit_series_push_char(c);
                            }
                            _ => {}
                        }
                        continue;
                    }

                    match (key.code, key.modifiers) {
                        (KeyCode::Char('c'), KeyModifiers::CONTROL)
                        | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                            app.should_quit = true;
                        }
                        (KeyCode::Char('.'), KeyModifiers::NONE) => {
                            app.toggle_show_hidden();
                        }
                        (KeyCode::Char('t'), KeyModifiers::NONE) => {
                            app.open_category_modal();
                        }
                        (KeyCode::Char('?'), _) => {
                            app.show_help_modal = true;
                        }
                        (KeyCode::Char('/'), KeyModifiers::NONE) => {
                            app.enter_search_mode();
                        }
                        (KeyCode::Char('f'), KeyModifiers::NONE) => {
                            app.toggle_filter_mode();
                        }
                        (KeyCode::Char('T'), _) => {
                            app.cycle_type_filter();
                        }
                        (KeyCode::Char('H'), _) => {
                            if let Err(err) = app.toggle_selected_series_hidden() {
                                app.set_toast(
                                    format!("Failed to toggle hidden tag: {}", err),
                                    true,
                                );
                            }
                        }
                        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                            app.next_item();
                        }
                        (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                            app.prev_item();
                        }
                        (KeyCode::Char('g'), KeyModifiers::NONE) => {
                            app.jump_list_top();
                        }
                        (KeyCode::Char('G'), _) => {
                            app.jump_list_bottom();
                        }
                        (KeyCode::Tab, _)
                        | (KeyCode::Right, _)
                        | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                            app.switch_pane_forward();
                        }
                        (KeyCode::BackTab, _)
                        | (KeyCode::Left, _)
                        | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                            app.switch_pane_backward();
                        }
                        (KeyCode::Enter, _) => {
                            if let Err(err) = app.handle_enter_action(&mut tui, &mut event_handler)
                            {
                                app.set_toast(format!("Action failed: {}", err), true);
                            }
                        }
                        (KeyCode::Char('d'), KeyModifiers::NONE) => {
                            if let Err(err) =
                                app.download_selected_chapter(&mut tui, &mut event_handler)
                            {
                                app.set_toast(format!("Fetch failed: {}", err), true);
                            }
                        }
                        (KeyCode::Char('D'), _) => {
                            if let Err(err) =
                                app.open_series_in_labrador(&mut tui, &mut event_handler)
                            {
                                app.set_toast(format!("Labrador error: {}", err), true);
                            }
                        }
                        (KeyCode::Char('s'), KeyModifiers::NONE) => {
                            app.set_toast("Scanning library in background...", false);
                            app.spawn_background_scan();
                        }
                        (KeyCode::Char('m'), KeyModifiers::NONE) => {
                            if let Err(err) = app.toggle_completed_selected() {
                                app.set_toast(format!("Failed to toggle status: {}", err), true);
                            }
                        }
                        (KeyCode::Char('M'), _) | (KeyCode::Char('v'), KeyModifiers::NONE) => {
                            if let Err(err) = app.toggle_reading_mode_selected() {
                                app.set_toast(
                                    format!("Failed to toggle reading mode: {}", err),
                                    true,
                                );
                            }
                        }
                        (KeyCode::Char('u'), KeyModifiers::NONE) => {
                            app.clear_progress_selected();
                        }
                        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                            app.page_down(8);
                        }
                        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                            app.page_up(8);
                        }
                        (KeyCode::Char('f'), KeyModifiers::CONTROL) | (KeyCode::PageDown, _) => {
                            app.page_down(15);
                        }
                        (KeyCode::Char('b'), KeyModifiers::CONTROL) | (KeyCode::PageUp, _) => {
                            app.page_up(15);
                        }
                        (KeyCode::Char('x'), KeyModifiers::NONE) => {
                            app.request_delete_selected();
                        }
                        (KeyCode::Delete, _) => {
                            app.request_delete_chapter();
                        }
                        (KeyCode::Esc, _) => {
                            app.clear_pending_deletes();
                            if !app.search_query.is_empty()
                                || app.filter_mode != app::FilterMode::All
                            {
                                app.clear_search_and_filters();
                            } else if app.active_pane == app::ActivePane::ChaptersList {
                                app.active_pane = app::ActivePane::SeriesList;
                            }
                        }
                        (KeyCode::Char('a'), KeyModifiers::NONE) => {
                            if let Err(err) = app.add_new_series(&mut tui, &mut event_handler) {
                                app.set_toast(format!("Add series failed: {}", err), true);
                            }
                        }
                        (KeyCode::Char('e'), KeyModifiers::NONE) => {
                            app.open_edit_series_modal();
                        }
                        (KeyCode::Char('E'), _) => {
                            if let Err(err) = app.toggle_series_status_selected() {
                                app.set_toast(format!("Failed to toggle status: {}", err), true);
                            }
                        }
                        (KeyCode::Char('r'), KeyModifiers::NONE) => {
                            let _ = app.reload_series();
                            let _ = app.reload_chapters();
                            app.set_toast("Refreshed library from database", false);
                        }
                        _ => {}
                    }
                }

                AppEvent::ScanCompleted(summary) => {
                    app.on_scan_completed(summary);
                }

                AppEvent::DownloadStarted {
                    task_id,
                    series_id,
                    series_title,
                    chapter_number,
                } => {
                    app.on_download_started(task_id, series_id, series_title, chapter_number);
                }

                AppEvent::DownloadSuccess(payload) => {
                    if let Err(err) = app.on_download_success(payload) {
                        app.set_toast(format!("Failed to record download: {}", err), true);
                    }
                }

                AppEvent::DownloadFailed {
                    task_id,
                    series_title,
                    chapter_number,
                    error,
                    ..
                } => {
                    app.on_download_failed(task_id, series_title, chapter_number, error);
                }

                AppEvent::Toast { message, is_error } => {
                    app.set_toast(message, is_error);
                }

                // Touchscreen / mouse input: tap selects, double-tap opens,
                // wheel scrolls the pane under the cursor.
                AppEvent::Mouse(mouse) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    let x = mouse.column;
                    let y = mouse.row;

                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            // 0. If Help Modal is open, tap dismisses modal and prevents click-through
                            if app.show_help_modal {
                                app.show_help_modal = false;
                                continue;
                            }

                            // 0b. If Edit Series modal is open, route clicks to its buttons
                            if app.input_mode == app::InputMode::EditSeries {
                                if let Some(r) = app.edit_save_rect {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        if let Err(err) = app.save_edit_series_modal() {
                                            app.set_toast(
                                                format!("Failed to save series: {}", err),
                                                true,
                                            );
                                        }
                                        continue;
                                    }
                                }
                                if let Some(r) = app.edit_cancel_rect {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        app.close_edit_series_modal();
                                        continue;
                                    }
                                }
                                for (r, idx) in &app.edit_status_rects {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        app.edit_status_idx = *idx;
                                        app.edit_field_idx = 0;
                                        break;
                                    }
                                }
                                for (r, idx) in &app.edit_mode_rects {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        app.edit_reading_mode_idx = *idx;
                                        app.edit_field_idx = 2;
                                        break;
                                    }
                                }
                                if let Some(r) = app.edit_title_rect {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        app.edit_field_idx = 1;
                                        continue;
                                    }
                                }
                                if let Some(r) = app.edit_category_rect {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        app.edit_field_idx = 3;
                                        continue;
                                    }
                                }
                                if let Some(r) = app.edit_fetch_url_rect {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        app.edit_field_idx = 4;
                                        continue;
                                    }
                                }
                                if let Some(modal_rect) = app.edit_modal_rect {
                                    if x < modal_rect.x
                                        || x >= modal_rect.x + modal_rect.width
                                        || y < modal_rect.y
                                        || y >= modal_rect.y + modal_rect.height
                                    {
                                        app.close_edit_series_modal();
                                        continue;
                                    }
                                }
                                continue;
                            }

                            // 1. If Category/Tag Picker Modal is open, handle modal touch targets
                            if app.input_mode == app::InputMode::CategoryPicker {
                                // 1a. Check Confirm button
                                if let Some(r) = app.category_confirm_rect {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        if let Err(err) = app.confirm_category_selection() {
                                            app.set_toast(
                                                format!("Failed to move series: {}", err),
                                                true,
                                            );
                                        }
                                        continue;
                                    }
                                }
                                // 1b. Check Cancel button
                                if let Some(r) = app.category_cancel_rect {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        app.close_category_modal();
                                        continue;
                                    }
                                }
                                // 1c. Check Clear button
                                if let Some(r) = app.category_clear_rect {
                                    if x >= r.x
                                        && x < r.x + r.width
                                        && y >= r.y
                                        && y < r.y + r.height
                                    {
                                        app.category_input.clear();
                                        continue;
                                    }
                                }
                                // 1d. Check Category List items
                                if let Some(rect) = app.category_list_rect {
                                    if x >= rect.x
                                        && x < rect.x + rect.width
                                        && y > rect.y
                                        && y < rect.y + rect.height.saturating_sub(1)
                                    {
                                        let visible_row = (y - (rect.y + 1)) as usize;
                                        let target_idx = app.category_state.offset() + visible_row;
                                        if target_idx < app.available_categories.len() {
                                            app.select_category_index(target_idx);
                                            if app.handle_category_tap(target_idx) {
                                                if let Err(err) = app.confirm_category_selection() {
                                                    app.set_toast(
                                                        format!("Failed to move series: {}", err),
                                                        true,
                                                    );
                                                }
                                            }
                                        }
                                        continue;
                                    }
                                }
                                // 1e. If tapped outside modal bounding box, dismiss modal
                                if let Some(modal_rect) = app.category_modal_rect {
                                    if x < modal_rect.x
                                        || x >= modal_rect.x + modal_rect.width
                                        || y < modal_rect.y
                                        || y >= modal_rect.y + modal_rect.height
                                    {
                                        app.close_category_modal();
                                        continue;
                                    }
                                }
                                // 1f. Tapped somewhere inside modal: consume event (prevent click-through)
                                continue;
                            }

                            // 2. Check navigation tabs (in portrait mode)
                            if let Some((_, pane)) = app.tab_rects.iter().find(|(r, _)| {
                                x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                            }) {
                                app.active_pane = *pane;
                                continue;
                            }

                            // 2. Action-bar buttons take precedence over list taps.
                            if let Some((_, action)) = app.action_rects.iter().find(|(r, _)| {
                                x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                            }) {
                                match action {
                                    AppAction::Open => {
                                        if let Err(err) =
                                            app.handle_enter_action(&mut tui, &mut event_handler)
                                        {
                                            app.set_toast(format!("Action failed: {}", err), true);
                                        }
                                    }
                                    AppAction::Fetch => {
                                        if let Err(err) = app
                                            .download_selected_chapter(&mut tui, &mut event_handler)
                                        {
                                            app.set_toast(format!("Fetch failed: {}", err), true);
                                        }
                                    }
                                    AppAction::FetchNext => {
                                        if let Err(err) = app
                                            .open_series_in_labrador(&mut tui, &mut event_handler)
                                        {
                                            app.set_toast(format!("Labrador error: {}", err), true);
                                        }
                                    }
                                    AppAction::Search => {
                                        app.enter_search_mode();
                                    }
                                    AppAction::Filter => {
                                        app.toggle_filter_mode();
                                    }
                                    AppAction::CycleType => {
                                        app.cycle_type_filter();
                                    }
                                    AppAction::CycleHidden => {
                                        app.cycle_hidden_filter();
                                    }
                                    AppAction::TagCategory => {
                                        app.open_category_modal();
                                    }
                                    AppAction::ToggleHidden => {
                                        if let Err(err) = app.toggle_selected_series_hidden() {
                                            app.set_toast(
                                                format!("Failed to toggle hidden tag: {}", err),
                                                true,
                                            );
                                        }
                                    }
                                    AppAction::Mode => {
                                        if let Err(err) = app.toggle_reading_mode_selected() {
                                            app.set_toast(
                                                format!("Failed to toggle reading mode: {}", err),
                                                true,
                                            );
                                        }
                                    }
                                    AppAction::MarkRead => {
                                        if let Err(err) = app.toggle_completed_selected() {
                                            app.set_toast(
                                                format!("Failed to toggle status: {}", err),
                                                true,
                                            );
                                        }
                                    }
                                    AppAction::Scan => {
                                        app.set_toast("Scanning library in background...", false);
                                        app.spawn_background_scan();
                                    }
                                    AppAction::Reset => {
                                        app.clear_progress_selected();
                                    }
                                    AppAction::Delete => {
                                        match app.active_pane {
                                            app::ActivePane::ChaptersList => {
                                                app.request_delete_chapter();
                                            }
                                            _ => {
                                                app.request_delete_selected();
                                            }
                                        }
                                    }
                                    AppAction::SwitchPane => {
                                        app.switch_pane_forward();
                                    }
                                    AppAction::AddSeries => {
                                        if let Err(err) =
                                            app.add_new_series(&mut tui, &mut event_handler)
                                        {
                                            app.set_toast(
                                                format!("Add series failed: {}", err),
                                                true,
                                            );
                                        }
                                    }
                                    AppAction::EditSeries => {
                                        app.open_edit_series_modal();
                                    }
                                    AppAction::Help => {
                                        app.show_help_modal = true;
                                    }
                                    AppAction::Quit => {
                                        app.should_quit = true;
                                    }
                                }
                                continue;
                            }

                            // 3. Exact hit-test for Series list (accounting for top border and list offset)
                            if let Some(rect) = app.series_rect {
                                if x >= rect.x && x < rect.x + rect.width {
                                    if y > rect.y && y < rect.y + rect.height.saturating_sub(1) {
                                        let visible_row = (y - (rect.y + 1)) as usize;
                                        let target_idx = app.series_state.offset() + visible_row;
                                        if target_idx < app.series_list.len() {
                                            app.select_series_index(target_idx);
                                            if app.handle_tap(ActivePane::SeriesList, target_idx) {
                                                if let Err(err) = app.handle_enter_action(
                                                    &mut tui,
                                                    &mut event_handler,
                                                ) {
                                                    app.set_toast(
                                                        format!("Action failed: {}", err),
                                                        true,
                                                    );
                                                }
                                            }
                                        }
                                        continue;
                                    } else if y == rect.y {
                                        app.active_pane = ActivePane::SeriesList;
                                    }
                                }
                            }

                            // 4. Exact hit-test for Chapters table (accounting for border, table header, and scroll offset)
                            if let Some(rect) = app.chapters_rect {
                                if x >= rect.x && x < rect.x + rect.width {
                                    if y >= rect.y + 2 && y < rect.y + rect.height.saturating_sub(1)
                                    {
                                        let visible_row = (y - (rect.y + 2)) as usize;
                                        let target_idx = app.chapters_state.offset() + visible_row;
                                        if target_idx < app.chapters_list.len() {
                                            app.select_chapter_index(target_idx);
                                            if app.handle_tap(ActivePane::ChaptersList, target_idx)
                                            {
                                                if let Err(err) = app.handle_enter_action(
                                                    &mut tui,
                                                    &mut event_handler,
                                                ) {
                                                    app.set_toast(
                                                        format!("Action failed: {}", err),
                                                        true,
                                                    );
                                                }
                                            }
                                        }
                                        continue;
                                    } else if y == rect.y || y == rect.y + 1 {
                                        app.active_pane = ActivePane::ChaptersList;
                                    }
                                }
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if app.input_mode == app::InputMode::CategoryPicker {
                                app.category_modal_select_next();
                                continue;
                            }
                            if let Some(rect) = app.series_rect {
                                if x >= rect.x
                                    && x < rect.x + rect.width
                                    && y >= rect.y
                                    && y < rect.y + rect.height
                                    && !app.series_list.is_empty()
                                {
                                    let n = app.series_list.len();
                                    app.select_series_index(
                                        (app.selected_series_idx + 1).min(n - 1),
                                    );
                                }
                            }
                            if let Some(rect) = app.chapters_rect {
                                if x >= rect.x
                                    && x < rect.x + rect.width
                                    && y >= rect.y
                                    && y < rect.y + rect.height
                                    && !app.chapters_list.is_empty()
                                {
                                    let n = app.chapters_list.len();
                                    app.select_chapter_index(
                                        (app.selected_chapter_idx + 1).min(n - 1),
                                    );
                                }
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            if app.input_mode == app::InputMode::CategoryPicker {
                                app.category_modal_select_prev();
                                continue;
                            }
                            if let Some(rect) = app.series_rect {
                                if x >= rect.x
                                    && x < rect.x + rect.width
                                    && y >= rect.y
                                    && y < rect.y + rect.height
                                    && !app.series_list.is_empty()
                                {
                                    app.select_series_index(
                                        app.selected_series_idx.saturating_sub(1),
                                    );
                                }
                            }
                            if let Some(rect) = app.chapters_rect {
                                if x >= rect.x
                                    && x < rect.x + rect.width
                                    && y >= rect.y
                                    && y < rect.y + rect.height
                                    && !app.chapters_list.is_empty()
                                {
                                    app.select_chapter_index(
                                        app.selected_chapter_idx.saturating_sub(1),
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                }

                AppEvent::Quit => {
                    app.should_quit = true;
                }

                _ => {}
            }
        }
    }

    tui.restore()?;
    info!("Dewey exited cleanly");
    Ok(())
}
