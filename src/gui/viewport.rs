//! Viewport rendering for the windowed workspace: cameras (perspective and pseudo orthographic),
//! supersampled shading, anti-aliased depth tested overlay lines and picking helpers.

use crate::dag::Scene;
use crate::math::{Mat4, Vec3};
use crate::render::anaglyph::{combine, default_stereo, eye_matrices};
use crate::render::rasterizer::{project_line, rasterize, shade, splat_particles, GBuffer};
use crate::render::{build_scene, camera_info, CameraInfo, RenderScene};
use crate::ui::viewport::{DisplayMode, ViewState};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewKind {
    Persp,
    Top,
    Front,
    Side,
}

impl ViewKind {
    pub fn label(self) -> &'static str {
        match self {
            ViewKind::Persp => "persp",
            ViewKind::Top => "top",
            ViewKind::Front => "front",
            ViewKind::Side => "side",
        }
    }
}

/// State of one orthographic view: what it looks at and the half height of the visible area.
#[derive(Clone, Copy, Debug)]
pub struct Ortho {
    pub target: Vec3,
    pub size: f32,
}

impl Default for Ortho {
    fn default() -> Self {
        Ortho { target: Vec3::ZERO, size: 12.0 }
    }
}

pub struct Cam {
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
    pub kind: ViewKind,
    pub size: f32,
    pub info: Option<CameraInfo>,
}

impl Cam {
    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }
}

const ORTHO_DIST: f32 = 4000.0;

pub fn make_cam(scene: &mut Scene, view: &ViewState, kind: ViewKind, ortho: &Ortho, aspect: f32) -> Option<Cam> {
    if kind == ViewKind::Persp {
        let info = camera_info(scene, view.camera.as_deref()).or_else(|_| camera_info(scene, None)).ok()?;
        return Some(Cam { view: info.view(), proj: info.projection(aspect), eye: info.eye(), kind, size: view.dist, info: Some(info) });
    }
    let (dir, up) = match kind {
        ViewKind::Top => (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, -1.0)),
        ViewKind::Front => (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)),
        _ => (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)),
    };
    let eye = ortho.target + dir * ORTHO_DIST;
    let fov = 2.0 * (ortho.size / ORTHO_DIST).atan();
    Some(Cam {
        view: Mat4::look_at(eye, ortho.target, up),
        proj: Mat4::perspective(fov, aspect, ORTHO_DIST - 2500.0, ORTHO_DIST + 2500.0),
        eye,
        kind,
        size: ortho.size,
        info: None,
    })
}

/// Project a world point into panel pixels (origin top left, `w` x `h`). None when behind the camera.
pub fn project(vp: &Mat4, p: Vec3, w: f32, h: f32) -> Option<(f32, f32, f32)> {
    let c = *vp * p.extend(1.0);
    if c.w <= 1e-6 || c.z + c.w < 0.0 {
        return None;
    }
    let inv = 1.0 / c.w;
    Some(((c.x * inv * 0.5 + 0.5) * w, (1.0 - (c.y * inv * 0.5 + 0.5)) * h, c.w))
}

struct Canvas<'a> {
    pix: &'a mut [Vec3],
    depth: Option<&'a [f32]>,
    w: usize,
    h: usize,
    bias0: f32,
    bias_k: f32,
}

