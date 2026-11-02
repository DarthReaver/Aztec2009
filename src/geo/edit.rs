//! Polygon editing kernel: welding, extrude, bevel, delete, merge, split, edge loops, smoothing.
//!
//! Every operation works on a `Poly` (welded positions plus polygon index lists) and returns the
//! component selection that should be active afterwards, using indices of the cleaned result.

use super::mesh::Mesh;
use crate::math::{Vec2, Vec3};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

pub type Edge = (u32, u32);

pub fn ekey(a: u32, b: u32) -> Edge {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sel {
    pub verts: BTreeSet<u32>,
    pub edges: BTreeSet<Edge>,
    pub faces: BTreeSet<u32>,
}

impl Sel {
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty() && self.edges.is_empty() && self.faces.is_empty()
    }
}

pub struct Remap {
    pub v: Vec<Option<u32>>,
    pub f: Vec<Option<u32>>,
}

impl Remap {
    pub fn sel(&self, s: &Sel) -> Sel {
        let mut o = Sel::default();
        for &v in &s.verts {
            if let Some(Some(n)) = self.v.get(v as usize) {
                o.verts.insert(*n);
            }
        }
        for &(a, b) in &s.edges {
            if let (Some(Some(x)), Some(Some(y))) = (self.v.get(a as usize), self.v.get(b as usize)) {
                o.edges.insert(ekey(*x, *y));
            }
        }
        for &f in &s.faces {
            if let Some(Some(n)) = self.f.get(f as usize) {
                o.faces.insert(*n);
            }
        }
        o
    }
}

#[derive(Clone, Debug, Default)]
pub struct Poly {
    pub pos: Vec<Vec3>,
    pub faces: Vec<Vec<u32>>,
}

