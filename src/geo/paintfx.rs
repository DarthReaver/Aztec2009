//! Paint Effects: brush presets and the growth of tubes, branches, leaves and flowers along a stroke.

use super::mesh::Mesh;
use crate::math::{Quat, Vec2, Vec3};

/// Brush channels with their defaults. Every brush node has all of them; presets override some.
pub const BRUSH_DEFAULTS: &[(&str, f32)] = &[
    ("brushWidth", 0.5),
    ("globalScale", 1.0),
    ("spacing", 0.4),
    ("tubesPerStep", 1.0),
    ("lengthMin", 1.0),
    ("lengthMax", 2.0),
    ("tubeWidth1", 0.06),
    ("tubeWidth2", 0.01),
    ("widthRand", 0.1),
    ("segments", 8.0),
    ("sides", 5.0),
    ("tubeRand", 0.15),
    ("gravity", 0.0),
    ("curl", 0.0),
    ("branches", 0.0),
    ("branchLevels", 1.0),
    ("branchCount", 3.0),
    ("branchAngle", 45.0),
    ("branchLength", 0.6),
    ("branchStart", 0.3),
    ("leaves", 0.0),
    ("leafSize", 0.15),
    ("leafCount", 3.0),
    ("flowers", 0.0),
    ("flowerSize", 0.2),
    ("color1R", 0.35),
    ("color1G", 0.22),
    ("color1B", 0.1),
    ("color2R", 0.2),
    ("color2G", 0.3),
    ("color2B", 0.08),
    ("leafColorR", 0.2),
    ("leafColorG", 0.55),
    ("leafColorB", 0.15),
    ("flowerColorR", 0.95),
    ("flowerColorG", 0.85),
    ("flowerColorB", 0.2),
    ("wind", 0.0),
];

/// Preset folders of the Visor and the brushes in them.
pub const PRESET_FOLDERS: &[(&str, &[&str])] = &[
    ("trees", &["gloppy", "oak", "pine", "bush"]),
    ("grasses", &["astroturf", "grassBermuda"]),
    ("flowers", &["daisy", "vine"]),
    ("fun", &["jetTrail", "airbrush"]),
];

