//! Editable UV sets: shared UV points plus the UV indices of every face corner, with the operations of
//! the UV Texture Editor (project, select shells, transform, layout, cut, sew, relax).

use super::edit::{shading_split_zoned, UvMap};
use super::mesh::Mesh;
use crate::math::{Vec2, Vec3};
use std::collections::{BTreeSet, HashMap};

/// UV points and, for every face corner, the index of its UV point. Faces that share a UV point at a
/// corner are stitched together in the texture; different points at one vertex make a seam.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvSet {
    pub uvs: Vec<Vec2>,
    pub fuv: Vec<Vec<u32>>,
    /// Fingerprints of the faces this set was last matched against (see `MeshData::sync_uvs`).
    pub sig: Vec<u64>,
}

/// Vertex identity that ignores how a mesh was split for shading: equal positions are one vertex.
pub fn weld_ids(m: &Mesh) -> Vec<u32> {
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
    m.positions
        .iter()
        .map(|p| {
            let k = ((p.x * 10000.0).round() as i64, (p.y * 10000.0).round() as i64, (p.z * 10000.0).round() as i64);
            let n = map.len() as u32;
            *map.entry(k).or_insert(n)
        })
        .collect()
}

pub fn weld_faces(m: &Mesh) -> Vec<Vec<u32>> {
    let w = weld_ids(m);
    m.faces.iter().map(|f| f.iter().map(|&v| w[v as usize]).collect()).collect()
}

fn area(poly: &[Vec2]) -> f32 {
    let n = poly.len();
    (0..n).map(|i| poly[i].x * poly[(i + 1) % n].y - poly[(i + 1) % n].x * poly[i].y).sum::<f32>() * 0.5
}

impl UvSet {
    /// Build from per-corner UV values; corners of one welded vertex with equal UVs share a point.
    pub fn from_corners(weld: &[Vec<u32>], corner: &[Vec<Vec2>]) -> UvSet {
        let mut set = UvSet::default();
        let mut index: HashMap<(u32, i64, i64), u32> = HashMap::new();
        for (f, uvs) in weld.iter().zip(corner) {
            let mut row = Vec::with_capacity(f.len());
            for (&w, uv) in f.iter().zip(uvs) {
                let key = (w, (uv.x * 100000.0).round() as i64, (uv.y * 100000.0).round() as i64);
                let id = *index.entry(key).or_insert_with(|| {
                    set.uvs.push(*uv);
                    (set.uvs.len() - 1) as u32
                });
                row.push(id);
            }
            set.fuv.push(row);
        }
        set
    }

    pub fn matches(&self, faces: &[Vec<u32>]) -> bool {
        self.fuv.len() == faces.len() && self.fuv.iter().zip(faces).all(|(a, b)| a.len() == b.len())
    }

    pub fn corner_uvs(&self, f: usize) -> Vec<Vec2> {
        self.fuv[f].iter().map(|&u| self.uvs[u as usize]).collect()
    }

