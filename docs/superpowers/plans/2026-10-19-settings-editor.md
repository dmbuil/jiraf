# Settings Screen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Edit every setting in `jiraf.toml` plus the Jira token from a Settings screen inside the
app, so the files never need to be opened; a bubble with bad JQL no longer breaks the dashboard.

**Architecture:** A new `src/settings.rs` owns everything Settings does on the Rust side: offline
validation, Jira checks, clean writes of `jiraf.toml` (with `.bak`) and `.env`, and the three
commands the page calls. `config.rs` gains the pieces it needs (serialize, parse without the token,
ids, a template). `app.rs` learns two new startup outcomes (`setup`, token problem). The page gets a
`#settings` section in `index.html` driven by a new plain `static/settings.js`.

**Tech Stack:** Rust 2024, Tauri 2 commands, serde/toml 1.x, reqwest 0.13, tokio; vanilla JS
(`window.__TAURI__.core.invoke`), no npm.

**Spec:** `docs/superpowers/specs/2026-10-19-settings-editor-design.md`

## Global Constraints

- Rustdoc on every item, `# Errors` sections on fallible fns, terse comments, edition 2024
  (let-chains ok).
- The token never reaches the page: not in `settings`, `app_status`, a save report, an error or a
  log line.
- Every file write is atomic: write `<name>.tmp` in the same folder, then `rename`.
- Before each `jiraf.toml` write, the existing file (if any) is copied to `jiraf.toml.bak`.
- `.env`: only the `JIRA_TOKEN=` line changes (replaced in place, or appended); all other lines stay
  byte-identical.