/// The channels a preset changes from `BRUSH_DEFAULTS`.
pub fn preset(name: &str) -> Option<Vec<(&'static str, f32)>> {
    Some(match name {
        "gloppy" => vec![("brushWidth", 0.4), ("tubesPerStep", 1.0), ("lengthMin", 2.5), ("lengthMax", 3.5), ("tubeWidth1", 0.28), ("tubeWidth2", 0.04), ("segments", 10.0), ("sides", 6.0), ("tubeRand", 0.2), ("curl", 0.25), ("branches", 1.0), ("branchLevels", 2.0), ("branchCount", 4.0), ("branchAngle", 55.0), ("branchLength", 0.55), ("branchStart", 0.4), ("leaves", 1.0), ("leafSize", 0.35), ("leafCount", 4.0), ("color1R", 0.45), ("color1G", 0.3), ("color1B", 0.35), ("leafColorR", 0.75), ("leafColorG", 0.35), ("leafColorB", 0.7)],
        "oak" => vec![("brushWidth", 0.4), ("lengthMin", 3.0), ("lengthMax", 4.0), ("tubeWidth1", 0.22), ("tubeWidth2", 0.03), ("segments", 10.0), ("sides", 6.0), ("curl", 0.1), ("branches", 1.0), ("branchLevels", 2.0), ("branchCount", 5.0), ("branchAngle", 50.0), ("branchLength", 0.5), ("branchStart", 0.35), ("leaves", 1.0), ("leafSize", 0.3), ("leafCount", 5.0), ("leafColorR", 0.25), ("leafColorG", 0.5), ("leafColorB", 0.12)],
        "pine" => vec![("brushWidth", 0.3), ("lengthMin", 4.0), ("lengthMax", 5.0), ("tubeWidth1", 0.15), ("tubeWidth2", 0.02), ("segments", 8.0), ("sides", 5.0), ("tubeRand", 0.03), ("branches", 1.0), ("branchLevels", 1.0), ("branchCount", 10.0), ("branchAngle", 70.0), ("branchLength", 0.45), ("branchStart", 0.15), ("gravity", 0.05), ("leaves", 1.0), ("leafSize", 0.25), ("leafCount", 3.0), ("leafColorR", 0.08), ("leafColorG", 0.35), ("leafColorB", 0.12)],
        "bush" => vec![("brushWidth", 0.6), ("tubesPerStep", 3.0), ("lengthMin", 0.8), ("lengthMax", 1.4), ("tubeWidth1", 0.04), ("tubeWidth2", 0.01), ("segments", 6.0), ("sides", 4.0), ("tubeRand", 0.5), ("branches", 1.0), ("branchLevels", 1.0), ("branchCount", 3.0), ("leaves", 1.0), ("leafSize", 0.2), ("leafCount", 4.0)],
        "astroturf" => vec![("brushWidth", 0.6), ("spacing", 0.15), ("tubesPerStep", 6.0), ("lengthMin", 0.15), ("lengthMax", 0.3), ("tubeWidth1", 0.025), ("tubeWidth2", 0.004), ("segments", 3.0), ("sides", 3.0), ("tubeRand", 0.3), ("gravity", 0.02), ("color1R", 0.12), ("color1G", 0.45), ("color1B", 0.1), ("color2R", 0.3), ("color2G", 0.6), ("color2B", 0.15)],
        "grassBermuda" => vec![("brushWidth", 0.7), ("spacing", 0.12), ("tubesPerStep", 8.0), ("lengthMin", 0.3), ("lengthMax", 0.6), ("tubeWidth1", 0.03), ("tubeWidth2", 0.003), ("widthRand", 0.3), ("segments", 4.0), ("sides", 3.0), ("tubeRand", 0.35), ("gravity", 0.08), ("color1R", 0.2), ("color1G", 0.5), ("color1B", 0.1), ("color2R", 0.55), ("color2G", 0.7), ("color2B", 0.2)],
        "daisy" => vec![("brushWidth", 0.5), ("spacing", 0.6), ("tubesPerStep", 2.0), ("lengthMin", 0.8), ("lengthMax", 1.3), ("tubeWidth1", 0.03), ("tubeWidth2", 0.02), ("segments", 5.0), ("sides", 4.0), ("tubeRand", 0.1), ("gravity", 0.03), ("flowers", 1.0), ("flowerSize", 0.22), ("leaves", 1.0), ("leafSize", 0.12), ("leafCount", 2.0), ("color1R", 0.15), ("color1G", 0.45), ("color1B", 0.1), ("flowerColorR", 1.0), ("flowerColorG", 0.95), ("flowerColorB", 0.85)],
        "vine" => vec![("brushWidth", 0.2), ("spacing", 0.8), ("tubesPerStep", 1.0), ("lengthMin", 2.0), ("lengthMax", 3.0), ("tubeWidth1", 0.05), ("tubeWidth2", 0.02), ("segments", 14.0), ("sides", 4.0), ("tubeRand", 0.4), ("curl", 0.8), ("gravity", 0.1), ("leaves", 1.0), ("leafSize", 0.15), ("leafCount", 8.0), ("color1R", 0.2), ("color1G", 0.35), ("color1B", 0.1)],
        "jetTrail" => vec![("brushWidth", 0.8), ("spacing", 0.2), ("tubesPerStep", 5.0), ("lengthMin", 0.4), ("lengthMax", 0.9), ("tubeWidth1", 0.25), ("tubeWidth2", 0.1), ("widthRand", 0.5), ("segments", 3.0), ("sides", 4.0), ("tubeRand", 0.9), ("color1R", 0.95), ("color1G", 0.95), ("color1B", 0.97), ("color2R", 0.8), ("color2G", 0.82), ("color2B", 0.88)],
        "airbrush" => vec![("brushWidth", 0.5), ("spacing", 0.1), ("tubesPerStep", 3.0), ("lengthMin", 0.05), ("lengthMax", 0.15), ("tubeWidth1", 0.18), ("tubeWidth2", 0.15), ("segments", 1.0), ("sides", 4.0), ("tubeRand", 0.8), ("color1R", 0.9), ("color1G", 0.2), ("color1B", 0.2), ("color2R", 0.9), ("color2G", 0.2), ("color2B", 0.2)],
        _ => return None,
    })
}

