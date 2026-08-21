# e03s01: Per-directory DB and empty-library guard verification

## §1 Business narrative
As a reader with multiple library directories, I need each to have its own isolated database — and dewey should refuse initializing a library with no comics.

## §5 Main flow
1. `dewey .` or `-l <dir>` -> DB at <dir>/.dewey.db (unless --db-path given)
2. No config file needed — per-dir DB is automatic
3. Empty library (no cbz, no DB) -> "No .cbz files found — skipping" + exit
4. Existing DB + empty dir -> opens normally

## §6 Constraints
- Per-dir DB only when library dir is explicit AND no --db-path
- Config file's db_path takes precedence
- .dewey.db is hidden and skipped by the scanner

## §17 Gherkin
```gherkin
Scenario: Per-directory DB creation
  Given dir A with cbz files and no config
  When I run "dewey ." in dir A
  Then A/.dewey.db is created

Scenario: Empty library guard
  Given dir B with no cbz and no DB
  When I run "dewey ." in dir B
  Then I see "No .cbz files found" and no DB is created

Scenario: Existing DB, empty library
  Given dir C with .dewey.db but no cbz currently
  When I run "dewey ." in dir C
  Then dewey opens normally
```

## §18 Out of scope
- Auto-migration from single dewey.db
- Cloud sync of per-directory databases
