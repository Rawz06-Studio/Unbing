//! Main window (sidebar with the last 8 days, large preview), the compact window
//! opened from the menu bar / tray icon, and the Settings and About windows.
//! Network and disk work runs on a worker thread: the UI never blocks.

pub mod icons;
mod native;
pub mod theme;
pub mod widgets;
mod worker;

use chrono::NaiveDate;
use eframe::egui::{
    self, Align, CornerRadius, Frame, Key, Layout, Margin, Rect, RichText, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2, vec2,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::autostart;
use crate::bing::{BingImage, MARKETS};
use crate::config::{Config, MAX_HISTORY, MIN_HISTORY, Theme};
use crate::menu::{self, Action, GITHUB_URL, SHOW_FOLDER};
use crate::sync::today;
use crate::updater;
use crate::wallpaper;
use icons::Icon;
use theme::pal;
use widgets as w;
use worker::{Job, Msg, Reporter};

/// On macOS the content extends under the title bar; leave room for the window buttons.
const TITLEBAR_INSET: f32 = if cfg!(target_os = "macos") { 38.0 } else { 10.0 };
const TOOLBAR_HEIGHT: f32 = 52.0;
const ICON_PNG: &[u8] = include_bytes!("../../assets/icon-256.png");
/// Margin after Bing's announced publication time before fetching it.
const PUBLICATION_MARGIN: i64 = 5 * 60;
/// Compact window opened from the tray icon.
const MINI_SIZE: [f32; 2] = [320.0, 464.0];

/// A wallpaper app needs no discrete GPU. By default wgpu asks for the high-performance one,
/// which on laptops wakes up the dedicated GPU: prefer the integrated one, and on Windows
/// use DirectX 12 only. `WGPU_BACKEND` / `WGPU_POWER_PREF` still override this.
fn gpu_options() -> eframe::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew, wgpu};
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.power_preference = wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower);
    if cfg!(windows) {
        setup.instance_descriptor.backends =
            wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12 | wgpu::Backends::GL);
    }
    eframe::WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(setup), ..Default::default() }
}

/// Runs the app. `background`: started at login, only the tray icon is shown.
pub fn run(background: bool) -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Unbing")
        .with_inner_size([1000.0, 640.0])
        .with_min_inner_size([760.0, 480.0])
        .with_icon(eframe::icon_data::from_png_bytes(ICON_PNG).unwrap_or_default());
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        viewport = viewport.with_transparent(true);
    }
    if cfg!(target_os = "macos") {
        viewport = viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false);
    }
    let options = eframe::NativeOptions { viewport, wgpu_options: gpu_options(), ..Default::default() };
    eframe::run_native("Unbing", options, Box::new(move |cc| Ok(Box::new(App::new(cc, background)))))
}

enum Status {
    Idle,
    Working(String),
    Done(String, Instant),
    Error(String),
}

enum Update {
    Idle,
    Checking { manual: bool },
    UpToDate,
    Available(updater::Release),
    Installing,
    Installed,
    Failed(String),
}

struct App {
    cfg: Config,
    images: Vec<BingImage>,
    offline: bool,
    selected: usize,
    preview: Option<(String, TextureHandle, PathBuf)>,
    thumbs: HashMap<String, TextureHandle>,
    applied_id: Option<String>,
    status: Status,
    busy: bool,
    /// Last action asked for while the worker was busy.
    queued: Option<Job>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    backend: Option<&'static str>,
    autostart: bool,
    requests_today: u32,
    day: String,
    /// Unix time after which Bing is expected to publish a new picture.
    due: Option<i64>,
    show_settings: bool,
    show_about: bool,
    window_pos: HashMap<&'static str, egui::Pos2>,
    window_opened: HashMap<&'static str, Instant>,
    icon: Option<TextureHandle>,
    update: Update,
    update_rx: Option<Receiver<Update>>,
    exe: Option<PathBuf>,
    /// Main window shown (it hides instead of closing while the tray icon exists).
    window_visible: bool,
    /// Started at login: hide the main window on the first frame.
    start_hidden: bool,
    background: bool,
    quitting: bool,
    /// Compact window: open, where (logical points), and whether it got focus yet.
    mini: Option<(egui::Pos2, bool, Instant)>,
    /// The app was active at the last check (macOS: detects a relaunch from the Finder).
    was_active: bool,
    menu_rx: Option<Receiver<Action>>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    tray: Option<crate::tray::Tray>,
    #[cfg(target_os = "macos")]
    native_menu: Option<menu::NativeMenu>,
    material_dark: Option<bool>,
    /// Debug builds: open the compact window a moment after launch (UI screenshots).
    #[cfg(debug_assertions)]
    debug_mini_at: Option<Instant>,
}

