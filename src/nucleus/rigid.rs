//! Rigid body dynamics: spheres, boxes and static triangle meshes, sequential impulse contacts with friction and
//! restitution, point / hinge / spring joints and barrier planes. Pure geometry and physics, no scene access.

use crate::math::{Quat, Vec3};
use std::sync::Arc;

type M3 = [[f32; 3]; 3];

fn m3_mul_v(m: &M3, v: Vec3) -> Vec3 {
    Vec3::new(m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z, m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z, m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z)
}

fn m3_inv(m: &M3) -> M3 {
    let d = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if d.abs() < 1e-20 {
        return [[0.0; 3]; 3];
    }
    let i = 1.0 / d;
    [
        [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) * i, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * i, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * i],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) * i, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * i, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * i],
        [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) * i, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * i, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * i],
    ]
}

/// Cross product matrix [v]x.
fn skew(v: Vec3) -> M3 {
    [[0.0, -v.z, v.y], [v.z, 0.0, -v.x], [-v.y, v.x, 0.0]]
}

fn m3_mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn m3_t(a: &M3) -> M3 {
    [[a[0][0], a[1][0], a[2][0]], [a[0][1], a[1][1], a[2][1]], [a[0][2], a[1][2], a[2][2]]]
}

/// A triangle mesh with a uniform grid for point queries; vertices are in the body's local frame.
#[derive(Debug)]
pub struct TriMesh {
    pub verts: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    pub normals: Vec<Vec3>,
    pub lo: Vec3,
    pub hi: Vec3,
    cell: f32,
    dims: [usize; 3],
    grid: Vec<Vec<u32>>,
}

impl TriMesh {
    pub fn new(verts: Vec<Vec3>, tris: Vec<[u32; 3]>) -> TriMesh {
        let normals: Vec<Vec3> = tris.iter().map(|t| (verts[t[1] as usize] - verts[t[0] as usize]).cross(verts[t[2] as usize] - verts[t[0] as usize]).normalized()).collect();
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for v in &verts {
            lo = Vec3::new(lo.x.min(v.x), lo.y.min(v.y), lo.z.min(v.z));
            hi = Vec3::new(hi.x.max(v.x), hi.y.max(v.y), hi.z.max(v.z));
        }
        if verts.is_empty() {
            lo = Vec3::ZERO;
            hi = Vec3::ZERO;
        }
        let ext = hi - lo;
        let cell = ((ext.x.max(ext.y).max(ext.z)) / 24.0).max(1e-3);
        let dims = [((ext.x / cell) as usize + 1).min(48), ((ext.y / cell) as usize + 1).min(48), ((ext.z / cell) as usize + 1).min(48)];
        let mut m = TriMesh { verts, tris, normals, lo, hi, cell, dims, grid: Vec::new() };
        m.grid = vec![Vec::new(); dims[0] * dims[1] * dims[2]];
        for (ti, t) in m.tris.iter().enumerate() {
            let (mut a, mut b) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for k in t {
                let v = m.verts[*k as usize];
                a = Vec3::new(a.x.min(v.x), a.y.min(v.y), a.z.min(v.z));
                b = Vec3::new(b.x.max(v.x), b.y.max(v.y), b.z.max(v.z));
            }
            let (c0, c1) = (m.cell_of(a), m.cell_of(b));
            for x in c0[0]..=c1[0] {
                for y in c0[1]..=c1[1] {
                    for z in c0[2]..=c1[2] {
                        let i = m.cell_index(x, y, z);
                        m.grid[i].push(ti as u32);
                    }
                }
            }
        }
        m
    }

    fn cell_of(&self, p: Vec3) -> [usize; 3] {
        let f = |v: f32, lo: f32, n: usize| (((v - lo) / self.cell).floor().max(0.0) as usize).min(n - 1);
        [f(p.x, self.lo.x, self.dims[0]), f(p.y, self.lo.y, self.dims[1]), f(p.z, self.lo.z, self.dims[2])]
    }

    fn cell_index(&self, x: usize, y: usize, z: usize) -> usize {
        (x * self.dims[1] + y) * self.dims[2] + z
    }

