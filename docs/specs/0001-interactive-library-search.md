# Spec 0001: Interactive Library Search & Multi-Criteria Filtering

**Status:** Ready for Implementation
**Triage Role:** `ready-for-agent`

## Problem Statement
As manga and comic libraries grow to hundreds of series and thousands of chapters, navigating strictly with sequential list scrolling becomes slow and cumbersome. Users cannot quickly locate a specific title by typing part of its name, nor can they isolate unread or ongoing series from completed backlogs.

## Solution
Introduce an interactive search overlay mode (`/` key) in Dewey with real-time fuzzy title/author matching, alongside dedicated quick-filter toggles (`F` key or status filters) to filter by completion status (e.g. Unread, In Progress, Completed). The filtered list seamlessly integrates with keyboard and touchscreen controls, retaining reading progress and Continuum launch capabilities.

## User Stories

1. As a reader with a large library, I want to press `/` to open a search prompt, so that I can type a title and instantly narrow the series list.
2. As a reader, I want live fuzzy filtering as I type each character, so that I get immediate visual feedback without pressing Enter.
3. As a reader, I want to press `Esc` while searching to clear the search query and restore the full library view.
4. As a reader, I want to press `Enter` or `Down` from the search bar to immediately focus and browse the matching results.
5. As a reader, I want the search to match against both the display title and the `sort_title` (e.g. matching "Solo" or "Chugong" for Solo Leveling).
6. As a reader, I want to toggle an "Unread Only" filter (hotkey `F`), so that I only see series that have unread or in-progress chapters.
7. As a reader, I want to toggle between "All", "Ongoing", and "Completed" publication status filters, so that I can focus on finished or active releases.
8. As a reader, I want a visual status badge/indicator in the UI header and status bar showing active filters (e.g. `[Filter: Unread (12/85 series)]`).
9. As a reader using a touchscreen tablet, I want a tappable Search / Filter button in the action bar, so that I can search and filter without a physical keyboard.
10. As a reader, I want opening a chapter in Continuum from a filtered list to properly track progress and return to the filtered view without resetting my query.
11. As a reader, I want search queries to be case-insensitive and resilient to minor typos or alternate punctuation (e.g. dashes, underscores).
12. As a reader, I want background scans and chapter downloads to update the filtered list reactively without interrupting an active search input.

## Implementation Decisions

- **Modal Input State Machine**:
  - Extend application state with an explicit `InputMode` enum (`Normal`, `SearchInput`, `FilterMenu`).
  - In `SearchInput` mode, raw keypresses route directly to an internal string buffer and update the active filtered projection immediately.
  - Arrow keys, `Enter`, `Tab`, and `Esc` handle navigation, selection, and mode exit.

- **Non-Destructive Projection Filtering**:
  - The master `series_list` remains intact in memory; the UI and table state render from an active filtered view slice (`filtered_series_indices: Vec<usize>`).
  - Index selection remains safe and bounded to `[0, filtered.len())`.

- **Persistent Search & Filter Bar Widget**:
  - Render an unobtrusive search bar above or integrated into the Series List panel border.
  - Display matched result count and active filter pills (e.g., `🔍 "solo" · Unread (1/2)`).

- **Touchscreen & Action Bar Integration**:
  - Add `AppAction::Search` and `AppAction::Filter` buttons to the bottom touch action bar.
  - Tapping Search focuses the query input; tapping the filter pill cycles filter modes.

## Testing Decisions

- **Behavioral Testing at the App Seam**:
  - Verify full keyboard interaction: typing `/` -> string keys -> checking filtered series list length and selected item.
  - Verify `Esc` restores initial unfiltered library state.
  - Verify deleting search text (Backspace) dynamically expands results back.
  - Verify action triggers (e.g., `Enter` to read, `m` to toggle read) operate on the correct underlying series/chapter even when filtered.
- **Prior Art**:
  - Follow the existing test architecture in `src/app.rs` and `src/db/mod.rs` using in-memory SQLite and mock event streams.

## Out of Scope

- Remote online metadata scrapers (managed via Labrador).
- Full-text optical character recognition (OCR) inside archive pages.
- Complex SQL boolean query DSL (simple space-separated fuzzy tokens are sufficient).

## Further Notes

- Maintains zero I/O overhead on USB/slow storage by performing filtering on the in-memory series list without disk re-queries.
- Fully compatible with both portrait and landscape touchscreen layouts.
