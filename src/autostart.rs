//! "Change wallpaper every day": registers Unbing to start with the session.
//!
//! - macOS: LaunchAgent `studio.rawz06.unbing` running `unbing --background` at login
//!   (menu bar icon only, no window).
//! - Windows: `Unbing` value under `HKCU\…\Run` running `Unbing.exe --background`
//!   (notification area icon only).
//! - Linux: XDG entry `~/.config/autostart/unbing.desktop` running `unbing --daemon`
//!   (no window, no icon).
//!
//! The running app then changes the wallpaper as soon as Bing publishes a new picture;
//! between two publications it makes no request.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub enum AutostartError {
    #[error("cannot find the executable: {0}")]
    Exe(std::io::Error),
    #[error("cannot find the home directory")]
    NoHome,
    #[error("Unbing runs from a temporary location: move it to Applications, then try again")]
    Translocated,
    #[error("I/O on {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[allow(dead_code)]
    #[error("{0}")]
    System(String),
}

fn exe() -> Result<PathBuf, AutostartError> {
    let exe = std::env::current_exe().map_err(AutostartError::Exe)?;
    // macOS runs apps that were not moved out of Downloads from a random copy (App Translocation).
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return Err(AutostartError::Translocated);
    }
    Ok(exe)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn write(path: &Path, content: &str) -> Result<(), AutostartError> {
    let io = |source| AutostartError::Io { path: path.to_owned(), source };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    std::fs::write(path, content).map_err(io)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn remove(path: &Path) -> Result<(), AutostartError> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(AutostartError::Io { path: path.to_owned(), source: e })
        }
        _ => Ok(()),
    }
}

/// Escapes text for an XML document.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub use imp::{is_enabled, set_enabled};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    use std::process::{Command, Stdio};

    const LABEL: &str = "studio.rawz06.unbing";

    fn plist_path() -> Result<PathBuf, AutostartError> {
        dirs::home_dir()
            .map(|h| h.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
            .ok_or(AutostartError::NoHome)
    }

    pub(crate) fn plist(exe: &Path) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
		<string>--background</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<key>ProcessType</key>
	<string>Interactive</string>
</dict>
</plist>
"#,
            xml_escape(&exe.to_string_lossy())
        )
    }

    pub fn is_enabled() -> bool {
        plist_path().is_ok_and(|p| p.is_file())
    }

    pub fn set_enabled(on: bool) -> Result<(), AutostartError> {
        let path = plist_path()?;
        if on {
            // Loaded by launchd at the next login; the app is already running now.
            return write(&path, &plist(&exe()?));
        }
        // Unload a previously loaded agent, if any.
        if let Some(home) = dirs::home_dir()
            && let Ok(meta) = std::fs::metadata(home)
        {
            let _ = Command::new("/bin/launchctl")
                .args(["bootout", &format!("gui/{}/{LABEL}", meta.uid())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        remove(&path)
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };
    use windows::core::{PCWSTR, w};

    const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const NAME: PCWSTR = w!("Unbing");

    pub fn is_enabled() -> bool {
        // SAFETY: only the return code is asked for (no buffer).
        unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
    }

    pub fn set_enabled(on: bool) -> Result<(), AutostartError> {
        if !on {
            // SAFETY: constant, null-terminated strings.
            let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) };
            return if err == ERROR_SUCCESS || err == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(AutostartError::System(format!("registry: {err:?}")))
            };
        }
        let exe = exe()?;
        let raw = exe.to_string_lossy();
        let exe_str = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
        let value: Vec<u16> = format!("\"{exe_str}\" --background").encode_utf16().chain(Some(0)).collect();
        // SAFETY: `value` is a null-terminated UTF-16 string, alive during the call.
        let err = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                NAME,
                REG_SZ.0,
                Some(value.as_ptr().cast::<c_void>()),
                (value.len() * 2) as u32,
            )
        };
        if err == ERROR_SUCCESS { Ok(()) } else { Err(AutostartError::System(format!("registry: {err:?}"))) }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use super::*;
    use std::process::{Command, Stdio};

    fn desktop_path() -> Result<PathBuf, AutostartError> {
        dirs::config_dir().map(|c| c.join("autostart/unbing.desktop")).ok_or(AutostartError::NoHome)
    }

    pub fn is_enabled() -> bool {
        desktop_path().is_ok_and(|p| p.is_file())
    }

    pub fn set_enabled(on: bool) -> Result<(), AutostartError> {
        let path = desktop_path()?;
        if !on {
            return remove(&path);
        }
        let exe = exe()?;
        // Desktop Entry spec: double quotes, with `"` `\` `$` and `` ` `` escaped.
        let quoted =
            exe.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`");
        write(
            &path,
            &format!(
                "[Desktop Entry]\nType=Application\nName=Unbing\nComment=Bing's picture of the day as your wallpaper\n\
                 Exec=\"{quoted}\" --daemon\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"
            ),
        )?;
        // No tray icon on Linux: start the background process now (one at a time: lock).
        let _ =
            Command::new(&exe).arg("--daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn xml_is_escaped() {
        assert_eq!(super::xml_escape("/A & B/<x>\""), "/A &amp; B/&lt;x&gt;&quot;");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn plist_is_valid_xml_with_escaped_path() {
        let p = super::imp::plist(std::path::Path::new("/Applications/R&D <x>.app/Contents/MacOS/unbing"));
        assert!(p.contains("<string>/Applications/R&amp;D &lt;x&gt;.app/Contents/MacOS/unbing</string>"));
        assert!(p.contains("<string>--background</string>"));
    }
}
