//! Construction history: polygon operation nodes evaluated in order to produce a mesh shape's geometry.

use super::graph::{NodeId, Scene};
use super::nodes::{build_mesh, MeshData, NodeData, PolyKind, PolyOpData};
use super::plugs::Plug;
use crate::geo::deform;
use crate::geo::edit::{ekey, Edge, Poly, Sel};
use crate::math::Vec3;
use std::collections::BTreeSet;

fn pv(plugs: &[Plug], name: &str, default: f32) -> f32 {
    plugs.iter().find(|p| p.name == name).map(|p| p.value).unwrap_or(default)
}

fn faces_in_range(poly: &Poly, ints: &[u32]) -> BTreeSet<u32> {
    ints.iter().copied().filter(|&f| (f as usize) < poly.faces.len()).collect()
}

fn edges_in_range(poly: &Poly, ints: &[u32]) -> BTreeSet<Edge> {
    let n = poly.pos.len() as u32;
    ints.chunks_exact(2).filter(|c| c[0] < n && c[1] < n).map(|c| ekey(c[0], c[1])).collect()
}

fn verts_in_range(poly: &Poly, ints: &[u32]) -> BTreeSet<u32> {
    let n = poly.pos.len() as u32;
    ints.iter().copied().filter(|&v| v < n).collect()
}

/// Keep the deformation only on the member vertices (all when `members` is empty) and scale it by the envelope.
pub fn finish_deform(before: &[Vec3], poly: &mut Poly, members: &[u32], envelope: f32) {
    let env = envelope.clamp(0.0, 1.0);
    if members.is_empty() && (env - 1.0).abs() < 1e-6 {
        return;
    }
    let member: std::collections::HashSet<u32> = members.iter().copied().collect();
    for (i, p) in poly.pos.iter_mut().enumerate() {
        let Some(b) = before.get(i) else { continue };
        if !members.is_empty() && !member.contains(&(i as u32)) {
            *p = *b;
        } else {
            *p = b.lerp(*p, env);
        }
    }
}

