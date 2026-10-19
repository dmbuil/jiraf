# jiraf as a Tauri App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn jiraf from an actix-web server + browser tab into a native macOS Tauri 2 app that
opens no port.

**Architecture:** The single crate at the repo root becomes a Tauri app. Startup work moves into
`app::init`, whose result lives in a managed `Holder` (`Starting | Ready | Failed`). Each actix
handler becomes a `#[tauri::command]`, Jira icons are served by a `jiraf://` URI scheme, and the
page calls the backend through `window.__TAURI__.core.invoke`. Config and `.env` live in the app
config folder and are re-read on Retry and Reload.

**Tech Stack:** Rust 2024 (≥ 1.88), Tauri 2 (`tauri`, `tauri-build`, `tauri-plugin-opener`,
`tauri-plugin-window-state`), reqwest, tokio, serde, toml, dotenvy, env_logger; plain HTML/JS (no
npm).

**Spec:** `docs/superpowers/specs/2026-10-19-tauri-migration-design.md`

## Global Constraints

- No local TCP port is opened. `actix-web` is removed from `Cargo.toml` by the end of Task 3.
- No npm / bundler. The frontend stays `static/index.html` + `static/app.js`; use
  `app.withGlobalTauri = true`.
- macOS (Apple Silicon) only for now; no signing, notarization, DMG or updater. Nothing may block a
  later Windows build: no macOS-only APIs, no hard-coded `jiraf://` origin (see Task 4), and
  `icon.ico` in the bundle icons.
- Bundle identifier `com.dmbuil.jiraf`, product name `jiraf`, `frontendDist` = `static`.
- Config folder: `$JIRAF_CONFIG_DIR` if set, else Tauri's `app_config_dir()`
  (`~/Library/Application Support/com.dmbuil.jiraf/`). It holds `jiraf.toml`, `.env`, `icons/`,
  `jiraf.log`.
- An existing `jiraf.toml` (including `[server] bind`) must load unchanged.
- Keep the safety checks: `check_key`, `check_icon_path`/`ICON_PREFIXES`, the `ext_max_len` limit,
  digit-only transition ids, a strict CSP with no remote origins.
- Style: rustdoc on every item, `# Errors` sections, terse comments, as in the current code.
- `cargo test` stays green after every task. `cargo clippy --all-targets` adds no new warnings.

## Review Focus

1. **An edited `.env` must be picked up on Retry/Reload.** `dotenvy` never overrides a variable that
   is already set, so the token must be read from the file on each load (Task 1,
   `token_from_edited_env_file_wins_on_reload`).
2. **Commands called while starting or after a failure** must return "jiraf is not ready", never
   panic (Task 2, `not_ready_until_ready`).
3. **Icon paths containing `?`, `&`, `=` or `%`** must survive the `icon_url` → `jiraf://` →
   `icon_query` round trip (Task 4, `icon_query_round_trip`).
4. **Look-alike hosts in links** (`https://jira.x.evil/…`, `https://jira.x@evil/…`) must not open
   (Task 6, `jira_links_only`).
5. **First launch must never overwrite** an existing `jiraf.toml` or `.env` (Task 1,
   `ensure_dir_keeps_existing_files`).
6. **Windows icon origin**: icon URLs must not hard-code `jiraf://` (Task 4,
   `icon_origin_per_platform`).

---

### Task 1: Config folder and token loading

**Files:**

- Modify: `src/config.rs` (`Server` docs, `Config::load`, new helpers, tests)
- Modify: `src/main.rs` (call the new `load`; still actix in this task)
- Modify: `Cargo.toml` (`[dev-dependencies] tempfile = "3"`)
- Modify: `jiraf.example.toml` (`bind` comment: "ignored by the app")

**Interfaces:**

