//! Small platform hooks for a menu bar / tray app.
//! On macOS the Dock icon is only shown while the main window is open.

#[cfg(target_os = "macos")]
mod imp {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    fn app() -> Option<objc2::rc::Retained<NSApplication>> {
        MainThreadMarker::new().map(NSApplication::sharedApplication)
    }

    /// Regular app (Dock icon, app menu) or accessory (menu bar icon only).
    pub fn set_dock_icon(visible: bool) {
        if let Some(app) = app() {
            let policy =
                if visible { NSApplicationActivationPolicy::Regular } else { NSApplicationActivationPolicy::Accessory };
            app.setActivationPolicy(policy);
        }
    }

    /// Brings the app to the front, so its windows get keyboard focus.
    #[allow(deprecated)] // `activate()` needs macOS 14; this works on every supported version.
    pub fn activate() {
        if let Some(app) = app() {
            // The app may have been hidden when the compact window closed.
            app.unhide(None);
            app.activateIgnoringOtherApps(true);
        }
    }

    /// Gives focus back to the previous app (after the compact window closes).
    pub fn deactivate() {
        if let Some(app) = app() {
            app.hide(None);
        }
    }

    pub fn is_active() -> bool {
        app().is_some_and(|a| a.isActive())
    }

    /// Compact window look: no traffic lights, shown on the current Space (or back to normal).
    pub fn set_window_buttons_hidden(window: &impl raw_window_handle::HasWindowHandle, hidden: bool) {
        use objc2_app_kit::{NSView, NSWindowButton};
        use raw_window_handle::RawWindowHandle;
        let Ok(handle) = window.window_handle() else { return };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return };
        if MainThreadMarker::new().is_none() {
            return;
        }
        // SAFETY: the handle comes from a live winit window, used on the main thread.
        let view: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
        let Some(window) = view.window() else { return };
        // Like a menu bar app: the window shows up on the Space the user is on, instead of
        // the Space it was last seen on; the compact one even over a full-screen app.
        use objc2_app_kit::NSWindowCollectionBehavior as B;
        let behavior = window.collectionBehavior() | B::MoveToActiveSpace;
        window.setCollectionBehavior(if hidden {
            behavior | B::FullScreenAuxiliary
        } else {
            behavior & !B::FullScreenAuxiliary
        });
        for kind in [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton] {
            if let Some(button) = window.standardWindowButton(kind) {
                button.setHidden(hidden);
            }
        }
    }

    /// Picture the system shows on the main screen, in the current Space.
    pub fn current_wallpaper() -> Option<std::path::PathBuf> {
        use objc2_app_kit::{NSScreen, NSWorkspace};
        let mtm = MainThreadMarker::new()?;
        let screen = NSScreen::mainScreen(mtm)?;
        let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
        Some(std::path::PathBuf::from(url.path()?.to_string()))
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn set_dock_icon(_visible: bool) {}
    pub fn activate() {}
    pub fn deactivate() {}
    pub fn is_active() -> bool {
        false
    }
    pub fn current_wallpaper() -> Option<std::path::PathBuf> {
        None
    }
    pub fn set_window_buttons_hidden(_window: &impl raw_window_handle::HasWindowHandle, _hidden: bool) {}
}

pub use imp::*;