    /// Triangles within `r` of the point (by cell).
    fn near(&self, p: Vec3, r: f32) -> Vec<u32> {
        let c0 = self.cell_of(p - Vec3::splat(r));
        let c1 = self.cell_of(p + Vec3::splat(r));
        let mut out = Vec::new();
        for x in c0[0]..=c1[0] {
            for y in c0[1]..=c1[1] {
                for z in c0[2]..=c1[2] {
                    out.extend_from_slice(&self.grid[self.cell_index(x, y, z)]);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Closest point on triangle `t` to `p`.
    pub fn closest_on_tri(&self, t: u32, p: Vec3) -> Vec3 {
        let tr = self.tris[t as usize];
        let (a, b, c) = (self.verts[tr[0] as usize], self.verts[tr[1] as usize], self.verts[tr[2] as usize]);
        closest_point_triangle(p, a, b, c)
    }

    /// Triangles whose cells overlap a box of half size `r` around `p`.
    pub fn near_tris(&self, p: Vec3, r: f32) -> Vec<u32> {
        self.near(p, r)
    }

    /// Nearest point of the mesh within `r`: (triangle, closest point, distance).
    pub fn nearest(&self, p: Vec3, r: f32) -> Option<(u32, Vec3, f32)> {
        let mut best: Option<(u32, Vec3, f32)> = None;
        for t in self.near(p, r) {
            let c = self.closest_on_tri(t, p);
            let d = (p - c).length();
            if d <= r && best.map(|b| d < b.2).unwrap_or(true) {
                best = Some((t, c, d));
            }
        }
        best
    }
}

pub fn closest_point_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[derive(Clone, Debug)]
pub enum Shape {
    /// Sphere at `off` from the center of mass.
    Sphere { r: f32, off: Vec3 },
    /// Box with half extents at `off` from the center of mass.
    Box { half: Vec3, off: Vec3 },
    /// Static triangle mesh (vertices relative to the center of mass); passive bodies only.
    Mesh(Arc<TriMesh>),
}

#[derive(Clone, Debug)]
pub struct Body {
    pub active: bool,
    pub collide: bool,
    pub mass: f32,
    pub bounce: f32,
    pub static_friction: f32,
    pub dynamic_friction: f32,
    pub damping: f32,
    pub shape: Shape,
    /// Center of mass in world space and orientation.
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub spin: Vec3,
    inv_mass: f32,
    inv_inertia: Vec3,
    rest: u32,
}

impl Body {
    pub fn new(shape: Shape, mass: f32, active: bool) -> Body {
        let mass = mass.max(1e-3);
        let inertia = match &shape {
            Shape::Sphere { r, .. } => Vec3::splat(0.4 * mass * r * r),
            Shape::Box { half, .. } => Vec3::new(mass / 3.0 * (half.y * half.y + half.z * half.z), mass / 3.0 * (half.x * half.x + half.z * half.z), mass / 3.0 * (half.x * half.x + half.y * half.y)),
            Shape::Mesh(_) => Vec3::splat(mass),
        };
        Body {
            active,
            collide: true,
            mass,
            bounce: 0.6,
            static_friction: 0.2,
            dynamic_friction: 0.2,
            damping: 0.0,
            shape,
            pos: Vec3::ZERO,
            rot: Quat::identity(),
            vel: Vec3::ZERO,
            spin: Vec3::ZERO,
            inv_mass: if active { 1.0 / mass } else { 0.0 },
            inv_inertia: if active { Vec3::new(1.0 / inertia.x.max(1e-6), 1.0 / inertia.y.max(1e-6), 1.0 / inertia.z.max(1e-6)) } else { Vec3::ZERO },
            rest: 0,
        }
    }

    pub fn set_active(&mut self, on: bool) {
        self.active = on;
        let fresh = Body::new(self.shape.clone(), self.mass, on);
        self.inv_mass = fresh.inv_mass;
        self.inv_inertia = fresh.inv_inertia;
    }

    pub fn inv_mass(&self) -> f32 {
        self.inv_mass
    }

    fn inv_inertia_world(&self) -> M3 {
        // R diag(i) R^T
        let r = rot_m3(self.rot);
        let d: M3 = [[self.inv_inertia.x, 0.0, 0.0], [0.0, self.inv_inertia.y, 0.0], [0.0, 0.0, self.inv_inertia.z]];
        m3_mul(&m3_mul(&r, &d), &m3_t(&r))
    }

    pub fn point_vel(&self, p: Vec3) -> Vec3 {
        self.vel + self.spin.cross(p - self.pos)
    }

    pub fn to_world(&self, local: Vec3) -> Vec3 {
        self.pos + self.rot.rotate(local)
    }

    pub fn to_local(&self, world: Vec3) -> Vec3 {
        self.rot.conjugate().rotate(world - self.pos)
    }

    /// World space bounds.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        match &self.shape {
            Shape::Sphere { r, off } => {
                let c = self.to_world(*off);
                (c - Vec3::splat(*r), c + Vec3::splat(*r))
            }
            Shape::Box { half, off } => {
                let c = self.to_world(*off);
                let e = Vec3::new(
                    (0..3).map(|k| self.rot.rotate(axis(k)).x.abs() * half[k]).sum(),
                    (0..3).map(|k| self.rot.rotate(axis(k)).y.abs() * half[k]).sum(),
                    (0..3).map(|k| self.rot.rotate(axis(k)).z.abs() * half[k]).sum(),
                );
                (c - e, c + e)
            }
            Shape::Mesh(m) => {
                let mut lo = Vec3::splat(f32::INFINITY);
                let mut hi = Vec3::splat(f32::NEG_INFINITY);
                for k in 0..8 {
                    let c = Vec3::new(if k & 1 == 0 { m.lo.x } else { m.hi.x }, if k & 2 == 0 { m.lo.y } else { m.hi.y }, if k & 4 == 0 { m.lo.z } else { m.hi.z });
                    let w = self.to_world(c);
                    lo = Vec3::new(lo.x.min(w.x), lo.y.min(w.y), lo.z.min(w.z));
                    hi = Vec3::new(hi.x.max(w.x), hi.y.max(w.y), hi.z.max(w.z));
                }
                (lo, hi)
            }
        }
    }
}

fn axis(k: usize) -> Vec3 {
    match k {
        0 => Vec3::new(1.0, 0.0, 0.0),
        1 => Vec3::new(0.0, 1.0, 0.0),
        _ => Vec3::new(0.0, 0.0, 1.0),
    }
}

fn rot_m3(q: Quat) -> M3 {
    let (c0, c1, c2) = (q.rotate(axis(0)), q.rotate(axis(1)), q.rotate(axis(2)));
    [[c0.x, c1.x, c2.x], [c0.y, c1.y, c2.y], [c0.z, c1.z, c2.z]]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointKind {
    /// Both anchors meet; rotation is free (Nail and Pin).
    Point,
    /// Anchors meet and the local axes stay aligned (Hinge).
    Hinge,
    /// A spring between the anchors.
    Spring,
}

#[derive(Clone, Debug)]
pub struct Joint {
    pub kind: JointKind,
    pub a: usize,
    /// None: anchored to the world point `world`.
    pub b: Option<usize>,
    pub local_a: Vec3,
    pub local_b: Vec3,
    pub world: Vec3,
    pub axis_a: Vec3,
    pub axis_b: Vec3,
    pub stiffness: f32,
    pub damping: f32,
    pub rest_length: f32,
}

#[derive(Clone, Copy, Debug)]
struct Contact {
    a: usize,
    b: usize,
    point: Vec3,
    /// From a toward b.
    normal: Vec3,
    depth: f32,
}

#[derive(Clone, Debug)]
pub struct World {
    pub bodies: Vec<Body>,
    pub joints: Vec<Joint>,
    /// Barrier planes: a point and the normal of the allowed side.
    pub barriers: Vec<(Vec3, Vec3)>,
    pub tolerance: f32,
    pub iterations: usize,
}

impl World {
    pub fn new() -> World {
        World { bodies: Vec::new(), joints: Vec::new(), barriers: Vec::new(), tolerance: 0.02, iterations: 10 }
    }

    /// Sample points of a box (corners, edge middles, face centers).
    fn box_samples(half: Vec3) -> Vec<Vec3> {
        let mut out = Vec::with_capacity(26);
        for x in -1i32..=1 {
            for y in -1i32..=1 {
                for z in -1i32..=1 {
                    if x == 0 && y == 0 && z == 0 {
                        continue;
                    }
                    out.push(Vec3::new(x as f32 * half.x, y as f32 * half.y, z as f32 * half.z));
                }
            }
        }
        out
    }

    fn contacts(&self) -> Vec<Contact> {
        let n = self.bodies.len();
        let bounds: Vec<(Vec3, Vec3)> = self.bodies.iter().map(|b| b.bounds()).collect();
        let margin = self.tolerance;
        let mut out = Vec::new();
        for i in 0..n {
            for j in (i + 1)..n {
                let (a, b) = (&self.bodies[i], &self.bodies[j]);
                if !a.collide || !b.collide || (!a.active && !b.active) {
                    continue;
                }
                let (ba, bb) = (bounds[i], bounds[j]);
                if ba.0.x > bb.1.x + margin || bb.0.x > ba.1.x + margin || ba.0.y > bb.1.y + margin || bb.0.y > ba.1.y + margin || ba.0.z > bb.1.z + margin || bb.0.z > ba.1.z + margin {
                    continue;
                }
                self.pair(i, j, &mut out);
            }
            // barriers
            let a = &self.bodies[i];
            if !a.active || !a.collide {
                continue;
            }
            for (pt, nrm) in &self.barriers {
                let pts: Vec<Vec3> = match &a.shape {
                    Shape::Sphere { r, off } => vec![a.to_world(*off) - *nrm * *r],
                    Shape::Box { half, off } => Self::box_samples(*half).into_iter().map(|s| a.to_world(*off + s)).collect(),
                    Shape::Mesh(_) => Vec::new(),
                };
                for p in pts {
                    let d = (p - *pt).dot(*nrm);
                    if d < margin {
                        out.push(Contact { a: usize::MAX, b: i, point: p, normal: *nrm, depth: margin - d });
                    }
                }
            }
        }
        out
    }

    fn pair(&self, i: usize, j: usize, out: &mut Vec<Contact>) {
        let (a, b) = (&self.bodies[i], &self.bodies[j]);
        match (&a.shape, &b.shape) {
            (Shape::Sphere { r: ra, off: oa }, Shape::Sphere { r: rb, off: ob }) => {
                let (ca, cb) = (a.to_world(*oa), b.to_world(*ob));
                let d = cb - ca;
                let l = d.length();
                if l < ra + rb {
                    let n = if l > 1e-6 { d * (1.0 / l) } else { Vec3::UP };
                    out.push(Contact { a: i, b: j, point: ca + n * *ra, normal: n, depth: ra + rb - l });
                }
            }
            (Shape::Sphere { .. }, Shape::Box { .. }) => self.sphere_box(i, j, false, out),
            (Shape::Box { .. }, Shape::Sphere { .. }) => self.sphere_box(j, i, true, out),
            (Shape::Sphere { .. }, Shape::Mesh(_)) => self.sphere_mesh(i, j, false, out),
            (Shape::Mesh(_), Shape::Sphere { .. }) => self.sphere_mesh(j, i, true, out),
            (Shape::Box { .. }, Shape::Box { .. }) => {
                self.box_in_box(i, j, false, out);
                self.box_in_box(j, i, true, out);
            }
            (Shape::Box { .. }, Shape::Mesh(_)) => self.box_mesh(i, j, false, out),
            (Shape::Mesh(_), Shape::Box { .. }) => self.box_mesh(j, i, true, out),
            _ => {}
        }
    }

    /// Contact with the normal from the first listed body toward the second; `flip` swaps which is a and b.
    fn push(&self, out: &mut Vec<Contact>, x: usize, y: usize, flip: bool, point: Vec3, n_x_to_y: Vec3, depth: f32) {
        if flip {
            out.push(Contact { a: y, b: x, point, normal: -n_x_to_y, depth });
        } else {
            out.push(Contact { a: x, b: y, point, normal: n_x_to_y, depth });
        }
    }

    fn sphere_box(&self, s: usize, bx: usize, flip: bool, out: &mut Vec<Contact>) {
        let (sp, bo) = (&self.bodies[s], &self.bodies[bx]);
        let (Shape::Sphere { r, off }, Shape::Box { half, off: boff }) = (&sp.shape, &bo.shape) else { return };
        let c = bo.to_local(sp.to_world(*off)) - *boff;
        let q = Vec3::new(c.x.clamp(-half.x, half.x), c.y.clamp(-half.y, half.y), c.z.clamp(-half.z, half.z));
        let d = c - q;
        let l = d.length();
        let (nrm_local, depth, pt) = if l > 1e-6 {
            if l >= *r {
                return;
            }
            (d * (1.0 / l), r - l, q)
        } else {
            // center inside the box: leave through the nearest face
            let gaps = [half.x - c.x.abs(), half.y - c.y.abs(), half.z - c.z.abs()];
            let k = (0..3).min_by(|a, b| gaps[*a].partial_cmp(&gaps[*b]).unwrap()).unwrap();
            let mut n = Vec3::ZERO;
            n[k] = if c[k] >= 0.0 { 1.0 } else { -1.0 };
            (n, gaps[k] + r, q)
        };
        let n_world_from_box = bo.rot.rotate(nrm_local);
        // normal from the sphere toward the box is -n; push uses x = sphere, y = box
        self.push(out, s, bx, flip, bo.to_world(pt + *boff), -n_world_from_box, depth);
    }

    fn sphere_mesh(&self, s: usize, m: usize, flip: bool, out: &mut Vec<Contact>) {
        let (sp, mo) = (&self.bodies[s], &self.bodies[m]);
        let (Shape::Sphere { r, off }, Shape::Mesh(mesh)) = (&sp.shape, &mo.shape) else { return };
        let c = mo.to_local(sp.to_world(*off));
        if let Some((t, cp, d)) = mesh.nearest(c, *r + self.tolerance) {
            let tn = mesh.normals[t as usize];
            let side = (c - cp).dot(tn);
            // the outward face normal decides the side, so a center that sank behind a face is pushed out again
            let (n, depth) = if d > 1e-6 && side >= 0.0 { ((c - cp) * (1.0 / d), r - d) } else { (tn, r + d) };
            let nw = mo.rot.rotate(n);
            self.push(out, s, m, flip, mo.to_world(cp), -nw, depth.max(-self.tolerance));
        }
    }

    /// Box sample points of `x` inside (or near) box `y`.
    fn box_in_box(&self, x: usize, y: usize, flip: bool, out: &mut Vec<Contact>) {
        let (bx, by) = (&self.bodies[x], &self.bodies[y]);
        let (Shape::Box { half: hx, off: ox }, Shape::Box { half: hy, off: oy }) = (&bx.shape, &by.shape) else { return };
        let tol = self.tolerance;
        for s in Self::box_samples(*hx) {
            let w = bx.to_world(*ox + s);
            let l = by.to_local(w) - *oy;
            let gaps = [hy.x - l.x.abs(), hy.y - l.y.abs(), hy.z - l.z.abs()];
            if gaps.iter().any(|g| *g < -tol) {
                continue;
            }
            let k = (0..3).min_by(|a, b| gaps[*a].partial_cmp(&gaps[*b]).unwrap()).unwrap();
            let mut n = Vec3::ZERO;
            n[k] = if l[k] >= 0.0 { 1.0 } else { -1.0 };
            // n points out of y toward x: normal from y to x
            let nw = by.rot.rotate(n);
            // contact normal runs from y toward x: push(x_is_first?) -> y is "a" in this orientation
            self.push(out, y, x, !flip, w, nw, gaps[k] + tol);
        }
    }

    fn box_mesh(&self, bx: usize, m: usize, flip: bool, out: &mut Vec<Contact>) {
        let (bo, mo) = (&self.bodies[bx], &self.bodies[m]);
        let (Shape::Box { half, off }, Shape::Mesh(mesh)) = (&bo.shape, &mo.shape) else { return };
        let tol = self.tolerance;
        for s in Self::box_samples(*half) {
            let w = bo.to_world(*off + s);
            let p = mo.to_local(w);
            if let Some((t, cp, d)) = mesh.nearest(p, tol * 2.0 + 1e-3) {
                let tn = mesh.normals[t as usize];
                let side = (p - cp).dot(tn);
                // in front of the face and close, or behind it (penetrating)
                let depth = if side >= 0.0 { tol - d } else { tol + d };
                if depth <= 0.0 {
                    continue;
                }
                let nw = mo.rot.rotate(tn);
                // normal from the mesh toward the box
                self.push(out, m, bx, !flip, w, nw, depth);
            }
        }
        // sharp mesh vertices poking into the box
        let inv = bo.rot.conjugate();
        for v in &mesh.verts {
            let w = mo.to_world(*v);
            let l = inv.rotate(w - bo.pos) - *off;
            let gaps = [half.x - l.x.abs(), half.y - l.y.abs(), half.z - l.z.abs()];
            if gaps.iter().any(|g| *g < 0.0) {
                continue;
            }
            let k = (0..3).min_by(|a, b| gaps[*a].partial_cmp(&gaps[*b]).unwrap()).unwrap();
            let mut n = Vec3::ZERO;
            n[k] = if l[k] >= 0.0 { 1.0 } else { -1.0 };
            // normal from the box toward the mesh point
            self.push(out, bx, m, flip, w, bo.rot.rotate(n), gaps[k] + tol);
        }
    }

    fn effective_inv_mass(&self, c: &Contact, dir: Vec3) -> f32 {
        let mut k = 0.0;
        for (idx, sign) in [(c.a, 1.0f32), (c.b, 1.0f32)] {
            let _ = sign;
            if idx == usize::MAX {
                continue;
            }
            let b = &self.bodies[idx];
            let r = c.point - b.pos;
            let rn = r.cross(dir);
            let iw = b.inv_inertia_world();
            k += b.inv_mass + rn.dot(m3_mul_v(&iw, rn));
        }
        k
    }

    fn apply(&mut self, idx: usize, p: Vec3, at: Vec3) {
        if idx == usize::MAX {
            return;
        }
        let (im, r) = (self.bodies[idx].inv_mass, at - self.bodies[idx].pos);
        let iw = self.bodies[idx].inv_inertia_world();
        let b = &mut self.bodies[idx];
        b.vel += p * im;
        b.spin += m3_mul_v(&iw, r.cross(p));
    }

    fn rel_vel(&self, c: &Contact) -> Vec3 {
        let va = if c.a == usize::MAX { Vec3::ZERO } else { self.bodies[c.a].point_vel(c.point) };
        self.bodies[c.b].point_vel(c.point) - va
    }

    /// Advance by `dt`. `accel(i, body)` gives the field acceleration of body i; kinematic bodies were placed by the caller.
    pub fn step(&mut self, dt: f32, accel: &dyn Fn(usize, &Body) -> Vec3) {
        for i in 0..self.bodies.len() {
            if self.bodies[i].active {
                let a = accel(i, &self.bodies[i]);
                let b = &mut self.bodies[i];
                b.vel += a * dt;
                let d = 1.0 / (1.0 + b.damping * dt);
                b.vel *= d;
                b.spin *= d;
            }
        }
        // springs
        for k in 0..self.joints.len() {
            if self.joints[k].kind == JointKind::Spring {
                self.spring(k, dt);
            }
        }
        let contacts = self.contacts();
        // pre-solve normal speed for restitution
        let approach: Vec<f32> = contacts.iter().map(|c| self.rel_vel(c).dot(c.normal)).collect();
        let mut normal_impulse = vec![0.0f32; contacts.len()];
        let mut tangent_impulse = vec![Vec3::ZERO; contacts.len()];
        for _ in 0..self.iterations {
            for k in 0..self.joints.len() {
                if self.joints[k].kind != JointKind::Spring {
                    self.solve_joint(k, dt);
                }
            }
            for (ci, c) in contacts.iter().enumerate() {
                let (ea, eb) = (self.restitution_of(c), 0.0);
                let _ = eb;
                let vrel = self.rel_vel(c);
                let vn = vrel.dot(c.normal);
                let km = self.effective_inv_mass(c, c.normal);
                if km <= 0.0 {
                    continue;
                }
                // target separation speed: restitution for fast impacts, a gentle push-out for overlap
                let target = if -approach[ci] > 0.8 { -approach[ci] * ea } else { 0.0 } + (c.depth - self.tolerance * 0.5).max(0.0) * 0.2 / dt.max(1e-4);
                let mut j = (target - vn) / km;
                let old = normal_impulse[ci];
                normal_impulse[ci] = (old + j).max(0.0);
                j = normal_impulse[ci] - old;
                let p = c.normal * j;
                self.apply(c.a, -p, c.point);
                self.apply(c.b, p, c.point);
                // friction
                let vrel = self.rel_vel(c);
                let vt = vrel - c.normal * vrel.dot(c.normal);
                let sp = vt.length();
                if sp > 1e-6 {
                    let t = vt * (1.0 / sp);
                    let kt = self.effective_inv_mass(c, t);
                    if kt > 0.0 {
                        let (ma, mb) = (self.mat(c.a), self.mat(c.b));
                        let mu_s = (ma.0 * mb.0).sqrt();
                        let mu_d = (ma.1 * mb.1).sqrt();
                        let mu = if sp < 0.05 { mu_s } else { mu_d };
                        let mut jt = -sp / kt;
                        let max = mu * normal_impulse[ci];
                        let old = tangent_impulse[ci];
                        let new = old + t * jt;
                        let l = new.length();
                        let new = if l > max && l > 0.0 { new * (max / l) } else { new };
                        jt = 0.0;
                        let _ = jt;
                        let delta = new - old;
                        tangent_impulse[ci] = new;
                        self.apply(c.a, -delta, c.point);
                        self.apply(c.b, delta, c.point);
                    }
                }
            }
        }
        // integrate positions
        for b in self.bodies.iter_mut().filter(|b| b.active) {
            b.pos += b.vel * dt;
            let w = Quat::new(b.spin.x, b.spin.y, b.spin.z, 0.0);
            let dq = w * b.rot;
            b.rot = Quat::new(b.rot.x + 0.5 * dt * dq.x, b.rot.y + 0.5 * dt * dq.y, b.rot.z + 0.5 * dt * dq.z, b.rot.w + 0.5 * dt * dq.w).normalized();
        }
        // push overlapping bodies apart
        for _ in 0..2 {
            let contacts = self.contacts();
            for c in &contacts {
                let slop = self.tolerance * 0.5;
                if c.depth <= slop {
                    continue;
                }
                let (ia, ib) = (if c.a == usize::MAX { 0.0 } else { self.bodies[c.a].inv_mass }, self.bodies[c.b].inv_mass);
                if ia + ib <= 0.0 {
                    continue;
                }
                let corr = c.normal * ((c.depth - slop) * 0.4 / (ia + ib));
                if c.a != usize::MAX {
                    self.bodies[c.a].pos -= corr * ia;
                }
                self.bodies[c.b].pos += corr * ib;
            }
        }
        // a body at rest on something stays put
        let contacts = self.contacts();
        let mut touched = vec![false; self.bodies.len()];
        for c in &contacts {
            if c.a != usize::MAX {
                touched[c.a] = true;
            }
            touched[c.b] = true;
        }
        for (i, b) in self.bodies.iter_mut().enumerate() {
            if !b.active {
                continue;
            }
            if touched[i] && b.vel.length() < 0.06 && b.spin.length() < 0.12 {
                b.rest += 1;
                if b.rest > 3 {
                    b.vel = Vec3::ZERO;
                    b.spin = Vec3::ZERO;
                }
            } else {
                b.rest = 0;
            }
        }
    }

    fn mat(&self, idx: usize) -> (f32, f32) {
        if idx == usize::MAX {
            (0.5, 0.5)
        } else {
            (self.bodies[idx].static_friction, self.bodies[idx].dynamic_friction)
        }
    }

    fn restitution_of(&self, c: &Contact) -> f32 {
        let ea = if c.a == usize::MAX { 0.5 } else { self.bodies[c.a].bounce };
        ea.min(self.bodies[c.b].bounce)
    }

    fn spring(&mut self, k: usize, dt: f32) {
        let j = self.joints[k].clone();
        let pa = self.bodies[j.a].to_world(j.local_a);
        let (pb, vb) = match j.b {
            Some(b) => (self.bodies[b].to_world(j.local_b), self.bodies[b].point_vel(self.bodies[b].to_world(j.local_b))),
            None => (j.world, Vec3::ZERO),
        };
        let d = pb - pa;
        let l = d.length();
        if l < 1e-6 {
            return;
        }
        let dir = d * (1.0 / l);
        let va = self.bodies[j.a].point_vel(pa);
        let rel = (vb - va).dot(dir);
        let force = dir * (j.stiffness * (l - j.rest_length) + j.damping * rel);
        let imp = force * dt;
        self.apply(j.a, imp, pa);
        if let Some(b) = j.b {
            self.apply(b, -imp, pb);
        }
    }

    fn solve_joint(&mut self, k: usize, dt: f32) {
        let j = self.joints[k].clone();
        let pa = self.bodies[j.a].to_world(j.local_a);
        let pb = match j.b {
            Some(b) => self.bodies[b].to_world(j.local_b),
            None => j.world,
        };
        let err = pb - pa;
        // 3x3 effective mass of the point constraint
        let ia = self.bodies[j.a].inv_mass;
        let (ra, rb) = (pa - self.bodies[j.a].pos, j.b.map(|b| pb - self.bodies[b].pos).unwrap_or(Vec3::ZERO));
        let (iwa, ima) = (self.bodies[j.a].inv_inertia_world(), ia);
        let mut kmat: M3 = [[0.0; 3]; 3];
        let sa = skew(ra);
        for r in 0..3 {
            for c in 0..3 {
                kmat[r][c] = -m3_mul(&m3_mul(&sa, &iwa), &sa)[r][c];
            }
            kmat[r][r] += ima;
        }
        if let Some(b) = j.b {
            let sb = skew(rb);
            let iwb = self.bodies[b].inv_inertia_world();
            let m = m3_mul(&m3_mul(&sb, &iwb), &sb);
            for r in 0..3 {
                for c in 0..3 {
                    kmat[r][c] -= m[r][c];
                }
                kmat[r][r] += self.bodies[b].inv_mass;
            }
        }
        let va = self.bodies[j.a].point_vel(pa);
        let vb = j.b.map(|b| self.bodies[b].point_vel(pb)).unwrap_or(Vec3::ZERO);
        // the impulse moves the anchor of a by K p; it must close the error and match the other anchor's velocity
        let impulse = m3_mul_v(&m3_inv(&kmat), (vb - va) + err * (0.2 / dt.max(1e-4)));
        self.apply(j.a, impulse, pa);
        if let Some(b) = j.b {
            self.apply(b, -impulse, pb);
        }
        if j.kind == JointKind::Hinge {
            // keep the hinge axes parallel: remove relative spin about the two directions across the axis
            let wa = self.bodies[j.a].rot.rotate(j.axis_a);
            let wb = match j.b {
                Some(b) => self.bodies[b].rot.rotate(j.axis_b),
                None => j.axis_b,
            };
            let misalign = wa.cross(wb);
            for dir in [perp1(wa), perp2(wa)] {
                let rel = j.b.map(|b| self.bodies[b].spin.dot(dir)).unwrap_or(0.0) - self.bodies[j.a].spin.dot(dir);
                let ka = dir.dot(m3_mul_v(&self.bodies[j.a].inv_inertia_world(), dir));
                let kb = j.b.map(|b| dir.dot(m3_mul_v(&self.bodies[b].inv_inertia_world(), dir))).unwrap_or(0.0);
                if ka + kb <= 0.0 {
                    continue;
                }
                let jn = -(rel - misalign.dot(dir) * 0.2 / dt.max(1e-4)) / (ka + kb);
                let (ia_w, ib_w) = (self.bodies[j.a].inv_inertia_world(), j.b.map(|b| self.bodies[b].inv_inertia_world()));
                self.bodies[j.a].spin -= m3_mul_v(&ia_w, dir * jn);
                if let (Some(b), Some(iw)) = (j.b, ib_w) {
                    self.bodies[b].spin += m3_mul_v(&iw, dir * jn);
                }
            }
        }
    }
}

fn perp1(v: Vec3) -> Vec3 {
    let h = if v.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
    v.cross(h).normalized()
}

fn perp2(v: Vec3) -> Vec3 {
    v.cross(perp1(v)).normalized()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> Body {
        let mut b = Body::new(Shape::Box { half: Vec3::new(10.0, 0.5, 10.0), off: Vec3::ZERO }, 1.0, false);
        b.pos = Vec3::new(0.0, -0.5, 0.0);
        b
    }

    fn run(w: &mut World, secs: f32) {
        let g = |_: usize, _: &Body| Vec3::new(0.0, -9.8, 0.0);
        let steps = (secs / 0.01) as usize;
        for _ in 0..steps {
            w.step(0.01, &g);
        }
    }

    #[test]
    fn sphere_falls_and_rests_on_a_floor() {
        let mut w = World::new();
        w.bodies.push(floor());
        let mut s = Body::new(Shape::Sphere { r: 0.5, off: Vec3::ZERO }, 1.0, true);
        s.pos = Vec3::new(0.0, 3.0, 0.0);
        w.bodies.push(s);
        run(&mut w, 3.0);
        assert!((w.bodies[1].pos.y - 0.5).abs() < 0.06, "{}", w.bodies[1].pos.y);
        assert!(w.bodies[1].vel.length() < 0.2);
    }

    #[test]
    fn bounciness_changes_the_rebound() {
        let height_after = |bounce: f32| {
            let mut w = World::new();
            let mut f = floor();
            f.bounce = bounce;
            w.bodies.push(f);
            let mut s = Body::new(Shape::Sphere { r: 0.5, off: Vec3::ZERO }, 1.0, true);
            s.bounce = bounce;
            s.pos = Vec3::new(0.0, 4.0, 0.0);
            w.bodies.push(s);
            let g = |_: usize, _: &Body| Vec3::new(0.0, -9.8, 0.0);
            let (mut hit, mut peak) = (false, 0.0f32);
            for _ in 0..400 {
                w.step(0.01, &g);
                if w.bodies[1].vel.y > 0.5 {
                    hit = true;
                }
                if hit {
                    peak = peak.max(w.bodies[1].pos.y);
                }
            }
            peak
        };
        assert!(height_after(0.9) > height_after(0.1) + 0.5);
    }

    #[test]
    fn a_box_stays_on_a_tilted_floor_with_friction_and_slides_without() {
        let slide = |mu: f32| {
            let mut w = World::new();
            let mut f = floor();
            f.rot = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 0.3);
            f.static_friction = mu;
            f.dynamic_friction = mu;
            w.bodies.push(f);
            let mut b = Body::new(Shape::Box { half: Vec3::splat(0.5), off: Vec3::ZERO }, 1.0, true);
            b.static_friction = mu;
            b.dynamic_friction = mu;
            b.rot = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 0.3);
            b.pos = w.bodies[0].rot.rotate(Vec3::new(0.0, 1.02, 0.0)) + w.bodies[0].pos;
            w.bodies.push(b);
            let start = w.bodies[1].pos;
            run(&mut w, 1.5);
            (w.bodies[1].pos - start).length()
        };
        assert!(slide(0.0) > 1.0, "frictionless slides: {}", slide(0.0));
        assert!(slide(0.9) < 0.3, "friction holds: {}", slide(0.9));
    }

    #[test]
    fn boxes_stack() {
        let mut w = World::new();
        w.bodies.push(floor());
        for i in 0..3 {
            let mut b = Body::new(Shape::Box { half: Vec3::splat(0.5), off: Vec3::ZERO }, 1.0, true);
            b.pos = Vec3::new(0.0, 0.51 + i as f32 * 1.02, 0.0);
            b.static_friction = 0.6;
            b.dynamic_friction = 0.6;
            w.bodies.push(b);
        }
        run(&mut w, 2.0);
        for (i, b) in w.bodies[1..].iter().enumerate() {
            assert!((b.pos.y - (0.5 + i as f32)).abs() < 0.12, "box {} at {}", i, b.pos.y);
            assert!(b.pos.x.abs() < 0.15 && b.pos.z.abs() < 0.15);
        }
    }

    #[test]
    fn a_sphere_collides_with_a_mesh_floor() {
        let verts = vec![Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, 5.0), Vec3::new(-5.0, 0.0, 5.0)];
        // counter clockwise seen from +Y: normal up
        let tris = vec![[0, 2, 1], [0, 3, 2]];
        let mesh = Arc::new(TriMesh::new(verts, tris));
        assert!(mesh.normals[0].y > 0.9);
        let mut w = World::new();
        w.bodies.push(Body::new(Shape::Mesh(mesh), 1.0, false));
        let mut s = Body::new(Shape::Sphere { r: 0.5, off: Vec3::ZERO }, 1.0, true);
        s.pos = Vec3::new(0.0, 2.0, 0.0);
        w.bodies.push(s);
        run(&mut w, 2.5);
        assert!((w.bodies[1].pos.y - 0.5).abs() < 0.08, "{}", w.bodies[1].pos.y);
    }

