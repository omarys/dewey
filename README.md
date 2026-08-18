# 📚 Dewey

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)
[![TUI](https://img.shields.io/badge/tui-ratatui-blue.svg)](https://github.com/ratatui-org/ratatui)
[![Database](https://img.shields.io/badge/database-sqlite-lightgrey.svg)](https://sqlite.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

**Dewey** is a modern, keyboard-driven Terminal User Interface (TUI) manga, manhwa, and comic library manager written in Rust.

Dewey acts as the central orchestration "brain" between your local library, reading tools, and download utilities:
1. **[Continuum](https://github.com/omarys/continuum)**: Minimalist GTK4/Rust manga reader.
2. **Labrador**: Automated scraping & chapter fetching tool.

---

## ✨ Features

- ⚡ **Dual-Pane Terminal UI**: Split layout built with `ratatui` and `crossterm` for browsing series, inspecting chapter lists, and viewing rich metadata.
- 🗄️ **SQLite Persistence**: Normalized database tracking series metadata, chapter records, reading progress, completed states, and source URLs.
- 📖 **Seamless Continuum Reader Integration**:
  - Automatically resumes reading from the exact `last_page_read`.
  - Spawns Continuum with `--file <path> --page <num> --tui`.
  - Suspends and resumes the terminal cleanly around the GUI reader session.
  - Captures stdout JSON payloads on close to persist reading state.
  - Displays celebratory completion toasts when finishing chapters.
- 🐕 **Non-Blocking Background Fetching (Labrador)**:
  - Spawns asynchronous download tasks with an animated spinner queue in the status bar.
  - Passes known source URLs (`--url <fetch_url>`) or allows Labrador to automatically resolve missing URLs.
  - Dynamically updates SQLite with new file paths and discovered provider URLs upon download completion.
- 📁 **Automated Library Scanner**:
  - Scans designated directories (e.g., `~/Documents/Books`, `~/Documents/Manga`).
  - Parses `series.json` for titles, authors, publishers, status, and fetch URLs.
  - Detects cover art (`cover.png`, `cover.jpg`, `folder.jpg`).
  - Robust chapter regex parser (`[0001]_Chapter_1.cbz`, `Solo Leveling - c105.cbz`, `Ch. 12.5.zip`).
  - Reads `.cbz` / `.zip` central directory headers to count pages without decompressing.
- 🎯 **Direct CLI File Launcher**: Run `dewey /path/to/chapter.cbz` to launch Continuum with progress resumption and auto-saving directly from the shell without entering the full TUI.

---

## 🏛️ Architecture & Orchestration

```
                      ┌────────────────────────┐
                      │      Dewey (TUI)       │
                      │  (Ratatui + SQLite)    │
                      └─────┬────────────┬─────┘
                            │            │
             Suspends TUI   │            │ Asynchronous
             & passes state │            │ Tokio background job
                            ▼            ▼
               ┌────────────────┐    ┌────────────────┐
               │   Continuum    │    │    Labrador    │
               │  (GTK4 Reader) │    │  (Downloader)  │
               └───────┬────────┘    └───────┬────────┘
                       │                     │
      stdout exit JSON │    Discovered URL / │ File path & page count
                       ▼                     ▼
           ┌─────────────────────────────────────┐
           │      SQLite Database (dewey.db)     │
           │  • series                           │
           │  • chapters                         │
           │  • progress                         │
           └─────────────────────────────────────┘
```

---

## ⌨️ Keybindings

| Key | Action | Description |
|:---|:---|:---|
| <kbd>j</kbd> / <kbd>↓</kbd> | **Down** | Navigate to the next series or chapter |
| <kbd>k</kbd> / <kbd>↑</kbd> | **Up** | Navigate to the previous series or chapter |
| <kbd>Tab</kbd> / <kbd>l</kbd> / <kbd>→</kbd> | **Next Pane** | Switch focus between Series List and Chapters List |
| <kbd>Shift+Tab</kbd> / <kbd>h</kbd> / <kbd>←</kbd> | **Prev Pane** | Switch focus backwards |
| <kbd>Enter</kbd> | **Read / Fetch** | Open chapter in Continuum (or fetch via Labrador if missing) |
| <kbd>d</kbd> | **Download** | Fetch selected chapter in background via Labrador |
| <kbd>D</kbd> | **Download Next** | Fetch the next un-downloaded chapter in current series |
| <kbd>s</kbd> | **Scan Library** | Re-scan the designated library folder for new files |
| <kbd>m</kbd> | **Toggle Read** | Toggle chapter completed / uncompleted status |
| <kbd>r</kbd> | **Reload** | Reload library entries and database stats |
| <kbd>?</kbd> | **Help** | Toggle the keyboard navigation help modal |
| <kbd>q</kbd> / <kbd>Ctrl+C</kbd> | **Quit** | Exit Dewey cleanly |

---

## ⚙️ Configuration

Dewey looks for a configuration file in `dewey.toml` (current directory) or `~/.config/dewey/config.toml`. If not found, a default configuration is generated automatically.

```toml
# Path to your comics / manga library directory
library_dir = "~/Documents/Books"

# Path to SQLite database file
db_path = "dewey.db"

# Log file path
log_file = "dewey.log"

# Binary names or paths for companion tools
continuum_bin = "continuum"
labrador_bin = "labrador"

# Scan the library folder automatically on launch
auto_scan_on_startup = true

# Seed sample data if database is empty
seed_sample_data = false
```

### CLI Options

```bash
# Launch interactive TUI
dewey

# Launch with custom library directory
dewey --library-dir ~/Documents/Manga

# Launch with custom config file
dewey -c /path/to/dewey.toml

# Direct file launch (runs Continuum with progress tracking & saves on close)
dewey ~/Documents/Books/Chainsaw_Man/[0001]_Chapter_1.cbz
```

---

## 📦 Building & Development

### Prerequisites
- **Rust Toolchain**: 1.85+ (`mise` or `rustup`)
- **SQLite3** (bundled automatically via `rusqlite`)
- **[mise-en-place](https://mise.jdx.dev/)** (recommended)

### Quick Start

```bash
# Clone the repository
git clone https://github.com/omarys/dewey.git
cd dewey

# Install toolchain & dependencies via mise
mise install

# Run the test suite
cargo test

# Run strict clippy & linter checks
cargo clippy -- -D warnings

# Build optimized binary
cargo build --release
```

### Testing

Dewey includes unit tests covering:
- In-memory SQLite database operations, schema migrations, and progress lookups.
- Continuum argument generation and stdout JSON parsing.
- Labrador async result and URL resolution parsing.
- Archive page detection and chapter naming regex patterns.

```bash
cargo test
```

---

## 📄 Database Schema

```sql
CREATE TABLE IF NOT EXISTS series (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    sort_title TEXT,
    cover_path TEXT,
    status TEXT,
    fetch_url TEXT,
    metadata_json TEXT
);

CREATE TABLE IF NOT EXISTS chapters (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    series_id INTEGER NOT NULL,
    chapter_number REAL NOT NULL,
    file_path TEXT,
    page_count INTEGER,
    fetch_url TEXT,
    FOREIGN KEY (series_id) REFERENCES series (id) ON DELETE CASCADE,
    UNIQUE(series_id, chapter_number)
);

CREATE TABLE IF NOT EXISTS progress (
    chapter_id INTEGER PRIMARY KEY,
    last_page_read INTEGER NOT NULL DEFAULT 0,
    is_completed BOOLEAN NOT NULL DEFAULT 0,
    last_read_at DATETIME,
    FOREIGN KEY (chapter_id) REFERENCES chapters (id) ON DELETE CASCADE
);
```

---

## 📜 License

MIT License. See [LICENSE](LICENSE) for details.
