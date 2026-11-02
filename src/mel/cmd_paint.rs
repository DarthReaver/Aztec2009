//! 3D Paint Tool: paint a file texture directly on a model through its UVs.

use super::cmd_model::current_shape;
use super::commands::Out;
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::{MeshData, NodeData};
use crate::dag::NodeId;
use crate::math::{Vec2, Vec3};
use crate::render::texture::{self, Texture};
use crate::session::Session;

/// The texture being painted (sRGB values 0..1) with the file it saves to, plus the brush.
pub struct PaintState {
    pub path: String,
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 3]>,
    /// The texture as assigned, so Erase can bring it back.
    pub original: Vec<[f32; 3]>,
    pub color: [f32; 3],
    /// Brush radius as a fraction of the texture width.
    pub radius: f32,
    pub opacity: f32,
    pub erase: bool,
    pub save_on_stroke: bool,
    pub dirty: bool,
}

impl Default for PaintState {
    fn default() -> PaintState {
        PaintState { path: String::new(), w: 0, h: 0, px: Vec::new(), original: Vec::new(), color: [1.0, 0.2, 0.2], radius: 0.03, opacity: 1.0, erase: false, save_on_stroke: false, dirty: false }
    }
}

impl PaintState {
    pub fn active(&self) -> bool {
        !self.path.is_empty() && !self.px.is_empty()
    }

    /// One round dab centered at (u, v) with a soft edge.
    pub fn dab(&mut self, u: f32, v: f32) {
        if !self.active() {
            return;
        }
        let (w, h) = (self.w as i64, self.h as i64);
        let r = (self.radius * self.w as f32).max(0.75);
        let (cx, cy) = (u * self.w as f32, (1.0 - v) * self.h as f32);
        let (x0, x1) = ((cx - r).floor() as i64, (cx + r).ceil() as i64);
        let (y0, y1) = ((cy - r).floor() as i64, (cy + r).ceil() as i64);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let d = (dx * dx + dy * dy).sqrt() / r;
                if d > 1.0 {
                    continue;
                }
                let a = (1.0 - d * d).clamp(0.0, 1.0).sqrt() * self.opacity;
                let i = (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize;
                let target = if self.erase { self.original[i] } else { self.color };
                for k in 0..3 {
                    self.px[i][k] += (target[k] - self.px[i][k]) * a;
                }
            }
        }
        self.dirty = true;
    }

    /// Dabs along the segment so fast strokes stay connected.
    pub fn stroke(&mut self, a: Vec2, b: Vec2) {
        let step = (self.radius * 0.35).max(1.0 / self.w.max(1) as f32);
        let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
        let n = ((len / step).ceil() as usize).max(1);
        for i in 0..=n {
            let t = i as f32 / n as f32;
            self.dab(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        }
    }

    /// Make the renderer show the painted pixels.
    pub fn publish(&mut self, file: &str) {
        if !self.active() {
            return;
        }
        let rgb: Vec<[f32; 3]> = self.px.iter().map(|c| [texture::srgb_to_linear(c[0]), texture::srgb_to_linear(c[1]), texture::srgb_to_linear(c[2])]).collect();
        texture::store(file, Texture { w: self.w, h: self.h, rgb, alpha: None });
    }

    pub fn save(&mut self) -> Result<(), String> {
        let mut img = image::RgbImage::new(self.w as u32, self.h as u32);
        for (i, p) in img.pixels_mut().enumerate() {
            let c = self.px[i];
            *p = image::Rgb([(c[0].clamp(0.0, 1.0) * 255.0).round() as u8, (c[1].clamp(0.0, 1.0) * 255.0).round() as u8, (c[2].clamp(0.0, 1.0) * 255.0).round() as u8]);
        }
        img.save(&self.path).map_err(|e| format!("Cannot save {}: {}", self.path, e))?;
        self.dirty = false;
        Ok(())
    }
}

fn load_pixels(file: &str, res: usize) -> Option<(usize, usize, Vec<[f32; 3]>)> {
    let img = image::open(file).ok()?.to_rgb8();
    let img = if img.width() as usize != res || img.height() as usize != res { image::imageops::resize(&img, res as u32, res as u32, image::imageops::FilterType::Triangle) } else { img };
    Some((res, res, img.pixels().map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect()))
}

fn md_mut(s: &mut Session, shape: NodeId) -> Result<&mut MeshData, String> {
    match s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
        Some(NodeData::Mesh(md)) if md.nurbs.is_some() => Err("3D painting needs a polygon object: convert the NURBS surface first (nurbsToPoly)".to_string()),
        Some(NodeData::Mesh(md)) => Ok(md),
        _ => Err("Not a polygon object".to_string()),
    }
}

