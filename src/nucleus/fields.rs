//! Dynamic fields (gravity, air, drag, newton, radial, turbulence, uniform, vortex) and the particle step.

use super::particles::Rng;
use super::rigid::TriMesh;
use crate::math::{Mat4, Vec3};

pub const FIELD_KINDS: &[&str] = &["gravity", "air", "drag", "newton", "radial", "turbulence", "uniform", "vortex"];
pub const VOLUMES: &[&str] = &["none", "cube", "sphere", "cylinder", "cone"];

/// A field evaluated in world space.
#[derive(Clone, Debug)]
pub struct Field {
    pub kind: u8,
    pub magnitude: f32,
    pub attenuation: f32,
    pub dir: Vec3,
    pub pos: Vec3,
    pub max_dist: Option<f32>,
    pub volume: u8,
    /// World to field space, for volume shapes (the transform's scale sizes the volume).
    pub inv: Mat4,
    pub speed: f32,
    pub frequency: f32,
    pub phase: f32,
    pub time: f32,
}

fn hash3(x: i32, y: i32, z: i32, k: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841) ^ (z as u32).wrapping_mul(0xcb1ab31f) ^ k.wrapping_mul(0x9e3779b9);
    h ^= h >> 13;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 32767.5 - 1.0
}

fn value_noise(p: Vec3, k: u32) -> f32 {
    let (ix, iy, iz) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    let (fx, fy, fz) = (p.x - ix as f32, p.y - iy as f32, p.z - iz as f32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy, sz) = (s(fx), s(fy), s(fz));
    let mut v = 0.0;
    for dx in 0..2 {
        for dy in 0..2 {
            for dz in 0..2 {
                let w = (if dx == 0 { 1.0 - sx } else { sx }) * (if dy == 0 { 1.0 - sy } else { sy }) * (if dz == 0 { 1.0 - sz } else { sz });
                v += w * hash3(ix + dx, iy + dy, iz + dz, k);
            }
        }
    }
    v
}

impl Field {
    pub fn new(kind: u8) -> Field {
        Field { kind, magnitude: 10.0, attenuation: 0.0, dir: Vec3::new(0.0, -1.0, 0.0), pos: Vec3::ZERO, max_dist: None, volume: 0, inv: Mat4::identity(), speed: 0.0, frequency: 1.0, phase: 0.0, time: 0.0 }
    }

    fn in_volume(&self, p: Vec3) -> bool {
        if self.volume == 0 {
            return true;
        }
        let l = self.inv.transform_point(p);
        match self.volume {
            1 => l.x.abs() <= 1.0 && l.y.abs() <= 1.0 && l.z.abs() <= 1.0,
            2 => l.length_sq() <= 1.0,
            3 => l.y.abs() <= 1.0 && l.x * l.x + l.z * l.z <= 1.0,
            _ => {
                // cone: apex at +y, base at -y
                let k = (1.0 - l.y) * 0.5;
                l.y.abs() <= 1.0 && l.x * l.x + l.z * l.z <= k * k
            }
        }
    }