- A process-environment `JIRA_TOKEN` still wins over `.env` (dev mode).
- Page stays plain `static/index.html` + `static/*.js`; both themes; `CmdOrCtrl+,` for Settings.
- Reserved ids: bubble `*` (the page's "All"); tab `att` and `other`.
- `cargo test` stays green after every task; `cargo clippy --all-targets` adds no warnings.

## Review Focus

1. A JQL with newlines, `"`, `\`, `'` and `'''` in it is saved and re-read byte-identical (Task 1
   test `jql_round_trips_exactly`).
2. A `.env` with comments, `RUST_LOG`, `export JIRA_TOKEN=…`, a quoted token, CRLF line ends or no
   final newline: only the token line changes, and a pasted token's surrounding whitespace is
   trimmed (Task 2 test `env_only_token_line_changes`).
3. Names that slug the same, non-ASCII names ("Evolución Técnica"), a new bubble named like a
   deleted one, or a name that slugs to a reserved id: ids are unique, URL-safe, not reserved, and
   existing ids never change on rename (Task 1 test `ids_are_unique_safe_and_stable`).
4. Deleting a bubble another one excludes: Save is blocked with the message on the excluding bubble,
   not a crash or a silent drop (Task 2 test `validation_blocks_what_load_would_reject`).
5. Saving while Jira is unreachable: files are written, `unverified` is true, no errors (Task 3 test
   `unreachable_jira_does_not_block`).

---

### Task 1: Config model: serialize, parse, ids, template

**Files:**

- Modify: `src/config.rs`

**Interfaces:**

- Produces:
  - `Config`, `Server`, `Jira` derive `Serialize` (and `Config` `Deserialize` from the page as well
    as TOML). `Bubble::exclude` loses `skip_serializing`. `Server::bind` gets `skip_serializing`;
    `Server::icon_dir` gets `skip_serializing_if = "is_default_icon_dir"`.
  - `Config::parse(raw: &str) -> Result<Config>`: TOML → `Config`, URL trailing `/` trimmed, nothing
    else (no token, no exclude resolution, no checks). `Config::load` is rebuilt on top of it,
    behaviour unchanged.
  - `Config::template() -> Config`: first-run defaults. `jira.url = ""`, `cache_seconds = 60`, every
    `fields.*` = `""`, `follow_up` = `FollowUp::default()`, `bubbles = []`,
    `tabs = [Tab { id: "main", name: "Main", activity_types: [], include_unset: true, issues: [] }]`,
    `server = Server::default()`.
  - `render_toml(cfg: &Config) -> Result<String>`: the clean file. First line exactly
    `# Written by jiraf's Settings screen. Edit it there (⌘,) rather than by hand.`, then a blank
    line, then `toml::to_string_pretty(cfg)`. Section order follows the struct field order:
    `[server]` (only if it has a key to write), `[jira]`, `[fields]`, `[follow_up]`, `[[bubbles]]`,
    `[[tabs]]`.
  - `assign_ids(cfg: &mut Config)`: every bubble/tab with an empty `id` gets `slug(name)`, made
    unique within its list by appending `-2`, `-3`…; reserved ids are skipped the same way. Existing
    non-empty ids are never touched.
  - `slug(name: &str) -> String`: lowercase, accents stripped (reuse the page's idea: NFD is not
    available, so map the Latin-1/Latin Extended-A vowels and `ñ`,`ç` by a small table), runs of
    anything not `[a-z0-9]` → `-`, trimmed of `-`; empty → `"item"`.

- [ ] **Step 1: Write the failing tests** (in `config.rs` tests)

```rust
#[test]
fn jql_round_trips_exactly() {
    let mut cfg = Config::template();
    cfg.jira.url = "https://j.example.com".into();
    let jql = "a = \"x\" AND b ~ 'y'\nOR c = \"p\\\\q\"\nOR d = ''' ";
    cfg.bubbles.push(Bubble { id: "b".into(), name: "B".into(), jql: jql.into(), icon_project: None, exclude: vec!["c".into()] });
    let back = Config::parse(&render_toml(&cfg).unwrap()).unwrap();
    assert_eq!(back.bubbles[0].jql, jql);
    assert_eq!(back.bubbles[0].exclude, ["c"]);
}

#[test]
fn clean_file_drops_bind_and_default_icon_dir() {
    let raw = include_str!("../jiraf.example.toml").replacen("[server]", "[server]\nbind = \"127.0.0.1:8080\"", 1);
    let out = render_toml(&Config::parse(&raw).unwrap()).unwrap();
    assert!(out.starts_with("# Written by jiraf's Settings screen."));
    assert!(!out.contains("bind") && !out.contains("icon_dir"), "{out}");
    let mut cfg = Config::parse(&raw).unwrap();
    cfg.server.icon_dir = "/tmp/i".into();
    assert!(render_toml(&cfg).unwrap().contains("icon_dir = \"/tmp/i\""));
}

#[test]
fn ids_are_unique_safe_and_stable() {
    let mut cfg = Config::template();
    let b = |id: &str, name: &str| Bubble { id: id.into(), name: name.into(), jql: "x".into(), icon_project: None, exclude: vec![] };
    cfg.bubbles = vec![b("keep", "Renamed"), b("", "Evolución Técnica"), b("", "evolucion tecnica"), b("", "*"), b("", "Keep")];
    cfg.tabs.push(Tab { id: "".into(), name: "Other".into(), activity_types: vec![], include_unset: false, issues: vec![] });
    cfg.tabs.push(Tab { id: "".into(), name: "ATT".into(), activity_types: vec![], include_unset: false, issues: vec![] });
    assign_ids(&mut cfg);
    let ids: Vec<_> = cfg.bubbles.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["keep", "evolucion-tecnica", "evolucion-tecnica-2", "item", "keep-2"]);
    let tabs: Vec<_> = cfg.tabs.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(tabs, ["main", "other-2", "att-2"]);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --bin jiraf config::`
Expected: compile errors: `parse`, `template`, `render_toml`, `assign_ids` not found.

- [ ] **Step 3: Implement the Interfaces above in `src/config.rs`**

`toml::to_string_pretty` may write multi-line JQL as a `"""` string rather than the spec's `'''`;
the spec's point is an exact round-trip, which `jql_round_trips_exactly` pins. `Config::load`
becomes: missing-file check (unchanged message) → `parse` → token → icon dir → bind warning →
existing checks. Note `"*"` slugs to `""` → `"item"`, which is why the test expects `item`.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test`
Expected: all pass (23 existing + 3 new).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "Config: serialize to a clean file, parse without the token, stable ids, first-run template"
```

---

### Task 2: Settings files and offline validation

**Files:**

- Create: `src/settings.rs` (module rustdoc: "The Settings screen: validation, Jira checks, clean
  writes of jiraf.toml and .env, and its commands.")
- Modify: `src/main.rs` (`mod settings;`)

**Interfaces:**

- Consumes: Task 1's `Config::parse`, `render_toml`, `resolve_excludes` (make it `pub(crate)`).
- Produces:
  - `#[derive(Serialize, Debug, PartialEq)] pub struct Problem { pub at: String, pub message: String }`.
    `at` is a form path: `"jira.url"`, `"token"`, `"fields.<key>"`, `"follow_up.<key>"`,
    `"bubbles"`, `"bubbles.<i>.<name|jql|exclude>"`, `"tabs"`, `"tabs.<i>.<name>"` (i = 0-based
    index in the list as sent).
  - `pub fn validate(cfg: &Config, token_available: bool) -> Vec<Problem>`, blocking problems only,
    each with the exact message:
    - `jira.url`: empty → `"Enter your Jira URL"`; not `http://` or `https://` + host (parse with
      `reqwest::Url`) → `"Not a valid URL"`.
    - `token`: `!token_available` → `"Paste your Jira token"`.
    - `fields.<key>`: empty → `"Required"`.
    - `follow_up.stale_days` / `done_days` < 1 → `"Must be 1 or more"`; `ext_max_len` outside
      1..=255 → `"Between 1 and 255"`; `hierarchy` empty → `"Add at least one issue type"` (at
      `follow_up.hierarchy`); `initiative_type` not in `hierarchy` →
      `"Must be one of the hierarchy levels"`.
    - `bubbles`: none → `"Add at least one bubble"`; `tabs`: none → `"Add at least one tab"`.
    - per bubble: empty name → `"Required"`; empty JQL → `"Required"`; name equal (case-insensitive,
      trimmed) to an earlier bubble's → `"Another bubble already has this name"`; exclude of itself
      → `"A bubble can't exclude itself"`; exclude of an id not in the list →
      `"Excludes a bubble that no longer exists"` (all at `bubbles.<i>.exclude`, one problem per bad
      entry).
    - per tab: empty name → `"Required"`; duplicate name → `"Another tab already has this name"`.
  - `pub fn write_config(dir: &Path, cfg: &Config) -> Result<()>`: `.bak` copy if `jiraf.toml`
    exists, then atomic write of `render_toml(cfg)`.
  - `pub fn write_token(env_file: &Path, token: &str) -> Result<()>`: token trimmed; replace the
    first line whose key (after an optional `export `) is `JIRA_TOKEN` with `JIRA_TOKEN=<token>`,
    keeping that line's original line ending; else append (adding a newline first if the file
    doesn't end in one); create the file if missing. Atomic.
  - `fn atomic_write(path: &Path, body: &str) -> Result<()>` (private helper).
- [ ] **Step 1: Write the failing tests** (in `settings.rs` tests). Helper `valid() -> Config`:
      `Config::template()` with `jira.url = "https://jira.example.com"`, `fields.*` =
      `customfield_1` … `customfield_6` (in struct order), one bubble
      `{ id: "alpha", name: "Alpha", jql: "project = A" }`.

```rust
#[test]
fn env_only_token_line_changes() {
    let d = tempfile::tempdir().unwrap();
    let env = d.path().join(".env");
    std::fs::write(&env, "# mine\r\nRUST_LOG=debug\r\nexport JIRA_TOKEN=\"old\"\r\nOTHER=1").unwrap();
    write_token(&env, "  new  ").unwrap();
    assert_eq!(std::fs::read_to_string(&env).unwrap(), "# mine\r\nRUST_LOG=debug\r\nJIRA_TOKEN=new\r\nOTHER=1");
    std::fs::write(&env, "RUST_LOG=info").unwrap();
    write_token(&env, "t").unwrap();
    assert_eq!(std::fs::read_to_string(&env).unwrap(), "RUST_LOG=info\nJIRA_TOKEN=t\n");
    std::fs::remove_file(&env).unwrap();
    write_token(&env, "t").unwrap();
    assert_eq!(std::fs::read_to_string(&env).unwrap(), "JIRA_TOKEN=t\n");
}

#[test]
fn config_write_keeps_a_backup_and_loads_back() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("jiraf.toml"), "# hand-written\n").unwrap();
    write_config(d.path(), &valid()).unwrap();
    assert_eq!(std::fs::read_to_string(d.path().join("jiraf.toml.bak")).unwrap(), "# hand-written\n");
    let back = Config::parse(&std::fs::read_to_string(d.path().join("jiraf.toml")).unwrap()).unwrap();
    assert_eq!(back.bubbles[0].jql, valid().bubbles[0].jql);
    assert!(!d.path().join("jiraf.toml.tmp").exists());
}

#[test]
fn validation_blocks_what_load_would_reject() {
    assert_eq!(validate(&valid(), true), vec![]);
    let mut c = valid();
    c.jira.url = "jira.example.com".into();
    c.bubbles[0].exclude = vec!["gone".into(), c.bubbles[0].id.clone()];
    c.bubbles.push(c.bubbles[0].clone()); // same name
    c.bubbles[1].id = "b2".into();
    c.bubbles[1].exclude = vec![];
    c.fields.epic_link = "".into();
    let at: Vec<_> = validate(&c, false).into_iter().map(|p| (p.at, p.message)).collect();
    assert!(at.contains(&("jira.url".into(), "Not a valid URL".into())), "{at:?}");
    assert!(at.contains(&("token".into(), "Paste your Jira token".into())));
    assert!(at.contains(&("fields.epic_link".into(), "Required".into())));
    assert!(at.contains(&("bubbles.0.exclude".into(), "Excludes a bubble that no longer exists".into())));
    assert!(at.contains(&("bubbles.0.exclude".into(), "A bubble can't exclude itself".into())));
    assert!(at.contains(&("bubbles.1.name".into(), "Another bubble already has this name".into())));
    let mut e = valid();
    e.bubbles.clear();
    e.tabs.clear();
    let at: Vec<_> = validate(&e, true).into_iter().map(|p| p.at).collect();
    assert_eq!(at, ["bubbles", "tabs"]);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --bin jiraf settings::`
Expected: compile errors: `write_token`, `write_config`, `validate` not found.

- [ ] **Step 3: Implement the Interfaces above in `src/settings.rs`**

- [ ] **Step 4: Run the whole suite**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/settings.rs src/main.rs src/config.rs
git commit -m "Settings: validation, clean jiraf.toml writes with a backup, token-only .env edits"
```

---

### Task 3: Jira checks on Save

**Files:**

- Modify: `src/jira.rs`, `src/settings.rs`, `Cargo.toml` (dev-dependency
  `tokio = { version = "1.53.3", features = ["net", "io-util"] }`)

**Interfaces:**

- Consumes: Task 2's `Problem`; `config::resolve_excludes`.
- Produces:
  - `Jira::with(url: &str, token: &str) -> Result<Jira>`; `Jira::new(cfg)` delegates to it.
  - `Jira::check_jql(&self, jql: &str) -> Result<()>`: `POST /rest/api/2/search` with
    `{"jql": jql, "maxResults": 0, "fields": []}`.
  - `#[derive(Default, Debug)] pub struct JiraCheck { pub errors: Vec<Problem>, pub warnings: Vec<Problem>, pub unverified: bool }`.
  - `pub async fn check_with_jira(cfg: &Config, token: &str) -> JiraCheck`, in order:
    1. `GET /rest/api/2/myself`. A `JiraError` with status 401 or 403 → error
       `("token", "Jira rejected this token")`, stop. Any other failure (transport, timeout, other
       status) → `unverified = true`, stop.
    2. `jira::field_names`. Each `cfg.fields.<key>` not among the ids → error
       `("fields.<key>", "No field with this id in Jira")`. Failure → `unverified = true`, stop.
    3. For each bubble i, `check_jql` on its effective JQL (after `resolve_excludes`,
       `strip_order_by`). A `JiraError` with status 400 → warning
       `("bubbles.<i>.jql", <Jira's message>)`. Other failures → `unverified = true` (keep checking
       the rest).
  - Classify errors with `e.downcast_ref::<JiraError>()`.

- [ ] **Step 1: Write the failing tests** with a fake Jira in `settings.rs` tests

`async fn fake_jira(routes: Vec<(&'static str, u16, &'static str)>) -> String`: binds `127.0.0.1:0`
with `tokio::net::TcpListener`, spawns a loop that reads one request, matches the request line's
path prefix against `routes` (first match; unmatched → 404 `{}`), answers
`HTTP/1.1 <status> X\r\nContent-Type: application/json\r\nContent-Length: <n>\r\nConnection: close\r\n\r\n<body>`,
and returns `http://127.0.0.1:<port>`. For the search route, a body containing `bad` in the JQL is
told apart by reading the request body: answer 400 `{"errorMessages":["Error in the JQL Query"]}`
when it contains `bad`, else 200 `{"issues":[],"total":0}` (give the route the status `0` to mean
"decide by body").

```rust
#[tokio::test]
async fn rejected_token_blocks() {
    let url = fake_jira(vec![("/rest/api/2/myself", 401, "{}")]).await;
    let r = check_with_jira(&with_url(&url), "t").await;
    assert_eq!(r.errors, vec![Problem { at: "token".into(), message: "Jira rejected this token".into() }]);
}

#[tokio::test]
async fn bad_jql_warns_and_unknown_field_blocks() {
    let fields = r#"[{"id":"customfield_1","name":"Ext"}]"#; // only extended_status exists
    let url = fake_jira(vec![("/rest/api/2/myself", 200, r#"{"displayName":"x"}"#), ("/rest/api/2/field", 200, fields), ("/rest/api/2/search", 0, "")]).await;
    let mut c = with_url(&url); // fields.* = customfield_1..6, bubbles "ok" (jql "x") and "broken" (jql "bad")
    c.bubbles[1].exclude = vec![];
    let r = check_with_jira(&c, "t").await;
    assert!(r.errors.iter().any(|p| p.at == "fields.epic_link" && p.message == "No field with this id in Jira"));
    assert!(!r.errors.iter().any(|p| p.at == "fields.extended_status"));
    assert_eq!(r.warnings, vec![Problem { at: "bubbles.1.jql".into(), message: "Error in the JQL Query".into() }]);
    assert!(!r.unverified);
}

#[tokio::test]
async fn unreachable_jira_does_not_block() {
    let r = check_with_jira(&with_url("http://127.0.0.1:9"), "t").await;
    assert!(r.errors.is_empty() && r.warnings.is_empty() && r.unverified);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --bin jiraf settings::`
Expected: compile errors: `check_with_jira`, `JiraCheck` not found.

- [ ] **Step 3: Implement the Interfaces above**

- [ ] **Step 4: Run the whole suite**

Run: `cargo test`
Expected: all pass, and none of the three tests takes more than a few seconds.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/jira.rs src/settings.rs
git commit -m "Settings: check token, field ids and each bubble's JQL with Jira; unreachable Jira doesn't block"
```

---

### Task 4: Startup outcomes and the Settings commands

**Files:**

- Modify: `src/app.rs`, `src/settings.rs`, `src/main.rs`

**Interfaces:**

- Consumes: Tasks 1–3.
- Produces:
  - In `app.rs`: `pub enum Failure { Setup, Token(String), Other(String) }`;
    `init(dir) -> Result<AppState, Failure>`:
    - `jiraf.toml` missing → `Setup` (after `ensure_dir`).
    - token missing/empty in `.env`, or `/myself` answering 401/403, **and** the process env has no
      non-empty `JIRA_TOKEN` → `Token(msg)`.
    - everything else → `Other(msg)` (msg = `format!("{e:#}")` as today).
  - `Phase::Failed(Failure)`. `Status` (camelCase) gains `token_problem: bool`; `state` is
    `"starting" | "ready" | "failed" | "setup"`; `error` is `None` for setup.
  - `pub fn token_from_env() -> bool` in `config.rs` (non-empty process `JIRA_TOKEN`), used by
    `read_token` and here.
  - In `settings.rs`, commands (registered in `main.rs`):
    - `settings(holder) -> SettingsView` where
      `#[serde(rename_all = "camelCase")] pub struct SettingsView { pub config: Config, pub token_set: bool, pub token_from_env: bool, pub first_run: bool, pub parse_error: Option<String>, pub config_dir: String }`.
      Missing file → `Config::template()`, `first_run: true`. Parse error → `Config::template()`,
      `parse_error: Some(msg)`. `token_set` = `read_token(.env)` is Ok.
    - `save_settings(holder, config: Config, token: Option<String>) -> SaveReport` where
      `#[serde(rename_all = "camelCase")] pub struct SaveReport { pub saved: bool, pub errors: Vec<Problem>, pub warnings: Vec<Problem>, pub unverified: bool, pub status: Option<Status> }`.
      Steps: `assign_ids` → `validate(cfg, token.is_some() || token_set)` → if errors, return
      unsaved → `check_with_jira(cfg, token or the current one)` → if errors, return unsaved →
      `write_config`, then `write_token` if a token was given → run the reload (`holder.restart()`,
      `init`, `holder.set`) and put the new `holder.status()` in `status`. `saved: true` even when
      the reload then fails (the page shows that status).
    - `save_token(holder, token: String) -> Result<Status, String>`: if `jiraf.toml` parses, check
      the token with `/myself` against its URL: 401/403 → `Err("Jira rejected this token")`,
      unreachable → go on. `write_token`, reload as above, `Ok(status)`.
  - Menu: `MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))` inserted
    before "Reload config"; `on_menu_event` emits `"open-settings"`.
- [ ] **Step 1: Write the failing tests** (`app.rs` tests)

```rust
#[test]
fn setup_and_token_failures_reach_the_page() {
    let h = Holder::new(PathBuf::from("/x"));
    h.set(h.restart(), Err(Failure::Setup));
    let s = h.status();
    assert_eq!((s.state, s.error, s.token_problem), ("setup", None, false));
    h.set(h.restart(), Err(Failure::Token("JIRA_TOKEN is empty".into())));
    let s = h.status();
    assert_eq!((s.state, s.error.as_deref(), s.token_problem), ("failed", Some("JIRA_TOKEN is empty"), true));
}

#[tokio::test]
async fn missing_config_means_setup() {
    let d = tempfile::tempdir().unwrap();
    assert!(matches!(init(d.path()).await, Err(Failure::Setup)));
}
```

Update `not_ready_until_ready` and `older_init_cannot_overwrite_newer` to pass `Failure::Other(..)`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --bin jiraf app::`
Expected: compile errors: `Failure` not found.

- [ ] **Step 3: Implement the Interfaces above**

`missing_config_says_what_to_copy` in `config.rs` stays as is (`Config::load` keeps its message for
hand-run cases).

- [ ] **Step 4: Run the whole suite and a build**

Run: `cargo test && cargo build`
Expected: all pass; build succeeds.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/settings.rs src/config.rs src/main.rs
git commit -m "Startup tells setup and token problems apart; settings, save_settings and save_token commands; Settings… menu item"
```

---

### Task 5: A broken bubble doesn't break the dashboard

**Files:**

- Modify: `src/jira.rs`, `src/api.rs`, `static/app.js`, `static/index.html`

**Interfaces:**

- Produces:
  - `pub struct Loaded { pub issues: Vec<Issue>, pub broken: HashMap<String, String> }` (bubble id →
    Jira's message); `load_issues(...) -> Result<Loaded>`.
  - `pub fn bubble_error(e: &anyhow::Error) -> Option<String>`: `Some(message)` when the error is a
    `JiraError` with status 400, else `None`.
  - In `load_issues`, a bubble search error goes to `broken` when `bubble_error` is `Some`; any
    other error fails the whole load as today.
  - Payload: each bubble JSON gains `"error": <message or null>`.
  - Page: a broken bubble's chip gets class `broken`, a `⚠` badge in place of the counts, and
    `title` = `JQL rejected by Jira: <message>`; selecting it shows the table empty message
    `This bubble's JQL was rejected by Jira: <message>. Fix it in Settings (⌘,).` The `All` scope
    just has no issues from it.

- [ ] **Step 1: Write the failing test** (`jira.rs` tests)

```rust
#[test]
fn only_jql_errors_break_a_single_bubble() {
    let jql = anyhow::Error::from(JiraError { status: StatusCode::BAD_REQUEST, message: "Error in the JQL Query".into() }).context("bubble `x`");
    assert_eq!(bubble_error(&jql).as_deref(), Some("Error in the JQL Query"));
    let auth = anyhow::Error::from(JiraError { status: StatusCode::UNAUTHORIZED, message: "no".into() });
    assert_eq!(bubble_error(&auth), None);
    assert_eq!(bubble_error(&anyhow::anyhow!("timeout")), None);
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --bin jiraf jira::`
Expected: compile error: `bubble_error` not found.

- [ ] **Step 3: Implement the Interfaces above** (Rust, then `renderTeam`, the empty-table message,
      and a `.member.broken` style using `--red`/`--red-soft` in both themes)

- [ ] **Step 4: Run the whole suite, then check by hand**

Run: `cargo test && node --check static/app.js` Expected: all pass. Then
`JIRAF_CONFIG_DIR=<copy of a working config folder> cargo tauri dev` with one bubble's JQL changed
to `project = NOPE_123`: the dashboard loads, that bubble shows ⚠ with Jira's message, the others
work.

- [ ] **Step 5: Commit**

```bash
git add src/jira.rs src/api.rs static/app.js static/index.html
git commit -m "A bubble whose JQL Jira rejects shows as broken; the rest of the dashboard still loads"
```

---

### Task 6: The Settings screen

**Files:**

- Create: `static/settings.js`
- Modify: `static/index.html` (`<section class="settings" id="settings" hidden>`, styles,
  `<script src="settings.js"></script>` after `app.js`), `static/app.js` (boot/retry/error-screen
  hooks)

**Interfaces:**

- Consumes: Task 4's commands and `Status`, Task 2's `Problem.at` paths.
- Produces (globals in `settings.js`): `openSettings()` and `closeSettings()`.

Behaviour (layout and styling are the builder's, within the page's look: `.card`, chips, `.tog`
buttons):

- `openSettings()` calls `settings`, hides `main` (and the error screen), shows `#settings` with
  five cards in order **Jira**, **Fields**, **Follow-up**, **Tabs**, **Bubbles**, and a sticky bar
  with **Cancel** and **Save**. `parseError` → a red banner:
  `jiraf.toml couldn't be read: <error>. Saving replaces it; the old file is kept as jiraf.toml.bak.`
  `firstRun` → a banner `Welcome! Fill in your Jira details and at least one bubble, then Save.`,
  and no Cancel.
- Token: if `tokenSet`, `••••••••` plus **Replace** (reveals an empty password input); otherwise the
  input is shown. `tokenFromEnv` → note
  `JIRA_TOKEN is set in the environment and wins over the one saved here.`
- Lists (`hierarchy`, `priority_order`, `activity_types`, `issues`) are chips with × and an input
  that adds on Enter; `hierarchy` and `priority_order` chips reorder with ↑/↓ (or drag). Tabs and
  bubbles are cards with ↑ ↓ and Delete; **+ Add tab** / **+ Add bubble** append a card with
  `id: ""`. A bubble's exclude is a set of checkboxes naming the other bubbles (stored by id; a
  bubble added in this session can't be excluded until saved, since it has no id yet). Deleting a
  bubble also removes it from the others' excludes.
- Each field label has an `ⓘ` with a `title` from a `HELP` map keyed by the same paths as
  `Problem.at` (index-free, e.g. `bubbles.jql`). Texts, from `jiraf.example.toml`: url
  `Base URL of your Jira, e.g. https://jira.example.com`; token
  `Jira > Profile > Personal Access Tokens. Stored in .env in the config folder, never shown again.`;
  cache_seconds `How long Jira answers are kept in memory. The ↻ button bypasses it.`; fields
  (shared hint) `Custom field ids differ per Jira: GET /rest/api/2/field lists them.`, plus
  extended_status `Single-line text: the follow-up note`, activity_type
  `Single-select "Activity Type": decides the tab`, parent_link `Advanced Roadmaps "Parent Link"`,
  epic_link `"Epic Link"`, target_start `Advanced Roadmaps "Target start"`, target_end
  `Advanced Roadmaps "Target end": editable, drives the "late" check`; stale_days
  `An Extended Status older than this counts as "no update"`; done_days
  `Closed issues stay visible this many days`; ext_max_len
  `Jira single-line text fields cap at 255`; hierarchy
  `Issue type names, top of the hierarchy first`; initiative_type
  `The Initiative level in the hierarchy`; priority_order
  `Display order for priorities (Jira's own order if empty)`; tabs.activity_types
  `Whole Activity Type values; case and accents are ignored. The top-most Activity Type in an item's line wins, so a whole tree stays in one tab.`;
  tabs.include_unset `Also holds items with no Activity Type anywhere up their tree`; tabs.issues
  `Issue keys pinned to this tab (their descendants follow); wins over everything`; bubbles.jql
  `Any JQL; ORDER BY is ignored. Open issues plus those closed in the last done_days are shown.`;
  bubbles.icon_project `A Jira project key whose avatar becomes the bubble icon`; bubbles.exclude
  `Subtract these bubbles: (this JQL) AND NOT (theirs)`.
- **Save** sends `{config, token}` (token only if typed) to `save_settings`, button shows `Saving…`.
  `errors` → each shown under its field (by `at`), the first scrolled into view, and a toast
  `Not saved: fix the marked fields`. Saved → if `warnings` or `unverified`, stay on Settings with
  the bubble warnings under their JQL boxes and/or a note
  `Saved, but jiraf couldn't verify it with Jira`, plus a **Back to dashboard** button; otherwise
  `location.reload()`. If the returned `status.state` is `failed`, go to the error screen.
- **Cancel** → `closeSettings()` (back to the dashboard or the error screen, as before).
- `⌘,` / Settings… menu: `window.__TAURI__.event.listen('open-settings', openSettings)`.
- In `app.js`: `boot()` and `retryFlow()` call `openSettings()` when `state === 'setup'`. The error
  screen gets an **Open Settings** button, and when `tokenProblem` a password input + **Save token**
  that calls `save_token` (error → shown under the input; success → `location.reload()` if ready,
  else the new error).
- [ ] **Step 1: Check the scripts parse**

Run: `node --check static/app.js && node --check static/settings.js`
Expected: no output.

- [ ] **Step 2: Check by hand in `cargo tauri dev`**, with `JIRAF_CONFIG_DIR` pointing at a scratch
  folder:
  1. Empty folder → Settings opens with the welcome banner; Save with nothing filled marks URL,
     token, fields, bubbles.
  2. Fill in real values and one bubble → Save → dashboard loads; `jiraf.toml` starts with the
     header; `jiraf.toml.bak` absent (no previous file).
  3. ⌘, → change a bubble's JQL to `project = NOPE_123` → Save → stays on Settings with Jira's
     message under that JQL → Back to dashboard shows it broken.
  4. Replace the token with `nope` → Save → `Jira rejected this token` under the token; files
     unchanged.
  5. Put `JIRA_TOKEN=` in `.env`, Reload config (⌘⇧R) → error screen with the token box; paste the
     real token → dashboard.
  6. Both themes look right; tooltips show on every ⓘ.
  Expected: each as described.

- [ ] **Step 3: Commit**

```bash
git add static/settings.js static/index.html static/app.js
git commit -m "Settings screen: every setting and the token, with help tooltips; first run and token problems lead there"
```

---

### Task 7: Docs

**Files:**

- Modify: `README.md`, `jiraf.example.toml` (top comment: "You can also edit all of this from the
  app: Settings (⌘,).")
- [ ] **Step 1:** README: replace "copy the example files and edit them by hand" with the Settings
      flow (first launch opens Settings; ⌘, later; token paste on the error screen), mention
      `jiraf.toml.bak` and that Settings writes a clean file without comments, keep hand-editing as
      an option; add `settings`, `save_settings`, `save_token` to the commands table.
- [ ] **Step 2:** Run: `cargo test && cargo clippy --all-targets`. Expected: all pass, no new
      warnings.
- [ ] **Step 3: Commit**

```bash
git add README.md jiraf.example.toml
git commit -m "Docs: the Settings screen"
```