/// Apply one history operation to `poly`; returns the selection the operation produces.
pub fn apply_poly_op(kind: PolyKind, ints: &[u32], source: &Option<Poly>, plugs: &[Plug], poly: &mut Poly) -> Sel {
    let mut out = Sel::default();
    match kind {
        PolyKind::Cube | PolyKind::Sphere | PolyKind::Cylinder | PolyKind::Plane | PolyKind::Cone | PolyKind::Torus => {
            let prim = match kind {
                PolyKind::Cube => "polyCube",
                PolyKind::Sphere => "polySphere",
                PolyKind::Cylinder => "polyCylinder",
                PolyKind::Cone => "polyCone",
                PolyKind::Torus => "polyTorus",
                _ => "polyPlane",
            };
            let params: Vec<(String, f32)> = plugs.iter().map(|p| (p.name.clone(), p.value)).collect();
            if let Some(m) = build_mesh(prim, &params) {
                *poly = Poly::welded(&m, 1e-4);
            }
        }
        PolyKind::Source => {
            if let Some(s) = source {
                *poly = s.clone();
            }
        }
        PolyKind::Tweak => {
            for p in plugs {
                if let Some(rest) = p.name.strip_prefix("pnts_") {
                    let mut it = rest.split('_');
                    if let (Some(i), Some(axis)) = (it.next().and_then(|v| v.parse::<usize>().ok()), it.next()) {
                        if let Some(pos) = poly.pos.get_mut(i) {
                            match axis {
                                "x" => pos.x += p.value,
                                "y" => pos.y += p.value,
                                "z" => pos.z += p.value,
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        PolyKind::Extrude => {
            let faces = faces_in_range(poly, ints);
            if !faces.is_empty() {
                let mut n = Vec3::ZERO;
                for &f in &faces {
                    n += poly.face_normal(f as usize);
                }
                let n = n.normalized();
                let d = pv(plugs, "localTranslateZ", 0.0);
                if pv(plugs, "keepFacesTogether", 1.0) > 0.5 || faces.len() == 1 {
                    let r = poly.extrude_faces(&faces);
                    if d.abs() > 1e-9 {
                        for &v in &r.verts {
                            poly.pos[v as usize] += n * d;
                        }
                    }
                    out.faces = r.faces;
                } else {
                    // every face extrudes along its own normal
                    for &f in &faces {
                        let fnorm = poly.face_normal(f as usize).normalized();
                        let one: BTreeSet<u32> = [f].into_iter().collect();
                        let r = poly.extrude_faces(&one);
                        if d.abs() > 1e-9 {
                            for &v in &r.verts {
                                poly.pos[v as usize] += fnorm * d;
                            }
                        }
                        out.faces.insert(f);
                    }
                }
            }
        }
        PolyKind::ExtrudeEdge => {
            let edges = edges_in_range(poly, ints);
            if !edges.is_empty() {
                let r = poly.extrude_edges(&edges);
                let d = pv(plugs, "localTranslateZ", 0.0);
                if d.abs() > 1e-9 {
                    for &v in &r.verts {
                        poly.pos[v as usize].y += d;
                    }
                }
                out.edges = r.edges;
            }
        }
        PolyKind::Bevel => {
            let edges = edges_in_range(poly, ints);
            if !edges.is_empty() {
                out = poly.bevel_edges(&edges, pv(plugs, "offset", 0.2));
            }
        }
        PolyKind::DelFace => {
            let faces = faces_in_range(poly, ints);
            if !faces.is_empty() {
                poly.delete_faces(&faces);
            }
        }
        PolyKind::DelEdge => {
            let edges = edges_in_range(poly, ints);
            if !edges.is_empty() {
                poly.delete_edges(&edges, true);
            }
        }
        PolyKind::DelVertex => {
            let verts = verts_in_range(poly, ints);
            if !verts.is_empty() {
                poly.delete_verts(&verts);
            }
        }
        PolyKind::Merge => {
            let verts = verts_in_range(poly, ints);
            if verts.len() >= 2 {
                let groups = poly.cluster_by_distance(&verts, pv(plugs, "distance", 0.001).max(1e-6));
                let remap = poly.merge_groups(&groups);
                for g in &groups {
                    if g.len() > 1 {
                        if let Some(Some(n)) = remap.v.get(g[0] as usize) {
                            out.verts.insert(*n);
                        }
                    }
                }
            }
        }
        PolyKind::Collapse => {
            // ints[0]: 0 = edges, 1 = faces
            if let Some((&mode, rest)) = ints.split_first() {
                if mode == 0 {
                    let e = edges_in_range(poly, rest);
                    if !e.is_empty() {
                        poly.collapse_edges(&e);
                    }
                } else {
                    let f = faces_in_range(poly, rest);
                    if !f.is_empty() {
                        poly.collapse_faces(&f);
                    }
                }
            }
        }
        PolyKind::Smooth => {
            let n = pv(plugs, "divisions", 1.0).clamp(1.0, 3.0) as u32;
            for _ in 0..n {
                *poly = poly.smooth();
            }
        }
        PolyKind::Triangulate => poly.triangulate(),
        PolyKind::Reverse => {
            let faces = faces_in_range(poly, ints);
            if faces.is_empty() {
                poly.reverse_faces(None);
            } else {
                poly.reverse_faces(Some(&faces));
            }
        }
        PolyKind::Mirror => poly.mirror(pv(plugs, "axis", 0.0).clamp(0.0, 2.0) as usize),
        PolyKind::InsertLoop => {
            if ints.len() >= 2 && (ints[0] as usize) < poly.pos.len() && (ints[1] as usize) < poly.pos.len() {
                let r = poly.insert_edge_loop((ints[0], ints[1]), pv(plugs, "weight", 0.5).clamp(0.02, 0.98));
                out.edges = r.edges;
            }
        }
        PolyKind::Split => {
            // ints: triples (kind, a, b); kind 0 = vertex a, kind 1 = point on edge a-b with parameter t{i}
            let mut ids: Vec<u32> = Vec::new();
            let n = poly.pos.len() as u32;
            for (i, c) in ints.chunks_exact(3).enumerate() {
                if c[0] == 0 {
                    if c[1] < n {
                        ids.push(c[1]);
                    }
                } else if c[1] < n && c[2] < n {
                    let t = pv(plugs, &format!("t{}", i), 0.5);
                    ids.push(poly.split_edge(c[1], c[2], t));
                }
            }
            for w in ids.windows(2) {
                if poly.split_face_between(w[0], w[1]).is_some() {
                    out.edges.insert(ekey(w[0], w[1]));
                }
            }
        }
        PolyKind::Skin | PolyKind::Lattice | PolyKind::Wire | PolyKind::Cluster | PolyKind::SoftMod | PolyKind::Jiggle => {}
        PolyKind::Wave => {
            let g = |n: &str, d: f32| pv(plugs, n, d);
            deform::wave(&mut poly.pos, g("axis", 1.0).clamp(0.0, 2.0) as usize, g("amplitude", 0.3), g("wavelength", 1.0), g("offset", 0.0), g("minRadius", 0.0), g("maxRadius", 5.0), g("dropoff", 0.0));
        }
        PolyKind::Bend | PolyKind::Twist | PolyKind::Flare | PolyKind::Squash | PolyKind::Sine => {
            let axis = pv(plugs, "axis", 1.0).clamp(0.0, 2.0) as usize;
            let (lo, hi) = (pv(plugs, "lowBound", -1.0), pv(plugs, "highBound", 1.0));
            let g = |n: &str, d: f32| pv(plugs, n, d);
            match kind {
                PolyKind::Bend => deform::bend(&mut poly.pos, axis, lo, hi, g("curvature", 0.0)),
                PolyKind::Twist => deform::twist(&mut poly.pos, axis, lo, hi, g("startAngle", 0.0), g("endAngle", 0.0)),
                PolyKind::Flare => deform::flare(&mut poly.pos, axis, lo, hi, (g("startFlareX", 1.0), g("startFlareZ", 1.0)), (g("endFlareX", 1.0), g("endFlareZ", 1.0)), g("curve", 0.0)),
                PolyKind::Squash => deform::squash(&mut poly.pos, axis, lo, hi, g("factor", 0.0), g("expand", 1.0)),
                _ => deform::sine(&mut poly.pos, axis, lo, hi, g("amplitude", 0.0), g("wavelength", 2.0), g("offset", 0.0), g("dropoff", 0.0)),
            }
        }
        PolyKind::Bridge => {
            let edges = edges_in_range(poly, ints);
            if let Ok(s) = poly.bridge(&edges) {
                out = s;
            }
        }
        PolyKind::OffsetLoop => {
            if ints.len() >= 2 && (ints[0] as usize) < poly.pos.len() && (ints[1] as usize) < poly.pos.len() {
                out = poly.offset_edge_loop(ekey(ints[0], ints[1]), pv(plugs, "offset", 0.25));
            }
        }
        PolyKind::SlideEdge => {
            let edges = edges_in_range(poly, ints);
            if !edges.is_empty() {
                poly.slide_edges(&edges, pv(plugs, "amount", 0.0));
                out.edges = edges;
            }
        }
        PolyKind::MirrorCut => {
            poly.mirror_cut(pv(plugs, "axis", 0.0).clamp(0.0, 2.0) as usize, pv(plugs, "planeOffset", 0.0), pv(plugs, "keepNegative", 0.0) > 0.5);
        }
        PolyKind::MergeEdge => {
            let edges = edges_in_range(poly, ints);
            if edges.len() >= 2 {
                out = poly.merge_edge_pairs(&edges).0;
            }
        }
        PolyKind::Blend => {
            if let Some(t) = source {
                if t.pos.len() == poly.pos.len() {
                    let w = pv(plugs, "weight", 0.0);
                    for (p, q) in poly.pos.iter_mut().zip(&t.pos) {
                        *p = p.lerp(*q, w);
                    }
                }
            }
        }
        PolyKind::ChipOff => {
            let faces = faces_in_range(poly, ints);
            if !faces.is_empty() {
                let dup = pv(plugs, "duplicate", 0.0) > 0.5;
                let mut copy: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
                let mut moved: Vec<u32> = Vec::new();
                for &f in &faces {
                    let old = poly.faces[f as usize].clone();
                    let mapped: Vec<u32> = old
                        .iter()
                        .map(|&v| {
                            *copy.entry(v).or_insert_with(|| {
                                poly.pos.push(poly.pos[v as usize]);
                                (poly.pos.len() - 1) as u32
                            })
                        })
                        .collect();
                    if dup {
                        poly.faces.push(mapped);
                        moved.push((poly.faces.len() - 1) as u32);
                    } else {
                        poly.faces[f as usize] = mapped;
                        moved.push(f);
                    }
                }
                out.faces = moved.into_iter().collect();
                let r = poly.cleanup();
                out = r.sel(&out);
            }
        }
        PolyKind::Transform => {
            let mut m = crate::math::Mat4::identity();
            for i in 0..16 {
                m.m[i / 4][i % 4] = pv(plugs, &format!("m{}", i), if i % 5 == 0 { 1.0 } else { 0.0 });
            }
            for p in poly.pos.iter_mut() {
                *p = m.transform_point(*p);
            }
        }
        PolyKind::Average => deform::average_vertices(poly, pv(plugs, "iterations", 1.0).max(0.0) as u32, pv(plugs, "strength", 1.0)),
        PolyKind::FillHole => {
            let edges = edges_in_range(poly, ints);
            let r = if edges.is_empty() { poly.fill_holes(None) } else { poly.fill_holes(Some(&edges)) };
            out = r;
        }
    }
    out
}

impl Scene {
    pub fn new_poly_node(&mut self, kind: PolyKind, ints: Vec<u32>, source: Option<Poly>, owner: Option<NodeId>) -> NodeId {
        let mut d = PolyOpData::new(kind);
        d.ints = ints;
        d.source = source;
        d.owner = owner;
        let id = self.create_node(NodeData::PolyOp(Box::new(d)), kind.node_base(), None, true);
        self.roots.retain(|r| *r != id);
        id
    }

    /// Delete History except deformers: everything before the first deformer (skin, bend, twist,
    /// flare, squash, sine, blend shape) is baked into one source node, the deformers and the nodes
    /// after them stay. Returns how many history nodes were removed.
    pub fn delete_non_deformer_history(&mut self, shape: NodeId) -> usize {
        let hist = self.history_of(shape);
        let is_deformer = |s: &Scene, h: NodeId| matches!(s.nodes.get(h).map(|n| &n.data), Some(NodeData::PolyOp(op)) if matches!(op.kind, PolyKind::Skin | PolyKind::Lattice | PolyKind::Wave | PolyKind::Wire | PolyKind::Cluster | PolyKind::SoftMod | PolyKind::Jiggle | PolyKind::Bend | PolyKind::Twist | PolyKind::Flare | PolyKind::Squash | PolyKind::Sine | PolyKind::Blend));
        let Some(first) = hist.iter().position(|h| is_deformer(self, *h)) else {
            return self.delete_history(shape);
        };
        if first <= 1 {
            // a lone source (or nothing) before the deformers: already baked
            return 0;
        }
        self.flush_meshes();
        let poly = match self.nodes.get(hist[first - 1]).map(|n| &n.data) {
            Some(NodeData::PolyOp(op)) => op.cache.clone(),
            _ => None,
        };
        let Some(poly) = poly else { return 0 };
        for h in &hist[..first] {
            self.disconnect_all(*h);
            self.anim.remove_node(*h);
            if let Some(node) = self.nodes.remove(*h) {
                self.names.remove(&node.name);
            }
        }
        let src = self.new_poly_node(PolyKind::Source, Vec::new(), Some(poly), Some(shape));
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            let mut nh = vec![src];
            nh.extend(hist[first..].iter().copied());
            md.history = nh;
        }
        self.mark_poly_dirty(src);
        self.flush_meshes();
        first
    }

    pub fn history_of(&self, shape: NodeId) -> Vec<NodeId> {
        match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.history.clone(),
            _ => Vec::new(),
        }
    }

    /// Mark a history node (and everything after it) for re-evaluation.
    pub fn mark_poly_dirty(&mut self, id: NodeId) {
        let owner = match self.nodes.get(id).map(|n| &n.data) {
            Some(NodeData::PolyOp(op)) => op.owner,
            _ => None,
        };
        let shape = match owner {
            Some(o) => o,
            None => return,
        };
        let hist = self.history_of(shape);
        if let Some(pos) = hist.iter().position(|h| *h == id) {
            for h in &hist[pos..] {
                if let Some(NodeData::PolyOp(op)) = self.nodes.get_mut(*h).map(|n| &mut n.data) {
                    op.dirty = true;
                }
            }
        }
        if !self.dirty_shapes.contains(&shape) {
            self.dirty_shapes.push(shape);
        }
    }

    pub fn flush_meshes(&mut self) {
        self.flush_nurbs();
        self.flush_strokes();
        let shapes = std::mem::take(&mut self.dirty_shapes);
        for s in shapes {
            self.rebuild_mesh(s);
        }
        self.sync_instances();
    }

    /// Bring instance shapes up to date with the shapes they instance.
    pub fn sync_instances(&mut self) {
        let insts: Vec<(NodeId, NodeId, u64)> = self
            .nodes
            .iter()
            .filter_map(|(id, n)| match &n.data {
                NodeData::Mesh(md) => md.instance_of.map(|s| (id, s, md.instance_ver)),
                _ => None,
            })
            .collect();
        for (id, src, ver) in insts {
            let copy = match self.nodes.get(src).map(|n| &n.data) {
                Some(NodeData::Mesh(m)) => {
                    if m.cache.version + 1 == ver {
                        continue;
                    }
                    Some((m.mesh.clone(), m.zones.clone(), m.face_zone.clone(), m.material, m.cache.version, m.uv_set.clone()))
                }
                // the original is gone: the instance becomes a normal mesh
                _ => None,
            };
            if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(id).map(|n| &mut n.data) {
                match copy {
                    Some((mesh, zones, fz, mat, v, uvs)) => {
                        md.mesh = mesh;
                        md.uv_set = uvs;
                        md.zones = zones;
                        md.face_zone = fz;
                        md.material = mat;
                        md.instance_ver = v + 1;
                        md.touch();
                    }
                    None => md.instance_of = None,
                }
            }
        }
    }

    pub fn rebuild_all_meshes(&mut self) {
        let shapes = self.ids_where(|d| matches!(d, NodeData::Mesh(_)));
        for s in shapes {
            for h in self.history_of(s) {
                if let Some(NodeData::PolyOp(op)) = self.nodes.get_mut(h).map(|n| &mut n.data) {
                    op.dirty = true;
                }
            }
            self.rebuild_mesh(s);
        }
        self.dirty_shapes.clear();
    }

    /// Re-run the dirty tail of a shape's history and refresh its mesh.
    pub fn rebuild_mesh(&mut self, shape: NodeId) {
        let hist = self.history_of(shape);
        if hist.is_empty() {
            return;
        }
        let first = hist
            .iter()
            .position(|id| match self.nodes.get(*id).map(|n| &n.data) {
                Some(NodeData::PolyOp(op)) => op.dirty || op.cache.is_none(),
                _ => true,
            })
            .unwrap_or(hist.len());
        if first == hist.len() {
            return;
        }
        let mut poly = if first == 0 {
            Poly::default()
        } else {
            match self.nodes.get(hist[first - 1]).map(|n| &n.data) {
                Some(NodeData::PolyOp(op)) => op.cache.clone().unwrap_or_default(),
                _ => Poly::default(),
            }
        };
        for id in hist.iter().skip(first) {
            let (kind, ints, source, plugs, skin, lattice) = match self.nodes.get(*id) {
                Some(n) => match &n.data {
                    NodeData::PolyOp(op) => (op.kind, op.ints.clone(), op.source.clone(), n.plugs.clone(), op.skin.clone(), op.lattice),
                    _ => continue,
                },
                None => continue,
            };
            let out = if kind == PolyKind::Skin {
                if let Some(sk) = &skin {
                    self.eval_skin(shape, sk, &mut poly);
                }
                Default::default()
            } else if kind == PolyKind::Lattice {
                if let Some(l) = lattice {
                    self.eval_lattice(shape, l, &mut poly);
                }
                Default::default()
            } else if matches!(kind, PolyKind::Wire | PolyKind::Cluster | PolyKind::SoftMod | PolyKind::Jiggle) {
                self.eval_point_deformer(shape, *id, kind, &ints, &plugs, &mut poly);
                Default::default()
            } else if matches!(kind, PolyKind::Bend | PolyKind::Twist | PolyKind::Flare | PolyKind::Squash | PolyKind::Sine | PolyKind::Wave) {
                // nonlinear deformers: membership (the vertices in `ints`) and envelope
                let before = poly.pos.clone();
                let out = apply_poly_op(kind, &ints, &source, &plugs, &mut poly);
                finish_deform(&before, &mut poly, &ints, pv(&plugs, "envelope", 1.0));
                out
            } else {
                apply_poly_op(kind, &ints, &source, &plugs, &mut poly)
            };
            if let Some(NodeData::PolyOp(op)) = self.nodes.get_mut(*id).map(|n| &mut n.data) {
                op.cache = Some(poly.clone());
                op.out = out;
                op.dirty = false;
            }
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            md.mesh = poly.to_mesh();
            md.touch();
        }
    }

    /// Output selection of the last history node (what the user should have selected after an op).
    pub fn last_op_selection(&self, shape: NodeId) -> Sel {
        let hist = self.history_of(shape);
        match hist.last().and_then(|id| self.nodes.get(*id)).map(|n| &n.data) {
            Some(NodeData::PolyOp(op)) => op.out.clone(),
            _ => Sel::default(),
        }
    }

    /// Append an operation node to the shape's history (creating a source node when there is none).
    pub fn append_history(
        &mut self,
        shape: NodeId,
        kind: PolyKind,
        ints: Vec<u32>,
        plug_values: &[(String, f32)],
    ) -> Result<NodeId, String> {
        let needs_source = match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => {
                if let Some(src) = md.instance_of {
                    return Err(format!("This shape is an instance of {}: edit the original, or run uninstance to make it independent", self.name_of(src)));
                }
                md.history.is_empty()
            }
            _ => return Err("Not a mesh".to_string()),
        };
        if needs_source {
            let poly = match self.nodes.get(shape).map(|n| &n.data) {
                Some(NodeData::Mesh(md)) => Poly::welded(&md.mesh, 1e-4),
                _ => unreachable!(),
            };
            let src = self.new_poly_node(PolyKind::Source, Vec::new(), Some(poly), Some(shape));
            if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
                md.history.push(src);
                md.prim = "history".to_string();
            }
            self.mark_poly_dirty(src);
            self.flush_meshes();
        }
        let id = self.new_poly_node(kind, ints, None, Some(shape));
        for (k, v) in plug_values {
            if let Some(n) = self.nodes.get_mut(id) {
                if n.plug(k).is_none() {
                    let mut p = Plug::new(k, *v);
                    p.custom = true;
                    n.plugs.push(p);
                } else if let Some(p) = n.plug_mut(k) {
                    p.value = *v;
                    p.rest = *v;
                }
            }
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            md.history.push(id);
        }
        self.mark_poly_dirty(id);
        self.flush_meshes();
        Ok(id)
    }

    /// Remove a history node again (used when an operation turned out to have no effect).
    pub fn pop_history(&mut self, shape: NodeId, node: NodeId) {
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            md.history.retain(|h| *h != node);
        }
        self.disconnect_all(node);
        self.anim.remove_node(node);
        if let Some(n) = self.nodes.remove(node) {
            self.names.remove(&n.name);
        }
        // the cached output of the new last node is still valid; refresh the mesh from it
        let hist = self.history_of(shape);
        if let Some(last) = hist.last() {
            let poly = match self.nodes.get(*last).map(|n| &n.data) {
                Some(NodeData::PolyOp(op)) => op.cache.clone(),
                _ => None,
            };
            if let (Some(p), Some(NodeData::Mesh(md))) = (poly, self.nodes.get_mut(shape).map(|n| &mut n.data)) {
                md.mesh = p.to_mesh();
                md.touch();
            }
        }
    }

    /// Remove a single (non generator) history node and re-evaluate the rest of the chain.
    pub fn delete_history_node(&mut self, shape: NodeId, node: NodeId) -> Result<(), String> {
        let hist = self.history_of(shape);
        let pos = hist.iter().position(|h| *h == node).ok_or("Node is not part of this shape's history")?;
        let kind = match self.nodes.get(node).map(|n| &n.data) {
            Some(NodeData::PolyOp(op)) => op.kind,
            _ => return Err("Not a history node".to_string()),
        };
        if kind.is_generator() {
            return Err("The first node creates the geometry; use Delete All History to bake it".to_string());
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            md.history.remove(pos);
        }
        self.disconnect_all(node);
        self.anim.remove_node(node);
        if let Some(n) = self.nodes.remove(node) {
            self.names.remove(&n.name);
        }
        let hist2 = self.history_of(shape);
        if let Some(next) = hist2.get(pos).or_else(|| hist2.last()) {
            self.mark_poly_dirty(*next);
        }
        self.flush_meshes();
        Ok(())
    }

    /// The tweak node at the end of the history (created when the last node is not a tweak).
    pub fn tweak_node(&mut self, shape: NodeId) -> Result<NodeId, String> {
        let last = self.history_of(shape).last().copied();
        if let Some(l) = last {
            if matches!(self.nodes.get(l).map(|n| &n.data), Some(NodeData::PolyOp(op)) if op.kind == PolyKind::Tweak) {
                return Ok(l);
            }
        }
        self.append_history(shape, PolyKind::Tweak, Vec::new(), &[])
    }

    /// Set vertex positions (local space) through the tweak node so the history stays intact.
    pub fn tweak_vertices(&mut self, shape: NodeId, edits: &[(u32, Vec3)]) -> Result<(), String> {
        // lattice points: the edit is stored as an offset from the rest grid
        if let Some(NodeData::Lattice(ld)) = self.nodes.get(shape).map(|n| &n.data) {
            let ld = (**ld).clone();
            let mut all: Vec<(u32, Vec3)> = edits.to_vec();
            if let Some(axis) = self.reflection {
                let flip = |mut p: Vec3| {
                    match axis {
                        0 => p.x = -p.x,
                        1 => p.y = -p.y,
                        _ => p.z = -p.z,
                    }
                    p
                };
                for (v, pos) in edits {
                    let target = flip(ld.rest(*v as usize));
                    if let Some(m) = (0..ld.count()).find(|i| (ld.rest(*i) - target).length() < 1e-4) {
                        if m as u32 != *v && !all.iter().any(|(e, _)| *e == m as u32) {
                            let off = *pos - ld.rest(*v as usize);
                            all.push((m as u32, ld.rest(m) + flip(off)));
                        }
                    }
                }
            }
            for (v, pos) in all {
                if (v as usize) >= ld.count() {
                    continue;
                }
                let (i, j, k) = ld.coords(v as usize);
                let off = pos - ld.rest(v as usize);
                for (a, x) in ["x", "y", "z"].iter().zip([off.x, off.y, off.z]) {
                    self.set_attr(shape, &format!("pt_{}_{}_{}_{}", i, j, k, a), x)?;
                }
            }
            return Ok(());
        }
        // NURBS shapes: the "vertices" are control points
        if matches!(self.nodes.get(shape).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.nurbs.is_some()) || matches!(self.nodes.get(shape).map(|n| &n.data), Some(NodeData::NurbsCurve(_))) {
            let mut all: Vec<(u32, Vec3)> = edits.to_vec();
            if let Some(axis) = self.reflection {
                let cur: Vec<Vec3> = match self.nodes.get(shape).map(|n| &n.data) {
                    Some(NodeData::Mesh(md)) => md.nurbs.as_ref().map(|n| n.surf.cvs.clone()).unwrap_or_default(),
                    Some(NodeData::NurbsCurve(cd)) => cd.curve.cvs.clone(),
                    _ => Vec::new(),
                };
                let flip = |mut p: Vec3| {
                    match axis {
                        0 => p.x = -p.x,
                        1 => p.y = -p.y,
                        _ => p.z = -p.z,
                    }
                    p
                };
                for (v, pos) in edits {
                    let Some(p0) = cur.get(*v as usize) else { continue };
                    let target = flip(*p0);
                    if let Some(m) = cur.iter().position(|q| (*q - target).length() < 1e-3) {
                        let m = m as u32;
                        if m != *v && !all.iter().any(|(e, _)| *e == m) {
                            all.push((m, flip(*pos)));
                        }
                    }
                }
            }
            self.set_nurbs_cvs(shape, &all);
            return Ok(());
        }
        let t = self.tweak_node(shape)?;
        self.flush_meshes();
        // Reflection: the vertex facing each edited one across the symmetry plane moves the opposite way
        let mut mirrored: Vec<(u32, Vec3)> = Vec::new();
        if let Some(axis) = self.reflection {
            if let Some(NodeData::Mesh(md)) = self.nodes.get(shape).map(|n| &n.data) {
                let cur = md.poly();
                let flip = |mut p: Vec3| {
                    match axis {
                        0 => p.x = -p.x,
                        1 => p.y = -p.y,
                        _ => p.z = -p.z,
                    }
                    p
                };
                for (v, pos) in edits {
                    let Some(p0) = cur.pos.get(*v as usize) else { continue };
                    let target = flip(*p0);
                    let found = cur.pos.iter().position(|q| (*q - target).length() < 1e-3);
                    if let Some(m) = found {
                        let m = m as u32;
                        if m != *v && !edits.iter().any(|(e, _)| *e == m) && !mirrored.iter().any(|(e, _)| *e == m) {
                            mirrored.push((m, flip(*pos)));
                        }
                    }
                }
            }
        }
        let owned: Vec<(u32, Vec3)> = edits.iter().copied().chain(mirrored).collect();
        let edits: &[(u32, Vec3)] = &owned;
        let hist = self.history_of(shape);
        let base: Poly = match hist.len().checked_sub(2).and_then(|i| self.nodes.get(hist[i])).map(|n| &n.data) {
            Some(NodeData::PolyOp(op)) => op.cache.clone().unwrap_or_default(),
            _ => Poly::default(),
        };
        if let Some(node) = self.nodes.get_mut(t) {
            for (v, pos) in edits {
                let b = match base.pos.get(*v as usize) {
                    Some(b) => *b,
                    None => continue,
                };
                let off = *pos - b;
                for (axis, val) in [("x", off.x), ("y", off.y), ("z", off.z)] {
                    let name = format!("pnts_{}_{}", v, axis);
                    match node.plug_mut(&name) {
                        Some(p) => {
                            p.value = val;
                            p.rest = val;
                        }
                        None => {
                            let mut p = Plug::new(&name, val);
                            p.custom = true;
                            node.plugs.push(p);
                        }
                    }
                }
            }
        }
        self.mark_poly_dirty(t);
        self.flush_meshes();
        Ok(())
    }

    /// Key the tweak channels of the given vertices at the current time.
    pub fn key_vertices(&mut self, shape: NodeId, verts: &[u32], layer: usize) -> Result<usize, String> {
        let t = self.tweak_node(shape)?;
        let mut n = 0;
        for v in verts {
            for axis in ["x", "y", "z"] {
                let name = format!("pnts_{}_{}", v, axis);
                if self.nodes.get(t).and_then(|nd| nd.plug(&name)).is_none() {
                    if let Some(nd) = self.nodes.get_mut(t) {
                        let mut p = Plug::new(&name, 0.0);
                        p.custom = true;
                        nd.plugs.push(p);
                    }
                }
                self.set_keyframe_noeval(t, &name, None, None, crate::anim::Interp::Bezier, layer)?;
                n += 1;
            }
        }
        self.evaluate_anim();
        Ok(n)
    }

    /// Bake the current result and drop the history nodes (Maya: Edit > Delete by Type > History).
    pub fn delete_history(&mut self, shape: NodeId) -> usize {
        if self.delete_nurbs_history(shape) {
            return 1;
        }
        let hist = self.history_of(shape);
        if hist.is_empty() {
            return 0;
        }
        self.flush_meshes();
        for h in &hist {
            self.disconnect_all(*h);
            self.anim.remove_node(*h);
            if let Some(node) = self.nodes.remove(*h) {
                self.names.remove(&node.name);
            }
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            md.history.clear();
            md.prim = "custom".to_string();
            md.params.clear();
        }
        self.dirty_shapes.retain(|s| *s != shape);
        hist.len()
    }

    /// Copy construction history (and its values) from one shape to another.
    pub fn clone_history(&mut self, src: NodeId, dst: NodeId) {
        let hist = self.history_of(src);
        let mut new_ids = Vec::new();
        for h in hist {
            let (kind, ints, source, plugs) = match self.nodes.get(h) {
                Some(n) => match &n.data {
                    NodeData::PolyOp(op) => (op.kind, op.ints.clone(), op.source.clone(), n.plugs.clone()),
                    _ => continue,
                },
                None => continue,
            };
            let id = self.new_poly_node(kind, ints, source, Some(dst));
            if let Some(n) = self.nodes.get_mut(id) {
                for p in plugs {
                    match n.plug_mut(&p.name) {
                        Some(q) => {
                            q.value = p.value;
                            q.rest = p.rest;
                        }
                        None => {
                            let mut q = Plug::new(&p.name, p.rest);
                            q.custom = true;
                            n.plugs.push(q);
                        }
                    }
                }
            }
            new_ids.push(id);
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(dst).map(|n| &mut n.data) {
            md.history = new_ids.clone();
            md.prim = "history".to_string();
        }
        for id in new_ids.first() {
            self.mark_poly_dirty(*id);
        }
        self.flush_meshes();
    }

    /// Nodes whose animation curves belong to the selection: the objects, their shapes and history.
    pub fn key_owner_ids(&self, selection: &[NodeId]) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = Vec::new();
        for id in selection {
            for d in self.descendants(*id) {
                out.push(d);
                out.extend(self.history_of(d));
            }
        }
        out
    }
}

pub fn mesh_data_empty() -> MeshData {
    MeshData { cache: Default::default(), history: Vec::new(), mesh: crate::geo::Mesh::new(), prim: "history".into(), params: Vec::new(), material: None, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(sc: &mut Scene) -> NodeId {
        let params = vec![
            ("width".to_string(), 1.0),
            ("height".to_string(), 1.0),
            ("depth".to_string(), 1.0),
            ("subdivisions".to_string(), 1.0),
        ];
        sc.create_mesh_object("polyCube", params, Some("box")).unwrap().1
    }

    fn xmax(sc: &Scene, shape: NodeId) -> f32 {
        match sc.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.mesh.positions.iter().map(|p| p.x).fold(f32::MIN, f32::max),
            _ => 0.0,
        }
    }

    #[test]
    fn history_reevaluates_when_an_input_changes() {
        let mut sc = Scene::new();
        let shape = cube(&mut sc);
        sc.append_history(shape, PolyKind::Extrude, vec![5], &[("localTranslateZ".to_string(), 0.5)]).unwrap();
        let faces = |sc: &Scene| match sc.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.mesh.faces.len(),
            _ => 0,
        };
        assert_eq!(faces(&sc), 10);
        assert!((xmax(&sc, shape) - 0.5).abs() < 1e-4);
        let cube_node = sc.find("polyCube1").unwrap();
        sc.set_attr(cube_node, "width", 4.0).unwrap();
        assert_eq!(faces(&sc), 10);
        assert!((xmax(&sc, shape) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn tweak_is_animatable_and_delete_history_bakes() {
        let mut sc = Scene::new();
        let shape = cube(&mut sc);
        sc.tweak_vertices(shape, &[(0, Vec3::new(-0.5, -0.5, 3.0))]).unwrap();
        let t = sc.tweak_node(shape).unwrap();
        sc.key_vertices(shape, &[0], 0).unwrap();
        sc.set_time(10.0);
        sc.set_attr(t, "pnts_0_z", 5.0).unwrap();
        sc.key_vertices(shape, &[0], 0).unwrap();
        sc.set_time(1.0);
        let z1 = match sc.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.mesh.positions[0].z,
            _ => 0.0,
        };
        sc.set_time(10.0);
        let z10 = match sc.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.mesh.positions[0].z,
            _ => 0.0,
        };
        assert!(z10 > z1 + 1.0, "z1={} z10={}", z1, z10);
        let n = sc.delete_history(shape);
        assert!(n >= 2);
        assert!(sc.history_of(shape).is_empty());
    }

    #[test]
    fn history_roundtrips_through_ascii() {
        let mut sc = Scene::new();
        let shape = cube(&mut sc);
        sc.append_history(shape, PolyKind::Extrude, vec![5], &[("localTranslateZ".to_string(), 1.0)]).unwrap();
        let text = crate::dag::serialize::to_ascii(&sc);
        let sc2 = crate::dag::serialize::from_ascii(&text).unwrap();
        let s2 = sc2.find("boxShape").unwrap();
        assert_eq!(sc2.history_of(s2).len(), 2);
        assert!((xmax(&sc2, s2) - 0.5).abs() < 1e-4);
    }
}