impl App {
    fn new(cc: &eframe::CreationContext, background: bool) -> Self {
        let ctx = &cc.egui_ctx;
        theme::install_fonts(ctx);
        let cfg = Config::load();
        let (tx, rx) = mpsc::channel();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let menu_rx = Some(menu::install_event_handler(ctx));
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let menu_rx = None;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let tray = crate::tray::Tray::install(ctx);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let has_tray = tray.is_some();
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let has_tray = false;

        let mut app = Self {
            backend: wallpaper::detect(&cfg.linux_backend).map(|s| s.name()),
            autostart: autostart::is_enabled(),
            cfg,
            images: Vec::new(),
            offline: false,
            selected: 0,
            preview: None,
            thumbs: HashMap::new(),
            applied_id: None,
            status: Status::Idle,
            busy: false,
            queued: None,
            tx,
            rx,
            requests_today: 0,
            day: today(),
            due: None,
            show_settings: false,
            show_about: false,
            window_pos: HashMap::new(),
            window_opened: HashMap::new(),
            icon: None,
            update: Update::Idle,
            update_rx: None,
            exe: std::env::current_exe().ok(),
            window_visible: true,
            start_hidden: background && has_tray,
            background,
            quitting: false,
            mini: None,
            was_active: false,
            menu_rx,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            tray,
            #[cfg(target_os = "macos")]
            native_menu: menu::NativeMenu::install(),
            material_dark: None,
            #[cfg(debug_assertions)]
            debug_mini_at: None,
        };
        if app.start_hidden {
            native::set_dock_icon(false);
        }
        app.apply_theme(ctx);
        let dark = ctx.system_theme().unwrap_or(egui::Theme::Dark) == egui::Theme::Dark;
        theme::apply_material(cc, app.effective_dark(dark));
        theme::install_style(ctx);
        app.run(Job::Startup, ctx);
        if app.cfg.check_updates && !cfg!(debug_assertions) {
            app.check_updates(ctx, false);
        }
        // Debug builds only: open a secondary window at launch, for UI screenshots.
        #[cfg(debug_assertions)]
        {
            app.show_settings = std::env::var("UNBING_DEBUG_SETTINGS").is_ok();
            app.show_about = std::env::var("UNBING_DEBUG_ABOUT").is_ok();
            if std::env::var("UNBING_DEBUG_MINI").is_ok() {
                app.debug_mini_at = Some(Instant::now() + Duration::from_secs(3));
            }
        }
        app
    }

    fn has_tray(&self) -> bool {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        return self.tray.is_some();
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        return false;
    }

