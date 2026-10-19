# jiraf Settings screen: design

Date: 2026-10-19 · Branch: `tauri`

## Intent

Today every change to jiraf's setup means opening `jiraf.toml` or `.env` in the config folder and
editing them by hand. The two edits that actually happen are **changing the bubbles' JQL** (worked out
in Jira first, then pasted in) and **renewing an expired Jira token**. Tabs change now and then; the
rest is set once. The user wants all of it editable from inside the app, so the files never need to be
opened. This is the "settings menu" the Tauri migration design deferred.

Users: the author, on their own Mac (Windows build planned). Same as the app.

## Goals

1. A **Settings screen** that replaces the dashboard in the window (not a second window). It opens
   with ⌘, (Ctrl+, on Windows) and a **Settings…** item in the app menu. **Cancel** returns to the
   dashboard with nothing changed; **Save** writes the files and reloads the dashboard.
2. Every setting in `jiraf.toml` is editable as plain form fields, plus the token. The user never
   sees TOML. Sections, in this order:
   1. **Jira**: URL, token, cache seconds.
   2. **Fields**: the six custom field ids (`extended_status`, `activity_type`, `parent_link`,
      `epic_link`, `target_start`, `target_end`) as text inputs.
   3. **Follow-up**: `stale_days`, `done_days`, `ext_max_len`, `initiative_type`, and the two ordered
      lists `hierarchy` and `priority_order` (reorderable chips; add/remove).
   4. **Tabs**: one card per tab with name, Activity Type values (chips), "also holds items with no
      Activity Type" (`include_unset`), pinned issue keys (chips). Add, delete, reorder.
   5. **Bubbles**: one card per bubble with name, JQL (multi-line box the user pastes into), icon
      project, exclude (picked from the other bubbles). Add, delete, reorder.
3. **Token**: shown masked with a **Replace** action; the value is never sent back to the page.
   A pasted token is checked against Jira (`/myself`) before it is kept.
4. **Startup error screen** gains an **Open Settings** button, and, when the failure is the token
   (missing, empty, or rejected by Jira), a paste box right there that saves the token and retries.
5. **First launch** (no `jiraf.toml` in the config folder): the app opens straight on an empty
   Settings screen instead of the "copy the example files" error. Saving it creates `jiraf.toml`
   (and `.env`) and starts the dashboard.
6. **Validation on Save**:
   - **Blocking** (nothing is written, the offending field is marked with the message): empty or
     invalid Jira URL, missing name/JQL on a bubble, duplicate names, an exclude that points at
     itself, a token Jira rejects, a field id that doesn't exist in Jira, anything `Config::load`
     would reject today.
   - **Warning only** (saved anyway, the warning shows on that bubble): a JQL that Jira rejects.
     The user explicitly chose "save it anyway and warn me".
   - **Jira unreachable** (off VPN, timeout, 5xx) while checking: the save is not blocked. The Jira
     checks are skipped and Settings says "couldn't verify with Jira" (my call: on-prem Jira is often
     out of reach, and a typo fix must not need the VPN). The same applies to the error screen's
     token paste box: it saves, then retries.
7. **A broken bubble doesn't break the dashboard** (user's choice): if one bubble's search fails in
   Jira, the dashboard still loads; that bubble shows as broken and empty, with Jira's message on it
   (e.g. in its tooltip and as the table's empty message when selected). A bubble whose `exclude`
   points at a broken bubble is broken too (its JQL contains the bad one). Today `jira::load_issues`
   fails the whole load on the first failed search; that changes to a per-bubble result. A network or
   auth failure (Jira down, token rejected) still fails the whole load as today.

## Non-goals

- A JQL builder, autocomplete or live preview. The user writes JQL in Jira and pastes it.
- A field picker fetched from Jira's field list (set-once values; plain text plus a check is enough).
- Editing `icon_dir` or `bind` (cache location / ignored); they stay file-only and are preserved.
- Keychain storage for the token; it stays in `.env`.
- A separate settings window, multiple profiles, import/export, undo history.

## Anti-goals (failure even if it "works")

- **Losing what is in the files**: saving must keep `jiraf.toml`'s comments, formatting, key order
  and untouched keys (including `icon_dir`, `bind`), and every other line of `.env` (e.g. `RUST_LOG`).
  Only the values that changed are rewritten. (My call; the user has a 110-line commented file.)
- The token ever reaching the page (in `app_status`, a settings payload, a log line or an error).
- A save that leaves a half-written file: write to a temp file in the same folder and rename.
- Breaking a hand-edited file: the files stay the source of truth and hand edits keep working.
- Renaming a bubble or tab silently breaking things: ids (used in the URL hash `#p=…&t=…` and in
  `exclude`) are stable; see "Ids".
- New npm/bundler tooling: the page stays plain `static/index.html` + `static/app.js`.

## Constraints

- Same stack and style as the app: Tauri 2 commands, `withGlobalTauri`, rustdoc on every item,
  `# Errors` sections, terse comments, Rust edition 2024. TDD for the Rust logic.
- Both themes (light/dark) and the existing visual language (cards, chips, `.tog` buttons).
- Windows stays possible: no macOS-only APIs; the shortcut is `CmdOrCtrl+,`.

## How it works

### Reading and writing the files (my calls)

- Read: a `settings()` command returns everything in `jiraf.toml` the form shows, plus
  `tokenSet: bool` (never the token). If `jiraf.toml` is missing it returns defaults with
  `firstRun: true`. If it doesn't parse, it returns the parse error and the screen shows it with
  the folder path (the user fixes the file by hand, or chooses to start from defaults). Starting
  from defaults first copies the broken file to `jiraf.toml.bak` (my call), then writes a new one.
- Write: edit the existing document with the `toml_edit` crate so comments and layout survive;
  arrays of tables (`[[tabs]]`, `[[bubbles]]`) are rewritten entry by entry, keeping comments
  attached to entries that still exist. A brand-new file starts from `jiraf.example.toml`'s layout
  with its example bubbles/tabs removed.
- `.env`: replace (or append) the `JIRA_TOKEN=` line only; leave every other line as is.
- Validate before writing, using the same rules as `Config::load` plus the Jira checks, so a saved
  config always loads.
- After writing: run the existing reload path (`Holder::restart` + `init`), then the page reloads
  the dashboard. The dev-mode rule stays: a `JIRA_TOKEN` in the real process environment wins over
  `.env`. In that case (my call) Replace still checks the pasted token and writes it to `.env`, and
  Settings notes that the environment token is the one in use until it is unset. The error screen's
  token box is not offered when the failing token came from the environment.

### Ids (my call)

Bubbles and tabs keep their `id` but the form doesn't show it. A new one gets an id slugged from its
name (made unique); renaming never changes the id. "Exclude" is picked by name and stored by id.

### Startup paths

- No `jiraf.toml` → `app_status` reports `state: "setup"` → the page opens Settings with defaults.
- Token problem → `failed` with a token kind, so the error screen shows the paste box.
- Any other failure → error screen with Retry, Open config folder, and Open Settings.

## Left to the builder

- Exact markup and styling of the screen and its cards, within the existing look.
- Reordering mechanics (drag or ↑/↓ buttons).
- Whether Jira checks run per field on blur or all at once on Save (all at once is fine).
- The command split and module layout (e.g. a `settings.rs`), each module with a rustdoc header.
