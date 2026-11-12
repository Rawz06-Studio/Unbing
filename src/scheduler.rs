//! Headless runs.
//!
//! - `--apply`: one pass, then exit (cron, scripts…). A pass makes **no** request
//!   until Bing's next picture is due, so running it often only costs a process start.
//! - `--daemon`: minimal background process (login item on Linux, where there is no
//!   tray icon). It sleeps until the market's next Bing publication (or 00:05 when
//!   unknown), 15 min at most at a time to stay on time after sleep, on a
//!   `recv_timeout` that allows a clean wake-up or stop.

use chrono::{DateTime, Local, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::sync::{Engine, Progress, SyncError};

/// Local fallback time when the publication time is unknown.
const FALLBACK_RUN_AT: (u32, u32) = (0, 5);
/// Margin after the announced publication time.
const PUBLICATION_MARGIN: TimeDelta = TimeDelta::minutes(5);
/// Longest nap: bounds the delay after the computer sleeps.
const MAX_NAP: Duration = Duration::from_secs(15 * 60);
/// Retry after an error (cache and wallpaper stay untouched).
const RETRY_AFTER_ERROR: TimeDelta = TimeDelta::hours(1);

#[allow(dead_code)] // `RunNow` / `Stop` are meant for external control.
pub enum Signal {
    RunNow,
    Stop,
}

/// One pass: reloads the settings, fetches and sets the picture of the day.
/// Returns when the next publication is expected, if known.
pub fn run_once() -> Result<Option<DateTime<Utc>>, SyncError> {
    let cfg = Config::load();
    if !cfg.auto_apply {
        info!("daily change disabled in the settings, nothing to do");
        return Ok(None);
    }
    let mut engine = Engine::new(cfg)?;
    let outcome = engine.daily(&mut |p| match p {
        Progress::Metadata => info!("fetching metadata"),
        Progress::Downloading(title) => info!("downloading: {title}"),
        Progress::Applying => info!("setting wallpaper"),
    })?;
    match outcome.applied {
        Some(backend) => info!("\"{}\" set with {backend} ({})", outcome.image.title, outcome.path.display()),
        None => info!("\"{}\" is already the wallpaper, nothing to do", outcome.image.title),
    }
    Ok(engine.next_due())
}

/// Next fallback time (00:05) strictly after `now`.
pub fn next_run_after(now: NaiveDateTime) -> NaiveDateTime {
    let at = NaiveTime::from_hms_opt(FALLBACK_RUN_AT.0, FALLBACK_RUN_AT.1, 0).unwrap_or(NaiveTime::MIN);
    let today = now.date().and_time(at);
    if now < today { today } else { now.date().succ_opt().map(|d| d.and_time(at)).unwrap_or(now + TimeDelta::days(1)) }
}

/// Next pass: announced Bing publication + margin, else 00:05 local time.
fn next_run(now: NaiveDateTime, due: Option<DateTime<Utc>>) -> NaiveDateTime {
    match due {
        Some(due) => {
            let at = (due + PUBLICATION_MARGIN).with_timezone(&Local).naive_local();
            if at > now { at } else { now + PUBLICATION_MARGIN }
        }
        None => next_run_after(now),
    }
}

pub fn run_daemon(signals: Receiver<Signal>) {
    // One background process at a time (enabling it from the app starts one).
    let lock_path = match crate::config::app_dir() {
        Ok(dir) => dir.join("daemon.lock"),
        Err(e) => {
            error!("{e}");
            return;
        }
    };
    let _ = std::fs::create_dir_all(lock_path.parent().unwrap_or(&lock_path));
    let _lock = match std::fs::File::create(&lock_path) {
        Ok(file) => match file.try_lock() {
            Ok(()) => file,
            Err(_) => {
                info!("a background process is already running, nothing to do");
                return;
            }
        },
        Err(e) => {
            error!("lock {}: {e}", lock_path.display());
            return;
        }
    };
    info!("background mode: one pass now, then at each new Bing picture");
    let mut next = Local::now().naive_local();
    loop {
        let now = Local::now().naive_local();
        if now >= next {
            next = match run_once() {
                Ok(due) => next_run(now, due),
                Err(SyncError::NoBackend) => {
                    warn!("{}", SyncError::NoBackend);
                    next_run_after(now)
                }
                Err(e) => {
                    error!("{e}; current wallpaper kept, retrying in 1 h");
                    now + RETRY_AFTER_ERROR
                }
            };
            info!("next pass: {next}");
            continue;
        }
        let nap = (next - now).to_std().unwrap_or(Duration::ZERO).min(MAX_NAP);
        match signals.recv_timeout(nap) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(Signal::RunNow) => next = now,
            Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => {
                info!("stopping background mode");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    #[test]
    fn fallback_before_run_time_is_same_day() {
        assert_eq!(next_run_after(at(2026, 11, 12, 0, 1)), at(2026, 11, 12, 0, 5));
    }

    #[test]
    fn fallback_after_run_time_is_next_day() {
        assert_eq!(next_run_after(at(2026, 11, 12, 0, 5)), at(2026, 11, 13, 0, 5));
        assert_eq!(next_run_after(at(2026, 11, 12, 23, 59)), at(2026, 11, 13, 0, 5));
    }

    #[test]
    fn fallback_crosses_month_and_year() {
        assert_eq!(next_run_after(at(2026, 12, 31, 12, 0)), at(2027, 1, 1, 0, 5));
        assert_eq!(next_run_after(at(2028, 2, 28, 12, 0)), at(2028, 2, 29, 0, 5));
    }

    #[test]
    fn fallback_on_daylight_saving_days() {
        // Daylight saving changes in France (2026-03-29 and 2026-10-25).
        assert_eq!(next_run_after(at(2026, 3, 28, 22, 0)), at(2026, 3, 29, 0, 5));
        assert_eq!(next_run_after(at(2026, 10, 24, 22, 0)), at(2026, 10, 25, 0, 5));
    }

    #[test]
    fn follows_publication_time_with_margin() {
        let due = DateTime::parse_from_rfc3339("2026-11-12T23:00:00Z").unwrap().with_timezone(&Utc);
        let expected = (due + PUBLICATION_MARGIN).with_timezone(&Local).naive_local();
        let now = (due - TimeDelta::hours(5)).with_timezone(&Local).naive_local();
        assert_eq!(next_run(now, Some(due)), expected);
        // Deadline already past: retry shortly after, not in a tight loop.
        let late = expected + TimeDelta::hours(1);
        assert_eq!(next_run(late, Some(due)), late + PUBLICATION_MARGIN);
        // Unknown: fall back to 00:05.
        assert_eq!(next_run(at(2026, 11, 12, 10, 0), None), at(2026, 11, 13, 0, 5));
    }
}