    fn effective_dark(&self, system_dark: bool) -> bool {
        match self.cfg.theme {
            Theme::Auto => system_dark,
            Theme::Light => false,
            Theme::Dark => true,
        }
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.cfg.theme {
            Theme::Auto => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
    }

    fn reporter(&self, ctx: &egui::Context) -> Reporter {
        Reporter { tx: self.tx.clone(), ctx: ctx.clone() }
    }

    fn run(&mut self, job: Job, ctx: &egui::Context) {
        if self.busy {
            self.queued = Some(job);
            return;
        }
        self.busy = true;
        if let Err(e) = worker::spawn(job, self.cfg.clone(), self.images.clone(), self.reporter(ctx)) {
            self.busy = false;
            self.status = Status::Error(format!("could not start background work: {e}"));
        }
    }

    // ---------- Updates ----------

    fn check_updates(&mut self, ctx: &egui::Context, manual: bool) {
        if matches!(self.update, Update::Checking { .. } | Update::Installing | Update::Installed) {
            return;
        }
        self.update = Update::Checking { manual };
        self.spawn_update(ctx, move || match updater::check() {
            Ok(Some(release)) => Update::Available(release),
            Ok(None) => Update::UpToDate,
            Err(e) if manual => Update::Failed(e),
            Err(_) => Update::Idle, // stay quiet when the automatic check fails (offline…)
        });
    }

    fn install_update(&mut self, ctx: &egui::Context, url: String) {
        self.update = Update::Installing;
        self.spawn_update(ctx, move || match updater::install(&url) {
            Ok(()) => Update::Installed,
            Err(e) => Update::Failed(e),
        });
    }

    fn spawn_update(&mut self, ctx: &egui::Context, job: impl FnOnce() -> Update + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.update_rx = Some(rx);
    }

    fn restart(&mut self, ctx: &egui::Context) {
        let Some(exe) = &self.exe else { return };
        let mut cmd = std::process::Command::new(exe);
        if self.background && !self.window_visible {
            cmd.arg("--background");
        }
        // Release the single-instance lock first: the new process must be able to take it.
        crate::instance::release();
        if cmd.spawn().is_ok() {
            self.quit(ctx);
        }
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
    }

    // ---------- Messages ----------

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.update_rx
            && let Ok(state) = rx.try_recv()
        {
            self.update = state;
            self.update_rx = None;
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Status(s) => self.status = Status::Working(s),
                Msg::Metadata { images, offline } => {
                    self.images = images;
                    self.offline = offline;
                    self.selected = 0;
                    let known = self.thumbs.keys().cloned().collect();
                    worker::spawn_thumbs(self.cfg.clone(), self.images.clone(), known, self.reporter(ctx));
                }
                Msg::Preview { index, id, image, path } => {
                    if self.images.get(index).map(BingImage::id).as_deref() == Some(id.as_str()) {
                        let tex = ctx.load_texture("preview", image, TextureOptions::LINEAR);
                        self.preview = Some((id, tex, path));
                    }
                }
                Msg::Thumb { id, image } => {
                    let tex = ctx.load_texture(format!("thumb-{id}"), image, TextureOptions::LINEAR);
                    self.thumbs.insert(id, tex);
                }
                Msg::Applied { id, backend } => {
                    self.applied_id = id;
                    if backend.is_some() {
                        self.status = Status::Done("Wallpaper set".into(), Instant::now());
                    }
                }
                Msg::NextDue(t) => self.due = Some(t),
                Msg::Requests(n) => self.requests_today = n,
                Msg::Failed(e) => self.status = Status::Error(e),
                Msg::Done => {
                    self.busy = false;
                    if let Status::Working(_) = self.status {
                        self.status = Status::Idle;
                    }
                    if let Some(job) = self.queued.take() {
                        self.run(job, ctx);
                    }
                }
            }
        }
    }

    /// Daily change while the app runs: a new day, or Bing's next publication time.
    fn schedule(&mut self, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        let now = today();
        let new_day = now != self.day;
        let published = self.due.is_some_and(|due| chrono::Utc::now().timestamp() >= due + PUBLICATION_MARGIN);
        if new_day || published {
            self.day = now;
            self.due = None;
            self.run(Job::Startup, ctx);
            if new_day && self.cfg.check_updates && !cfg!(debug_assertions) {
                self.check_updates(ctx, false);
            }
        }
    }

    // ---------- Selection and actions ----------

    fn current(&self) -> Option<&BingImage> {
        self.images.get(self.selected)
    }

    fn select(&mut self, index: usize, ctx: &egui::Context) {
        if index >= self.images.len() || index == self.selected && self.preview_matches() {
            return;
        }
        self.selected = index;
        if !self.preview_matches() {
            self.run(Job::Show(index), ctx);
        }
    }

    fn preview_matches(&self) -> bool {
        let id = self.current().map(BingImage::id);
        self.preview.as_ref().map(|(p, _, _)| p) == id.as_ref()
    }

    fn is_applied(&self) -> bool {
        self.current().is_some_and(|img| self.applied_id.as_deref() == Some(img.id().as_str()))
    }

    fn menu_state(&self) -> menu::Enabled {
        menu::Enabled {
            newer: self.selected > 0,
            older: self.selected + 1 < self.images.len(),
            apply: self.backend.is_some() && self.current().is_some() && !self.is_applied(),
        }
    }

    fn act(&mut self, action: Action, ctx: &egui::Context) {
        let state = self.menu_state();
        match action {
            Action::Newer if state.newer => self.select(self.selected - 1, ctx),
            Action::Older if state.older => self.select(self.selected + 1, ctx),
            Action::Apply if state.apply => self.run(Job::Apply(self.selected), ctx),
            Action::Today => {
                self.selected = 0;
                if self.backend.is_some() && !self.images.is_empty() {
                    self.run(Job::Apply(0), ctx);
                }
            }
            Action::ShowFolder => match self.cfg.images_dir() {
                Ok(dir) => worker::open_folder(&dir),
                Err(e) => self.status = Status::Error(e.to_string()),
            },
            Action::ShowWindow => self.show_window(ctx),
            Action::Settings => self.open_window(ctx, Action::Settings),
            Action::About => self.open_window(ctx, Action::About),
            Action::CheckUpdates => {
                if matches!(self.update, Update::Failed(_) | Update::UpToDate) {
                    self.update = Update::Idle;
                }
                self.open_window(ctx, Action::About);
                self.check_updates(ctx, true);
            }
            Action::GitHub => ctx.open_url(egui::OpenUrl::new_tab(GITHUB_URL)),
            Action::Quit => self.quit(ctx),
            _ => {}
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.mini = None;
        self.window_visible = true;
        native::set_dock_icon(true);
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        native::activate();
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.window_visible = false;
        self.show_settings = false;
        self.show_about = false;
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(false));
        native::set_dock_icon(false);
    }

    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    fn toggle_mini(&mut self, ctx: &egui::Context, anchor: Option<crate::tray::Anchor>) {
        if self.mini.take().is_some() {
            return;
        }
        let ppp = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(ctx.pixels_per_point());
        let pos = match anchor {
            Some(a) => {
                let center_x = (a.x + a.width / 2.0) as f32 / ppp;
                let x = (center_x - MINI_SIZE[0] / 2.0).max(8.0);
                // Menu bar at the top on macOS; taskbar usually at the bottom on Windows.
                let below = (a.y as f32 / ppp) < 200.0;
                let y =
                    if below { (a.y + a.height) as f32 / ppp + 6.0 } else { a.y as f32 / ppp - MINI_SIZE[1] - 10.0 };
                let x = match ctx.input(|i| i.viewport().monitor_size) {
                    Some(size) => x.min(size.x - MINI_SIZE[0] - 8.0),
                    None => x,
                };
                egui::pos2(x, y.max(0.0))
            }
            None => egui::pos2(80.0, 80.0),
        };
        self.mini = Some((pos, false, Instant::now()));
        native::activate();
        // The main window may be hidden: ask for a full pass so the compact window opens.
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::RequestPaintWhileHidden);
        ctx.request_repaint();
    }

    fn save_config(&mut self) {
        self.cfg = self.cfg.clone().sanitized();
        if let Err(e) = self.cfg.save() {
            self.status = Status::Error(format!("Could not save settings: {e}"));
        }
    }

    fn set_daily(&mut self, on: bool) {
        match autostart::set_enabled(on) {
            Ok(()) => {
                self.autostart = on;
                self.cfg.auto_apply = on;
                self.save_config();
            }
            Err(e) => self.status = Status::Error(e.to_string()),
        }
    }

    // ---------- Sidebar ----------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let top = Rect::from_min_size(ui.max_rect().min, vec2(ui.available_width(), TITLEBAR_INSET));
        if cfg!(target_os = "macos") {
            w::window_drag_area(ui, top, "sidebar_drag");
        }
        ui.add_space(TITLEBAR_INSET);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            w::caption(ui, "Recent Pictures");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(&self.cfg.market).size(11.0).color(p.weak));
            });
        });
        ui.add_space(4.0);

        let footer = 34.0;
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink(false).max_height(ui.available_height() - footer).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, img) in self.images.iter().enumerate() {
                let id = img.id();
                let date = img.local_date().map(short_date).unwrap_or_else(|| img.display_date());
                let title = if img.title.is_empty() { "Untitled" } else { &img.title };
                let applied = self.applied_id.as_deref() == Some(id.as_str());
                let resp = w::image_row(ui, self.thumbs.get(&id), &date, title, i == self.selected, applied);
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
            if self.images.is_empty() && !self.busy {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("No pictures yet.").color(p.weak));
                });
            }
        });
        if let Some(i) = clicked {
            let ctx = ui.ctx().clone();
            self.select(i, &ctx);
        }

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                if w::icon_button(ui, Icon::Gear, "Settings", true).clicked() {
                    let ctx = ui.ctx().clone();
                    self.open_window(&ctx, Action::Settings);
                }
                ui.label(RichText::new(requests_label(self.requests_today)).size(11.0).color(p.weak))
                    .on_hover_text("HTTPS requests to www.bing.com, the only server contacted for pictures.");
            });
        });
    }

    // ---------- Toolbar ----------

    fn toolbar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = pal(ui);
        if cfg!(target_os = "macos") {
            w::window_drag_area(ui, rect, "toolbar_drag");
        }
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.separator));
        let inner = rect.shrink2(vec2(18.0, 0.0));
        let ctx = ui.ctx().clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let state = self.menu_state();
            if self.is_applied() {
                w::button(ui, "Current Wallpaper", Some(Icon::Check), false);
            } else {
                let hint = if self.backend.is_none() {
                    "No way to change the wallpaper was found on this system"
                } else if cfg!(target_os = "macos") {
                    "⌘↵"
                } else {
                    "Ctrl+Enter"
                };
                let resp = w::accent_button(ui, "Set as Wallpaper", Some(Icon::Display), state.apply);
                if resp.on_hover_text(hint).clicked() {
                    self.act(Action::Apply, &ctx);
                }
            }
            if w::icon_button(ui, Icon::Folder, SHOW_FOLDER, true).clicked() {
                self.act(Action::ShowFolder, &ctx);
            }
            ui.add_space(4.0);
            self.update_notice(ui);
            self.status_label(ui);

            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if w::icon_button(ui, Icon::ChevronLeft, "Older picture", state.older).clicked() {
                    self.act(Action::Older, &ctx);
                }
                if w::icon_button(ui, Icon::ChevronRight, "Newer picture", state.newer).clicked() {
                    self.act(Action::Newer, &ctx);
                }
                ui.add_space(8.0);
                let title = self.current().and_then(BingImage::local_date).map(long_date).unwrap_or_default();
                ui.label(RichText::new(title).size(15.0).strong().color(p.text));
                if self.offline {
                    ui.add_space(8.0);
                    w::pill(ui, "Offline", p.orange);
                }
            });
        });
    }

    fn status_label(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        match &self.status {
            Status::Idle => {}
            Status::Working(s) => {
                ui.label(RichText::new(s).size(12.0).color(p.weak));
                ui.spinner();
            }
            Status::Done(s, at) => {
                let age = at.elapsed();
                if age < Duration::from_millis(2500) {
                    ui.label(RichText::new(s).size(12.0).color(p.weak));
                    ui.ctx().request_repaint_after(Duration::from_millis(2600) - age);
                } else {
                    self.status = Status::Idle;
                }
            }
            Status::Error(e) => {
                let short: String = e.chars().take(60).collect();
                let short = if short.len() < e.len() { format!("{short}…") } else { short };
                ui.label(RichText::new(short).size(12.0).color(p.red)).on_hover_text(e.as_str());
            }
        }
    }

    fn update_notice(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let p = pal(ui);
        match &self.update {
            Update::Available(rel) => {
                let rel = rel.clone();
                let label = format!("Update to {}", rel.version);
                let resp = w::button(ui, &label, Some(Icon::Download), true).on_hover_text("See what's new on GitHub");
                if resp.clicked() {
                    match rel.asset_url {
                        Some(url) => self.install_update(&ctx, url),
                        None => ctx.open_url(egui::OpenUrl::new_tab(&rel.page_url)),
                    }
                }
            }
            Update::Installing => {
                ui.label(RichText::new("Updating…").size(12.0).color(p.weak));
                ui.spinner();
            }
            Update::Installed => {
                if w::accent_button(ui, "Restart to Update", None, true).clicked() {
                    self.restart(&ctx);
                }
            }
            Update::Failed(e) if !self.show_settings && !self.show_about => {
                ui.label(RichText::new("Update failed").size(12.0).color(p.red)).on_hover_text(e);
            }
            _ => {}
        }
    }

    // ---------- Preview ----------

    fn content(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let area = ui.available_rect_before_wrap();
        let info_height = 70.0;
        let img_area = Rect::from_min_max(area.min, egui::pos2(area.right(), area.bottom() - info_height));

        let aspect = self.preview.as_ref().map(|(_, t, _)| t.aspect_ratio()).unwrap_or(16.0 / 9.0);
        let mut size = vec2(img_area.width(), img_area.width() / aspect);
        if size.y > img_area.height() {
            size = vec2(img_area.height() * aspect, img_area.height());
        }
        // Picture and caption centered vertically in the available space.
        let block = size.y + info_height - 14.0;
        let top = img_area.top() + ((area.height() - block) / 2.0).max(0.0);
        let rect = Rect::from_min_size(egui::pos2(img_area.center().x - size.x / 2.0, top), size);
        self.picture(ui, rect, 10);

        if let Some(img) = self.current() {
            let info = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() + 14.0), area.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(info), |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                let title = if img.title.is_empty() { "Untitled" } else { &img.title };
                ui.add(egui::Label::new(RichText::new(title).size(17.0).strong().color(p.text)).truncate());
                ui.add(egui::Label::new(RichText::new(&img.copyright).size(12.5).color(p.weak)).wrap());
            });
        }
    }

    /// The selected picture in `rect`, or a placeholder while it loads.
    fn picture(&mut self, ui: &mut egui::Ui, rect: Rect, radius: u8) {
        let p = pal(ui);
        let radius = CornerRadius::same(radius);
        let thumb = self.current().and_then(|img| self.thumbs.get(&img.id()));
        let texture = match &self.preview {
            Some((_, tex, _)) if self.preview_matches() => Some(tex),
            _ => thumb,
        };
        if let Some(tex) = texture {
            egui::Image::from_texture(tex).corner_radius(radius).paint_at(ui, rect);
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, p.border), StrokeKind::Inside);
            return;
        }
        ui.painter().rect_filled(rect, radius, p.code);
        let center = rect.center();
        if self.busy {
            ui.put(Rect::from_center_size(center, Vec2::splat(20.0)), egui::Spinner::new().size(20.0));
        } else if self.images.is_empty() {
            let r = Rect::from_center_size(center, vec2(rect.width(), 80.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(r).layout(Layout::top_down(Align::Center)), |ui| {
                ui.label(RichText::new("Bing can't be reached right now.").color(p.weak));
                ui.add_space(6.0);
                if w::button(ui, "Try Again", None, true).clicked() {
                    let ctx = ui.ctx().clone();
                    self.run(Job::Startup, &ctx);
                }
            });
        } else {
            icons::paint(ui.painter(), Rect::from_center_size(center, Vec2::splat(28.0)), Icon::Photo, p.weak);
        }
    }

    // ---------- Compact window (tray) ----------

    fn mini_window(&mut self, ctx: &egui::Context) {
        let Some((pos, had_focus, opened)) = self.mini else { return };
        let builder = egui::ViewportBuilder::default()
            .with_title("Unbing")
            .with_inner_size(MINI_SIZE)
            .with_position(pos)
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        let close = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("mini"), builder, |ui, _| {
            let p = pal(ui);
            let frame = Frame::new()
                .fill(ui.visuals().window_fill)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(14));
            egui::CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
                frame.show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    self.mini_content(ui);
                });
            });
            let focused = ui.input(|i| i.viewport().focused);
            // Ignore focus moves while it opens (the main window may take focus once).
            let settled = opened.elapsed() > Duration::from_millis(500);
            if !settled {
                ui.ctx().request_repaint_after(Duration::from_millis(550));
            }
            let lost_focus = had_focus && settled && focused == Some(false);
            let gained = focused == Some(true);
            let close = ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape)) || lost_focus;
            (close, gained)
        });
        let (close, gained) = close;
        if close {
            self.mini = None;
            if !self.window_visible {
                native::deactivate();
            }
        } else if gained && let Some(m) = &mut self.mini {
            m.1 = true;
        }
    }

    fn mini_content(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let ctx = ui.ctx().clone();
        let state = self.menu_state();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Unbing").size(13.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if w::icon_button(ui, Icon::ChevronRight, "Newer picture", state.newer).clicked() {
                    self.act(Action::Newer, &ctx);
                }
                if w::icon_button(ui, Icon::ChevronLeft, "Older picture", state.older).clicked() {
                    self.act(Action::Older, &ctx);
                }
                let date = self.current().and_then(BingImage::local_date).map(short_date).unwrap_or_default();
                ui.add_space(6.0);
                ui.label(RichText::new(date).size(12.0).color(p.weak));
            });
        });
        ui.add_space(6.0);

        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), egui::Sense::hover());
        self.picture(ui, rect, 8);
        ui.add_space(8.0);

        if let Some(img) = self.current().cloned() {
            let title = if img.title.is_empty() { "Untitled".to_owned() } else { img.title.clone() };
            ui.add(egui::Label::new(RichText::new(title).size(13.5).strong().color(p.text)).truncate());
            ui.add(egui::Label::new(RichText::new(&img.copyright).size(11.0).color(p.weak)).truncate())
                .on_hover_text(&img.copyright);
        }
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            if self.is_applied() {
                w::button(ui, "Current Wallpaper", Some(Icon::Check), false);
            } else if w::accent_button(ui, "Set as Wallpaper", Some(Icon::Display), state.apply).clicked() {
                self.act(Action::Apply, &ctx);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.busy {
                    ui.spinner();
                }
            });
        });
        ui.add_space(8.0);
        ui.separator();

        ui.horizontal(|ui| {
            let mut daily = self.autostart;
            w::checkbox(ui, &mut daily, self.backend.is_some());
            ui.label("Change wallpaper every day");
            if daily != self.autostart {
                self.set_daily(daily);
            }
        });
        match &self.update {
            Update::Available(rel) => {
                let label = format!("Update to {}", rel.version);
                if w::button(ui, &label, Some(Icon::Download), true).clicked() {
                    self.show_window(&ctx);
                }
            }
            Update::Installed => {
                if w::accent_button(ui, "Restart to Update", None, true).clicked() {
                    self.restart(&ctx);
                }
            }
            _ => {
                if let Status::Error(e) = &self.status {
                    let short: String = e.chars().take(48).collect();
                    ui.label(RichText::new(short).size(11.0).color(p.red)).on_hover_text(e.as_str());
                } else {
                    ui.label(RichText::new(requests_label(self.requests_today)).size(11.0).color(p.weak));
                }
            }
        }

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(2.0);
        {
            ui.horizontal(|ui| {
                if w::button(ui, "Open Unbing", None, true).clicked() {
                    self.show_window(&ctx);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::button(ui, "Quit", None, true).clicked() {
                        self.quit(&ctx);
                    }
                    if w::icon_button(ui, Icon::Gear, "Settings", true).clicked() {
                        self.show_window(&ctx);
                        self.open_window(&ctx, Action::Settings);
                    }
                });
            });
        }
    }

    // ---------- Secondary windows ----------

    fn open_window(&mut self, ctx: &egui::Context, which: Action) {
        let (key, size, _) = window_spec(which);
        // Centered over the main window, computed once so it doesn't follow it around.
        if let Some(main) = ctx.input(|i| i.viewport().outer_rect) {
            let pos = main.center() - Vec2::from(size) / 2.0 - vec2(0.0, 40.0);
            self.window_pos.insert(key, pos.max(main.min));
        }
        match which {
            Action::About => self.show_about = true,
            _ => {
                // The real state may have changed (file removed by hand…).
                self.autostart = autostart::is_enabled();
                self.show_settings = true;
            }
        }
        ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of(key), egui::ViewportCommand::Focus);
    }

    fn windows(&mut self, ctx: &egui::Context) {
        for which in [Action::Settings, Action::About] {
            let open = if matches!(which, Action::About) { self.show_about } else { self.show_settings };
            let (key, size, resizable) = window_spec(which);
            if !open {
                self.window_opened.remove(key);
                continue;
            }
            // macOS may show a new window after its first frame was drawn (leaving it blank).
            if self.window_opened.entry(key).or_insert_with(Instant::now).elapsed() < Duration::from_secs(1) {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            let mut builder = egui::ViewportBuilder::default()
                .with_title(key)
                .with_inner_size(size)
                .with_resizable(resizable)
                .with_minimize_button(false)
                .with_maximize_button(false);
            if let Some(pos) = self.window_pos.get(key) {
                builder = builder.with_position(*pos);
            }
            let close = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(key), builder, |ui, _| {
                let fill = ui.visuals().window_fill;
                egui::CentralPanel::default().frame(Frame::new().fill(fill).inner_margin(Margin::same(20))).show(
                    ui,
                    |ui| match which {
                        Action::About => self.about_content(ui),
                        _ => self.settings_content(ui),
                    },
                );
                ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape))
            });
            if close {
                match which {
                    Action::About => self.show_about = false,
                    _ => self.show_settings = false,
                }
            }
        }
    }

    fn settings_content(&mut self, ui: &mut egui::Ui) {
        let before = self.cfg.clone();
        let mut daily = self.autostart;
        let mut check_now = false;
        let p = pal(ui);
        let ctx = ui.ctx().clone();
        egui::Grid::new("settings").num_columns(2).spacing([20.0, 14.0]).show(ui, |ui| {
            ui.label(RichText::new("Appearance").color(p.weak));
            w::segmented(
                ui,
                &mut self.cfg.theme,
                &[(Theme::Auto, "Auto"), (Theme::Light, "Light"), (Theme::Dark, "Dark")],
            );
            ui.end_row();

            ui.label(RichText::new("Market").color(p.weak));
            let selected = MARKETS
                .iter()
                .find(|(code, _)| *code == self.cfg.market)
                .map(|(code, name)| format!("{name} ({code})"))
                .unwrap_or_else(|| self.cfg.market.clone());
            egui::ComboBox::from_id_salt("market").selected_text(selected).width(220.0).icon(w::chevron).show_ui(
                ui,
                |ui| {
                    for (code, name) in MARKETS {
                        ui.selectable_value(&mut self.cfg.market, (*code).to_owned(), format!("{name} ({code})"));
                    }
                },
            );
            ui.end_row();

            ui.label(RichText::new("Resolution").color(p.weak));
            w::segmented(ui, &mut self.cfg.uhd, &[(false, "1080p"), (true, "4K")]);
            ui.end_row();

            ui.label(RichText::new("Wallpaper").color(p.weak));
            ui.horizontal(|ui| {
                w::checkbox(ui, &mut daily, self.backend.is_some());
                ui.label("Change every day");
                if self.backend.is_none() {
                    ui.label(RichText::new("(not supported here)").color(p.weak));
                }
            });
            ui.end_row();

            ui.label(RichText::new("History").color(p.weak));
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.cfg.history_limit)
                        .range(MIN_HISTORY..=MAX_HISTORY)
                        .suffix(" pictures"),
                );
                ui.label(RichText::new("kept on disk").color(p.weak));
            });
            ui.end_row();

            #[cfg(target_os = "linux")]
            {
                ui.label(RichText::new("Backend").color(p.weak));
                egui::ComboBox::from_id_salt("linux_backend")
                    .selected_text(self.cfg.linux_backend.clone())
                    .icon(w::chevron)
                    .show_ui(ui, |ui| {
                        for b in ["auto", "gnome", "kde", "hyprpaper", "sway", "swaybg", "feh", "none"] {
                            ui.selectable_value(&mut self.cfg.linux_backend, b.to_owned(), b);
                        }
                    });
                ui.end_row();
            }

            ui.label(RichText::new("Updates").color(p.weak));
            ui.horizontal(|ui| {
                w::checkbox(ui, &mut self.cfg.check_updates, true);
                ui.label("Check on startup");
            });
            ui.end_row();

            ui.label(RichText::new("Version").color(p.weak));
            ui.horizontal(|ui| {
                ui.label(updater::VERSION);
                match &self.update {
                    Update::Checking { manual: true } => {
                        ui.spinner();
                    }
                    Update::UpToDate => {
                        ui.label(RichText::new("Up to date").color(p.green));
                    }
                    Update::Available(rel) => {
                        ui.label(RichText::new(format!("{} available", rel.version)).color(p.accent));
                    }
                    Update::Failed(e) => {
                        ui.label(RichText::new("Check failed").color(p.red)).on_hover_text(e);
                    }
                    _ => {}
                }
                if w::button(ui, "Check now", None, true).clicked() {
                    check_now = true;
                }
            });
            ui.end_row();

            ui.label(RichText::new("Pictures").color(p.weak));
            let path = self.cfg.images_dir().map(|d| d.display().to_string()).unwrap_or_default();
            ui.add(egui::Label::new(RichText::new(&path).monospace().size(11.5)).truncate()).on_hover_text(path);
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.hyperlink_to("Unbing on GitHub", GITHUB_URL);

        if check_now {
            if matches!(self.update, Update::Failed(_) | Update::UpToDate) {
                self.update = Update::Idle;
            }
            self.check_updates(&ctx, true);
        }
        if daily != self.autostart {
            self.set_daily(daily);
        }
        if self.cfg != before {
            if self.cfg.theme != before.theme {
                self.apply_theme(&ctx);
            }
            if self.cfg.linux_backend != before.linux_backend {
                self.backend = wallpaper::detect(&self.cfg.linux_backend).map(|s| s.name());
            }
            self.save_config();
            if self.cfg.market != before.market {
                self.preview = None;
                self.run(Job::Refresh, &ctx);
            }
        }
    }

    fn about_content(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let icon = self.icon.get_or_insert_with(|| {
            let data = eframe::icon_data::from_png_bytes(ICON_PNG).unwrap_or_default();
            let image =
                egui::ColorImage::from_rgba_unmultiplied([data.width as usize, data.height as usize], &data.rgba);
            ui.ctx().load_texture("unbing-icon", image, TextureOptions::LINEAR)
        });
        let icon = (icon.id(), vec2(96.0, 96.0));
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            ui.add(egui::Image::new(icon));
            ui.add_space(6.0);
            ui.label(RichText::new("Unbing").size(22.0).strong());
            ui.label(RichText::new(format!("Version {}", updater::VERSION)).color(p.weak));
            ui.add_space(10.0);
            ui.label(
                RichText::new("Bing's picture of the day as your wallpaper.\nNo telemetry, no account, no tracking.")
                    .color(p.text),
            );
            ui.add_space(14.0);
            match &self.update {
                Update::Checking { .. } => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Checking for updates…").color(p.weak));
                    });
                }
                Update::UpToDate => {
                    ui.label(RichText::new("Unbing is up to date.").color(p.green));
                }
                Update::Available(rel) => {
                    let rel = rel.clone();
                    if w::accent_button(ui, &format!("Update to {}", rel.version), Some(Icon::Download), true).clicked()
                    {
                        match rel.asset_url {
                            Some(url) => self.install_update(&ctx, url),
                            None => ctx.open_url(egui::OpenUrl::new_tab(&rel.page_url)),
                        }
                    }
                }
                Update::Installing => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Updating…").color(p.weak));
                    });
                }
                Update::Installed => {
                    if w::accent_button(ui, "Restart to update", None, true).clicked() {
                        self.restart(&ctx);
                    }
                }
                Update::Failed(e) => {
                    ui.label(RichText::new("Could not check for updates.").color(p.red)).on_hover_text(e);
                }
                Update::Idle => {
                    if w::button(ui, "Check for Updates", None, true).clicked() {
                        self.check_updates(&ctx, true);
                    }
                }
            }
            ui.add_space(14.0);
            // Center the two links: measure them first.
            let font = egui::TextStyle::Body.resolve(ui.style());
            let width: f32 = ["GitHub", "·", "MIT License"]
                .iter()
                .map(|t| ui.painter().layout_no_wrap(t.to_string(), font.clone(), p.text).size().x)
                .sum::<f32>()
                + 2.0 * ui.spacing().item_spacing.x;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                ui.hyperlink_to("GitHub", GITHUB_URL);
                ui.label(RichText::new("·").color(p.weak));
                ui.hyperlink_to("MIT License", format!("{GITHUB_URL}/blob/main/LICENSE"));
            });
            ui.add_space(4.0);
            ui.label(RichText::new("© 2026 Rawz06 Studio").size(11.0).color(p.weak));
            ui.add_space(8.0);
            ui.label(
                RichText::new("Not affiliated with Microsoft. Pictures © Bing\nand their authors, for personal use.")
                    .size(11.0)
                    .color(p.weak),
            );
        });
    }
}

