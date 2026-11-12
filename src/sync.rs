//! Flow shared by the app and the headless modes:
//! metadata → download → cache → set the wallpaper.
//!
//! Network budget when running on its own: 1 metadata request per Bing publication
//! (every 24 h, at the market's own time), then 1 picture if it isn't cached yet.
//! Between two publications: no request.

use chrono::{DateTime, TimeDelta, Utc};
use std::path::PathBuf;
use thiserror::Error;
use tracing::{info, warn};

use crate::bing::{BingClient, BingError, BingImage};
use crate::cache::{Cache, CacheError};
use crate::config::Config;
use crate::wallpaper::{self, WallpaperError, WallpaperSetter};

#[derive(Debug, Error)]
pub enum SyncError {
    #[error(transparent)]
    Bing(#[from] BingError),
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error(transparent)]
    Wallpaper(#[from] WallpaperError),
    #[error("no way to change the wallpaper was found on this system")]
    NoBackend,
}

/// Steps reported to the caller (shown by the UI).
#[derive(Debug, Clone)]
pub enum Progress {
    Metadata,
    Downloading(String),
    Applying,
}

pub struct DailyOutcome {
    pub image: BingImage,
    pub path: PathBuf,
    /// `Some(backend)` if the wallpaper was (re)applied, `None` if it already was.
    pub applied: Option<&'static str>,
}

/// Bing publishes a picture every 24 h.
const PUBLICATION_PERIOD: TimeDelta = TimeDelta::hours(24);
/// If the expected picture isn't there yet (Bing late, daylight saving change),
/// don't check again before this delay.
const LATE_RECHECK: TimeDelta = TimeDelta::hours(3);

pub struct Engine {
    pub cfg: Config,
    pub cache: Cache,
    client: BingClient,
    setter: Option<Box<dyn WallpaperSetter>>,
}

pub fn today() -> String {
    chrono::Local::now().date_naive().format("%Y-%m-%d").to_string()
}

/// When a new picture is expected.
fn next_publication(images: &[BingImage], now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let next = images.first()?.start_utc()? + PUBLICATION_PERIOD;
    Some(if next > now { next } else { now + LATE_RECHECK })
}

impl Engine {
    pub fn new(cfg: Config) -> Result<Self, SyncError> {
        let cache = Cache::open_default(&cfg)?;
        let setter = wallpaper::detect(&cfg.linux_backend);
        Ok(Self { cfg, cache, client: BingClient::new(), setter })
    }

    /// Metadata of the last 8 days, from the index while still current.
    pub fn metadata(&mut self, progress: &mut dyn FnMut(Progress)) -> Result<Vec<BingImage>, SyncError> {
        let day = today();
        let now = Utc::now();
        if let Some(images) = self.cache.metadata_for(&day, now.timestamp(), &self.cfg.market) {
            info!("no new picture expected yet, no request");
            return Ok(images.to_vec());
        }
        progress(Progress::Metadata);
        self.cache.count_request(&day);
        let result = self.client.fetch_archive(&self.cfg.market);
        // The counter is saved even on failure; never any pruning here.
        let saved = match &result {
            Ok(images) => {
                let due = next_publication(images, now);
                if let Some(due) = due {
                    info!("next picture expected: {}", due.with_timezone(&chrono::Local));
                }
                self.cache.record_fetch(&day, &self.cfg.market, images.clone(), due.map(|d| d.timestamp()))
            }
            Err(_) => self.cache.save(),
        };
        if let Err(e) = saved {
            warn!("index not saved: {e}");
        }
        Ok(result?)
    }

    /// Next publication expected for the configured market.
    pub fn next_due(&self) -> Option<DateTime<Utc>> {
        self.cache.next_due(&self.cfg.market).and_then(|t| DateTime::from_timestamp(t, 0))
    }

    /// Is this picture already the wallpaper (and does it stay so on its own)?
    pub fn is_applied(&self, path: &std::path::Path) -> bool {
        let needs = self.setter.as_ref().is_some_and(|s| s.needs_reapply());
        !needs && self.cache.applied_path().as_deref() == Some(path)
    }

