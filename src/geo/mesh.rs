use crate::math::{Vec2, Vec3};

/// Polygon mesh buffers: positions, per-vertex normals, UVs and polygon face index lists.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub faces: Vec<Vec<u32>>,
}

impl Mesh {
    pub fn new() -> Mesh {
        Mesh::default()
    }

    pub fn add_vertex(&mut self, p: Vec3, n: Vec3, uv: Vec2) -> u32 {
        self.positions.push(p);
        self.normals.push(n);
        self.uvs.push(uv);
        (self.positions.len() - 1) as u32
    }

    pub fn add_face(&mut self, idx: &[u32]) {
        self.faces.push(idx.to_vec());
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Fan triangulation of every polygon face. Degenerate faces (fewer than 3 verts) are skipped.
    pub fn triangulate(&self) -> Vec<[u32; 3]> {
        let mut tris = Vec::new();
        for f in &self.faces {
            if f.len() < 3 {
                continue;
            }
            for i in 1..f.len() - 1 {
                tris.push([f[0], f[i], f[i + 1]]);
            }
        }
        tris
    }

    pub fn triangle_count(&self) -> usize {
        self.faces.iter().map(|f| f.len().saturating_sub(2)).sum()
    }

    /// Unique undirected edges for wireframe drawing.
    pub fn edges(&self) -> Vec<(u32, u32)> {
        let mut set = std::collections::BTreeSet::new();
        for f in &self.faces {
            let n = f.len();
            for i in 0..n {
                let a = f[i];
                let b = f[(i + 1) % n];
                if a != b {
                    set.insert((a.min(b), a.max(b)));
                }
            }
        }
        set.into_iter().collect()
    }

    /// Recompute smooth vertex normals from face geometry.
    pub fn recompute_normals(&mut self) {
        let mut acc = vec![Vec3::ZERO; self.positions.len()];
        for t in self.triangulate() {
            let a = self.positions[t[0] as usize];
            let b = self.positions[t[1] as usize];
            let c = self.positions[t[2] as usize];
            let n = (b - a).cross(c - a);
            for &i in &t {
                acc[i as usize] += n;
            }
        }
        for (i, n) in acc.into_iter().enumerate() {
            let nn = n.normalized();
            self.normals[i] = if nn.length_sq() > 0.0 { nn } else { Vec3::UP };
        }
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        if self.positions.is_empty() {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        let mut lo = self.positions[0];
        let mut hi = self.positions[0];
        for p in &self.positions {
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        (lo, hi)
    }
}
