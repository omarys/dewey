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
}

fn init_logging(log_path: &Path) -> Result<tracing_appender::non_blocking::WorkerGuard> {
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

    let _guard = init_logging(&config.log_file).context("Failed to initialize file logger")?;

    info!("Starting Dewey TUI Library Manager");
    info!(
        db = ?config.db_path,
        library = ?config.library_dir,
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

    let db = Database::open(&config.db_path)?;
    if config.seed_sample_data {
        db.seed_sample_data_if_empty()?;
    }

    // Direct file launch mode (e.g. `dewey path/to/chapter.cbz`).
    // A directory argument is a library directory, not a file to launch.
    if let Some(target_file) = cli.file.as_ref().filter(|p| !p.is_dir()) {
        let abs_path = std::fs::canonicalize(target_file).unwrap_or(target_file.clone());
        let chapter_id = db.get_or_create_chapter_for_file(&abs_path)?;

        let (last_page, chapter_num) = {
            let conn = db.get_progress_for_file(&abs_path)?;
            let p = conn
                .as_ref()
                .map(|(_, prog)| prog.last_page_read)
                .unwrap_or(0);
            let num =
                crate::scanner::LibraryScanner::parse_chapter_number(&abs_path).unwrap_or(1.0);
            (p, num)
        };

        println!(
            "📖 Opening {:?} in Continuum at page {}...",
            abs_path.file_name().unwrap_or_default(),
            last_page
        );

        let runner = ContinuumRunner::new(&config.continuum_bin);
        let result = runner.spawn_and_wait(&abs_path, last_page)?;

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

                    match (key.code, key.modifiers) {
                        (KeyCode::Char('c'), KeyModifiers::CONTROL)
                        | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                            app.should_quit = true;
                        }
                        (KeyCode::Char('?'), _) => {
                            app.show_help_modal = true;
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
                            if let Err(err) = app.handle_enter_action(&mut tui) {
                                app.set_toast(format!("Action failed: {}", err), true);
                            }
                        }
                        (KeyCode::Char('d'), KeyModifiers::NONE) => {
                            app.download_selected_chapter();
                        }
                        (KeyCode::Char('D'), _) => {
                            app.download_next_unread_chapter();
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
                        (KeyCode::Char('u'), KeyModifiers::NONE) => {
                            app.clear_progress_selected();
                        }
                        (KeyCode::Char('x'), KeyModifiers::NONE) => {
                            app.request_delete_selected();
                        }
                        (KeyCode::Esc, _) => {
                            app.pending_delete_id = None;
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

                    let hit_series = app.series_rect.is_some_and(|r| {
                        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                    });
                    let hit_chapters = app.chapters_rect.is_some_and(|r| {
                        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                    });

                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            // Action-bar buttons take precedence over list taps.
                            if let Some((_, action)) = app.action_rects.iter().find(|(r, _)| {
                                x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                            }) {
                                match action {
                                    AppAction::Open => {
                                        if let Err(err) = app.handle_enter_action(&mut tui) {
                                            app.set_toast(format!("Action failed: {}", err), true);
                                        }
                                    }
                                    AppAction::Fetch => {
                                        app.download_selected_chapter();
                                    }
                                    AppAction::FetchNext => {
                                        app.download_next_unread_chapter();
                                    }
                                    AppAction::Scan => {
                                        app.set_toast("Scanning library in background...", false);
                                        app.spawn_background_scan();
                                    }
                                    AppAction::Reset => {
                                        app.clear_progress_selected();
                                    }
                                    AppAction::Delete => {
                                        app.request_delete_selected();
                                    }
                                    AppAction::Quit => {
                                        app.should_quit = true;
                                    }
                                }
                            } else if hit_series {
                                if let Some(rect) = app.series_rect {
                                    let idx = y.saturating_sub(rect.y) as usize;
                                    if idx < app.series_list.len() {
                                        app.select_series_index(idx);
                                        if app.handle_tap(ActivePane::SeriesList, idx) {
                                            if let Err(err) = app.handle_enter_action(&mut tui) {
                                                app.set_toast(
                                                    format!("Action failed: {}", err),
                                                    true,
                                                );
                                            }
                                        }
                                    }
                                }
                            } else if hit_chapters {
                                if let Some(rect) = app.chapters_rect {
                                    let idx = y.saturating_sub(rect.y) as usize;
                                    if idx < app.chapters_list.len() {
                                        app.select_chapter_index(idx);
                                        if app.handle_tap(ActivePane::ChaptersList, idx) {
                                            if let Err(err) = app.handle_enter_action(&mut tui) {
                                                app.set_toast(
                                                    format!("Action failed: {}", err),
                                                    true,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if hit_series && !app.series_list.is_empty() {
                                let n = app.series_list.len();
                                app.select_series_index((app.selected_series_idx + 1) % n);
                            } else if hit_chapters && !app.chapters_list.is_empty() {
                                let n = app.chapters_list.len();
                                app.select_chapter_index((app.selected_chapter_idx + 1) % n);
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            if hit_series && !app.series_list.is_empty() {
                                let n = app.series_list.len();
                                app.select_series_index((app.selected_series_idx + n - 1) % n);
                            } else if hit_chapters && !app.chapters_list.is_empty() {
                                let n = app.chapters_list.len();
                                app.select_chapter_index((app.selected_chapter_idx + n - 1) % n);
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
