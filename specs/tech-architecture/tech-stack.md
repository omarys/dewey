# Dewey — Tech Stack & Architecture

*Derived by `map-codebase` from a cold scan of the codebase (4494 LOC, 15 source files).*

## Stack

- **Language:** Rust 1.85+ (edition 2021)
- **TUI framework:** ratatui 0.29 (`all-widgets`), crossterm 0.28 (`event-stream`), unicode-width 0.2
- **Async runtime:** tokio 1.43 (`rt-multi-thread`, `macros`, `sync`, `time`, `process`), futures 0.3
- **Database:** rusqlite 0.32 (`bundled`, `chrono`, `serde_json`) — SQLite in WAL mode with `PRAGMA foreign_keys = ON`, `synchronous = NORMAL`, `cache_size = -64000` (64 MB), `mmap_size = 268435456` (256 MB cap)
- **Serialization:** serde 1.0 (`derive`), serde_json 1.0, toml 0.8, chrono 0.4 (`serde`)
- **CLI:** clap 4.5 (`derive`)
- **Utilities:** regex 1.11, zip 2.2 (`deflate` only, `default-features = false`), anyhow 1.0, thiserror 2.0
- **Logging:** tracing 0.1, tracing-subscriber 0.3 (`env-filter`), tracing-appender 0.2 — **file-based** (avoids corrupting TUI stdout)

## Architecture

### Entry point

`#[tokio::main] async fn main()` in `src/main.rs` — clap CLI parsing → Config load (TOML with tilde expansion) → Database open → branch:
- **Direct-file launch:** `dewey /path/chapter.cbz` — spawns Continuum, parses JSON payload, persists progress, exits (no TUI).
- **TUI mode:** enters ratatui event loop (alternate screen, raw mode, mouse capture).

### Data flow

```
CLI args → Config (TOML) → Database::open (SQLite WAL + schema + repair)
  → App::new (loads series/chapters from DB, spawns background scan)
  → TUI event loop:
      crossterm EventStream → tokio mpsc → AppEvent
      → App handles key/mouse → DB writes / child-process spawns
      → ratatui renders frame (series list, chapters table, action bar, details, downloads bar)
```

### Module map (by LOC)

| Module | LOC | Responsibility |
|--------|-----|----------------|
| `db/mod.rs` | 886 | SQLite schema, models, CRUD, progress tracking (bounds-clamped), batch operations |
| `app.rs` | 798 | Application state, selection, keyboard/mouse handling, companion runner integration |
| `ui/components.rs` | 707 | ratatui widgets: series list, chapters table, action bar, details pane, help modal, toasts |
| `scanner.rs` | 503 | Parallel library scanner (thread::scope, atomic work-stealing), ZIP page-count detection |
| `main.rs` | 452 | CLI parsing, config loading, event loop dispatch, direct-file launch |
| `runner/labrador.rs` | 276 | Async Labrador downloader (tokio::spawn, tokio::process::Command) |
| `runner/continuum.rs` | 257 | Continuum reader runner (std::process::Command, JSON payload parsing) |
| `config.rs` | 133 | TOML config with tilde expansion, auto-generation on first run |
| `event.rs` | 114 | Crossterm event stream → AppEvent (key, mouse, tick, scan/download events) |
| `ui/theme.rs` | 90 | Color palette and widget styling |
| `terminal.rs` | 79 | Raw mode, alternate screen, suspend/resume for child-process spawning |
| `db/models.rs` | 77 | Series, Chapter, Progress, SeriesWithStats structs |
| `db/schema.rs` | 37 | CREATE_SCHEMA SQL constant |
| `runner/mod.rs` | 5 | Module re-exports |

### Database schema

Three tables with FK CASCADE:
- `series` — id, title, sort_title, cover_path, status, fetch_url, metadata_json
- `chapters` — id, series_id (FK CASCADE), chapter_number, file_path, page_count, fetch_url; UNIQUE(series_id, chapter_number)
- `progress` — chapter_id (PK, FK CASCADE), last_page_read, is_completed, last_read_at

Indexes: `idx_chapters_series_id`, `idx_chapters_file_path`, `idx_progress_last_read`.

## Conventions (Observed)

### Error handling

- `anyhow::Result` throughout — `?` propagation, `.context()` for error messages.
- `thiserror` 2.0 declared as a dependency but **not actively used** for custom error types.
- No global error handler — `main()` returns `Result<()>`; panics abort (release profile `panic = "abort"`).
- DB migrations use `ALTER TABLE ... ADD COLUMN` with errors silently ignored (`let _ = conn.execute(...)`).

### Async & concurrency

- Tokio multi-thread runtime; `#[tokio::main]` in main.rs.
- Background scan: `tokio::task::spawn_blocking` → `std::thread::scope` with bounded workers (atomic index work-stealing).
- Labrador downloads: `tokio::spawn` + `tokio::process::Command` (async child process).
- Continuum reads: `std::process::Command` (blocking, synchronous — TUI suspended).
- DB access: `Arc<Mutex<Connection>>` — all DB operations lock the mutex; critical sections kept short.

### Type safety

- **Zero `unsafe` blocks** in the codebase.
- Strictly typed via serde derive + clap derive.
- No type-erased patterns (no `Any`-equivalent).

### Observability

- `tracing` macros (`info!`, `warn!`, `error!`) — 30+ calls across 7 modules.
- File-based logging via `tracing-appender` (non-blocking) → `dewey.log` by default.
- No structured JSON logging; no health checks (TUI app, not a service).
- `EnvFilter` from `RUST_LOG` env var, default `info`.

### Testing

- **22 unit tests** across 6 modules (db: 8, runner/continuum: 6, scanner: 3, runner/labrador: 2, app: 2, config: 1).
- DB tests: `Database::in_memory()` — no disk I/O.
- Scanner tests: temp directories with synthetic `.cbz` archives (zip crate).
- **No integration/E2E tests** — Continuum/Labrador spawn flow tested only via JSON parsing unit tests.
- **No mocks** — all tests use real in-memory SQLite or temp files.
- Tests live in `#[cfg(test)] mod tests` within each source file.

## Signals / Active Considerations

- **Debt hotspots:** `db/mod.rs` (886 LOC) and `app.rs` (798 LOC) are the largest modules — candidates for splitting if they grow further.
- **Companion-tool coupling:** Continuum/Labrador integration is via child-process stdout JSON parsing — tightly coupled to their payload contracts (`ContinuumExitPayload`, `LabradorResultPayload`). Contract changes require coordinated releases.
- **DB migrations are fragile:** `ALTER TABLE ... ADD COLUMN` with silently-ignored errors — works for the current additive migrations but would break on schema divergence.
- **No config validation:** missing `library_dir` silently defaults to auto-detected path; no error on invalid paths.
- **Per-directory DB isolation** (`.dewey.db`): recent addition — no migration path for users who previously used a single `dewey.db` in cwd.
- **Progress bounds enforcement:** `upsert_progress` clamps `last_page_read` to `[0, page_count]` — defense-in-depth against reader payload corruption (prior bug: 1446/15). One-time repair query runs on every `init_schema`.
- **Parallel scan safety:** ZIP reads parallelize across `std::thread::scope` workers; DB writes serialize behind the connection mutex. Scan tolerates per-series errors (logs + continues).
- **Touchscreen support:** mouse events piped end-to-end (crossterm → AppEvent → handler) but requires a touch-to-mouse terminal (foot recommended; kitty scroll-only).