    #[test]
    fn a_nail_holds_a_pendulum_and_a_barrier_stops_a_fall() {
        let mut w = World::new();
        let mut b = Body::new(Shape::Sphere { r: 0.2, off: Vec3::ZERO }, 1.0, true);
        b.pos = Vec3::new(2.0, 5.0, 0.0);
        w.bodies.push(b);
        w.joints.push(Joint { kind: JointKind::Point, a: 0, b: None, local_a: Vec3::new(-2.0, 0.0, 0.0), local_b: Vec3::ZERO, world: Vec3::new(0.0, 5.0, 0.0), axis_a: Vec3::UP, axis_b: Vec3::UP, stiffness: 0.0, damping: 0.0, rest_length: 0.0 });
        run(&mut w, 1.0);
        let d = (w.bodies[0].pos - Vec3::new(0.0, 5.0, 0.0)).length();
        assert!((d - 2.0).abs() < 0.15, "the pendulum keeps its length: {}", d);
        assert!(w.bodies[0].pos.y < 4.9, "it swings down");
        let mut w2 = World::new();
        let mut s = Body::new(Shape::Sphere { r: 0.5, off: Vec3::ZERO }, 1.0, true);
        s.pos = Vec3::new(0.0, 3.0, 0.0);
        w2.bodies.push(s);
        w2.barriers.push((Vec3::ZERO, Vec3::UP));
        run(&mut w2, 2.5);
        assert!(w2.bodies[0].pos.y > 0.4, "{}", w2.bodies[0].pos.y);
    }
}
