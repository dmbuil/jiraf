//! Logging to `jiraf.log` in the config folder: an app started from Finder has no terminal.

use crate::config;
use anyhow::Context;
use std::path::Path;

/// The `env_logger` filter: `RUST_LOG` from the process environment, else from `.env`, else `info`.
/// Empty values count as unset.
pub fn log_filter(process: Option<String>, env_file: Option<String>) -> String {
    [process, env_file].into_iter().flatten().find(|v| !v.trim().is_empty()).unwrap_or_else(|| "info".into())
}

/// Send all logging to `dir/jiraf.log` (appended), with the [`log_filter`] level.
///
/// # Errors
/// If the folder or the log file can't be created.
pub fn init(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join("jiraf.log");
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).with_context(|| format!("opening {}", path.display()))?;
    let from_env_file = config::env_file_var(&dir.join(config::ENV_FILE), "RUST_LOG").ok().flatten();
    env_logger::Builder::new()
        .parse_filters(&log_filter(std::env::var("RUST_LOG").ok(), from_env_file))
        .target(env_logger::Target::Pipe(Box::new(file)))
        .init();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_filter_precedence() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(log_filter(s("debug"), s("warn")), "debug");
        assert_eq!(log_filter(None, s("warn")), "warn");
        assert_eq!(log_filter(s(""), None), "info");
        assert_eq!(log_filter(None, None), "info");
    }
}
