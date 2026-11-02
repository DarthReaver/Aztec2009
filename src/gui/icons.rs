//! Shelf, tool box and transport icons drawn directly with the painter (resolution independent).

use eframe::egui::{epaint::PathStroke, pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Select,
    Move,
    Rotate,
    Scale,
    Cube,
    Sphere,
    Cylinder,
    Plane,
    Camera,
    StereoCamera,
    Light,
    Directional,
    Particles,
    Material,
    Key,
    Group,
    Duplicate,
    Delete,
    Frame,
    Single,
    Four,
    PerspOutliner,
    Rewind,
    StepBack,
    PlayBack,
    Play,
    Pause,
    StepFwd,
    End,
    Render,
    Layer,
    Grid,
    Wire,
    Smooth,
    Textured,
    Anaglyph,
    Extrude,
    Bevel,
    Merge,
    SplitTool,
    LoopTool,
    Smooth2,
    Combine,
    Separate,
    Mirror,
    DelEdge,
    Triangulate,
    FillHole,
    Joint,
    IkHandle,
    Bind,
}

fn line(p: &Painter, a: Pos2, b: Pos2, c: Color32, w: f32) {
    p.line_segment([a, b], Stroke::new(w, c));
}

fn poly(p: &Painter, pts: Vec<Pos2>, fill: Color32, stroke: Color32) {
    p.add(Shape::convex_polygon(pts, fill, Stroke::new(1.2_f32, stroke)));
}