pub fn face_normal(pos: &[Vec3], f: &[u32]) -> Vec3 {
    // Newell's method
    let mut n = Vec3::ZERO;
    for i in 0..f.len() {
        let a = pos[f[i] as usize];
        let b = pos[f[(i + 1) % f.len()] as usize];
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

impl Poly {
    pub fn from_mesh(m: &Mesh) -> Poly {
        Poly { pos: m.positions.clone(), faces: m.faces.clone() }
    }

    /// Merge vertices closer than `eps` and drop degenerate faces. Normals and UVs are rebuilt.
    pub fn welded(m: &Mesh, eps: f32) -> Poly {
        Poly::welded_with_faces(m, eps).0
    }

    /// Like `welded`, also returning where each input face went (`None` for faces that were dropped).
    pub fn welded_with_faces(m: &Mesh, eps: f32) -> (Poly, Vec<Option<u32>>) {
        let q = |v: f32| (v / eps).round() as i64;
        let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let mut pos = Vec::new();
        let mut remap = Vec::with_capacity(m.positions.len());
        for p in &m.positions {
            let k = (q(p.x), q(p.y), q(p.z));
            let id = *map.entry(k).or_insert_with(|| {
                pos.push(*p);
                (pos.len() - 1) as u32
            });
            remap.push(id);
        }
        let faces = m.faces.iter().map(|f| f.iter().map(|&v| remap[v as usize]).collect()).collect();
        let mut p = Poly { pos, faces };
        let remap = p.cleanup();
        (p, remap.f)
    }

    pub fn to_mesh(&self) -> Mesh {
        let mut m = Mesh {
            positions: self.pos.clone(),
            normals: vec![Vec3::UP; self.pos.len()],
            uvs: vec![Vec2::new(0.0, 0.0); self.pos.len()],
            faces: self.faces.clone(),
        };
        m.recompute_normals();
        m
    }

    pub fn edge_faces(&self) -> HashMap<Edge, Vec<usize>> {
        let mut m: HashMap<Edge, Vec<usize>> = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for i in 0..f.len() {
                let (a, b) = (f[i], f[(i + 1) % f.len()]);
                if a != b {
                    let e = m.entry(ekey(a, b)).or_default();
                    if !e.contains(&fi) {
                        e.push(fi);
                    }
                }
            }
        }
        m
    }

    pub fn edge_list(&self) -> Vec<Edge> {
        let mut v: Vec<Edge> = self.edge_faces().into_keys().collect();
        v.sort();
        v
    }

    /// Drop repeated corners, faces with fewer than three corners and unused vertices.
    pub fn cleanup(&mut self) -> Remap {
        let mut fmap = vec![None; self.faces.len()];
        let mut faces: Vec<Vec<u32>> = Vec::with_capacity(self.faces.len());
        for (i, f) in self.faces.iter().enumerate() {
            let mut g: Vec<u32> = Vec::with_capacity(f.len());
            for &v in f {
                if g.last() != Some(&v) {
                    g.push(v);
                }
            }
            while g.len() > 1 && g.first() == g.last() {
                g.pop();
            }
            let uniq: HashSet<&u32> = g.iter().collect();
            if g.len() >= 3 && uniq.len() == g.len() {
                fmap[i] = Some(faces.len() as u32);
                faces.push(g);
            }
        }
        let n = self.pos.len();
        let mut used = vec![false; n];
        for f in &faces {
            for &v in f {
                used[v as usize] = true;
            }
        }
        let mut vmap = vec![None; n];
        let mut pos = Vec::new();
        for i in 0..n {
            if used[i] {
                vmap[i] = Some(pos.len() as u32);
                pos.push(self.pos[i]);
            }
        }
        for f in &mut faces {
            for v in f.iter_mut() {
                *v = vmap[*v as usize].unwrap();
            }
        }
        self.pos = pos;
        self.faces = faces;
        Remap { v: vmap, f: fmap }
    }

    pub fn face_center(&self, f: usize) -> Vec3 {
        let fv = &self.faces[f];
        let mut c = Vec3::ZERO;
        for &v in fv {
            c += self.pos[v as usize];
        }
        c * (1.0 / fv.len().max(1) as f32)
    }

    pub fn face_normal(&self, f: usize) -> Vec3 {
        face_normal(&self.pos, &self.faces[f]).normalized()
    }

    // ------------------------------------------------------------------ selection helpers

    pub fn verts_of_faces(&self, faces: &BTreeSet<u32>) -> BTreeSet<u32> {
        let mut s = BTreeSet::new();
        for &f in faces {
            if let Some(fv) = self.faces.get(f as usize) {
                s.extend(fv.iter().copied());
            }
        }
        s
    }

    pub fn faces_to_edges(&self, faces: &BTreeSet<u32>) -> BTreeSet<Edge> {
        let mut s = BTreeSet::new();
        for &f in faces {
            if let Some(fv) = self.faces.get(f as usize) {
                for i in 0..fv.len() {
                    s.insert(ekey(fv[i], fv[(i + 1) % fv.len()]));
                }
            }
        }
        s
    }

    /// Faces whose every vertex is in `verts`.
    pub fn faces_inside_verts(&self, verts: &BTreeSet<u32>) -> BTreeSet<u32> {
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.iter().all(|v| verts.contains(v)))
            .map(|(i, _)| i as u32)
            .collect()
    }

    /// Faces whose every edge is in `edges`.
    pub fn faces_inside_edges(&self, edges: &BTreeSet<Edge>) -> BTreeSet<u32> {
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, f)| (0..f.len()).all(|i| edges.contains(&ekey(f[i], f[(i + 1) % f.len()]))))
            .map(|(i, _)| i as u32)
            .collect()
    }

    pub fn grow(&self, sel: &Sel) -> Sel {
        let mut out = sel.clone();
        let mut verts: BTreeSet<u32> = sel.verts.clone();
        verts.extend(sel.edges.iter().flat_map(|e| [e.0, e.1]));
        verts.extend(self.verts_of_faces(&sel.faces));
        for (fi, f) in self.faces.iter().enumerate() {
            if f.iter().any(|v| verts.contains(v)) {
                out.faces.insert(fi as u32);
                if !sel.faces.is_empty() {
                    continue;
                }
                for i in 0..f.len() {
                    let e = ekey(f[i], f[(i + 1) % f.len()]);
                    if verts.contains(&e.0) || verts.contains(&e.1) {
                        if !sel.edges.is_empty() {
                            out.edges.insert(e);
                        }
                        if !sel.verts.is_empty() {
                            out.verts.insert(e.0);
                            out.verts.insert(e.1);
                        }
                    }
                }
            }
        }
        if sel.faces.is_empty() {
            out.faces.clear();
        }
        out
    }

    /// The edge loop through `e`, following valence four vertices (or a border).
    pub fn edge_loop(&self, e: Edge) -> BTreeSet<Edge> {
        let ef = self.edge_faces();
        let mut nbrs: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(a, b) in ef.keys() {
            nbrs.entry(a).or_default().push(b);
            nbrs.entry(b).or_default().push(a);
        }
        let is_border = |k: &Edge| ef.get(k).map(|f| f.len() == 1).unwrap_or(false);
        let border = is_border(&e);
        let shares_face = |u: u32, v: u32, w: u32| -> bool {
            // is there a face with u, v, w consecutive (v in the middle)?
            self.faces.iter().any(|f| {
                let n = f.len();
                (0..n).any(|i| f[i] == v && ((f[(i + n - 1) % n] == u && f[(i + 1) % n] == w) || (f[(i + n - 1) % n] == w && f[(i + 1) % n] == u)))
            })
        };
        let mut out = BTreeSet::new();
        out.insert(e);
        for &(start_from, start_to) in &[(e.0, e.1), (e.1, e.0)] {
            let (mut u, mut v) = (start_from, start_to);
            for _ in 0..self.pos.len() + 4 {
                let ns = match nbrs.get(&v) {
                    Some(n) => n,
                    None => break,
                };
                let next = if border {
                    let b: Vec<u32> = ns.iter().copied().filter(|&w| w != u && is_border(&ekey(v, w))).collect();
                    if b.len() == 1 {
                        Some(b[0])
                    } else {
                        None
                    }
                } else if ns.len() == 4 {
                    let c: Vec<u32> = ns.iter().copied().filter(|&w| w != u && !shares_face(u, v, w)).collect();
                    if c.len() == 1 {
                        Some(c[0])
                    } else {
                        None
                    }
                } else {
                    None
                };
                match next {
                    Some(w) => {
                        let k = ekey(v, w);
                        if !out.insert(k) {
                            break;
                        }
                        u = v;
                        v = w;
                    }
                    None => break,
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------ extrude

    /// Extrude faces together. Returns the same faces (now the cap) selected.
    pub fn extrude_faces(&mut self, sel: &BTreeSet<u32>) -> Sel {
        let mut directed: HashSet<(u32, u32)> = HashSet::new();
        for &f in sel {
            let fv = &self.faces[f as usize];
            for i in 0..fv.len() {
                directed.insert((fv[i], fv[(i + 1) % fv.len()]));
            }
        }
        let verts = self.verts_of_faces(sel);
        let mut map: HashMap<u32, u32> = HashMap::new();
        for &v in &verts {
            map.insert(v, self.pos.len() as u32);
            self.pos.push(self.pos[v as usize]);
        }
        let mut walls: Vec<Vec<u32>> = Vec::new();
        let mut keys: Vec<(u32, u32)> = directed.iter().copied().collect();
        keys.sort();
        for (a, b) in keys {
            if !directed.contains(&(b, a)) {
                walls.push(vec![a, b, map[&b], map[&a]]);
            }
        }
        for &f in sel {
            for v in self.faces[f as usize].iter_mut() {
                *v = map[v];
            }
        }
        self.faces.extend(walls);
        let mut out = Sel::default();
        out.faces = sel.clone();
        out.verts = map.values().copied().collect();
        out
    }

    pub fn extrude_edges(&mut self, sel: &BTreeSet<Edge>) -> Sel {
        let ef = self.edge_faces();
        let mut dir: HashSet<(u32, u32)> = HashSet::new();
        for f in &self.faces {
            for i in 0..f.len() {
                dir.insert((f[i], f[(i + 1) % f.len()]));
            }
        }
        let mut map: HashMap<u32, u32> = HashMap::new();
        for &(a, b) in sel {
            for v in [a, b] {
                map.entry(v).or_insert_with(|| {
                    self.pos.push(self.pos[v as usize]);
                    (self.pos.len() - 1) as u32
                });
            }
        }
        let mut out = Sel::default();
        for &(a, b) in sel {
            let (na, nb) = (map[&a], map[&b]);
            let _ = &ef;
            if dir.contains(&(a, b)) {
                self.faces.push(vec![b, a, na, nb]);
            } else {
                self.faces.push(vec![a, b, nb, na]);
            }
            out.edges.insert(ekey(na, nb));
        }
        out.verts = map.values().copied().collect();
        out
    }

    // ------------------------------------------------------------------ delete

    pub fn delete_faces(&mut self, sel: &BTreeSet<u32>) {
        let mut i = 0usize;
        self.faces.retain(|_| {
            let keep = !sel.contains(&(i as u32));
            i += 1;
            keep
        });
        self.cleanup();
    }

    fn merge_across(&mut self, a: u32, b: u32, work: &mut [Option<Vec<u32>>]) -> bool {
        let mut fa = None;
        let mut fb = None;
        for (i, f) in work.iter().enumerate() {
            if let Some(f) = f {
                let n = f.len();
                for k in 0..n {
                    if f[k] == a && f[(k + 1) % n] == b {
                        fa = Some((i, k));
                    }
                    if f[k] == b && f[(k + 1) % n] == a {
                        fb = Some((i, k));
                    }
                }
            }
        }
        let ((ia, ka), (ib, kb)) = match (fa, fb) {
            (Some(x), Some(y)) if x.0 != y.0 => (x, y),
            _ => return false,
        };
        let fa = work[ia].take().unwrap();
        let fb = work[ib].take().unwrap();
        let (na, nb) = (fa.len(), fb.len());
        // fa rotated so it starts at b and ends at a
        let mut merged: Vec<u32> = (0..na).map(|j| fa[(ka + 1 + j) % na]).collect();
        // fb rotated to start at a: elements after a up to (excluding) b
        let after: Vec<u32> = (0..nb).map(|j| fb[(kb + 1 + j) % nb]).collect();
        // fb[kb] == b, fb[kb+1] == a ; after = [a, y1..yk, b]
        for &v in after.iter().skip(1).take(after.len().saturating_sub(2)) {
            merged.push(v);
        }
        work[ia] = Some(merged);
        true
    }

    pub fn delete_edges(&mut self, sel: &BTreeSet<Edge>, clean_verts: bool) -> Sel {
        let mut work: Vec<Option<Vec<u32>>> = self.faces.iter().cloned().map(Some).collect();
        let mut touched: BTreeSet<u32> = BTreeSet::new();
        for &(a, b) in sel {
            if self.merge_across(a, b, &mut work) {
                touched.insert(a);
                touched.insert(b);
            }
        }
        let mut faces: Vec<Vec<u32>> = work.into_iter().flatten().collect();
        if clean_verts {
            // remove vertices that only sit inside a single polygon and have just two edges
            let mut use_count: HashMap<u32, usize> = HashMap::new();
            for f in &faces {
                for &v in f {
                    *use_count.entry(v).or_default() += 1;
                }
            }
            for &v in &touched {
                if use_count.get(&v) == Some(&1) {
                    if let Some(f) = faces.iter_mut().find(|f| f.contains(&v)) {
                        if f.len() > 3 {
                            f.retain(|&x| x != v);
                        }
                    }
                }
            }
        }
        self.faces = faces;
        self.cleanup();
        Sel::default()
    }

    pub fn delete_verts(&mut self, sel: &BTreeSet<u32>) {
        let ef = self.edge_faces();
        let mut edges: BTreeSet<Edge> = BTreeSet::new();
        for e in ef.keys() {
            if sel.contains(&e.0) || sel.contains(&e.1) {
                edges.insert(*e);
            }
        }
        let mut work: Vec<Option<Vec<u32>>> = self.faces.iter().cloned().map(Some).collect();
        for &(a, b) in &edges {
            self.merge_across(a, b, &mut work);
        }
        let mut faces: Vec<Vec<u32>> = work.into_iter().flatten().collect();
        for f in faces.iter_mut() {
            if f.iter().any(|v| sel.contains(v)) {
                f.retain(|v| !sel.contains(v));
            }
        }
        self.faces = faces;
        self.cleanup();
    }

    // ------------------------------------------------------------------ merge / collapse

    /// Merge each group of vertices into its centroid.
    pub fn merge_groups(&mut self, groups: &[Vec<u32>]) -> Remap {
        let mut remap: Vec<u32> = (0..self.pos.len() as u32).collect();
        for g in groups {
            if g.len() < 2 {
                continue;
            }
            let mut c = Vec3::ZERO;
            for &v in g {
                c += self.pos[v as usize];
            }
            c = c * (1.0 / g.len() as f32);
            let keep = g[0];
            self.pos[keep as usize] = c;
            for &v in g {
                remap[v as usize] = keep;
            }
        }
        for f in &mut self.faces {
            for v in f.iter_mut() {
                *v = remap[*v as usize];
            }
        }
        self.cleanup()
    }

    /// Group the given vertices that lie within `dist` of each other (single link clustering).
    pub fn cluster_by_distance(&self, verts: &BTreeSet<u32>, dist: f32) -> Vec<Vec<u32>> {
        let vs: Vec<u32> = verts.iter().copied().collect();
        let mut parent: Vec<usize> = (0..vs.len()).collect();
        fn find(p: &mut [usize], x: usize) -> usize {
            let mut r = x;
            while p[r] != r {
                r = p[r];
            }
            let mut c = x;
            while p[c] != r {
                let n = p[c];
                p[c] = r;
                c = n;
            }
            r
        }
        for i in 0..vs.len() {
            for j in i + 1..vs.len() {
                if self.pos[vs[i] as usize].distance(self.pos[vs[j] as usize]) <= dist {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a] = b;
                    }
                }
            }
        }
        let mut groups: HashMap<usize, Vec<u32>> = HashMap::new();
        for i in 0..vs.len() {
            let r = find(&mut parent, i);
            groups.entry(r).or_default().push(vs[i]);
        }
        let mut out: Vec<Vec<u32>> = groups.into_values().collect();
        out.sort();
        out
    }

    /// Connected groups of vertices linked by the given edges.
    pub fn groups_from_edges(edges: &BTreeSet<Edge>) -> Vec<Vec<u32>> {
        let mut parent: HashMap<u32, u32> = HashMap::new();
        fn find(p: &mut HashMap<u32, u32>, x: u32) -> u32 {
            let mut r = x;
            while let Some(&n) = p.get(&r) {
                if n == r {
                    break;
                }
                r = n;
            }
            r
        }
        for &(a, b) in edges {
            parent.entry(a).or_insert(a);
            parent.entry(b).or_insert(b);
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent.insert(ra, rb);
            }
        }
        let keys: Vec<u32> = parent.keys().copied().collect();
        let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
        for k in keys {
            let r = find(&mut parent, k);
            groups.entry(r).or_default().push(k);
        }
        let mut out: Vec<Vec<u32>> = groups.into_values().map(|mut g| {
            g.sort();
            g
        }).collect();
        out.sort();
        out
    }

    pub fn collapse_edges(&mut self, edges: &BTreeSet<Edge>) {
        let groups = Poly::groups_from_edges(edges);
        self.merge_groups(&groups);
    }

    pub fn collapse_faces(&mut self, faces: &BTreeSet<u32>) {
        let mut edges = BTreeSet::new();
        for &f in faces {
            let fv = &self.faces[f as usize];
            for i in 0..fv.len() {
                edges.insert(ekey(fv[i], fv[(i + 1) % fv.len()]));
            }
        }
        self.collapse_edges(&edges);
    }

    // ------------------------------------------------------------------ bevel

    /// Chamfer the given interior edges by `offset` (single segment). Returns the new faces.
    pub fn bevel_edges(&mut self, sel: &BTreeSet<Edge>, offset: f32) -> Sel {
        let ef = self.edge_faces();
        let mut dir_face: HashMap<(u32, u32), usize> = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for i in 0..f.len() {
                dir_face.insert((f[i], f[(i + 1) % f.len()]), fi);
            }
        }
        let mut edges: BTreeSet<Edge> = sel
            .iter()
            .copied()
            .filter(|e| dir_face.contains_key(&(e.0, e.1)) && dir_face.contains_key(&(e.1, e.0)))
            .collect();
        let offset = offset.max(1e-5);

        struct Sector {
            faces: Vec<usize>,
            left: u32,
            right: u32,
            left_sel: bool,
            right_sel: bool,
            vtx: u32,
        }
        struct VFan {
            sectors: Vec<Sector>,
            closed: bool,
            virtual_ins: Vec<(usize, u32, u32)>, // (face, neighbor vertex, new vertex to insert)
        }

        // Build fans per vertex; vertices with broken fans drop their edges (repeat to a fixpoint).
        let mut fans: HashMap<u32, (Vec<usize>, bool)> = HashMap::new();
        loop {
            fans.clear();
            let mut bad: BTreeSet<u32> = BTreeSet::new();
            let bverts: BTreeSet<u32> = edges.iter().flat_map(|e| [e.0, e.1]).collect();
            for &v in &bverts {
                // corners
                let mut corners: Vec<(usize, u32, u32)> = Vec::new(); // face, prev, next
                for (fi, f) in self.faces.iter().enumerate() {
                    let n = f.len();
                    for i in 0..n {
                        if f[i] == v {
                            corners.push((fi, f[(i + n - 1) % n], f[(i + 1) % n]));
                        }
                    }
                }
                if corners.is_empty() {
                    bad.insert(v);
                    continue;
                }
                let by_prev: HashMap<u32, usize> = corners.iter().enumerate().map(|(i, c)| (c.1, i)).collect();
                if by_prev.len() != corners.len() {
                    bad.insert(v);
                    continue;
                }
                let has_pred: HashSet<usize> = corners.iter().filter_map(|c| by_prev.get(&c.2).copied()).collect();
                let start = (0..corners.len()).find(|i| !has_pred.contains(i)).unwrap_or(0);
                let closed = (0..corners.len()).all(|i| has_pred.contains(&i));
                let mut order = Vec::new();
                let mut cur = start;
                let mut seen = HashSet::new();
                loop {
                    if !seen.insert(cur) {
                        break;
                    }
                    order.push(corners[cur].0);
                    match by_prev.get(&corners[cur].2) {
                        Some(&n) => cur = n,
                        None => break,
                    }
                }
                if order.len() != corners.len() {
                    bad.insert(v);
                    continue;
                }
                fans.insert(v, (order, closed));
            }
            let before = edges.len();
            edges.retain(|e| !bad.contains(&e.0) && !bad.contains(&e.1));
            if edges.len() == before {
                break;
            }
        }
        if edges.is_empty() {
            return Sel::default();
        }

        let mut new_pos = self.pos.clone();
        let mut new_faces = self.faces.clone();
        let mut corner_map: HashMap<(usize, u32), u32> = HashMap::new();
        let mut patches: Vec<Vec<u32>> = Vec::new();
        let mut vfans: HashMap<u32, VFan> = HashMap::new();
        let mut inserts: Vec<(usize, u32, u32, u32)> = Vec::new(); // face, at vertex, neighbor, new

        let bverts: BTreeSet<u32> = edges.iter().flat_map(|e| [e.0, e.1]).collect();
        for &v in &bverts {
            let (order, closed) = fans[&v].clone();
            let n = order.len();
            // spoke after fan face i = next vertex of that face's corner at v
            let corner = |fi: usize| -> (u32, u32) {
                let f = &self.faces[fi];
                let k = f.iter().position(|&x| x == v).unwrap();
                (f[(k + f.len() - 1) % f.len()], f[(k + 1) % f.len()])
            };
            let spoke_sel = |w: u32| edges.contains(&ekey(v, w));
            // selected spoke between fan[i] and fan[i+1]
            let sel_after: Vec<bool> = (0..n).map(|i| spoke_sel(corner(order[i]).1)).collect();
            let k_closed = if closed { sel_after.iter().filter(|&&b| b).count() } else { 0 };
            let mut sectors: Vec<(Vec<usize>, u32, u32, bool, bool)> = Vec::new(); // faces, left spoke, right spoke, left_sel, right_sel
            let mut virt: Vec<(usize, u32, bool)> = Vec::new(); // (face index in order, spoke vertex, true if spoke is next of that face)
            if closed && k_closed == 1 && n >= 3 {
                let j = (0..n).find(|&i| sel_after[i]).unwrap(); // face order[j] has the selected spoke as next
                let g = order[(j + 1) % n]; // face across the selected spoke
                let f = order[j];
                let mid: Vec<usize> = (2..n).map(|o| order[(j + o) % n]).collect(); // faces between g and f going around... exclude g and f
                // order around: f, g, then rest up to f again; mid = rest
                let rest: Vec<usize> = (1..n - 1).map(|o| order[(j + 1 + o) % n]).collect();
                let _ = mid;
                let sp = corner(f).1;
                // sectors: [g], rest, [f]  (fan order starting after the selected spoke)
                let g_left = corner(g).0; // == sp
                let g_right = corner(g).1;
                sectors.push((vec![g], g_left, g_right, true, false));
                if !rest.is_empty() {
                    let rl = corner(rest[0]).0;
                    let rr = corner(*rest.last().unwrap()).1;
                    sectors.push((rest.clone(), rl, rr, false, false));
                }
                let f_left = corner(f).0;
                sectors.push((vec![f], f_left, sp, false, true));
                virt.push((0, g_right, true));
                virt.push((sectors.len() - 1, f_left, false));
            } else if closed && k_closed >= 2 {
                let j = (0..n).find(|&i| sel_after[i]).unwrap();
                let mut cur: Vec<usize> = Vec::new();
                for o in 0..n {
                    let idx = (j + 1 + o) % n;
                    cur.push(order[idx]);
                    if sel_after[idx] {
                        let l = corner(*cur.first().unwrap()).0;
                        let r = corner(*cur.last().unwrap()).1;
                        sectors.push((std::mem::take(&mut cur), l, r, true, true));
                    }
                }
            } else if !closed {
                let mut cur: Vec<usize> = Vec::new();
                for i in 0..n {
                    cur.push(order[i]);
                    let end = i == n - 1;
                    if end || sel_after[i] {
                        let l = corner(*cur.first().unwrap()).0;
                        let r = corner(*cur.last().unwrap()).1;
                        let ls = !sectors.is_empty();
                        sectors.push((std::mem::take(&mut cur), l, r, ls, !end));
                    }
                }
            } else {
                continue;
            }
            // positions
            let pv = self.pos[v as usize];
            let mut vf = VFan { sectors: Vec::new(), closed, virtual_ins: Vec::new() };
            for (si, (sf, l, r, ls, rs)) in sectors.iter().enumerate() {
                let ul = (self.pos[*l as usize] - pv).normalized();
                let ur = (self.pos[*r as usize] - pv).normalized();
                let mut minlen = f32::INFINITY;
                for fi in sf {
                    let (p, nx) = corner(*fi);
                    minlen = minlen.min(self.pos[p as usize].distance(pv)).min(self.pos[nx as usize].distance(pv));
                }
                let mut centroid = Vec3::ZERO;
                for fi in sf {
                    centroid += self.face_center(*fi) - pv;
                }
                let virtual_left = virt.iter().any(|(s, _, nextflag)| *s == si && !*nextflag);
                let virtual_right = virt.iter().any(|(s, _, nextflag)| *s == si && *nextflag);
                let off = offset;
                let pos = if sectors.len() == 3 && closed && k_closed == 1 && (si == 0 || si == sectors.len() - 1) {
                    // face next to the single selected edge: slide along its other spoke
                    let (u_sel, u_other) = if si == 0 { (ul, ur) } else { (ur, ul) };
                    let sin_t = (1.0 - u_sel.dot(u_other).clamp(-1.0, 1.0).powi(2)).sqrt().max(0.2);
                    let d = (off / sin_t).min(minlen * 0.45);
                    pv + u_other * d
                } else if sectors.len() == 2 && closed && k_closed == 1 {
                    pv
                } else if !closed && (*ls != *rs) {
                    let (u_other, _) = if *ls { (ur, ul) } else { (ul, ur) };
                    let u_sel = if *ls { ul } else { ur };
                    let sin_t = (1.0 - u_sel.dot(u_other).clamp(-1.0, 1.0).powi(2)).sqrt().max(0.2);
                    let d = (off / sin_t).min(minlen * 0.45);
                    pv + u_other * d
                } else if *ls && *rs && !(closed && k_closed == 1) {
                    let mut bis = ul + ur;
                    if bis.length_sq() < 1e-8 {
                        bis = face_normal(&self.pos, &self.faces[sf[0]]).cross(ul);
                    }
                    let mut bis = bis.normalized();
                    if bis.dot(centroid) < 0.0 {
                        bis = -bis;
                    }
                    let half = (ul.dot(ur).clamp(-1.0, 1.0)).acos() * 0.5;
                    let d = (off / half.sin().max(0.2)).min(minlen * 0.45);
                    pv + bis * d
                } else {
                    pv
                };
                let _ = (virtual_left, virtual_right);
                let vid = if (pos - pv).length_sq() < 1e-14 && si == usize::MAX {
                    v
                } else if (pos - pv).length_sq() < 1e-14 {
                    v
                } else {
                    new_pos.push(pos);
                    (new_pos.len() - 1) as u32
                };
                vf.sectors.push(Sector { faces: sf.clone(), left: *l, right: *r, left_sel: *ls, right_sel: *rs, vtx: vid });
            }
            // corner replacement
            for sct in &vf.sectors {
                for &fi in &sct.faces {
                    corner_map.insert((fi, v), sct.vtx);
                }
            }
            // virtual spoke insertions (k == 1): insert the F/G sector vertex into the middle faces
            if closed && k_closed == 1 && vf.sectors.len() == 3 {
                // sector 0 = [g], 1 = rest, 2 = [f]; the spoke between g and rest, and rest and f
                let s_g = vf.sectors[0].vtx;
                let s_f = vf.sectors[2].vtx;
                let rest = vf.sectors[1].faces.clone();
                let g_right = vf.sectors[0].right;
                let f_left = vf.sectors[2].left;
                if let Some(&first) = rest.first() {
                    inserts.push((first, v, g_right, s_g));
                }
                if let Some(&last) = rest.last() {
                    inserts.push((last, v, f_left, s_f));
                }
                vf.virtual_ins.clear();
            }
            if closed && k_closed >= 3 {
                let mut poly: Vec<u32> = vf.sectors.iter().map(|s| s.vtx).collect();
                poly.reverse();
                patches.push(poly);
            } else if closed && k_closed == 1 && vf.sectors.len() == 3 {
                let mut poly: Vec<u32> = vf.sectors.iter().map(|s| s.vtx).collect();
                poly.reverse();
                patches.push(poly);
            }
            vfans.insert(v, vf);
        }
        // rewrite corners
        for (fi, f) in new_faces.iter_mut().enumerate() {
            for v in f.iter_mut() {
                if let Some(&n) = corner_map.get(&(fi, *v)) {
                    *v = n;
                }
            }
        }
        // virtual insertions: add `newv` next to corner `v` on the side of `nbr`
        for (fi, v, nbr, newv) in inserts {
            let f = &mut new_faces[fi];
            if let Some(k) = f.iter().position(|&x| x == v) {
                let n = f.len();
                if f[(k + 1) % n] == nbr {
                    f.insert(k + 1, newv);
                } else {
                    f.insert(k, newv);
                }
            }
        }
        let first_new = new_faces.len();
        // strips
        for &(a, b) in &edges {
            let f = dir_face[&(a, b)];
            let g = dir_face[&(b, a)];
            let sv = |face: usize, v: u32| corner_map.get(&(face, v)).copied().unwrap_or(v);
            new_faces.push(vec![sv(g, a), sv(g, b), sv(f, b), sv(f, a)]);
        }
        for p in patches {
            new_faces.push(p);
        }
        let _ = ef;
        self.pos = new_pos;
        self.faces = new_faces;
        let remap = self.cleanup();
        let mut out = Sel::default();
        for i in first_new..first_new + (self.faces.len().max(first_new)) {
            if i >= remap.f.len() {
                break;
            }
            if let Some(n) = remap.f[i] {
                out.faces.insert(n);
            }
        }
        out
    }

    // ------------------------------------------------------------------ splitting

    /// Insert a vertex at parameter `t` (from `a` to `b`) on the edge, updating all faces using it.
    pub fn split_edge(&mut self, a: u32, b: u32, t: f32) -> u32 {
        let p = self.pos[a as usize].lerp(self.pos[b as usize], t);
        self.pos.push(p);
        let m = (self.pos.len() - 1) as u32;
        for f in self.faces.iter_mut() {
            let n = f.len();
            for k in 0..n {
                let (x, y) = (f[k], f[(k + 1) % n]);
                if x == a && y == b {
                    f.insert(k + 1, m);
                    break;
                }
                if x == b && y == a {
                    f.insert(k + 1, m);
                    break;
                }
            }
        }
        m
    }

    /// Split the face containing both vertices by an edge between them.
    pub fn split_face_between(&mut self, v1: u32, v2: u32) -> Option<(usize, usize)> {
        for fi in 0..self.faces.len() {
            let f = &self.faces[fi];
            let (i, j) = match (f.iter().position(|&x| x == v1), f.iter().position(|&x| x == v2)) {
                (Some(i), Some(j)) if i != j => (i, j),
                _ => continue,
            };
            let n = f.len();
            if (i + 1) % n == j || (j + 1) % n == i {
                continue;
            }
            let mut a = Vec::new();
            let mut k = i;
            loop {
                a.push(f[k]);
                if k == j {
                    break;
                }
                k = (k + 1) % n;
            }
            let mut b = Vec::new();
            let mut k = j;
            loop {
                b.push(f[k]);
                if k == i {
                    break;
                }
                k = (k + 1) % n;
            }
            self.faces[fi] = a;
            self.faces.push(b);
            return Some((fi, self.faces.len() - 1));
        }
        None
    }

    /// Ring of quads through `e` as ordered (start, end) edge pairs; `start` sides stay consistent.
    pub fn edge_ring(&self, e: Edge) -> Vec<(u32, u32)> {
        let ef = self.edge_faces();
        let faces = match ef.get(&e) {
            Some(f) => f.clone(),
            None => return vec![],
        };
        let mut ring = vec![(e.0, e.1)];
        let key0 = e;
        let walk = |f0: usize, start: (u32, u32), out: &mut Vec<(u32, u32)>| {
            let (mut s, mut en) = start;
            let mut f = f0;
            for _ in 0..self.faces.len() + 2 {
                let face = &self.faces[f];
                if face.len() != 4 {
                    return;
                }
                let i = match (0..4).find(|&i| ekey(face[i], face[(i + 1) % 4]) == ekey(s, en)) {
                    Some(i) => i,
                    None => return,
                };
                let (p0, p1, p2, p3) = (face[i], face[(i + 1) % 4], face[(i + 2) % 4], face[(i + 3) % 4]);
                let (ns, ne) = if p0 == s { (p3, p2) } else { (p2, p3) };
                let _ = p1;
                let key = ekey(ns, ne);
                if key == key0 {
                    return;
                }
                out.push((ns, ne));
                let others = match ef.get(&key) {
                    Some(o) => o,
                    None => return,
                };
                match others.iter().copied().find(|&g| g != f) {
                    Some(g) => f = g,
                    None => return,
                }
                s = ns;
                en = ne;
            }
        };
        let mut fwd = Vec::new();
        walk(faces[0], (e.0, e.1), &mut fwd);
        ring.extend(fwd);
        if faces.len() > 1 {
            let mut back = Vec::new();
            walk(faces[1], (e.0, e.1), &mut back);
            // keep the loop in order: reversed backward walk, start edge, forward walk
            let mut fresh: Vec<(u32, u32)> = Vec::new();
            for p in back {
                if !ring.iter().any(|q| ekey(q.0, q.1) == ekey(p.0, p.1)) {
                    fresh.push(p);
                }
            }
            fresh.reverse();
            fresh.extend(ring);
            ring = fresh;
        }
        ring
    }

    /// Insert an edge loop across the ring through `e` at `t` along the stored edge direction.
    pub fn insert_edge_loop(&mut self, e: Edge, t: f32) -> Sel {
        let ring = self.edge_ring(e);
        if ring.is_empty() {
            return Sel::default();
        }
        let mut mids: HashMap<Edge, u32> = HashMap::new();
        for &(s, en) in &ring {
            let p = self.pos[s as usize].lerp(self.pos[en as usize], t);
            self.pos.push(p);
            mids.insert(ekey(s, en), (self.pos.len() - 1) as u32);
        }
        let old = std::mem::take(&mut self.faces);
        let mut out = Sel::default();
        for f in old {
            let n = f.len();
            let hits: Vec<usize> = (0..n).filter(|&k| mids.contains_key(&ekey(f[k], f[(k + 1) % n]))).collect();
            if n == 4 && hits.len() == 2 && (hits[1] + 4 - hits[0]) % 4 == 2 {
                let k = hits[0];
                let (p0, p1, p2, p3) = (f[k], f[(k + 1) % 4], f[(k + 2) % 4], f[(k + 3) % 4]);
                let m1 = mids[&ekey(p0, p1)];
                let m2 = mids[&ekey(p2, p3)];
                self.faces.push(vec![p0, m1, m2, p3]);
                self.faces.push(vec![m1, p1, p2, m2]);
                out.edges.insert(ekey(m1, m2));
            } else if hits.is_empty() {
                self.faces.push(f);
            } else {
                let mut g = Vec::with_capacity(n + hits.len());
                for k in 0..n {
                    g.push(f[k]);
                    if let Some(&m) = mids.get(&ekey(f[k], f[(k + 1) % n])) {
                        g.push(m);
                    }
                }
                self.faces.push(g);
            }
        }
        out.verts = mids.values().copied().collect();
        out
    }

    /// Offset Edge Loop: two new loops, one on each side of the loop through `e`, `offset` (0..0.5) of the
    /// way along the crossing edges. Returns the new loops' edges.
    pub fn offset_edge_loop(&mut self, e: Edge, offset: f32) -> Sel {
        let off = offset.clamp(0.02, 0.5);
        let ef = self.edge_faces();
        let mut crossing: Vec<(u32, u32)> = Vec::new();
        if let Some(faces) = ef.get(&e) {
            for &f in faces {
                let fv = &self.faces[f];
                let n = fv.len();
                if let Some(i) = (0..n).find(|&i| ekey(fv[i], fv[(i + 1) % n]) == e) {
                    // the face edge leaving e.0 away from the loop
                    let (a, b) = (fv[i], fv[(i + 1) % n]);
                    let (prev, next) = (fv[(i + n - 1) % n], fv[(i + 2) % n]);
                    if fv.len() == 4 {
                        if a == e.0 {
                            crossing.push((a, prev));
                        } else {
                            crossing.push((b, next));
                        }
                    }
                }
            }
        }
        let mut out = Sel::default();
        for (from, to) in crossing {
            if from == to {
                continue;
            }
            let k = ekey(from, to);
            // a ring parameter runs from the smaller vertex index
            let t = if from == k.0 { off } else { 1.0 - off };
            let r = self.insert_edge_loop(k, t);
            out.edges.extend(r.edges);
            out.verts.extend(r.verts);
        }
        out
    }

    /// Slide Edge Tool: move the vertices of the loop through each edge toward their crossing neighbors.
    /// `amount` is -1..1; positive slides toward the side of the first crossing edge found.
    pub fn slide_edges(&mut self, edges: &BTreeSet<Edge>, amount: f32) {
        let mut loop_edges: BTreeSet<Edge> = BTreeSet::new();
        for &e in edges {
            loop_edges.insert(e);
        }
        let verts: BTreeSet<u32> = loop_edges.iter().flat_map(|e| [e.0, e.1]).collect();
        let all = self.edge_list();
        let mut nbrs: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(a, b) in &all {
            nbrs.entry(a).or_default().push(b);
            nbrs.entry(b).or_default().push(a);
        }
        let amt = amount.clamp(-1.0, 1.0);
        let mut reference: Option<Vec3> = None;
        let mut moves: Vec<(u32, Vec3)> = Vec::new();
        for &v in &verts {
            let cands: Vec<u32> = nbrs.get(&v).cloned().unwrap_or_default().into_iter().filter(|&w| !loop_edges.contains(&ekey(v, w))).collect();
            if cands.is_empty() {
                continue;
            }
            // reference side: the first vertex fixes it, the others follow by direction
            let dirs: Vec<(u32, Vec3)> = cands.iter().map(|&w| (w, self.pos[w as usize] - self.pos[v as usize])).collect();
            let pick = match reference {
                None => {
                    reference = Some(dirs[0].1);
                    dirs[0]
                }
                Some(r) => *dirs.iter().max_by(|a, b| a.1.dot(r).partial_cmp(&b.1.dot(r)).unwrap()).unwrap(),
            };
            let opposite = dirs.iter().filter(|d| d.0 != pick.0).min_by(|a, b| a.1.dot(pick.1).partial_cmp(&b.1.dot(pick.1)).unwrap()).copied();
            let target = if amt >= 0.0 { Some(pick.1) } else { opposite.map(|o| o.1) };
            if let Some(d) = target {
                moves.push((v, d * amt.abs()));
            }
        }
        for (v, d) in moves {
            self.pos[v as usize] += d;
        }
    }

    /// Keep the part of the mesh on the positive side of the plane `pos[axis] = offset` (or the negative
    /// side when `keep_positive` is false); faces crossing the plane are cut.
    pub fn clip_plane(&mut self, axis: usize, offset: f32, keep_positive: bool) {
        let sgn = if keep_positive { 1.0 } else { -1.0 };
        let comp = |p: Vec3| match axis {
            0 => p.x,
            1 => p.y,
            _ => p.z,
        };
        let d: Vec<f32> = self.pos.iter().map(|p| (comp(*p) - offset) * sgn).collect();
        let eps = 1e-5;
        let mut cuts: HashMap<Edge, u32> = HashMap::new();
        let old = std::mem::take(&mut self.faces);
        let mut faces: Vec<Vec<u32>> = Vec::new();
        for f in old {
            let n = f.len();
            let mut out: Vec<u32> = Vec::new();
            for i in 0..n {
                let (a, b) = (f[i], f[(i + 1) % n]);
                let (da, db) = (d[a as usize], d[b as usize]);
                let ina = da >= -eps;
                let inb = db >= -eps;
                if ina {
                    out.push(a);
                }
                if (da > eps && db < -eps) || (da < -eps && db > eps) {
                    let k = ekey(a, b);
                    let m = *cuts.entry(k).or_insert_with(|| {
                        let t = da / (da - db);
                        let p = self.pos[a as usize].lerp(self.pos[b as usize], t);
                        self.pos.push(p);
                        (self.pos.len() - 1) as u32
                    });
                    out.push(m);
                }
                let _ = inb;
            }
            out.dedup();
            while out.len() > 1 && out.first() == out.last() {
                out.pop();
            }
            if out.len() >= 3 {
                faces.push(out);
            }
        }
        self.faces = faces;
        // vertices exactly on the plane stay; the clipped-away ones are dropped
        self.cleanup();
        let n = self.pos.len();
        for i in 0..n {
            let p = self.pos[i];
            let c = comp(p);
            if (c - offset).abs() < eps {
                let mut q = p;
                match axis {
                    0 => q.x = offset,
                    1 => q.y = offset,
                    _ => q.z = offset,
                }
                self.pos[i] = q;
            }
        }
    }

    /// Mirror Cut: clip at the plane and mirror the kept half across it so the halves weld.
    pub fn mirror_cut(&mut self, axis: usize, offset: f32, keep_positive: bool) {
        self.clip_plane(axis, offset, keep_positive);
        let n = self.pos.len() as u32;
        let flip = |mut p: Vec3| {
            match axis {
                0 => p.x = 2.0 * offset - p.x,
                1 => p.y = 2.0 * offset - p.y,
                _ => p.z = 2.0 * offset - p.z,
            }
            p
        };
        let mirrored: Vec<Vec3> = self.pos.iter().map(|p| flip(*p)).collect();
        self.pos.extend(mirrored);
        let extra: Vec<Vec<u32>> = self.faces.iter().map(|f| f.iter().rev().map(|&v| v + n).collect()).collect();
        self.faces.extend(extra);
        *self = Poly::welded(&self.to_mesh(), 1e-4);
    }

    /// Edges that belong to one face only.
    pub fn border_edges(&self) -> BTreeSet<Edge> {
        self.edge_faces().into_iter().filter(|(_, f)| f.len() == 1).map(|(e, _)| e).collect()
    }

    // ------------------------------------------------------------------ fill hole

    /// Fill gaps with n-gons. Every closed border loop that contains one of the `only` edges is filled
    /// (the whole hole, even if just a part of its rim was selected); without `only`, every hole is.
    /// Selected border edges that form an open chain (no complete rim) are closed into a polygon by
    /// joining the ends of the chain. Returns the new faces.
    pub fn fill_holes(&mut self, only: Option<&BTreeSet<Edge>>) -> Sel {
        let ef = self.edge_faces();
        let is_border = |a: u32, b: u32| ef.get(&ekey(a, b)).map(|v| v.len() == 1).unwrap_or(false);
        // border edges in face direction: tail -> head
        let mut border: Vec<(u32, u32)> = Vec::new();
        let mut by_head: HashMap<u32, Vec<u32>> = HashMap::new();
        for f in &self.faces {
            for i in 0..f.len() {
                let (a, b) = (f[i], f[(i + 1) % f.len()]);
                if a != b && is_border(a, b) {
                    border.push((a, b));
                    by_head.entry(b).or_default().push(a);
                }
            }
        }
        border.sort();
        let mut used: HashSet<(u32, u32)> = HashSet::new();
        let mut new_faces: Vec<Vec<u32>> = Vec::new();
        // closed rims
        for &(a, b) in &border {
            if used.contains(&(a, b)) {
                continue;
            }
            // the hole polygon runs b -> a -> x ... until it returns to b
            let mut poly = vec![b, a];
            let mut chain = vec![(a, b)];
            let mut cur = a;
            let mut closed = false;
            for _ in 0..border.len() + 2 {
                let next = by_head.get(&cur).and_then(|tails| tails.iter().copied().find(|&t| !used.contains(&(t, cur)) && !chain.contains(&(t, cur))));
                match next {
                    Some(t) => {
                        chain.push((t, cur));
                        if t == b {
                            closed = true;
                            break;
                        }
                        poly.push(t);
                        cur = t;
                    }
                    None => break,
                }
            }
            if closed && poly.len() >= 3 {
                let wanted = only.map(|o| chain.iter().any(|(x, y)| o.contains(&ekey(*x, *y)))).unwrap_or(true);
                for e in &chain {
                    used.insert(*e);
                }
                if wanted {
                    new_faces.push(poly);
                }
            }
        }
        // open chains of selected border edges: close them with one polygon
        if let Some(sel) = only {
            let mut open: Vec<(u32, u32)> = border.iter().copied().filter(|&(a, b)| !used.contains(&(a, b)) && sel.contains(&ekey(a, b))).collect();
            open.sort();
            let mut taken: HashSet<(u32, u32)> = HashSet::new();
            for &(a, b) in &open {
                if taken.contains(&(a, b)) {
                    continue;
                }
                // grow the chain backward and forward along selected edges
                let mut chain: VecDeque<u32> = VecDeque::from([a, b]);
                taken.insert((a, b));
                loop {
                    let tail = *chain.back().unwrap();
                    match open.iter().find(|&&(x, y)| x == tail && !taken.contains(&(x, y)) && !chain.contains(&y)) {
                        Some(&(x, y)) => {
                            taken.insert((x, y));
                            chain.push_back(y);
                        }
                        None => break,
                    }
                }
                loop {
                    let head = *chain.front().unwrap();
                    match open.iter().find(|&&(x, y)| y == head && !taken.contains(&(x, y)) && !chain.contains(&x)) {
                        Some(&(x, y)) => {
                            taken.insert((x, y));
                            chain.push_front(x);
                        }
                        None => break,
                    }
                }
                if chain.len() >= 3 {
                    // the new polygon runs against the rim direction so it faces the same way as its neighbours
                    let mut poly: Vec<u32> = chain.into_iter().collect();
                    poly.reverse();
                    new_faces.push(poly);
                }
            }
        }
        let first = self.faces.len() as u32;
        let mut out = Sel::default();
        for (i, f) in new_faces.into_iter().enumerate() {
            self.faces.push(f);
            out.faces.insert(first + i as u32);
        }
        out
    }

    /// Connect two border edge chains (two holes, or the two sides of a gap) with a strip of quads.
    /// `edges` must contain only border edges forming exactly two chains with the same number of
    /// edges, both closed loops or both open. Returns the new faces.
    pub fn bridge(&mut self, edges: &BTreeSet<Edge>) -> Result<Sel, String> {
        let ef = self.edge_faces();
        // border edges in face direction, restricted to the selection
        let mut next: HashMap<u32, u32> = HashMap::new();
        let mut prev: HashMap<u32, u32> = HashMap::new();
        let mut used = 0;
        for f in &self.faces {
            for i in 0..f.len() {
                let (a, b) = (f[i], f[(i + 1) % f.len()]);
                let k = ekey(a, b);
                if edges.contains(&k) && ef.get(&k).map(|v| v.len() == 1).unwrap_or(false) {
                    if next.insert(a, b).is_some() || prev.insert(b, a).is_some() {
                        return Err("The selected edges branch: select two simple edge loops or chains".to_string());
                    }
                    used += 1;
                }
            }
        }
        if used == 0 {
            return Err("Select border edges (edges with a face on one side only) of two holes or gaps".to_string());
        }
        // chains: start at vertices without a predecessor, then whatever remains are closed loops
        let mut chains: Vec<(Vec<u32>, bool)> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        for &start in next.keys().filter(|v| !prev.contains_key(v)) {
            let mut c = vec![start];
            seen.insert(start);
            let mut cur = start;
            while let Some(&n) = next.get(&cur) {
                c.push(n);
                seen.insert(n);
                cur = n;
            }
            chains.push((c, false));
        }
        let mut starts: Vec<u32> = next.keys().copied().filter(|v| !seen.contains(v)).collect();
        starts.sort_unstable();
        for start in starts {
            if seen.contains(&start) {
                continue;
            }
            let mut c = vec![start];
            seen.insert(start);
            let mut cur = next[&start];
            while cur != start {
                c.push(cur);
                seen.insert(cur);
                cur = next[&cur];
            }
            chains.push((c, true));
        }
        if chains.len() != 2 {
            return Err(format!("Found {} edge chains: select the border edges of exactly two holes or gaps", chains.len()));
        }
        let (a, a_closed) = chains[0].clone();
        let (mut b, b_closed) = chains[1].clone();
        if a_closed != b_closed {
            return Err("One selection is a closed loop and the other an open chain".to_string());
        }
        if a.len() != b.len() {
            return Err(format!("The two sides have {} and {} vertices: they need the same number of edges", a.len(), b.len()));
        }
        // the strip runs against the direction of B so winding stays consistent
        b.reverse();
        let n = a.len();
        if a_closed {
            // pick the rotation of B that keeps the quads shortest
            let cost = |s: usize| -> f32 { (0..n).map(|i| self.pos[a[i] as usize].distance(self.pos[b[(i + s) % n] as usize])).sum() };
            let best = (0..n).min_by(|x, y| cost(*x).partial_cmp(&cost(*y)).unwrap()).unwrap_or(0);
            b.rotate_left(best);
        }
        let first = self.faces.len() as u32;
        let segs = if a_closed { n } else { n - 1 };
        let mut out = Sel::default();
        for i in 0..segs {
            let j = (i + 1) % n;
            self.faces.push(vec![a[j], a[i], b[i], b[j]]);
            out.faces.insert(first + i as u32);
        }
        Ok(out)
    }

    /// Merge border edges in pairs: each selected border edge is joined to the nearest other selected
    /// one (matching their ends so the pair does not twist), and the matched vertices become one.
    /// Returns the merged vertices (indices after cleanup) and how many pairs were joined.
    pub fn merge_edge_pairs(&mut self, edges: &BTreeSet<Edge>) -> (Sel, usize) {
        let ef = self.edge_faces();
        let list: Vec<Edge> = edges.iter().copied().filter(|e| ef.get(e).map(|v| v.len() == 1).unwrap_or(false)).collect();
        let d = |a: u32, b: u32| self.pos[a as usize].distance(self.pos[b as usize]);
        // all candidate pairs with the cost of the better end matching
        let mut cand: Vec<(f32, usize, usize, bool)> = Vec::new();
        for i in 0..list.len() {
            for j in i + 1..list.len() {
                let (e, f) = (list[i], list[j]);
                if e.0 == f.0 || e.0 == f.1 || e.1 == f.0 || e.1 == f.1 {
                    continue;
                }
                let straight = d(e.0, f.0) + d(e.1, f.1);
                let crossed = d(e.0, f.1) + d(e.1, f.0);
                cand.push((straight.min(crossed), i, j, crossed < straight));
            }
        }
        cand.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut taken = vec![false; list.len()];
        let mut groups: Vec<Vec<u32>> = Vec::new();
        for (_, i, j, crossed) in cand {
            if taken[i] || taken[j] {
                continue;
            }
            taken[i] = true;
            taken[j] = true;
            let (e, f) = (list[i], list[j]);
            let (f0, f1) = if crossed { (f.1, f.0) } else { (f.0, f.1) };
            groups.push(vec![e.0, f0]);
            groups.push(vec![e.1, f1]);
        }
        let pairs = groups.len() / 2;
        // two pairs sharing a corner vertex must end up in one group
        let mut merged: Vec<Vec<u32>> = Vec::new();
        for g in groups {
            let hit: Vec<usize> = (0..merged.len()).filter(|&k| merged[k].iter().any(|v| g.contains(v))).collect();
            let mut m = g;
            for &k in hit.iter().rev() {
                m.extend(merged.remove(k));
            }
            m.sort_unstable();
            m.dedup();
            merged.push(m);
        }
        let remap = self.merge_groups(&merged);
        let mut out = Sel::default();
        for g in &merged {
            if let Some(Some(n)) = remap.v.get(g[0] as usize) {
                out.verts.insert(*n);
            }
        }
        (out, pairs)
    }

    // ------------------------------------------------------------------ global operations

    pub fn triangulate(&mut self) {
        let mut out = Vec::new();
        for f in &self.faces {
            for i in 1..f.len().saturating_sub(1) {
                out.push(vec![f[0], f[i], f[i + 1]]);
            }
        }
        self.faces = out;
    }

    pub fn reverse_faces(&mut self, which: Option<&BTreeSet<u32>>) {
        for (i, f) in self.faces.iter_mut().enumerate() {
            if which.map(|w| w.contains(&(i as u32))).unwrap_or(true) {
                f.reverse();
            }
        }
    }

    /// Mirror across the plane `axis = 0` (0 = X, 1 = Y, 2 = Z) and weld the seam.
    pub fn mirror(&mut self, axis: usize) {
        let n = self.pos.len() as u32;
        let flip = |mut p: Vec3| {
            match axis {
                0 => p.x = -p.x,
                1 => p.y = -p.y,
                _ => p.z = -p.z,
            }
            p
        };
        let mirrored: Vec<Vec3> = self.pos.iter().map(|p| flip(*p)).collect();
        self.pos.extend(mirrored);
        let extra: Vec<Vec<u32>> = self
            .faces
            .iter()
            .map(|f| f.iter().rev().map(|&v| v + n).collect())
            .collect();
        self.faces.extend(extra);
        let m = Poly::welded(&self.to_mesh(), 1e-4);
        *self = m;
    }

    /// One level of Catmull-Clark subdivision.
    pub fn smooth(&self) -> Poly {
        let ef = self.edge_faces();
        let fcount = self.faces.len();
        let fpts: Vec<Vec3> = (0..fcount).map(|i| self.face_center(i)).collect();
        let nverts = self.pos.len();
        let mut out = Poly::default();
        out.pos = self.pos.clone();
        // vertex adjacency
        let mut vfaces: Vec<Vec<usize>> = vec![Vec::new(); nverts];
        let mut vedges: Vec<Vec<Edge>> = vec![Vec::new(); nverts];
        for (fi, f) in self.faces.iter().enumerate() {
            for &v in f {
                vfaces[v as usize].push(fi);
            }
        }
        for e in ef.keys() {
            vedges[e.0 as usize].push(*e);
            vedges[e.1 as usize].push(*e);
        }
        let face_idx: Vec<u32> = (0..fcount)
            .map(|i| {
                out.pos.push(fpts[i]);
                (out.pos.len() - 1) as u32
            })
            .collect();
        let mut edge_idx: HashMap<Edge, u32> = HashMap::new();
        let mut keys: Vec<Edge> = ef.keys().copied().collect();
        keys.sort();
        for e in &keys {
            let fs = &ef[e];
            let mid = (self.pos[e.0 as usize] + self.pos[e.1 as usize]) * 0.5;
            let p = if fs.len() == 2 { (self.pos[e.0 as usize] + self.pos[e.1 as usize] + fpts[fs[0]] + fpts[fs[1]]) * 0.25 } else { mid };
            out.pos.push(p);
            edge_idx.insert(*e, (out.pos.len() - 1) as u32);
        }
        // move original vertices
        for v in 0..nverts {
            let es = &vedges[v];
            let fs = &vfaces[v];
            if es.is_empty() {
                continue;
            }
            let border: Vec<&Edge> = es.iter().filter(|e| ef[*e].len() != 2).collect();
            let pv = self.pos[v];
            let np = if !border.is_empty() {
                if border.len() == 2 {
                    let o = |e: &Edge| if e.0 as usize == v { e.1 } else { e.0 };
                    (self.pos[o(border[0]) as usize] + self.pos[o(border[1]) as usize] + pv * 6.0) * (1.0 / 8.0)
                } else {
                    pv
                }
            } else {
                let n = es.len() as f32;
                let mut favg = Vec3::ZERO;
                for &f in fs {
                    favg += fpts[f];
                }
                favg = favg * (1.0 / fs.len().max(1) as f32);
                let mut ravg = Vec3::ZERO;
                for e in es {
                    ravg += (self.pos[e.0 as usize] + self.pos[e.1 as usize]) * 0.5;
                }
                ravg = ravg * (1.0 / n);
                (favg + ravg * 2.0 + pv * (n - 3.0)) * (1.0 / n)
            };
            out.pos[v] = np;
        }
        for (fi, f) in self.faces.iter().enumerate() {
            let n = f.len();
            for i in 0..n {
                let prev = f[(i + n - 1) % n];
                let cur = f[i];
                let next = f[(i + 1) % n];
                out.faces.push(vec![cur, edge_idx[&ekey(cur, next)], face_idx[fi], edge_idx[&ekey(prev, cur)]]);
            }
        }
        out
    }
}

