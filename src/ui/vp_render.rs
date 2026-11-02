//! Viewport drawing: shaded / wireframe / red-cyan anaglyph views with grid, helpers and selection.

use super::viewport::{DisplayMode, ViewState};
use crate::dag::{NodeId, Scene};
use crate::math::{Mat4, Vec3};
use crate::render::anaglyph::{combine, default_stereo, eye_matrices};
use crate::render::rasterizer::{project_line, rasterize, shade, splat_particles, walk_line, GBuffer};
use crate::render::{build_scene, camera_info, RenderScene};

fn gradient(y: usize, h: usize) -> Vec3 {
    let t = y as f32 / h.max(1) as f32;
    let top = Vec3::new(0.27, 0.31, 0.37);
    let bot = Vec3::new(0.11, 0.12, 0.14);
    top.lerp(bot, t)
}

fn overlay_line(pix: &mut [Vec3], depth: Option<&[f32]>, w: usize, h: usize, vp: &Mat4, a: Vec3, b: Vec3, color: Vec3) {
    if let Some((s0, s1)) = project_line(vp, a, b, w, h) {
        walk_line(s0, s1, w, h, 1.0, |x, y, d, _| {
            let i = y as usize * w + x as usize;
            if let Some(dp) = depth {
                if d > dp[i] + 0.03 + d * 0.012 {
                    return;
                }
            }
            pix[i] = color;
        });
    }
}

fn draw_grid(pix: &mut [Vec3], depth: Option<&[f32]>, w: usize, h: usize, vp: &Mat4) {
    let minor = Vec3::new(0.36, 0.38, 0.42);
    let major = Vec3::new(0.52, 0.54, 0.58);
    let ax = Vec3::new(0.85, 0.30, 0.30);
    let az = Vec3::new(0.30, 0.42, 0.92);
    for i in -10..=10 {
        let f = i as f32;
        let (cx, cz) = if i == 0 {
            (ax, az)
        } else if i % 5 == 0 {
            (major, major)
        } else {
            (minor, minor)
        };
        overlay_line(pix, depth, w, h, vp, Vec3::new(-10.0, 0.0, f), Vec3::new(10.0, 0.0, f), cx);
        overlay_line(pix, depth, w, h, vp, Vec3::new(f, 0.0, -10.0), Vec3::new(f, 0.0, 10.0), cz);
    }
}

fn draw_item_edges(pix: &mut [Vec3], depth: Option<&[f32]>, w: usize, h: usize, vp: &Mat4, rs: &RenderScene, only_selected: bool, color_sel: Vec3, color: Vec3) {
    for item in &rs.items {
        if only_selected && !item.selected {
            continue;
        }
        let c = if item.selected { color_sel } else { color };
        let pos: Vec<Vec3> = item.mesh.positions.iter().map(|p| item.world.transform_point(*p)).collect();
        for (a, b) in item.mesh.edges() {
            overlay_line(pix, depth, w, h, vp, pos[a as usize], pos[b as usize], c);
        }
    }
}

fn render_eye(rs: &RenderScene, view: &Mat4, proj: &Mat4, eye: Vec3, w: usize, h: usize, mode: DisplayMode, grid: bool) -> Vec<Vec3> {
    let vp = *proj * *view;
    let sel_color = Vec3::new(1.0, 0.78, 0.2);
    if mode == DisplayMode::Wireframe {
        let mut gb = GBuffer::new(w, h);
        splat_particles(&mut gb, rs, view, proj);
        let shaded = shade(&gb, rs, eye, Vec3::ZERO);
        let mut pix: Vec<Vec3> = (0..w * h)
            .map(|i| if gb.depth[i].is_finite() { shaded[i] } else { gradient(i / w, h) })
            .collect();
        if grid {
            draw_grid(&mut pix, None, w, h, &vp);
        }
        draw_item_edges(&mut pix, None, w, h, &vp, rs, false, sel_color, Vec3::new(0.55, 0.95, 0.62));
        for g in &rs.gizmos {
            overlay_line(&mut pix, None, w, h, &vp, g.a, g.b, g.color);
        }
        return pix;
    }
    let gb = rasterize(rs, view, proj, w, h);
    let mut pix = shade(&gb, rs, eye, Vec3::ZERO);
    for i in 0..w * h {
        if !gb.depth[i].is_finite() {
            pix[i] = gradient(i / w, h);
        }
    }
    if grid {
        draw_grid(&mut pix, Some(&gb.depth), w, h, &vp);
    }
    draw_item_edges(&mut pix, Some(&gb.depth), w, h, &vp, rs, true, sel_color, sel_color);
    for g in &rs.gizmos {
        overlay_line(&mut pix, Some(&gb.depth), w, h, &vp, g.a, g.b, g.color);
    }
    pix
}