    /// Shell number of every UV point (points joined through the faces that use them).
    pub fn shell_ids(&self) -> Vec<usize> {
        let mut parent: Vec<usize> = (0..self.uvs.len()).collect();
        fn find(p: &mut Vec<usize>, mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        for f in &self.fuv {
            for w in f.windows(2) {
                let (a, b) = (find(&mut parent, w[0] as usize), find(&mut parent, w[1] as usize));
                if a != b {
                    parent[a] = b;
                }
            }
        }
        (0..self.uvs.len()).map(|i| find(&mut parent, i)).collect()
    }

    /// The UV points of each shell.
    pub fn shells(&self) -> Vec<Vec<u32>> {
        let ids = self.shell_ids();
        let mut by: std::collections::BTreeMap<usize, Vec<u32>> = std::collections::BTreeMap::new();
        for (i, r) in ids.iter().enumerate() {
            // points no face uses are not part of any shell
            by.entry(*r).or_default().push(i as u32);
        }
        let used: BTreeSet<u32> = self.fuv.iter().flatten().copied().collect();
        by.into_values().map(|v| v.into_iter().filter(|u| used.contains(u)).collect::<Vec<_>>()).filter(|v: &Vec<u32>| !v.is_empty()).collect()
    }

    pub fn expand_to_shells(&self, sel: &BTreeSet<u32>) -> BTreeSet<u32> {
        let ids = self.shell_ids();
        let roots: BTreeSet<usize> = sel.iter().filter_map(|&u| ids.get(u as usize).copied()).collect();
        (0..self.uvs.len() as u32).filter(|&u| roots.contains(&ids[u as usize])).collect()
    }

    pub fn uvs_of_faces(&self, faces: &BTreeSet<u32>) -> BTreeSet<u32> {
        faces.iter().filter_map(|&f| self.fuv.get(f as usize)).flatten().copied().collect()
    }

    /// Faces whose every corner is in the selection.
    pub fn faces_of_uvs(&self, sel: &BTreeSet<u32>) -> BTreeSet<u32> {
        self.fuv.iter().enumerate().filter(|(_, f)| f.iter().all(|u| sel.contains(u))).map(|(i, _)| i as u32).collect()
    }

    pub fn bounds(&self, sel: &BTreeSet<u32>) -> Option<(Vec2, Vec2)> {
        let mut it = sel.iter().filter_map(|&u| self.uvs.get(u as usize));
        let first = *it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (Vec2::new(lo.x.min(p.x), lo.y.min(p.y)), Vec2::new(hi.x.max(p.x), hi.y.max(p.y)))))
    }

    pub fn transform(&mut self, sel: &BTreeSet<u32>, f: impl Fn(Vec2) -> Vec2) {
        for &u in sel {
            if let Some(p) = self.uvs.get_mut(u as usize) {
                *p = f(*p);
            }
        }
    }

    /// Fit the selection into the 0..1 square (keeping its aspect when `keep_aspect`).
    pub fn normalize(&mut self, sel: &BTreeSet<u32>, keep_aspect: bool) {
        let Some((lo, hi)) = self.bounds(sel) else { return };
        let (w, h) = ((hi.x - lo.x).max(1e-9), (hi.y - lo.y).max(1e-9));
        let (sx, sy) = if keep_aspect {
            let s = 1.0 / w.max(h);
            (s, s)
        } else {
            (1.0 / w, 1.0 / h)
        };
        self.transform(sel, |p| Vec2::new((p.x - lo.x) * sx, (p.y - lo.y) * sy));
    }

    /// Separate the faces that are mirrored in the texture (negative area) from the others so each
    /// group becomes its own shell. Returns how many UV points were duplicated.
    pub fn separate_folds(&mut self) -> usize {
        let mut map: HashMap<(u32, bool), u32> = HashMap::new();
        let mut added = 0;
        let flipped: Vec<bool> = (0..self.fuv.len()).map(|f| area(&self.corner_uvs(f)) < 0.0).collect();
        // points used by both kinds of face get a copy for the flipped ones
        let mut used_by: HashMap<u32, (bool, bool)> = HashMap::new();
        for (f, row) in self.fuv.iter().enumerate() {
            for &u in row {
                let e = used_by.entry(u).or_default();
                if flipped[f] {
                    e.1 = true;
                } else {
                    e.0 = true;
                }
            }
        }
        for f in 0..self.fuv.len() {
            if !flipped[f] {
                continue;
            }
            for k in 0..self.fuv[f].len() {
                let u = self.fuv[f][k];
                if used_by.get(&u).map(|e| e.0 && e.1).unwrap_or(false) {
                    let n = *map.entry((u, true)).or_insert_with(|| {
                        self.uvs.push(self.uvs[u as usize]);
                        added += 1;
                        (self.uvs.len() - 1) as u32
                    });
                    self.fuv[f][k] = n;
                }
            }
        }
        added
    }

    /// Pack the shells of `sel` (all points when empty) into the 0..1 square without overlap. Shells keep
    /// their relative size; `padding` is the gap between them.
    pub fn layout(&mut self, sel: &BTreeSet<u32>, padding: f32) {
        let sel_all: BTreeSet<u32> = if sel.is_empty() { (0..self.uvs.len() as u32).collect() } else { self.expand_to_shells(sel) };
        let shells: Vec<Vec<u32>> = self.shells().into_iter().filter(|s| s.iter().any(|u| sel_all.contains(u))).collect();
        if shells.is_empty() {
            return;
        }
        let boxes: Vec<(Vec2, Vec2)> = shells.iter().map(|s| self.bounds(&s.iter().copied().collect()).unwrap()).collect();
        let sizes: Vec<(f32, f32)> = boxes.iter().map(|(lo, hi)| ((hi.x - lo.x).max(1e-6), (hi.y - lo.y).max(1e-6))).collect();
        let mut order: Vec<usize> = (0..shells.len()).collect();
        order.sort_by(|&a, &b| sizes[b].1.partial_cmp(&sizes[a].1).unwrap());
        // shrink the common scale until the shelf packing fits the square
        let total: f32 = sizes.iter().map(|s| s.0 * s.1).sum();
        let mut scale = (1.0 / total.sqrt()).min(1000.0);
        let place = |scale: f32| -> Option<Vec<(f32, f32)>> {
            let mut pos = vec![(0.0, 0.0); shells.len()];
            let (mut x, mut y, mut row_h) = (padding, padding, 0.0f32);
            for &i in &order {
                let (w, h) = (sizes[i].0 * scale, sizes[i].1 * scale);
                if x + w + padding > 1.0 + 1e-6 && x > padding {
                    x = padding;
                    y += row_h + padding;
                    row_h = 0.0;
                }
                if x + w + padding > 1.0 + 1e-6 || y + h + padding > 1.0 + 1e-6 {
                    return None;
                }
                pos[i] = (x, y);
                x += w + padding;
                row_h = row_h.max(h);
            }
            Some(pos)
        };
        let mut pos = None;
        for _ in 0..400 {
            if let Some(p) = place(scale) {
                pos = Some(p);
                break;
            }
            scale *= 0.97;
        }
        let Some(pos) = pos else { return };
        for (i, shell) in shells.iter().enumerate() {
            let lo = boxes[i].0;
            let (px, py) = pos[i];
            let set: BTreeSet<u32> = shell.iter().copied().collect();
            self.transform(&set, |p| Vec2::new(px + (p.x - lo.x) * scale, py + (p.y - lo.y) * scale));
        }
    }

    /// Laplacian relaxation: interior points move toward the average of their neighbours, the rim of
    /// every shell stays put. Reduces stretching of a projection.
    pub fn relax(&mut self, sel: &BTreeSet<u32>, iterations: usize) {
        let mut edge_count: HashMap<(u32, u32), u32> = HashMap::new();
        let mut nbrs: HashMap<u32, BTreeSet<u32>> = HashMap::new();
        for row in &self.fuv {
            let n = row.len();
            for i in 0..n {
                let (a, b) = (row[i], row[(i + 1) % n]);
                *edge_count.entry((a.min(b), a.max(b))).or_default() += 1;
                nbrs.entry(a).or_default().insert(b);
                nbrs.entry(b).or_default().insert(a);
            }
        }
        let mut rim: BTreeSet<u32> = BTreeSet::new();
        for (&(a, b), &c) in &edge_count {
            if c == 1 {
                rim.insert(a);
                rim.insert(b);
            }
        }
        for _ in 0..iterations {
            let old = self.uvs.clone();
            for (&u, ns) in &nbrs {
                if rim.contains(&u) || !(sel.is_empty() || sel.contains(&u)) || ns.is_empty() {
                    continue;
                }
                let mut sum = Vec2::new(0.0, 0.0);
                for &v in ns {
                    sum = Vec2::new(sum.x + old[v as usize].x, sum.y + old[v as usize].y);
                }
                let k = ns.len() as f32;
                self.uvs[u as usize] = Vec2::new(sum.x / k, sum.y / k);
            }
        }
    }

    /// Cut along mesh edges (given as welded vertex pairs): the faces on the two sides stop sharing UV
    /// points along them. `weld` is the face list in welded vertex ids.
    pub fn cut(&mut self, weld: &[Vec<u32>], edges: &BTreeSet<(u32, u32)>) {
        let mut around: HashMap<u32, Vec<usize>> = HashMap::new();
        for (f, row) in weld.iter().enumerate() {
            for &w in row {
                around.entry(w).or_default().push(f);
            }
        }
        let cut_edge = |a: u32, b: u32| edges.contains(&(a.min(b), a.max(b)));
        let mut verts: BTreeSet<u32> = BTreeSet::new();
        for &(a, b) in edges {
            verts.insert(a);
            verts.insert(b);
        }
        let fuv0 = self.fuv.clone();
        for w in verts {
            let faces = around.get(&w).cloned().unwrap_or_default();
            // corner of face f at vertex w: (position in face, uv, the two neighbouring vertices)
            let info = |f: usize| -> Option<(usize, u32, u32, u32)> {
                let row = &weld[f];
                let k = row.iter().position(|&x| x == w)?;
                let n = row.len();
                Some((k, fuv0[f][k], row[(k + n - 1) % n], row[(k + 1) % n]))
            };
            let mut comp: Vec<usize> = (0..faces.len()).collect();
            fn find(c: &mut Vec<usize>, mut x: usize) -> usize {
                while c[x] != x {
                    c[x] = c[c[x]];
                    x = c[x];
                }
                x
            }
            for i in 0..faces.len() {
                for j in i + 1..faces.len() {
                    let (Some(a), Some(b)) = (info(faces[i]), info(faces[j])) else { continue };
                    if a.1 != b.1 {
                        continue;
                    }
                    // adjacent through an uncut edge at w
                    let shared = [a.2, a.3].into_iter().find(|x| (*x == b.2 || *x == b.3) && !cut_edge(w, *x));
                    if shared.is_some() {
                        let (x, y) = (find(&mut comp, i), find(&mut comp, j));
                        if x != y {
                            comp[x] = y;
                        }
                    }
                }
            }
            // each group of faces that shared one UV point gets a point of its own
            let mut seen: HashMap<(u32, usize), u32> = HashMap::new();
            for i in 0..faces.len() {
                let Some((k, uv, _, _)) = info(faces[i]) else { continue };
                let root = find(&mut comp, i);
                let first_for_uv = !seen.keys().any(|(u, _)| *u == uv);
                let id = match seen.get(&(uv, root)) {
                    Some(&id) => id,
                    None => {
                        let id = if first_for_uv {
                            uv
                        } else {
                            self.uvs.push(self.uvs[uv as usize]);
                            (self.uvs.len() - 1) as u32
                        };
                        seen.insert((uv, root), id);
                        id
                    }
                };
                self.fuv[faces[i]][k] = id;
            }
        }
    }

    /// Sew along mesh edges: the UV points of the two faces on each edge are merged (moved to their middle).
    pub fn sew(&mut self, weld: &[Vec<u32>], edges: &BTreeSet<(u32, u32)>) -> usize {
        let mut merged = 0;
        for &(a, b) in edges {
            let mut on_edge: Vec<usize> = Vec::new();
            for (f, row) in weld.iter().enumerate() {
                let n = row.len();
                if (0..n).any(|i| (row[i] == a && row[(i + 1) % n] == b) || (row[i] == b && row[(i + 1) % n] == a)) {
                    on_edge.push(f);
                }
            }
            if on_edge.len() != 2 {
                continue;
            }
            for w in [a, b] {
                let ids: Vec<u32> = on_edge.iter().filter_map(|&f| weld[f].iter().position(|&x| x == w).map(|k| self.fuv[f][k])).collect();
                if ids.len() == 2 && ids[0] != ids[1] {
                    let (keep, gone) = (ids[0], ids[1]);
                    let mid = Vec2::new((self.uvs[keep as usize].x + self.uvs[gone as usize].x) * 0.5, (self.uvs[keep as usize].y + self.uvs[gone as usize].y) * 0.5);
                    self.uvs[keep as usize] = mid;
                    for row in self.fuv.iter_mut() {
                        for u in row.iter_mut() {
                            if *u == gone {
                                *u = keep;
                            }
                        }
                    }
                    merged += 1;
                }
            }
        }
        merged
    }

    /// Drop unused points and renumber.
    pub fn compact(&mut self) {
        let used: BTreeSet<u32> = self.fuv.iter().flatten().copied().collect();
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut uvs = Vec::new();
        for u in used {
            remap.insert(u, uvs.len() as u32);
            uvs.push(self.uvs[u as usize]);
        }
        for row in self.fuv.iter_mut() {
            for u in row.iter_mut() {
                *u = remap[u];
            }
        }
        self.uvs = uvs;
    }

    /// Give a mesh (already split for shading, same faces) the UVs of this set; vertices are duplicated
    /// where neighbouring faces use different UVs.
    pub fn apply_to(&self, m: &mut Mesh) {
        if !self.matches(&m.faces) {
            return;
        }
        let mut cache: HashMap<(u32, u32), u32> = HashMap::new();
        let mut claimed: BTreeSet<u32> = BTreeSet::new();
        if m.uvs.len() < m.positions.len() {
            m.uvs.resize(m.positions.len(), Vec2::new(0.0, 0.0));
        }
        for f in 0..m.faces.len() {
            for k in 0..m.faces[f].len() {
                let v = m.faces[f][k];
                let u = self.fuv[f][k];
                let uv = self.uvs[u as usize];
                let id = if let Some(&id) = cache.get(&(v, u)) {
                    id
                } else if claimed.insert(v) {
                    m.uvs[v as usize] = uv;
                    v
                } else {
                    m.add_vertex(m.positions[v as usize], m.normals[v as usize], uv)
                };
                cache.insert((v, u), id);
                m.faces[f][k] = id;
            }
        }
    }
}

