//! One app instance at a time (the tray app may already be running in the background).
//!
//! The first instance holds a lock file. A second one, started by the user, leaves a
//! "show" request next to it and exits; the running instance picks it up and shows its window.

use std::fs::File;
use std::path::PathBuf;

pub struct Instance {
    _lock: Option<File>,
}

static HELD: std::sync::Mutex<Option<Instance>> = std::sync::Mutex::new(None);

/// Keeps the lock until `release` (or the end of the process).
pub fn hold(instance: Instance) {
    if let Ok(mut held) = HELD.lock() {
        *held = Some(instance);
    }
}

/// Lets a new instance start (used when restarting after an update).
pub fn release() {
    if let Ok(mut held) = HELD.lock() {
        *held = None;
    }
}

fn path(name: &str) -> Option<PathBuf> {
    crate::config::app_dir().ok().map(|d| d.join(name))
}

/// Takes the lock, or returns `None` if another instance holds it.
pub fn acquire() -> Option<Instance> {
    // No config directory: run anyway rather than refusing to start.
    let Some(path) = path("app.lock") else { return Some(Instance { _lock: None }) };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = File::create(&path).ok()?;
    file.try_lock().ok()?;
    Some(Instance { _lock: Some(file) })
}

/// Asks the running instance to show its main window.
pub fn request_show() {
    if let Some(path) = path("show.request") {
        let _ = std::fs::write(path, b"");
    }
}

/// True (once) if another launch asked us to show the window.
pub fn take_show_request() -> bool {
    path("show.request").is_some_and(|p| std::fs::remove_file(p).is_ok())
}
