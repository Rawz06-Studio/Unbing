use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::UI::WindowsAndMessaging::{
    SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW,
};

use super::{WallpaperError, WallpaperSetter, absolute};

pub struct Windows;

impl WallpaperSetter for Windows {
    fn name(&self) -> &'static str {
        "Windows (SystemParametersInfoW)"
    }

    fn set_wallpaper(&self, path: &Path) -> Result<(), WallpaperError> {
        let path = absolute(path)?;
        // canonicalize() returns a `\\?\C:\...` path that Explorer handles badly.
        let raw = path.as_os_str().to_string_lossy();
        let cleaned = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
        let mut wide: Vec<u16> = std::ffi::OsStr::new(cleaned).encode_wide().chain(Some(0)).collect();

        // SAFETY: `wide` is a null-terminated UTF-16 string, alive during the call;
        // SPI_SETDESKWALLPAPER expects exactly this pointer.
        unsafe {
            SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(wide.as_mut_ptr().cast::<c_void>()),
                SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
            )
        }
        .map_err(|e| WallpaperError::Os(e.to_string()))
    }
}
