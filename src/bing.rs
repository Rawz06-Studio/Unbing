//! Minimal client for Bing's public `HPImageArchive` API.
//!
//! Only `www.bing.com` is contacted, over HTTPS, without following redirects,
//! with a 10 s timeout and a single retry on transient failures.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

/// The only origin any request may go to.
pub const BING_ORIGIN: &str = "https://www.bing.com";
/// Days of history the API exposes.
pub const HISTORY_LEN: usize = 8;
/// Global timeout (connect + transfer) of each request.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Pause before the single retry.
const RETRY_DELAY: Duration = Duration::from_secs(2);
/// Largest accepted picture (UHD ones are a few MB).
const MAX_IMAGE_BYTES: u64 = 40 * 1024 * 1024;
/// Largest accepted metadata JSON.
const MAX_JSON_BYTES: u64 = 1024 * 1024;

const RES_1080: &str = "_1920x1080";
const RES_UHD: &str = "_UHD";

/// Markets offered in Settings (code, label).
pub const MARKETS: &[(&str, &str)] = &[
    ("fr-FR", "France"),
    ("fr-CA", "Canada (French)"),
    ("en-US", "United States"),
    ("en-GB", "United Kingdom"),
    ("en-CA", "Canada (English)"),
    ("en-AU", "Australia"),
    ("en-IN", "India"),
    ("de-DE", "Germany"),
    ("es-ES", "Spain"),
    ("it-IT", "Italy"),
    ("ja-JP", "Japan"),
    ("zh-CN", "China"),
    ("pt-BR", "Brazil"),
];

#[derive(Debug, Error)]
pub enum BingError {
    #[error("network: {0}")]
    Network(String),
    #[error("the server answered HTTP {0}")]
    Status(u16),
    #[error("unexpected JSON answer (has the API changed?): {0}")]
    Json(#[from] serde_json::Error),
    #[error("no picture in Bing's answer")]
    Empty,
    #[error("the downloaded file is not a JPEG picture")]
    NotJpeg,
    #[error("URL refused (only {BING_ORIGIN} is allowed): {0}")]
    ForbiddenUrl(String),
    #[error("invalid market code: {0:?}")]
    InvalidMarket(String),
}

impl BingError {
    /// Could a retry succeed?
    fn is_transient(&self) -> bool {
        match self {
            BingError::Network(_) => true,
            BingError::Status(code) => *code == 429 || *code >= 500,
            _ => false,
        }
    }
}

impl From<ureq::Error> for BingError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::StatusCode(code) => BingError::Status(code),
            other => BingError::Network(other.to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Archive {
    #[serde(default)]
    images: Vec<BingImage>,
}

/// One entry of `images[]`. Unknown fields are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BingImage {
    /// Relative path, e.g. `/th?id=OHR.Foo_FR-FR123_1920x1080.jpg&rf=...&pid=hp`.
    #[serde(default)]
    pub url: String,
    /// Start date, `YYYYMMDD` (UTC date when the picture goes live).
    #[serde(default)]
    pub startdate: String,
    /// When the picture goes live, `YYYYMMDDHHMM` in UTC (e.g. `202611112300` for fr-FR:
    /// midnight in Paris). This field gives the picture's real "day".
    #[serde(default)]
    pub fullstartdate: String,
    #[serde(default)]
    pub enddate: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub copyright: String,
    #[serde(default)]
    pub hsh: String,
}

impl BingImage {
    /// Stable id, safe as a file name, e.g. `OHR.Foo_FR-FR123`.
    pub fn id(&self) -> String {
        let raw = self
            .url
            .split_once("id=")
            .map(|(_, rest)| rest.split('&').next().unwrap_or(rest))
            .map(|s| s.trim_end_matches(".jpg"))
            .map(|s| s.strip_suffix(RES_1080).or(s.strip_suffix(RES_UHD)).unwrap_or(s))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}_{}", self.startdate, self.hsh));
        sanitize_file_stem(&raw)
    }

    /// Absolute URL of the picture, 1080p or UHD.
    pub fn image_url(&self, uhd: bool) -> Result<String, BingError> {
        let path = if uhd { to_uhd(&self.url) } else { self.url.clone() };
        absolute_url(&path)
    }

    /// When (UTC) the picture becomes the picture of the day on its market.
    pub fn start_utc(&self) -> Option<DateTime<Utc>> {
        NaiveDateTime::parse_from_str(&self.fullstartdate, "%Y%m%d%H%M").ok().map(|d| d.and_utc())
    }