    /// Last known metadata (offline display).
    pub fn offline_metadata(&self) -> Option<Vec<BingImage>> {
        self.cache.last_metadata(&self.cfg.market).map(<[_]>::to_vec)
    }

    /// Local path of the picture, downloaded only if it isn't cached.
    pub fn ensure_image(&mut self, img: &BingImage, progress: &mut dyn FnMut(Progress)) -> Result<PathBuf, SyncError> {
        if let Some(path) = self.cache.lookup(&img.id(), self.cfg.uhd) {
            info!("picture already cached: {}", path.display());
            return Ok(path);
        }
        progress(Progress::Downloading(img.title.clone()));
        let day = today();
        let want_uhd = self.cfg.uhd;
        self.cache.count_request(&day);
        let first = self.client.download(&img.image_url(want_uhd)?);
        let (bytes, got_uhd, uhd_unavailable) = match first {
            Ok(b) => (b, want_uhd, false),
            // Some pictures have no UHD version: fall back to 1080p.
            Err(BingError::Status(404)) if want_uhd => {
                warn!("no UHD version of {}, falling back to 1920x1080", img.id());
                self.cache.count_request(&day);
                (self.client.download(&img.image_url(false)?)?, false, true)
            }
            Err(e) => {
                let _ = self.cache.save();
                return Err(e.into());
            }
        };
        let path = self.cache.store(img, &self.cfg.market, got_uhd, uhd_unavailable, &bytes)?;
        // Prune only after a successful download.
        self.cache.prune(self.cfg.history_limit)?;
        Ok(path)
    }

    /// Sets a local picture as wallpaper.
    pub fn apply(&mut self, path: &std::path::Path) -> Result<&'static str, SyncError> {
        let setter = self.setter.as_ref().ok_or(SyncError::NoBackend)?;
        setter.set_wallpaper(path)?;
        let name = setter.name();
        self.cache.set_applied(path)?;
        info!("wallpaper set with {name}: {}", path.display());
        Ok(name)
    }

    /// Daily task: fetch the picture of the day and set it.
    /// On error, the current wallpaper and the cache stay untouched.
    /// A picture already set isn't set again: if the user picked another wallpaper
    /// in the meantime, it is only replaced by the next picture.
    pub fn daily(&mut self, progress: &mut dyn FnMut(Progress)) -> Result<DailyOutcome, SyncError> {
        if self.setter.is_none() {
            return Err(SyncError::NoBackend);
        }
        let images = self.metadata(progress)?;
        let image = images.into_iter().next().ok_or(BingError::Empty)?;
        let path = self.ensure_image(&image, progress)?;
        if self.is_applied(&path) {
            return Ok(DailyOutcome { image, path, applied: None });
        }
        progress(Progress::Applying);
        let backend = self.apply(&path)?;
        Ok(DailyOutcome { image, path, applied: Some(backend) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(fullstartdate: &str) -> BingImage {
        BingImage {
            url: "/th?id=OHR.X_FR-FR1_1920x1080.jpg".into(),
            startdate: String::new(),
            fullstartdate: fullstartdate.into(),
            enddate: String::new(),
            title: String::new(),
            copyright: String::new(),
            hsh: String::new(),
        }
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn next_publication_is_24h_after_latest_start() {
        let images = [img("202611112300"), img("202611102300")];
        let now = utc("2026-11-12T08:00:00Z");
        assert_eq!(next_publication(&images, now), Some(utc("2026-11-12T23:00:00Z")));
    }

    #[test]
    fn late_publication_waits_before_rechecking() {
        // The picture expected at 23:00 still isn't there at 23:30: no request before 02:30.
        let images = [img("202611112300")];
        let now = utc("2026-11-12T23:30:00Z");
        assert_eq!(next_publication(&images, now), Some(utc("2026-11-13T02:30:00Z")));
    }

    #[test]
    fn unknown_start_means_no_deadline() {
        assert_eq!(next_publication(&[img("")], Utc::now()), None);
        assert_eq!(next_publication(&[], Utc::now()), None);
    }
}