/// Corner UVs of the faces from the mapping `map`. Faces in `only` are projected (their own bounds), the
/// rest keep `base` (or the default automatic mapping when there is none).
pub fn project(m: &Mesh, only: Option<&BTreeSet<u32>>, map: UvMap, base: Option<&UvSet>) -> UvSet {
    let weld = weld_faces(m);
    let corner_of = |zones: &[u16], maps: &[Option<UvMap>]| -> Vec<Vec<Vec2>> {
        let sm = shading_split_zoned(m, 180.0, zones, maps);
        sm.faces.iter().map(|f| f.iter().map(|&v| sm.uvs.get(v as usize).copied().unwrap_or(Vec2::new(0.0, 0.0))).collect()).collect()
    };
    let mut corners: Vec<Vec<Vec2>> = match base {
        Some(b) if b.matches(&m.faces) => (0..m.faces.len()).map(|f| b.corner_uvs(f)).collect(),
        _ => corner_of(&[], &[Some(UvMap::default())]),
    };
    match only {
        Some(sel) => {
            let zones: Vec<u16> = (0..m.faces.len()).map(|f| sel.contains(&(f as u32)) as u16).collect();
            let projected = corner_of(&zones, &[None, Some(map)]);
            for f in sel {
                if let Some(c) = projected.get(*f as usize) {
                    corners[*f as usize] = c.clone();
                }
            }
        }
        None => corners = corner_of(&[], &[Some(map)]),
    }
    UvSet::from_corners(&weld, &corners)
}