    /// The picture's day, in local time (from `fullstartdate`, else `startdate`).
    pub fn local_date(&self) -> Option<NaiveDate> {
        match self.start_utc() {
            Some(start) => Some(start.with_timezone(&Local).date_naive()),
            None => NaiveDate::parse_from_str(&self.startdate, "%Y%m%d").ok(),
        }
    }

    /// Readable date such as `Nov 12, 2026` (local time when `fullstartdate` is known).
    pub fn display_date(&self) -> String {
        match self.local_date() {
            Some(d) => d.format("%b %-d, %Y").to_string(),
            None => self.startdate.clone(),
        }
    }
}

/// Replaces the first `_1920x1080` with `_UHD` (4K version).
///
/// Only the first occurrence is touched: it is the `id` parameter; the following
/// parameters (`rf=...`) don't matter.
pub fn to_uhd(url: &str) -> String {
    url.replacen(RES_1080, RES_UHD, 1)
}

/// Prefixes a relative path with `https://www.bing.com`; refuses any other host.
pub fn absolute_url(path: &str) -> Result<String, BingError> {
    if path.starts_with('/') && !path.starts_with("//") {
        Ok(format!("{BING_ORIGIN}{path}"))
    } else if path.starts_with(BING_ORIGIN) && path[BING_ORIGIN.len()..].starts_with('/') {
        Ok(path.to_owned())
    } else {
        Err(BingError::ForbiddenUrl(path.to_owned()))
    }
}

/// Checks a market code of the form `ll-CC` (no injection into the URL).
pub fn validate_market(market: &str) -> Result<(), BingError> {
    let b = market.as_bytes();
    let ok = b.len() == 5
        && b[0].is_ascii_lowercase()
        && b[1].is_ascii_lowercase()
        && b[2] == b'-'
        && b[3].is_ascii_uppercase()
        && b[4].is_ascii_uppercase();
    if ok { Ok(()) } else { Err(BingError::InvalidMarket(market.to_owned())) }
}

/// Parses the JSON answer of `HPImageArchive`.
pub fn parse_archive(json: &str) -> Result<Vec<BingImage>, BingError> {
    let archive: Archive = serde_json::from_str(json)?;
    let images: Vec<BingImage> =
        archive.images.into_iter().filter(|img| !img.url.is_empty() && !img.startdate.is_empty()).collect();
    if images.is_empty() {
        return Err(BingError::Empty);
    }
    Ok(images)
}

fn sanitize_file_stem(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .take(120)
        .collect();
    let trimmed = cleaned.trim_start_matches('.');
    if trimmed.is_empty() { "image".to_owned() } else { trimmed.to_owned() }
}

pub struct BingClient {
    agent: ureq::Agent,
}

impl BingClient {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .https_only(true)
            // No redirects: we only ever talk to www.bing.com.
            .max_redirects(0)
            .user_agent(concat!("Unbing/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent: ureq::Agent::new_with_config(config) }
    }

    /// Metadata request (last 8 days) for a market.
    pub fn fetch_archive(&self, market: &str) -> Result<Vec<BingImage>, BingError> {
        validate_market(market)?;
        let url = format!("{BING_ORIGIN}/HPImageArchive.aspx?format=js&idx=0&n={HISTORY_LEN}&mkt={market}");
        let body = with_retry("metadata", || {
            debug!(%url, "GET");
            let mut resp = self.agent.get(&url).call()?;
            Ok(resp.body_mut().with_config().limit(MAX_JSON_BYTES).read_to_string()?)
        })?;
        parse_archive(&body)
    }

    /// Downloads a picture (absolute URL on www.bing.com only).
    pub fn download(&self, url: &str) -> Result<Vec<u8>, BingError> {
        let url = absolute_url(url)?;
        let bytes = with_retry("picture", || {
            debug!(%url, "GET");
            let mut resp = self.agent.get(&url).call()?;
            Ok(resp.body_mut().with_config().limit(MAX_IMAGE_BYTES).read_to_vec()?)
        })?;
        // If the API changes and returns an HTML page, don't write it to the cache.
        if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Err(BingError::NotJpeg);
        }
        Ok(bytes)
    }
}

