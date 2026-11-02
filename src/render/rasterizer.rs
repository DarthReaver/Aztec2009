use super::{LightKind, RenderLight, RenderScene};
use crate::math::{Mat4, Vec2, Vec3, Vec4};

pub const NO_MAT: u16 = u16::MAX;

/// Geometry buffer produced by the single raster pass. All render passes derive from it.
pub struct GBuffer {
    pub w: usize,
    pub h: usize,
    /// First row (of the full image) stored in this buffer and the full image height.
    pub y0: usize,
    pub full_h: usize,
    /// Linear view depth (distance along the camera axis). INFINITY marks empty pixels.
    pub depth: Vec<f32>,
    pub normal: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub pos: Vec<Vec3>,
    pub mat: Vec<u16>,
    /// Wireframe layer: 0 none, 1 '-', 2 '|', 3 '/', 4 '\\'.
    pub wire: Vec<u8>,
}

impl GBuffer {
    pub fn new(w: usize, h: usize) -> GBuffer {
        let n = w * h;
        GBuffer {
            w,
            h,
            y0: 0,
            full_h: h,
            depth: vec![f32::INFINITY; n],
            normal: vec![Vec3::ZERO; n],
            uv: vec![Vec2::new(0.0, 0.0); n],
            pos: vec![Vec3::ZERO; n],
            mat: vec![NO_MAT; n],
            wire: vec![0; n],
        }
    }
    pub fn idx(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }

    /// A buffer holding rows `y0..y1` of a `w` x `full_h` image.
    pub fn band(w: usize, full_h: usize, y0: usize, y1: usize) -> GBuffer {
        let mut g = GBuffer::new(w, y1 - y0);
        g.y0 = y0;
        g.full_h = full_h;
        g
    }

    /// Stack consecutive bands (top to bottom) into one full buffer.
    pub fn concat(bands: Vec<GBuffer>) -> GBuffer {
        let w = bands.first().map(|b| b.w).unwrap_or(0);
        let h: usize = bands.iter().map(|b| b.h).sum();
        let mut g = GBuffer { w, h, y0: 0, full_h: h, depth: Vec::new(), normal: Vec::new(), uv: Vec::new(), pos: Vec::new(), mat: Vec::new(), wire: Vec::new() };
        for b in bands {
            g.depth.extend(b.depth);
            g.normal.extend(b.normal);
            g.uv.extend(b.uv);
            g.pos.extend(b.pos);
            g.mat.extend(b.mat);
            g.wire.extend(b.wire);
        }
        g
    }
}

#[derive(Clone, Copy)]
struct ClipVert {
    clip: Vec4,
    pos: Vec3,
    nrm: Vec3,
    uv: Vec2,
}

fn lerp_cv(a: &ClipVert, b: &ClipVert, t: f32) -> ClipVert {
    ClipVert {
        clip: a.clip.lerp(b.clip, t),
        pos: a.pos.lerp(b.pos, t),
        nrm: a.nrm.lerp(b.nrm, t),
        uv: Vec2::new(a.uv.x + (b.uv.x - a.uv.x) * t, a.uv.y + (b.uv.y - a.uv.y) * t),
    }
}

fn clip_near(poly: &[ClipVert]) -> Vec<ClipVert> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let a = &poly[i];
        let b = &poly[(i + 1) % poly.len()];
        let da = a.clip.z + a.clip.w;
        let db = b.clip.z + b.clip.w;
        if da >= 0.0 {
            out.push(*a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            out.push(lerp_cv(a, b, t));
        }
    }
    out
}