/// All channel values of a preset (the defaults with the preset's changes).
pub fn preset_values(name: &str) -> Option<Vec<(String, f32)>> {
    let over = preset(name)?;
    Some(BRUSH_DEFAULTS.iter().map(|(k, v)| (k.to_string(), over.iter().find(|(o, _)| o == k).map(|(_, ov)| *ov).unwrap_or(*v))).collect())
}

/// Brush channel values by name.
#[derive(Clone, Debug)]
pub struct Params {
    vals: Vec<(String, f32)>,
}

impl Params {
    pub fn new(vals: Vec<(String, f32)>) -> Params {
        Params { vals }
    }
    pub fn g(&self, k: &str) -> f32 {
        self.vals.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or_else(|| BRUSH_DEFAULTS.iter().find(|(n, _)| *n == k).map(|(_, v)| *v).unwrap_or(0.0))
    }
    fn col(&self, base: &str) -> Vec3 {
        Vec3::new(self.g(&format!("{}R", base)), self.g(&format!("{}G", base)), self.g(&format!("{}B", base)))
    }
}

/// One tube: control points with a radius at each; its generation level (0 stem, 1+ branches).
#[derive(Clone, Debug)]
pub struct Tube {
    pub pts: Vec<Vec3>,
    pub radii: Vec<f32>,
    pub level: u32,
}

