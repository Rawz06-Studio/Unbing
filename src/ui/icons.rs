//! Small line icons drawn with the painter (no icon font needed, crisp at any scale).

use eframe::egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Check,
    Folder,
    Gear,
    Photo,
    Display,
    ChevronLeft,
    ChevronRight,
    Download,
}

/// Draws `icon` centered in `rect`, on a 16×16 design grid scaled to the rect.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 16.0;
    let o = rect.center() - vec2(8.0, 8.0) * s;
    let p = |x: f32, y: f32| -> Pos2 { o + vec2(x, y) * s };
    let stroke = Stroke::new(1.4 * s.max(1.0), color);
    let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);
    let r = |v: f32| CornerRadius::same((v * s) as u8);

    match icon {
        Icon::Check => {
            painter.add(Shape::line(vec![p(3.5, 8.5), p(6.5, 11.5), p(12.5, 4.5)], Stroke::new(1.8 * s, color)));
        }
        Icon::Folder => {
            painter.add(Shape::line(
                vec![p(2.0, 12.5), p(2.0, 3.5), p(6.0, 3.5), p(7.5, 5.0), p(14.0, 5.0), p(14.0, 12.5), p(2.0, 12.5)],
                stroke,
            ));
            line(p(2.0, 7.0), p(14.0, 7.0));
        }
        Icon::Gear => {
            painter.circle_stroke(p(8.0, 8.0), 2.2 * s, stroke);
            painter.circle_stroke(p(8.0, 8.0), 5.0 * s, stroke);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let (sin, cos) = a.sin_cos();
                line(p(8.0 + 5.0 * cos, 8.0 + 5.0 * sin), p(8.0 + 6.8 * cos, 8.0 + 6.8 * sin));
            }
        }
        Icon::Photo => {
            painter.rect_stroke(Rect::from_min_max(p(1.5, 3.0), p(14.5, 13.0)), r(2.0), stroke, StrokeKind::Middle);
            painter
                .add(Shape::line(vec![p(2.0, 12.0), p(6.0, 8.0), p(9.0, 10.5), p(11.0, 9.0), p(14.0, 11.5)], stroke));
            painter.circle_filled(p(10.8, 6.0), 1.2 * s, color);
        }
        Icon::Display => {
            painter.rect_stroke(Rect::from_min_max(p(1.5, 2.5), p(14.5, 11.0)), r(1.5), stroke, StrokeKind::Middle);
            line(p(8.0, 11.0), p(8.0, 13.5));
            line(p(5.0, 13.5), p(11.0, 13.5));
        }
        Icon::ChevronLeft => {
            painter.add(Shape::line(vec![p(10.0, 3.0), p(5.0, 8.0), p(10.0, 13.0)], stroke));
        }
        Icon::Download => {
            line(p(8.0, 2.5), p(8.0, 10.0));
            painter.add(Shape::line(vec![p(4.5, 6.8), p(8.0, 10.3), p(11.5, 6.8)], stroke));
            line(p(3.0, 13.0), p(13.0, 13.0));
        }
        Icon::ChevronRight => {
            painter.add(Shape::line(vec![p(6.0, 3.0), p(11.0, 8.0), p(6.0, 13.0)], stroke));
        }
    }
}
