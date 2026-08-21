# e05s01: Delete series, mark unread, init/reset

## §1 Business narrative
As a solo reader, I need to remove series I no longer want, reset chapter progress when I re-read, and wipe the database to start over — all without accidental data loss.

## §5 Main flow
1. `x` on a series -> first press arms, second press confirms -> cascade-deletes series+chapters+progress
2. `u` on a chapter -> clears progress row (page 0, uncompleted)
3. `dewey --init` -> wipes DB file + WAL/SHM, exits; next launch recreates fresh

## §6 Constraints
- Delete requires double-press confirmation; navigation cancels
- Clear progress removes the progress row entirely
- --init deletes the DB file; does NOT recreate in-process
- FK ON DELETE CASCADE removes chapters and progress automatically

## §17 Gherkin
```gherkin
Scenario: Delete a series with double-confirm
  Given "Solo Leveling" is selected
  When I press x
  Then I see "Press x again to delete this series"
  When I press x again
  Then "Solo Leveling" is removed and its chapters/progress are cascade-deleted

Scenario: Mark chapter unread
  Given a chapter with progress at page 12/48
  When I press u
  Then the chapter shows no progress and its progress row is deleted

Scenario: Reset database
  When I run "dewey --init"
  Then the database file and WAL/SHM sidecars are removed
```

## §18 Out of scope
- Bulk delete (multiple series at once)
- Undo/restore for deleted series
- Progress history (only current state tracked)
