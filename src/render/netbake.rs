//! Texture networks: file / pattern / ramp / noise / fractal / cloth nodes whose color inputs can be fed by
//! other textures (Hypershade connections). A network is compiled from the scene and baked into one image,
//! so the renderers keep sampling a single texture.

use super::texture::{self, Texture};
use crate::dag::graph::{NodeId, Scene};
use crate::dag::nodes::{NodeData, NET_KINDS};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

type Rgb = [f32; 3];

pub enum Input {
    Const(Rgb),
    Tex(Box<Net>),
}

pub struct Place {
    pub rep: (f32, f32),
    pub off: (f32, f32),
    pub rot_deg: f32,
    pub wrap: (bool, bool),
    pub default: Rgb,
}

pub enum Kind {
    Image(Arc<Texture>),
    Ramp { ty: u8, interp: u8, entries: Vec<(f32, Input)> },
    Noise { amp: f32, ratio: f32, fratio: f32, octaves: u32, threshold: f32, freq: f32, bias: f32, seed: u32 },
    Cloth { u: Input, v: Input, uw: f32, vw: f32, uwave: f32, vwave: f32, spread: f32 },
}

pub struct Net {
    pub kind: Kind,
    pub place: Place,
    pub gain: Input,
    pub offset: Rgb,
}

fn hash_f(h: &mut impl Hasher, v: f32) {
    v.to_bits().hash(h);
}

/// Does a node's color need network evaluation (procedural kind, or any connected input)?
pub fn needs_network(scene: &Scene, node: NodeId) -> bool {
    let Some(n) = scene.node(node) else { return false };
    let NodeData::Texture(td) = &n.data else { return false };
    if NET_KINDS.contains(&td.kind.as_str()) {
        return true;
    }
    n.plugs.iter().any(|p| p.source.as_ref().map(|s| matches!(scene.node(s.node).map(|x| &x.data), Some(NodeData::Texture(_)))).unwrap_or(false))
}

/// The input feeding the three channels `base` + R/G/B of a node: a texture, or the constant values.
fn input(scene: &mut Scene, node: NodeId, base: &str, h: &mut impl Hasher, depth: u32) -> Input {
    let names = [format!("{}R", base), format!("{}G", base), format!("{}B", base)];
    let src = scene.node(node).and_then(|n| n.plug(&names[0])).and_then(|p| p.source.clone());
    if let Some(s) = src {
        if matches!(scene.node(s.node).map(|x| &x.data), Some(NodeData::Texture(_))) && depth < 8 {
            if let Some(net) = compile_node(scene, s.node, h, depth + 1) {
                return Input::Tex(Box::new(net));
            }
        }
    }
    let v: Vec<f32> = names.iter().map(|n| scene.get_attr(node, n).unwrap_or(0.0)).collect();
    for x in &v {
        hash_f(h, *x);
    }
    Input::Const([v[0], v[1], v[2]])
}

