//! Setting the wallpaper, one implementation per OS (cfg(target_os)).

use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Error)]
pub enum WallpaperError {
    #[error("file {0} does not exist")]
    MissingFile(PathBuf),
    #[error("cannot run `{cmd}`: {source}")]
    Spawn { cmd: String, source: std::io::Error },
    #[error("`{cmd}` failed ({status}): {stderr}")]
    CommandFailed { cmd: String, status: String, stderr: String },
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    #[error("system error: {0}")]
    Os(String),
}

/// Common interface for every OS.
pub trait WallpaperSetter: Send {
    /// Readable name of the mechanism (shown in the UI and logs).
    fn name(&self) -> &'static str;
    fn set_wallpaper(&self, path: &Path) -> Result<(), WallpaperError>;
    /// Is the wallpaper lost on its own (e.g. swaybg stopped at logout)?
    fn needs_reapply(&self) -> bool {
        false
    }
}

/// Finds the available mechanism. `None` disables the daily change.
/// `linux_backend` is only used on Linux (`auto` or a forced name).
#[allow(unused_variables)]
pub fn detect(linux_backend: &str) -> Option<Box<dyn WallpaperSetter>> {
    #[cfg(target_os = "macos")]
    return Some(Box::new(macos::MacOs));
    #[cfg(target_os = "windows")]
    return Some(Box::new(windows::Windows));
    #[cfg(target_os = "linux")]
    return linux::detect(linux_backend);
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return None;
}

/// Absolute, existing path: system tools require it.
#[allow(dead_code)]
pub(crate) fn absolute(path: &Path) -> Result<PathBuf, WallpaperError> {
    std::fs::canonicalize(path).map_err(|_| WallpaperError::MissingFile(path.to_owned()))
}

/// Runs a command and turns a non-zero exit code into a readable error.
#[allow(dead_code)]
pub(crate) fn run(cmd: &mut Command) -> Result<(), WallpaperError> {
    let label = format!("{}", cmd.get_program().to_string_lossy());
    tracing::debug!(?cmd, "running");
    let out = cmd.output().map_err(|source| WallpaperError::Spawn { cmd: label.clone(), source })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(WallpaperError::CommandFailed {
            cmd: label,
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        })
    }
}
