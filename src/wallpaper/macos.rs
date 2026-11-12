//! macOS: the official `NSWorkspace.setDesktopImageURL` API (through JXA: no Objective-C
//! crate, no Automation permission), falling back to System Events.
//!
//! Either way the path is passed as an argument (`argv`) to the script: it is never
//! interpolated into the source, so there is nothing to escape and no injection
//! possible, whatever its characters.

use std::path::Path;
use std::process::Command;
use tracing::warn;

use super::{WallpaperError, WallpaperSetter, absolute, run};

const OSASCRIPT: &str = "/usr/bin/osascript";

/// Sets the picture on every connected screen (the current Space of each).
const JXA_NSWORKSPACE: &str = r#"
function run(argv) {
  ObjC.import('AppKit');
  const url = $.NSURL.fileURLWithPath(argv[0]);
  const ws = $.NSWorkspace.sharedWorkspace;
  const screens = $.NSScreen.screens;
  for (let i = 0; i < screens.count; i++) {
    const err = Ref();
    if (!ws.setDesktopImageURLForScreenOptionsError(url, screens.objectAtIndex(i), $({}), err)) {
      throw new Error('screen ' + i + ': ' + ObjC.unwrap(err[0].localizedDescription));
    }
  }
}
"#;

const APPLESCRIPT_SYSTEM_EVENTS: [&str; 3] = [
    "on run argv",
    "tell application \"System Events\" to tell every desktop to set picture to POSIX file (item 1 of argv)",
    "end run",
];

pub struct MacOs;

impl WallpaperSetter for MacOs {
    fn name(&self) -> &'static str {
        "macOS (NSWorkspace)"
    }

    fn set_wallpaper(&self, path: &Path) -> Result<(), WallpaperError> {
        // Absolute path (starts with "/"): it can't be mistaken for an option.
        let path = absolute(path)?;
        let primary = run(Command::new(OSASCRIPT).args(["-l", "JavaScript", "-e", JXA_NSWORKSPACE]).arg(&path));
        match primary {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!("NSWorkspace failed ({e}), falling back to System Events");
                let mut cmd = Command::new(OSASCRIPT);
                for line in APPLESCRIPT_SYSTEM_EVENTS {
                    cmd.arg("-e").arg(line);
                }
                run(cmd.arg(&path))
            }
        }
    }
}