pub fn draw(p: &Painter, r: Rect, icon: Icon) {
    let c = r.center();
    let s = r.width().min(r.height()) * 0.5;
    let ink = Color32::from_rgb(0x20, 0x20, 0x20);
    let blue = Color32::from_rgb(0x3a, 0x66, 0xb8);
    let blue_l = Color32::from_rgb(0x8f, 0xb4, 0xe8);
    let green = Color32::from_rgb(0x3f, 0x9a, 0x3f);
    let red = Color32::from_rgb(0xc2, 0x3a, 0x30);
    let orange = Color32::from_rgb(0xe0, 0x90, 0x30);
    let yellow = Color32::from_rgb(0xf2, 0xd2, 0x3c);
    let gray = Color32::from_rgb(0x8c, 0x8c, 0x8c);
    let gray_l = Color32::from_rgb(0xc8, 0xc8, 0xd0);
    match icon {
        Icon::Select => {
            let pts = vec![
                c + vec2(-0.45, -0.7) * s,
                c + vec2(-0.45, 0.6) * s,
                c + vec2(-0.12, 0.3) * s,
                c + vec2(0.15, 0.78) * s,
                c + vec2(0.35, 0.68) * s,
                c + vec2(0.1, 0.22) * s,
                c + vec2(0.55, 0.2) * s,
            ];
            poly(p, pts, Color32::WHITE, ink);
        }
        Icon::Move => {
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                let tip = c + vec2(dx, dy) * s * 0.85;
                line(p, c, tip, ink, 1.6);
                let (px, py) = (-dy, dx);
                poly(p, vec![tip + vec2(dx, dy) * s * 0.15, tip - vec2(dx, dy) * s * 0.3 + vec2(px, py) * s * 0.28, tip - vec2(dx, dy) * s * 0.3 - vec2(px, py) * s * 0.28], blue, ink);
            }
        }
        Icon::Rotate => {
            p.circle_stroke(c, s * 0.62, Stroke::new(2.0_f32, green));
            p.circle_stroke(c, s * 0.62, Stroke::new(0.6_f32, ink));
            let tip = c + vec2(0.62, -0.1) * s;
            poly(p, vec![tip + vec2(0.0, 0.35) * s, tip + vec2(-0.25, -0.1) * s, tip + vec2(0.25, -0.1) * s], red, ink);
        }
        Icon::Scale => {
            let b = Rect::from_center_size(c + vec2(-0.2, 0.2) * s, vec2(1.0, 1.0) * s);
            p.rect(b, 0.0, blue_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            let b2 = Rect::from_center_size(c + vec2(0.3, -0.3) * s, vec2(0.6, 0.6) * s);
            p.rect(b2, 0.0, orange, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            line(p, c + vec2(0.05, 0.0) * s, c + vec2(0.65, -0.65) * s, ink, 1.2);
        }
        Icon::Cube => {
            let o = c + vec2(0.0, 0.05) * s;
            let top = vec![o + vec2(0.0, -0.8) * s, o + vec2(0.75, -0.4) * s, o + vec2(0.0, 0.0) * s, o + vec2(-0.75, -0.4) * s];
            let left = vec![o + vec2(-0.75, -0.4) * s, o + vec2(0.0, 0.0) * s, o + vec2(0.0, 0.85) * s, o + vec2(-0.75, 0.45) * s];
            let right = vec![o + vec2(0.75, -0.4) * s, o + vec2(0.0, 0.0) * s, o + vec2(0.0, 0.85) * s, o + vec2(0.75, 0.45) * s];
            poly(p, top, Color32::from_rgb(0xe6, 0xe9, 0xf2), ink);
            poly(p, left, gray_l, ink);
            poly(p, right, gray, ink);
        }
        Icon::Sphere => {
            p.circle(c, s * 0.78, gray_l, Stroke::new(1.2_f32, ink));
            p.circle_filled(c + vec2(-0.25, -0.25) * s, s * 0.22, Color32::WHITE);
            p.circle_stroke(c, s * 0.78, Stroke::new(1.2_f32, ink));
        }
        Icon::Cylinder => {
            let top = Rect::from_center_size(c + vec2(0.0, -0.55) * s, vec2(1.3, 0.55) * s);
            p.rect_filled(Rect::from_min_max(pos2(top.left(), top.center().y), pos2(top.right(), c.y + 0.55 * s)), 0.0, gray);
            p.circle_filled(c + vec2(0.0, 0.55) * s, 0.0, gray);
            p.add(Shape::ellipse_filled(c + vec2(0.0, 0.55) * s, vec2(0.65, 0.28) * s, gray));
            p.add(Shape::ellipse_filled(c + vec2(0.0, -0.55) * s, vec2(0.65, 0.28) * s, Color32::from_rgb(0xe6, 0xe9, 0xf2)));
            p.add(Shape::ellipse_stroke(c + vec2(0.0, -0.55) * s, vec2(0.65, 0.28) * s, Stroke::new(1.2_f32, ink)));
            line(p, c + vec2(-0.65, -0.55) * s, c + vec2(-0.65, 0.55) * s, ink, 1.2);
            line(p, c + vec2(0.65, -0.55) * s, c + vec2(0.65, 0.55) * s, ink, 1.2);
        }
        Icon::Plane => {
            let pts = vec![c + vec2(-0.2, -0.55) * s, c + vec2(0.85, -0.2) * s, c + vec2(0.2, 0.55) * s, c + vec2(-0.85, 0.2) * s];
            poly(p, pts, Color32::from_rgb(0xe6, 0xe9, 0xf2), ink);
            line(p, c + vec2(-0.5, -0.17) * s, c + vec2(0.52, 0.18) * s, gray, 1.0);
            line(p, c + vec2(0.32, -0.38) * s, c + vec2(-0.32, 0.38) * s, gray, 1.0);
        }
        Icon::Camera | Icon::StereoCamera => {
            let b = Rect::from_center_size(c + vec2(-0.1, 0.0) * s, vec2(1.2, 0.85) * s);
            p.rect(b, 1.0, Color32::from_rgb(0x4a, 0x4a, 0x50), Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            poly(p, vec![c + vec2(0.5, 0.0) * s, c + vec2(0.95, -0.4) * s, c + vec2(0.95, 0.4) * s], gray_l, ink);
            p.circle_filled(c + vec2(-0.45, -0.55) * s, s * 0.22, gray_l);
            if icon == Icon::StereoCamera {
                p.circle_filled(c + vec2(0.05, -0.55) * s, s * 0.22, red);
            }
        }
        Icon::Light | Icon::Directional => {
            p.circle(c, s * 0.38, yellow, Stroke::new(1.2_f32, ink));
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                line(p, c + d * s * 0.55, c + d * s * 0.85, ink, 1.2);
            }
            if icon == Icon::Directional {
                line(p, c + vec2(-0.8, 0.85) * s, c + vec2(0.0, 0.2) * s, blue, 2.0);
            }
        }
        Icon::Particles => {
            for (dx, dy, r) in [(-0.5, 0.4, 0.2), (0.0, -0.1, 0.26), (0.5, 0.5, 0.17), (0.45, -0.5, 0.2), (-0.35, -0.55, 0.15), (0.05, 0.65, 0.12)] {
                p.circle(c + vec2(dx, dy) * s, r * s, Color32::from_rgb(0x70, 0xa8, 0xf0), Stroke::new(1.0_f32, ink));
            }
        }
        Icon::Material => {
            p.circle(c, s * 0.75, Color32::from_rgb(0xd8, 0x55, 0x45), Stroke::new(1.2_f32, ink));
            p.circle_filled(c + vec2(-0.25, -0.25) * s, s * 0.2, Color32::from_rgb(0xf8, 0xc0, 0xb0));
        }
        Icon::Key => {
            line(p, c + vec2(-0.7, 0.0) * s, c + vec2(0.2, 0.0) * s, ink, 1.6);
            p.circle(c + vec2(0.45, 0.0) * s, s * 0.32, red, Stroke::new(1.2_f32, ink));
            line(p, c + vec2(-0.5, 0.0) * s, c + vec2(-0.5, 0.35) * s, ink, 1.6);
            line(p, c + vec2(-0.2, 0.0) * s, c + vec2(-0.2, 0.3) * s, ink, 1.6);
        }
        Icon::Group => {
            p.rect(Rect::from_center_size(c + vec2(-0.2, -0.2) * s, vec2(0.9, 0.9) * s), 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            p.rect(Rect::from_center_size(c + vec2(0.25, 0.25) * s, vec2(0.9, 0.9) * s), 0.0, blue_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
        }
        Icon::Duplicate => {
            p.rect(Rect::from_center_size(c + vec2(-0.25, -0.2) * s, vec2(0.9, 0.9) * s), 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            p.rect(Rect::from_center_size(c + vec2(0.25, 0.25) * s, vec2(0.9, 0.9) * s), 0.0, Color32::WHITE, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
        }
        Icon::Delete => {
            line(p, c + vec2(-0.6, -0.6) * s, c + vec2(0.6, 0.6) * s, red, 2.6);
            line(p, c + vec2(0.6, -0.6) * s, c + vec2(-0.6, 0.6) * s, red, 2.6);
        }
        Icon::Frame => {
            let b = Rect::from_center_size(c, vec2(1.3, 1.3) * s);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let corner = c + vec2(sx, sy) * 0.65 * s;
                line(p, corner, corner - vec2(sx, 0.0) * 0.4 * s, ink, 1.6);
                line(p, corner, corner - vec2(0.0, sy) * 0.4 * s, ink, 1.6);
            }
            p.circle_filled(b.center(), s * 0.15, red);
        }
        Icon::Single => {
            p.rect(Rect::from_center_size(c, vec2(1.5, 1.2) * s), 0.0, Color32::from_rgb(0x76, 0x76, 0x76), Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
        }
        Icon::Four => {
            let b = Rect::from_center_size(c, vec2(1.5, 1.2) * s);
            p.rect(b, 0.0, Color32::from_rgb(0x76, 0x76, 0x76), Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            line(p, pos2(b.center().x, b.top()), pos2(b.center().x, b.bottom()), ink, 1.2);
            line(p, pos2(b.left(), b.center().y), pos2(b.right(), b.center().y), ink, 1.2);
        }
        Icon::PerspOutliner => {
            let b = Rect::from_center_size(c, vec2(1.5, 1.2) * s);
            p.rect(b, 0.0, Color32::from_rgb(0x76, 0x76, 0x76), Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            let x = b.left() + b.width() * 0.35;
            p.rect_filled(Rect::from_min_max(b.min, pos2(x, b.bottom())), 0.0, Color32::from_rgb(0xd8, 0xd8, 0xd8));
            line(p, pos2(x, b.top()), pos2(x, b.bottom()), ink, 1.2);
        }
        Icon::Rewind | Icon::End => {
            let d = if icon == Icon::Rewind { -1.0 } else { 1.0 };
            line(p, c + vec2(d * 0.7, -0.6) * s, c + vec2(d * 0.7, 0.6) * s, ink, 1.8);
            poly(p, vec![c + vec2(d * 0.5, 0.0) * s, c + vec2(-d * 0.1, -0.6) * s, c + vec2(-d * 0.1, 0.6) * s], ink, ink);
            poly(p, vec![c + vec2(-d * 0.1, 0.0) * s, c + vec2(-d * 0.7, -0.6) * s, c + vec2(-d * 0.7, 0.6) * s], ink, ink);
        }
        Icon::StepBack | Icon::StepFwd => {
            let d = if icon == Icon::StepBack { -1.0 } else { 1.0 };
            line(p, c + vec2(d * 0.55, -0.6) * s, c + vec2(d * 0.55, 0.6) * s, ink, 1.8);
            poly(p, vec![c + vec2(d * 0.35, 0.0) * s, c + vec2(-d * 0.45, -0.6) * s, c + vec2(-d * 0.45, 0.6) * s], ink, ink);
        }
        Icon::PlayBack => {
            poly(p, vec![c + vec2(-0.6, 0.0) * s, c + vec2(0.5, -0.65) * s, c + vec2(0.5, 0.65) * s], Color32::from_rgb(0x30, 0x80, 0x30), ink);
        }
        Icon::Play => {
            poly(p, vec![c + vec2(0.6, 0.0) * s, c + vec2(-0.5, -0.65) * s, c + vec2(-0.5, 0.65) * s], Color32::from_rgb(0x30, 0x80, 0x30), ink);
        }
        Icon::Pause => {
            p.rect_filled(Rect::from_center_size(c + vec2(-0.3, 0.0) * s, vec2(0.38, 1.3) * s), 0.0, red);
            p.rect_filled(Rect::from_center_size(c + vec2(0.3, 0.0) * s, vec2(0.38, 1.3) * s), 0.0, red);
        }
        Icon::Render => {
            let b = Rect::from_center_size(c, vec2(1.5, 1.1) * s);
            p.rect(b, 1.0, Color32::from_rgb(0xe8, 0xe8, 0xe8), Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            poly(p, vec![b.left_bottom() + vec2(0.1, -0.1) * s, b.left_bottom() + vec2(0.55, -0.6) * s, b.left_bottom() + vec2(0.9, -0.1) * s], green, ink);
            p.circle_filled(b.right_top() + vec2(-0.35, 0.3) * s, s * 0.14, orange);
        }
        Icon::Layer => {
            for i in 0..3 {
                let o = vec2(0.0, (i as f32 - 1.0) * 0.45) * s;
                poly(p, vec![c + o + vec2(0.0, -0.3) * s, c + o + vec2(0.8, 0.0) * s, c + o + vec2(0.0, 0.3) * s, c + o + vec2(-0.8, 0.0) * s], blue_l, ink);
            }
        }
        Icon::Grid => {
            let b = Rect::from_center_size(c, vec2(1.4, 1.4) * s);
            for i in 0..=3 {
                let t = i as f32 / 3.0;
                line(p, pos2(b.left(), b.top() + t * b.height()), pos2(b.right(), b.top() + t * b.height()), ink, 1.0);
                line(p, pos2(b.left() + t * b.width(), b.top()), pos2(b.left() + t * b.width(), b.bottom()), ink, 1.0);
            }
        }
        Icon::Wire => {
            p.circle_stroke(c, s * 0.75, Stroke::new(1.2_f32, ink));
            p.add(Shape::ellipse_stroke(c, vec2(0.75, 0.3) * s, Stroke::new(1.0_f32, ink)));
            p.add(Shape::ellipse_stroke(c, vec2(0.3, 0.75) * s, Stroke::new(1.0_f32, ink)));
        }
        Icon::Smooth => {
            p.circle(c, s * 0.75, gray_l, Stroke::new(1.2_f32, ink));
            p.circle_filled(c + vec2(-0.25, -0.25) * s, s * 0.2, Color32::WHITE);
        }
        Icon::Textured => {
            // a sphere with a checker pattern
            p.circle(c, s * 0.75, gray_l, Stroke::new(1.2_f32, ink));
            for (dx, dy) in [(-0.3, -0.3), (0.1, 0.1), (-0.3, 0.1), (0.1, -0.3)] {
                let on = (dx < 0.0) == (dy < 0.0);
                if on {
                    p.rect_filled(Rect::from_min_size(c + vec2(dx, dy) * s, vec2(0.4, 0.4) * s), 0.0, Color32::from_rgb(0xc0, 0x50, 0x30));
                }
            }
        }
        Icon::Extrude => {
            let base = Rect::from_center_size(c + vec2(0.0, 0.5) * s, vec2(1.3, 0.5) * s);
            p.rect(base, 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            let top = Rect::from_center_size(c + vec2(0.0, -0.35) * s, vec2(0.8, 0.5) * s);
            p.rect(top, 0.0, Color32::from_rgb(0xff, 0xb0, 0x40), Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            line(p, base.center_top() + vec2(-0.4, 0.0) * s, top.center_bottom() + vec2(-0.4, 0.0) * s, ink, 1.2);
            line(p, base.center_top() + vec2(0.4, 0.0) * s, top.center_bottom() + vec2(0.4, 0.0) * s, ink, 1.2);
        }
        Icon::Bevel => {
            let pts = vec![c + vec2(-0.7, -0.7) * s, c + vec2(0.25, -0.7) * s, c + vec2(0.7, -0.25) * s, c + vec2(0.7, 0.7) * s, c + vec2(-0.7, 0.7) * s];
            poly(p, pts, gray_l, ink);
            line(p, c + vec2(0.25, -0.7) * s, c + vec2(0.7, -0.25) * s, orange, 2.4);
        }
        Icon::Merge => {
            p.circle_filled(c + vec2(-0.7, 0.0) * s, s * 0.14, ink);
            p.circle_filled(c + vec2(0.7, 0.0) * s, s * 0.14, ink);
            line(p, c + vec2(-0.55, 0.0) * s, c + vec2(-0.12, 0.0) * s, blue, 2.0);
            line(p, c + vec2(0.55, 0.0) * s, c + vec2(0.12, 0.0) * s, blue, 2.0);
            p.circle_filled(c, s * 0.2, orange);
        }
        Icon::SplitTool => {
            let b = Rect::from_center_size(c, vec2(1.5, 1.2) * s);
            p.rect(b, 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            line(p, b.left_top() + vec2(0.0, 0.2) * s, b.right_bottom() - vec2(0.0, 0.3) * s, orange, 2.0);
            p.circle_filled(b.left_top() + vec2(0.0, 0.2) * s, s * 0.12, red);
            p.circle_filled(b.right_bottom() - vec2(0.0, 0.3) * s, s * 0.12, red);
        }
        Icon::LoopTool => {
            let b = Rect::from_center_size(c, vec2(1.5, 1.3) * s);
            p.rect(b, 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            line(p, pos2(b.left(), c.y), pos2(b.right(), c.y), orange, 2.4);
            line(p, pos2(c.x - 0.25 * s, b.top()), pos2(c.x - 0.25 * s, b.bottom()), ink, 1.0);
            line(p, pos2(c.x + 0.25 * s, b.top()), pos2(c.x + 0.25 * s, b.bottom()), ink, 1.0);
        }
        Icon::Smooth2 => {
            let b = Rect::from_center_size(c, vec2(1.5, 1.5) * s);
            p.rect_stroke(b, 0.0, Stroke::new(1.0_f32, gray), eframe::egui::StrokeKind::Inside);
            p.circle(c, s * 0.75, gray_l, Stroke::new(1.4_f32, ink));
        }
        Icon::Combine => {
            p.rect(Rect::from_center_size(c + vec2(-0.55, 0.0) * s, vec2(0.8, 0.8) * s), 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            p.rect(Rect::from_center_size(c + vec2(0.55, 0.0) * s, vec2(0.8, 0.8) * s), 0.0, blue_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            line(p, c + vec2(-0.1, 0.0) * s, c + vec2(0.1, 0.0) * s, ink, 2.0);
        }
        Icon::Separate => {
            p.rect(Rect::from_center_size(c + vec2(-0.65, -0.3) * s, vec2(0.7, 0.7) * s), 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            p.rect(Rect::from_center_size(c + vec2(0.65, 0.3) * s, vec2(0.7, 0.7) * s), 0.0, blue_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
        }
        Icon::Mirror => {
            poly(p, vec![c + vec2(-0.1, -0.7) * s, c + vec2(-0.8, 0.5) * s, c + vec2(-0.1, 0.5) * s], gray_l, ink);
            poly(p, vec![c + vec2(0.1, -0.7) * s, c + vec2(0.8, 0.5) * s, c + vec2(0.1, 0.5) * s], blue_l, ink);
            line(p, c + vec2(0.0, -0.85) * s, c + vec2(0.0, 0.85) * s, red, 1.2);
        }
        Icon::DelEdge => {
            line(p, c + vec2(-0.8, 0.5) * s, c + vec2(0.8, -0.5) * s, orange, 2.4);
            p.circle_filled(c + vec2(-0.8, 0.5) * s, s * 0.14, ink);
            p.circle_filled(c + vec2(0.8, -0.5) * s, s * 0.14, ink);
            line(p, c + vec2(-0.35, -0.35) * s, c + vec2(0.35, 0.35) * s, red, 2.4);
            line(p, c + vec2(0.35, -0.35) * s, c + vec2(-0.35, 0.35) * s, red, 2.4);
        }
        Icon::FillHole => {
            let b = Rect::from_center_size(c, vec2(1.5, 1.5) * s);
            p.rect(b, 0.0, Color32::from_rgb(0xff, 0xb0, 0x40), Stroke::new(1.4_f32, ink), eframe::egui::StrokeKind::Inside);
            p.rect(b.shrink(s * 0.35), 0.0, Color32::from_rgb(0x50, 0x50, 0x50), Stroke::new(1.0_f32, ink), eframe::egui::StrokeKind::Inside);
        }
        Icon::Joint => {
            line(p, c + vec2(-0.7, 0.7) * s, c + vec2(0.0, 0.0) * s, ink, 2.0);
            line(p, c + vec2(0.0, 0.0) * s, c + vec2(0.7, -0.6) * s, ink, 2.0);
            for (x, y) in [(-0.7, 0.7), (0.0, 0.0), (0.7, -0.6)] {
                p.circle(c + vec2(x, y) * s, s * 0.22, Color32::from_rgb(0x90, 0xf0, 0x50), Stroke::new(1.2_f32, ink));
            }
        }
        Icon::IkHandle => {
            line(p, c + vec2(-0.7, 0.7) * s, c + vec2(-0.1, 0.1) * s, gray, 2.0);
            line(p, c + vec2(-0.1, 0.1) * s, c + vec2(0.3, -0.5) * s, gray, 2.0);
            line(p, c + vec2(0.3, -0.5) * s, c + vec2(0.8, -0.8) * s, Color32::from_rgb(0x20, 0x90, 0xc0), 2.4);
            p.circle_filled(c + vec2(0.8, -0.8) * s, s * 0.2, Color32::from_rgb(0x20, 0x90, 0xc0));
        }
        Icon::Bind => {
            p.rect(Rect::from_center_size(c + vec2(0.35, 0.0) * s, vec2(0.8, 1.4) * s), 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            line(p, c + vec2(-0.8, -0.6) * s, c + vec2(-0.1, 0.0) * s, Color32::from_rgb(0x50, 0xb0, 0x30), 2.4);
            line(p, c + vec2(-0.1, 0.0) * s, c + vec2(-0.8, 0.6) * s, Color32::from_rgb(0x50, 0xb0, 0x30), 2.4);
        }
        Icon::Triangulate => {
            let b = Rect::from_center_size(c, vec2(1.5, 1.5) * s);
            p.rect(b, 0.0, gray_l, Stroke::new(1.2_f32, ink), eframe::egui::StrokeKind::Inside);
            line(p, b.left_top(), b.right_bottom(), ink, 1.4);
        }
        Icon::Anaglyph => {
            p.circle_filled(c + vec2(-0.2, 0.0) * s, s * 0.55, Color32::from_rgba_unmultiplied(0xe0, 0x30, 0x30, 200));
            p.circle_filled(c + vec2(0.2, 0.0) * s, s * 0.55, Color32::from_rgba_unmultiplied(0x30, 0xc0, 0xd0, 200));
        }
    }
    let _ = PathStroke::NONE;
}
