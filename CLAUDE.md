# Dewey

<!-- BEGIN bigpowers:project -->
**Dewey** is a modern, keyboard-driven TUI manga, manhwa, and comic library manager written in Rust. Orchestration brain between your local library, reading tools, and download utilities.

## Stack

- **Language:** Rust 1.85+ (edition 2021)
- **TUI:** ratatui 0.29 + crossterm 0.28
- **Database:** SQLite via rusqlite 0.32 (bundled, WAL mode)
- **Async:** tokio (rt-multi-thread, sync, time, process)
- **Config:** TOML via toml 0.8
- **Serialization:** serde + serde_json

## Commands

| Action | Command |
|--------|---------|
| Run | `cargo run` |
| Test | `cargo test --release` |
| Build | `cargo build --release` |
| Lint | `cargo clippy --release -- -D warnings` |
| Format | `cargo fmt` |
| **Preflight** | `cargo test --release && cargo clippy --release -- -D warnings` |

## Architecture

TUI (ratatui + crossterm) → event loop (app.rs) → SQLite database (db/) + background scanner (scanner.rs, parallel threads) + companion tools (Continuum reader, Labrador downloader) spawned as child processes.

Key modules:
- `src/app.rs` — application state, selection, keyboard/mouse handling
- `src/db/` — SQLite schema, models, progress tracking (bounds-clamped)
- `src/scanner.rs` — parallel library scanner (series/chapter detection, ZIP page counts)
- `src/runner/` — ContinuumReader and LabradorRunner (child-process management)
- `src/ui/` — ratatui widgets (series list, chapters table, action bar, details pane)
- `src/event.rs` — crossterm key/mouse event stream
- `src/config.rs` — TOML config with tilde expansion and defaults
- `src/terminal.rs` — raw mode, alternate screen, suspend/resume

Companion repos: [continuum](https://github.com/omarys/continuum) (GTK4 reader), labrador (scraper).
<!-- END bigpowers:project -->

## Conventions

- Rust idioms: snake_case, Result-based error handling (anyhow), prefer existing patterns
- DB operations use `Arc<Mutex<Connection>>` — keep critical sections short
- Chapter progress writes are ALWAYS clamped to page_count in `upsert_progress`
- Destructive actions (delete series, clear progress) require confirmation
- Tests: unit tests in each module's `#[cfg(test)] mod tests`; DB tests use `Database::in_memory()`
- Background work: use `tokio::task::spawn_blocking` for I/O-heavy operations

## Specs

All planning documents live in `specs/`. See `specs/README.md`.