impl eframe::App for App {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if theme::glass() { [0.0; 4] } else { visuals.panel_fill.to_normalized_gamma_f32() }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.schedule(ctx);

        let mut actions: Vec<Action> = Vec::new();
        if let Some(rx) = &self.menu_rx {
            actions.extend(rx.try_iter());
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let mut anchors = Vec::new();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(tray) = &self.tray {
            while let Some(crate::tray::TrayMsg::Toggle(anchor)) = tray.poll() {
                anchors.push(anchor);
            }
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        for anchor in anchors {
            self.toggle_mini(ctx, Some(anchor));
        }
        for action in actions {
            self.act(action, ctx);
        }

        #[cfg(debug_assertions)]
        if let Some(at) = self.debug_mini_at {
            if Instant::now() >= at {
                self.debug_mini_at = None;
                self.toggle_mini(ctx, None);
            } else {
                ctx.request_repaint_after(at - Instant::now());
            }
        }

        // Another launch (Finder, Start menu…) while we run in the background: show the window.
        let active = native::is_active();
        let reopened = active && !self.was_active && !self.window_visible && self.mini.is_none();
        self.was_active = active;
        if crate::instance::take_show_request() || reopened {
            self.show_window(ctx);
        }

        // In the background, poll a little more often for the show request.
        let tick = if self.window_visible { 60 } else { 1 };
        ctx.request_repaint_after(Duration::from_secs(tick));
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if std::mem::take(&mut self.start_hidden) {
            self.hide_window(&ctx);
        }
        // With a tray icon, closing the window keeps Unbing running in the menu bar.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.has_tray() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(&ctx);
        }

        // Mica takes the light/dark variant explicitly: re-apply when it changes.
        let dark = ui.visuals().dark_mode;
        if cfg!(target_os = "windows") && self.material_dark != Some(dark) {
            theme::apply_material(frame, dark);
            self.material_dark = Some(dark);
        }
        let _ = &frame;

        let mut actions: Vec<Action> = ctx.input_mut(|i| std::iter::from_fn(|| Action::from_keys(i)).collect());
        #[cfg(target_os = "macos")]
        {
            let state = self.menu_state();
            if let Some(m) = &mut self.native_menu {
                m.set_enabled(state);
            }
        }

        let p = pal(ui);
        if !cfg!(target_os = "macos") {
            let state = self.menu_state();
            egui::Panel::top("menubar")
                .frame(Frame::new().fill(p.sidebar).inner_margin(Margin::symmetric(8, 3)))
                .show(ui, |ui| actions.extend(menu::in_window_bar(ui, state)));
        }
        for action in actions {
            self.act(action, &ctx);
        }

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(270.0)
            .size_range(230.0..=380.0)
            .frame(Frame::new().fill(p.sidebar).inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 8 }))
            .show(ui, |ui| self.sidebar(ui));

        egui::CentralPanel::default().frame(Frame::new().fill(p.content)).show(ui, |ui| {
            let full = ui.max_rect();
            let bar = Rect::from_min_size(full.min, vec2(full.width(), TOOLBAR_HEIGHT));
            self.toolbar(ui, bar);
            let body = Rect::from_min_max(egui::pos2(full.left(), bar.bottom()), full.max).shrink2(vec2(24.0, 0.0));
            let body = Rect::from_min_max(body.min + vec2(0.0, 20.0), body.max - vec2(0.0, 12.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.content(ui));
        });

        if self.window_visible {
            self.windows(&ctx);
        }
        self.mini_window(&ctx);
    }
}