- Produces:
  - `pub const CONFIG_FILE: &str = "jiraf.toml"`, `pub const ENV_FILE: &str = ".env"`
  - `pub fn env_file_var(env_file: &Path, name: &str) -> Result<Option<String>>`: the value of
    `name` in a dotenv file. `Ok(None)` if the file or variable is missing. Uses
    `dotenvy::from_path_iter`, never touches the process environment.
  - `pub fn read_token(env_file: &Path) -> Result<String>`: process `JIRA_TOKEN` if set and
    non-empty, else `env_file_var`. Errors: `"JIRA_TOKEN is not set (put it in <full path>)"` and
    `"JIRA_TOKEN is empty in <full path>"`.
  - `pub fn resolve_icon_dir(dir: &Path, icon_dir: &str) -> PathBuf`: absolute paths as-is, relative
    ones joined to `dir`.
  - `pub fn ensure_dir(dir: &Path) -> Result<()>`: creates `dir` if missing, and writes
    `jiraf.example.toml` and `.env.example` (`include_str!` of the repo files) only when they don't
    exist. Never writes `jiraf.toml` or `.env`.
  - `Config::load(dir: &Path) -> Result<Config>`: reads `dir/jiraf.toml` and `read_token(dir/.env)`.
    When `jiraf.toml` is missing:
    `"<full path> not found: copy jiraf.example.toml to jiraf.toml and .env.example to .env in <dir>, then fill them in"`.
    Logs `warn!("[server] bind is ignored by the app")` when `bind` was present in the file (make
    `Server::bind` an `Option<String>` with no default).
  - `Config.icon_dir: PathBuf` (resolved, `#[serde(skip)]`), set by `load`.
- [ ] **Step 1: Write the failing tests** in `config.rs` `mod tests`, using `tempfile::tempdir()`:
  - `token_from_edited_env_file_wins_on_reload`: write `.env` with `JIRA_TOKEN=a` → `read_token` is
    `"a"`. Rewrite it with `JIRA_TOKEN=b` → `"b"`. Guard with
    `if std::env::var("JIRA_TOKEN").is_ok() { return; }`.
  - `token_missing_or_empty_names_the_file`: no `.env` → the error contains `"is not set"` and the
    `.env` path. `JIRA_TOKEN=` → the error contains `"is empty"`.
  - `icon_dir_relative_and_absolute`: `resolve_icon_dir(Path::new("/c"), "icons") == "/c/icons"`,
    and `resolve_icon_dir(Path::new("/c"), "/tmp/i") == "/tmp/i"`.
  - `ensure_dir_keeps_existing_files`: in a fresh dir, `ensure_dir` creates both example files.
    Pre-write `jiraf.toml` = `"mine"` and `.env` = `"JIRA_TOKEN=x"`, call `ensure_dir` again, and
    both are unchanged.
  - `missing_config_says_what_to_copy`: `Config::load(empty dir)` errors with text containing
    `"jiraf.example.toml"` and the dir path.
  - `loads_example_with_bind`: copy `jiraf.example.toml` (with an uncommented
    `bind = "127.0.0.1:8080"`) plus a `.env` into a temp dir, and `Config::load` returns `Ok`.
- [ ] **Step 2: Run** `cargo test config::` and expect compile errors or FAILs for the new tests.
- [ ] **Step 3: Implement** the interfaces above. `main.rs`:
      `let dir = std::env::current_dir()?; config::ensure_dir(&dir)?; let cfg = config::Config::load(&dir)?;`,
      with the CLI path argument removed. Drop `dotenvy::dotenv()` from `main.rs`.
      `AppState.icon_dir` is filled from `cfg.icon_dir`.
- [ ] **Step 4: Run** `cargo test` and expect all PASS. Run `cargo run` from the repo root and check
      the dashboard still loads at `http://127.0.0.1:8080`.
- [ ] **Step 5: Commit** with
      `git commit -am "Config folder: load jiraf.toml and .env from a directory, re-read the token each time"`.

### Task 2: `init` and startup phases

**Files:**

- Create: `src/app.rs` (module rustdoc: startup and the state holder)
- Modify: `src/main.rs` (actix `main` uses `app::init`), `src/api.rs` (`AppState` stays here)

**Interfaces:**

- Consumes: `Config::load(dir)`, `config::ensure_dir(dir)` (Task 1).
- Produces:
  - `pub async fn init(dir: &Path) -> anyhow::Result<api::AppState>`: `ensure_dir`, `Config::load`,
    `Jira::new`, `GET /rest/api/2/myself` (context `"checking JIRA_TOKEN against Jira"`), and
    `field_names` → `ext_field_name`. These are the same checks `main` does today.
  - `pub enum Phase { Starting, Ready(Arc<api::AppState>), Failed(String) }`
  - `pub struct Holder { pub dir: PathBuf, pub phase: std::sync::RwLock<Phase> }` with
    `Holder::new(dir) -> Self` (phase `Starting`)
  - `impl Holder { pub fn ready(&self) -> Result<Arc<api::AppState>, api::ApiError>; pub fn set(&self, r: anyhow::Result<api::AppState>); pub fn status(&self) -> Status }`.
    `ready` errors with `"jiraf is not ready"`. `set` stores `Ready` or `Failed(format!("{e:#}"))`
    and logs failures with `log::error!`.
  - `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct Status { pub state: &'static str /* "starting" | "ready" | "failed" */, pub error: Option<String>, pub config_dir: String }`
  - `api::ApiError::msg(s: impl Into<String>) -> ApiError` (a plain message error, mapped like
    `bad`) if `ready` needs it.
