//! Local pictures folder + JSON index (metadata, current wallpaper, request log).
//!
//! Rules:
//! - only files listed in the index are ever deleted (never an unknown file);
//! - the current wallpaper is never deleted;
//! - pruning only happens after a successful download, never on a network error.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::bing::BingImage;
use crate::config::{self, Config, ConfigError};

const INDEX_FILE: &str = "index.json";

#[derive(Debug, Error)]
pub enum CacheError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("I/O on {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("index serialization: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    /// File name in the pictures folder.
    pub file: String,
    pub id: String,
    pub startdate: String,
    pub title: String,
    pub copyright: String,
    pub market: String,
    /// Resolution actually downloaded.
    pub uhd: bool,
    /// UHD was asked for but doesn't exist (the 1080p one was taken).
    #[serde(default)]
    pub uhd_unavailable: bool,
}

/// Last metadata answer, reused until a new picture is expected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchRecord {
    pub day: String,
    pub market: String,
    pub images: Vec<BingImage>,
    /// Unix time (UTC) before which Bing won't publish a new picture.
    #[serde(default)]
    pub valid_until: Option<i64>,
}

/// Local count of today's HTTP requests (shown in the UI, never sent anywhere).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestLog {
    pub day: String,
    pub count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Index {
    pub entries: Vec<CacheEntry>,
    pub last_fetch: Option<FetchRecord>,
    /// File name of the current wallpaper.
    pub applied: Option<String>,
    pub requests: RequestLog,
}

pub struct Cache {
    images_dir: PathBuf,
    index_path: PathBuf,
    pub index: Index,
}

impl Cache {
    pub fn open_default(cfg: &Config) -> Result<Self, CacheError> {
        Self::open(cfg.images_dir()?, config::app_dir()?.join(INDEX_FILE))
    }

