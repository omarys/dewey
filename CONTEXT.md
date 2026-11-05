# Dewey

Dewey is a keyboard-driven TUI library manager and orchestration brain for comic, manga, and manhwa collections, managing metadata, reading progress, and external companion tools.

## Language

### Core Entities

**Library**:
The designated local root directory containing Series folders, scanned by Dewey to discover content and synchronize state.
_Avoid_: Collection, workspace, book directory

**Series**:
A cohesive comic, manga, or manhwa work consisting of an ordered sequence of Chapters, cover art, and metadata.
_Avoid_: Title, book, comic, folder

**Chapter**:
A single numbered installment within a Series, identified by a fractional chapter number, which can be backed by a local Archive or pending remote download.
_Avoid_: Issue, episode, installment, file

**Archive**:
The physical local container file (`.cbz`, `.zip`, `.cbr`, `.pdf`, `.epub`) holding compressed page images for a downloaded Chapter.
_Avoid_: Zip, comic file, payload, bundle

**Progress**:
The persisted reading state for a Chapter, strictly bounded to `[0, page_count]`, with an explicit completion status and timestamp.
_Avoid_: Bookmark, global page offset, read state

**Reading Mode**:
The directional presentation format for a Series or Chapter, specifically `webtoon` (continuous vertical strip) or `manga`/`comic` (paged view).
_Avoid_: Layout mode, view style, format

### Architecture & System

**Companion Tool**:
A standalone child-process utility spawned and orchestrated by Dewey, specifically Continuum (GTK4 reader) or Labrador (chapter scraper).
_Avoid_: Plugin, internal module, extension

**Storage Profile**:
An operational I/O configuration (`Fast` vs `Usb`) that tunes SQLite memory-mapping, scanner thread pool concurrency, and reader archive pre-buffering to the storage medium.
_Avoid_: Theme, performance tier, hardware mode

### Import & Classification

**Category**:
A Series' primary type, stored as free text and doubling as the directory-name fragment when a Series is downloaded, where `Manga`, `Manhwa` and `Comicbook` are the values the importer produces.
_Avoid_: Genre, tag, label, collection

**Type Tag**:
One of Komikku's category assignments `Manga`, `Manhwa`, `Comics`; the only kind of tag that sets a Series' Category and leaves the Series visible.
_Avoid_: Main category, primary genre

**Rating Bucket**:
Any other Komikku category — `H`, `Best`, `Good`, `Okay`, `Unrated`, `Partial` — recorded as metadata because it grades a Series rather than describing its type.
_Avoid_: Rating, score, tier, status

**Hidden Series**:
A Series excluded from the default TUI list; a Series with no Type Tag is imported hidden so untriaged entries stay out of the way.
_Avoid_: Archived, deleted, filtered out

**Komikku Backup**:
A gzipped Protocol Buffers snapshot (`.tachibk`) exported by Komikku/Tachiyomi, the only external library format Dewey imports from.
_Avoid_: Export file, dump, archive
