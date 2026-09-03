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
  - Spawns Continuum with `--file <path> --page <num>`.
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
| <kbd>g</kbd> / <kbd>G</kbd> | **Jump Top / Bottom** | Jump to the first / last item of the active list |
| <kbd>Tab</kbd> / <kbd>l</kbd> / <kbd>→</kbd> | **Next Pane** | Switch focus between Series List and Chapters List |
| <kbd>Shift+Tab</kbd> / <kbd>h</kbd> / <kbd>←</kbd> | **Prev Pane** | Switch focus backwards |
| <kbd>Enter</kbd> | **Read / Fetch** | Open chapter in Continuum (or fetch via Labrador if missing) |
| <kbd>d</kbd> | **Download** | Fetch selected chapter in background via Labrador |
| <kbd>D</kbd> | **Download Next** | Fetch the next un-downloaded chapter in current series |
| <kbd>s</kbd> | **Scan Library** | Re-scan the designated library folder for new files |
| <kbd>m</kbd> | **Toggle Read** | Toggle chapter completed / uncompleted status |
| <kbd>u</kbd> | **Mark Unread** | Clear a chapter's progress (page 0, not completed) |
| <kbd>x</kbd> | **Delete Series** | Remove selected series (press twice to confirm) |
| <kbd>Delete</kbd> | **Delete Chapter** | Remove selected chapter (press twice to confirm) |
| <kbd>r</kbd> | **Reload** | Reload library entries and database stats |
| <kbd>?</kbd> | **Help** | Toggle the keyboard navigation help modal |
| <kbd>q</kbd> / <kbd>Ctrl+C</kbd> | **Quit** | Exit Dewey cleanly |

### Touchscreen (tablets)

In a terminal that translates touch to mouse events (e.g. **foot** on
Wayland):

- **Tap** an item to select it (and focus that pane)
- **Double-tap** a series to open its chapters, a chapter to read it
- **Scroll wheel** moves the selection in the pane under the cursor
- A bottom **action bar** (Open / Fetch / Next / Scan / Reset / Delete /
  Quit) gives one-tap access to the main actions without a keyboard
- In **portrait** orientation the panes stack vertically instead of
  side-by-side, so columns stay wide enough for fingers.

> **Terminal choice**: **foot** translates touchscreen taps into mouse-button
> events (its `[touch]` section) and is lightweight — the recommended terminal
> on tablets. **kitty** supports SGR mouse reporting, so a *pointer/wheel*
> works, but its touchscreen support is scroll-only (no tap→click synthesis),
> so kitty is not sufficient for finger-based use.


---

## ⚙️ Configuration

Dewey looks for a configuration file in `dewey.toml` (current directory) or `~/.config/dewey/config.toml`. If not found, a default configuration is generated automatically.

```toml
# Path to your comics / manga library directory
library_dir = "~/Documents/Books"

# Path to SQLite database file (defaults to ~/.local/share/dewey/dewey.db when unset)
db_path = "~/.local/share/dewey/dewey.db"

# Log file path (defaults to ~/.local/state/dewey/dewey.log when unset)
log_file = "~/.local/state/dewey/dewey.log"

# Binary names or paths for companion tools
continuum_bin = "continuum"
labrador_bin = "labrador"

# Scan the library folder automatically on launch
auto_scan_on_startup = true

# Seed sample data if database is empty
seed_sample_data = false
```

### SQLite Memory Tradeoff

On open, Dewey sets generous SQLite memory sizes:

| Pragma | Default | What it does |
|:---|:---|:---|
| `mmap_size` | 256 MB | *Cap* on files SQLite maps into memory. It is lazy — only pages actually read are mapped — so it rarely uses the full amount. |
| `cache_size` | 64 MB (page cache) | Reserved RAM held for SQLite's page cache. This *is* reserved up front. |

`mmap_size` is just an upper bound and is safe to leave at the default. On a
low-RAM device the real cost is `cache_size`: 64 MB of page cache is generous
for a comic library. To trade a little scan/read speed for much lower memory
pressure, lower it in `Database::open` (src/db/mod.rs) — e.g. `-8000` (~8 MB)
and `mmap_size=67108864` (64 MB). The defaults favor speed; tune down only if
you observe memory pressure.

### CLI Options

```bash
# Launch interactive TUI
dewey

# Launch with custom library directory
dewey --library-dir ~/Documents/Manga

# Launch with USB / removable media optimization profile
dewey -u --library-dir /media/user/DRIVE/Manga
# or: dewey --storage-profile usb --library-dir /media/user/DRIVE/Manga

# Launch with custom config file
dewey -c /path/to/dewey.toml

# Reset the database and start over (deletes all data)
dewey --init

# Direct file launch (runs Continuum with progress tracking & saves on close)
dewey ~/Documents/Books/Chainsaw_Man/[0001]_Chapter_1.cbz
```

> **Library location**: the library directory is **not hard-coded**. Set
> `library_dir` in your config file (`dewey.toml` or
> `~/.config/dewey/config.toml`), or pass `-l/--library-dir` to override it. A
> default is only auto-detected (and written to a freshly generated config) when
> **no** config file exists — it picks the first existing of
> `~/Documents/Books`, `~/Documents/Manga`, `~/Manga`, `~/Books`.
>
> **Multiple libraries (per-directory databases)**: when you point Dewey at a
> library directory explicitly — `dewey .`, `dewey <dir>`, or `-l <dir>` — it
> keeps a **separate database per directory** (`<dir>/.dewey.db`, created if
> missing), so several libraries stay fully isolated. Override with
> `--db-path` if you want a specific DB. When no directory is given (plain
> `dewey`), the configured `library_dir` / `db_path` are used instead.
> A directory positional argument is treated as a library directory; a file
> (`.cbz`/`.zip`) positional still triggers direct-launch.
>
> **Empty-library guard**: initialization is skipped (with a message) when the
> target directory contains no comic archives (`.cbz`, `.zip`, `.epub`, `.pdf`,
> `.cbr`) anywhere in its tree and no database exists yet — an existing library
> that is temporarily empty still opens normally.

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

### Native Device Build & Desktop Shortcut (internal)

For maximum performance on a specific machine — especially a low-resource
ARM device like the PineTab 2 — build with CPU-native optimizations. The
compiler then uses every instruction the local CPU supports (NEON/SIMD on
ARM, AVX on x86). `install.sh` also sets up the desktop application shortcut
and icon for launching in the `foot` terminal:

```bash
./install.sh        # builds with -C target-cpu=native and installs binary, desktop entry & icon
# or: DEWEY_PREFIX=/opt/bin ./install.sh
```

> ⚠️ A native build is **not portable** (it targets the build machine's exact
> CPU), so it is intended for internal/per-device use.
>
> Run `install.sh` on each device you deploy to. If you set `RUSTFLAGS`
> yourself, note that `install.sh` appends `-C target-cpu=native` to it.

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
