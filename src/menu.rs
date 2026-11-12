//! App actions and menus: the native menu bar on macOS, an in-window menu bar elsewhere.

use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    About,
    Settings,
    CheckUpdates,
    Newer,
    Older,
    Apply,
    /// Select today's picture and set it as wallpaper (tray menu).
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Today,
    ShowFolder,
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    ShowWindow,
    GitHub,
    Quit,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
const ALL: [Action; 11] = [
    Action::About,
    Action::Settings,
    Action::CheckUpdates,
    Action::Newer,
    Action::Older,
    Action::Apply,
    Action::Today,
    Action::ShowFolder,
    Action::ShowWindow,
    Action::GitHub,
    Action::Quit,
];

pub const GITHUB_URL: &str = "https://github.com/Rawz06-Studio/Unbing";

pub const SHOW_FOLDER: &str = if cfg!(target_os = "macos") {
    "Show in Finder"
} else if cfg!(windows) {
    "Show in Explorer"
} else {
    "Open Pictures Folder"
};

impl Action {
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    pub fn id(self) -> &'static str {
        match self {
            Action::About => "about",
            Action::Settings => "settings",
            Action::CheckUpdates => "check-updates",
            Action::Newer => "newer",
            Action::Older => "older",
            Action::Apply => "apply",
            Action::Today => "today",
            Action::ShowFolder => "show-folder",
            Action::ShowWindow => "show-window",
            Action::GitHub => "github",
            Action::Quit => "quit",
        }
    }

    /// Keyboard shortcuts handled by egui (on macOS the menu bar catches them first).
    pub fn from_keys(i: &mut egui::InputState) -> Option<Self> {
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let none = Modifiers::NONE;
        [
            (cmd, Key::Enter, Action::Apply),
            (cmd, Key::Comma, Action::Settings),
            (none, Key::ArrowUp, Action::Newer),
            (none, Key::ArrowLeft, Action::Older),
            (none, Key::ArrowDown, Action::Older),
            (none, Key::ArrowRight, Action::Newer),
        ]
        .into_iter()
        .find(|(m, k, _)| i.consume_key(*m, *k))
        .map(|(_, _, a)| a)
    }
}

/// Which actions currently make sense (used to grey out menu items).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Enabled {
    pub newer: bool,
    pub older: bool,
    pub apply: bool,
}

/// Routes clicks on menu items (menu bar and tray menu) to a channel.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn install_event_handler(ctx: &egui::Context) -> std::sync::mpsc::Receiver<Action> {
    use tray_icon::menu::MenuEvent;
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(a) = ALL.into_iter().find(|a| a.id() == event.id().as_ref()) {
            let _ = tx.send(a);
            ctx.request_repaint();
        }
    }));
    rx
}

// ---------- macOS: native menu bar ----------

#[cfg(target_os = "macos")]
pub use macos::NativeMenu;

#[cfg(target_os = "macos")]
mod macos {
    use super::{Action, Enabled, SHOW_FOLDER};
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

    pub struct NativeMenu {
        _menu: Menu,
        newer: MenuItem,
        older: MenuItem,
        apply: MenuItem,
        enabled: Enabled,
    }

    fn item(a: Action, text: &str, key: Option<(Modifiers, Code)>) -> MenuItem {
        MenuItem::with_id(a.id(), text, true, key.map(|(m, c)| Accelerator::new(m, c)))
    }

    impl NativeMenu {
        pub fn install() -> Option<Self> {
            let cmd = Modifiers::META;
            let sep = PredefinedMenuItem::separator;

            let app = Submenu::with_items(
                "Unbing",
                true,
                &[
                    &item(Action::About, "About Unbing", None),
                    &item(Action::CheckUpdates, "Check for Updates…", None),
                    &sep(),
                    &item(Action::Settings, "Settings…", Some((cmd, Code::Comma))),
                    &sep(),
                    &PredefinedMenuItem::services(None),
                    &sep(),
                    &PredefinedMenuItem::hide(Some("Hide Unbing")),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &sep(),
                    &item(Action::Quit, "Quit Unbing", Some((cmd, Code::KeyQ))),
                ],
            )
            .ok()?;

            let newer = item(Action::Newer, "Newer Picture", Some((cmd, Code::BracketRight)));
            let older = item(Action::Older, "Older Picture", Some((cmd, Code::BracketLeft)));
            let apply = item(Action::Apply, "Set as Wallpaper", Some((cmd, Code::Enter)));
            let image = Submenu::with_items(
                "Picture",
                true,
                &[
                    &newer,
                    &older,
                    &sep(),
                    &apply,
                    &item(Action::ShowFolder, SHOW_FOLDER, None),
                    &sep(),
                    &PredefinedMenuItem::close_window(None),
                ],
            )
            .ok()?;

            let window = Submenu::with_items(
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::maximize(Some("Zoom")),
                    &sep(),
                    &PredefinedMenuItem::bring_all_to_front(None),
                ],
            )
            .ok()?;

            let help = Submenu::with_items("Help", true, &[&item(Action::GitHub, "Unbing on GitHub", None)]).ok()?;

            let menu = Menu::with_items(&[&app, &image, &window, &help]).ok()?;
            menu.init_for_nsapp();
            window.set_as_windows_menu_for_nsapp();
            help.set_as_help_menu_for_nsapp();
            Some(Self { _menu: menu, newer, older, apply, enabled: Enabled::default() })
        }

        pub fn set_enabled(&mut self, e: Enabled) {
            if e == self.enabled {
                return;
            }
            self.newer.set_enabled(e.newer);
            self.older.set_enabled(e.older);
            self.apply.set_enabled(e.apply);
            self.enabled = e;
        }
    }
}

// ---------- Windows / Linux: menu bar inside the window ----------

/// Draws "File · Picture · Help" at the top of the window, Windows 11 style.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn in_window_bar(ui: &mut egui::Ui, e: Enabled) -> Option<Action> {
    let mut action = None;
    let mut entry = |ui: &mut egui::Ui, a: Action, text: &str, shortcut: &str, enabled: bool| {
        let button = egui::Button::new(text).shortcut_text(shortcut);
        if ui.add_enabled(enabled, button).clicked() {
            action = Some(a);
        }
    };
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            entry(ui, Action::Settings, "Settings", "Ctrl+,", true);
            ui.separator();
            entry(ui, Action::Quit, "Exit", "", true);
        });
        ui.menu_button("Picture", |ui| {
            // egui's default fonts (Linux) have no arrow glyphs.
            let (newer, older) = if cfg!(windows) { ("→", "←") } else { ("", "") };
            entry(ui, Action::Newer, "Newer picture", newer, e.newer);
            entry(ui, Action::Older, "Older picture", older, e.older);
            ui.separator();
            entry(ui, Action::Apply, "Set as wallpaper", "Ctrl+Enter", e.apply);
            entry(ui, Action::ShowFolder, SHOW_FOLDER, "", true);
        });
        ui.menu_button("Help", |ui| {
            entry(ui, Action::CheckUpdates, "Check for updates", "", true);
            entry(ui, Action::GitHub, "Unbing on GitHub", "", true);
            ui.separator();
            entry(ui, Action::About, "About Unbing", "", true);
        });
    });
    action
}