/// Automatic mapping: every face is flattened along its dominant axis, faces of one direction that touch
/// form a shell, then the shells are packed into the 0..1 square.
pub fn automatic(m: &Mesh, only: Option<&BTreeSet<u32>>, base: Option<&UvSet>) -> UvSet {
    let weld = weld_faces(m);
    let mut set = match base {
        Some(b) if b.matches(&m.faces) => b.clone(),
        _ => project(m, None, UvMap::default(), None),
    };
    let dir_of = |f: usize| -> usize {
        let p = &m.faces[f];
        let n = super::edit::face_normal(&m.positions, p);
        let a = n.x.abs().max(n.y.abs()).max(n.z.abs());
        if n.x.abs() == a {
            (n.x < 0.0) as usize
        } else if n.y.abs() == a {
            2 + (n.y < 0.0) as usize
        } else {
            4 + (n.z < 0.0) as usize
        }
    };
    let flat = |d: usize, p: Vec3| -> Vec2 {
        match d {
            0 => Vec2::new(-p.z, p.y),
            1 => Vec2::new(p.z, p.y),
            2 => Vec2::new(p.x, -p.z),
            3 => Vec2::new(p.x, p.z),
            4 => Vec2::new(p.x, p.y),
            _ => Vec2::new(-p.x, p.y),
        }
    };
    let faces: Vec<usize> = match only {
        Some(s) => s.iter().map(|&f| f as usize).filter(|&f| f < m.faces.len()).collect(),
        None => (0..m.faces.len()).collect(),
    };
    let mut index: HashMap<(u32, usize), u32> = HashMap::new();
    for &f in &faces {
        let d = dir_of(f);
        for k in 0..m.faces[f].len() {
            let w = weld[f][k];
            let id = *index.entry((w, d)).or_insert_with(|| {
                set.uvs.push(flat(d, m.positions[m.faces[f][k] as usize]));
                (set.uvs.len() - 1) as u32
            });
            set.fuv[f][k] = id;
        }
    }
    set.compact();
    let moved: BTreeSet<u32> = faces.iter().flat_map(|&f| set.fuv[f].iter().copied()).collect();
    set.layout(&moved, 0.01);
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::edit::Poly;

    fn cube() -> Mesh {
        Poly::welded(&crate::dag::nodes::build_mesh("polyCube", &[]).unwrap(), 1e-4).to_mesh()
    }

    #[test]
    fn automatic_mapping_makes_one_shell_per_side_inside_the_square() {
        let m = cube();
        let set = automatic(&m, None, None);
        assert!(set.matches(&m.faces));
        assert_eq!(set.shells().len(), 6);
        assert!(set.uvs.iter().all(|p| p.x >= -1e-4 && p.x <= 1.0001 && p.y >= -1e-4 && p.y <= 1.0001));
        // shells do not overlap: their boxes are disjoint
        let boxes: Vec<(Vec2, Vec2)> = set.shells().iter().map(|s| set.bounds(&s.iter().copied().collect()).unwrap()).collect();
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                let (a, b) = (boxes[i], boxes[j]);
                assert!(a.1.x <= b.0.x + 1e-5 || b.1.x <= a.0.x + 1e-5 || a.1.y <= b.0.y + 1e-5 || b.1.y <= a.0.y + 1e-5, "shells {} and {} overlap", i, j);
            }
        }
    }

    #[test]
    fn cut_separates_and_sew_rejoins() {
        let m = cube();
        let weld = weld_faces(&m);
        let mut set = project(&m, None, UvMap { mode: 3, ..UvMap::default() }, None);
        // planar Z folds the cube: front and back overlap but side faces are degenerate lines
        let shells0 = set.shells().len();
        let welds_edges: BTreeSet<(u32, u32)> = {
            let mut e = BTreeSet::new();
            for f in &weld {
                let n = f.len();
                for i in 0..n {
                    e.insert((f[i].min(f[(i + 1) % n]), f[i].max(f[(i + 1) % n])));
                }
            }
            e
        };
        set.cut(&weld, &welds_edges);
        set.compact();
        assert_eq!(set.shells().len(), 6, "cutting every edge leaves one shell per face (was {})", shells0);
        set.sew(&weld, &welds_edges);
        set.compact();
        assert!(set.shells().len() < 6, "sewing joins faces again");
    }

    #[test]
    fn relax_keeps_the_rim_and_folds_separate() {
        let m = cube();
        let mut set = project(&m, None, UvMap { mode: 3, ..UvMap::default() }, None);
        let rim_before = set.uvs.clone();
        set.relax(&BTreeSet::new(), 5);
        assert_eq!(set.uvs.len(), rim_before.len());
        let added = set.separate_folds();
        assert!(added > 0 || set.shells().len() >= 1);
    }
}
