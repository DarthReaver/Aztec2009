//! CPU ray tracer: BVH accelerated, hard shadows, mirror reflections, jittered anti-aliasing.
//! Uses the same render snapshot as the rasterizer so materials, lights and color maps match.

use super::{LightKind, RenderLight, RenderScene};
use crate::math::{Mat4, Vec2, Vec3, Vec4};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy)]
struct Tri {
    p: [Vec3; 3],
    n: [Vec3; 3],
    uv: [Vec2; 3],
    mat: u16,
    item: u32,
}

#[derive(Clone, Copy)]
struct Node {
    lo: Vec3,
    hi: Vec3,
    /// Leaf: first triangle and count. Inner: left child index (right is left + 1), count = 0.
    first: u32,
    count: u32,
}

struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<Tri>,
}

fn tri_bounds(t: &Tri) -> (Vec3, Vec3) {
    let mut lo = t.p[0];
    let mut hi = t.p[0];
    for q in &t.p[1..] {
        lo = Vec3::new(lo.x.min(q.x), lo.y.min(q.y), lo.z.min(q.z));
        hi = Vec3::new(hi.x.max(q.x), hi.y.max(q.y), hi.z.max(q.z));
    }
    (lo, hi)
}

impl Bvh {
    fn build(mut tris: Vec<Tri>) -> Bvh {
        let mut nodes: Vec<Node> = Vec::new();
        if tris.is_empty() {
            return Bvh { nodes, tris };
        }
        nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, first: 0, count: 0 });
        let n = tris.len();
        Self::split(&mut nodes, &mut tris, 0, 0, n);
        Bvh { nodes, tris }
    }

    fn split(nodes: &mut Vec<Node>, tris: &mut [Tri], idx: usize, start: usize, end: usize) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for t in &tris[start..end] {
            let (a, b) = tri_bounds(t);
            lo = Vec3::new(lo.x.min(a.x), lo.y.min(a.y), lo.z.min(a.z));
            hi = Vec3::new(hi.x.max(b.x), hi.y.max(b.y), hi.z.max(b.z));
        }
        let count = end - start;
        if count <= 4 {
            nodes[idx] = Node { lo, hi, first: start as u32, count: count as u32 };
            return;
        }
        let ext = hi - lo;
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        let key = |t: &Tri| -> f32 {
            let c = (t.p[0] + t.p[1] + t.p[2]) * (1.0 / 3.0);
            match axis {
                0 => c.x,
                1 => c.y,
                _ => c.z,
            }
        };
        tris[start..end].sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
        let mid = start + count / 2;
        let left = nodes.len();
        nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, first: 0, count: 0 });
        nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, first: 0, count: 0 });
        nodes[idx] = Node { lo, hi, first: left as u32, count: 0 };
        Self::split(nodes, tris, left, start, mid);
        Self::split(nodes, tris, left + 1, mid, end);
    }
}

#[derive(Clone, Copy)]
struct Hit {
    /// 0 triangle, 1 sphere particle, 2 blobby group
    kind: u8,
    t: f32,
    tri: usize,
    u: f32,
    v: f32,
}

fn ray_box(o: Vec3, inv: Vec3, lo: Vec3, hi: Vec3, tmax: f32) -> bool {
    let t1 = (lo.x - o.x) * inv.x;
    let t2 = (hi.x - o.x) * inv.x;
    let mut tmin = t1.min(t2);
    let mut tm = t1.max(t2);
    let t1 = (lo.y - o.y) * inv.y;
    let t2 = (hi.y - o.y) * inv.y;
    tmin = tmin.max(t1.min(t2));
    tm = tm.min(t1.max(t2));
    let t1 = (lo.z - o.z) * inv.z;
    let t2 = (hi.z - o.z) * inv.z;
    tmin = tmin.max(t1.min(t2));
    tm = tm.min(t1.max(t2));
    tm >= tmin.max(0.0) && tmin < tmax
}

