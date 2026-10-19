# Contributing

Thank you for your interest in contributing to jiraf. Read the [README](README.md) for what the app
does and how it is set up; this file covers how to report problems and send changes.

## Reporting issues

- **Crashes, panics, a blank window, an app that won't start:** report them. jiraf should never crash;
  that is a bug.
- **Something in the dashboard looks wrong** (a missing issue, a wrong date, a bubble that shows
  nothing): first check the bubble's JQL in Jira itself. jiraf shows what Jira returns, so a JQL that
  finds the wrong issues in Jira finds them in jiraf too. If Jira and jiraf disagree, report it.
- **Errors from Jira** (a rejected token, an unknown field, a failed edit): say which Jira version you
  run (Data Center / Server, e.g. 10.3) and whether Advanced Roadmaps and ScriptRunner are installed.
- **Windows:** builds are made on every release but are less tested than macOS. Reports from Windows
  are especially welcome.

In every report, include the jiraf version (**About jiraf** in the app menu on macOS, or the release you
installed), your OS, and the relevant
lines from `jiraf.log` in the config folder (`RUST_LOG=debug` in `.env` gives more detail).

**Before you paste anything**, remove what belongs to your company: the Jira URL, project keys, issue
keys and summaries, people's names, and any JQL that reveals them. Never paste your `.env`, your
`JIRA_TOKEN` or your `jiraf.toml` as is.

A security problem (for example, a way for the page to open arbitrary URLs or to read the token) should
not go into a public issue with details: open an issue asking for a private contact, and the details
can follow there.

## Design decisions

These hold for every change; a change that breaks one needs a good reason in its pull request.

- **Stateless.** jiraf stores nothing but its config, an icon cache, a log and the window size. Jira is
  the only source of truth; edits go straight to Jira.
- **No network port, no remote code.** The page is plain HTML/JS under a strict CSP: no framework, no
  npm, no bundler, nothing loaded from the internet. It talks to Rust through Tauri commands only.
- **The token stays in Rust.** It never reaches the page, a log line or an error message.
- **Links only lead to your Jira.** The page can only open links under the configured Jira URL.
- **macOS and Windows.** Don't use a macOS-only API without a Windows path.
- **Stable ids.** Bubble and tab ids appear in URLs and in other bubbles' `exclude` lists: an existing
  id never changes.

## Rust conventions

- Rust 2024 edition; let-chains are fine.
- Rustdoc on every item, with an `# Errors` section on fallible functions. Comments are short and say
  why, not what.
- If a function needs to take ownership of the data, pass it by value; if it only reads it, pass a
  reference; if it changes it, pass a mutable reference.
- Use `anyhow::Result` with `.context(…)` inside the app. Commands hand errors to the page as a message
  a person can act on (`ApiError` in `api.rs`, or a `String`), never with the token in it.
- Test first: write the failing test, watch it fail, then implement. Pure logic is tested next to the
  code in `#[cfg(test)]` modules; code that calls Jira is tested against the local fake Jira in
  `settings.rs`, never a real one.
- Writes in the config folder are atomic (`.tmp` + rename) and keep the file's permissions.

## Build and test

You need Rust 1.88+, the Tauri CLI 2 (`cargo install tauri-cli --version "^2" --locked`) and
cargo-nextest (`cargo install cargo-nextest --locked`).

| Goal                                   | Command (from the repository root)                          |
| -------------------------------------- | ----------------------------------------------------------- |
| Run all tests                          | `cargo nextest run`                                         |
| Run some tests                         | `cargo nextest run settings::`                              |
| Lint (must stay warning-free)          | `cargo clippy --all-targets -- -D warnings`                 |
| Check the page scripts parse           | `node --check static/app.js && node --check static/settings.js` |
| Run the app against a config folder    | `JIRAF_CONFIG_DIR=/path/to/folder cargo tauri dev`          |
| Build the macOS app                    | `cargo tauri build --bundles app`                           |
| API docs                               | `cargo doc --no-deps --document-private-items --open`       |

Use `cargo nextest run`, not `cargo test`. Run the tests without `JIRA_TOKEN` in your environment: a
token there wins over `.env`, and the token-file tests skip themselves when it is set.

To try a change against your own Jira, copy your `jiraf.toml` to a scratch folder, point
`JIRAF_CONFIG_DIR` at it and pass the token through the environment only.

## Pull requests

- One topic per pull request, and one topic per commit.
- Commit summaries are imperative and say what changes for the person using jiraf (e.g. "Settings:
  Open config folder shows jiraf.toml in its folder"). Wrap the body at 72 characters and explain why
  when it isn't obvious.
- Update the README when behaviour or setup changes, and add a line under **Unreleased** in
  [CHANGELOG.md](CHANGELOG.md).
- Larger changes start with a short design in `docs/superpowers/specs/` and a plan in
  `docs/superpowers/plans/`, so they can be discussed before the code.
- CI runs the tests, clippy and the page checks on macOS and Windows; a pull request needs it green.
- Keep your branch up to date by rebasing rather than merging `main` into it, and allow edits from
  maintainers so small fixes can be made on the spot.
- Never commit `.env`, `jiraf.toml`, `icons/`, `jiraf.log` or `mockup/`. They are git-ignored because
  they hold a token or real Jira data.

## Releasing

Releases are cut from `main` by the maintainer. The workflow in `.github/workflows/build.yml` builds
the bundles (a universal macOS `.dmg`, a Windows `.exe` and `.msi`) and attaches them to the release:

1. Move the **Unreleased** notes in `CHANGELOG.md` under the new version and set `version` in
   `Cargo.toml`.
2. Commit, push, then tag and push the tag: `git tag -s vX.Y.Z -m "…" && git push origin vX.Y.Z`.
3. Create the release straight away, with that version's changelog section as notes:
   `gh release create vX.Y.Z --verify-tag --notes-file <notes>`.
4. The tag's workflow run attaches the bundles. To rebuild an existing release, run
   `gh workflow run build.yml -f tag=vX.Y.Z`.

Versions follow [Semantic Versioning](https://semver.org/).

## Coding agents

Coding agents (Claude Code, Codex and others) read [AGENTS.md](AGENTS.md): the repository layout, the
commands, and the rules above in a compact form. Keep it in step with this file.

---

This guide is adapted from the
[CONTRIBUTING.md of gtema/openstack](https://github.com/gtema/openstack/blob/main/CONTRIBUTING.md)
(Apache-2.0).
