# 4. Komikku Backup Import

Dewey imports Komikku/Tachiyomi `.tachibk` backups (a gzip-compressed Protocol
Buffers `Backup` message) with `--import-komikku <file>`, so a library that
already lives in Komikku can be adopted without losing reading progress,
bookmarks, or series metadata. The importer reads the protobuf by hand
(`src/komikku/proto.rs`) and skips unknown fields rather than depending on
generated code, because the message shape varies by Komikku version and only a
small subset of fields is needed.

Scope is *favorited series only*. Komikku accumulates un-favorited copies of the
same title when a source breaks and the user migrates, but those copies still
hold reading state, so their chapters are unioned into the favorite by chapter
number. Copies with no reading state at all are dropped; read markers that point
at chapters missing from the favorite copy are reported as unrecoverable rather
than invented.

Dewey's `category` is derived from Komikku's own category assignments, split into
two kinds:

* **Type tags** — `Manga`, `Manhwa`, `Comics`. These set `category` to `Manga`,
  `Manhwa` or `Comicbook` and leave the series **visible**.
* **Rating buckets** — `H`, `Best`, `Good`, `Okay`, `Unrated`, `Partial`. These
  are stored in `metadata_json`, never in `category`.

A series carrying no type tag is **hidden**: a lone `H` becomes a hidden `Manga`,
anything else a hidden `Manhwa`, since Manhwa is the larger bucket among untagged
series. This keeps 1,000+ untagged one-shots and adult entries out of the TUI
until the operator classifies them. `Comicbook` keeps that spelling because the
category doubles as a directory-name fragment and `Comic` reads ambiguously
there.

Because `category` also drives the type filter, the *unfiltered* All view shows
hidden series too — the category and visibility rules stay orthogonal.

Reading mode follows the type (`Manhwa` → `webtoon`, everything else → `manga`)
instead of Komikku's numeric viewer setting, which is a per-series override users
change temporarily and does not describe the medium.

Re-imports are safe. New rows receive category, visibility, reading mode and
metadata; existing rows matched by normalized title keep theirs (the scanner and
the user own those). Progress is a monotone union by default — a chapter never
goes from completed back to unread, page positions never move backwards, and
timestamps keep the newer value — while `--replace` lets the backup win outright.

Relative chapter links (Weeb Central's `/chapters/01J76X`) are not stored as
`fetch_url`, because Labrador needs an absolute URL and a relative path would
merely fail later. They are resolved at import time only when
`[komikku.sources]` maps the source name to a base URL; otherwise the chapter is
imported without a fetch URL and can be re-matched later.

Chapters that lost their URL this way can still be recovered later: add the
source to `[komikku.sources]` and re-import with `--replace`. ADR 5 (`--export-komikku`)
writes the library back out, and it can only export chapters that kept an
absolute `fetch_url`.