fn raster_tri(gb: &mut GBuffer, v: [ClipVert; 3], mat: u16) {
    let wf = gb.w as f32;
    let hf = gb.full_h as f32;
    let mut sx = [0f32; 3];
    let mut sy = [0f32; 3];
    let mut iw = [0f32; 3];
    for i in 0..3 {
        let c = v[i].clip;
        if c.w <= 1e-6 {
            return;
        }
        let inv = 1.0 / c.w;
        sx[i] = (c.x * inv * 0.5 + 0.5) * wf;
        sy[i] = (1.0 - (c.y * inv * 0.5 + 0.5)) * hf;
        iw[i] = inv;
    }
    let area = (sx[1] - sx[0]) * (sy[2] - sy[0]) - (sx[2] - sx[0]) * (sy[1] - sy[0]);
    if area.abs() < 1e-9 {
        return;
    }
    let minx = sx.iter().cloned().fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
    let maxx = sx.iter().cloned().fold(f32::NEG_INFINITY, f32::max).ceil().min(wf - 1.0);
    let miny = (sy.iter().cloned().fold(f32::INFINITY, f32::min).floor().max(0.0) as usize).max(gb.y0);
    let maxy = sy.iter().cloned().fold(f32::NEG_INFINITY, f32::max).ceil().min(hf - 1.0).min((gb.y0 + gb.h) as f32 - 1.0);
    if maxx < 0.0 || maxy < 0.0 {
        return;
    }
    let (maxx, maxy) = (maxx as usize, maxy as usize);
    if miny > maxy {
        return;
    }
    for y in miny..=maxy {
        for x in minx..=maxx {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 = ((sx[1] - px) * (sy[2] - py) - (sx[2] - px) * (sy[1] - py)) / area;
            let w1 = ((sx[2] - px) * (sy[0] - py) - (sx[0] - px) * (sy[2] - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
                continue;
            }
            let inv_w = w0 * iw[0] + w1 * iw[1] + w2 * iw[2];
            if inv_w <= 0.0 {
                continue;
            }
            let depth = 1.0 / inv_w;
            let i = (y - gb.y0) * gb.w + x;
            if depth >= gb.depth[i] {
                continue;
            }
            let p0 = w0 * iw[0] * depth;
            let p1 = w1 * iw[1] * depth;
            let p2 = w2 * iw[2] * depth;
            gb.depth[i] = depth;
            gb.pos[i] = v[0].pos * p0 + v[1].pos * p1 + v[2].pos * p2;
            gb.normal[i] = (v[0].nrm * p0 + v[1].nrm * p1 + v[2].nrm * p2).normalized();
            gb.uv[i] = Vec2::new(
                v[0].uv.x * p0 + v[1].uv.x * p1 + v[2].uv.x * p2,
                v[0].uv.y * p0 + v[1].uv.y * p1 + v[2].uv.y * p2,
            );
            gb.mat[i] = mat;
        }
    }
}

struct Prepared {
    clip: Vec<Vec4>,
    wpos: Vec<Vec3>,
    wnrm: Vec<Vec3>,
}

fn raster_items(rs: &RenderScene, prep: &[Prepared], gb: &mut GBuffer) {
    for (item, pr) in rs.items.iter().zip(prep) {
        for f in &item.mesh.faces {
            if f.len() < 3 {
                continue;
            }
            for k in 1..f.len() - 1 {
                let ids = [f[0] as usize, f[k] as usize, f[k + 1] as usize];
                let poly: Vec<ClipVert> = ids
                    .iter()
                    .map(|&i| ClipVert {
                        clip: pr.clip[i],
                        pos: pr.wpos[i],
                        nrm: pr.wnrm[i],
                        uv: item.mesh.uvs.get(i).copied().unwrap_or_default(),
                    })
                    .collect();
                if poly.iter().all(|c| c.clip.z + c.clip.w < 0.0) {
                    continue;
                }
                let clipped = if poly.iter().all(|c| c.clip.z + c.clip.w >= 0.0) { poly } else { clip_near(&poly) };
                if clipped.len() < 3 {
                    continue;
                }
                for j in 1..clipped.len() - 1 {
                    raster_tri(gb, [clipped[0], clipped[j], clipped[j + 1]], item.material);
                }
            }
        }
    }
}

/// Rasterize every mesh and particle into a fresh G-buffer. Rows are split across cores.
pub fn rasterize(rs: &RenderScene, view: &Mat4, proj: &Mat4, w: usize, h: usize) -> GBuffer {
    let vp = *proj * *view;
    let prep: Vec<Prepared> = rs
        .items
        .iter()
        .map(|item| {
            let mvp = vp * item.world;
            let nm = item.world;
            Prepared {
                clip: item.mesh.positions.iter().map(|p| mvp * p.extend(1.0)).collect(),
                wpos: item.mesh.positions.iter().map(|p| nm.transform_point(*p)).collect(),
                wnrm: item.mesh.normals.iter().map(|n| nm.transform_dir(*n).normalized()).collect(),
            }
        })
        .collect();
    let tris: usize = rs.items.iter().map(|i| i.mesh.faces.len()).sum();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(8);
    let mut gb = if threads <= 1 || h < 48 || w * h < 40_000 || tris == 0 {
        let mut g = GBuffer::new(w, h);
        raster_items(rs, &prep, &mut g);
        g
    } else {
        let bands = threads * 2;
        let rows = h.div_ceil(bands);
        let ranges: Vec<(usize, usize)> = (0..bands).map(|i| (i * rows, ((i + 1) * rows).min(h))).filter(|(a, b)| a < b).collect();
        let prep_ref = &prep;
        let out: Vec<GBuffer> = std::thread::scope(|sc| {
            let handles: Vec<_> = ranges
                .iter()
                .map(|&(a, b)| {
                    sc.spawn(move || {
                        let mut g = GBuffer::band(w, h, a, b);
                        raster_items(rs, prep_ref, &mut g);
                        g
                    })
                })
                .collect();
            handles.into_iter().map(|j| j.join().unwrap()).collect()
        });
        GBuffer::concat(out)
    };
    splat_particles(&mut gb, rs, view, proj);
    gb
}

pub fn splat_particles(gb: &mut GBuffer, rs: &RenderScene, view: &Mat4, proj: &Mat4) {
    if rs.particles.is_empty() {
        return;
    }
    let vp = *proj * *view;
    let inv_view = view.inverse();
    let wf = gb.w as f32;
    let hf = gb.h as f32;
    for p in &rs.particles {
        // points and streaks are drawn small in the views
        let pr = if p.kind == 0 || p.kind == 4 { p.radius * 0.1 } else { p.radius };
        let c = vp * p.pos.extend(1.0);
        if c.w <= 1e-4 || c.z + c.w < 0.0 {
            continue;
        }
        let inv = 1.0 / c.w;
        let cx = (c.x * inv * 0.5 + 0.5) * wf;
        let cy = (1.0 - (c.y * inv * 0.5 + 0.5)) * hf;
        let r_px = pr * proj.m[1][1] * 0.5 * hf * inv;
        let r_px = r_px.max(0.6);
        let x0 = ((cx - r_px).floor().max(0.0)) as usize;
        let x1 = ((cx + r_px).ceil().min(wf - 1.0)).max(0.0) as usize;
        let y0 = ((cy - r_px).floor().max(0.0)) as usize;
        let y1 = ((cy + r_px).ceil().min(hf - 1.0)).max(0.0) as usize;
        if cx + r_px < 0.0 || cy + r_px < 0.0 || cx - r_px > wf || cy - r_px > hf {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = (x as f32 + 0.5 - cx) / r_px;
                let dy = (y as f32 + 0.5 - cy) / r_px;
                let d2 = dx * dx + dy * dy;
                if d2 > 1.0 {
                    continue;
                }
                let nz = (1.0 - d2).sqrt();
                let depth = c.w - nz * pr;
                if depth <= 1e-4 {
                    continue;
                }
                let i = y * gb.w + x;
                if depth >= gb.depth[i] {
                    continue;
                }
                let nview = Vec3::new(dx, -dy, nz).normalized();
                let nworld = inv_view.transform_dir(nview).normalized();
                gb.depth[i] = depth;
                gb.normal[i] = nworld;
                gb.pos[i] = p.pos + nworld * pr;
                gb.mat[i] = super::PARTICLE_MATERIAL;
            }
        }
    }
}

