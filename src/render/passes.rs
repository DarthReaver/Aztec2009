use super::anaglyph::{default_stereo, render_stereo};
use super::rasterizer::{draw_wireframe, rasterize, shade};
use super::{build_scene, camera_info, CameraInfo, RenderScene};
use crate::dag::Scene;
use crate::math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PassKind {
    Beauty,
    Depth,
    Normal,
    Wireframe,
    Anaglyph,
}

impl PassKind {
    pub fn name(self) -> &'static str {
        match self {
            PassKind::Beauty => "beauty",
            PassKind::Depth => "depth",
            PassKind::Normal => "normal",
            PassKind::Wireframe => "wireframe",
            PassKind::Anaglyph => "anaglyph",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Ascii,
    Ansi,
    Ppm,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s.to_ascii_lowercase().as_str() {
            "ascii" | "txt" | "text" => Some(Format::Ascii),
            "ansi" | "color" => Some(Format::Ansi),
            "ppm" => Some(Format::Ppm),
            _ => None,
        }
    }
}

/// All passes from one raster of the scene.
pub struct Passes {
    pub w: usize,
    pub h: usize,
    pub beauty: Vec<Vec3>,
    /// 1.0 = nearest surface, 0.0 = farthest or empty.
    pub depth: Vec<f32>,
    /// World normal encoded to 0..1 color.
    pub normal_rgb: Vec<Vec3>,
    /// |N . V| facing ratio used for the ASCII normal pass.
    pub facing: Vec<f32>,
    pub wire: Vec<u8>,
    pub covered: Vec<bool>,
}

pub fn render_passes(rs: &RenderScene, cam: &CameraInfo, w: usize, h: usize, pixel_aspect: f32, bg: Vec3) -> Passes {
    let aspect = (w as f32 / h as f32) * pixel_aspect;
    let view = cam.view();
    let proj = cam.projection(aspect);
    let mut gb = rasterize(rs, &view, &proj, w, h);
    draw_wireframe(&mut gb, rs, &view, &proj, 1.0 / pixel_aspect.max(0.05));
    let eye = cam.eye();
    let beauty = shade(&gb, rs, eye, bg);
    let n = w * h;
    let mut dmin = f32::INFINITY;
    let mut dmax = f32::NEG_INFINITY;
    for &d in &gb.depth {
        if d.is_finite() {
            dmin = dmin.min(d);
            dmax = dmax.max(d);
        }
    }
    let span = (dmax - dmin).max(1e-6);
    let mut depth = vec![0.0; n];
    let mut normal_rgb = vec![Vec3::ZERO; n];
    let mut facing = vec![0.0; n];
    let mut covered = vec![false; n];
    for i in 0..n {
        if gb.depth[i].is_finite() {
            covered[i] = true;
            depth[i] = if dmax > dmin { 1.0 - (gb.depth[i] - dmin) / span * 0.9 } else { 1.0 };
            let nr = gb.normal[i];
            normal_rgb[i] = Vec3::new(nr.x * 0.5 + 0.5, nr.y * 0.5 + 0.5, nr.z * 0.5 + 0.5);
            let v = (eye - gb.pos[i]).normalized();
            facing[i] = nr.dot(v).abs().clamp(0.0, 1.0);
        }
    }
    Passes { w, h, beauty, depth, normal_rgb, facing, wire: gb.wire, covered }
}

const RAMP: &[u8] = b" .:-=+*#%@";

fn ramp_char(v: f32) -> char {
    let i = (v.clamp(0.0, 1.0) * (RAMP.len() - 1) as f32).round() as usize;
    RAMP[i] as char
}

pub fn luminance(c: Vec3) -> f32 {
    0.299 * c.x + 0.587 * c.y + 0.114 * c.z
}

fn rows_to_string(chars: &[char], w: usize, h: usize) -> String {
    let mut s = String::with_capacity((w + 1) * h);
    for y in 0..h {
        let line: String = chars[y * w..(y + 1) * w].iter().collect();
        s.push_str(line.trim_end());
        s.push('\n');
    }
    s
}

pub fn ascii_pass(p: &Passes, kind: PassKind) -> String {
    let n = p.w * p.h;
    let mut chars = vec![' '; n];
    for i in 0..n {
        chars[i] = match kind {
            PassKind::Beauty | PassKind::Anaglyph => ramp_char(luminance(p.beauty[i])),
            PassKind::Depth => {
                if p.covered[i] {
                    ramp_char(p.depth[i])
                } else {
                    ' '
                }
            }
            PassKind::Normal => {
                if p.covered[i] {
                    ramp_char(p.facing[i])
                } else {
                    ' '
                }
            }
            PassKind::Wireframe => match p.wire[i] {
                1 => '-',
                2 => '|',
                3 => '/',
                4 => '\\',
                _ => ' ',
            },
        };
    }
    rows_to_string(&chars, p.w, p.h)
}

pub fn ascii_from_colors(pixels: &[Vec3], w: usize, h: usize) -> Vec<char> {
    pixels.iter().map(|c| ramp_char(luminance(*c))).take(w * h).collect()
}

/// Half block truecolor image: two pixel rows per text row.
pub fn ansi_image(pixels: &[Vec3], w: usize, h: usize) -> String {
    let mut s = String::new();
    let px = |x: usize, y: usize| -> (u8, u8, u8) {
        let y = y.min(h - 1);
        let c = pixels[y * w + x];
        ((c.x.clamp(0.0, 1.0) * 255.0) as u8, (c.y.clamp(0.0, 1.0) * 255.0) as u8, (c.z.clamp(0.0, 1.0) * 255.0) as u8)
    };
    let mut y = 0;
    while y < h {
        for x in 0..w {
            let t = px(x, y);
            let b = px(x, y + 1);
            s.push_str(&format!("\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m\u{2580}", t.0, t.1, t.2, b.0, b.1, b.2));
        }
        s.push_str("\x1b[0m\n");
        y += 2;
    }
    s
}

