# jiraf as a native Tauri app: design

Date: 2026-10-19 · Branch: `tauri`

## Intent

jiraf today is an actix-web server started from the terminal (`cargo run`) plus a browser tab on
`http://127.0.0.1:8080`. The user wants it to behave like any other Mac app: click it in the Dock or
Launchpad, a window opens on the dashboard. No terminal, no browser.

This is a change of shell, not of features. Everything the dashboard does today stays the same:
bubbles, tabs, the tree, inline edits written straight to Jira, the icon cache, the "stateless,
nothing stored except config" principle.

Users: the author, on their own Mac, for now. Sharing with other managers may come later, so nothing
in this design should make that harder, but nothing is built for it yet.

## Goals

1. A `.app` built locally with the Tauri CLI. Launching it opens one window showing the dashboard.
   Closing the window quits the app.
2. The Rust backend talks to the page through Tauri commands, not HTTP. **No port is opened.**
   actix-web and the `local_only` middleware / `X-Jiraf` header are removed.
3. Config (`jiraf.toml`) and secret (`.env` with `JIRA_TOKEN`) live in the app's config folder under
   `~/Library/Application Support/` and are still edited by hand.
4. Startup failures (bad config, missing/expired token, Jira unreachable, unknown field id) show an
   error screen in the window with the message, a **Retry** button and an **Open config folder**
   button.
5. Config edits made while the app runs take effect by restarting the app or with a reload action
   (see "Reload").
6. External links (issue keys, label chips, JQL ↗, "All comments in Jira ↗") open in the default
   browser.

## Non-goals (for now)

- Windows / Linux builds. macOS (Apple Silicon) only for now,
  **but nothing may block a later Windows build**: no macOS-only APIs or hard-coded macOS
  URLs/paths. A GitHub Action building both the macOS and Windows bundles is planned as follow-up
  work.
- Code signing, notarization, DMG, auto-update, distribution to others.
- A settings screen. A settings menu is planned later; keep config loading isolated so it can plug
  in without a rewrite, but don't build any of it.
- Keychain storage for the token. The `.env` file stays.
- Keeping a browser/server mode alongside the app.
- Tray/menu-bar icon, background service, multiple windows.
- Any new dashboard feature or UI change beyond the error screen.

## Anti-goals (failure even if it "works")

- Opening any local TCP port.
- Storing anything beyond: config files, the icon cache, a log file, and window size/position.
- Breaking an existing `jiraf.toml`: the user's current file must load unchanged.
- Introducing a JS bundler / npm toolchain. The frontend stays plain `static/index.html` +
  `static/app.js`.
- Weakening the current safety posture: CSP stays strict, the icon proxy keeps its path whitelist,
  issue keys keep `check_key`, and the page can't make the app open arbitrary URLs.

## Constraints

- Tauri 2, Rust edition 2024, Rust ≥ 1.88 (let-chains). Keep `reqwest`, `tokio`, `serde`, `toml`,
  `chrono`, `anyhow`, `dotenvy`.
- No npm: use `app.withGlobalTauri = true` and call `window.__TAURI__.core.invoke` from `app.js`.
  The Tauri CLI is installed with `cargo install tauri-cli` (version 2).
- Existing unit tests (`cargo test`, 11 test blocks across config, mapping, JQL helpers, key checks)
  must keep passing; adapt them only where signatures change.
- Match the codebase's style: rustdoc on every item, `# Errors` sections, terse comments.

## How it works

### Project layout (my call)

Keep the single crate at the repo root rather than moving it into `src-tauri/`; the history stays
readable and the diff small. Add at the root: `tauri.conf.json`, `build.rs`
(`tauri_build::build()`), `capabilities/default.json`. App icons for the bundle go in `app-icons/`
(set via `bundle.icon`, including `icon.ico` for a later Windows build), **not** `icons/`, because
`/icons` is today's git-ignored Jira icon cache.

Bundle identifier: `com.dmbuil.jiraf` (my call). Product name: `jiraf`. `frontendDist`: `static`.

### Config folder

- Location: Tauri's `app_config_dir()` → `~/Library/Application Support/com.dmbuil.jiraf/`.
- Dev override (my call): if the env var `JIRAF_CONFIG_DIR` is set, use it instead. This lets
  `cargo tauri dev` run against the repo's own `jiraf.toml` / `.env`.