- [ ] **Step 1: Write the failing test** `not_ready_until_ready` in `app.rs`. A `Holder::new("/x")`
      has `status().state == "starting"` and `ready()` errors with `"jiraf is not ready"`. After
      `set(Err(anyhow!("boom")))`, the state is `"failed"`, `error == Some("boom")` and `ready()`
      still errors. `config_dir == "/x"`.
- [ ] **Step 2: Run** `cargo test app::` and expect FAIL (does not compile).
- [ ] **Step 3: Implement** `app.rs` as above. `main.rs` (still actix):
      `let st = app::init(&std::env::current_dir()?).await?;`, then serve as before.
- [ ] **Step 4: Run** `cargo test` and expect PASS. `cargo run` still serves the dashboard.
- [ ] **Step 5: Commit** with `git commit -am "Move startup checks into app::init behind a Holder"`.

### Task 3: Tauri shell and commands

This replaces actix in one go: the page, the commands and the build config change together.

**Files:**

- Modify: `Cargo.toml`: remove `actix-web`, `futures-util` only if unused after the change. Add
  `tauri = { version = "2", features = [] }`, `tauri-plugin-opener = "2"`,
  `[build-dependencies] tauri-build = { version = "2", features = [] }`. Keep `tokio`.
- Create: `build.rs` (`fn main() { tauri_build::build() }`), `tauri.conf.json`,
  `capabilities/default.json`, `app-icons/` (generated)
- Modify: `src/main.rs`, `src/api.rs`, `static/app.js`, `.gitignore`

**Interfaces:**

- Consumes: `app::{init, Holder, Status}` (Task 2).
- Produces commands, all `async`, each taking `holder: tauri::State<'_, app::Holder>` first and
  returning `api::ApiResult<T>`: `data(refresh: bool) -> Value`,
  `transitions(key: String) -> Value`, `priorities(key: String) -> Vec<String>`,
  `assignable(key: String, q: String) -> Vec<Value>`, `set_ext(key, value: String) -> Issue`,
  `set_priority(key, name: Option<String>) -> Issue`,
  `set_assignee(key, name: Option<String>) -> Issue`,
  `set_target(key, date: Option<String>) -> Issue`, `set_due(key, date: Option<String>) -> Issue`,
  `transition(key, id: String) -> Issue`.
- `impl serde::Serialize for ApiError` serializes to the message string: Jira's `JiraError` text,
  else `format!("{:#}")`. `ResponseError`, `HttpResponse`, `index`, `app_js`, `CSP` and the
  `*Body`/`*Q` structs are removed. `bad`/`BadRequest` stay. `ApiResult<T = Value>`.
- JS: `api(cmd, args = {})` resolves with the command result and rejects with `Error(message)`.
- [ ] **Step 1: Install the CLI** with `cargo install tauri-cli --version "^2" --locked`. Expected:
      `cargo tauri --version` prints `tauri-cli 2.x`.
- [ ] **Step 2: Generate icons** with `cargo tauri icon docs/jiraf.png -o app-icons`. Expected:
      `app-icons/icon.icns` and `app-icons/icon.ico` exist.
- [ ] **Step 3: Write `tauri.conf.json`** with: `productName "jiraf"`,
      `identifier "com.dmbuil.jiraf"`, `build.frontendDist "static"` (no `devUrl`),
      `app.withGlobalTauri true`, one window
      `{ "label": "main", "title": "jiraf", "width": 1400, "height": 900, "resizable": true }`, and
      `bundle { "active": true, "targets": ["app"], "icon": ["app-icons/32x32.png","app-icons/128x128.png","app-icons/128x128@2x.png","app-icons/icon.icns","app-icons/icon.ico"] }`.
      Set `app.security.csp` to exactly:
      `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: jiraf: http://jiraf.localhost; connect-src ipc: http://ipc.localhost; frame-ancestors 'none'`
      `capabilities/default.json`:
      `{ "identifier": "default", "windows": ["main"], "permissions": ["core:default"] }`. Do
      **not** grant `opener:*` to the page.
