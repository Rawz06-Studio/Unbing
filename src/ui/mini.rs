//! Compact window shown from the menu bar / tray icon.
//!
//! It is the main window itself, shrunk and placed under the icon: creating another
//! native window while the main one is hidden is not possible with eframe, while
//! showing an existing hidden window is. The app passes a snapshot of what to show
//! (`MiniView`); the content returns what the user asked for (`MiniIntent`).

use eframe::egui::{self, Align, CornerRadius, Layout, Rect, RichText, Stroke, StrokeKind, TextureHandle, Vec2, vec2};

use super::icons::{self, Icon};
use super::theme::pal;
use super::widgets as w;
use crate::menu::Action;

pub const SIZE: [f32; 2] = [320.0, 452.0];

/// What the compact window shows (a copy of the app state, made on each pass).
#[derive(Default, Clone)]
pub struct MiniView {
    pub texture: Option<TextureHandle>,
    pub loading: bool,
    pub title: String,
    pub copyright: String,
    pub date: String,
    pub newer: bool,
    pub older: bool,
    pub can_apply: bool,
    pub applied: bool,
    pub daily: bool,
    pub daily_supported: bool,
    /// "Update to x.y.z" button, and whether the update is already installed.
    pub update: Option<(String, bool)>,
    /// Status line, and whether it is an error.
    pub status: (String, bool),
}

pub enum MiniIntent {
    Act(Action),
    SetDaily(bool),
    Update,
    Restart,
}

pub fn content(ui: &mut egui::Ui, v: &MiniView, out: &mut Vec<MiniIntent>) {
    let p = pal(ui);

    ui.horizontal(|ui| {
        ui.label(RichText::new("Unbing").size(13.0).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if w::icon_button(ui, Icon::ChevronRight, "Newer picture", v.newer).clicked() {
                out.push(MiniIntent::Act(Action::Newer));
            }
            if w::icon_button(ui, Icon::ChevronLeft, "Older picture", v.older).clicked() {
                out.push(MiniIntent::Act(Action::Older));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(&v.date).size(12.0).color(p.weak));
        });
    });
    ui.add_space(6.0);

    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), egui::Sense::hover());
    let radius = CornerRadius::same(8);
    match &v.texture {
        Some(tex) => {
            egui::Image::from_texture(tex).corner_radius(radius).paint_at(ui, rect);
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, p.border), StrokeKind::Inside);
        }
        None => {
            ui.painter().rect_filled(rect, radius, p.code);
            let r = Rect::from_center_size(rect.center(), Vec2::splat(20.0));
            if v.loading {
                ui.put(r, egui::Spinner::new().size(20.0));
            } else {
                icons::paint(ui.painter(), r, Icon::Photo, p.weak);
            }
        }
    }
    ui.add_space(8.0);

    ui.add(egui::Label::new(RichText::new(&v.title).size(13.5).strong().color(p.text)).truncate());
    ui.add(egui::Label::new(RichText::new(&v.copyright).size(11.0).color(p.weak)).truncate())
        .on_hover_text(&v.copyright);
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        // Always clickable: the system may show another picture (another Space, another app…).
        let (label, icon) = if v.applied { ("Set Again", Icon::Check) } else { ("Set as Wallpaper", Icon::Display) };
        let resp = if v.applied {
            w::button(ui, label, Some(icon), v.can_apply)
        } else {
            w::accent_button(ui, label, Some(icon), v.can_apply)
        };
        if resp.clicked() {
            out.push(MiniIntent::Act(Action::Apply));
        }
        if v.loading {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spinner();
            });
        }
    });
    ui.add_space(8.0);
    ui.separator();

    ui.horizontal(|ui| {
        let mut daily = v.daily;
        w::checkbox(ui, &mut daily, v.daily_supported);
        ui.label("Change wallpaper every day");
        if daily != v.daily {
            out.push(MiniIntent::SetDaily(daily));
        }
    });
    match &v.update {
        Some((label, false)) => {
            if w::button(ui, label, Some(Icon::Download), true).clicked() {
                out.push(MiniIntent::Update);
            }
        }
        Some((_, true)) => {
            if w::accent_button(ui, "Restart to Update", None, true).clicked() {
                out.push(MiniIntent::Restart);
            }
        }
        None => {
            let (text, error) = &v.status;
            let short: String = text.chars().take(48).collect();
            let color = if *error { p.red } else { p.weak };
            ui.label(RichText::new(short).size(11.0).color(color)).on_hover_text(text.as_str());
        }
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if w::button(ui, "Open Unbing", None, true).clicked() {
            out.push(MiniIntent::Act(Action::ShowWindow));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::button(ui, "Quit", None, true).clicked() {
                out.push(MiniIntent::Act(Action::Quit));
            }
            if w::icon_button(ui, Icon::Gear, "Settings", true).clicked() {
                out.push(MiniIntent::Act(Action::ShowWindow));
                out.push(MiniIntent::Act(Action::Settings));
            }
        });
    });
}
