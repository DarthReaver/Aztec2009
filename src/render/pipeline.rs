//! The renderer front end: Render Settings, render layers and passes, motion blur, light effects,
//! the three renderer styles (software, hardware, vector) and the numbered image files of a batch render.

use super::raytrace::{raytrace_buffers, Buffers, Options, Pass};
use super::{camera_info, CameraInfo, LightKind, RenderScene, F_BLUR};
use crate::dag::nodes::{NodeData, RenderLayerData};
use crate::dag::{NodeId, Scene};
use crate::math::{Mat4, Vec3, Vec4};
use std::collections::HashMap;

pub const PASS_NAMES: &[&str] = &["beauty", "diffuse", "specular", "shadow", "reflection", "color", "depth", "normal", "mask", "motion"];
pub const RENDERERS: &[&str] = &["software", "hardware", "vector"];

#[derive(Clone, Debug)]
pub struct Settings {
    pub w: usize,
    pub h: usize,
    pub renderer: u8,
    pub quality: u8,
    pub start: i32,
    pub end: i32,
    pub by: i32,
    pub padding: usize,
    pub animation: u8,
    pub raytracing: bool,
    pub motion_blur: bool,
    pub blur_3d: bool,
    pub blur_length: f32,
    pub shutter: f32,
    pub blur_samples: usize,
    pub alpha: bool,
    pub depth: bool,
    pub geometry_mask: bool,
    pub exposures: usize,
    pub vector_fill: u8,
    pub vector_edges: bool,
    pub edge_weight: f32,
    pub vector_shadows: bool,
    pub vector_highlights: bool,
    pub vector_reflections: bool,
    pub bg_top: Vec3,
    pub bg_bottom: Vec3,
    pub prefix: String,
    pub ext: String,
    pub camera: String,
}

impl Settings {
    /// The Render Settings of the scene with the overrides of `layer` applied.
    pub fn from_scene(scene: &mut Scene, layer: Option<&RenderLayerData>) -> Settings {
        let rg = scene.ids_where(|d| matches!(d, NodeData::RenderGlobals(_))).first().copied();
        let get = |scene: &mut Scene, k: &str, d: f32| -> f32 {
            if let Some((_, v)) = layer.and_then(|l| l.overrides.iter().find(|(n, _)| n == k)) {
                return *v;
            }
            rg.and_then(|id| scene.get_attr(id, k).ok()).unwrap_or(d)
        };
        let (mut prefix, mut ext, mut camera) = match rg.and_then(|id| scene.node(id)).map(|n| &n.data) {
            Some(NodeData::RenderGlobals(d)) => (d.prefix.clone(), d.ext.clone(), d.camera.clone()),
            _ => (String::new(), "png".to_string(), String::new()),
        };
        if let Some(l) = layer {
            for (k, v) in &l.text_overrides {
                match k.as_str() {
                    "prefix" => prefix = v.clone(),
                    "ext" => ext = v.clone(),
                    "camera" => camera = v.clone(),
                    _ => {}
                }
            }
        }
        let col = |scene: &mut Scene, b: &str, d: [f32; 3]| Vec3::new(get(scene, &format!("{}R", b), d[0]), get(scene, &format!("{}G", b), d[1]), get(scene, &format!("{}B", b), d[2]));
        let shutter = get(scene, "blurLength", 1.0) * get(scene, "shutterAngle", 144.0) / 360.0;
        Settings {
            w: get(scene, "width", 640.0).clamp(16.0, 4096.0) as usize,
            h: get(scene, "height", 480.0).clamp(16.0, 4096.0) as usize,
            renderer: get(scene, "renderer", 0.0).round().clamp(0.0, 2.0) as u8,
            quality: get(scene, "quality", 1.0).round().clamp(0.0, 3.0) as u8,
            start: get(scene, "startFrame", 1.0).round() as i32,
            end: get(scene, "endFrame", 10.0).round() as i32,
            by: (get(scene, "byFrame", 1.0).round() as i32).max(1),
            padding: get(scene, "framePadding", 4.0).clamp(1.0, 8.0) as usize,
            animation: get(scene, "animation", 1.0).round().clamp(0.0, 2.0) as u8,
            raytracing: get(scene, "raytracing", 1.0) > 0.5,
            motion_blur: get(scene, "motionBlur", 0.0) > 0.5,
            blur_3d: get(scene, "motionBlurType", 0.0) > 0.5,
            blur_length: get(scene, "blurLength", 1.0),
            shutter,
            blur_samples: get(scene, "blurSamples", 5.0).clamp(2.0, 16.0) as usize,
            alpha: get(scene, "alphaChannel", 1.0) > 0.5,
            depth: get(scene, "depthChannel", 0.0) > 0.5,
            geometry_mask: get(scene, "geometryMask", 0.0) > 0.5,
            exposures: get(scene, "hardwareExposures", 7.0).clamp(1.0, 32.0) as usize,
            vector_fill: get(scene, "vectorFill", 1.0).round().clamp(0.0, 2.0) as u8,
            vector_edges: get(scene, "vectorEdges", 1.0) > 0.5,
            edge_weight: get(scene, "vectorEdgeWeight", 1.5).max(0.0),
            vector_shadows: get(scene, "vectorShadows", 1.0) > 0.5,
            vector_highlights: get(scene, "vectorHighlights", 1.0) > 0.5,
            vector_reflections: get(scene, "vectorReflections", 1.0) > 0.5,
            bg_top: col(scene, "bgTop", [0.36, 0.40, 0.46]),
            bg_bottom: col(scene, "bgBottom", [0.14, 0.15, 0.17]),
            prefix,
            ext,
            camera,
        }
    }
}

