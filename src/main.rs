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

use app::App;
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

    if let Some(dir) = cli.library_dir {
        config.library_dir = dir;
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

    let db = Database::open(&config.db_path)?;
    if config.seed_sample_data {
        db.seed_sample_data_if_empty()?;
    }

    // Direct file launch mode (e.g. `dewey path/to/chapter.cbz`)
    if let Some(target_file) = cli.file {
        let abs_path = std::fs::canonicalize(&target_file).unwrap_or(target_file);
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

        db.upsert_progress(chapter_id, result.last_page, result.completed)?;
        println!("{}", result.completion_message(chapter_num));
        return Ok(());
    }

    // 2. Set up TUI and Event loop
    let mut tui = Tui::new()?;
    tui.init()?;

    let (mut event_handler, event_tx) = EventHandler::new(Duration::from_millis(80));
    let mut app = App::new(config, db, event_tx)?;

    // 3. Main Ratatui event loop
    while !app.should_quit {
        // Render UI frame
        tui.terminal_mut().draw(|f| ui::render(f, &app))?;

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
                            if let Err(err) = app.scan_library() {
                                app.set_toast(format!("Scan failed: {}", err), true);
                            }
                        }
                        (KeyCode::Char('m'), KeyModifiers::NONE) => {
                            if let Err(err) = app.toggle_completed_selected() {
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
