# 8. Chapter numbers must be unique per series

Dewey keys chapters by `UNIQUE(series_id, chapter_number)` and writes chapter
lists through an upsert on that key. A provider that reports the same number for
several chapters therefore does not "add duplicates" — it silently overwrites,
and the earlier chapters lose their title and fetch URL.

This is not hypothetical. NovelPing names its chapters volume-first
(`Volume 1  1 — Could This Be Another World?`), and the shared
`domain.ParseChapterNumber` returns the *first* number in the string: the
volume. Every chapter of volume 1 reported `1`. Importing a 285-chapter novel
stored 26 rows — one per volume, titled with whichever chapter landed last.

Two changes, at the two layers that were wrong:

- **Provider (Labrador).** NovelPing numbers by volume and position:
  `volume + position/1000` — `1.0`, `1.001`, `1.002`, `2.0`, … Unique, ordered,
  and it keeps the volume structure visible in the number.
- **Consumer (Dewey).** `runner::labrador::remote_chapters` checks the reported
  numbers before trusting them. If any repeat, it falls back to the provider's
  listing order (`1..n`) for the whole series. Providers emit chapters in
  reading order, so this is a faithful ordering, and it protects every provider,
  not just the one that broke.

The fallback is deliberately all-or-nothing rather than "bump the duplicates":
mixing reported numbers with synthesized ones produces sequences like
`1, 2.001, 3, 4.5` that sort plausibly but lie about the reading order.

Two consequences worth knowing:

- Which scheme was used shows up in the UI: `Ch. 105.5` means the provider's own
  numbering survived; `Ch. 1..285` on a prose series means it was replaced.
- Re-importing a series whose numbers change rewrites rows that already exist
  (same key, `COALESCE`-updated title/URL, `file_path` untouched), so downloads
  stay attached. It does not orphan the old rows as long as the new numbering
  still covers them.
