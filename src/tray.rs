//! Menu bar icon (macOS) / notification area icon (Windows).
//!
//! Left click toggles the compact window; right click shows a small menu.
//! Not available on Linux, where tray support depends on the desktop.

/// Where the tray icon is, in physical pixels (to place the compact window next to it).
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub enum TrayMsg {
    Toggle(Anchor),
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use imp::Tray;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod imp {
    use super::{Anchor, TrayMsg};
    use eframe::egui;
    use std::sync::mpsc::Receiver;
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use crate::menu::Action;

    pub struct Tray {
        _icon: TrayIcon,
        rx: Receiver<TrayMsg>,
    }

    #[cfg(target_os = "macos")]
    const PNG: &[u8] = include_bytes!("../assets/tray-macos.png");
    #[cfg(target_os = "windows")]
    const PNG: &[u8] = include_bytes!("../assets/tray-windows.png");

    impl Tray {
        /// Creates the icon. Must run on the main thread, once the event loop is running.
        /// Its menu items carry `Action` ids: clicks arrive through `menu::install_event_handler`.
        pub fn install(ctx: &egui::Context) -> Option<Self> {
            let image = image::load_from_memory(PNG).ok()?.to_rgba8();
            let (w, h) = image.dimensions();
            let icon = tray_icon::Icon::from_rgba(image.into_raw(), w, h).ok()?;

            let menu = Menu::new();
            let open = MenuItem::with_id(Action::ShowWindow.id(), "Open Unbing", true, None);
            let apply = MenuItem::with_id(Action::Today.id(), "Use Today's Picture", true, None);
            let quit = MenuItem::with_id(Action::Quit.id(), "Quit Unbing", true, None);
            menu.append_items(&[&open, &apply, &PredefinedMenuItem::separator(), &quit]).ok()?;

            let builder = TrayIconBuilder::new();
            // macOS tints a template icon to match the menu bar (light, dark, highlighted).
            #[cfg(target_os = "macos")]
            let builder = builder.with_icon_templated(icon);
            #[cfg(not(target_os = "macos"))]
            let builder = builder.with_icon(icon);
            let built = builder.with_tooltip("Unbing").with_menu(Box::new(menu)).with_menu_on_left_click(false).build();
            let tray = match built {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("tray icon unavailable: {e}");
                    return None;
                }
            };

            let (tx, rx) = std::sync::mpsc::channel();
            let repaint = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    rect,
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let anchor = Anchor {
                        x: rect.position.x,
                        y: rect.position.y,
                        width: rect.size.width as f64,
                        height: rect.size.height as f64,
                    };
                    let _ = tx.send(TrayMsg::Toggle(anchor));
                    repaint.request_repaint();
                }
            }));
            Some(Self { _icon: tray, rx })
        }

        pub fn poll(&self) -> Option<TrayMsg> {
            self.rx.try_recv().ok()
        }
    }
}
