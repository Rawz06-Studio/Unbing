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
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn set_dock_icon(_visible: bool) {}
    pub fn activate() {}
    pub fn deactivate() {}
    pub fn is_active() -> bool {
        false
    }
}

pub use imp::*;
