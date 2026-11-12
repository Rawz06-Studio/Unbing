//! Linux : GNOME (gsettings), KDE Plasma, Hyprland (hyprpaper), Sway, swaybg, feh.

use std::env;
use std::path::Path;
use std::process::{Command, Stdio};

use tracing::{info, warn};

use super::{WallpaperError, WallpaperSetter, absolute, run};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Gnome,
    Kde,
    Hyprpaper,
    Sway,
    Swaybg,
    Feh,
}

struct Linux(Backend);

pub fn detect(forced: &str) -> Option<Box<dyn WallpaperSetter>> {
    let backend = match forced {
        "auto" | "" => auto_detect(),
        "none" => None,
        "gnome" => Some(Backend::Gnome),
        "kde" => Some(Backend::Kde),
        "hyprpaper" => Some(Backend::Hyprpaper),
        "sway" => Some(Backend::Sway),
        "swaybg" => Some(Backend::Swaybg),
        "feh" => Some(Backend::Feh),
        other => {
            warn!("unknown Linux backend {other:?}, detecting");
            auto_detect()
        }
    };
    match backend {
        Some(b) => {
            info!("wallpaper backend: {b:?}");
            Some(Box::new(Linux(b)))
        }
        None => {
            warn!("no wallpaper backend found: daily change disabled");
            None
        }
    }
}

fn auto_detect() -> Option<Backend> {
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let has = |var: &str| env::var_os(var).is_some_and(|v| !v.is_empty());

    if has("HYPRLAND_INSTANCE_SIGNATURE") && in_path("hyprctl") {
        return Some(Backend::Hyprpaper);
    }
    if has("SWAYSOCK") && in_path("swaymsg") {
        return Some(Backend::Sway);
    }
    if desktop.contains("kde") && in_path("plasma-apply-wallpaperimage") {
        return Some(Backend::Kde);
    }
    // GNOME and derivatives (Ubuntu's "ubuntu:GNOME", Budgie, Unity): gsettings by default.
    if ["gnome", "unity", "budgie"].iter().any(|d| desktop.contains(d)) && in_path("gsettings") {
        return Some(Backend::Gnome);
    }
    if has("WAYLAND_DISPLAY") && in_path("swaybg") {
        return Some(Backend::Swaybg);
    }
    if has("DISPLAY") && in_path("feh") {
        return Some(Backend::Feh);
    }
    None
}

fn in_path(bin: &str) -> bool {
    env::var_os("PATH").map(|paths| env::split_paths(&paths).any(|dir| is_executable(&dir.join(bin)))).unwrap_or(false)
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// `file://` + percent-encoded path (gsettings expects a URI).
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

impl WallpaperSetter for Linux {
    fn name(&self) -> &'static str {
        match self.0 {
            Backend::Gnome => "GNOME (gsettings)",
            Backend::Kde => "KDE Plasma",
            Backend::Hyprpaper => "Hyprland (hyprpaper)",
            Backend::Sway => "Sway (swaymsg)",
            Backend::Swaybg => "swaybg",
            Backend::Feh => "feh",
        }
    }

    fn needs_reapply(&self) -> bool {
        // swaybg doesn't outlive the session: set it again if it isn't running.
        self.0 == Backend::Swaybg && !Command::new("pgrep").args(["-x", "swaybg"]).status().is_ok_and(|s| s.success())
    }

    fn set_wallpaper(&self, path: &Path) -> Result<(), WallpaperError> {
        let path = absolute(path)?;
        match self.0 {
            Backend::Gnome => {
                let uri = file_uri(&path);
                let schema = "org.gnome.desktop.background";
                run(Command::new("gsettings").args(["set", schema, "picture-uri", &uri]))?;
                // Key missing before GNOME 42: a failure is not fatal.
                if let Err(e) = run(Command::new("gsettings").args(["set", schema, "picture-uri-dark", &uri])) {
                    warn!("picture-uri-dark not set: {e}");
                }
                let _ = run(Command::new("gsettings").args(["set", schema, "picture-options", "zoom"]));
                Ok(())
            }
            Backend::Kde => run(Command::new("plasma-apply-wallpaperimage").arg(&path)),
            Backend::Hyprpaper => {
                let p = path.to_string_lossy();
                run(Command::new("hyprctl").args(["hyprpaper", "preload", &p]))?;
                run(Command::new("hyprctl").args(["hyprpaper", "wallpaper", &format!(",{p}")]))?;
                let _ = run(Command::new("hyprctl").args(["hyprpaper", "unload", "unused"]));
                Ok(())
            }
            Backend::Sway => {
                // swaymsg takes a sway command: the path must be quoted.
                let quoted = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
                run(Command::new("swaymsg").arg(format!("output * bg \"{quoted}\" fill")))
            }
            Backend::Swaybg => {
                // swaybg must stay alive: replace the previous instance.
                let _ = Command::new("pkill").args(["-x", "swaybg"]).status();
                Command::new("swaybg")
                    .args(["-m", "fill", "-i"])
                    .arg(&path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map(drop)
                    .map_err(|source| WallpaperError::Spawn { cmd: "swaybg".into(), source })
            }
            Backend::Feh => run(Command::new("feh").arg("--bg-fill").arg(&path)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_is_percent_encoded() {
        assert_eq!(
            file_uri(Path::new("/home/a b/Images/l'été.jpg")),
            "file:///home/a%20b/Images/l%27%C3%A9t%C3%A9.jpg"
        );
    }
}
