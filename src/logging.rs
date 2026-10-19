//! Logging to `jiraf.log` in the config folder: an app started from Finder has no terminal.

use crate::config;
use anyhow::Context;
use std::path::Path;

/// The `env_logger` filter: `RUST_LOG` from the process environment, else from `.env`, else `info`.
/// Empty values count as unset.
pub fn log_filter(process: Option<String>, env_file: Option<String>) -> String {
    [process, env_file].into_iter().flatten().find(|v| !v.trim().is_empty()).unwrap_or_else(|| "info".into())
}

/// Past this size `jiraf.log` is emptied at the next launch; older entries aren't kept.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// Open `path` for appending, emptying it first when it is over `max` bytes.
///
/// # Errors
/// If the file can't be opened or created.
fn open_log(path: &Path, max: u64) -> std::io::Result<std::fs::File> {
    let over = std::fs::metadata(path).is_ok_and(|m| m.len() > max);
    std::fs::OpenOptions::new().create(true).append(!over).write(true).truncate(over).open(path)
}

/// Send all logging to `dir/jiraf.log` (appended; started over once past 4 MB), with the
/// [`log_filter`] level.
///
/// # Errors
/// If the folder or the log file can't be created.
pub fn init(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join("jiraf.log");
    let file = open_log(&path, MAX_LOG_BYTES).with_context(|| format!("opening {}", path.display()))?;
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

    #[test]
    fn log_starts_over_once_it_passes_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jiraf.log");
        std::fs::write(&path, "1234").unwrap();
        drop(open_log(&path, 4).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1234", "at the cap: kept");
        std::fs::write(&path, "12345").unwrap();
        drop(open_log(&path, 4).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "", "over the cap: emptied");
        drop(open_log(&dir.path().join("new.log"), 4).unwrap());
        assert!(dir.path().join("new.log").exists(), "missing: created");
    }
}