- [ ] **Step 4: Convert the handlers** in `api.rs` to the commands above. Bodies keep their
      validation and Jira calls. Each starts with `let st = holder.ready()?;`. `data` keeps the TTL
      cache logic, and returns the cached `Value` cloned. Update the module rustdoc's route table
      into a command table.
- [ ] **Step 5: Rewrite `main.rs`** as a plain `fn main()`:
      `tauri::Builder::default().plugin(tauri_plugin_opener::init()).setup(|app| { … })`. `setup`
      resolves the dir (`JIRAF_CONFIG_DIR` or `app.path().app_config_dir()?`),
      `app.manage(Holder::new(dir))`, and spawns
      `tauri::async_runtime::spawn(async move { let r = app::init(&dir).await; handle.state::<Holder>().set(r); })`.
      Then `.invoke_handler(tauri::generate_handler![…all ten…])` and
      `.run(tauri::generate_context!())`. Delete `local_only`. Keep `env_logger` to stderr for now
      (Task 7 moves it). Update the crate rustdoc.
- [ ] **Step 6: Update `app.js`.** `api()` becomes
  `(cmd, args = {}) => window.__TAURI__.core.invoke(cmd, args).catch(e => { throw new Error(typeof e === 'string' ? e : e?.message || String(e)); })`.
  Call sites:
  - `load`: `api('data', { refresh: !!refresh })`
  - transitions/priorities/assignable menus: `api('transitions', { key: i.k })` etc., and assignable
    adds `q`
  - `save(i, field, path, body, …)`: command =
    `path === 'transition' ? 'transition' : 'set_' + path`, args = `{ key: i.k, ...body }`.
    Assignee/priority bodies are already `{ name }`, dates `{ date }`, ext `{ value }`, transition
    `{ id }`.
- [ ] **Step 7: `.gitignore`**: add `/gen/`.
- [ ] **Step 8: Run** `cargo test` (PASS, with the `icon_urls` test unchanged for now), then
      `JIRAF_CONFIG_DIR=$PWD cargo tauri dev`. Expected: a window opens, the dashboard loads from
      Jira, and editing Extended Status on one issue saves. Icons are broken until Task 4
      (expected). `lsof -nP -iTCP -sTCP:LISTEN -p $(pgrep -n jiraf)` lists nothing.
- [ ] **Step 9: Commit** with
      `git add -A && git commit -m "Tauri shell: commands replace the actix server"`.

### Task 4: Icon URI scheme

**Files:**

- Modify: `src/api.rs` (`icon_url`, the new `icon_query`, the scheme handler replacing `icon`),
  `src/main.rs` (register the scheme)

**Interfaces:**

- Consumes: `Holder::ready` (Task 2), `get_icon`, `check_icon_path` (existing).
- Produces:
  - `pub const ICON_ORIGIN: &str`: `"http://jiraf.localhost"` under `#[cfg(windows)]`, else
    `"jiraf://localhost"` (how Tauri exposes custom schemes per platform). `icon_url(base, url)`
    returns `format!("{ICON_ORIGIN}/icon?u={}", pct(path))` for Jira-hosted URLs (other hosts
    unchanged, as today).
  - `fn icon_query(uri: &str) -> Option<String>`: parse with `reqwest::Url::parse`, require path
    `/icon`, and return the decoded `u` query value.
  - `pub async fn icon_response(holder: &app::Holder, uri: &str) -> tauri::http::Response<Vec<u8>>`:
    200 with the content type and `Cache-Control: max-age=86400`. 400 if `icon_query` is `None` or
    `check_icon_path` fails, 503 if not ready, 502 if Jira fails (body = message, logged with
    `warn!`).
- [ ] **Step 1: Write the failing tests.** Update `icon_urls` to expect
      `format!("{ICON_ORIGIN}/icon?u=/secure/viewavatar%3Fsize%3Dxsmall%26avatarId%3D1")`, and add
      `icon_origin_per_platform`: `ICON_ORIGIN == "http://jiraf.localhost"` under `cfg!(windows)`,
      else `"jiraf://localhost"`. Add `icon_query_round_trip`: for
      `"/secure/viewavatar?size=xsmall&avatarId=1"` and `"/images/icons/a b%20c.png"`,
      `icon_query(&icon_url(b, &(b.to_string() + p))) == Some(p)`. Also
      `icon_query("jiraf://localhost/other?u=/images/x") == None`, and
      `icon_query("http://jiraf.localhost/icon?u=/images/x") == Some("/images/x")` (the Windows form
      parses too).