    pub fn open(images_dir: PathBuf, index_path: PathBuf) -> Result<Self, CacheError> {
        fs::create_dir_all(&images_dir).map_err(|source| CacheError::Io { path: images_dir.clone(), source })?;
        let index = match fs::read_to_string(&index_path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                // Start again from an empty index, but leave every picture alone.
                warn!("unreadable index {} ({e}), starting from an empty one", index_path.display());
                Index::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(source) => {
                return Err(CacheError::Io { path: index_path, source });
            }
        };
        Ok(Self { images_dir, index_path, index })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn images_dir(&self) -> &Path {
        &self.images_dir
    }

    pub fn save(&self) -> Result<(), CacheError> {
        let json = serde_json::to_vec_pretty(&self.index)?;
        config::write_atomic(&self.index_path, &json)?;
        Ok(())
    }

    /// Local picture for this id and resolution, if any.
    pub fn lookup(&self, id: &str, want_uhd: bool) -> Option<PathBuf> {
        let usable = |e: &&CacheEntry| self.images_dir.join(&e.file).is_file();
        let same_id = || self.index.entries.iter().filter(|e| e.id == id).filter(usable);
        same_id()
            .find(|e| e.uhd == want_uhd)
            // A UHD file also does for a 1080p request.
            .or_else(|| same_id().find(|e| !want_uhd && e.uhd))
            // UHD asked for but not offered by Bing: the 1080p file will do.
            .or_else(|| same_id().find(|e| want_uhd && e.uhd_unavailable))
            .map(|e| self.images_dir.join(&e.file))
    }

    /// Writes the picture (atomically) and adds it to the index.
    pub fn store(
        &mut self,
        img: &BingImage,
        market: &str,
        uhd: bool,
        uhd_unavailable: bool,
        bytes: &[u8],
    ) -> Result<PathBuf, CacheError> {
        let id = img.id();
        let file = format!("{}_{}.jpg", id, if uhd { "UHD" } else { "1080" });
        let path = self.images_dir.join(&file);
        config::write_atomic(&path, bytes)?;
        self.index.entries.retain(|e| e.file != file);
        self.index.entries.push(CacheEntry {
            file,
            id,
            startdate: img.startdate.clone(),
            title: img.title.clone(),
            copyright: img.copyright.clone(),
            market: market.to_owned(),
            uhd,
            uhd_unavailable,
        });
        self.save()?;
        info!("picture saved: {}", path.display());
        Ok(path)
    }

    /// Keeps the `limit` most recent pictures (plus the current wallpaper).
    pub fn prune(&mut self, limit: usize) -> Result<(), CacheError> {
        // Forget entries whose file is gone (deleted by hand).
        let dir = self.images_dir.clone();
        self.index.entries.retain(|e| dir.join(&e.file).is_file());

        let evict = plan_eviction(&self.index.entries, limit, self.index.applied.as_deref());
        if evict.is_empty() {
            return self.save();
        }
        let mut kept = Vec::with_capacity(self.index.entries.len());
        for (i, entry) in std::mem::take(&mut self.index.entries).into_iter().enumerate() {
            if !evict.contains(&i) {
                kept.push(entry);
                continue;
            }
            let path = self.images_dir.join(&entry.file);
            match fs::remove_file(&path) {
                Ok(()) => debug!("pruned: {}", path.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    warn!("cannot delete {}: {e}", path.display());
                    kept.push(entry);
                }
            }
        }
        self.index.entries = kept;
        self.save()
    }

    pub fn set_applied(&mut self, path: &Path) -> Result<(), CacheError> {
        self.index.applied = path.file_name().map(|f| f.to_string_lossy().into_owned());
        self.save()
    }

    /// Id of the current wallpaper.
    pub fn applied_id(&self) -> Option<String> {
        let file = self.index.applied.as_deref()?;
        self.index.entries.iter().find(|e| e.file == file).map(|e| e.id.clone())
    }

    pub fn applied_path(&self) -> Option<PathBuf> {
        self.index.applied.as_ref().map(|f| self.images_dir.join(f))
    }

    /// Metadata still current for this market (no new picture expected before
    /// `valid_until`, or, when unknown, same local day).
    pub fn metadata_for(&self, day: &str, now: i64, market: &str) -> Option<&[BingImage]> {
        self.index
            .last_fetch
            .as_ref()
            .filter(|r| r.market == market)
            .filter(|r| match r.valid_until {
                Some(until) => now < until,
                None => r.day == day,
            })
            .map(|r| r.images.as_slice())
    }

    /// Next publication expected for this market (Unix time, UTC).
    pub fn next_due(&self, market: &str) -> Option<i64> {
        self.index.last_fetch.as_ref().filter(|r| r.market == market).and_then(|r| r.valid_until)
    }

    /// Last known metadata for this market, whatever the day (offline mode).
    pub fn last_metadata(&self, market: &str) -> Option<&[BingImage]> {
        self.index.last_fetch.as_ref().filter(|r| r.market == market).map(|r| r.images.as_slice())
    }

    pub fn record_fetch(
        &mut self,
        day: &str,
        market: &str,
        images: Vec<BingImage>,
        valid_until: Option<i64>,
    ) -> Result<(), CacheError> {
        self.index.last_fetch =
            Some(FetchRecord { day: day.to_owned(), market: market.to_owned(), images, valid_until });
        self.save()
    }

    pub fn count_request(&mut self, day: &str) {
        if self.index.requests.day != day {
            self.index.requests = RequestLog { day: day.to_owned(), count: 0 };
        }
        self.index.requests.count += 1;
    }

    pub fn requests_on(&self, day: &str) -> u32 {
        if self.index.requests.day == day { self.index.requests.count } else { 0 }
    }
}

/// Indices of the entries to delete: everything past the `limit` most recent ones,
/// except the current wallpaper.
fn plan_eviction(entries: &[CacheEntry], limit: usize, applied: Option<&str>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    // Most recent first; on the same date, UHD before 1080p.
    order.sort_by(|&a, &b| {
        let (ea, eb) = (&entries[a], &entries[b]);
        eb.startdate.cmp(&ea.startdate).then(eb.uhd.cmp(&ea.uhd))
    });
    order.into_iter().skip(limit).filter(|&i| Some(entries[i].file.as_str()) != applied).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(day: u32) -> BingImage {
        BingImage {
            url: format!("/th?id=OHR.Img{day}_FR-FR1_1920x1080.jpg&pid=hp"),
            startdate: format!("202611{day:02}"),
            fullstartdate: String::new(),
            enddate: String::new(),
            title: format!("Titre {day}"),
            copyright: "© test".into(),
            hsh: String::new(),
        }
    }

    fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::open(dir.path().join("img"), dir.path().join("index.json")).unwrap();
        (dir, c)
    }

