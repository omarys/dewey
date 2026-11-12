# 9. Series identity is title *and* media kind

Adding a light novel whose title Dewey already knew as a manhwa did not create a
library entry. Labrador's chapters were downloaded, and the series never appeared:
`insert_or_get_series_full` matched on `title` alone, so the add flow resolved to
the existing comic row and stopped at "already in library". A webtoon and its
light novel are different things — different reading mode, different chapter
format, both legitimately owned — so a shared title must not be enough to make
them one Series.

Identity is now the title plus the **Media Kind**:

- `MediaKind::Prose` — Category's first path segment is `Light Novel`.
- `MediaKind::Comic` — everything else, including an unshelved row (`category IS
  NULL`), which is what every row looked like before shelves existed.

Manga, manhwa and comicbook deliberately share `Comic`: they are the same kind of
thing to read and to download, and Komikku exports the same series under
whichever of those Type Tags its source was sorted into, so treating them as
distinct would duplicate series on import.

Consequences:

- `insert_or_get_series_full` matches candidates by title and keeps the first
  whose Media Kind agrees. A brand-new row on the other kind of shelf is created
  rather than the existing one being relabelled.
- The Komikku importer keys its existing-row index by `(normalized title, media
  kind)` instead of title alone, so a re-import still finds its series when its
  Type Tag moved between `Manga` and `Manhwa`.
- Nothing changes for rows inside one media kind: two comics with the same title
  are still one Series, whatever sub-shelves (`Manga/Action`, `Manga`) they were
  filed under.

The remaining gap is deliberate: an unshelved row always counts as `Comic`, so
the very first light novel of a title Dewey only knows unshelved creates a second
row rather than adopting it. Comic pages and prose chapters never share a
filename (`.cbz`/`.zip` vs `.md` by chapter number), so the scan keeps them apart
even when both rows own the same folder.