/// `paintAssign [-res N] [-name FILE]`: Assign/Edit Textures. Gives the selected object its own UVs (if it has
/// none), duplicates its color map (or starts from the material color) into `3dpainttextures/` and makes it the
/// texture being painted.
pub fn paint_assign(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["res", "r"], arity: 1 }, FlagSpec { names: &["name", "n"], arity: 1 }])?;
    let res = p.num("res")?.unwrap_or(512.0) as usize;
    if ![64, 128, 256, 512, 1024, 2048].contains(&res) {
        return Err("-res must be 64, 128, 256, 512, 1024 or 2048".to_string());
    }
    let shape = current_shape(s).ok_or("Select a polygon object first")?;
    s.scene.flush_meshes();
    {
        let md = md_mut(s, shape)?;
        md.ensure_custom();
        if md.uv_set.is_none() {
            let set = crate::geo::uv::automatic(&md.mesh, None, None);
            md.uv_set = Some(set);
            md.touch();
        }
    }
    let owner = s.scene.node(shape).and_then(|n| n.parent).map(|t| s.scene.name_of(t)).unwrap_or_else(|| "object".into());
    // the object's own material, so painting does not change other objects
    let mat = match md_mut(s, shape)?.material {
        Some(m) => m,
        None => {
            let m = s.scene.create_material(Some(&format!("{}_paint", owner)));
            let t = s.scene.node(shape).and_then(|n| n.parent).unwrap_or(shape);
            s.scene.assign_material(m, t)?;
            m
        }
    };
    let old = s.scene.color_map(mat).map(|m| s.scene.texture_file(&m));
    let dir = s.scene.project_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(".")).join("3dpainttextures");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {}", dir.display(), e))?;
    let fname = p.string("name").unwrap_or_else(|| format!("{}_color.png", owner));
    let path = dir.join(&fname);
    let path_s = path.to_string_lossy().to_string();
    let (w, h, px) = match old.as_deref().filter(|o| !o.starts_with("builtin:")).and_then(|o| load_pixels(o, res)) {
        Some(x) => x,
        None => {
            // start from the material color (or the built-in pattern, e.g. the checker)
            let (r, g, b) = (s.scene.get_attr(mat, "colorR").unwrap_or(0.7), s.scene.get_attr(mat, "colorG").unwrap_or(0.7), s.scene.get_attr(mat, "colorB").unwrap_or(0.7));
            let mut px = vec![[r, g, b]; res * res];
            if let Some(t) = old.as_deref().and_then(texture::load) {
                for y in 0..res {
                    for x in 0..res {
                        let c = t.sample((x as f32 + 0.5) / res as f32, 1.0 - (y as f32 + 0.5) / res as f32);
                        px[y * res + x] = [c.x.powf(1.0 / 2.2), c.y.powf(1.0 / 2.2), c.z.powf(1.0 / 2.2)];
                    }
                }
            }
            (res, res, px)
        }
    };
    let color = s.paint.color;
    let (radius, opacity, sos) = (s.paint.radius, s.paint.opacity, s.paint.save_on_stroke);
    s.paint = PaintState { path: path_s.clone(), w, h, original: px.clone(), px, color, radius, opacity, erase: false, save_on_stroke: sos, dirty: true };
    s.paint.save()?;
    texture::invalidate(&path_s);
    let stored = s.scene.texture_path_for_storage(&path_s);
    s.scene.set_color_map(mat, Some(&stored))?;
    for a in ["colorR", "colorG", "colorB"] {
        s.scene.set_attr(mat, a, 1.0).ok();
    }
    s.paint.publish(&path_s);
    s.view.show_textures();
    Ok(vec![format!("Painting {} ({}x{})", stored, w, h)])
}

/// `paintOptions [-color R G B] [-radius FRACTION] [-opacity 0..1] [-erase 0|1] [-sos 0|1]`
pub fn paint_options(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(
        rest,
        &[
            FlagSpec { names: &["color", "c"], arity: 3 },
            FlagSpec { names: &["radius", "r"], arity: 1 },
            FlagSpec { names: &["opacity", "o"], arity: 1 },
            FlagSpec { names: &["erase", "e"], arity: 1 },
            FlagSpec { names: &["saveOnStroke", "sos"], arity: 1 },
        ],
    )?;
    if let Some(c) = p.get("color") {
        for k in 0..3 {
            s.paint.color[k] = c[k].as_f32()?.clamp(0.0, 1.0);
        }
    }
    if let Some(r) = p.num("radius")? {
        s.paint.radius = r.clamp(0.002, 0.5);
    }
    if let Some(o) = p.num("opacity")? {
        s.paint.opacity = o.clamp(0.0, 1.0);
    }
    if let Some(e) = p.num("erase")? {
        s.paint.erase = e > 0.5;
    }
    if let Some(e) = p.num("saveOnStroke")? {
        s.paint.save_on_stroke = e > 0.5;
    }
    let b = &s.paint;
    Ok(vec![format!("brush color {:.2} {:.2} {:.2}, radius {}, opacity {}, {}, save on stroke {}", b.color[0], b.color[1], b.color[2], b.radius, b.opacity, if b.erase { "erase" } else { "paint" }, b.save_on_stroke as u8)])
}

