# AGENTS.md

## Cursor Cloud specific instructions

kaigua (开刮) is a single **Tauri 2 desktop app** — a local media-library scraper/organizer.
There is no backend server; SQLite is embedded and the config TOML is auto-created on
first run. "Services" here are dev-toolchain processes, not daemons.

### Layout / commands (see `README.md` and `desktop/package.json` for the canonical list)
- Rust workspace (`crates/media-core`, `crates/scraper-kit`, `crates/renamer`, `desktop/src-tauri`).
- Frontend + Tauri shell live in `desktop/` (pnpm).
- Rust tests: `cargo test -p media-core -p renamer` (run from repo root; no GUI/network needed).
- Lint / token boundary: `cd desktop && pnpm tokens:check`.
- Frontend-only dev server: `cd desktop && pnpm dev` (Vite on port 1420).
- Full app (dev): `cd desktop && pnpm tauri dev` — starts Vite, compiles the Rust host,
  then opens the desktop window.

### Non-obvious caveats
- **Rust toolchain must be >= 1.85 (edition 2024).** `Cargo.lock` pins transitive deps
  (e.g. `moxcms`) that require the `edition2024` feature. The pre-provisioned toolchain is
  the latest `stable` (installed via `rustup default stable`); an older 1.83 toolchain fails
  to even parse the manifests. CI uses `dtolnay/rust-toolchain@stable`, so always build with
  stable, not a pinned-old toolchain.
- **`pnpm tauri dev` needs a display.** This VM provides `DISPLAY=:1`, so the window opens
  there. The first `tauri dev` compiles the whole Rust workspace (~1–2 min); subsequent runs
  are incremental. Run it as a long-lived process (e.g. a tmux terminal) and read its logs —
  do not run it in `install`.
- Software rendering prints a harmless `libEGL warning: DRI3 error ...`; it is not a failure.
- Runtime state is written outside the repo: SQLite DB at
  `~/.local/share/kaigua/kaigua.sqlite3` and config at `~/.local/share/kaigua/config.toml`
  (auto-created). Delete that dir to reset the app to first-run state.
- **Scraping needs API keys** entered in the app UI (Settings / 设置), not env vars — at
  minimum a TMDB key. Library scanning and file organization work fully offline without keys;
  only online metadata matching ("刮削") requires them.
- The token-boundary check (`pnpm tokens:check`) validates committed generated CSS
  (`desktop/src/styles/brand.generated.css`); it runs offline and does not need the external
  `robeshell/kai-brand-design` repo (that repo is only used in CI to re-verify tokens).