/// Render the viewport into `w` x `h` pixels (half block cells give two pixel rows per text row).
pub fn render_view(scene: &mut Scene, view: &ViewState, w: usize, h: usize) -> Vec<Vec3> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let rs = build_scene(scene);
    let cam = match camera_info(scene, view.camera.as_deref()).or_else(|_| camera_info(scene, None)) {
        Ok(c) => c,
        Err(_) => return (0..w * h).map(|i| gradient(i / w, h)).collect(),
    };
    let aspect = w as f32 / h as f32;
    if view.mode == DisplayMode::Anaglyph {
        let mut st = cam.stereo.clone().unwrap_or_else(|| default_stereo(&cam));
        if cam.stereo.is_none() {
            st.interaxial *= view.stereo_scale;
        }
        let (l, r) = eye_matrices(&cam, &st, aspect);
        let left = render_eye(&rs, &l.view, &l.proj, l.eye, w, h, DisplayMode::Shaded, view.grid);
        let right = render_eye(&rs, &r.view, &r.proj, r.eye, w, h, DisplayMode::Shaded, view.grid);
        return combine(&left, &right);
    }
    render_eye(&rs, &cam.view(), &cam.projection(aspect), cam.eye(), w, h, view.mode, view.grid)
}

fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let t = o - a;
    let u = t.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let tt = e2.dot(q) * inv;
    if tt > 1e-4 {
        Some(tt)
    } else {
        None
    }
}

/// Cast a ray through normalized viewport coordinates (0..1, origin top left) and return the mesh node hit.
pub fn pick(scene: &mut Scene, view: &ViewState, nx: f32, ny: f32, aspect: f32) -> Option<NodeId> {
    let rs = build_scene(scene);
    let cam = camera_info(scene, view.camera.as_deref()).or_else(|_| camera_info(scene, None)).ok()?;
    pick_matrix(&rs, &(cam.projection(aspect) * cam.view()), nx, ny)
}

/// Same as `pick` but with an explicit view-projection matrix (used by the windowed viewports).
pub fn pick_matrix(rs: &RenderScene, view_proj: &Mat4, nx: f32, ny: f32) -> Option<NodeId> {
    let inv = view_proj.inverse();
    let x = nx * 2.0 - 1.0;
    let y = 1.0 - ny * 2.0;
    let unproject = |z: f32| -> Vec3 {
        let v = inv * crate::math::Vec4::new(x, y, z, 1.0);
        if v.w.abs() > 1e-12 {
            v.xyz() / v.w
        } else {
            v.xyz()
        }
    };
    let near = unproject(-1.0);
    let far = unproject(1.0);
    let dir = (far - near).normalized();
    let mut best: Option<(f32, NodeId)> = None;
    for item in &rs.items {
        let pos: Vec<Vec3> = item.mesh.positions.iter().map(|p| item.world.transform_point(*p)).collect();
        for t in item.mesh.triangulate() {
            if let Some(d) = ray_tri(near, dir, pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]) {
                if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                    best = Some((d, item.node));
                }
            }
        }
    }
    best.map(|(_, n)| n)
}