// ---------- Helpers ----------

/// Title (also the window id), inner size and resizability of each secondary window.
fn window_spec(which: Action) -> (&'static str, [f32; 2], bool) {
    match which {
        Action::About => ("About Unbing", [340.0, 440.0], false),
        _ => ("Settings", [500.0, if cfg!(target_os = "linux") { 440.0 } else { 396.0 }], false),
    }
}

fn requests_label(n: u32) -> String {
    match n {
        0 => "No request today".to_owned(),
        1 => "1 request today".to_owned(),
        n => format!("{n} requests today"),
    }
}

/// "Thursday, November 12, 2026"
fn long_date(d: NaiveDate) -> String {
    d.format("%A, %B %-d, %Y").to_string()
}

/// "Today", "Yesterday", otherwise "Tuesday, November 10".
fn short_date(d: NaiveDate) -> String {
    let today = chrono::Local::now().date_naive();
    if d == today {
        "Today".into()
    } else if Some(d) == today.pred_opt() {
        "Yesterday".into()
    } else {
        d.format("%A, %B %-d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        assert_eq!(long_date(d), "Thursday, November 12, 2026");
        let old = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        assert_eq!(short_date(old), "Monday, August 3");
    }

    #[test]
    fn window_icon_decodes() {
        let icon = eframe::icon_data::from_png_bytes(ICON_PNG).unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
    }
}