/// A leaf or petal: a flat quad.
#[derive(Clone, Debug)]
pub struct Blade {
    pub quad: [Vec3; 4],
    pub flower: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Growth {
    pub tubes: Vec<Tube>,
    pub blades: Vec<Blade>,
}

struct Rng(u64);

impl Rng {
    fn new(seed: u32) -> Rng {
        Rng((seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
    fn unit(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(self.range(-1.0, 1.0), self.range(-1.0, 1.0), self.range(-1.0, 1.0));
            let l = v.length_sq();
            if l > 1e-4 && l <= 1.0 {
                return v * (1.0 / l.sqrt());
            }
        }
    }
}

fn perp(v: Vec3) -> Vec3 {
    let h = if v.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
    v.cross(h).normalized()
}

/// Grow the tubes of a stroke. `path` holds world positions and surface normals; `percent` keeps that share of the
/// tubes; growth stops at `max_tubes`.
pub fn grow(p: &Params, path: &[(Vec3, Vec3)], seed: u32, percent: f32, max_tubes: usize) -> Growth {
    let mut g = Growth::default();
    if path.is_empty() {
        return g;
    }
    let mut rng = Rng::new(seed);
    let scale = p.g("globalScale").max(1e-3);
    let width = p.g("brushWidth").max(0.0) * scale;
    let spacing = (p.g("spacing") * width.max(0.05)).max(0.01 * scale);
    // sample points along the path at the spacing (always the first one)
    let mut steps: Vec<(Vec3, Vec3)> = vec![path[0]];
    let mut carry = 0.0f32;
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = a.0.distance(b.0);
        let mut d = spacing - carry;
        while d <= len {
            let f = if len > 1e-9 { d / len } else { 0.0 };
            steps.push((a.0.lerp(b.0, f), a.1.lerp(b.1, f).normalized()));
            d += spacing;
        }
        carry = (carry + len) % spacing;
    }
    let per = p.g("tubesPerStep").round().max(1.0) as usize;
    let keep = (percent / 100.0).clamp(0.0, 1.0);
    let mut stems = 0usize;
    for (pos, nrm) in &steps {
        for _ in 0..per {
            stems += 1;
            // the displayed share is every n-th tube so the whole stroke thins out evenly
            if keep < 1.0 && (stems as f32 * keep).floor() == ((stems - 1) as f32 * keep).floor() {
                continue;
            }
            if g.tubes.len() >= max_tubes {
                return g;
            }
            let (t1, t2) = (perp(*nrm), nrm.cross(perp(*nrm)).normalized());
            let jitter = {
                let a = rng.range(0.0, std::f32::consts::TAU);
                let r = width * 0.5 * rng.next().sqrt();
                t1 * (a.cos() * r) + t2 * (a.sin() * r)
            };
            let dir0 = (*nrm + rng.unit() * p.g("tubeRand")).normalized();
            let len = rng.range(p.g("lengthMin"), p.g("lengthMax").max(p.g("lengthMin"))) * scale;
            grow_tube(p, &mut g, &mut rng, *pos + jitter, dir0, len, p.g("tubeWidth1") * scale, p.g("tubeWidth2") * scale, 0, max_tubes);
        }
    }
    g
}

#[allow(clippy::too_many_arguments)]
fn grow_tube(p: &Params, g: &mut Growth, rng: &mut Rng, start: Vec3, dir0: Vec3, len: f32, w1: f32, w2: f32, level: u32, max_tubes: usize) {
    if g.tubes.len() >= max_tubes {
        return;
    }
    let n = (p.g("segments").round().max(1.0) as usize).min(24);
    let step = len / n as f32;
    let wr = p.g("widthRand");
    let w1 = w1 * (1.0 + rng.range(-wr, wr));
    let mut pts = vec![start];
    let mut radii = vec![w1 * 0.5];
    let mut dir = dir0;
    let curl_axis = perp(dir0);
    let wind = p.g("wind");
    for i in 1..=n {
        let f = i as f32 / n as f32;
        // gravity bends the tube down, curl turns it about a fixed axis, a little noise keeps it organic
        dir = (dir + Vec3::new(0.0, -1.0, 0.0) * (p.g("gravity") * f) + rng.unit() * 0.04 + Vec3::new(wind * 0.02, 0.0, 0.0)).normalized();
        if p.g("curl").abs() > 1e-4 {
            dir = Quat::from_axis_angle(curl_axis, p.g("curl") * 0.2).rotate(dir).normalized();
        }
        let last = *pts.last().unwrap();
        pts.push(last + dir * step);
        radii.push((w1 + (w2 - w1) * f).max(1e-4) * 0.5);
    }
    // leaves and flowers at the tip of the last generation
    let max_level = if p.g("branches") > 0.5 { p.g("branchLevels").round().max(1.0) as u32 } else { 0 };
    let tip = *pts.last().unwrap();
    let tube = Tube { pts: pts.clone(), radii: radii.clone(), level };
    g.tubes.push(tube);
    if level < max_level {
        let count = p.g("branchCount").round().max(0.0) as usize;
        for b in 0..count {
            // branches leave from the upper part of the tube, spread around it
            let f = p.g("branchStart") + (1.0 - p.g("branchStart")) * (b as f32 + rng.next()) / count.max(1) as f32;
            let idx = ((f * n as f32) as usize).clamp(1, n);
            let at = pts[idx];
            let along = (pts[idx] - pts[idx - 1]).normalized();
            let side = Quat::from_axis_angle(along, rng.range(0.0, std::f32::consts::TAU)).rotate(perp(along));
            let ang = p.g("branchAngle").to_radians() * rng.range(0.8, 1.2);
            let bdir = (along * ang.cos() + side * ang.sin()).normalized();
            let blen = len * p.g("branchLength") * rng.range(0.8, 1.1);
            grow_tube(p, g, rng, at, bdir, blen, radii[idx] * 1.6, radii[idx] * 0.4, level + 1, max_tubes);
        }
    } else {
        if p.g("leaves") > 0.5 {
            let c = p.g("leafCount").round().max(1.0) as usize;
            let sz = p.g("leafSize") * p.g("globalScale");
            for _ in 0..c {
                let at = tip + rng.unit() * sz * 0.5;
                add_blade(g, rng, at, sz, false);
            }
        }
        if p.g("flowers") > 0.5 {
            let sz = p.g("flowerSize") * p.g("globalScale");
            let up = (tip - pts[pts.len().saturating_sub(2)]).normalized();
            let (a, b) = (perp(up), up.cross(perp(up)).normalized());
            for k in 0..8 {
                let ang = std::f32::consts::TAU * k as f32 / 8.0;
                let out = a * ang.cos() + b * ang.sin();
                let (l, side) = (out * sz, up.cross(out).normalized() * sz * 0.18);
                g.blades.push(Blade { quad: [tip, tip + l * 0.5 + side, tip + l, tip + l * 0.5 - side], flower: true });
            }
        }
    }
}

fn add_blade(g: &mut Growth, rng: &mut Rng, at: Vec3, size: f32, flower: bool) {
    let a = rng.unit();
    let b = perp(a);
    let c = a.cross(b).normalized();
    g.blades.push(Blade { quad: [at, at + b * size * 0.5 + a * size * 0.3, at + a * size, at - b * size * 0.5 + a * size * 0.3], flower });
    let _ = c;
}

/// Mesh of a growth: faces of tubes in zone 0, leaves in zone 1 and flowers in zone 2.
pub fn build_mesh(p: &Params, g: &Growth) -> (Mesh, Vec<u16>) {
    let sides = (p.g("sides").round().max(3.0) as usize).min(12);
    let mut m = Mesh::new();
    let mut zone: Vec<u16> = Vec::new();
    for t in &g.tubes {
        let n = t.pts.len();
        if n < 2 {
            continue;
        }
        let mut rings: Vec<Vec<u32>> = Vec::new();
        let mut frame = perp((t.pts[1] - t.pts[0]).normalized());
        for i in 0..n {
            let tangent = if i + 1 < n { (t.pts[i + 1] - t.pts[i]).normalized() } else { (t.pts[i] - t.pts[i - 1]).normalized() };
            // parallel transport of the frame along the tube
            frame = (frame - tangent * tangent.dot(frame)).normalized();
            if frame.length_sq() < 1e-8 {
                frame = perp(tangent);
            }
            let other = tangent.cross(frame).normalized();
            let r = if i + 1 == n { t.radii[i] * 0.15 } else { t.radii[i] };
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let a = std::f32::consts::TAU * s as f32 / sides as f32;
                let off = frame * (a.cos() * r) + other * (a.sin() * r);
                ring.push(m.add_vertex(t.pts[i] + off, off.normalized(), Vec2::new(s as f32 / sides as f32, i as f32 / (n - 1) as f32)));
            }
            rings.push(ring);
        }
        for i in 0..n - 1 {
            for s in 0..sides {
                let s2 = (s + 1) % sides;
                m.add_face(&[rings[i][s], rings[i][s2], rings[i + 1][s2], rings[i + 1][s]]);
                zone.push(0);
            }
        }
    }
    for b in &g.blades {
        let ids: Vec<u32> = b.quad.iter().enumerate().map(|(k, q)| m.add_vertex(*q, Vec3::UP, Vec2::new((k % 2) as f32, (k / 2) as f32))).collect();
        m.add_face(&ids);
        // seen from both sides
        let rev = [ids[3], ids[2], ids[1], ids[0]];
        m.add_face(&rev);
        let z = if b.flower { 2 } else { 1 };
        zone.push(z);
        zone.push(z);
    }
    m.recompute_normals();
    (m, zone)
}

/// Colors of a brush: stem, leaf and flower.
pub fn colors(p: &Params) -> [Vec3; 3] {
    [p.col("color1"), p.col("leafColor"), p.col("flowerColor")]
}

/// Linear blend of two sets of brush values (Blend Brush): every channel moves `amount` toward `b`.
pub fn blend(a: &[(String, f32)], b: &[(String, f32)], amount: f32, only_shading: bool) -> Vec<(String, f32)> {
    let shading = |k: &str| k.starts_with("color") || k.starts_with("leafColor") || k.starts_with("flowerColor");
    a.iter()
        .map(|(k, v)| {
            let other = b.iter().find(|(n, _)| n == k).map(|(_, x)| *x).unwrap_or(*v);
            if only_shading == shading(k) {
                (k.clone(), v + (other - v) * amount.clamp(0.0, 1.0))
            } else {
                (k.clone(), *v)
            }
        })
        .collect()
}

/// A 2D rendering of a growth into an RGBA canvas: the stroke path lies in the x / y plane, y up.
pub fn rasterize(canvas: &mut [[f32; 4]], w: usize, h: usize, p: &Params, g: &Growth) {
    let cols = colors(p);
    let c2 = p.col("color2");
    let proj = |v: Vec3| -> (f32, f32) { (v.x, h as f32 - v.y) };
    let mut tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32), col: Vec3| {
        let (minx, maxx) = (a.0.min(b.0).min(c.0).floor().max(0.0) as usize, (a.0.max(b.0).max(c.0).ceil() as i64).clamp(0, w as i64 - 1) as usize);
        let (miny, maxy) = (a.1.min(b.1).min(c.1).floor().max(0.0) as usize, (a.1.max(b.1).max(c.1).ceil() as i64).clamp(0, h as i64 - 1) as usize);
        let den = (b.1 - c.1) * (a.0 - c.0) + (c.0 - b.0) * (a.1 - c.1);
        if den.abs() < 1e-9 {
            return;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let l1 = ((b.1 - c.1) * (px - c.0) + (c.0 - b.0) * (py - c.1)) / den;
                let l2 = ((c.1 - a.1) * (px - c.0) + (a.0 - c.0) * (py - c.1)) / den;
                let l3 = 1.0 - l1 - l2;
                if l1 >= -0.01 && l2 >= -0.01 && l3 >= -0.01 {
                    canvas[y * w + x] = [col.x, col.y, col.z, 1.0];
                }
            }
        }
    };
    for t in &g.tubes {
        let n = t.pts.len();
        for i in 0..n.saturating_sub(1) {
            let f = i as f32 / (n - 1) as f32;
            let col = cols[0].lerp(c2, f);
            let (a, b) = (proj(t.pts[i]), proj(t.pts[i + 1]));
            let d = (b.0 - a.0, b.1 - a.1);
            let l = (d.0 * d.0 + d.1 * d.1).sqrt().max(1e-6);
            let nrm = (-d.1 / l, d.0 / l);
            let (r0, r1) = (t.radii[i].max(0.5), t.radii[i + 1].max(0.5));
            let (a1, a2) = ((a.0 + nrm.0 * r0, a.1 + nrm.1 * r0), (a.0 - nrm.0 * r0, a.1 - nrm.1 * r0));
            let (b1, b2) = ((b.0 + nrm.0 * r1, b.1 + nrm.1 * r1), (b.0 - nrm.0 * r1, b.1 - nrm.1 * r1));
            tri(a1, a2, b1, col);
            tri(a2, b2, b1, col);
        }
    }
    for b in &g.blades {
        let q: Vec<(f32, f32)> = b.quad.iter().map(|v| proj(*v)).collect();
        let col = if b.flower { cols[2] } else { cols[1] };
        tri(q[0], q[1], q[2], col);
        tri(q[0], q[2], q[3], col);
    }
}