pub fn ppm_image(pixels: &[Vec3], w: usize, h: usize) -> String {
    let mut s = format!("P3\n{} {}\n255\n", w, h);
    for y in 0..h {
        let mut parts: Vec<String> = Vec::with_capacity(w);
        for x in 0..w {
            let c = pixels[y * w + x];
            parts.push(format!(
                "{} {} {}",
                (c.x.clamp(0.0, 1.0) * 255.0).round() as u8,
                (c.y.clamp(0.0, 1.0) * 255.0).round() as u8,
                (c.z.clamp(0.0, 1.0) * 255.0).round() as u8
            ));
        }
        s.push_str(&parts.join(" "));
        s.push('\n');
    }
    s
}

fn pass_pixels(p: &Passes, kind: PassKind) -> Vec<Vec3> {
    let n = p.w * p.h;
    match kind {
        PassKind::Beauty | PassKind::Anaglyph => p.beauty.clone(),
        PassKind::Depth => (0..n)
            .map(|i| if p.covered[i] { Vec3::splat(p.depth[i]) } else { Vec3::ZERO })
            .collect(),
        PassKind::Normal => p.normal_rgb.clone(),
        PassKind::Wireframe => (0..n).map(|i| if p.wire[i] != 0 { Vec3::ONE } else { Vec3::ZERO }).collect(),
    }
}

pub struct RenderRequest<'a> {
    /// beauty, depth, normal, wireframe, anaglyph, or all
    pub mode: &'a str,
    pub format: Option<Format>,
    pub w: usize,
    pub h: usize,
    pub camera: Option<&'a str>,
}

/// Render the scene at its current frame into text for a file or the terminal.
pub fn render_text(scene: &mut Scene, req: &RenderRequest) -> Result<String, String> {
    let kinds: Vec<PassKind> = match req.mode.to_ascii_lowercase().as_str() {
        "beauty" | "color" => vec![PassKind::Beauty],
        "depth" | "z" => vec![PassKind::Depth],
        "normal" | "normals" => vec![PassKind::Normal],
        "wireframe" | "wire" => vec![PassKind::Wireframe],
        "anaglyph" | "stereo" => vec![PassKind::Anaglyph],
        "all" => vec![PassKind::Beauty, PassKind::Depth, PassKind::Normal, PassKind::Wireframe],
        other => return Err(format!("Unknown render mode '{}'. Use beauty, depth, normal, wireframe, anaglyph or all", other)),
    };
    if req.w < 8 || req.h < 4 || req.w > 600 || req.h > 400 {
        return Err("Resolution out of range (width 8..600, height 4..400)".to_string());
    }
    scene.evaluate_frame();
    let rs = build_scene(scene);
    let cam = camera_info(scene, req.camera)?;
    let frame = scene.timeline.frame();
    let bg = Vec3::new(0.0, 0.0, 0.0);
    let mut out = String::new();
    for kind in kinds {
        let fmt = req.format.unwrap_or(if kind == PassKind::Anaglyph { Format::Ansi } else { Format::Ascii });
        let header = format!(
            "# aztec2009 {} pass {}x{} frame {} camera {} format {:?}\n",
            kind.name(),
            req.w,
            req.h,
            frame,
            cam.name,
            fmt
        );
        if kind == PassKind::Anaglyph {
            let st = cam.stereo.clone().unwrap_or_else(|| default_stereo(&cam));
            match fmt {
                Format::Ascii => {
                    // Parallel view: left eye image, then right eye image.
                    let imgs = render_stereo(&rs, &cam, &st, req.w, req.h, 0.5, bg);
                    let l = ascii_from_colors(&imgs.left, req.w, req.h);
                    let r = ascii_from_colors(&imgs.right, req.w, req.h);
                    out.push_str(&header);
                    for y in 0..req.h {
                        let ll: String = l[y * req.w..(y + 1) * req.w].iter().collect();
                        let rr: String = r[y * req.w..(y + 1) * req.w].iter().collect();
                        out.push_str(format!("{} | {}\n", ll, rr).trim_end());
                        out.push('\n');
                    }
                }
                Format::Ansi => {
                    let imgs = render_stereo(&rs, &cam, &st, req.w, req.h, 1.0, bg);
                    out.push_str(&header);
                    out.push_str(&ansi_image(&imgs.anaglyph, req.w, req.h));
                }
                Format::Ppm => {
                    let imgs = render_stereo(&rs, &cam, &st, req.w, req.h, 1.0, bg);
                    out.push_str(&ppm_image(&imgs.anaglyph, req.w, req.h));
                }
            }
            continue;
        }
        let aspect = if fmt == Format::Ascii { 0.5 } else { 1.0 };
        let passes = render_passes(&rs, &cam, req.w, req.h, aspect, bg);
        match fmt {
            Format::Ascii => {
                out.push_str(&header);
                out.push_str(&ascii_pass(&passes, kind));
            }
            Format::Ansi => {
                out.push_str(&header);
                out.push_str(&ansi_image(&pass_pixels(&passes, kind), req.w, req.h));
            }
            Format::Ppm => {
                out.push_str(&ppm_image(&pass_pixels(&passes, kind), req.w, req.h));
            }
        }
        if req.mode.eq_ignore_ascii_case("all") {
            out.push('\n');
        }
    }
    Ok(out)
}