impl Canvas<'_> {
    fn plot(&mut self, x: i32, y: i32, depth: f32, color: Vec3, cov: f32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 || cov <= 0.0 {
            return;
        }
        let i = y as usize * self.w + x as usize;
        if let Some(dp) = self.depth {
            if depth > dp[i] + self.bias0 + depth * self.bias_k {
                return;
            }
        }
        self.pix[i] = self.pix[i].lerp(color, cov.min(1.0));
    }

    /// Anti-aliased line of `width` pixels between two projected points (x, y, 1/w).
    fn line(&mut self, vp: &Mat4, a: Vec3, b: Vec3, color: Vec3, alpha: f32, width: f32) {
        let (s0, s1) = match project_line(vp, a, b, self.w, self.h) {
            Some(v) => v,
            None => return,
        };
        // Liang-Barsky clip against the canvas (with a margin) so huge off-screen lines stay cheap.
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        let (dx, dy) = (s1.0 - s0.0, s1.1 - s0.1);
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let m = width + 2.0;
        let checks = [
            (-dx, s0.0 + m),
            (dx, self.w as f32 + m - s0.0),
            (-dy, s0.1 + m),
            (dy, self.h as f32 + m - s0.1),
        ];
        for (p, q) in checks {
            if p == 0.0 {
                if q < 0.0 {
                    return;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if t0 > t1 {
            return;
        }
        let lerp3 = |t: f32| (s0.0 + dx * t, s0.1 + dy * t, s0.2 + (s1.2 - s0.2) * t);
        let (p0, p1) = (lerp3(t0), lerp3(t1));
        let half = width * 0.5;
        let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
        let depth_at = |iw: f32| if iw > 0.0 { 1.0 / iw } else { f32::INFINITY };
        if dx.abs() < 1e-4 && dy.abs() < 1e-4 {
            self.plot(p0.0.floor() as i32, p0.1.floor() as i32, depth_at(p0.2), color, alpha);
            return;
        }
        let x_major = dx.abs() >= dy.abs();
        let (a0, a1, b0, b1, i0, i1) = if x_major { (p0.0, p1.0, p0.1, p1.1, p0.2, p1.2) } else { (p0.1, p1.1, p0.0, p1.0, p0.2, p1.2) };
        let (a0, a1, b0, b1, i0, i1) = if a1 < a0 { (a1, a0, b1, b0, i1, i0) } else { (a0, a1, b0, b1, i0, i1) };
        let span = (a1 - a0).max(1e-6);
        let first = a0.floor() as i32;
        let last = a1.floor() as i32;
        for ai in first..=last {
            let t = ((ai as f32 + 0.5 - a0) / span).clamp(0.0, 1.0);
            let bc = b0 + (b1 - b0) * t;
            let d = depth_at(i0 + (i1 - i0) * t);
            let lo = (bc - half).floor() as i32;
            let hi = (bc + half).floor() as i32;
            for bj in lo..=hi {
                let cov = ((bj as f32 + 1.0).min(bc + half) - (bj as f32).max(bc - half)).clamp(0.0, 1.0);
                if x_major {
                    self.plot(ai, bj, d, color, cov * alpha);
                } else {
                    self.plot(bj, ai, d, color, cov * alpha);
                }
            }
        }
    }
}

fn grid_step(cam: &Cam, aspect: f32) -> (f32, f32) {
    if cam.kind == ViewKind::Persp {
        return (1.0, 12.0);
    }
    let reach = cam.size * aspect.max(1.0);
    let step = if reach < 45.0 {
        1.0
    } else if reach < 200.0 {
        5.0
    } else if reach < 800.0 {
        10.0
    } else {
        50.0
    };
    let ext = ((reach * 1.6 / step).ceil() * step).max(12.0);
    (step, ext)
}

fn draw_grid(cv: &mut Canvas, vp: &Mat4, cam: &Cam, aspect: f32, ss: f32, target: Vec3) {
    let (step, ext) = grid_step(cam, aspect);
    let minor = Vec3::splat(0.30);
    let major = Vec3::splat(0.22);
    let axis_dark = Vec3::splat(0.10);
    let n = (ext / step).round() as i32;
    // The plane the grid lies in, and the two in-plane axes.
    let (u, v) = match cam.kind {
        ViewKind::Persp | ViewKind::Top => (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)),
        ViewKind::Front => (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)),
        ViewKind::Side => (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)),
    };
    let _ = target;
    let e = ext;
    for i in -n..=n {
        let f = i as f32 * step;
        let (col, wd) = if i == 0 {
            (axis_dark, 1.6)
        } else if (f / step).round() as i32 % 5 == 0 {
            (major, 1.0)
        } else {
            (minor, 1.0)
        };
        cv.line(vp, u * -e + v * f, u * e + v * f, col, 1.0, wd * ss);
        cv.line(vp, v * -e + u * f, v * e + u * f, col, 1.0, wd * ss);
    }
}

fn draw_edges(cv: &mut Canvas, vp: &Mat4, rs: &RenderScene, only_selected: bool, ss: f32, unsel: Vec3) {
    let sel = Vec3::new(0.40, 1.0, 0.42);
    for item in &rs.items {
        if only_selected && !item.selected {
            continue;
        }
        let pos: Vec<Vec3> = item.mesh.positions.iter().map(|p| item.world.transform_point(*p)).collect();
        let (col, wd) = if item.selected { (sel, 1.3) } else { (unsel, 1.0) };
        for (a, b) in item.mesh.edges() {
            cv.line(vp, pos[a as usize], pos[b as usize], col, 1.0, wd * ss);
        }
    }
}

