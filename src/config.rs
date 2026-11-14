//! Settings, stored as TOML in the user's config directory:
//! `~/.config/unbing/` (Linux), `%APPDATA%\unbing\` (Windows),
//! `~/Library/Application Support/unbing/` (macOS).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{info, warn};

use crate::bing;

const APP_DIR: &str = "unbing";
const CONFIG_FILE: &str = "config.toml";
pub const MIN_HISTORY: usize = 1;
pub const MAX_HISTORY: usize = 365;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot find the user's config directory")]
    NoConfigDir,
    #[error("cannot find a directory for the pictures")]
    NoImagesDir,
    #[error("I/O on {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("TOML serialization: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Size of the downloaded pictures.
///
/// Stored as `resolution`; the older `uhd = false` key (written by default by the first
/// versions, hence not a real choice) is ignored, so those settings move to 4K.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Resolution {
    /// 3840×2160: sharp on Retina / 4K screens (a few MB per picture).
    #[default]
    #[serde(rename = "4k")]
    Uhd,
    /// 1920×1080: lighter, enough for a 1080p screen.
    #[serde(rename = "1080p")]
    Hd,
}

/// Look of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follows the system setting.
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Bing market (`fr-FR`, `en-US`…): the picture of the day differs per market.
    pub market: String,
    /// `4k` (default) or `1080p`.
    pub resolution: Resolution,
    /// Pictures kept in the local folder.
    pub history_limit: usize,
    /// Apply the picture of the day automatically (app, `--apply`, `--daemon`).
    pub auto_apply: bool,
    /// Linux only: `auto`, `gnome`, `kde`, `hyprpaper`, `sway`, `swaybg`, `feh` or `none`.
    pub linux_backend: String,
    /// Pictures folder. Defaults to `~/Pictures/Unbing` (or equivalent).
    pub images_dir: Option<PathBuf>,
    /// Appearance: `auto`, `light` or `dark`.
    pub theme: Theme,
    /// Check GitHub for a new release on startup (and once a day while running).
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            market: "fr-FR".to_owned(),
            resolution: Resolution::Uhd,
            history_limit: 30,
            auto_apply: true,
            linux_backend: "auto".to_owned(),
            images_dir: None,
            theme: Theme::Auto,
            check_updates: true,
        }
    }
}

impl Config {
    /// Loads the settings. Never panics: missing file → defaults,
    /// corrupt file → kept as `.bak`, defaults, warning.
    pub fn load() -> Self {
        match config_path() {
            Ok(path) => Self::load_from(&path),
            Err(e) => {
                warn!("{e}, using default settings");
                Self::default()
            }
        }
    }

    pub fn load_from(path: &Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                info!("no settings yet, using defaults ({})", path.display());
                return Self::default();
            }
            Err(e) => {
                warn!("cannot read {} ({e}), using defaults", path.display());
                return Self::default();
            }
        };
        match toml::from_str::<Config>(&text) {
            Ok(cfg) => cfg.sanitized(),
            Err(e) => {
                let backup = path.with_extension("toml.bak");
                warn!("invalid settings ({e}); copy kept as {}, using defaults", backup.display());
                let _ = fs::copy(path, &backup);
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to(&config_path()?)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(&self.clone().sanitized())?;
        write_atomic(path, text.as_bytes())
    }

    /// Fixes out-of-range values instead of failing.
    pub fn sanitized(mut self) -> Self {
        if let Err(e) = bing::validate_market(&self.market) {
            warn!("{e}, back to fr-FR");
            self.market = Config::default().market;
        }
        self.history_limit = self.history_limit.clamp(MIN_HISTORY, MAX_HISTORY);
        self
    }

    /// Download the 4K (`_UHD`) version?
    pub fn uhd(&self) -> bool {
        self.resolution == Resolution::Uhd
    }

    /// Folder where the pictures are kept.
    pub fn images_dir(&self) -> Result<PathBuf, ConfigError> {
        if let Some(dir) = &self.images_dir {
            return Ok(dir.clone());
        }
        dirs::picture_dir()
            .map(|p| p.join("Unbing"))
            .or_else(|| dirs::data_dir().map(|p| p.join(APP_DIR).join("pictures")))
            .ok_or(ConfigError::NoImagesDir)
    }
}

/// `<config_dir>/unbing/config.toml`
pub fn config_path() -> Result<PathBuf, ConfigError> {
    Ok(app_dir()?.join(CONFIG_FILE))
}

/// `<config_dir>/unbing/`: settings and local index.
pub fn app_dir() -> Result<PathBuf, ConfigError> {
    dirs::config_dir().map(|p| p.join(APP_DIR)).ok_or(ConfigError::NoConfigDir)
}

/// Writes to a temporary file, then renames it: never a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let io_err = |source| ConfigError::Io { path: path.to_owned(), source };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_err)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(io_err)?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.market, "fr-FR");
        assert!(c.uhd(), "4K by default");
        assert_eq!(c.history_limit, 30);
        assert!(c.auto_apply);
    }

    #[test]
    fn roundtrip_and_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config { market: "en-US".into(), resolution: Resolution::Hd, history_limit: 12, ..Config::default() };
        cfg.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path), cfg);

        // A partial file is completed with defaults.
        fs::write(&path, "resolution = \"1080p\"\n").unwrap();
        let partial = Config::load_from(&path);
        assert_eq!(partial.resolution, Resolution::Hd);
        assert_eq!(partial.market, "fr-FR");
    }

    #[test]
    fn old_default_moves_to_4k() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Written as-is by the first versions, without the user choosing it.
        fs::write(&path, "market = \"fr-FR\"\nuhd = false\nhistory_limit = 30\n").unwrap();
        assert!(Config::load_from(&path).uhd());
    }

    #[test]
    fn corrupt_file_falls_back_and_keeps_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "market = [oops").unwrap();
        assert_eq!(Config::load_from(&path), Config::default());
        assert!(dir.path().join("config.toml.bak").exists());
    }

    #[test]
    fn out_of_range_values_are_fixed() {
        let c = Config { market: "fr-FR&n=99".into(), history_limit: 0, ..Config::default() }.sanitized();
        assert_eq!(c.market, "fr-FR");
        assert_eq!(c.history_limit, MIN_HISTORY);
    }
}