fn ray_tri(o: Vec3, d: Vec3, t: &Tri) -> Option<(f32, f32, f32)> {
    let e1 = t.p[1] - t.p[0];
    let e2 = t.p[2] - t.p[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t.p[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let tt = e2.dot(q) * inv;
    if tt > 1e-4 {
        Some((tt, u, v))
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Ray {
    Primary,
    Shadow,
    Secondary,
}

/// What a render pass shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pass {
    Beauty,
    Diffuse,
    Specular,
    /// White where lit, dark where shadowed.
    Shadow,
    Reflection,
    /// The material color without lighting.
    Color,
}

struct Scene<'a> {
    pass: Pass,
    extent: f32,
    reflections: bool,
    bvh: Bvh,
    rs: &'a RenderScene,
    spheres: Vec<(Vec3, f32)>,
    /// Index into `rs.particles` of every sphere.
    sphere_idx: Vec<usize>,
    /// Blobby particles (position, radius, group) and the groups (threshold).
    blobs: Vec<(Vec3, f32, u32)>,
    groups: Vec<(u32, f32)>,
    bg_top: Vec3,
    bg_bottom: Vec3,
}

impl Scene<'_> {
    fn closest(&self, o: Vec3, d: Vec3, tmax: f32) -> Option<(Hit, bool)> {
        self.closest_ex(o, d, tmax, Ray::Secondary)
    }

    fn closest_ex(&self, o: Vec3, d: Vec3, tmax: f32, kind: Ray) -> Option<(Hit, bool)> {
        let inv = Vec3::new(
            if d.x.abs() > 1e-12 { 1.0 / d.x } else { 1e12 },
            if d.y.abs() > 1e-12 { 1.0 / d.y } else { 1e12 },
            if d.z.abs() > 1e-12 { 1.0 / d.z } else { 1e12 },
        );
        let mut best: Option<Hit> = None;
        let mut tbest = tmax;
        if !self.bvh.nodes.is_empty() {
            let mut stack = [0u32; 64];
            let mut sp = 1;
            stack[0] = 0;
            while sp > 0 {
                sp -= 1;
                let n = &self.bvh.nodes[stack[sp] as usize];
                if !ray_box(o, inv, n.lo, n.hi, tbest) {
                    continue;
                }
                if n.count > 0 {
                    for i in n.first as usize..(n.first + n.count) as usize {
                        let flags = self.rs.items.get(self.bvh.tris[i].item as usize).map(|it| it.flags).unwrap_or(255);
                        let skip = match kind {
                            Ray::Primary => flags & super::F_PRIMARY == 0,
                            Ray::Shadow => flags & super::F_CAST == 0,
                            Ray::Secondary => flags & super::F_REFLECT == 0,
                        };
                        if skip {
                            continue;
                        }
                        if let Some((t, u, v)) = ray_tri(o, d, &self.bvh.tris[i]) {
                            if t < tbest {
                                tbest = t;
                                best = Some(Hit { kind: 0, t, tri: i, u, v });
                            }
                        }
                    }
                } else if sp + 2 <= stack.len() {
                    stack[sp] = n.first;
                    stack[sp + 1] = n.first + 1;
                    sp += 2;
                }
            }
        }
        // particles are spheres
        let mut sphere_hit = false;
        let mut non_tri = false;
        for (k, (c, r)) in self.spheres.iter().enumerate() {
            let oc = o - *c;
            let b = oc.dot(d);
            let cc = oc.dot(oc) - r * r;
            let disc = b * b - cc;
            if disc > 0.0 {
                let t = -b - disc.sqrt();
                if t > 1e-4 && t < tbest {
                    tbest = t;
                    best = Some(Hit { kind: 1, t, tri: k, u: 0.0, v: 0.0 });
                    sphere_hit = true;
                    non_tri = true;
                }
            }
        }
        let _ = sphere_hit;
        for (gi, (g, thr)) in self.groups.iter().enumerate() {
            if let Some(t) = self.blob_hit(*g, *thr, o, d, tbest) {
                if t < tbest {
                    tbest = t;
                    best = Some(Hit { kind: 2, t, tri: gi, u: 0.0, v: 0.0 });
                    non_tri = true;
                }
            }
        }
        best.map(|h| (h, non_tri && h.kind != 0))
    }

    /// Metaball field of a group: the sum of smooth kernels of the particles near `p` (two per blob at its center).
    fn blob_field(&self, group: u32, p: Vec3, near: &[usize]) -> f32 {
        let mut f = 0.0;
        for &i in near {
            let (c, r, g) = self.blobs[i];
            if g != group {
                continue;
            }
            let s = 2.0 * r;
            let d2 = (p - c).length_sq();
            if d2 < s * s {
                let w = 1.0 - d2 / (s * s);
                f += 2.0 * w * w * w;
            }
        }
        f
    }

    /// First hit of the blobby surface (field = threshold) of a group along the ray.
    fn blob_hit(&self, group: u32, thr: f32, o: Vec3, d: Vec3, tmax: f32) -> Option<f32> {
        let mut near: Vec<usize> = Vec::new();
        let (mut t0, mut t1) = (f32::INFINITY, 0.0f32);
        let mut min_r = f32::INFINITY;
        for (i, (c, r, g)) in self.blobs.iter().enumerate() {
            if *g != group {
                continue;
            }
            let s = 2.0 * r;
            let oc = o - *c;
            let b = oc.dot(d);
            let disc = b * b - (oc.dot(oc) - s * s);
            if disc > 0.0 {
                let sq = disc.sqrt();
                let (a, z) = ((-b - sq).max(0.0), -b + sq);
                if z > 0.0 {
                    near.push(i);
                    t0 = t0.min(a);
                    t1 = t1.max(z);
                    min_r = min_r.min(*r);
                }
            }
        }
        if near.is_empty() {
            return None;
        }
        let t1 = t1.min(tmax);
        let step = (min_r * 0.2).max(1e-3);
        let mut t = t0;
        let mut prev = self.blob_field(group, o + d * t, &near);
        if prev >= thr {
            return Some(t.max(1e-4));
        }
        while t < t1 {
            let tn = (t + step).min(t1);
            let f = self.blob_field(group, o + d * tn, &near);
            if f >= thr {
                // bisect between t and tn
                let (mut a, mut b) = (t, tn);
                for _ in 0..10 {
                    let m = 0.5 * (a + b);
                    if self.blob_field(group, o + d * m, &near) >= thr {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                return Some(b);
            }
            prev = f;
            t = tn;
        }
        let _ = prev;
        None
    }

    /// Normal of the blobby surface at `p`: against the gradient of the field.
    fn blob_normal(&self, group: u32, p: Vec3) -> Vec3 {
        let near: Vec<usize> = (0..self.blobs.len()).collect();
        let e = 1e-3;
        let f = |q: Vec3| self.blob_field(group, q, &near);
        let g = Vec3::new(f(p + Vec3::new(e, 0.0, 0.0)) - f(p - Vec3::new(e, 0.0, 0.0)), f(p + Vec3::new(0.0, e, 0.0)) - f(p - Vec3::new(0.0, e, 0.0)), f(p + Vec3::new(0.0, 0.0, e)) - f(p - Vec3::new(0.0, 0.0, e)));
        let n = -g;
        if n.length_sq() < 1e-14 {
            Vec3::UP
        } else {
            n.normalized()
        }
    }

    fn occluded(&self, o: Vec3, d: Vec3, tmax: f32) -> bool {
        self.closest_ex(o, d, tmax, Ray::Shadow).is_some()
    }

    /// Fraction of the light that reaches `origin`: a single hard shadow ray, or several rays toward
    /// points spread over the light (its radius, or the area light's rectangle) for feathered edges.
    fn visibility(&self, l: &RenderLight, origin: Vec3, ldir: Vec3, dist: f32) -> f32 {
        if !l.shadows {
            return 1.0;
        }
        let area = l.kind == LightKind::Area;
        let dm = l.dm_blur;
        if l.softness <= 1e-6 && !area && dm <= 1e-6 {
            return if self.occluded(origin, ldir, dist) { 0.0 } else { 1.0 };
        }
        let n = if area { l.samples.max(4) } else { l.samples }.clamp(1, 64);
        // two perpendicular vectors to spread the samples over
        let up = if ldir.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
        let t = ldir.cross(up).normalized();
        let b = ldir.cross(t);
        // a per-point offset turns banding into noise; the sequence below covers the light evenly
        let h = (origin.x.to_bits() ^ origin.y.to_bits().rotate_left(11) ^ origin.z.to_bits().rotate_left(21)).wrapping_mul(2654435761);
        let off = (h >> 8) as f32 / (1u32 << 24) as f32;
        let mut lit = 0.0;
        for i in 0..n {
            let u = (off + i as f32 * 0.754_877_7).fract();
            let v = (off * 0.5 + i as f32 * 0.569_840_3).fract();
            let (dir, tmax) = match l.kind {
                LightKind::Directional => {
                    let r = u.sqrt() * (l.softness.to_radians().tan() + dm);
                    let phi = v * std::f32::consts::TAU;
                    ((ldir + t * (r * phi.cos()) + b * (r * phi.sin())).normalized(), f32::INFINITY)
                }
                LightKind::Area => {
                    let p = l.pos + l.area_x * (2.0 * u - 1.0) + l.area_y * (2.0 * v - 1.0);
                    let d = p - origin;
                    (d.normalized(), d.length())
                }
                _ => {
                    let phi = v * std::f32::consts::TAU;
                    let r = u.sqrt() * (l.softness + dm * self.extent);
                    let p = l.pos + t * (r * phi.cos()) + b * (r * phi.sin());
                    let d = p - origin;
                    (d.normalized(), d.length())
                }
            };
            if !self.occluded(origin, dir, tmax) {
                lit += 1.0;
            }
        }
        lit / n as f32
    }

    fn background(&self, d: Vec3) -> Vec3 {
        let t = (d.y * 0.5 + 0.5).clamp(0.0, 1.0);
        self.bg_bottom.lerp(self.bg_top, t)
    }

    fn lights(&self, pos: Vec3, eye: Vec3) -> Vec<RenderLight> {
        if !self.rs.lights.is_empty() {
            return self.rs.lights.clone();
        }
        let to_eye = (eye - pos).normalized();
        vec![
            RenderLight::simple(LightKind::Directional, Vec3::ZERO, to_eye, Vec3::ONE, 0.85),
            RenderLight::simple(LightKind::Directional, Vec3::ZERO, Vec3::new(-0.4, 0.8, 0.3).normalized(), Vec3::new(0.6, 0.65, 0.85), 0.35),
        ]
    }

    fn trace(&self, o: Vec3, d: Vec3, depth: u32) -> Vec3 {
        match self.closest(o, d, f32::INFINITY) {
            Some((hit, is_sphere)) => self.shade_hit(o, d, depth, hit, is_sphere).0,
            None => self.background(d),
        }
    }

    /// Color and opacity of the surface a ray hit.
    fn shade_hit(&self, o: Vec3, d: Vec3, depth: u32, hit: Hit, is_sphere: bool) -> (Vec3, f32) {
        let mut particle_color: Option<Vec3> = None;
        let (pos, mut n, uv, mi, item) = if is_sphere && hit.kind == 2 {
            let p = o + d * hit.t;
            let g = self.groups[hit.tri].0;
            // the material of the nearest blob of the group
            let near = self.blobs.iter().enumerate().filter(|(_, b)| b.2 == g).min_by(|a, b| (a.1 .0 - p).length_sq().partial_cmp(&(b.1 .0 - p).length_sq()).unwrap()).map(|(i, _)| i);
            let src = near.and_then(|_| self.rs.particles.iter().find(|q| q.kind == 2 && q.group == g));
            if let Some(q) = src {
                if q.color.x >= 0.0 && q.material == super::PARTICLE_MATERIAL {
                    particle_color = Some(q.color);
                }
            }
            (p, self.blob_normal(g, p), Vec2::new(0.0, 0.0), src.map(|q| q.material as usize).unwrap_or(super::PARTICLE_MATERIAL as usize), None)
        } else if is_sphere {
            let (c, _r) = self.spheres[hit.tri];
            let p = o + d * hit.t;
            let src = &self.rs.particles[self.sphere_idx[hit.tri]];
            if src.color.x >= 0.0 && src.material == super::PARTICLE_MATERIAL {
                particle_color = Some(src.color);
            }
            (p, (p - c).normalized(), Vec2::new(0.0, 0.0), src.material as usize, None)
        } else {
            let t = &self.bvh.tris[hit.tri];
            let w = 1.0 - hit.u - hit.v;
            let n = (t.n[0] * w + t.n[1] * hit.u + t.n[2] * hit.v).normalized();
            let uv = Vec2::new(
                t.uv[0].x * w + t.uv[1].x * hit.u + t.uv[2].x * hit.v,
                t.uv[0].y * w + t.uv[1].y * hit.u + t.uv[2].y * hit.v,
            );
            (o + d * hit.t, n, uv, t.mat as usize, self.rs.items.get(t.item as usize))
        };
        let view = -d;
        if n.dot(view) < 0.0 {
            n = -n;
        }
        // bump map: the brightness of the connected texture tilts the normal
        if self.rs.use_textures && !is_sphere {
            if let (Some(Some(bump)), Some(m0)) = (self.rs.bumps.get(mi), self.rs.materials.get(mi)) {
                if m0.bump_depth.abs() > 1e-6 {
                    let t = &self.bvh.tris[hit.tri];
                    let (e1, e2) = (t.p[1] - t.p[0], t.p[2] - t.p[0]);
                    let (du1, dv1, du2, dv2) = (t.uv[1].x - t.uv[0].x, t.uv[1].y - t.uv[0].y, t.uv[2].x - t.uv[0].x, t.uv[2].y - t.uv[0].y);
                    let det = du1 * dv2 - du2 * dv1;
                    if det.abs() > 1e-12 {
                        let tu = (e1 * dv2 - e2 * dv1) * (1.0 / det);
                        let tv = (e2 * du1 - e1 * du2) * (1.0 / det);
                        let h = |u: f32, v: f32| {
                            let c = bump.sample(u, v);
                            c.x * 0.3 + c.y * 0.59 + c.z * 0.11
                        };
                        let eps = 0.004;
                        let (dhu, dhv) = ((h(uv.x + eps, uv.y) - h(uv.x - eps, uv.y)) / (2.0 * eps), (h(uv.x, uv.y + eps) - h(uv.x, uv.y - eps)) / (2.0 * eps));
                        // gradient of the height over the surface, per world unit
                        let grad = tu.normalized() * (dhu * tu.length_sq().sqrt().recip().max(1e-6)) + tv.normalized() * (dhv * tv.length_sq().sqrt().recip().max(1e-6));
                        let nn = (n - grad * (m0.bump_depth * 0.3)).normalized();
                        if nn.length_sq() > 0.5 {
                            n = nn;
                        }
                    }
                }
            }
        }
        let m = &self.rs.materials[mi.min(self.rs.materials.len() - 1)];
        let mut base = particle_color.unwrap_or(m.color);
        let spec_override = if self.rs.use_textures { self.rs.spec_maps.get(mi).and_then(|t| t.as_ref()).map(|t| t.sample(uv.x, uv.y)) } else { None };
        if self.rs.use_textures {
            if let Some(Some(tex)) = self.rs.textures.get(mi) {
                base = tex.sample(uv.x, uv.y);
            }
        }
        // transparency: a constant, or the alpha channel of the color texture
        let mut transp = m.transparency;
        if self.rs.use_textures {
            if let Some(Some(tex)) = self.rs.textures.get(mi) {
                if tex.alpha_transparency {
                    transp = 1.0 - tex.alpha(uv.x, uv.y);
                }
            }
        }
        let alpha = (1.0 - transp).clamp(0.0, 1.0);
        let pass = if depth == 0 { self.pass } else { Pass::Beauty };
        if pass == Pass::Color {
            return (base, alpha);
        }
        // passes shade a changed copy of the material
        let mut mp;
        let mut m = m;
        match pass {
            Pass::Diffuse => {
                mp = m.clone();
                mp.specular = 0.0;
                m = &mp;
            }
            Pass::Specular => {
                mp = m.clone();
                mp.diffuse = 0.0;
                mp.ambient = 0.0;
                mp.incandescence = Vec3::ZERO;
                m = &mp;
            }
            _ => {}
        }
        // a specular color map
        let mut ms;
        if let Some(sc) = spec_override {
            ms = m.clone();
            ms.spec_color = sc;
            m = &ms;
        }
        let origin = pos + n * 1e-3;
        let lights = self.lights(pos, o);
        let receive = item.map(|i| i.flags & super::F_RECEIVE != 0).unwrap_or(true);
        let lit_by = |l: &RenderLight| -> bool {
            match (&l.links, item) {
                (Some(objs), Some(i)) => objs.contains(&i.node) || objs.contains(&i.owner),
                _ => true,
            }
        };
        if pass == Pass::Shadow {
            let mut sum = 0.0;
            let mut count = 0.0;
            for l in lights.iter().filter(|l| l.kind != LightKind::Ambient && l.shadows && lit_by(l)) {
                if let Some((ldir, dist, _)) = l.incident(pos) {
                    sum += if receive { self.visibility(l, origin, ldir, dist) } else { 1.0 };
                    count += 1.0;
                }
            }
            let v = if count > 0.0 { sum / count } else { 1.0 };
            return (Vec3::splat(v), alpha);
        }
        let mut col = if pass == Pass::Reflection {
            Vec3::ZERO
        } else {
            super::shade_surface(m, base, n, view, pos, &lights, &|l, ldir, dist| {
                if !lit_by(l) {
                    return None;
                }
                let vis = if receive { self.visibility(l, origin, ldir, dist) } else { 1.0 };
                // shadowed light is replaced by the light's shadow color
                let tint = Vec3::splat(vis) + l.shadow_color * (1.0 - vis);
                if tint.length_sq() < 1e-8 {
                    None
                } else {
                    Some(tint)
                }
            })
        };
        if self.reflections && m.reflectivity > 0.001 && depth < 3 && pass != Pass::Diffuse && pass != Pass::Specular {
            let r = d.reflect(n).normalized();
            let rc = self.trace(origin, r, depth + 1);
            col = if pass == Pass::Reflection { rc * m.reflectivity } else { col * (1.0 - m.reflectivity) + rc * m.reflectivity };
        }
        if self.reflections && transp > 0.002 && depth < 6 && pass == Pass::Beauty {
            let behind = self.trace(pos + d * 1e-3, d, depth + 1);
            col = col * (1.0 - transp) + behind * transp;
        }
        (col, alpha)
    }
}

pub struct Options {
    /// Samples per pixel along each axis (1 = no anti-aliasing, 3 = 9 samples).
    pub grid: usize,
    pub bg_top: Vec3,
    pub bg_bottom: Vec3,
    pub pass: Pass,
    /// Mirror reflections and refraction (off for the hardware style render).
    pub reflections: bool,
    /// Render only this pixel rectangle (x0, y0, x1, y1; end exclusive).
    pub region: Option<(usize, usize, usize, usize)>,
}

impl Default for Options {
    fn default() -> Self {
        Options { grid: 2, bg_top: Vec3::new(0.36, 0.40, 0.46), bg_bottom: Vec3::new(0.14, 0.15, 0.17), pass: Pass::Beauty, reflections: true, region: None }
    }
}

/// Everything one render produces per pixel.
pub struct Buffers {
    pub w: usize,
    pub h: usize,
    /// Gamma corrected color.
    pub rgb: Vec<Vec3>,
    /// Coverage 0..1 (0 where only the background shows).
    pub alpha: Vec<f32>,
    /// Distance along the view ray to the first surface, infinity for the background.
    pub depth: Vec<f32>,
    /// Index into `RenderScene::items` of the first surface (u32::MAX: none or a particle).
    pub item: Vec<u32>,
    /// World position of the first surface.
    pub pos: Vec<Vec3>,
    /// World normal of the first surface.
    pub normal: Vec<Vec3>,
}

/// Render the snapshot. `progress` is incremented by the number of finished rows.
pub fn raytrace(rs: &RenderScene, view: &Mat4, proj: &Mat4, w: usize, h: usize, opt: &Options, progress: Option<&AtomicUsize>) -> Vec<Vec3> {
    raytrace_buffers(rs, view, proj, w, h, opt, progress).rgb
}

pub fn raytrace_buffers(rs: &RenderScene, view: &Mat4, proj: &Mat4, w: usize, h: usize, opt: &Options, progress: Option<&AtomicUsize>) -> Buffers {
    let mut tris: Vec<Tri> = Vec::new();
    for (ii, item) in rs.items.iter().enumerate() {
        let world = item.world;
        let m = &*item.mesh;
        let wp: Vec<Vec3> = m.positions.iter().map(|p| world.transform_point(*p)).collect();
        let wn: Vec<Vec3> = m.normals.iter().map(|n| world.transform_dir(*n).normalized()).collect();
        for f in &m.faces {
            for k in 1..f.len().saturating_sub(1) {
                let ids = [f[0] as usize, f[k] as usize, f[k + 1] as usize];
                tris.push(Tri {
                    p: [wp[ids[0]], wp[ids[1]], wp[ids[2]]],
                    n: [wn[ids[0]], wn[ids[1]], wn[ids[2]]],
                    uv: [m.uvs.get(ids[0]).copied().unwrap_or_default(), m.uvs.get(ids[1]).copied().unwrap_or_default(), m.uvs.get(ids[2]).copied().unwrap_or_default()],
                    mat: item.material,
                    item: ii as u32,
                });
            }
        }
    }
    let bvh = Bvh::build(tris);
    let extent = bvh.nodes.first().map(|n| (n.hi - n.lo).length()).unwrap_or(10.0).max(1.0);
    let scene = Scene { pass: opt.pass, extent, reflections: opt.reflections, bvh, rs, spheres: rs.particles.iter().filter(|p| p.kind == 1).map(|p| (p.pos, p.radius)).collect(), sphere_idx: rs.particles.iter().enumerate().filter(|(_, p)| p.kind == 1).map(|(i, _)| i).collect(), blobs: rs.particles.iter().filter(|p| p.kind == 2).map(|p| (p.pos, p.radius, p.group)).collect(), groups: { let mut g: Vec<(u32, f32)> = Vec::new(); for p in rs.particles.iter().filter(|p| p.kind == 2) { if !g.iter().any(|x| x.0 == p.group) { g.push((p.group, p.threshold)); } } g }, bg_top: opt.bg_top, bg_bottom: opt.bg_bottom };
    let inv = (*proj * *view).inverse();
    let unproject = |x: f32, y: f32, z: f32| -> Vec3 {
        let v = inv * Vec4::new(x, y, z, 1.0);
        if v.w.abs() > 1e-12 {
            v.xyz() / v.w
        } else {
            v.xyz()
        }
    };
    let grid = opt.grid.max(1);
    let (rx0, ry0, rx1, ry1) = opt.region.map(|(a, b, c, d)| (a.min(w), b.min(h), c.min(w), d.min(h))).unwrap_or((0, 0, w, h));
    let mut out = Buffers { w, h, rgb: vec![Vec3::ZERO; w * h], alpha: vec![0.0; w * h], depth: vec![f32::INFINITY; w * h], item: vec![u32::MAX; w * h], pos: vec![Vec3::ZERO; w * h], normal: vec![Vec3::ZERO; w * h] };
    struct Px {
        rgb: Vec3,
        alpha: f32,
        depth: f32,
        item: u32,
        pos: Vec3,
        normal: Vec3,
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16);
    let next = AtomicUsize::new(ry0);
    let rows: Vec<std::sync::Mutex<Vec<Px>>> = (0..h).map(|_| std::sync::Mutex::new(Vec::new())).collect();
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let y = next.fetch_add(1, Ordering::Relaxed);
                if y >= ry1 {
                    break;
                }
                let mut row = Vec::with_capacity(rx1 - rx0);
                for x in rx0..rx1 {
                    let mut acc = Vec3::ZERO;
                    let mut alpha = 0.0;
                    let mut first: Option<(f32, u32, Vec3, Vec3)> = None;
                    let mut center_best = f32::INFINITY;
                    for sy in 0..grid {
                        for sx in 0..grid {
                            let fx = x as f32 + (sx as f32 + 0.5) / grid as f32;
                            let fy = y as f32 + (sy as f32 + 0.5) / grid as f32;
                            let ndx = fx / w as f32 * 2.0 - 1.0;
                            let ndy = 1.0 - fy / h as f32 * 2.0;
                            let near = unproject(ndx, ndy, -1.0);
                            let far = unproject(ndx, ndy, 1.0);
                            let d = (far - near).normalized();
                            match scene.closest_ex(near, d, f32::INFINITY, Ray::Primary) {
                                Some((hit, is_sphere)) => {
                                    let (c, a) = scene.shade_hit(near, d, 0, hit, is_sphere);
                                    acc += c;
                                    alpha += a;
                                    if hit.t < center_best {
                                        center_best = hit.t;
                                        let (item, nrm) = if is_sphere {
                                            (u32::MAX, Vec3::UP)
                                        } else {
                                            let t = &scene.bvh.tris[hit.tri];
                                            let w0 = 1.0 - hit.u - hit.v;
                                            (t.item, (t.n[0] * w0 + t.n[1] * hit.u + t.n[2] * hit.v).normalized())
                                        };
                                        first = Some((hit.t, item, near + d * hit.t, nrm));
                                    }
                                }
                                None => {
                                    // pixels past the last surface show the background; alpha stays clear
                                    if opt.pass == Pass::Beauty || opt.pass == Pass::Color {
                                        acc += scene.background(d);
                                    } else if opt.pass == Pass::Shadow {
                                        acc += Vec3::ONE;
                                    }
                                }
                            }
                        }
                    }
                    let k = 1.0 / (grid * grid) as f32;
                    let c = acc * k;
                    let (depth, item, pos, normal) = first.map(|f| (f.0, f.1, f.2, f.3)).unwrap_or((f32::INFINITY, u32::MAX, Vec3::ZERO, Vec3::ZERO));
                    row.push(Px { rgb: Vec3::new(c.x.clamp(0.0, 1.0).powf(1.0 / 2.2), c.y.clamp(0.0, 1.0).powf(1.0 / 2.2), c.z.clamp(0.0, 1.0).powf(1.0 / 2.2)), alpha: alpha * k, depth, item, pos, normal });
                }
                if let Ok(mut r) = rows[y].lock() {
                    *r = row;
                }
                if let Some(p) = progress {
                    p.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    for (y, r) in rows.into_iter().enumerate() {
        if let Ok(r) = r.into_inner() {
            if r.len() == rx1 - rx0 {
                for (i, px) in r.into_iter().enumerate() {
                    let o = y * w + rx0 + i;
                    out.rgb[o] = px.rgb;
                    out.alpha[o] = px.alpha;
                    out.depth[o] = px.depth;
                    out.item[o] = px.item;
                    out.pos[o] = px.pos;
                    out.normal[o] = px.normal;
                }
            }
        }
    }
    out
}
