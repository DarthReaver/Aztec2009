//! Color map loading and sampling for the software renderer.

use crate::math::Vec3;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub struct Texture {
    pub w: usize,
    pub h: usize,
    /// Linear RGB, row major, row 0 at the top of the image.
    pub rgb: Vec<[f32; 3]>,
    /// Alpha channel of images that have one that is not fully opaque.
    pub alpha: Option<Vec<f32>>,
}

fn cache() -> &'static Mutex<HashMap<String, Option<Arc<Texture>>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<Arc<Texture>>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn srgb_to_linear(v: f32) -> f32 {
    v.powf(2.2)
}

fn checker() -> Texture {
    let n = 256usize;
    let cells = 8usize;
    let mut rgb = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let on = ((x * cells / n) + (y * cells / n)) % 2 == 0;
            let c = if on { [0.92, 0.92, 0.92] } else { [0.22, 0.34, 0.62] };
            rgb.push([srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2])]);
        }
    }
    Texture { w: n, h: n, rgb, alpha: None }
}

/// Names of the generated patterns usable as `builtin:NAME`.
pub const BUILTINS: [&str; 7] = ["checker", "grid", "bricks", "stripes", "dots", "noise", "gradient"];

fn hash2(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393).wrapping_add((y as u32).wrapping_mul(668_265_263));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn value_noise(x: f32, y: f32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash2(xi, yi) + (hash2(xi + 1, yi) - hash2(xi, yi)) * sx;
    let b = hash2(xi, yi + 1) + (hash2(xi + 1, yi + 1) - hash2(xi, yi + 1)) * sx;
    a + (b - a) * sy
}

/// A generated 256 x 256 pattern that tiles seamlessly.
fn pattern(name: &str) -> Option<Texture> {
    let n = 256usize;
    let mut rgb = Vec::with_capacity(n * n);
    let lin = |c: [f32; 3]| [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2])];
    for y in 0..n {
        for x in 0..n {
            let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
            let c = match name {
                "grid" => {
                    let line = |t: f32| (t * 8.0).fract().min(1.0 - (t * 8.0).fract()) < 0.04;
                    if line(u) || line(v) { [0.12, 0.12, 0.14] } else { [0.85, 0.87, 0.9] }
                }
                "bricks" => {
                    let row = (v * 8.0).floor();
                    let shift = if row as i32 % 2 == 0 { 0.0 } else { 0.5 / 4.0 };
                    let (bu, bv) = (((u + shift) * 4.0).fract(), (v * 8.0).fract());
                    if bu < 0.06 || bv < 0.12 {
                        [0.78, 0.76, 0.7]
                    } else {
                        let k = hash2(((u + shift) * 4.0).floor() as i32, row as i32) * 0.25;
                        [0.62 + k, 0.28 + k * 0.5, 0.2 + k * 0.4]
                    }
                }
                "stripes" => {
                    if ((u * 8.0).floor() as i32) % 2 == 0 { [0.9, 0.82, 0.2] } else { [0.15, 0.15, 0.18] }
                }
                "dots" => {
                    let (du, dv) = ((u * 8.0).fract() - 0.5, (v * 8.0).fract() - 0.5);
                    if du * du + dv * dv < 0.14 * 0.14 * 4.0 { [0.8, 0.2, 0.25] } else { [0.93, 0.92, 0.88] }
                }
                "noise" => {
                    // four octaves of value noise, wrapped so the pattern tiles
                    let mut s = 0.0;
                    let mut amp = 0.5;
                    let mut freq = 4.0;
                    for _ in 0..4 {
                        let (px, py) = (u * freq, v * freq);
                        let f = freq as i32;
                        let (xi, yi) = (px.floor() as i32, py.floor() as i32);
                        let (fx, fy) = (px - px.floor(), py - py.floor());
                        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
                        let h = |a: i32, b: i32| hash2(a.rem_euclid(f), b.rem_euclid(f));
                        let a = h(xi, yi) + (h(xi + 1, yi) - h(xi, yi)) * sx;
                        let b = h(xi, yi + 1) + (h(xi + 1, yi + 1) - h(xi, yi + 1)) * sx;
                        s += (a + (b - a) * sy) * amp;
                        amp *= 0.5;
                        freq *= 2.0;
                    }
                    let _ = value_noise;
                    [0.08 + s * 0.45, 0.2 + s * 0.6, 0.12 + s * 0.35]
                }
                "gradient" => [0.15 + 0.8 * u, 0.3 + 0.5 * (1.0 - v), 0.9 - 0.7 * u],
                _ => return None,
            };
            rgb.push(lin(c));
        }
    }
    Some(Texture { w: n, h: n, rgb, alpha: None })
}