    #[test]
    fn store_lookup_and_reload() {
        let (dir, mut c) = cache();
        let img = image(1);
        let p = c.store(&img, "fr-FR", false, false, b"\xFF\xD8\xFFdata").unwrap();
        assert!(p.is_file());
        assert_eq!(c.lookup(&img.id(), false), Some(p.clone()));
        assert_eq!(c.lookup(&img.id(), true), None, "pas d'UHD en cache");

        let reopened = Cache::open(dir.path().join("img"), dir.path().join("index.json")).unwrap();
        assert_eq!(reopened.lookup(&img.id(), false), Some(p));
    }

    #[test]
    fn uhd_fallback_and_upgrade_rules() {
        let (_dir, mut c) = cache();
        let img = image(2);
        // UHD asked for, not available: the 1080p one was stored.
        let p = c.store(&img, "fr-FR", false, true, b"x").unwrap();
        assert_eq!(c.lookup(&img.id(), true), Some(p));
        // A UHD file also serves a 1080p request.
        let img3 = image(3);
        let uhd = c.store(&img3, "fr-FR", true, false, b"x").unwrap();
        assert_eq!(c.lookup(&img3.id(), false), Some(uhd));
    }

    #[test]
    fn prune_keeps_recent_and_applied() {
        let (_dir, mut c) = cache();
        let mut paths = Vec::new();
        for day in 1..=6 {
            paths.push(c.store(&image(day), "fr-FR", false, false, b"x").unwrap());
        }
        // The day-1 picture (the oldest) is the current wallpaper.
        c.set_applied(&paths[0]).unwrap();
        c.prune(3).unwrap();

        let alive: Vec<bool> = paths.iter().map(|p| p.is_file()).collect();
        assert_eq!(alive, [true, false, false, true, true, true]);
        assert_eq!(c.index.entries.len(), 4);
    }

    #[test]
    fn prune_never_touches_unknown_files() {
        let (_dir, mut c) = cache();
        let foreign = c.images_dir().join("photo_perso.jpg");
        fs::write(&foreign, b"x").unwrap();
        for day in 1..=3 {
            c.store(&image(day), "fr-FR", false, false, b"x").unwrap();
        }
        c.prune(1).unwrap();
        assert!(foreign.is_file());
    }

    #[test]
    fn corrupt_index_does_not_delete_images() {
        let (dir, mut c) = cache();
        let p = c.store(&image(4), "fr-FR", false, false, b"x").unwrap();
        fs::write(dir.path().join("index.json"), "{not json").unwrap();
        let reopened = Cache::open(dir.path().join("img"), dir.path().join("index.json")).unwrap();
        assert!(reopened.index.entries.is_empty());
        assert!(p.is_file());
    }

    #[test]
    fn metadata_is_reused_only_same_day_and_market() {
        let (_dir, mut c) = cache();
        // No known deadline: reused on the same local day only.
        c.record_fetch("2026-11-12", "fr-FR", vec![image(12)], None).unwrap();
        assert!(c.metadata_for("2026-11-12", 0, "fr-FR").is_some());
        assert!(c.metadata_for("2026-11-13", 0, "fr-FR").is_none());
        assert!(c.metadata_for("2026-11-12", 0, "en-US").is_none());
        assert!(c.last_metadata("fr-FR").is_some());

        // With a deadline: reused until the next publication, even past midnight.
        c.record_fetch("2026-11-12", "en-US", vec![image(12)], Some(1_000)).unwrap();
        assert!(c.metadata_for("2026-11-13", 999, "en-US").is_some());
        assert!(c.metadata_for("2026-11-12", 1_000, "en-US").is_none());
        assert_eq!(c.next_due("en-US"), Some(1_000));
        assert_eq!(c.next_due("fr-FR"), None);
    }

    #[test]
    fn request_counter_resets_daily() {
        let (_dir, mut c) = cache();
        c.count_request("2026-11-12");
        c.count_request("2026-11-12");
        assert_eq!(c.requests_on("2026-11-12"), 2);
        c.count_request("2026-11-13");
        assert_eq!(c.requests_on("2026-11-13"), 1);
        assert_eq!(c.requests_on("2026-11-12"), 0);
    }
}