fn compile_node(scene: &mut Scene, node: NodeId, h: &mut impl Hasher, depth: u32) -> Option<Net> {
    let (kind_name, path) = match scene.node(node).map(|n| &n.data) {
        Some(NodeData::Texture(td)) => (td.kind.clone(), td.path.clone()),
        _ => return None,
    };
    kind_name.hash(h);
    let g = |s: &mut Scene, a: &str, d: f32| s.get_attr(node, a).unwrap_or(d);
    let place = Place {
        rep: (g(scene, "repeatU", 1.0), g(scene, "repeatV", 1.0)),
        off: (g(scene, "offsetU", 0.0), g(scene, "offsetV", 0.0)),
        rot_deg: g(scene, "rotateUV", 0.0),
        wrap: (g(scene, "wrapU", 1.0) > 0.5, g(scene, "wrapV", 1.0) > 0.5),
        default: [g(scene, "defaultColorR", 0.5), g(scene, "defaultColorG", 0.5), g(scene, "defaultColorB", 0.5)],
    };
    for v in [place.rep.0, place.rep.1, place.off.0, place.off.1, place.rot_deg, place.wrap.0 as u8 as f32, place.wrap.1 as u8 as f32, place.default[0], place.default[1], place.default[2]] {
        hash_f(h, v);
    }
    let gain = input(scene, node, "colorGain", h, depth);
    let offset = [g(scene, "colorOffsetR", 0.0), g(scene, "colorOffsetG", 0.0), g(scene, "colorOffsetB", 0.0)];
    for v in offset {
        hash_f(h, v);
    }
    let kind = match kind_name.as_str() {
        "ramp" => {
            let count = (g(scene, "entryCount", 3.0).round() as usize).clamp(2, 5);
            let mut entries = Vec::new();
            for i in 0..count {
                let pos = g(scene, &format!("ePos{}", i), 0.0);
                hash_f(h, pos);
                entries.push((pos, input(scene, node, &format!("eCol{}", i), h, depth)));
            }
            entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let (ty, interp) = (g(scene, "rampType", 0.0).round() as u8, g(scene, "interpolation", 1.0).round() as u8);
            hash_f(h, ty as f32);
            hash_f(h, interp as f32);
            Kind::Ramp { ty, interp, entries }
        }
        "noise" | "brownian" | "fractal" => {
            let n = Kind::Noise {
                amp: g(scene, "amplitude", 1.0),
                ratio: g(scene, "ratio", 0.707),
                fratio: g(scene, "frequencyRatio", 2.0),
                octaves: g(scene, "octaves", 4.0).round().clamp(1.0, 10.0) as u32,
                threshold: g(scene, "threshold", 0.0),
                freq: g(scene, "frequency", 8.0),
                bias: g(scene, "bias", 0.0),
                seed: if kind_name == "fractal" { 7 } else if kind_name == "brownian" { 13 } else { 1 },
            };
            if let Kind::Noise { amp, ratio, fratio, octaves, threshold, freq, bias, seed } = &n {
                for v in [*amp, *ratio, *fratio, *octaves as f32, *threshold, *freq, *bias, *seed as f32] {
                    hash_f(h, v);
                }
            }
            n
        }
        "cloth" => {
            let u = input(scene, node, "uColor", h, depth);
            let v = input(scene, node, "vColor", h, depth);
            let k = Kind::Cloth { u, v, uw: g(scene, "uWidth", 0.1).max(0.001), vw: g(scene, "vWidth", 0.1).max(0.001), uwave: g(scene, "uWave", 0.0), vwave: g(scene, "vWave", 0.0), spread: g(scene, "brightSpread", 0.5) };
            if let Kind::Cloth { uw, vw, uwave, vwave, spread, .. } = &k {
                for v in [*uw, *vw, *uwave, *vwave, *spread] {
                    hash_f(h, v);
                }
            }
            k
        }
        other => {
            let file = if other == "file" { scene.texture_file(&path) } else { format!("builtin:{}", other) };
            file.hash(h);
            if other == "file" {
                if let Ok(md) = std::fs::metadata(&file) {
                    md.len().hash(h);
                    md.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).hash(h);
                }
            }
            // an image that cannot be read shows as the default color
            match texture::load(&file) {
                Some(t) => Kind::Image(t),
                None => Kind::Image(Arc::new(Texture { w: 1, h: 1, rgb: vec![place.default], alpha: None })),
            }
        }
    };
    Some(Net { kind, place, gain, offset })
}

/// Compile the network feeding `node` and a hash of everything that affects its look.
pub fn compile(scene: &mut Scene, node: NodeId) -> Option<(Net, u64)> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let net = compile_node(scene, node, &mut h, 0)?;
    Some((net, h.finish()))
}

fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393).wrapping_add((y as u32).wrapping_mul(668_265_263)).wrapping_add(seed.wrapping_mul(2_246_822_519));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65535.0
}

fn vnoise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash2(ix, iy, seed) + (hash2(ix + 1, iy, seed) - hash2(ix, iy, seed)) * sx;
    let b = hash2(ix, iy + 1, seed) + (hash2(ix + 1, iy + 1, seed) - hash2(ix, iy + 1, seed)) * sx;
    a + (b - a) * sy
}

fn fbm(u: f32, v: f32, freq: f32, octaves: u32, ratio: f32, fratio: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, freq, 0.0);
    for o in 0..octaves {
        sum += vnoise(u * f + o as f32 * 17.3, v * f + o as f32 * 9.1, seed + o) * amp;
        norm += amp;
        amp *= ratio;
        f *= fratio;
    }
    sum / norm.max(1e-6)
}

impl Place {
    fn apply(&self, u: f32, v: f32) -> (f32, f32) {
        let (u, v) = if self.rot_deg.abs() > 1e-6 {
            let (s, c) = self.rot_deg.to_radians().sin_cos();
            let (du, dv) = (u - 0.5, v - 0.5);
            (du * c - dv * s + 0.5, du * s + dv * c + 0.5)
        } else {
            (u, v)
        };
        (u * self.rep.0 + self.off.0, v * self.rep.1 + self.off.1)
    }
}

