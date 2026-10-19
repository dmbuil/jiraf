# Security policy

jiraf holds a Jira personal access token that can read and edit Jira as you. This page says which
versions get security fixes, how to report a vulnerability, and what jiraf does (and doesn't do) to
protect that token and your Jira data.

## Supported versions

Only the latest release gets security fixes. There are no backports: update to the newest version on
the [releases page](https://github.com/dmbuil/jiraf/releases).

| Version         | Supported |
| --------------- | --------- |
| latest (0.2.x)  | yes       |
| older           | no        |

## Reporting a vulnerability

Please **don't put the details in a public issue**. Open an issue titled "Security contact" with no
details, and the maintainer will reply with a private way to send them.

A useful report says:

- which jiraf version and OS you used;
- what an attacker can do (read the token, open a non-Jira URL, read or write files, run code…) and
  what they need first (a malicious Jira issue, a crafted config file, local access…);
- the steps to reproduce it, with **your own data removed**: no token, no real Jira URL, project
  keys, issue keys or names. A made-up issue on `https://jira.example.com` is fine.

You can expect a first answer within a week. Once a fix is released, the changelog says what was
fixed, and you are credited unless you'd rather not be.

## What jiraf protects

The two things worth protecting are the **token** and the **Jira data** shown in the dashboard.
Issue text (summaries, comments, labels, names) comes from Jira and is treated as untrusted: anyone
who can edit an issue can put text in front of you.

| Protection                       | How                                                                                                                                                                                                                                                           |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **No network port**              | The page talks to the Rust side over Tauri's in-process IPC. No other program or website can reach jiraf.                                                                                                                                                     |
| **The token stays in Rust**      | It is read from `.env` (or the environment) and only ever sent to Jira as an `Authorization: Bearer` header. It is never sent to the page, written to `jiraf.log` or put in an error message. The Settings screen shows it masked and only accepts a new one. |
| **Token file is private**        | `.env` is created readable by its owner only (Unix `0600`), keeps its permissions on every save, and only its `JIRA_TOKEN` line is ever rewritten.                                                                                                            |
| **No remote code in the window** | A strict Content-Security-Policy (`tauri.conf.json`) allows only jiraf's own scripts and styles; the page can't load anything from the internet. There is no npm, bundler or framework.                                                                       |
| **Jira text stays text**         | Everything from Jira or the config is escaped before it goes into the page, so issue text can't inject HTML or script.                                                                                                                                        |
| **Links only open your Jira**    | `open_external` opens a URL in the browser only if it is under the configured Jira URL; anything else is refused and logged.                                                                                                                                  |
| **Issue keys are checked**       | Keys must look like `ABC-123` before they go into a Jira API path.                                                                                                                                                                                            |
| **Icon proxy is fenced**         | Jira icons and avatars are fetched by Rust (with the token) and served to the page on the private `jiraf://` scheme. Only Jira's avatar and image paths are allowed; `..`, `\`, their percent-encoded forms and `//` are refused.                             |
| **Minimal page permissions**     | The window gets Tauri's core APIs plus window dragging and maximizing (`capabilities/default.json`). The opener plugin is used from Rust only, never granted to the page.                                                                                     |
| **Atomic config writes**         | `jiraf.toml` and `.env` are written to a `.tmp` file and renamed into place, so a crash never leaves half a file. The previous `jiraf.toml` is kept as `jiraf.toml.bak`.                                                                                      |

## What jiraf doesn't protect against

- **Someone with access to your user account.** The token sits in plain text in `.env` in the config
  folder (`~/Library/Application Support/com.dmbuil.jiraf/`, or `%APPDATA%\com.dmbuil.jiraf\` on
  Windows). Any program running as you can read it. jiraf doesn't use the macOS Keychain or the
  Windows Credential Manager.
- **A Jira URL with `http://`.** Settings accepts `http` as well as `https`; with `http` the token
  travels unencrypted. Use `https`.
- **What the token itself can do.** jiraf acts as you: it can see and edit everything your Jira
  account can. Give the token an expiry date in Jira and revoke it when you stop using jiraf.
- **Unsigned builds.** The macOS and Windows builds are not code-signed, so macOS and Windows can't
  vouch for them. Download them only from this repository's releases page; GitHub lists a SHA-256
  digest for every file there, which you can compare with `shasum -a 256 <file>` (macOS) or
  `Get-FileHash <file>` (Windows). Or build jiraf yourself from source.
- **The log file.** `jiraf.log` never contains the token, but it can contain your Jira URL, issue
  keys and error messages from Jira. Remove those before sharing it.

## For contributors

Changes must keep the rules above. In particular, never:

- send the token to the page, a log line or an error message;
- open a network port, or load remote scripts or styles into the page;
- let the page open URLs outside the configured Jira;
- weaken the icon path check (`check_icon_path`) or the issue key check (`check_key`) in
  `src/api.rs`;
- commit `.env`, `jiraf.toml`, `icons/`, `jiraf.log` or `mockup/` (they hold a token or real Jira
  data).

See [CONTRIBUTING.md](CONTRIBUTING.md) for the rest.