/// Split vertices so that faces meeting at less than `angle_deg` share a smooth normal and
/// sharper corners get hard edges. Used for display so edited meshes keep crisp creases.
/// Texture coordinate generation: 0 automatic (box), 1..3 planar X / Y / Z, 4 cylindrical, 5 spherical.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvMap {
    pub mode: u8,
    pub tile_u: f32,
    pub tile_v: f32,
    pub off_u: f32,
    pub off_v: f32,
    pub rot_deg: f32,
}

impl Default for UvMap {
    fn default() -> UvMap {
        UvMap { mode: 0, tile_u: 1.0, tile_v: 1.0, off_u: 0.0, off_v: 0.0, rot_deg: 0.0 }
    }
}

/// What a texture zone's projection is measured against: the bounds of the zone's own faces, and for
/// mode 6 (best plane) the plane that fits the faces.
struct ZoneCtx {
    lo: Vec3,
    size: Vec3,
    plane: Option<(Vec3, Vec3, f32, f32, f32, f32)>,
}

fn finish_uv(u: f32, v: f32, map: &UvMap) -> Vec2 {
    // rotate about the middle of the map, then tile and offset
    let (u, v) = if map.rot_deg.abs() > 1e-6 {
        let (sn, cs) = map.rot_deg.to_radians().sin_cos();
        let (du, dv) = (u - 0.5, v - 0.5);
        (du * cs - dv * sn + 0.5, du * sn + dv * cs + 0.5)
    } else {
        (u, v)
    };
    Vec2::new(u * map.tile_u + map.off_u, v * map.tile_v + map.off_v)
}