/// A finished render: gamma corrected colors, coverage and distance per pixel.
#[derive(Clone)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<Vec3>,
    pub alpha: Vec<f32>,
    pub depth: Vec<f32>,
}

impl Image {
    pub fn blank(w: usize, h: usize) -> Image {
        Image { w, h, rgb: vec![Vec3::ZERO; w * h], alpha: vec![0.0; w * h], depth: vec![f32::INFINITY; w * h] }
    }

    /// 8 bit RGBA; with `alpha` off the picture is opaque and the background color shows.
    pub fn to_rgba(&self, alpha: bool) -> Vec<u8> {
        let mut out = vec![255u8; self.w * self.h * 4];
        for i in 0..self.w * self.h {
            let c = self.rgb[i];
            out[i * 4] = (c.x.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i * 4 + 1] = (c.y.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i * 4 + 2] = (c.z.clamp(0.0, 1.0) * 255.0).round() as u8;
            if alpha {
                out[i * 4 + 3] = (self.alpha[i].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        out
    }

    /// Copy the pixels of a rectangle (x0, y0, x1, y1) from `src`.
    pub fn paste(&mut self, src: &Image, r: (usize, usize, usize, usize)) {
        for y in r.1.min(self.h)..r.3.min(self.h) {
            for x in r.0.min(self.w)..r.2.min(self.w) {
                let i = y * self.w + x;
                self.rgb[i] = src.rgb[i];
                self.alpha[i] = src.alpha[i];
                self.depth[i] = src.depth[i];
            }
        }
    }

    pub fn save(&self, path: &str, alpha: bool) -> Result<(), String> {
        if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create '{}': {}", dir.display(), e))?;
        }
        let rgba = self.to_rgba(alpha);
        let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
        if matches!(ext.as_str(), "jpg" | "jpeg") {
            let img = image::RgbImage::from_fn(self.w as u32, self.h as u32, |x, y| {
                let i = (y as usize * self.w + x as usize) * 4;
                image::Rgb([rgba[i], rgba[i + 1], rgba[i + 2]])
            });
            return img.save(path).map_err(|e| format!("Cannot write {}: {}", path, e));
        }
        let img = image::RgbaImage::from_fn(self.w as u32, self.h as u32, |x, y| {
            let i = (y as usize * self.w + x as usize) * 4;
            image::Rgba([rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]])
        });
        img.save(path).map_err(|e| format!("Cannot write {}: {}", path, e))
    }
}

/// Is `id` or one of its ancestors in the list?
fn under(scene: &Scene, id: NodeId, list: &[NodeId]) -> bool {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if list.contains(&c) {
            return true;
        }
        cur = scene.node(c).and_then(|n| n.parent);
    }
    false
}

/// Keep only what a render layer contains. A layer without lights is lit by the default lights.
pub fn filter_layer(scene: &Scene, rs: &mut RenderScene, layer: &RenderLayerData) {
    rs.items.retain(|it| under(scene, it.node, &layer.members));
    rs.lights.retain(|l| l.node.map(|n| under(scene, n, &layer.members)).unwrap_or(false));
}