    /// Acceleration of a particle or body of `mass` at `p` moving with `v`.
    pub fn accel(&self, p: Vec3, v: Vec3, mass: f32) -> Vec3 {
        if !self.in_volume(p) {
            return Vec3::ZERO;
        }
        let d_vec = p - self.pos;
        let d = d_vec.length();
        if let Some(m) = self.max_dist {
            if d > m && self.volume == 0 {
                return Vec3::ZERO;
            }
        }
        // smooth fall-off toward the maximum distance, or a power law for the radial kinds
        let mut fac = 1.0;
        if let (Some(m), true) = (self.max_dist, self.attenuation > 0.0) {
            fac *= (1.0 - d / m.max(1e-4)).clamp(0.0, 1.0).powf(self.attenuation);
        }
        let inv_mass = 1.0 / mass.max(1e-4);
        let m = self.magnitude * fac;
        match self.kind {
            // gravity is an acceleration: every body falls alike
            0 => self.dir.normalized() * m,
            // air: pushes toward the speed of the wind
            1 => {
                let wind = self.dir.normalized() * self.speed;
                (wind - v) * (m * 0.1 * inv_mass)
            }
            2 => -v * (m * 0.1 * inv_mass),
            // newton pulls toward the field, radial pushes away
            3 | 4 => {
                if d < 1e-4 {
                    return Vec3::ZERO;
                }
                let k = m * d.max(0.1).powf(-self.attenuation.max(0.0)) * inv_mass;
                let dir = d_vec * (1.0 / d);
                if self.kind == 3 {
                    -dir * k
                } else {
                    dir * k
                }
            }
            5 => {
                let q = p * self.frequency + Vec3::splat(self.phase * self.time);
                Vec3::new(value_noise(q, 1), value_noise(q, 2), value_noise(q, 3)) * (m * inv_mass)
            }
            6 => self.dir.normalized() * (m * inv_mass),
            _ => {
                // vortex about the field's direction axis
                let axis = self.dir.normalized();
                let r = d_vec - axis * axis.dot(d_vec);
                let rl = r.length();
                if rl < 1e-4 {
                    return Vec3::ZERO;
                }
                let k = m * rl.max(0.1).powf(-self.attenuation.max(0.0)) * inv_mass;
                axis.cross(r * (1.0 / rl)) * k
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct P {
    pub id: u32,
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    /// Seconds to live; 0 lives forever.
    pub life: f32,
    pub radius: f32,
    /// A collision event happened to this particle.
    pub evented: bool,
}

#[derive(Clone, Debug)]
pub struct PSim {
    pub items: Vec<P>,
    pub next_id: u32,
    pub rng: Rng,
}

impl Default for PSim {
    fn default() -> Self {
        PSim { items: Vec::new(), next_id: 0, rng: Rng::new(1) }
    }
}

impl PSim {
    pub fn spawn(&mut self, pos: Vec3, vel: Vec3, life: f32, radius: f32) {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(P { id, pos, vel, age: 0.0, life, radius, evented: false });
    }
}

/// A collision surface for particles in world space.
pub struct Collider {
    pub mesh: std::sync::Arc<TriMesh>,
    pub resilience: f32,
    pub friction: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub index: usize,
    pub pos: Vec3,
    pub vel: Vec3,
    pub normal: Vec3,
}

pub struct Params {
    pub conserve: f32,
    pub damping: f32,
    pub mass: f32,
    /// Radius used for collisions.
    pub radius: f32,
}

/// Integrate one substep: fields, conserve, motion, collisions, aging. Returns the collisions and the indices that died.
pub fn step(sim: &mut PSim, par: &Params, h: f32, fps: f32, accel: &dyn Fn(Vec3, Vec3) -> Vec3, colliders: &[Collider]) -> Vec<Hit> {
    let mut hits = Vec::new();
    let keep = par.conserve.clamp(0.0, 1.0).powf(h * fps);
    let damp = 1.0 / (1.0 + par.damping * h);
    for (i, p) in sim.items.iter_mut().enumerate() {
        p.vel += accel(p.pos, p.vel) * h;
        p.vel *= keep * damp;
        let from = p.pos;
        p.pos += p.vel * h;
        for c in colliders {
            let seg = p.pos - from;
            let mid = (from + p.pos) * 0.5;
            let reach = seg.length() * 0.5 + par.radius.max(0.001);
            let mut best: Option<(f32, Vec3, Vec3)> = None;
            for t in c.mesh.near_tris(mid, reach) {
                let tr = c.mesh.tris[t as usize];
                let (a, b, cc) = (c.mesh.verts[tr[0] as usize], c.mesh.verts[tr[1] as usize], c.mesh.verts[tr[2] as usize]);
                let n = c.mesh.normals[t as usize];
                // swept: the segment crosses the triangle plane inside the triangle
                let (da, db) = ((from - a).dot(n), (p.pos - a).dot(n));
                let mut hit: Option<(f32, Vec3)> = None;
                if (da > 0.0) != (db > 0.0) {
                    let f = da / (da - db);
                    let q = from + seg * f;
                    let cp = super::rigid::closest_point_triangle(q, a, b, cc);
                    if (cp - q).length_sq() < 1e-8 {
                        hit = Some((f, q));
                    }
                }
                if hit.is_none() {
                    // resting on or within the radius of the face
                    let cp = super::rigid::closest_point_triangle(p.pos, a, b, cc);
                    let d = (p.pos - cp).length();
                    if d < par.radius.max(0.001) {
                        hit = Some((1.0, cp));
                    }
                }
                if let Some((f, q)) = hit {
                    if best.map(|b| f < b.0).unwrap_or(true) {
                        let side = if (from - a).dot(n) >= 0.0 { n } else { -n };
                        best = Some((f, q, side));
                    }
                }
            }
            if let Some((_, q, n)) = best {
                let vn = p.vel.dot(n);
                let vt = p.vel - n * vn;
                let before = p.vel;
                let vn_new = if vn < 0.0 { -vn * c.resilience } else { vn };
                p.vel = vt * (1.0 - c.friction.clamp(0.0, 1.0)) + n * vn_new;
                p.pos = q + n * par.radius.max(0.001);
                if vn < -0.05 {
                    hits.push(Hit { index: i, pos: p.pos, vel: before, normal: n });
                }
            }
        }
        p.age += h;
    }
    hits
}