fn project_uv(p: Vec3, n: Vec3, lo: Vec3, size: Vec3, map: &UvMap) -> Vec2 {
    let nx = |v: f32, s: f32, o: f32| (v - o) / s.max(1e-4);
    let (u, v) = match map.mode {
        1 => (nx(p.z, size.z, lo.z), nx(p.y, size.y, lo.y)),
        2 => (nx(p.x, size.x, lo.x), nx(p.z, size.z, lo.z)),
        3 => (nx(p.x, size.x, lo.x), nx(p.y, size.y, lo.y)),
        4 => {
            let c = lo + size * 0.5;
            let a = (p.z - c.z).atan2(p.x - c.x);
            (a / std::f32::consts::TAU + 0.5, nx(p.y, size.y, lo.y))
        }
        5 => {
            let c = lo + size * 0.5;
            let a = (p.z - c.z).atan2(p.x - c.x);
            let r = (p - c).length().max(1e-4);
            (a / std::f32::consts::TAU + 0.5, ((p.y - c.y) / r).clamp(-1.0, 1.0).acos() / std::f32::consts::PI)
        }
        _ => {
            let (ax, ay, az) = (n.x.abs(), n.y.abs(), n.z.abs());
            if ax >= ay && ax >= az {
                let u = nx(p.z, size.z, lo.z);
                (if n.x < 0.0 { 1.0 - u } else { u }, nx(p.y, size.y, lo.y))
            } else if ay >= az {
                let v = nx(p.z, size.z, lo.z);
                (nx(p.x, size.x, lo.x), if n.y < 0.0 { 1.0 - v } else { v })
            } else {
                let u = nx(p.x, size.x, lo.x);
                (if n.z < 0.0 { 1.0 - u } else { u }, nx(p.y, size.y, lo.y))
            }
        }
    };
    finish_uv(u, v, map)
}