fn render_eye(rs: &RenderScene, cam: &Cam, view: &Mat4, proj: &Mat4, eye: Vec3, aspect: f32, w: usize, h: usize, ss: f32, mode: DisplayMode, grid: bool, overlays: bool, wire_all: bool) -> (Vec<Vec3>, Vec<f32>) {
    let vp = *proj * *view;
    let bg = Vec3::new(crate::gui::theme::VIEW_BG[0], crate::gui::theme::VIEW_BG[1], crate::gui::theme::VIEW_BG[2]);
    let (bias0, bias_k) = if cam.kind == ViewKind::Persp { (0.03, 0.010) } else { (cam.size * 0.01 + 0.02, 0.0) };
    let wire = mode == DisplayMode::Wireframe;
    let gb = if wire {
        let mut g = GBuffer::new(w, h);
        splat_particles(&mut g, rs, view, proj);
        g
    } else {
        rasterize(rs, view, proj, w, h)
    };
    let mut pix = shade(&gb, rs, eye, bg);
    let depth_out = if wire { Vec::new() } else { gb.depth.clone() };
    if !overlays {
        return (pix, depth_out);
    }
    let depth: Option<&[f32]> = if wire { None } else { Some(&gb.depth) };
    let mut cv = Canvas { pix: &mut pix, depth, w, h, bias0, bias_k };
    if grid {
        draw_grid(&mut cv, &vp, cam, aspect, ss, Vec3::ZERO);
    }
    draw_edges(&mut cv, &vp, rs, !wire && !wire_all, ss, Vec3::new(0.14, 0.18, 0.62));
    for g in &rs.gizmos {
        if g.xray {
            let saved = cv.depth;
            cv.depth = None;
            cv.line(&vp, g.a, g.b, g.color, 1.0, 1.4 * ss);
            cv.depth = saved;
        } else {
            cv.line(&vp, g.a, g.b, g.color, 1.0, 1.2 * ss);
        }
    }
    (pix, depth_out)
}

/// Render a panel and return RGBA8 bytes of exactly `w` x `h` pixels (`ss` x supersampled).
pub struct RenderOut {
    pub rgba: Vec<u8>,
    /// View depth of the (supersampled) first eye; empty in wireframe mode.
    pub depth: Vec<f32>,
    pub dw: usize,
    pub dh: usize,
}

pub fn render(rs: &RenderScene, cam: &Cam, mode: DisplayMode, grid: bool, overlays: bool, wire_all: bool, stereo_scale: f32, w: usize, h: usize, ss: usize) -> RenderOut {
    let ss = ss.max(1);
    let (bw, bh) = (w * ss, h * ss);
    let aspect = w as f32 / h as f32;
    let mut depth_keep: Vec<f32> = Vec::new();
    let buf = match (mode, &cam.info) {
        (DisplayMode::Anaglyph, Some(info)) => {
            let mut st = info.stereo.clone().unwrap_or_else(|| default_stereo(info));
            if info.stereo.is_none() {
                st.interaxial *= stereo_scale;
            }
            let (l, r) = eye_matrices(info, &st, aspect);
            let (left, ld) = render_eye(rs, cam, &l.view, &l.proj, l.eye, aspect, bw, bh, ss as f32, DisplayMode::Shaded, grid, overlays, wire_all);
            let (right, _) = render_eye(rs, cam, &r.view, &r.proj, r.eye, aspect, bw, bh, ss as f32, DisplayMode::Shaded, grid, overlays, wire_all);
            depth_keep = ld;
            combine(&left, &right)
        }
        _ => {
            let m = if mode == DisplayMode::Anaglyph { DisplayMode::Shaded } else { mode };
            let (pix, d) = render_eye(rs, cam, &cam.view, &cam.proj, cam.eye, aspect, bw, bh, ss as f32, m, grid, overlays, wire_all);
            depth_keep = d;
            pix
        }
    };
    let mut out = vec![255u8; w * h * 4];
    let inv = 1.0 / (ss * ss) as f32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = Vec3::ZERO;
            for sy in 0..ss {
                let row = (y * ss + sy) * bw + x * ss;
                for sx in 0..ss {
                    acc += buf[row + sx];
                }
            }
            let o = (y * w + x) * 4;
            out[o] = (acc.x * inv * 255.0).clamp(0.0, 255.0) as u8;
            out[o + 1] = (acc.y * inv * 255.0).clamp(0.0, 255.0) as u8;
            out[o + 2] = (acc.z * inv * 255.0).clamp(0.0, 255.0) as u8;
        }
    }
    RenderOut { rgba: out, depth: depth_keep, dw: bw, dh: bh }
}

/// World space bounds of the selected meshes (or of everything when `selected_only` is false).
pub fn bounds(rs: &RenderScene, selected_only: bool) -> Option<(Vec3, Vec3)> {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for it in &rs.items {
        if selected_only && !it.selected {
            continue;
        }
        for p in &it.mesh.positions {
            let w = it.world.transform_point(*p);
            lo = Vec3::new(lo.x.min(w.x), lo.y.min(w.y), lo.z.min(w.z));
            hi = Vec3::new(hi.x.max(w.x), hi.y.max(w.y), hi.z.max(w.z));
            any = true;
        }
    }
    if any {
        Some((lo, hi))
    } else {
        None
    }
}

pub fn scene_snapshot(scene: &mut Scene) -> RenderScene {
    build_scene(scene)
}