- [ ] **Step 2: Run** `cargo test api::` and expect FAIL.
- [ ] **Step 3: Implement** the interfaces. In `main.rs`:
      `.register_asynchronous_uri_scheme_protocol("jiraf", |ctx, req, responder| { let app = ctx.app_handle().clone(); let uri = req.uri().to_string(); tauri::async_runtime::spawn(async move { responder.respond(api::icon_response(&app.state::<app::Holder>(), &uri).await) }); })`.
- [ ] **Step 4: Run** `cargo test` and expect PASS. `JIRAF_CONFIG_DIR=$PWD cargo tauri dev`: issue
      type, priority and avatar icons show. The CSP reports no violations in the devtools console.
- [ ] **Step 5: Commit** with `git commit -am "Serve Jira icons through a jiraf:// scheme"`.

### Task 5: Startup error screen and Retry

**Files:**

- Modify: `src/api.rs` or `src/app.rs` (three commands), `src/main.rs` (register them),
  `static/index.html` (markup + CSS for both themes), `static/app.js` (boot flow)
- Create: `static/jiraf.png` (copy of `docs/jiraf.png`, shown on the error screen)

**Interfaces:**

- Consumes: `Holder`, `Status`, `init` (Task 2).
- Produces commands:
  - `app_status(holder) -> Status`
  - `retry(holder) -> Status`: sets `Starting`, awaits `init(&holder.dir)`, then `set`, then returns
    the status.
  - `open_config_dir(app: tauri::AppHandle, holder) -> Result<(), String>`: uses
    `tauri_plugin_opener::OpenerExt`, `app.opener().open_path(dir, None::<&str>)`.
- JS: `boot()` replaces the bare `load(false)` at the end of `app.js`. It polls `app_status` every
  250 ms while `starting` (showing today's "Loading from Jira…" state), calls `load(false)` on
  `ready`, and calls `showStartupError(status)` on `failed`. `showStartupError` shows `#startup`
  (hidden by default) with `jiraf.png`, the error text (rendered with `esc`,
  `white-space: pre-wrap`), the `configDir` path, and the buttons `#retry` (shows the loading state,
  `api('retry')`, then same branching, with `location.reload()` on ready) and `#openDir`
  (`api('open_config_dir')`). While shown, `main` and the top bar's dashboard controls are hidden.
- [ ] **Step 1: Implement** the commands and register them. There's no Rust unit test beyond Task
      2's (it's a thin wrapper over `Holder`).
- [ ] **Step 2: Implement** the markup, CSS and JS above.
- [ ] **Step 3: Manual check** with a scratch config dir `D=$(mktemp -d)` and
  `JIRAF_CONFIG_DIR=$D cargo tauri dev`. Each must hold:
  - The first launch shows the error screen naming `jiraf.example.toml`, and `$D` now holds the two
    example files.
  - "Open config folder" opens `$D` in Finder.
  - Copy your real `jiraf.toml` and a `.env` with `JIRA_TOKEN=bad`, then Retry: the screen shows
    Jira's 401 message.
  - Fix the token, then Retry: the dashboard loads without restarting.
- [ ] **Step 4: Commit** with
      `git add -A && git commit -m "Startup error screen with Retry and Open config folder"`.

### Task 6: External links

**Files:**

- Modify: `src/api.rs` (command and pure check), `src/main.rs` (register), `static/app.js` (click
  delegation)

**Interfaces:**

- Produces:
  - `fn jira_link_allowed(base: &str, url: &str) -> bool`: true only when `url` starts with
    `format!("{base}/")`.
  - `open_external(app: tauri::AppHandle, holder, url: String) -> ApiResult<()>`: `holder.ready()?`.
    If the link is allowed, `app.opener().open_url(url, None::<&str>)`. Otherwise `warn!` and
    `Err(bad("only Jira links can be opened"))`.
  - JS: one `document.addEventListener('click', …)` for `a[target=_blank]` that calls
    `preventDefault()` then `api('open_external', { url: a.href })`, showing errors with
    `toast(…, true)`.
- [ ] **Step 1: Write the failing test** `jira_links_only` with `b = "https://jira.x"`: true for
      `"https://jira.x/browse/DV-1"` and `"https://jira.x/issues/?jql=labels%20%3D%20%22a%22"`;
      false for `"https://jira.x"`, `"https://jira.x.evil/browse/DV-1"`, `"https://jira.x@evil/"`,
      `"http://jira.x/browse/DV-1"` and `"javascript:alert(1)"`.