pub fn shading_split(m: &Mesh, angle_deg: f32, uvmap: Option<&UvMap>) -> Mesh {
    match uvmap {
        Some(u) => split_impl(m, angle_deg, &[], &[Some(*u)]),
        None => split_impl(m, angle_deg, &[], &[]),
    }
}

/// Like `shading_split`, with a separate texture mapping per face group: `face_zone[f]` picks the
/// entry of `maps` (index 0 is the base zone) used for face `f`; `None` keeps the stored UVs.
pub fn shading_split_zoned(m: &Mesh, angle_deg: f32, face_zone: &[u16], maps: &[Option<UvMap>]) -> Mesh {
    split_impl(m, angle_deg, face_zone, maps)
}

fn split_impl(m: &Mesh, angle_deg: f32, face_zone: &[u16], maps: &[Option<UvMap>]) -> Mesh {
    let n = m.positions.len();
    let fnorm: Vec<Vec3> = m.faces.iter().map(|f| face_normal(&m.positions, f)).collect();
    let cos_t = angle_deg.to_radians().cos();
    let zone_of = |fi: usize| -> usize { face_zone.get(fi).copied().unwrap_or(0) as usize };
    // per zone: bounds of its faces (the whole mesh when there is a single zone) and best plane
    let single = face_zone.is_empty();
    let ctxs: Vec<Option<ZoneCtx>> = maps
        .iter()
        .enumerate()
        .map(|(z, map)| {
            let map = (*map)?;
            let faces: Vec<usize> = (0..m.faces.len()).filter(|&fi| single || zone_of(fi) == z).collect();
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            let mut nsum = Vec3::ZERO;
            for &fi in &faces {
                nsum += fnorm[fi];
                for &v in &m.faces[fi] {
                    let p = m.positions[v as usize];
                    lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
                    hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
                }
            }
            if !lo.x.is_finite() {
                return None;
            }
            let plane = if map.mode == 6 {
                let nn = if nsum.length_sq() > 1e-12 { nsum.normalized() } else { Vec3::UP };
                let r = if nn.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
                let u = r.cross(nn).normalized();
                let v = nn.cross(u);
                let (mut u0, mut u1, mut v0, mut v1) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
                for &fi in &faces {
                    for &vi in &m.faces[fi] {
                        let p = m.positions[vi as usize];
                        let (a, b) = (p.dot(u), p.dot(v));
                        u0 = u0.min(a);
                        u1 = u1.max(a);
                        v0 = v0.min(b);
                        v1 = v1.max(b);
                    }
                }
                Some((u, v, u0, (u1 - u0).max(1e-4), v0, (v1 - v0).max(1e-4)))
            } else {
                None
            };
            Some(ZoneCtx { lo, size: hi - lo, plane })
        })
        .collect();
    let mut corner_uv: Vec<Vec<Vec2>> = Vec::new();
    let any_map = maps.iter().any(|m| m.is_some());
    if any_map {
        for (fi, f) in m.faces.iter().enumerate() {
            let z = if single { 0 } else { zone_of(fi) };
            let (map, ctx) = match (maps.get(z).and_then(|m| m.as_ref()), ctxs.get(z).and_then(|c| c.as_ref())) {
                (Some(map), Some(ctx)) => (map, ctx),
                _ => {
                    // no projection for this zone: keep the stored per-vertex UVs
                    corner_uv.push(f.iter().map(|&v| m.uvs.get(v as usize).copied().unwrap_or(Vec2::new(0.0, 0.0))).collect());
                    continue;
                }
            };
            let nn = fnorm[fi].normalized();
            let mut uvs: Vec<Vec2> = f
                .iter()
                .map(|&v| {
                    let p = m.positions[v as usize];
                    match &ctx.plane {
                        Some((u, vv, u0, su, v0, sv)) => finish_uv((p.dot(*u) - u0) / su, (p.dot(*vv) - v0) / sv, map),
                        None => project_uv(p, nn, ctx.lo, ctx.size, map),
                    }
                })
                .collect();
            if matches!(map.mode, 4 | 5) && !uvs.is_empty() {
                let (mut mn, mut mx) = (f32::INFINITY, f32::NEG_INFINITY);
                for u in &uvs {
                    mn = mn.min(u.x);
                    mx = mx.max(u.x);
                }
                let period = map.tile_u.abs().max(1e-3);
                if mx - mn > 0.5 * period {
                    for u in uvs.iter_mut() {
                        if u.x < mn + 0.5 * period {
                            u.x += period;
                        }
                    }
                }
            }
            corner_uv.push(uvs);
        }
    }
    let mut corners: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n]; // vertex -> (face, corner index)
    for (fi, f) in m.faces.iter().enumerate() {
        for (ci, &v) in f.iter().enumerate() {
            if (v as usize) < n {
                corners[v as usize].push((fi, ci));
            }
        }
    }
    let mut out = Mesh::new();
    let mut faces: Vec<Vec<u32>> = m.faces.iter().map(|f| vec![0; f.len()]).collect();
    for v in 0..n {
        let cs = &corners[v];
        if cs.is_empty() {
            continue;
        }
        let mut clusters: Vec<(Vec3, u32, Vec2)> = Vec::new();
        for &(fi, ci) in cs {
            let nf = fnorm[fi].normalized();
            let mut acc = Vec3::ZERO;
            for &(gj, _) in cs {
                let ng = fnorm[gj].normalized();
                if nf.length_sq() < 1e-12 || nf.dot(ng) >= cos_t {
                    acc += fnorm[gj];
                }
            }
            let nn = if acc.length_sq() > 0.0 { acc.normalized() } else { Vec3::UP };
            let uv = if any_map {
                corner_uv[fi][ci]
            } else {
                m.uvs.get(v).copied().unwrap_or(Vec2::new(0.0, 0.0))
            };
            let found = clusters
                .iter()
                .find(|(c, _, cu)| c.dot(nn) > 0.9999 && (cu.x - uv.x).abs() < 1e-4 && (cu.y - uv.y).abs() < 1e-4)
                .map(|(_, id, _)| *id);
            let id = match found {
                Some(id) => id,
                None => {
                    let id = out.add_vertex(m.positions[v], nn, uv);
                    clusters.push((nn, id, uv));
                    id
                }
            };
            faces[fi][ci] = id;
        }
    }
    out.faces = faces;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::primitives;

    fn cube() -> Poly {
        Poly::welded(&primitives::poly_cube(1.0, 1.0, 1.0, 1), 1e-5)
    }

    fn assert_closed(p: &Poly) {
        let ef = p.edge_faces();
        for (e, fs) in &ef {
            assert_eq!(fs.len(), 2, "edge {:?} has {} faces", e, fs.len());
        }
        // directed edges are unique (consistent winding)
        let mut seen = HashSet::new();
        for f in &p.faces {
            for i in 0..f.len() {
                assert!(seen.insert((f[i], f[(i + 1) % f.len()])), "duplicate directed edge");
            }
        }
    }

    fn euler(p: &Poly) -> i32 {
        p.pos.len() as i32 - p.edge_list().len() as i32 + p.faces.len() as i32
    }

    #[test]
    fn weld_cube() {
        let c = cube();
        assert_eq!((c.pos.len(), c.faces.len()), (8, 6));
        assert_closed(&c);
        assert_eq!(euler(&c), 2);
    }

    #[test]
    fn extrude_face() {
        let mut c = cube();
        let s = c.extrude_faces(&[0u32].into_iter().collect());
        assert_eq!(c.faces.len(), 10);
        assert_eq!(c.pos.len(), 12);
        assert_eq!(s.faces.len(), 1);
        assert_closed(&c);
        assert_eq!(euler(&c), 2);
    }

    #[test]
    fn extrude_two_faces_together() {
        let mut c = cube();
        c.extrude_faces(&[0u32, 1].into_iter().collect());
        assert_closed(&c);
        assert_eq!(euler(&c), 2);
    }

    #[test]
    fn delete_face_leaves_hole() {
        let mut c = cube();
        c.delete_faces(&[0u32].into_iter().collect());
        assert_eq!(c.faces.len(), 5);
        assert_eq!(c.pos.len(), 8);
    }

    #[test]
    fn delete_edge_merges() {
        let mut c = cube();
        let e = c.edge_list()[0];
        c.delete_edges(&[e].into_iter().collect(), true);
        assert_eq!(c.faces.len(), 5);
        assert!(c.faces.iter().any(|f| f.len() == 6));
    }

    #[test]
    fn bevel_one_edge() {
        let mut c = cube();
        let e = c.edge_list()[0];
        let s = c.bevel_edges(&[e].into_iter().collect(), 0.2);
        assert_closed(&c);
        assert_eq!(euler(&c), 2);
        assert!(!s.faces.is_empty());
        assert!(c.faces.len() > 6);
    }

    #[test]
    fn bevel_all_edges() {
        let mut c = cube();
        let all: BTreeSet<Edge> = c.edge_list().into_iter().collect();
        c.bevel_edges(&all, 0.1);
        assert_closed(&c);
        assert_eq!(euler(&c), 2);
        // 6 faces + 12 strips + 8 corner triangles
        assert_eq!(c.faces.len(), 26);
        assert_eq!(c.pos.len(), 24);
    }

    #[test]
    fn bevel_edge_chain() {
        let mut c = cube();
        // two edges meeting at a vertex
        let v = 0u32;
        let es: BTreeSet<Edge> = c.edge_list().into_iter().filter(|e| e.0 == v || e.1 == v).take(2).collect();
        c.bevel_edges(&es, 0.1);
        assert_closed(&c);
        assert_eq!(euler(&c), 2);
    }

    #[test]
    fn edge_loop_insert() {
        let mut c = Poly::welded(&primitives::poly_cube(1.0, 1.0, 1.0, 1), 1e-5);
        let e = c.edge_list()[0];
        c.insert_edge_loop(e, 0.5);
        assert_closed(&c);
        assert_eq!(euler(&c), 2);
        assert_eq!(c.faces.len(), 10);
    }

    #[test]
    fn smooth_cube() {
        let c = cube();
        let s = c.smooth();
        assert_eq!(s.faces.len(), 24);
        assert_closed(&s);
        assert_eq!(euler(&s), 2);
    }

    #[test]
    fn mirror_half() {
        let m = primitives::poly_plane(1.0, 1.0, 1);
        let mut p = Poly::welded(&m, 1e-5);
        for v in p.pos.iter_mut() {
            v.x += 0.5; // plane from x=0..1
        }
        p.mirror(0);
        assert_eq!(p.faces.len(), 2);
        assert_eq!(p.pos.len(), 6);
    }

    #[test]
    fn merge_and_collapse() {
        let mut c = cube();
        let e = c.edge_list()[0];
        c.collapse_edges(&[e].into_iter().collect());
        assert_eq!(c.pos.len(), 7);
        let mut c2 = cube();
        let all: BTreeSet<u32> = (0..8).collect();
        let g = c2.cluster_by_distance(&all, 1.1);
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn edge_loop_select() {
        let c = Poly::welded(&primitives::poly_cube(1.0, 1.0, 1.0, 2), 1e-5);
        let mut valence: HashMap<u32, usize> = HashMap::new();
        for e in c.edge_list() {
            *valence.entry(e.0).or_default() += 1;
            *valence.entry(e.1).or_default() += 1;
        }
        let e = c.edge_list().into_iter().find(|e| valence[&e.0] == 4 && valence[&e.1] == 4).unwrap();
        let l = c.edge_loop(e);
        assert_eq!(l.len(), 8);
    }

    #[test]
    fn split_face() {
        let mut c = cube();
        let f = c.faces[0].clone();
        c.split_face_between(f[0], f[2]).unwrap();
        assert_eq!(c.faces.len(), 7);
        assert_closed(&c);
    }

    #[test]
    fn fill_hole_closes_cube() {
        let mut c = cube();
        c.delete_faces(&[0u32].into_iter().collect());
        assert_eq!(c.faces.len(), 5);
        let ring: BTreeSet<Edge> = {
            let ef = c.edge_faces();
            ef.into_iter().filter(|(_, f)| f.len() == 1).map(|(e, _)| e).collect()
        };
        assert_eq!(ring.len(), 4);
        let s = c.fill_holes(Some(&ring));
        assert_eq!(s.faces.len(), 1);
        assert_eq!(c.faces.len(), 6);
        assert_closed(&c);
        assert_eq!(euler(&c), 2);
    }

    #[test]
    fn fill_hole_with_part_of_the_rim_selected_fills_the_whole_hole() {
        let mut c = cube();
        c.delete_faces(&[0u32].into_iter().collect());
        let one: BTreeSet<Edge> = {
            let ef = c.edge_faces();
            ef.into_iter().filter(|(_, f)| f.len() == 1).map(|(e, _)| e).take(1).collect()
        };
        let s = c.fill_holes(Some(&one));
        assert_eq!(s.faces.len(), 1);
        assert_closed(&c);
    }

    #[test]
    fn fill_closes_an_open_chain_of_border_edges() {
        // two triangles wound inconsistently along their shared edge: the border does not form closed rims
        let mut p = Poly {
            pos: vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 1.0), Vec3::new(2.0, 0.0, 0.0)],
            faces: vec![vec![0, 1, 2], vec![1, 2, 3]],
        };
        let chain: BTreeSet<Edge> = [ekey(0, 2), ekey(0, 1)].into_iter().collect();
        let s = p.fill_holes(Some(&chain));
        assert_eq!(s.faces.len(), 1, "one polygon closes the chain");
        let f = &p.faces[2];
        assert_eq!(f.len(), 3, "{:?}", f);
        for v in [0u32, 1, 2] {
            assert!(f.contains(&v), "{:?}", f);
        }
    }

    #[test]
    fn fill_without_a_selection_fills_every_hole() {
        let mut c = cube();
        c.delete_faces(&[0u32].into_iter().collect());
        // find a face that shares no vertex with the first hole (the opposite side)
        let hole: HashSet<u32> = c.edge_faces().into_iter().filter(|(_, f)| f.len() == 1).flat_map(|(e, _)| [e.0, e.1]).collect();
        let far = c.faces.iter().position(|f| f.iter().all(|v| !hole.contains(v))).unwrap() as u32;
        c.delete_faces(&[far].into_iter().collect());
        let s = c.fill_holes(None);
        assert_eq!(s.faces.len(), 2);
        assert_closed(&c);
    }

    #[test]
    fn merge_edges_joins_the_two_sides_of_a_seam() {
        // two unit quads with a gap of 0.01 between facing edges
        let mut p = Poly {
            pos: vec![
                Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(1.01, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 1.0), Vec3::new(1.01, 0.0, 1.0),
            ],
            faces: vec![vec![0, 3, 2, 1], vec![4, 7, 6, 5]],
        };
        let edges: BTreeSet<Edge> = [ekey(1, 2), ekey(4, 7)].into_iter().collect();
        let (sel, pairs) = p.merge_edge_pairs(&edges);
        assert_eq!(pairs, 1);
        assert_eq!(p.pos.len(), 6, "two vertex pairs became one each");
        assert_eq!(sel.verts.len(), 2);
        // the seam is now shared by both faces
        let ef = p.edge_faces();
        assert_eq!(ef.values().filter(|f| f.len() == 2).count(), 1);
        // the merged vertices sit halfway between the originals
        assert!(p.pos.iter().any(|v| (v.x - 1.005).abs() < 1e-5 && v.z.abs() < 1e-5));
    }

    #[test]
    fn merge_edges_ignores_edges_that_are_not_on_the_border() {
        let mut c = cube();
        let inner: BTreeSet<Edge> = c.edge_list().into_iter().take(2).collect();
        let (_, pairs) = c.merge_edge_pairs(&inner);
        assert_eq!(pairs, 0);
        assert_eq!(c.pos.len(), 8);
    }

    #[test]
    fn bridge_joins_two_holes_into_a_closed_tube() {
        // two cubes, the lower one open at the top and the upper one open at the bottom
        let a = cube();
        let mut p = a.clone();
        let base = p.pos.len() as u32;
        for v in &a.pos {
            p.pos.push(*v + Vec3::new(0.0, 3.0, 0.0));
        }
        for f in &a.faces {
            p.faces.push(f.iter().map(|v| v + base).collect());
        }
        let top = (0..6).max_by(|x, y| p.face_normal(*x).y.partial_cmp(&p.face_normal(*y).y).unwrap()).unwrap() as u32;
        let bottom = (6..12).min_by(|x, y| p.face_normal(*x).y.partial_cmp(&p.face_normal(*y).y).unwrap()).unwrap() as u32;
        p.delete_faces(&[top, bottom].into_iter().collect());
        let border: BTreeSet<Edge> = p.edge_faces().into_iter().filter(|(_, f)| f.len() == 1).map(|(e, _)| e).collect();
        assert_eq!(border.len(), 8);
        let s = p.bridge(&border).unwrap();
        assert_eq!(s.faces.len(), 4);
        assert_eq!(p.faces.len(), 10 + 4);
        // every edge now has two faces and directed edges are unique: a consistent closed surface
        assert_closed(&p);
        assert_eq!(euler(&p), 2, "two cubes joined by a tube are still a sphere topologically");
    }

    #[test]
    fn bridge_open_chains_and_errors() {
        // two separate quads, one above the other: bridge the top edge of the lower one to the bottom edge of the upper one
        let mut p = Poly {
            pos: vec![
                Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.0, 2.0, 0.0), Vec3::new(1.0, 3.0, 0.0), Vec3::new(0.0, 3.0, 0.0),
            ],
            faces: vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
        };
        let before = p.clone();
        let edges: BTreeSet<Edge> = [ekey(2, 3), ekey(4, 5)].into_iter().collect();
        let s = p.bridge(&edges).unwrap();
        assert_eq!(s.faces.len(), 1);
        assert_eq!(p.faces.len(), 3);
        // the strip's quad uses all four corner vertices and keeps the winding consistent with its neighbours
        let f = &p.faces[2];
        for v in [2u32, 3, 4, 5] {
            assert!(f.contains(&v), "{:?}", f);
        }
        let mut seen = HashSet::new();
        for f in &p.faces {
            for i in 0..f.len() {
                assert!(seen.insert((f[i], f[(i + 1) % f.len()])), "duplicate directed edge");
            }
        }
        // errors: one chain only, and chains of different length
        let one: BTreeSet<Edge> = [ekey(2, 3)].into_iter().collect();
        assert!(before.clone().bridge(&one).unwrap_err().contains("exactly two"));
        let mut q = before.clone();
        let uneven: BTreeSet<Edge> = [ekey(0, 1), ekey(1, 2), ekey(4, 5)].into_iter().collect();
        assert!(q.bridge(&uneven).unwrap_err().contains("same number"));
        // an inner edge is not a border edge
        let mut c = cube();
        let inner: BTreeSet<Edge> = c.edge_list().into_iter().take(2).collect();
        assert!(c.bridge(&inner).is_err());
    }

    #[test]
    fn shading_split_keeps_cube_hard() {
        let c = cube();
        let m = shading_split(&c.to_mesh(), 40.0, None);
        assert_eq!(m.positions.len(), 24);
    }
}
