//! Boolean operations on closed triangle meshes (union, difference, intersection) with BSP trees.

use crate::math::Vec3;

const EPS: f32 = 1e-5;

#[derive(Clone, Debug)]
struct Plane {
    n: Vec3,
    w: f32,
}

impl Plane {
    fn from_points(a: Vec3, b: Vec3, c: Vec3) -> Option<Plane> {
        let n = (b - a).cross(c - a);
        if n.length_sq() < 1e-20 {
            return None;
        }
        let n = n.normalized();
        Some(Plane { n, w: n.dot(a) })
    }

    fn flip(&mut self) {
        self.n = -self.n;
        self.w = -self.w;
    }
}

#[derive(Clone, Debug)]
pub struct Poly {
    v: Vec<Vec3>,
    plane: Plane,
}

#[derive(Default)]
struct Parts {
    cf: Vec<Poly>,
    cb: Vec<Poly>,
    f: Vec<Poly>,
    b: Vec<Poly>,
}

impl Plane {
    fn split(&self, p: &Poly, parts: &mut Parts) {
        const COPLANAR: u8 = 0;
        const FRONT: u8 = 1;
        const BACK: u8 = 2;
        const SPANNING: u8 = 3;
        let mut ptype = 0u8;
        let types: Vec<u8> = p
            .v
            .iter()
            .map(|v| {
                let t = self.n.dot(*v) - self.w;
                let ty = if t < -EPS {
                    BACK
                } else if t > EPS {
                    FRONT
                } else {
                    COPLANAR
                };
                ptype |= ty;
                ty
            })
            .collect();
        match ptype {
            COPLANAR => {
                if self.n.dot(p.plane.n) > 0.0 {
                    parts.cf.push(p.clone());
                } else {
                    parts.cb.push(p.clone());
                }
            }
            FRONT => parts.f.push(p.clone()),
            BACK => parts.b.push(p.clone()),
            _ => {
                let _ = SPANNING;
                let (mut f, mut b): (Vec<Vec3>, Vec<Vec3>) = (Vec::new(), Vec::new());
                let n = p.v.len();
                for i in 0..n {
                    let j = (i + 1) % n;
                    let (ti, tj) = (types[i], types[j]);
                    let (vi, vj) = (p.v[i], p.v[j]);
                    if ti != BACK {
                        f.push(vi);
                    }
                    if ti != FRONT {
                        b.push(vi);
                    }
                    if (ti | tj) == SPANNING {
                        let t = (self.w - self.n.dot(vi)) / self.n.dot(vj - vi);
                        let v = vi.lerp(vj, t);
                        f.push(v);
                        b.push(v);
                    }
                }
                if f.len() >= 3 {
                    parts.f.push(Poly { v: f, plane: p.plane.clone() });
                }
                if b.len() >= 3 {
                    parts.b.push(Poly { v: b, plane: p.plane.clone() });
                }
            }
        }
    }
}

#[derive(Default)]
struct Node {
    plane: Option<Plane>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
    polys: Vec<Poly>,
}

impl Node {
    fn new(polys: Vec<Poly>) -> Node {
        let mut n = Node::default();
        n.build(polys);
        n
    }

    fn invert(&mut self) {
        for p in self.polys.iter_mut() {
            p.v.reverse();
            p.plane.flip();
        }
        if let Some(p) = &mut self.plane {
            p.flip();
        }
        if let Some(f) = &mut self.front {
            f.invert();
        }
        if let Some(b) = &mut self.back {
            b.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    fn clip_polygons(&self, polys: Vec<Poly>) -> Vec<Poly> {
        let Some(plane) = &self.plane else { return polys };
        let mut parts = Parts::default();
        for p in &polys {
            plane.split(p, &mut parts);
        }
        let mut front = parts.cf;
        front.extend(parts.f);
        let mut back = parts.cb;
        back.extend(parts.b);
        let front = match &self.front {
            Some(f) => f.clip_polygons(front),
            None => front,
        };
        let back = match &self.back {
            Some(b) => b.clip_polygons(back),
            None => Vec::new(),
        };
        let mut out = front;
        out.extend(back);
        out
    }

    fn clip_to(&mut self, other: &Node) {
        self.polys = other.clip_polygons(std::mem::take(&mut self.polys));
        if let Some(f) = &mut self.front {
            f.clip_to(other);
        }
        if let Some(b) = &mut self.back {
            b.clip_to(other);
        }
    }

    fn all(&self) -> Vec<Poly> {
        let mut out = self.polys.clone();
        if let Some(f) = &self.front {
            out.extend(f.all());
        }
        if let Some(b) = &self.back {
            out.extend(b.all());
        }
        out
    }

    fn build(&mut self, polys: Vec<Poly>) {
        if polys.is_empty() {
            return;
        }
        let plane = self.plane.get_or_insert_with(|| polys[0].plane.clone()).clone();
        let mut parts = Parts::default();
        for p in &polys {
            plane.split(p, &mut parts);
        }
        self.polys.extend(parts.cf);
        self.polys.extend(parts.cb);
        if !parts.f.is_empty() {
            self.front.get_or_insert_with(Default::default).build(parts.f);
        }
        if !parts.b.is_empty() {
            self.back.get_or_insert_with(Default::default).build(parts.b);
        }
    }
}

/// Triangles of a polygon mesh (fan triangulation), degenerate ones dropped.
pub fn to_polys(pos: &[Vec3], faces: &[Vec<u32>]) -> Vec<Poly> {
    let mut out = Vec::new();
    for f in faces {
        for i in 1..f.len().saturating_sub(1) {
            let (a, b, c) = (pos[f[0] as usize], pos[f[i] as usize], pos[f[i + 1] as usize]);
            if let Some(plane) = Plane::from_points(a, b, c) {
                out.push(Poly { v: vec![a, b, c], plane });
            }
        }
    }
    out
}

/// Positions and polygon faces of a polygon soup (shared positions are merged by the caller).
pub fn from_polys(polys: &[Poly]) -> (Vec<Vec3>, Vec<Vec<u32>>) {
    let mut pos = Vec::new();
    let mut faces = Vec::new();
    for p in polys {
        let base = pos.len() as u32;
        pos.extend(p.v.iter().copied());
        faces.push((0..p.v.len() as u32).map(|i| base + i).collect());
    }
    (pos, faces)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Difference,
    Intersection,
}

pub fn boolean(a: Vec<Poly>, b: Vec<Poly>, op: BoolOp) -> Vec<Poly> {
    let mut a = Node::new(a);
    let mut b = Node::new(b);
    match op {
        BoolOp::Union => {
            a.clip_to(&b);
            b.clip_to(&a);
            b.invert();
            b.clip_to(&a);
            b.invert();
            a.build(b.all());
        }
        BoolOp::Difference => {
            a.invert();
            a.clip_to(&b);
            b.clip_to(&a);
            b.invert();
            b.clip_to(&a);
            b.invert();
            a.build(b.all());
            a.invert();
        }
        BoolOp::Intersection => {
            a.invert();
            b.clip_to(&a);
            b.invert();
            a.clip_to(&b);
            b.clip_to(&a);
            a.build(b.all());
            a.invert();
        }
    }
    a.all()
}