/// Runs `f`, and a second time only if the error is transient.
fn with_retry<T>(what: &str, f: impl Fn() -> Result<T, BingError>) -> Result<T, BingError> {
    match f() {
        Err(e) if e.is_transient() => {
            warn!("{what} request failed ({e}), retrying in {}s", RETRY_DELAY.as_secs());
            std::thread::sleep(RETRY_DELAY);
            f()
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/hpimagearchive_fr-FR.json");
    const MINIMAL: &str = include_str!("../tests/fixtures/hpimagearchive_minimal.json");

    #[test]
    fn parses_full_fixture() {
        let images = parse_archive(FIXTURE).unwrap();
        assert_eq!(images.len(), 8);
        let first = &images[0];
        assert_eq!(first.startdate, "20261111");
        assert_eq!(first.enddate, "20261112");
        assert_eq!(first.title, "Le renard polaire");
        assert!(first.copyright.starts_with("Renard polaire, Islande"));
        assert!(first.url.starts_with("/th?id=OHR.ArcticFox_FR-FR"));
        assert_eq!(first.id(), "OHR.ArcticFox_FR-FR4415698203");
        // fr-FR: live from 23:00 UTC, i.e. midnight in Paris.
        assert_eq!(first.fullstartdate, "202611112300");
        assert_eq!(first.start_utc().unwrap().to_rfc3339(), "2026-11-11T23:00:00+00:00");
    }

    #[test]
    fn display_date_falls_back_to_startdate() {
        let mut img = parse_archive(MINIMAL).unwrap().remove(0);
        assert!(img.start_utc().is_none());
        assert_eq!(img.display_date(), "Nov 12, 2026");
        img.startdate = "soon".into();
        assert_eq!(img.display_date(), "soon");
    }

    #[test]
    fn tolerates_missing_and_unknown_fields() {
        let images = parse_archive(MINIMAL).unwrap();
        // The entry without `url` is dropped.
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].title, "");
        assert_eq!(images[0].copyright, "");
    }

    #[test]
    fn empty_or_garbage_is_an_error() {
        assert!(matches!(parse_archive(r#"{"images":[]}"#), Err(BingError::Empty)));
        assert!(matches!(parse_archive(r#"{"tooltips":{}}"#), Err(BingError::Empty)));
        assert!(matches!(parse_archive("<html>maintenance</html>"), Err(BingError::Json(_))));
    }

    #[test]
    fn uhd_replaces_only_the_id_resolution() {
        let url = "/th?id=OHR.Foo_FR-FR123_1920x1080.jpg&rf=LaDigue_1920x1080.jpg&pid=hp";
        assert_eq!(to_uhd(url), "/th?id=OHR.Foo_FR-FR123_UHD.jpg&rf=LaDigue_1920x1080.jpg&pid=hp");
    }

    #[test]
    fn uhd_leaves_unknown_formats_untouched() {
        let url = "/th?id=OHR.Foo_FR-FR123_1366x768.jpg";
        assert_eq!(to_uhd(url), url);
    }

    #[test]
    fn builds_absolute_urls() {
        let images = parse_archive(FIXTURE).unwrap();
        let hd = images[0].image_url(false).unwrap();
        let uhd = images[0].image_url(true).unwrap();
        assert_eq!(
            hd,
            "https://www.bing.com/th?id=OHR.ArcticFox_FR-FR4415698203_1920x1080.jpg&rf=LaDigue_1920x1080.jpg&pid=hp"
        );
        assert_eq!(
            uhd,
            "https://www.bing.com/th?id=OHR.ArcticFox_FR-FR4415698203_UHD.jpg&rf=LaDigue_1920x1080.jpg&pid=hp"
        );
    }

    #[test]
    fn rejects_foreign_hosts() {
        for bad in [
            "//evil.example/x.jpg",
            "https://evil.example/x.jpg",
            "https://www.bing.com.evil.example/x.jpg",
            "http://www.bing.com/x.jpg",
            "th?id=x",
        ] {
            assert!(absolute_url(bad).is_err(), "{bad} should have been refused");
        }
        assert!(absolute_url("https://www.bing.com/th?id=x").is_ok());
    }

    #[test]
    fn validates_markets() {
        for (code, _) in MARKETS {
            validate_market(code).unwrap();
        }
        for bad in ["fr", "FR-fr", "fr-FR&n=100", "fr_FR", ""] {
            assert!(validate_market(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn id_is_filename_safe() {
        let img = BingImage {
            url: "/th?id=../../etc/pass wd_1920x1080.jpg".into(),
            startdate: "20260101".into(),
            fullstartdate: String::new(),
            enddate: String::new(),
            title: String::new(),
            copyright: String::new(),
            hsh: String::new(),
        };
        let id = img.id();
        assert!(!id.contains('/') && !id.contains(' ') && !id.starts_with('.'), "{id}");
    }
}
