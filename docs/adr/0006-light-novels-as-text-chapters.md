# 6. Light Novels as Text Chapters

Dewey tracks light novels alongside comics. A novel chapter is a plain-text file
(`.txt`, `.md`), so there is nothing to render: opening a chapter suspends the TUI
and hands the file to an external reader, then restores the TUI. The reader is not
Continuum — no page model, no image pipeline, no decompression.

**Markdown goes to a markdown viewer; everything else to the editor.** `.md`/
`.markdown` chapters open in Leaf, then Glow, then the editor, whichever works
first. Other text chapters (`.txt`) open in the editor directly — a markdown
renderer has nothing to offer them. The editor is resolved from the `editor`
config key, then `$EDITOR`, `$VISUAL`, and `vi`; `markdown_viewer` overrides the
viewer chain, and `markdown_viewer = "editor"` opts out of it entirely.

**Candidates are probed by execution, not by `PATH` lookup.** A version-manager
shim (mise, asdf) sits on `PATH` and still exits non-zero with "No version is set
for shim: glow" — a `PATH` check would happily select it and replace the chapter
with an error message. So each candidate is run once with `--version` at startup
and only used if that succeeds. This is why `glow` is spelled `glow -p` in the
chain: plain `glow` prints and exits, scrolling the chapter away before the TUI
redraws, so the pager is what makes it a usable reader. A *configured* viewer is
taken as-is and never probed — an explicit setting means the user insists.

**Detection is by file extension, per chapter.** A chapter whose file is
`.txt`/`.md` opens in the editor; every other chapter opens in Continuum. Mixed
series therefore work without a per-series flag. Text files only count as chapters
when their name carries a chapter number (`Chapter 001.md`, `12.txt`, `Volume 3.md`),
so a series folder's `README.md` or `notes.txt` is not mistaken for chapter 0. The
existing chapter-number parser is reused for this, which also means text chapters
sort and resume like any other chapter.

**A text-only series is filed as a Light Novel.** The scanner sets the category to
`Light Novel` when a series' chapters are plain text *and* its path does not
already name a shelf — an explicit folder (`Library/Manga/`) still wins, because
the category is also the directory the scanner and the category mover operate on.
`TypeFilter` gained a `LightNovel` variant (cycling with `T`), matching the
`Light Novel` shelf, its sub-shelves, and `Novel`/`Novels` folders.

**Progress records "opened", not pages.** A text chapter has no page count, so
`mark_chapter_opened` writes an in-progress row with a fresh `last_read_at`, and on
conflict updates *only* the timestamp: re-opening a finished chapter never
un-completes it, and no page position is invented. Marking a chapter read stays the
existing manual `m` toggle. The chapter list shows `Reading` for these rows instead
of the meaningless `Page 0/0`.

Consequences: a novel series never needs a `page_count`, its chapters are never
"downloaded" in the Continuum sense, and `--import-komikku`/`--export-komikku` are
unaffected — Komikku has no light-novel concept, so these series only round-trip as
ordinary entries if a text chapter is ever imported to one (it is not: the importer
only maps chapters that carry a URL or a local archive path).
