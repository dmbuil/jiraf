# Changelog

All notable changes to jiraf. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.2.1] - 2026-10-19

First release with ready-made builds for macOS and Windows.

### Added

- **Windows and Intel Mac builds.** A GitHub workflow tests jiraf on macOS and Windows and builds a
  universal macOS `.dmg` and a Windows installer (`.exe` and `.msi`). Release builds are attached to
  the GitHub release.

### Fixed

- jiraf builds on Windows without warnings (`cargo clippy` is clean there too).

## [0.2.0] - 2026-10-19

jiraf becomes a native desktop app (Tauri 2) with a Settings screen. No more terminal, browser tab or
hand-edited config files.

### Added

- **Desktop app.** jiraf is now a macOS app (`cargo tauri build --bundles app`): open it from the Dock or
  Launchpad and the dashboard appears in its own window. Closing the window quits the app. The code
  keeps a Windows build possible.
- **Settings screen** (⌘, , **Settings…** in the app menu, or the ⚙ button at the right of the top bar):
  every setting of `jiraf.toml` plus the Jira token, as a form.
  - Sections: Jira (URL, token, cache), Fields (custom field ids), Follow-up (thresholds, hierarchy,
    priority order), Tabs and Bubbles. Tabs and bubbles can be added, deleted and reordered.
  - An ⓘ tooltip on every field explains how to fill it in, and short notes explain what tabs and
    bubbles are for.
  - On **Save**, jiraf checks the token, the field ids and every bubble's JQL with Jira. Problems that
    would stop the app (empty URL, rejected token, unknown field id…) block the save and are shown under
    the field. A JQL that Jira rejects is saved anyway, with a warning. If Jira can't be reached, the
    save goes through unchecked.
  - The token is shown masked with a **Replace** button and never sent back to the page.
  - **Open jiraf.toml** opens the file in your editor for hand edits.
- **First launch** opens an empty Settings screen instead of asking you to copy example files.
- **Startup error screen** in the window, with the message, the config folder, **Retry**, **Open
  Settings** and **Open config folder**, plus an illustration. When the token is the problem (missing,
  empty, expired), the screen has a box to paste a new one.
- **Broken bubbles stay contained.** A bubble whose JQL Jira rejects shows as broken (dashed red, ⚠, Jira's
  message on hover and in the table). The rest of the dashboard keeps loading.
- **Reload config** menu item (⌘⇧R) re-reads `jiraf.toml` and `.env` without restarting.
- The window remembers its size and position.
- Log file `jiraf.log` in the config folder (`RUST_LOG` from the environment or `.env` sets the level).
- App icon from the jiraf mascot.

### Changed

- **Config folder.** `jiraf.toml` and `.env` live in `~/Library/Application Support/com.dmbuil.jiraf/`
  (or `$JIRAF_CONFIG_DIR`), along with the icon cache and the log. The first launch writes
  `jiraf.example.toml` and `.env.example` there.
- **Settings writes a clean `jiraf.toml`** without comments. The previous file is kept as
  `jiraf.toml.bak` on every save, and an unreadable one as `jiraf.toml.broken`. In `.env`, only the
  `JIRA_TOKEN` line is touched, and the file stays readable by its owner only.
- The token is re-read from `.env` on every reload, so a new token takes effect without a restart. A
  `JIRA_TOKEN` in the real environment still wins.
- The page talks to the app through Tauri commands instead of HTTP. Jira icons are served through a
  private `jiraf://` scheme.
- External links (issue keys, labels, JQL, comments) open in the default browser, and only links to
  your Jira.
- The top bar doubles as the window's title bar, in both themes, with room to drag the window.
- The filter box ignores accents and case.

### Removed

- The actix-web server and its `127.0.0.1:8080` port: the app opens no network port.
- The `jiraf path/to/jiraf.toml` command-line argument (use `JIRAF_CONFIG_DIR`).
- `[server] bind` is no longer used. Older files still load; Settings drops the key when it saves.

### Fixed

- Icon paths with encoded dot segments (`%2e%2e`) or backslashes are refused.
- Failed icon fetches are no longer cached by the window.
- A slow startup check can no longer overwrite the result of a newer Retry.
- Inline styles keep working under the app's Content Security Policy.

## [0.1.0] - 2026-10-14

First version: a stateless follow-up dashboard over Jira (Data Center) Plans, served by a local actix-web
server and opened in the browser.

### Added

- Bubbles (any JQL) in the top bar, a tree of Initiatives, Epics, Tasks and Sub-tasks, and a "Needs
  attention" view (blocked, late, no update, recently closed).
- Inline edits written straight to Jira: status transitions, priority, assignee, Target end, Due date and
  Extended Status.
- Tabs split by the "Activity Type" field, with pinned issues and an "Other" tab.
- Assignee avatars and Jira icons cached locally.
- Light and dark themes.

[Unreleased]: https://github.com/dmbuil/jiraf/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/dmbuil/jiraf/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/dmbuil/jiraf/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/dmbuil/jiraf/releases/tag/v0.1.0
