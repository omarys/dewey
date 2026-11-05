# 5. Komikku Backup Export

Dewey writes its library back out as a Komikku/Tachiyomi `.tachibk` backup with
`--export-komikku <path>`, the inverse of ADR 4. It exists so the import is not a
one-way door: a library adopted from Komikku can return to Komikku (or to a
successor reader) with its sources, categories, chapters and reading state
intact. The encoder is hand-written next to the decoder (`src/komikku/proto.rs`),
for the same reason: a small, version-tolerant subset beats generated code that
pins one Komikku release.

**Every series is exported as a favorite.** In Komikku (and in Mihon, which shares
schemas) library membership *is* the favorite flag; there is no separate list. A
non-favorite would be the exact equivalent of a series that vanished from the
library, so the fidelity this feature promises — "everything Dewey has" — would
be lost. Non-favorite Komikku entries are notable, occasionally interesting data,
not a trustworthy boundary.

**Categories are replayed, not derived.** The import recorded the category names
each series actually carried in `metadata_json.komikku.categories`, and the export
writes exactly those. Deriving them from Dewey's `category` column instead would
silently un-hide series: visibility is decided by the presence of a type tag, so
a hidden rating-only entry ("`Unrated`") promoted to a hidden `Manhwa` would come
back visible on the next import. A round-trip therefore lands on the same
hidden/visible split it started from. Only a series with no recorded categories
(a Dewey-only series that matched a Komikku twin) falls back to Dewey's type.

**Chapters without a `fetch_url` are skipped**, and counted in the report. A
Komikku chapter is addressed by URL; writing a chapter with an empty URL would
produce a backup that imports cleanly and then fails on tap, which is worse than
a visible gap. The gaps are the flip side of ADR 4's relative-link rule, and they
are not small: on the reference library 43,787 of 74,665 chapters had no URL,
carrying 79% of the completed markers and 88% of the bookmarks, with 893 of 1,186
series left with nothing exportable at all.

The remedy is config, not code, and the import report now names it: it lists
every source whose relative links could not be resolved, with chapter counts, and
prints the instruction. Add those sources to `[komikku.sources]` and re-import
with `--replace`, and the same chapters gain absolute `fetch_url`s — after which
the export covers them. Until then the gap is reported rather than hidden.

**Viewer, dates and scanlators are left at their defaults.** Komikku's `viewer`
is a per-series override users flip temporarily, not a property of the medium, so
Dewey does not guess one from its own `reading_mode`. Upload/fetch dates and
scanlator names were never stored, and inventing them would make the backup look
more complete than it is.

Source bindings come from `metadata_json.komikku` (`id`, `source`, `url`,
`thumbnail_url`, `status`), and every distinct `(source name, id)` pair is also
written to the backup's source list so Komikku can resolve the extension on
restore. A series Komikku never saw borrows the binding of a same-titled series it
does have; if there is no such twin and no `fetch_url`, the series is skipped and
counted rather than exported under a fabricated source.

The whole library goes through one in-memory encode and a single file write, so a
failed export never leaves a half-written backup behind. `--dry-run` prints the
same report without touching the filesystem.