/// Project a world space segment to screen space with near plane clipping.
/// Returns ((x, y, 1/w), (x, y, 1/w)) in pixel coordinates.
pub fn project_line(vp: &Mat4, a: Vec3, b: Vec3, w: usize, h: usize) -> Option<((f32, f32, f32), (f32, f32, f32))> {
    let mut ca = *vp * a.extend(1.0);
    let mut cb = *vp * b.extend(1.0);
    let da = ca.z + ca.w;
    let db = cb.z + cb.w;
    if da < 0.0 && db < 0.0 {
        return None;
    }
    if da < 0.0 {
        let t = da / (da - db);
        ca = ca.lerp(cb, t);
    } else if db < 0.0 {
        let t = da / (da - db);
        cb = ca.lerp(cb, t);
    }
    if ca.w <= 1e-6 || cb.w <= 1e-6 {
        return None;
    }
    let (wf, hf) = (w as f32, h as f32);
    let f = |c: Vec4| {
        let inv = 1.0 / c.w;
        ((c.x * inv * 0.5 + 0.5) * wf, (1.0 - (c.y * inv * 0.5 + 0.5)) * hf, inv)
    };
    Some((f(ca), f(cb)))
}

/// Step along a projected line, calling `plot(x, y, depth, code)` for each pixel.
/// `yscale` corrects slope classification for non square pixels.
pub fn walk_line<F: FnMut(i32, i32, f32, u8)>(
    p0: (f32, f32, f32),
    p1: (f32, f32, f32),
    w: usize,
    h: usize,
    yscale: f32,
    mut plot: F,
) {
    let dx = p1.0 - p0.0;
    let dy = p1.1 - p0.1;
    if !(dx.is_finite() && dy.is_finite()) || dx.abs() > 1e5 || dy.abs() > 1e5 {
        return;
    }
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as usize;
    let vdy = dy * yscale;
    let code = if vdy.abs() < 0.4 * dx.abs() {
        1
    } else if dx.abs() < 0.4 * vdy.abs() {
        2
    } else if (dx > 0.0) == (vdy > 0.0) {
        4
    } else {
        3
    };
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (p0.0 + dx * t).floor() as i32;
        let y = (p0.1 + dy * t).floor() as i32;
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            continue;
        }
        let inv_w = p0.2 + (p1.2 - p0.2) * t;
        if inv_w <= 0.0 {
            continue;
        }
        plot(x, y, 1.0 / inv_w, code);
    }
}

