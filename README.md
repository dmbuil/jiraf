# jiraf

[![Release](https://img.shields.io/github/v/release/dmbuil/jiraf?include_prereleases&sort=semver)](https://github.com/dmbuil/jiraf/releases)
[![Version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Fdmbuil%2Fjiraf%2FHEAD%2FCargo.toml&query=%24.package.version&label=version)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-blue)](https://www.rust-lang.org/)
![Platform](https://img.shields.io/badge/platform-macOS-lightgrey)
[![Last commit](https://img.shields.io/github/last-commit/dmbuil/jiraf)](https://github.com/dmbuil/jiraf/commits)
![Jira](https://img.shields.io/badge/Jira-Data%20Center-blue)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE.md)

<p align="center"><b>Jira + follow-up = jiraf.</b> Keeping an eye on everything, from a tall vantage point.</p>


<p align="center">
  <img src="docs/jiraf.png" alt="jiraf, a cartoon giraffe in round glasses tapping a follow-up dashboard" width="250">
</p>

jiraf is a lean, stateless follow-up dashboard for **Jira Data Center** with **Advanced Roadmaps
(Plans)**. Plans is built for planning: timelines, scheduling, releases, teams. jiraf is built for the
weekly follow-up meeting. It answers "where are we, what's stuck, what's stale" and lets you fix it
on the spot:

- **Bubbles**: each one is a named JQL filter. Switch between projects or workstreams in one click.
- **Hierarchy tree**: Initiative › Epic › Task › Sub-task, collapsible at every level.
- **Tabs by purpose**: e.g. *Roadmap*, *Sustainability & Support*, *Tech. Evolution*, decided by the
  *Activity Type* field.
- **Needs attention**: what's blocked, late, gone quiet, or just closed.
- **Inline editing** that writes straight to Jira: Status, Priority, Assignee, Target end, Due date
  and the *Extended Status* follow-up note.

**Stateless by design.** Every view is built from live Jira queries, and every edit goes straight back to
Jira. jiraf has no database. It only keeps a short in-memory cache and local copies of Jira's icons.

jiraf is a desktop app (built with [Tauri](https://tauri.app)): click it in the Dock and the dashboard
opens in its own window. No terminal, no browser tab, no local server.

---

## Contents

- [Requirements](#requirements)
- [Quick start](#quick-start)
- [Using the dashboard](#using-the-dashboard)
- [Configuration](#configuration)
  - [Bubbles](#bubbles)
  - [Tabs](#tabs)
  - [Field ids](#field-ids)
  - [Reference](#reference)
- [Recipe: a bubble that follows a hierarchy](#recipe-a-bubble-that-follows-a-hierarchy)
- [How it works](#how-it-works)
- [Security](#security)
- [Development](#development)
- [Troubleshooting](#troubleshooting)

---

## Requirements

| What                                                   | Why                                                                                                    |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ |
| **macOS** (Apple Silicon)                              | the only platform built today (Windows is planned)                                                     |
| **Rust 1.88+** (`rustup update`)                       | to build jiraf                                                                                         |
| **Tauri CLI 2** (`cargo install tauri-cli --version "^2" --locked`) | to build the `.app`                                                                       |
| **Jira Data Center / Server** with REST API v2         | data source (tested on Jira DC 10.3)                                                                   |
| A Jira **Personal Access Token**                       | jiraf acts as you: it sees what you see and edits as you                                               |
| **Advanced Roadmaps** (Plans)                          | *Parent Link*, *Target start* and *Target end* fields                                                  |
| A single-line text custom field for the follow-up note | the *Extended Status* column                                                                           |
| *Optional:* **ScriptRunner**                           | only for bubbles that follow a hierarchy (see the [recipe](#recipe-a-bubble-that-follows-a-hierarchy)) |

## Quick start

```sh
git clone <this repo> jiraf && cd jiraf
cargo tauri build --bundles app   # → target/release/bundle/macos/jiraf.app
```

Drag `jiraf.app` into **Applications** and open it. The first launch opens the **Settings** screen:
fill in your Jira URL, token, field ids and at least one bubble, then **Save**. jiraf checks them with
Jira and loads the dashboard. Later, **Settings…** (⌘,) in the app menu brings it back.

Everything lives in the config folder:

```
~/Library/Application Support/com.dmbuil.jiraf/
├── jiraf.toml           written by Settings: your Jira URL, field ids, tabs and bubbles
├── jiraf.toml.bak       the previous jiraf.toml, kept on every save
├── .env                 JIRA_TOKEN=… (Settings only ever changes that line)
├── jiraf.example.toml   a commented example, if you prefer editing by hand
├── .env.example
├── icons/               local copies of Jira icons (created on first load)
└── jiraf.log            the app's log
```

Already have a `jiraf.toml` and `.env` from the server version? Move them into that folder.
**Open config folder** on the error screen opens it in Finder.

To create a token, go to **Jira › your avatar › Profile › Personal Access Tokens › Create token**.

On startup jiraf checks the token (`GET /rest/api/2/myself`) and that the configured Extended Status
field exists. If anything fails, the window shows the error, the config folder, **Retry**, **Open
Settings** and **Open config folder**. When the token is the problem (missing, empty, expired), the
error screen also has a box to paste a new one. When all is well, `jiraf.log` says:

```
INFO jiraf::app] connected to https://jira.example.com as JANE DOE
```

Set `RUST_LOG=debug` in `.env` for more detail, or `RUST_LOG=warn` for less.

## Using the dashboard

```
┌ jiraf  [∗ All 368] [Alpha 20] [Beta 224] [Beta Core 52] …                     ● 14:05  ◐  ↻ ┐
│ Beta  JQL ↗   Roadmap 157 │ Sust. & Support 67 │ Needs attention 200     224 Open  18 Blocked … │
│ ┌ filter… ┐ [Initiative][Epic][Task][Sub-task]  + closed 14d            Expand all  Collapse all │
│ ISSUE                           STATUS       P  WHO  TARGET END  DUE        EXTENDED STATUS 💬 │
│ ▾ ABC-9  Initiative …           BLOCKED      ⌃  (JD) —           1 Mar 26   601d  Kick-off  27 │
│     ▸ ABC-964  Epic …           IN PROGRESS  ⌄  (?)  —           —          —  No status note  │
```

### Top bar

- **Bubbles** each show three counts: open (grey), blocked (red), and no update in `stale_days`+ (amber).
  **All** combines every bubble. Use **← / →** to move between bubbles. When they don't fit, **‹ ›**
  buttons appear next to the status dot to scroll the strip (the mouse wheel scrolls it too), and the
  strip fades out on each side that has more bubbles.
- **●** shows when the data was last loaded. It's amber while loading and red on error.
- **◐ / ☀ / ☾** cycles the theme: Auto (follows your OS), Light, Dark. The choice is remembered.
- **↻** reloads from Jira, bypassing the cache.
- **Reload config** (⌘⇧R, in the jiraf menu) re-reads `jiraf.toml` and `.env` after you edit them.
- Jira links (issue keys, labels, **JQL ↗**, comments) open in your default browser.

### Header row

- **JQL ↗** opens the bubble's effective JQL in Jira's issue search.
- **Tabs**: one per configured tab, then **Needs attention**. An **Other** tab appears only if some
  item's Activity Type matches no tab.
- **Counters**: Open, Blocked, Late, No update, Closed recently. Click one to jump to its list.

The URL keeps your place (`#p=<bubble>&t=<tab>`), so a view can be bookmarked or shared.

### The tree

- **▸/▾** collapses or expands a row. It's greyed out when the row has no children. A collapsed row
  shows **+N**, the number of hidden children.
- Parents come from the Sub-task parent, then *Parent Link*, then *Epic Link*. Parents outside your
  filters (closed, or in another project) are fetched anyway and shown **dimmed**, so the tree is never
  broken.
- **Labels** show as chips under the summary: the first two, then a **+N** chip that lists the rest on
  hover. Click a label to open a Jira search for `labels = "<label>"` in a new tab.
- **Filter box**: matches key, summary, assignee, labels and the status note. The ancestors of any
  match stay visible.
- **Type toggles** hide a level. Its children then attach to the nearest visible ancestor.
- **+ closed Nd** also shows issues closed within the last `done_days`.
- **Column headers** sort the rows within each level. Click once for ascending, twice for descending,
  and a third time to turn sorting off.
- In **All**, rows are grouped under each bubble.

### Editing

Every edit goes straight to Jira. The row then shows the issue as Jira returns it, and the edited cell
gets a blue marker. If Jira refuses an edit (workflow rule, permission, validation), its message
appears in a red toast at the bottom of the page.

| Column              | How                                                                                            | Jira call                       |
| ------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------- |
| **Status**          | click the pill; you get only the transitions the workflow allows from the current status       | `POST /issue/{key}/transitions` |
| **P** (priority)    | click the icon; you get only the priorities the issue's project allows (its priority scheme)   | `PUT /issue/{key}`              |
| **Who** (assignee)  | hover the avatar for the name; click to search people who can be assigned (deactivated hidden) | `PUT /issue/{key}/assignee`     |
| **Target end**      | click; pick a date, or **Clear**                                                               | `PUT /issue/{key}`              |
| **Due**             | click; pick a date, or **Clear**                                                               | `PUT /issue/{key}`              |
| **Extended status** | click the text; **⌘/Ctrl + Enter** saves, **Esc** cancels; **+ dd/mm** prepends today's date   | `PUT /issue/{key}`              |

The **Due** column is Jira's own *Due date*, in red once it has passed on an open issue.

The **Who** column shows only the assignee's avatar: Jira's picture when Jira hosts it, otherwise
their initials (pictures on other hosts, such as Gravatar, are never loaded), and a dashed **?** when
unassigned.

The **age chip** next to the Extended Status shows how long ago *the field itself* was last edited.
jiraf reads that from the issue changelog, so a date typed into the text doesn't matter:

| Chip        | Meaning                                            |
| ----------- | -------------------------------------------------- |
| green `3d`  | edited ≤ 7 days ago                                |
| amber `21d` | ≤ `stale_days`                                     |
| red `120d`  | older than `stale_days`                            |
| `orig.`     | set when the issue was created, never edited since |
| `—`         | empty                                              |

**💬 N** is the comment count. Hover over it to read the latest comment (trimmed to 3,000 characters). The
link in the popover opens the full thread in Jira.

### Needs attention

The same issues as the tree, as four lists. Each open issue appears in at most one of the first three:

| List                  | Rule                                                                                                          |
| --------------------- | ------------------------------------------------------------------------------------------------------------- |
| 🚧 **Blocked**         | status name contains "Blocked"                                                                                |
| ⏰ **Past target end** | *Target end* is in the past (and not blocked)                                                                 |
| 💤 **No update**       | Extended Status (or, if never set, the issue) unchanged for more than `stale_days` (and neither of the above) |
| ✅ **Closed recently** | resolved within `done_days`                                                                                   |

Rows here are editable too.

## Configuration

Use **Settings** (⌘,): every value below is a form field there, with an ⓘ tooltip on how to fill it
in. On **Save**, jiraf refuses what would stop it from working (an empty URL, a token Jira rejects, a
field id Jira doesn't know…) and marks the field. A bubble whose JQL Jira rejects is saved anyway with a
warning, and shows as broken (⚠) on the dashboard while the other bubbles keep working. If Jira can't
be reached, the save goes through unchecked. Settings writes a clean `jiraf.toml` (no comments) and
keeps the previous one as `jiraf.toml.bak`.

You can still edit the files by hand. jiraf reads two files from its config folder
(`~/Library/Application Support/com.dmbuil.jiraf/`):

| File         | Contents                                                                |
| ------------ | ----------------------------------------------------------------------- |
| `.env`       | `JIRA_TOKEN=…`, the only secret (a real environment variable wins), and optionally `RUST_LOG=…` |
| `jiraf.toml` | everything else. Start from [`jiraf.example.toml`](jiraf.example.toml)  |

Config changes take effect with **Reload config** (⌘⇧R) or on restart.

### Bubbles

```toml
[[bubbles]]
id = "alpha"              # unique; used in the URL (#p=alpha)
name = "Alpha"            # label in the top bar
jql = "project = ALPHA"   # any JQL; a trailing ORDER BY is ignored
icon_project = "ALPHA"    # optional: borrow this Jira project's avatar
```

- For every bubble, jiraf fetches `(<jql>) AND (statusCategory != Done OR resolved >= -<done_days>d)`,
  i.e. open issues plus recently closed ones.
- An issue can belong to several bubbles. It's counted in each.
- **Complements**: `exclude = ["other-id", …]` subtracts other bubbles, so two filters split a project
  with no overlap and nothing left out:

  ```toml
  [[bubbles]]
  id = "beta"
  name = "Beta"
  jql = "project = BETA"
  exclude = ["beta-core"]   # effective JQL: (project = BETA) AND NOT (<beta-core jql>)
  ```

  Only the excluded bubble's own `jql` is used (excludes don't chain). jiraf won't start if `exclude`
  names an unknown bubble or the bubble itself.

### Tabs

Tabs split the tree by the **Activity Type** field (a single select, see [Field ids](#field-ids)).
It's usually set on Initiatives, and the **top-most** value in an item's line wins: an Epic tagged
differently from its Initiative still follows the Initiative, so a whole tree always lives in one tab. Tabs appear in the order you list them.

```toml
[[tabs]]
id = "road"
name = "Roadmap"
activity_types = ["New Roadmap"]                # whole values; case and accents are ignored
include_unset = true                            # items with no Activity Type up the tree land here

[[tabs]]
id = "sus"
name = "Sust. & Support"
activity_types = ["Sustainability", "Support"]  # several values can share a tab
issues = ["ABC-123"]                            # pin these issues (and their descendants) here

[[tabs]]
id = "evo"
name = "Tech. Evol."
activity_types = ["Technology Evolution"]
```

jiraf picks the tab for each item in this order:

1. A tab listing the item, or one of its ancestors, in `issues`.
2. The first tab whose `activity_types` lists the top-most Activity Type in the item's line (its root
   ancestor's if set, then the next one down, and so on to its own).
3. If there's none anywhere up the tree: the tab with `include_unset = true`.
4. Otherwise: the **Other** tab.

The older label-based keys (`match`, `include_unlabeled`, `[fields] initiative_label`) are rejected at
startup, so an outdated `jiraf.toml` fails loudly instead of sorting everything into **Other**.

### Field ids

Custom field ids differ on every Jira instance. List yours with:

```sh
curl -s -H "Authorization: Bearer $JIRA_TOKEN" https://jira.example.com/rest/api/2/field \
  | jq -r '.[] | select(.custom) | "\(.id)\t\(.name)"' | sort -k2
```

| Key               | The field it should point to                                     |
| ----------------- | ---------------------------------------------------------------- |
| `extended_status` | your single-line text follow-up note                             |
| `activity_type`   | the *Activity Type* single select that drives the tabs           |
| `parent_link`     | Advanced Roadmaps *Parent Link*                                  |
| `epic_link`       | *Epic Link*                                                      |
| `target_start`    | Advanced Roadmaps *Target start* (shown when editing Target end) |
| `target_end`      | Advanced Roadmaps *Target end* (editable; drives *Late*)         |

### Reference

| Section       | Key                                                       | Default                                   | Description                                                 |
| ------------- | --------------------------------------------------------- | ----------------------------------------- | ----------------------------------------------------------- |
| `[server]`    | `icon_dir`                                                | `"icons"`                                 | local copies of Jira icons, relative to the config folder; safe to delete |
|               | `bind`                                                    | —                                         | no longer used (the app opens no port); accepted and ignored |
| `[jira]`      | `url`                                                     | *required*                                | base URL, e.g. `https://jira.example.com`                   |
|               | `cache_seconds`                                           | `60`                                      | how long the dashboard data is kept in memory               |
| `[fields]`    | *see [Field ids](#field-ids)*                             | instance-specific                         | custom field ids                                            |
| `[follow_up]` | `stale_days`                                              | `30`                                      | Extended Status older than this counts as "no update"       |
|               | `done_days`                                               | `14`                                      | closed issues stay visible this long                        |
|               | `ext_max_len`                                             | `255`                                     | max length of the Extended Status (Jira's text-field limit) |
|               | `hierarchy`                                               | `["Initiative","Epic","Task","Sub-task"]` | issue types, top first: type toggles and default sort       |
|               | `initiative_type`                                         | `"Initiative"`                            | name of the Initiative level                                |
|               | `priority_order`                                          | Jira's order                              | display/sort order of priorities                            |
| `[[tabs]]`    | `id`, `name`, `activity_types`, `include_unset`, `issues` |                                           | see [Tabs](#tabs)                                           |
| `[[bubbles]]` | `id`, `name`, `jql`, `icon_project`, `exclude`            |                                           | see [Bubbles](#bubbles)                                     |

## Recipe: a bubble that follows a hierarchy

JQL can't say "this issue and everything below it". With ScriptRunner you can chain functions, one
clause per level. With `I` standing for the Initiative query:

| Clause                                  | Picks up                                          |
| --------------------------------------- | ------------------------------------------------- |
| `I`                                     | the matching Initiatives                          |
| `subtasksOf(I)`                         | Sub-tasks hanging directly off an Initiative      |
| `portfolioChildrenOf(I)`                | *Parent Link* children: Epics, nested Initiatives |
| `subtasksOf(portfolioChildrenOf(I))`    | Sub-tasks of those Epics                          |
| `issuesInEpics(portfolioChildrenOf(I))` | *Epic Link* children: Tasks, Stories…             |
| `subtasksOf(issuesInEpics(…))`          | Sub-tasks of those                                |

See the `beta-core` bubble in [`jiraf.example.toml`](jiraf.example.toml) for the full query. Notes:

- Write it as a **TOML literal string** (`'''…'''`), so the `\"` needed at the third level of nested
  quotes is passed to Jira unchanged.
- `portfolioChildrenOf` goes down **one** level, so Initiatives nested under other Initiatives must match
  `I` themselves (e.g. carry the same label).
- Advanced Roadmaps' `issue in childIssuesOf("KEY")` takes only one key and, in testing, skipped
  Sub-tasks. That's why the recipe uses ScriptRunner.
- Pair it with `exclude` on the project's catch-all bubble to split the project cleanly.

## How it works

```
 window (static/index.html, app.js) ──Tauri IPC──▶ jiraf (Rust) ──REST v2 + Bearer token──▶ Jira Data Center
   ▲                                                 │  per-bubble searches (parallel, paginated, with changelog)
   └──────────── JSON ◀──────────────────────────────┘  + missing parents, issue types, priorities, avatars
```

- **One load = one payload.** The `data` command runs every bubble's search in parallel, merges the
  results (tagging each issue with its bubbles), adds missing ancestors, and returns a compact JSON. The
  tree, tabs, counters and lists are all computed in the page from that payload.
- **Cache**: the payload is kept in memory for `cache_seconds`. **↻** and any edit invalidate it.
- **Edits** go through jiraf's commands. After writing, jiraf reads the issue back and returns it, so
  the page always shows what Jira actually stored.
- **Icons** (issue types, priorities, avatars) are served by jiraf on its own `jiraf://` scheme, fetched
  with your token, and saved in `icon_dir`. The page never contacts Jira directly.

### Commands

| Command                                 | Purpose                                            |
| --------------------------------------- | -------------------------------------------------- |
| `app_status`, `retry`, `open_config_dir` | startup state, Retry / Reload config, Finder      |
| `data { refresh }`                      | the dashboard payload                              |
| `transitions { key }`                   | transitions available for the status menu          |
| `priorities { key }`                    | priorities the issue allows, for the priority menu |
| `assignable { key, q }`                 | assignable users for the assignee menu             |
| `transition { key, id }`                | move through a workflow transition                 |
| `set_priority { key, name }`            | set the priority                                   |
| `set_assignee { key, name }`            | assign (`name`) or unassign (`null`)               |
| `set_target { key, date }`              | set or clear Target end (`YYYY-MM-DD` or `null`)   |
| `set_due { key, date }`                 | same, for Jira's Due date                          |
| `set_ext { key, value }`                | set the Extended Status                            |
| `open_external { url }`                 | open a Jira link in the default browser            |
| `settings`                              | what the Settings screen shows (never the token)   |
| `save_settings { config, token }`       | validate, check with Jira, write the files, reload |
| `save_token { token }`                  | the error screen's token box                       |

## Security

jiraf holds a token that can edit Jira as you:

- **No port is opened.** The page talks to jiraf over Tauri's in-process IPC, so no other program or
  website can reach it.
- **The token stays in the Rust process.** It's read from `.env` and never sent to the page.
- **Input checks**: issue keys are validated before they're used in Jira URLs, and the icon scheme only
  serves Jira avatar/icon paths.
- **Only Jira links open**: `open_external` refuses any URL outside the configured Jira.
- **Content-Security-Policy**: only jiraf's own script runs, and the page can't load anything remote.
  Jira text is always inserted as text, never as HTML.

**Never commit** `.env`, `jiraf.toml` (real URLs, project keys and JQL), `icons/`, `jiraf.log` or `mockup/`.
All five are in `.gitignore`.

## Development

```sh
cargo test                                # unit tests (config, mapping, JQL helpers, key checks, links, icons)
JIRAF_CONFIG_DIR=$PWD cargo tauri dev     # run with ./jiraf.toml and ./.env; rebuilds on change
cargo tauri build --bundles app           # release .app
cargo doc --no-deps --document-private-items --open   # API docs for every module, type and function
```

`JIRAF_CONFIG_DIR` points the app at any config folder; without it, the app uses its own
(`~/Library/Application Support/com.dmbuil.jiraf/`). The log goes to `jiraf.log` in that folder.

```
src/main.rs      Tauri setup: config folder, menu, icon scheme, command list
src/app.rs       startup checks (init), startup state, Retry
src/config.rs    jiraf.toml / .env loading, defaults, validation, bubble excludes
src/jira.rs      Jira client, searches, issue mapping, changelog → Extended Status age
src/api.rs       commands, cache, icon store, edits, link check
src/logging.rs   log file
static/          index.html (layout + styles) and app.js (rendering + interactions)
tauri.conf.json  window, CSP, bundle; capabilities/ (page permissions)
app-icons/       app icons: `cargo tauri icon app-icons/jiraf-icon-1024.png -o app-icons`
                 (that file is docs/jiraf-icon.png fitted to Apple's 824/1024 icon grid)
docs/            the mascot, the app icon artwork, design and plan
```

## Troubleshooting

| Symptom                                               | Fix                                                                                                  |
| ----------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `JIRA_TOKEN is not set (put it in …/.env)`            | create `.env` from `.env.example` in the config folder, then Retry                                   |
| `…/jiraf.toml not found: copy jiraf.example.toml…`    | copy `jiraf.example.toml` to `jiraf.toml` in the config folder and fill it in, then Retry            |
| `checking JIRA_TOKEN against Jira` + 401              | token expired or revoked: create a new one                                                           |
| `field customfield_… (extended_status) not found`     | wrong id in `[fields]`; see [Field ids](#field-ids)                                                  |
| ``bubble `x`: Jira answered 400 …``                   | that bubble's JQL is invalid; test it in Jira's issue search first                                   |
| `issueFunction` errors                                | the JQL uses ScriptRunner functions, which this Jira doesn't have                                    |
| An **Other** tab appears                              | some Activity Type matches no tab: add it to a tab's `activity_types` or pin the issue with `issues` |
| Rows look dimmed                                      | they're parents outside your filters, shown for context (read-only)                                  |
| `jiraf is not ready`                                  | startup failed: use Reload config (⌘⇧R) to see the error screen                                      |
| Stale or missing icons                                | delete `icons/` in the config folder and restart                                                     |
| Anything else                                         | look in `jiraf.log` in the config folder (`RUST_LOG=debug` in `.env` for more)                      |

## License

[MIT](LICENSE.md) © 2026 dmbuil