/// Load (and cache) a texture. `builtin:NAME` (see `BUILTINS`) is a generated pattern.
pub fn load(path: &str) -> Option<Arc<Texture>> {
    let mut c = cache().lock().ok()?;
    if let Some(t) = c.get(path) {
        return t.clone();
    }
    let tex: Option<Arc<Texture>> = if path == "builtin:checker" {
        Some(Arc::new(checker()))
    } else if let Some(name) = path.strip_prefix("builtin:") {
        pattern(name).map(Arc::new)
    } else {
        match image::open(path) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let alpha: Option<Vec<f32>> = if rgba.pixels().any(|p| p[3] < 255) { Some(rgba.pixels().map(|p| p[3] as f32 / 255.0).collect()) } else { None };
                let img = img.to_rgb8();
                let (w, h) = (img.width() as usize, img.height() as usize);
                let rgb = img
                    .pixels()
                    .map(|p| {
                        [
                            srgb_to_linear(p[0] as f32 / 255.0),
                            srgb_to_linear(p[1] as f32 / 255.0),
                            srgb_to_linear(p[2] as f32 / 255.0),
                        ]
                    })
                    .collect();
                Some(Arc::new(Texture { w, h, rgb, alpha }))
            }
            Err(_) => None,
        }
    };
    c.insert(path.to_string(), tex.clone());
    tex
}

/// Put an image into the cache under `path` (the 3D Paint tool does this after every stroke).
pub fn store(path: &str, tex: Texture) {
    if let Ok(mut c) = cache().lock() {
        c.insert(path.to_string(), Some(Arc::new(tex)));
    }
}

/// Forget a cached file so a changed image is picked up again.
pub fn invalidate(path: &str) {
    if let Ok(mut c) = cache().lock() {
        c.remove(path);
    }
}

/// A texture as one material uses it: the image plus the placement and gain of its texture node.
#[derive(Clone)]
pub struct TexInst {
    pub tex: Arc<Texture>,
    pub rep: (f32, f32),
    pub off: (f32, f32),
    pub rot_deg: f32,
    pub gain: Vec3,
    /// The material's transparency comes from this texture's alpha channel.
    pub alpha_transparency: bool,
}

impl TexInst {
    pub fn plain(tex: Arc<Texture>) -> TexInst {
        TexInst { tex, rep: (1.0, 1.0), off: (0.0, 0.0), rot_deg: 0.0, gain: Vec3::ONE, alpha_transparency: false }
    }

    /// UV after the placement: rotate about the middle, then repeat and offset.
    fn place(&self, u: f32, v: f32) -> (f32, f32) {
        let (u, v) = if self.rot_deg.abs() > 1e-6 {
            let (s, c) = self.rot_deg.to_radians().sin_cos();
            let (du, dv) = (u - 0.5, v - 0.5);
            (du * c - dv * s + 0.5, du * s + dv * c + 0.5)
        } else {
            (u, v)
        };
        (u * self.rep.0 + self.off.0, v * self.rep.1 + self.off.1)
    }

    pub fn sample(&self, u: f32, v: f32) -> Vec3 {
        let (a, b) = self.place(u, v);
        self.tex.sample(a, b).mul_elem(self.gain)
    }

    /// Coverage 0..1 (1 = opaque) from the image's alpha channel.
    pub fn alpha(&self, u: f32, v: f32) -> f32 {
        let (a, b) = self.place(u, v);
        self.tex.sample_alpha(a, b)
    }
}

impl Texture {
    /// Nearest sample of the alpha channel with wrapping (1.0 when the image has none).
    pub fn sample_alpha(&self, u: f32, v: f32) -> f32 {
        let Some(a) = &self.alpha else { return 1.0 };
        let uu = u - u.floor();
        let vv = 1.0 - (v - v.floor());
        let x = ((uu * self.w as f32) as usize).min(self.w - 1);
        let y = ((vv * self.h as f32) as usize).min(self.h - 1);
        a[y * self.w + x]
    }

    /// Bilinear sample with wrapping. `v` = 0 is the bottom of the image.
    pub fn sample(&self, u: f32, v: f32) -> Vec3 {
        let uu = u - u.floor();
        let vv = 1.0 - (v - v.floor());
        let fx = uu * self.w as f32 - 0.5;
        let fy = vv * self.h as f32 - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let (tx, ty) = (fx - x0, fy - y0);
        let wrap = |i: i64, n: usize| -> usize { i.rem_euclid(n as i64) as usize };
        let (xi, yi) = (x0 as i64, y0 as i64);
        let px = |x: i64, y: i64| -> Vec3 {
            let c = self.rgb[wrap(y, self.h) * self.w + wrap(x, self.w)];
            Vec3::new(c[0], c[1], c[2])
        };
        let a = px(xi, yi).lerp(px(xi + 1, yi), tx);
        let b = px(xi, yi + 1).lerp(px(xi + 1, yi + 1), tx);
        a.lerp(b, ty)
    }
}