/// UV at the point where the ray (world space) first hits the shape's surface.
pub fn pick_uv(s: &mut Session, shape: NodeId, origin: Vec3, dir: Vec3) -> Option<Vec2> {
    let owner = s.scene.node(shape).and_then(|n| n.parent).unwrap_or(shape);
    let inv = s.scene.world_matrix(owner).inverse();
    let (o, d) = (inv.transform_point(origin), inv.transform_dir(dir));
    let md = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md,
        _ => return None,
    };
    let set = md.uv_set.as_ref().filter(|u| u.matches(&md.mesh.faces))?;
    let mut best: Option<(f32, Vec2)> = None;
    for (fi, f) in md.mesh.faces.iter().enumerate() {
        for k in 1..f.len().saturating_sub(1) {
            let (i0, i1, i2) = (0, k, k + 1);
            let (a, b, c) = (md.mesh.positions[f[i0] as usize], md.mesh.positions[f[i1] as usize], md.mesh.positions[f[i2] as usize]);
            // Moller-Trumbore, both sides
            let (e1, e2) = (b - a, c - a);
            let pv = d.cross(e2);
            let det = e1.dot(pv);
            if det.abs() < 1e-12 {
                continue;
            }
            let inv_det = 1.0 / det;
            let tv = o - a;
            let u = tv.dot(pv) * inv_det;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let qv = tv.cross(e1);
            let v = d.dot(qv) * inv_det;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(qv) * inv_det;
            if t > 1e-5 && best.map(|b| t < b.0).unwrap_or(true) {
                let (ua, ub, uc) = (set.uvs[set.fuv[fi][i0] as usize], set.uvs[set.fuv[fi][i1] as usize], set.uvs[set.fuv[fi][i2] as usize]);
                let w = 1.0 - u - v;
                best = Some((t, Vec2::new(ua.x * w + ub.x * u + uc.x * v, ua.y * w + ub.y * u + uc.y * v)));
            }
        }
    }
    best.map(|b| b.1)
}

/// Paint at (or along a stroke through) a surface point; returns the UV that was hit.
pub fn paint_ray(s: &mut Session, shape: NodeId, origin: Vec3, dir: Vec3, from: Option<Vec2>) -> Result<Option<Vec2>, String> {
    if !s.paint.active() {
        return Err("Run paintAssign first (Assign/Edit Textures)".to_string());
    }
    let Some(uv) = pick_uv(s, shape, origin, dir) else { return Ok(None) };
    s.paint.stroke(from.unwrap_or(uv), uv);
    let path = s.paint.path.clone();
    s.paint.publish(&path);
    Ok(Some(uv))
}

/// `paintStroke -uv U V [-to U2 V2]` or `paintStroke -ray OX OY OZ DX DY DZ`: apply the brush.
pub fn paint_stroke(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["uv"], arity: 2 }, FlagSpec { names: &["to"], arity: 2 }, FlagSpec { names: &["ray"], arity: 6 }])?;
    if !s.paint.active() {
        return Err("Run paintAssign first (Assign/Edit Textures)".to_string());
    }
    if let Some(r) = p.get("ray") {
        let v: Vec<f32> = r.iter().map(|t| t.as_f32()).collect::<Result<_, _>>()?;
        let shape = current_shape(s).ok_or("Select a polygon object first")?;
        return match paint_ray(s, shape, Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]), None)? {
            Some(uv) => {
                if s.paint.save_on_stroke {
                    s.paint.save()?;
                }
                Ok(vec![format!("painted at uv {:.3} {:.3}", uv.x, uv.y)])
            }
            None => Ok(vec!["the ray missed the surface".to_string()]),
        };
    }
    let a = p.get("uv").ok_or("Usage: paintStroke -uv U V [-to U V] | -ray OX OY OZ DX DY DZ")?;
    let a = Vec2::new(a[0].as_f32()?, a[1].as_f32()?);
    let b = match p.get("to") {
        Some(t) => Vec2::new(t[0].as_f32()?, t[1].as_f32()?),
        None => a,
    };
    s.paint.stroke(a, b);
    let path = s.paint.path.clone();
    s.paint.publish(&path);
    if s.paint.save_on_stroke {
        s.paint.save()?;
    }
    Ok(vec!["painted".to_string()])
}

/// `paintSave`: Save Textures. `paintReload`: Reload File Textures after editing the file elsewhere.
pub fn paint_save(s: &mut Session, reload: bool) -> Out {
    if !s.paint.active() {
        return Err("Nothing is being painted".to_string());
    }
    if reload {
        let path = s.paint.path.clone();
        let (w, h, px) = load_pixels(&path, s.paint.w).ok_or_else(|| format!("Cannot read {}", path))?;
        s.paint.w = w;
        s.paint.h = h;
        s.paint.original = px.clone();
        s.paint.px = px;
        s.paint.dirty = false;
        texture::invalidate(&path);
        return Ok(vec![format!("Reloaded {}", path)]);
    }
    s.paint.save()?;
    Ok(vec![format!("Saved {}", s.paint.path)])
}
