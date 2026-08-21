# Conventions

## Code style

- Rust edition 2021, idioms preferred
- `anyhow::Result` for error propagation, `thiserror` for domain errors
- `serde` with `#[serde(default)]` for optional fields
- Prefer stdlib over dependencies; add a crate only when stdlib cannot do it

## Defensive code

- **Bounds clamping:** `upsert_progress` clamps `last_page_read` to `[0, page_count]` — every caller goes through it
- **FK constraints:** `ON DELETE CASCADE` on chapters/progress; `PRAGMA foreign_keys = ON` on every open
- **Double-confirm on delete:** `x` requires pressing twice on the same series; navigation cancels
- **Input validation:** clap derives CLI args; config defaults generated on first run
- **Parallel scan:** ZIP reads run in parallel threads; DB writes serialize behind the connection mutex

## Git

- Conventional Commits style
- `main` branch is protected; work in feature branches or worktrees
- Do NOT commit unless explicitly asked
- Do NOT push to origin without explicit approval
- Pre-commit hooks run `cargo fmt` + `cargo clippy` + `cargo test` (see `.pre-commit-config.yaml`)

## Tests

- Unit tests in each module's `#[cfg(test)] mod tests`
- DB tests use `Database::in_memory()` — no disk I/O
- Scanner tests build temp directories with synthetic `.cbz` archives
- Run: `cargo test --release`

## Specs

All planning output goes to `specs/` at the project root. Key files:
- `specs/state.yaml` — session state, active epic
- `specs/release-plan.yaml` — release index
- `specs/product/` — scope, vision, glossary
- `specs/tech-architecture/` — tech stack, security, test plans
- `specs/bugs/registry.yaml` — bug tracker
