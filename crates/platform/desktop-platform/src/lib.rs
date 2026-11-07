//! Windows, macOS, and Linux platform integration boundary.

use std::{env, path::PathBuf};

const LAYOUT_FILE: &str = "workbench-layout.json";

/// Returns the platform configuration path for the persisted workbench layout.
///
/// `IDEAGIT_LAYOUT_PATH` is an explicit test/portable override. Missing platform configuration
/// variables disable persistence rather than falling back to the current working directory.
#[must_use]
pub fn workbench_layout_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("IDEAGIT_LAYOUT_PATH") {
        return Some(PathBuf::from(path));
    }
    platform_layout_path()
}

#[cfg(target_os = "windows")]
fn platform_layout_path() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("IdeaGit").join(LAYOUT_FILE))
}

#[cfg(target_os = "macos")]
fn platform_layout_path() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).map(|path| {
        path.join("Library/Application Support/IdeaGit")
            .join(LAYOUT_FILE)
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_layout_path() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|path| path.join("ideagit").join(LAYOUT_FILE))
}

#[cfg(not(any(target_os = "windows", unix)))]
fn platform_layout_path() -> Option<PathBuf> {
    None
}