/// The camera to render: the named one, else the first renderable one, else persp.
pub fn render_cameras(scene: &mut Scene, st: &Settings) -> Vec<String> {
    if !st.camera.is_empty() {
        return vec![st.camera.clone()];
    }
    let cams = scene.ids_where(|d| matches!(d, NodeData::Camera | NodeData::StereoCamera));
    let mut out = Vec::new();
    for c in cams {
        if scene.node(c).and_then(|n| n.plug("renderable")).map(|p| p.value > 0.5).unwrap_or(false) {
            let t = scene.node(c).and_then(|n| n.parent).unwrap_or(c);
            out.push(scene.name_of(t));
        }
    }
    out.sort();
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn cam_by_name(scene: &mut Scene, name: &str) -> Result<CameraInfo, String> {
    camera_info(scene, if name.is_empty() { None } else { Some(name) })
}

fn options_for(st: &Settings, pass: Pass) -> Options {
    let grid = [1, 2, 3, 4][st.quality as usize];
    Options { grid, bg_top: st.bg_top, bg_bottom: st.bg_bottom, pass, reflections: st.raytracing && st.renderer == 0 || (st.renderer == 2 && st.vector_reflections), region: None }
}

/// Snapshot the scene at the current time as the renderer sees it.
fn snapshot(scene: &mut Scene, st: &Settings, layer: Option<&RenderLayerData>) -> RenderScene {
    scene.final_render = true;
    let mut rs = super::build_scene(scene);
    scene.final_render = false;
    rs.use_textures = true;
    if let Some(l) = layer {
        filter_layer(scene, &mut rs, l);
    }
    if st.renderer == 1 {
        // the hardware renderer does not trace rays: soft shadows are cheaper depth map looks
        for l in rs.lights.iter_mut() {
            l.samples = l.samples.min(4);
        }
    }
    if st.renderer == 2 && !st.vector_shadows {
        for l in rs.lights.iter_mut() {
            l.shadows = false;
        }
    }
    rs
}

fn project(view: &Mat4, proj: &Mat4, p: Vec3, w: usize, h: usize) -> Option<(f32, f32, f32)> {
    let c = *proj * (*view * Vec4::new(p.x, p.y, p.z, 1.0));
    if c.w <= 1e-6 {
        return None;
    }
    let (nx, ny) = (c.x / c.w, c.y / c.w);
    Some(((nx * 0.5 + 0.5) * w as f32, (1.0 - (ny * 0.5 + 0.5)) * h as f32, c.w))
}

fn to_image(b: Buffers, depth_infinite: bool) -> Image {
    let _ = depth_infinite;
    Image { w: b.w, h: b.h, rgb: b.rgb, alpha: b.alpha, depth: b.depth }
}

/// Render the current frame of one pass through one camera. `region` renders only a rectangle.
pub fn render_pass(scene: &mut Scene, st: &Settings, layer: Option<&RenderLayerData>, camera: &str, pass: &str, region: Option<(usize, usize, usize, usize)>) -> Result<Image, String> {
    let (w, h) = (st.w, st.h);
    let t0 = scene.timeline.current;
    let cam = cam_by_name(scene, camera)?;
    let aspect = w as f32 / h as f32;
    let (view, proj) = (cam.view(), cam.projection(aspect));
    let rs = snapshot(scene, st, layer);
    let mut opt = options_for(st, Pass::Beauty);
    opt.region = region;
    let simple = |p: Pass, grid: usize| -> Options {
        let mut o = options_for(st, p);
        o.grid = grid;
        o.region = region;
        o
    };
    let mut img = match pass {
        "beauty" | "diffuse" | "specular" | "shadow" | "reflection" | "color" => {
            let p = match pass {
                "diffuse" => Pass::Diffuse,
                "specular" => Pass::Specular,
                "shadow" => Pass::Shadow,
                "reflection" => Pass::Reflection,
                "color" => Pass::Color,
                _ => Pass::Beauty,
            };
            if p == Pass::Beauty && st.motion_blur && st.blur_3d {
                render_3d_blur(scene, st, layer, camera, &rs, region)?
            } else {
                let mut o = simple(p, [1, 2, 3, 4][st.quality as usize]);
                if p == Pass::Beauty && st.renderer == 2 && !st.vector_highlights {
                    o.pass = Pass::Diffuse;
                }
                let b = raytrace_buffers(&rs, &view, &proj, w, h, &o, None);
                let mut img = to_image(b, false);
                if p == Pass::Beauty {
                    let b2 = if st.renderer == 2 || (st.motion_blur && !st.blur_3d) || has_fx(scene) { Some(raytrace_buffers(&rs, &view, &proj, w, h, &simple(Pass::Color, 1), None)) } else { None };
                    if let Some(b2) = &b2 {
                        if st.renderer == 2 {
                            vector_style(&mut img, b2, st);
                        }
                        if st.motion_blur && !st.blur_3d {
                            blur_2d(scene, st, layer, camera, &rs, b2, &mut img, region)?;
                        }
                    }
                    if st.renderer == 1 {
                        draw_hardware_particles(&mut img, &rs, st, &view, &proj, &cam);
                    }
                    light_effects(scene, &mut img, &cam, &view, &proj, layer, region);
                }
                img
            }
        }
        "depth" | "normal" | "mask" | "motion" => {
            let b = raytrace_buffers(&rs, &view, &proj, w, h, &simple(Pass::Color, 1), None);
            let mut img = Image::blank(w, h);
            img.alpha = b.alpha.clone();
            img.depth = b.depth.clone();
            match pass {
                "depth" => {
                    // near is white, far is black, the range fitted to the visible surfaces
                    let far = b.depth.iter().cloned().filter(|d| d.is_finite()).fold(0.0f32, f32::max).max(1e-3);
                    for i in 0..w * h {
                        let v = if b.depth[i].is_finite() { 1.0 - b.depth[i] / far } else { 0.0 };
                        img.rgb[i] = Vec3::splat(v);
                    }
                }
                "normal" => {
                    for i in 0..w * h {
                        if b.depth[i].is_finite() {
                            img.rgb[i] = b.normal[i] * 0.5 + Vec3::splat(0.5);
                        }
                    }
                }
                "mask" => {
                    for i in 0..w * h {
                        img.rgb[i] = Vec3::splat(b.alpha[i]);
                    }
                }
                _ => {
                    // motion vectors: screen space movement over one frame, red = x, green = y, 0.5 = still
                    let vel = velocities(scene, st, layer, camera, &rs, &b)?;
                    for i in 0..w * h {
                        let v = vel[i];
                        img.rgb[i] = Vec3::new((0.5 + v.0 / (w as f32 * 0.1)).clamp(0.0, 1.0), (0.5 + v.1 / (h as f32 * 0.1)).clamp(0.0, 1.0), 0.5);
                    }
                }
            }
            img
        }
        other => return Err(format!("Unknown render pass '{}': {}", other, PASS_NAMES.join(", "))),
    };
    scene.timeline.current = t0;
    if !st.alpha && pass == "beauty" {
        img.alpha.iter_mut().for_each(|a| *a = 1.0);
    }
    Ok(img)
}

fn has_fx(scene: &Scene) -> bool {
    scene.ids_where(|d| matches!(d, NodeData::Light)).iter().any(|id| {
        let g = |k: &str| scene.node(*id).and_then(|n| n.plug(k)).map(|p| p.value).unwrap_or(0.0);
        g("lightGlow") > 0.5 || g("lensFlare") > 0.5 || g("fogType") > 0.5
    })
}

// ---------------------------------------------------------------------------------------------- motion blur

/// Screen space movement in pixels of every pixel's surface over the shutter, from the objects' positions one frame earlier.
fn velocities(scene: &mut Scene, st: &Settings, layer: Option<&RenderLayerData>, camera: &str, rs: &RenderScene, b: &Buffers) -> Result<Vec<(f32, f32)>, String> {
    let t = scene.timeline.current;
    let (w, h) = (st.w, st.h);
    let aspect = w as f32 / h as f32;
    let cam_now = cam_by_name(scene, camera)?;
    let (view, proj) = (cam_now.view(), cam_now.projection(aspect));
    scene.timeline.current = t - 1.0;
    scene.evaluate_frame();
    let prev = snapshot(scene, st, layer);
    let cam_prev = cam_by_name(scene, camera)?;
    scene.timeline.current = t;
    scene.evaluate_frame();
    let (vp, pp) = (cam_prev.view(), cam_prev.projection(aspect));
    let mut prev_world: HashMap<(NodeId, u16), Mat4> = HashMap::new();
    for it in &prev.items {
        prev_world.entry((it.node, it.material)).or_insert(it.world);
    }
    let mut out = vec![(0.0, 0.0); w * h];
    for i in 0..w * h {
        let ii = b.item[i];
        if ii == u32::MAX {
            continue;
        }
        let it = &rs.items[ii as usize];
        let Some(pw) = prev_world.get(&(it.node, it.material)) else { continue };
        let p_prev = (*pw * it.world.inverse()).transform_point(b.pos[i]);
        if let (Some(a), Some(c)) = (project(&view, &proj, b.pos[i], w, h), project(&vp, &pp, p_prev, w, h)) {
            out[i] = (a.0 - c.0, a.1 - c.1);
        }
    }
    Ok(out)
}

/// 2D motion blur: smear the finished picture along each pixel's movement during the shutter.
#[allow(clippy::too_many_arguments)]
fn blur_2d(scene: &mut Scene, st: &Settings, layer: Option<&RenderLayerData>, camera: &str, rs: &RenderScene, b: &Buffers, img: &mut Image, region: Option<(usize, usize, usize, usize)>) -> Result<(), String> {
    let vel = velocities(scene, st, layer, camera, rs, b)?;
    let (w, h) = (img.w, img.h);
    let src = img.clone();
    let (x0, y0, x1, y1) = region.unwrap_or((0, 0, w, h));
    // keep the shutter symmetric around the frame
    let amount = st.shutter.max(0.0);
    for y in y0..y1.min(h) {
        for x in x0..x1.min(w) {
            let v = vel[y * w + x];
            if v.0.abs() + v.1.abs() < 0.5 {
                continue;
            }
            let n = (((v.0.abs() + v.1.abs()) * amount).ceil() as usize).clamp(2, 24);
            let (mut acc, mut al) = (Vec3::ZERO, 0.0);
            for k in 0..n {
                let s = (k as f32 / (n - 1) as f32 - 0.5) * amount;
                let sx = (x as f32 + 0.5 + v.0 * s).clamp(0.0, w as f32 - 1.0) as usize;
                let sy = (y as f32 + 0.5 + v.1 * s).clamp(0.0, h as f32 - 1.0) as usize;
                acc += src.rgb[sy * w + sx];
                al += src.alpha[sy * w + sx];
            }
            img.rgb[y * w + x] = acc * (1.0 / n as f32);
            img.alpha[y * w + x] = al / n as f32;
        }
    }
    Ok(())
}

/// 3D motion blur: render several moments of the shutter and average them. Objects with Motion Blur off stay where they are at the frame.
fn render_3d_blur(scene: &mut Scene, st: &Settings, layer: Option<&RenderLayerData>, camera: &str, center: &RenderScene, region: Option<(usize, usize, usize, usize)>) -> Result<Image, String> {
    let t = scene.timeline.current;
    let (w, h) = (st.w, st.h);
    let n = st.blur_samples.max(2);
    let mut acc = Image::blank(w, h);
    for k in 0..n {
        let ti = t + (k as f32 / (n - 1) as f32 - 0.5) * st.shutter;
        scene.timeline.current = ti;
        scene.evaluate_frame();
        let cam = cam_by_name(scene, camera)?;
        let mut rs = snapshot(scene, st, layer);
        rs.items.retain(|it| it.flags & F_BLUR != 0);
        rs.items.extend(center.items.iter().filter(|it| it.flags & F_BLUR == 0).cloned());
        let mut o = options_for(st, Pass::Beauty);
        o.region = region;
        let aspect = w as f32 / h as f32;
        let b = raytrace_buffers(&rs, &cam.view(), &cam.projection(aspect), w, h, &o, None);
        for i in 0..w * h {
            acc.rgb[i] += b.rgb[i] * (1.0 / n as f32);
            acc.alpha[i] += b.alpha[i] / n as f32;
        }
        if k == n / 2 {
            acc.depth = b.depth;
        }
    }
    scene.timeline.current = t;
    scene.evaluate_frame();
    Ok(acc)
}

// ---------------------------------------------------------------------------------------------- hardware particles

/// Points, sprites and streaks: only the hardware renderer draws them. Geometry hides what is behind it; with the geometry
/// mask on the geometry itself becomes an empty (transparent) mask so the particles can be composited over a software render.
fn draw_hardware_particles(img: &mut Image, rs: &RenderScene, st: &Settings, view: &Mat4, proj: &Mat4, cam: &CameraInfo) {
    let (w, h) = (img.w, img.h);
    let eye = cam.eye();
    let mut list: Vec<&super::RenderParticle> = rs.particles.iter().filter(|p| matches!(p.kind, 0 | 3 | 4)).collect();
    if st.geometry_mask {
        for i in 0..w * h {
            if img.depth[i].is_finite() && img.alpha[i] > 0.0 {
                img.rgb[i] = Vec3::ZERO;
                img.alpha[i] = 0.0;
            }
        }
    }
    if list.is_empty() {
        return;
    }
    list.sort_by(|a, b| (b.pos - eye).length_sq().partial_cmp(&(a.pos - eye).length_sq()).unwrap());
    let tan_half = 1.0 / proj.m[1][1].max(1e-6);
    // motion blur by several exposures along the movement during the shutter
    let n = if st.motion_blur { st.exposures.max(1) } else { 1 };
    for p in list {
        for k in 0..n {
            let off = if n > 1 { (k as f32 / (n - 1) as f32 - 0.5) * st.shutter / 24.0 } else { 0.0 };
            let pos = p.pos + p.vel * off;
            let dist = (pos - eye).length();
            let Some((cx, cy, _)) = project(view, proj, pos, w, h) else { continue };
            let radius_px = (p.radius / (dist * tan_half).max(1e-4) * h as f32 * 0.5).max(0.7);
            let radius_px = if p.kind == 0 { radius_px.min(2.0) } else { radius_px };
            let weight = p.opacity / n as f32;
            let (x0, x1) = (((cx - radius_px).floor() as i32).max(0), ((cx + radius_px).ceil() as i32).min(w as i32 - 1));
            let (y0, y1) = (((cy - radius_px).floor() as i32).max(0), ((cy + radius_px).ceil() as i32).min(h as i32 - 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = (((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt()) / radius_px;
                    if d >= 1.0 {
                        continue;
                    }
                    let i = y as usize * w + x as usize;
                    if img.depth[i].is_finite() && img.depth[i] < dist {
                        continue;
                    }
                    // a soft puff for sprites, a hard dot for points
                    let cover = (if p.kind == 3 { (1.0 - d * d).powi(2) } else { 1.0 }) * weight;
                    let c = if p.color.x < 0.0 { Vec3::splat(0.8) } else { p.color };
                    img.rgb[i] = img.rgb[i] * (1.0 - cover) + c * cover;
                    img.alpha[i] = img.alpha[i] + (1.0 - img.alpha[i]) * cover;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------- light effects

struct Fx {
    pos: Vec3,
    color: Vec3,
    intensity: f32,
    glow: Option<(u8, f32, f32)>,
    halo: (u8, f32),
    flare: Option<f32>,
    fog: Option<(u8, f32, f32)>,
    kind: LightKind,
    radius_scale: f32,
}

fn collect_fx(scene: &mut Scene, layer: Option<&RenderLayerData>) -> Vec<Fx> {
    let mut out = Vec::new();
    for id in scene.ids_where(|d| matches!(d, NodeData::Light)) {
        if !scene.is_visible(id) || layer.map(|l| !under(scene, id, &l.members)).unwrap_or(false) {
            continue;
        }
        let world = scene.world_matrix(id);
        let g = |s: &mut Scene, k: &str| s.get_attr(id, k).unwrap_or(0.0);
        let kind = match g(scene, "lightType").round() as i32 {
            1 => LightKind::Directional,
            2 => LightKind::Spot,
            3 => LightKind::Ambient,
            4 => LightKind::Area,
            _ => LightKind::Point,
        };
        let glow = (g(scene, "lightGlow") > 0.5).then(|| (g(scene, "glowType").round() as u8, g(scene, "glowIntensity"), g(scene, "glowSpread")));
        let flare = (g(scene, "lensFlare") > 0.5).then(|| g(scene, "flareIntensity"));
        let fog = (g(scene, "fogType") > 0.5 && matches!(kind, LightKind::Point | LightKind::Spot | LightKind::Area)).then(|| (g(scene, "fogType").round() as u8, g(scene, "fogRadius"), g(scene, "fogIntensity")));
        if glow.is_none() && flare.is_none() && fog.is_none() {
            continue;
        }
        out.push(Fx {
            pos: world.translation_part(),
            color: Vec3::new(g(scene, "colorR"), g(scene, "colorG"), g(scene, "colorB")),
            intensity: g(scene, "intensity"),
            glow,
            halo: (g(scene, "haloType").round() as u8, g(scene, "haloIntensity")),
            flare,
            fog,
            kind,
            radius_scale: world.transform_dir(Vec3::new(1.0, 0.0, 0.0)).length().max(1e-3),
        });
    }
    out
}

fn add_disc(img: &mut Image, cx: f32, cy: f32, r: f32, col: Vec3, k: f32, expo: bool, region: (usize, usize, usize, usize)) {
    let (w, h) = (img.w as i32, img.h as i32);
    let (x0, x1) = (((cx - r).floor() as i32).max(region.0 as i32), ((cx + r).ceil() as i32).min(w.min(region.2 as i32) - 1));
    let (y0, y1) = (((cy - r).floor() as i32).max(region.1 as i32), ((cy + r).ceil() as i32).min(h.min(region.3 as i32) - 1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = (((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt()) / r;
            if d >= 1.0 {
                continue;
            }
            let f = if expo { (-3.0 * d).exp() * (1.0 - d) } else { 1.0 - d };
            let i = (y * w + x) as usize;
            img.rgb[i] += col * (k * f);
            img.alpha[i] = img.alpha[i].max((k * f).clamp(0.0, 1.0));
        }
    }
}

fn light_effects(scene: &mut Scene, img: &mut Image, cam: &CameraInfo, view: &Mat4, proj: &Mat4, layer: Option<&RenderLayerData>, region: Option<(usize, usize, usize, usize)>) {
    let fx = collect_fx(scene, layer);
    if fx.is_empty() {
        return;
    }
    let (w, h) = (img.w, img.h);
    let region = region.unwrap_or((0, 0, w, h));
    let m = w.min(h) as f32;
    let eye = cam.eye();
    let inv = (*proj * *view).inverse();
    for f in &fx {
        // fog: integrate a glow along every view ray through the light's sphere (a cone for spots)
        if let Some((kind, radius, intensity)) = f.fog {
            let rad = (radius * f.radius_scale).max(1e-3);
            for y in region.1..region.3.min(h) {
                for x in region.0..region.2.min(w) {
                    let nd = |z: f32| {
                        let v = inv * Vec4::new((x as f32 + 0.5) / w as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / h as f32 * 2.0, z, 1.0);
                        v.xyz() / v.w
                    };
                    let (o, far) = (nd(-1.0), nd(1.0));
                    let d = (far - o).normalized();
                    let tc = (f.pos - o).dot(d);
                    let closest = (o + d * tc - f.pos).length();
                    if closest >= rad {
                        continue;
                    }
                    let half = (rad * rad - closest * closest).sqrt();
                    let i = y * w + x;
                    let surf = img.depth[i];
                    let (t0, t1) = ((tc - half).max(0.0), (tc + half).min(if surf.is_finite() { surf } else { f32::INFINITY }));
                    if t1 <= t0 {
                        continue;
                    }
                    let steps = 14;
                    let dt = (t1 - t0) / steps as f32;
                    let mut dens = 0.0;
                    for s in 0..steps {
                        let p = o + d * (t0 + (s as f32 + 0.5) * dt);
                        let r = (p - f.pos).length() / rad;
                        let fall = if kind == 2 { 1.0 - r } else { (1.0 - r) * (1.0 - r) };
                        dens += fall.max(0.0) * dt;
                    }
                    let amount = (dens / rad * intensity * 0.8).clamp(0.0, 1.0);
                    img.rgb[i] += f.color * (amount * f.intensity.clamp(0.0, 2.0));
                    img.alpha[i] = img.alpha[i].max(amount);
                }
            }
        }
        if f.glow.is_none() && f.flare.is_none() {
            continue;
        }
        let Some((cx, cy, _)) = project(view, proj, f.pos, w, h) else { continue };
        if cx < 0.0 || cy < 0.0 || cx >= w as f32 || cy >= h as f32 {
            continue;
        }
        // hidden behind a surface: no glow
        let dist = (f.pos - eye).length();
        let px = (cy as usize).min(h - 1) * w + (cx as usize).min(w - 1);
        if img.depth[px].is_finite() && img.depth[px] < dist {
            continue;
        }
        let k = f.intensity.clamp(0.0, 2.0);
        if let Some((gt, gi, spread)) = f.glow {
            if gt > 0 {
                add_disc(img, cx, cy, 0.12 * m * spread.max(0.05), f.color, gi * k, gt == 2, region);
                if f.halo.0 > 0 && f.halo.0 != 3 {
                    add_disc(img, cx, cy, 0.3 * m * spread.max(0.05), f.color, f.halo.1 * 0.25 * k, f.halo.0 == 2, region);
                }
            }
        }
        if let Some(fi) = f.flare {
            // ghosts along the line through the middle of the picture and a ring around the light
            let (mx, my) = (w as f32 * 0.5, h as f32 * 0.5);
            for (t, size, tint) in [(-0.45f32, 0.05f32, Vec3::new(1.0, 0.6, 0.3)), (0.35, 0.035, Vec3::new(0.4, 1.0, 0.5)), (0.7, 0.07, Vec3::new(0.4, 0.5, 1.0)), (1.4, 0.04, Vec3::new(1.0, 0.4, 0.8)), (1.9, 0.09, Vec3::new(1.0, 0.9, 0.5))] {
                let (gx, gy) = (cx + (mx - cx) * t * 2.0, cy + (my - cy) * t * 2.0);
                add_disc(img, gx, gy, size * m, f.color.mul_elem(tint), 0.35 * fi * k, false, region);
            }
            let ring = 0.18 * m;
            for a in 0..360 {
                let ang = (a as f32).to_radians();
                let (rx, ry) = (cx + ring * ang.cos(), cy + ring * ang.sin());
                add_disc(img, rx, ry, 0.012 * m, f.color, 0.25 * fi * k, false, region);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------- maya vector style

/// Flat fills (single, full or two color) and outlines at changes of object, surface direction or distance.
fn vector_style(img: &mut Image, ids: &Buffers, st: &Settings) {
    let (w, h) = (img.w, img.h);
    for i in 0..w * h {
        if !ids.depth[i].is_finite() {
            continue;
        }
        let c = img.rgb[i];
        let lum = (c.x * 0.3 + c.y * 0.59 + c.z * 0.11).max(1e-4);
        let base = ids.rgb[i];
        img.rgb[i] = match st.vector_fill {
            0 => base,
            2 => {
                if lum > 0.45 * ids.rgb[i].length().max(0.2) {
                    base
                } else {
                    base * 0.55
                }
            }
            _ => {
                // full color: tone steps of a quarter
                let q = (lum * 4.0 + 0.5).floor() / 4.0;
                c * (q.max(0.05) / lum)
            }
        };
    }
    if st.vector_edges {
        let r = (st.edge_weight * 0.7).round().max(0.0) as i32;
        let mut edge = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                for (dx, dy) in [(1usize, 0usize), (0, 1)] {
                    if x + dx >= w || y + dy >= h {
                        continue;
                    }
                    let j = (y + dy) * w + x + dx;
                    let differs = ids.item[i] != ids.item[j] || ids.depth[i].is_finite() != ids.depth[j].is_finite() || ids.normal[i].dot(ids.normal[j]) < 0.8 || (ids.depth[i].is_finite() && ids.depth[j].is_finite() && (ids.depth[i] - ids.depth[j]).abs() > 0.05 * ids.depth[i].max(1.0));
                    if differs {
                        edge[i] = true;
                    }
                }
            }
        }
        let src = edge.clone();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if !src[(y as usize) * w + x as usize] {
                    continue;
                }
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (xx, yy) = (x + dx, y + dy);
                        if xx >= 0 && yy >= 0 && xx < w as i32 && yy < h as i32 && dx * dx + dy * dy <= r * r + 1 {
                            edge[yy as usize * w + xx as usize] = true;
                        }
                    }
                }
            }
        }
        for i in 0..w * h {
            if edge[i] {
                img.rgb[i] = Vec3::ZERO;
                img.alpha[i] = 1.0;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------- files

/// Expand the tokens of a file name prefix.
pub fn expand_prefix(prefix: &str, scene_name: &str, layer: &str, camera: &str, pass: &str) -> String {
    prefix.replace("<Scene>", scene_name).replace("<RenderLayer>", layer).replace("<Camera>", camera).replace("<RenderPass>", pass).replace("<Layer>", layer)
}

/// Path of one image of a render: `<prefix>.<frame>.<ext>` inside the project's images folder.
pub fn image_path(scene: &Scene, st: &Settings, layer: Option<&str>, camera: &str, pass: &str, frame: i32) -> String {
    let scene_name = scene.file_path.as_ref().and_then(|p| std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().to_string())).unwrap_or_else(|| "untitled".to_string());
    let mut prefix = if st.prefix.trim().is_empty() { "<Scene>".to_string() } else { st.prefix.clone() };
    let lname = layer.unwrap_or("masterLayer");
    if layer.is_some() && !prefix.contains("<RenderLayer>") {
        prefix = format!("<RenderLayer>/{}", prefix);
    }
    if pass != "beauty" && !prefix.contains("<RenderPass>") {
        prefix = format!("{}_<RenderPass>", prefix);
    }
    let name = expand_prefix(&prefix, &scene_name, lname, camera, pass);
    let file = match st.animation {
        0 => format!("{}.{}", name, st.ext),
        2 => format!("{}.{}.{:0pad$}", name, st.ext, frame, pad = st.padding),
        _ => format!("{}.{:0pad$}.{}", name, frame, st.ext, pad = st.padding),
    };
    let p = std::path::Path::new(&file);
    if p.is_absolute() {
        return file;
    }
    match &scene.project_dir {
        Some(d) => d.join("images").join(file).to_string_lossy().to_string(),
        None => std::path::Path::new("images").join(file).to_string_lossy().to_string(),
    }
}

/// The layers a batch render covers: the renderable layers, or the master layer when there are none.
pub fn batch_layers(scene: &Scene) -> Vec<Option<RenderLayerData>> {
    let mut v: Vec<Option<RenderLayerData>> = scene.render_layers.iter().filter(|l| l.renderable).cloned().map(Some).collect();
    if v.is_empty() {
        v.push(None);
    }
    v
}

/// Render the frames start..end by `by` to numbered files. Returns the files written.
#[allow(clippy::too_many_arguments)]
pub fn batch_render(scene: &mut Scene, start: Option<i32>, end: Option<i32>, only_layer: Option<&str>, camera: Option<&str>, progress: &mut dyn FnMut(&str)) -> Result<Vec<String>, String> {
    let base = Settings::from_scene(scene, None);
    let (a, b) = (start.unwrap_or(base.start), end.unwrap_or(base.end));
    if b < a {
        return Err("The end frame is before the start frame".to_string());
    }
    let keep = scene.timeline.current;
    let mut written = Vec::new();
    let layers = batch_layers(scene);
    let result = (|| -> Result<(), String> {
        let mut f = a;
        while f <= b {
            scene.timeline.current = f as f32;
            scene.evaluate_frame();
            for layer in &layers {
                if let (Some(only), Some(l)) = (only_layer, layer) {
                    if l.name != only {
                        continue;
                    }
                }
                let st = Settings::from_scene(scene, layer.as_ref());
                let cams = match camera {
                    Some(c) => vec![c.to_string()],
                    None => render_cameras(scene, &st),
                };
                let mut passes: Vec<String> = vec!["beauty".to_string()];
                if let Some(l) = layer {
                    passes.extend(l.passes.iter().cloned());
                }
                for cam in &cams {
                    for pass in &passes {
                        let img = render_pass(scene, &st, layer.as_ref(), cam, pass, None)?;
                        let path = image_path(scene, &st, layer.as_ref().map(|l| l.name.as_str()), cam, pass, f);
                        img.save(&path, st.alpha || pass != "beauty")?;
                        progress(&path);
                        written.push(path);
                    }
                }
            }
            if st_animation_single(&base) {
                break;
            }
            f += base.by;
        }
        Ok(())
    })();
    scene.timeline.current = keep;
    scene.evaluate_frame();
    result?;
    Ok(written)
}

fn st_animation_single(st: &Settings) -> bool {
    st.animation == 0
}

/// Frames of an image sequence next to `path` (`name.0001.png`): the sorted file names and the frame numbers.
pub fn find_sequence(path: &str) -> Vec<(i32, String)> {
    let p = std::path::Path::new(path);
    let (Some(dir), Some(file)) = (p.parent().map(|d| if d.as_os_str().is_empty() { std::path::Path::new(".") } else { d }), p.file_name().map(|f| f.to_string_lossy().to_string())) else { return Vec::new() };
    // name.<digits>.ext
    let parts: Vec<&str> = file.split('.').collect();
    let (stem, ext) = if parts.len() >= 3 && parts[parts.len() - 2].chars().all(|c| c.is_ascii_digit()) {
        (parts[..parts.len() - 2].join("."), parts[parts.len() - 1].to_string())
    } else {
        return if p.exists() { vec![(0, path.to_string())] } else { Vec::new() };
    };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let ps: Vec<&str> = name.split('.').collect();
            if ps.len() >= 3 && ps[ps.len() - 1] == ext && ps[..ps.len() - 2].join(".") == stem && ps[ps.len() - 2].chars().all(|c| c.is_ascii_digit()) {
                if let Ok(n) = ps[ps.len() - 2].parse::<i32>() {
                    out.push((n, dir.join(&name).to_string_lossy().to_string()));
                }
            }
        }
    }
    out.sort();
    out
}
