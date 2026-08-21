# e01s01: Multi-chapter progress payload contract

## §1 Business narrative
As a reader who reads across multiple chapters in one Continuum session, I need dewey to persist per-chapter progress for ALL chapters I read — not just the first one — with page numbers local to each file, so progress is never corrupted by global offsets.

## §5 Main flow
1. Open chapter in dewey -> Continuum spawns with --file --page
2. Read across multiple chapters (Continuum auto-appends)
3. Close Continuum -> JSON: {"last_page":N,"completed":B,"chapters":[{"file":"...","last_page":N,"completed":B},...]}
4. dewey parses chapters[] -> for each: resolve by file_path -> upsert_progress (clamped)
5. Toast: "Saved progress - N chapters updated"

## §6 Constraints
- Every page number LOCAL to its file (no global counters)
- Legacy top-level fields mirror first chapter (backward compat)
- upsert_progress clamps to [0, page_count] at DB layer
- chapters[] absent -> legacy single-chapter path

## §17 Gherkin
```gherkin
Scenario: Multi-chapter reading session
  Given chapters [0047], [0048], [0049] in the library
  When I open [0047] and read through [0049]
  Then dewey persists progress for all 3 chapters
  And each last_page is local and <= page_count

Scenario: Backward compat with old Continuum
  Given old Continuum emits only {"last_page":5,"completed":false}
  Then chapters is None and dewey uses legacy single-chapter path
```

## §18 Out of scope
- Continuum reader implementation
- Labrador integration
- Schema versioning protocol