/// Draw every mesh edge into the wire layer, hidden line removed with a depth bias.
pub fn draw_wireframe(gb: &mut GBuffer, rs: &RenderScene, view: &Mat4, proj: &Mat4, yscale: f32) {
    let vp = *proj * *view;
    let (w, h) = (gb.w, gb.h);
    for item in &rs.items {
        let edges = item.mesh.edges();
        for (a, b) in edges {
            let pa = item.world.transform_point(item.mesh.positions[a as usize]);
            let pb = item.world.transform_point(item.mesh.positions[b as usize]);
            if let Some((s0, s1)) = project_line(&vp, pa, pb, w, h) {
                let depth_ref = &gb.depth;
                let mut hits: Vec<(usize, u8)> = Vec::new();
                walk_line(s0, s1, w, h, yscale, |x, y, d, code| {
                    let i = y as usize * w + x as usize;
                    let bias = 0.02 + d * 0.01;
                    if d <= depth_ref[i] + bias {
                        hits.push((i, code));
                    }
                });
                for (i, code) in hits {
                    gb.wire[i] = code;
                }
            }
        }
    }
}

fn default_lights(eye: Vec3, pos: Vec3) -> Vec<RenderLight> {
    let to_eye = (eye - pos).normalized();
    vec![
        RenderLight::simple(LightKind::Directional, Vec3::ZERO, to_eye, Vec3::ONE, 0.85),
        RenderLight::simple(LightKind::Directional, Vec3::ZERO, Vec3::new(-0.4, 0.8, 0.3).normalized(), Vec3::new(0.6, 0.65, 0.85), 0.35),
    ]
}

fn shade_pixel(gb: &GBuffer, i: usize, rs: &RenderScene, eye: Vec3, bg: Vec3) -> Vec3 {
    if !gb.depth[i].is_finite() {
        return bg;
    }
    let pos = gb.pos[i];
    let mut n = gb.normal[i];
    let v = (eye - pos).normalized();
    if n.dot(v) < 0.0 {
        n = -n;
    }
    let mi = if gb.mat[i] == NO_MAT { 0 } else { gb.mat[i] as usize };
    let m = &rs.materials[mi.min(rs.materials.len() - 1)];
    let fallback;
    let lights: &[RenderLight] = if rs.lights.is_empty() {
        fallback = default_lights(eye, pos);
        &fallback
    } else {
        &rs.lights
    };
    let mut base = m.color;
    if rs.use_textures {
        if let Some(Some(t)) = rs.textures.get(mi) {
            base = t.sample(gb.uv[i].x, gb.uv[i].y);
        }
    }
    let col = super::shade_surface(m, base, n, v, pos, lights, &|_, _, _| Some(Vec3::ONE));
    Vec3::new(
        col.x.clamp(0.0, 1.0).powf(1.0 / 2.2),
        col.y.clamp(0.0, 1.0).powf(1.0 / 2.2),
        col.z.clamp(0.0, 1.0).powf(1.0 / 2.2),
    )
}

/// Deferred Phong shading of the G-buffer. Returns linear-to-display (gamma corrected) colors.
/// Rows are shaded in parallel across the available cores.
pub fn shade(gb: &GBuffer, rs: &RenderScene, eye: Vec3, bg: Vec3) -> Vec<Vec3> {
    let n = gb.w * gb.h;
    let mut out = vec![bg; n];
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16);
    if threads <= 1 || n < 20_000 {
        for (i, o) in out.iter_mut().enumerate() {
            *o = shade_pixel(gb, i, rs, eye, bg);
        }
        return out;
    }
    let chunk = n.div_ceil(threads);
    std::thread::scope(|sc| {
        for (ci, part) in out.chunks_mut(chunk).enumerate() {
            sc.spawn(move || {
                let base = ci * chunk;
                for (k, o) in part.iter_mut().enumerate() {
                    *o = shade_pixel(gb, base + k, rs, eye, bg);
                }
            });
        }
    });
    out
}