- Contents: `jiraf.toml`, `.env`, `icons/` (the cache), `jiraf.log`.
- First launch: if the folder doesn't exist, create it and write `jiraf.example.toml` and
  `.env.example` (both compiled in with `include_str!`). Never write `jiraf.toml` or `.env`
  themselves. Startup then fails with a "copy jiraf.example.toml to jiraf.toml and .env.example to
  .env and fill them in" error screen.
- `[server] icon_dir`: a relative path resolves against the config folder; absolute paths are used
  as-is. Default stays `icons`.
- `[server] bind`: still accepted so existing files load, but ignored. If present, log a warning
  ("bind is ignored by the app"). Update its rustdoc and `jiraf.example.toml` accordingly.

### Token loading

Today `Config::load` reads `JIRA_TOKEN` from the process environment after `dotenvy::dotenv()`. That
breaks Retry/Reload: dotenvy never overrides a variable already set, so an edited `.env` would be
ignored. Instead, parse `<config dir>/.env` on every load (e.g. `dotenvy::from_path_iter`) and pass
the token into config loading explicitly. A `JIRA_TOKEN` in the real process environment, if
present, wins (keeps `cargo tauri dev` convenient). Same error messages as today ("JIRA_TOKEN is not
set…", "JIRA_TOKEN is empty"), but naming the full path of the `.env` file.

### Startup and app state

Startup work that is in `main()` today (load config, build `Jira`, `GET /rest/api/2/myself`, resolve
the Extended Status field name, build `AppState`) moves into one function, e.g.
`async fn init(config_dir: &Path) -> anyhow::Result<AppState>`. This is the seam a settings screen
will reuse later.

The window opens immediately (showing the page's existing loading state) and `init` runs in the
background. Managed state is a holder such as `RwLock<Phase>` where `Phase` is
`Starting | Ready(Arc<AppState>) | Failed(String)`.

Commands for the page:

- `app_status()` →
  `{ "state": "starting" | "ready" | "failed", "error"?: string, "configDir": string }`. The page
  calls this first; while `starting`, it waits (polling or a Tauri event, builder's choice).
- `retry()` → runs `init` again, swaps the result into the holder, returns the new status.
- `open_config_dir()` → reveals the config folder in Finder.

Every dashboard command takes the `Arc<AppState>` out of the holder and fails with "jiraf is not
ready" if the phase isn't `Ready`.

### Dashboard commands

Each actix handler in `src/api.rs` becomes a `#[tauri::command]` with the same logic and validation
(`check_key`, length limits, digit-only transition ids, cache invalidation in `after_write`):

| Today                                | Command (args)                    |
| ------------------------------------ | --------------------------------- |
| `GET /api/data[?refresh=1]`          | `data(refresh: bool)`             |
| `GET /api/issue/{key}/transitions`   | `transitions(key)`                |
| `GET /api/issue/{key}/priorities`    | `priorities(key)`                 |
| `GET /api/issue/{key}/assignable?q=` | `assignable(key, q)`              |
| `POST …/ext` `{value}`               | `set_ext(key, value)`             |
| `POST …/priority` `{name}`           | `set_priority(key, name)`         |
| `POST …/assignee` `{name}`           | `set_assignee(key, name: Option)` |
| `POST …/target` `{date}`             | `set_target(key, date: Option)`   |
| `POST …/due` `{date}`                | `set_due(key, date: Option)`      |
| `POST …/transition` `{id}`           | `transition(key, id)`             |

Errors: commands return `Result<T, String>` (or a serializable error type) carrying the same message
text the page shows today (`ApiError`'s message, Jira's own message for `JiraError`). HTTP status
codes go away; keep logging each error with `log::warn!` as `error_response` does now.

`index` and `app_js` handlers go away; Tauri serves `static/`.

### Page changes (`static/app.js`, `static/index.html`)

- `api()` becomes a thin wrapper over `invoke(command, args)` that throws `Error(message)` on
  failure, so callers' `try/catch` and inline error rendering keep working. Call sites switch from
  URL paths to command names.
- New error screen in `index.html`, hidden by default: the mascot, the message, the config folder
  path, **Retry** and **Open config folder** buttons. Shown when `app_status()` is `failed`. Retry
  shows the loading state, calls `retry()`, then either loads the dashboard or shows the new error.
  Mid-session errors (Jira down, a failed edit) keep using today's red dot and inline messages.
- Theme toggle (localStorage) and hash routing (`#p=…&t=…`) are unchanged.

### Icons

The `/icon?u=<jira path>` proxy becomes a custom URI scheme registered with
`register_asynchronous_uri_scheme_protocol("jiraf", …)`. The origin differs per platform:
`jiraf://localhost` on macOS/Linux, `http://jiraf.localhost` on Windows (WebView2), chosen at
compile time. `icon_url` / `local_avatar` produce `<origin>/icon?u=<path>`. The handler reuses
`check_icon_path`, `ICON_PREFIXES`, the memory → disk → Jira lookup, and returns the cached content
type plus `Cache-Control: max-age=86400` as today.

### CSP

Moves from the `index` handler into `tauri.conf.json` (`app.security.csp`), as strict as today, with
only the additions Tauri needs: IPC (`connect-src ipc: http://ipc.localhost`) and the icon scheme in
`img-src` (`jiraf:`). Still no remote origins.

### External links

All `target="_blank"` links point at the Jira base URL. A click handler in `app.js` (event
delegation on `a[target=_blank]`) prevents the default and calls an `open_external(url)` command. My
call: that command opens the URL with the opener plugin's Rust API **only if** it starts with the
configured Jira base URL followed by `/`. Anything else is refused and logged. The JS opener API is
not exposed to the page, so the page can't open arbitrary URLs.

### Reload

The user is fine with either "restart the app" or "a reload button". My call: add a
**Reload config** item (⌘⇧R) to the app's native menu. It runs the same path as `retry()`. On
success the page reloads its data; on failure the page shows the error screen (the config is broken
anyway). The ↻ button in the page keeps meaning "refetch from Jira" (`data(refresh: true)`).

### Window

One window titled `jiraf`, sensible default size (e.g. 1400×900), resizable. On Windows the same
item lands in the window's menu bar and ⌘⇧R reads Ctrl+Shift+R. Remember size and position between
launches with `tauri-plugin-window-state` (my call). Closing the window quits the app. Keep the
default macOS app menu (Edit menu needed for copy/paste in inputs) plus Reload config.

### Logging

Replace stderr logging with a log file `jiraf.log` in the config folder (e.g. `tauri-plugin-log`
with a folder target, or `env_logger` writing to the file). Default filter `info`. `RUST_LOG` read
from the real environment or from `.env` sets the level, since apps launched from Finder have no
shell environment.

### Build and docs

- Dev: `JIRAF_CONFIG_DIR=$PWD cargo tauri dev`.
- Release: `cargo tauri build --bundles app` → `target/release/bundle/macos/jiraf.app`; drag into
  Applications.
- README: replace the server-oriented Install/Run sections with the app flow, the config folder
  location, how to move an existing `jiraf.toml` / `.env` there, and the log file location. Update
  the module docs in `main.rs` / `api.rs` (the route table becomes a command table).
- `.gitignore`: add Tauri's generated `gen/` folder if the CLI creates one.
- The `jiraf path/to/jiraf.toml` command-line argument goes away (my call); `JIRAF_CONFIG_DIR`
  replaces it.

### Tests

Besides keeping the existing tests green, add unit tests for the new pure logic (my call): the
`open_external` URL check (base URL + `/` prefix, rejects look-alike hosts such as
`https://jira.example.com.evil`), `icon_dir` resolution (relative vs absolute), and `.env` token
parsing (file value, process env override, missing/empty token messages).

## Left to the builder

- How `starting` is signalled to the page (polling `app_status` vs an emitted event).
- Exact module split (e.g. keep `api.rs` for commands, add `icons.rs`/`app.rs`), as long as each
  module keeps a rustdoc header.
- The error screen's exact markup and styling, consistent with the existing page and both themes.
- Choice between `tauri-plugin-log` and `env_logger` for the file log.
- App icon artwork: derive from `docs/jiraf.png`.