impl Input {
    fn at(&self, u: f32, v: f32) -> Rgb {
        match self {
            Input::Const(c) => *c,
            Input::Tex(n) => n.at(u, v),
        }
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

impl Net {
    /// Color of the texture at surface coordinates (u, v), placement and color balance included.
    pub fn at(&self, u: f32, v: f32) -> Rgb {
        let (pu, pv) = self.place.apply(u, v);
        if (!self.place.wrap.0 && !(0.0..=1.0).contains(&pu)) || (!self.place.wrap.1 && !(0.0..=1.0).contains(&pv)) {
            return self.place.default;
        }
        let base: Rgb = match &self.kind {
            Kind::Image(t) => {
                let c = t.sample(pu, pv);
                [c.x, c.y, c.z]
            }
            Kind::Noise { amp, ratio, fratio, octaves, threshold, freq, bias, seed } => {
                let n = fbm(pu, pv, *freq, *octaves, *ratio, *fratio, *seed);
                let x = (threshold + (n - 0.5) * amp * 2.0 + 0.5 + bias).clamp(0.0, 1.0);
                [x, x, x]
            }
            Kind::Ramp { ty, interp, entries } => {
                let wu = pu - pu.floor();
                let wv = pv - pv.floor();
                let t = match ty {
                    1 => wu,
                    2 => (wu + wv) * 0.5,
                    3 => ((wv - 0.5).atan2(wu - 0.5) / std::f32::consts::TAU + 0.5).clamp(0.0, 1.0),
                    4 => (((wu - 0.5).powi(2) + (wv - 0.5).powi(2)).sqrt() * 2.0).clamp(0.0, 1.0),
                    5 => ((wu - 0.5).abs().max((wv - 0.5).abs()) * 2.0).clamp(0.0, 1.0),
                    _ => wv,
                };
                ramp_color(entries, *interp, t, u, v)
            }
            Kind::Cloth { u: uc, v: vc, uw, vw, uwave, vwave, spread } => {
                let su = pu + (pv * 12.0).sin() * uwave * 0.1;
                let sv = pv + (pu * 12.0).sin() * vwave * 0.1;
                let (cu, cv) = (su / uw, sv / vw);
                let (fu, fv) = (cu - cu.floor(), cv - cv.floor());
                let over = ((cu.floor() as i64 + cv.floor() as i64) & 1) == 0;
                let (col, across) = if over { (uc.at(u, v), fv) } else { (vc.at(u, v), fu) };
                let bright = 1.0 - spread * (2.0 * across - 1.0).powi(2);
                [col[0] * bright, col[1] * bright, col[2] * bright]
            }
        };
        let g = self.gain.at(u, v);
        [(base[0] * g[0] + self.offset[0]).max(0.0), (base[1] * g[1] + self.offset[1]).max(0.0), (base[2] * g[2] + self.offset[2]).max(0.0)]
    }
}

fn ramp_color(entries: &[(f32, Input)], interp: u8, t: f32, u: f32, v: f32) -> Rgb {
    if entries.is_empty() {
        return [0.0; 3];
    }
    if t <= entries[0].0 {
        return entries[0].1.at(u, v);
    }
    for w in entries.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-6);
            let f = ((t - w[0].0) / span).clamp(0.0, 1.0);
            let f = match interp {
                0 => 0.0,
                2 => f * f,
                3 => 1.0 - (1.0 - f) * (1.0 - f),
                4 => f * f * (3.0 - 2.0 * f),
                _ => f,
            };
            return mix(w[0].1.at(u, v), w[1].1.at(u, v), f);
        }
    }
    entries.last().unwrap().1.at(u, v)
}

/// Evaluate the network over the unit square into an image (v = 0 at the bottom).
pub fn bake(net: &Net, res: usize) -> Texture {
    let mut rgb = Vec::with_capacity(res * res);
    for y in 0..res {
        let v = 1.0 - (y as f32 + 0.5) / res as f32;
        for x in 0..res {
            let u = (x as f32 + 0.5) / res as f32;
            rgb.push(net.at(u, v));
        }
    }
    Texture { w: res, h: res, rgb, alpha: None }
}

/// The baked image of a texture node's network, cached while its inputs do not change.
pub fn baked(scene: &mut Scene, node: NodeId, res: usize) -> Option<Arc<Texture>> {
    let (net, key) = compile(scene, node)?;
    let cache_key = (node, res);
    if let Some((k, t)) = scene.net_cache.get(&cache_key) {
        if *k == key {
            return Some(t.clone());
        }
    }
    let tex = Arc::new(bake(&net, res));
    scene.net_cache.insert(cache_key, (key, tex.clone()));
    Some(tex)
}
