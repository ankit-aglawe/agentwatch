//! Cross-platform path helpers.
//!
//! Uses XDG on Linux, `~/Library/Application Support/agentwatch` on macOS,
//! `%APPDATA%\agentwatch` on Windows - all via `directories-next` so we never
//! hand-construct platform-specific paths.

use std::path::PathBuf;

use directories_next::ProjectDirs;

const QUALIFIER: &str = "";
const ORG: &str = "";
const APP: &str = "agentwatch";

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORG, APP)
}

/// Where SQLite, config, and logs live.
pub fn data_dir() -> PathBuf {
    if let Some(dirs) = project_dirs() {
        return dirs.data_dir().to_path_buf();
    }
    // Last-resort fallback when HOME is unset (rare; CI hermetic shells).
    PathBuf::from(".agentwatch")
}

/// Default SQLite location.
pub fn db_path() -> PathBuf {
    data_dir().join("db.sqlite")
}

/// Config TOML location.
pub fn config_path() -> PathBuf {
    if let Some(dirs) = project_dirs() {
        return dirs.config_dir().join("config.toml");
    }
    PathBuf::from(".agentwatch").join("config.toml")
}

/// Daily-rotated log file location.
pub fn log_dir() -> PathBuf {
    if let Some(dirs) = project_dirs() {
        return dirs.data_dir().join("log");
    }
    PathBuf::from(".agentwatch").join("log")
}

/// User's $HOME - the root we resolve all adapter session paths against.
pub fn home_dir() -> Option<PathBuf> {
    directories_next::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
}
