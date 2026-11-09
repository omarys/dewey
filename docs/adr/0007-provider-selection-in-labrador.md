# 7. Provider selection lives in Labrador, not Dewey

Dewey's add-series flow shells out to Labrador's interactive picker
(`labrador select`). Two things were wrong with it:

1. It starts on the **search** screen, so the provider screen — which Labrador
   already implements, with checkboxes and a unified multi-provider search — was
   unreachable. `esc` and `q` quit select mode outright; the only path back
   (`handleBack`, `screenSearch → screenProviders`) was dead code in that mode.
2. The provider set was a hardcoded literal
   (`mangakatana`, `mangadistrict`, `private_gallery`, `weebcentral`), so the
   other seven providers could not be searched at all from Dewey.

The obvious alternative — a Dewey-owned provider checklist feeding
`labrador search --provider <id> --json` per provider — was rejected. It would
duplicate a screen Labrador already has, and it would produce a worse result
list: `search --json` emits only `{provider_id, provider_name, id, title, url}`
(`internal/domain/series.go`), with no cover, description, or latest chapter,
while Labrador's TUI shows the richer listing and offers browse/tags.

So the split is:

- **Labrador** owns the provider screen. `select` opens on it, `esc` steps back
  to it instead of quitting, `--providers a,b` pre-checks ids, and the result
  JSON gains the checked ids.
- **Dewey** owns the memory. It passes `[labrador].providers` from `config.toml`
  as `--providers`, and writes back whatever Labrador reports, so the next
  launch pre-checks the same set.

Dewey keeps no provider list of its own and never enumerates providers — it
would only ever be a worse copy of `labrador providers --json`. Configuration
stays in Dewey because Labrador has no config file at all (its only persisted
state is the download queue), and because the set is a Dewey-level preference:
which sources this library wants results from.

A consequence worth stating: a saved set is a *filter*, not just a default. If
every provider you tick goes down, searches come back empty — empty the
`providers` array (or tick others in the picker) to fall back.
