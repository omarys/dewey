# e02s01: Parallel scan verification and stress test

## §1 Business narrative
As a reader with a large library on a low-resource device, I need the library scan to complete fast so the TUI populates quickly.

## §5 Main flow
1. dewey scans library -> collects series subdirs
2. Bounded worker pool (available_parallelism) -> atomic index work-stealing
3. Per-series: read series.json, find cover, detect chapters, open ZIP central dirs
4. DB writes serialized behind Arc<Mutex<Connection>>
5. ScanCompleted -> App reloads

## §6 Constraints
- ZIP reads parallelize; DB writes serialize
- Per-series errors logged and skipped
- Worker count = min(available_parallelism, series_count)

## §17 Gherkin
```gherkin
Scenario: Parallel scan hydrates library
  Given 4 series x 3 cbz in a temp library
  When scan_directory runs
  Then 4 series and 12 chapters are in the DB with page_count

Scenario: Scan tolerates per-series errors
  Given one series with a corrupt cbz
  When scan_directory runs
  Then other series are scanned and corrupt series is warned
```

## §18 Out of scope
- Recursive subdirectory scanning beyond one level
- Real-time filesystem watching