- [ ] **Step 2: Run** `cargo test api::tests::jira_links_only` and expect FAIL.
- [ ] **Step 3: Implement** the interfaces above.
- [ ] **Step 4: Run** `cargo test` and expect PASS. In `cargo tauri dev`, clicking an issue key, a
      label chip, "JQL ↗" and "All comments in Jira ↗" each opens the default browser, and the jiraf
      window doesn't navigate.
- [ ] **Step 5: Commit** with
      `git commit -am "Open Jira links in the default browser, and only Jira links"`.

### Task 7: Reload config menu, window state, file log

**Files:**

- Create: `src/logging.rs`
- Modify: `Cargo.toml` (`tauri-plugin-window-state = "2"`), `src/main.rs`, `static/app.js`

**Interfaces:**

- Consumes: `config::env_file_var` (Task 1), `retry` flow (Task 5).
- Produces:
  - `pub fn log_filter(process: Option<String>, env_file: Option<String>) -> String`: process
    `RUST_LOG`, else the `.env` value, else `"info"`. Empty strings count as unset.
  - `pub fn init(dir: &Path) -> anyhow::Result<()>`: `env_logger` with that filter, `Target::Pipe`
    to `dir/jiraf.log` opened in append mode (create `dir` first). Called first thing in `setup`,
    before `Holder` and `init`.
  - Menu: start from `tauri::menu::Menu::default(app.handle())?` and add
    `MenuItem::with_id(app, "reload-config", "Reload config", true, Some("CmdOrCtrl+Shift+R"))` to
    its first (app) submenu. `on_menu_event`: for id `reload-config`,
    `app.emit("reload-config", ())`.
  - JS: `window.__TAURI__.event.listen('reload-config', () => retryFlow())`, where `retryFlow` is
    the Retry button's handler from Task 5 (reuse it, don't copy it).
  - `.plugin(tauri_plugin_window_state::Builder::default().build())`.
- [ ] **Step 1: Write the failing test** `log_filter_precedence`:
      `(Some("debug"), Some("warn")) → "debug"`, `(None, Some("warn")) → "warn"`,
      `(Some(""), None) → "info"`, `(None, None) → "info"`.
- [ ] **Step 2: Run** `cargo test logging::` and expect FAIL.
- [ ] **Step 3: Implement** the interfaces above.
- [ ] **Step 4: Run** `cargo test` and expect PASS. Then check in `cargo tauri dev`:
  - `jiraf.log` in the config dir gets the `connected to … as …` line.
  - Rename a bubble in `jiraf.toml`, press ⌘⇧R, and the new name shows.
  - Break the TOML, press ⌘⇧R, and the error screen shows.
  - Copy/paste work in the filter box (Edit menu present).
  - Resize, quit, relaunch, and the size is kept.
- [ ] **Step 5: Commit** with
      `git add -A && git commit -m "Reload config menu item, remembered window size, log file"`.

### Task 8: Docs and release build

**Files:**

- Modify: `README.md`, `src/main.rs` / `src/api.rs` / `src/app.rs` module docs (check them),
  `jiraf.example.toml`
- [ ] **Step 1: README.** Replace the server Install/Run sections:
  - Prerequisites: Rust 1.88+ and `cargo install tauri-cli --version "^2" --locked`.
  - Build with `cargo tauri build --bundles app`, which produces
    `target/release/bundle/macos/jiraf.app`; drag it into Applications.
  - The config folder location, and moving an existing `jiraf.toml`/`.env` there.
  - Dev with `JIRAF_CONFIG_DIR=$PWD cargo tauri dev`.
  - The log file location and `RUST_LOG` in `.env`.
  - Reload config (⌘⇧R) vs ↻.
  - The security section: no port; the token stays in the Rust process; only Jira links open.
  - Remove references to `bind`, `X-Jiraf`, the Host check and `cargo run`.
  - Update the troubleshooting table for the error screen.
- [ ] **Step 2: Run** `cargo doc --no-deps --document-private-items` and expect no broken intra-doc
      links. Run `cargo clippy --all-targets` and expect no new warnings. Run `cargo test` and
      expect PASS.
- [ ] **Step 3: Release build** with `cargo tauri build --bundles app`. Expected:
      `target/release/bundle/macos/jiraf.app`. Launch it from Finder with the real config in
      `~/Library/Application Support/com.dmbuil.jiraf/`: the dashboard loads, and closing the window
      quits the app (it disappears from the Dock).
- [ ] **Step 4: Commit** with `git commit -am "README and docs for the Tauri app"`.
