# AGENTS.md

Guidance for coding agents working on jiraf. Read [README.md](README.md) for what the app does, and
[CHANGELOG.md](CHANGELOG.md) for recent changes.

## What this is

A stateless follow-up dashboard over Jira (Data Center) Plans, shipped as a Tauri 2 desktop app
(macOS today; keep a Windows build possible). The Rust side talks to Jira; the page is plain HTML/JS
that calls Tauri commands. Nothing is stored except config, an icon cache, a log and the window size.

## Layout

| Path | What |
| --- | --- |
| `src/main.rs` | Tauri builder: plugins, menu (Settings…, Reload config), `jiraf://` icon scheme, command list |
| `src/app.rs` | startup (`init`), `Holder` (phase + generation counter), `Failure` (setup / token / other), startup commands |
| `src/config.rs` | `jiraf.toml` model, parse/load/validate, clean rendering, ids, token from `.env` |
| `src/settings.rs` | Settings screen: validation, Jira checks, atomic writes of `jiraf.toml` / `.env`, its commands |
| `src/api.rs` | dashboard commands, icon proxy, external links |
| `src/jira.rs` | Jira REST client, searches, issue mapping |
| `src/logging.rs` | `jiraf.log` in the config folder |
| `static/` | `index.html` (markup + all CSS), `app.js` (dashboard), `settings.js` (Settings screen) |
| `tauri.conf.json`, `capabilities/` | window, CSP, bundle, permissions |
| `.github/workflows/build.yml` | CI: tests + clippy on macOS and Windows; bundles for `main` and `v*` tags (attached to the release) |
| `app-icons/` | bundle icons (not `icons/`, which is the git-ignored Jira icon cache) |
| `docs/superpowers/` | designs (`specs/`) and implementation plans (`plans/`) |

## Commands

```sh
cargo nextest run                           # all Rust tests (cargo-nextest; not `cargo test`)
cargo nextest run settings::                # a subset, by test-name filter
cargo clippy --all-targets                  # must stay warning-free
node --check static/app.js && node --check static/settings.js
JIRAF_CONFIG_DIR=$PWD cargo tauri dev       # run against the repo's own jiraf.toml / .env
cargo tauri build --bundles app             # → target/release/bundle/macos/jiraf.app
cargo tauri icon app-icons/jiraf-icon-1024.png -o app-icons   # regenerate icons (then drop android/ios)
```

Use `cargo nextest run`, never `cargo test` (install it with `cargo install cargo-nextest --locked`).
Each test runs in its own process. There are no doctests (jiraf is a binary crate), so nextest covers
every test. Run tests with `JIRA_TOKEN` unset: a token in the environment wins over `.env`, and some
tests skip themselves when it is set.

## Never

- Read, print, copy or commit `.env` (holds `JIRA_TOKEN`), `jiraf.toml` (real Jira data), `/icons`,
  `/mockup` (real data) or `jiraf.log`. They are git-ignored on purpose. To test against real Jira,
  copy `jiraf.toml` to a scratch folder and pass the token through the environment only.
- Send the token to the page, a log line or an error message.
- Open a network port, add npm or a bundler, or load remote scripts/styles: the page stays plain files
  under a strict CSP (`tauri.conf.json`).
- Let the page open arbitrary URLs: `open_external` only opens links under the configured Jira URL.
- Weaken the icon proxy's path whitelist (`check_icon_path`) or issue-key checks (`check_key`).
- Use macOS-only APIs without a Windows path.

## Conventions

- Rust 2024 edition (let-chains are fine). Rustdoc on every item, an `# Errors` section on fallible
  functions, terse comments that say why. Match the surrounding code.
- Test-first for Rust logic: write the failing test, watch it fail (`cargo nextest run <name>`), then
  implement. Pure logic goes in `#[cfg(test)]` modules next to the code; Jira calls are tested
  against a local fake (see `settings.rs` tests).
- File writes in the config folder are atomic (`.tmp` + rename) and keep permissions; `.env` is
  owner-only and only its `JIRA_TOKEN` line is ever rewritten.
- Bubble and tab ids are stable: never change an existing id (it is in the URL hash and in `exclude`).
  Reserved: bubble `*`, tabs `att` and `other`.
- Page code: no framework; `esc()` everything that comes from Jira or config before putting it in HTML.
  Both themes use the CSS variables in `:root`.
- Commits: one topic each, imperative summary saying what changes for the user. Larger work goes
  through a design in `docs/superpowers/specs/` and a plan in `docs/superpowers/plans/`.
- Update `README.md` and `CHANGELOG.md` (Unreleased) when behaviour or setup changes.
