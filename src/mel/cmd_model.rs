//! Polygon modeling operations and their MEL commands (polyExtrudeFacet, polyBevel, ...).
//! The windowed workspace calls `run_op` directly so menus, shelf buttons and scripts share one path.

use super::commands::Out;
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::{MeshData, NodeData, PolyKind};
use crate::dag::NodeId;
use crate::geo::edit::{ekey, Edge, Poly, Sel};
use crate::math::Vec3;
use crate::session::{SelMode, Session};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug)]
pub enum SplitPt {
    Vertex(u32),
    OnEdge(Edge, f32),
}

#[derive(Clone, Debug)]
pub enum ModelOp {
    Extrude { ltz: f32 },
    Bevel { offset: f32 },
    Delete { clean: bool },
    Merge { dist: f32 },
    Collapse,
    Smooth { levels: u32 },
    Triangulate,
    Reverse,
    FillHole,
    Mirror { axis: usize },
    Combine,
    Separate,
    InsertLoop { edge: Edge, t: f32 },
    Split(Vec<SplitPt>),
    SelectLoop,
    Grow,
    Convert(SelMode),
    /// Detach (or with `dup` copy) the selected faces as their own shell.
    ChipOff { dup: bool },
    /// Merge the selected vertices into one at their center, however far apart they are.
    MergeCenter,
    /// Join pairs of selected border edges.
    MergeEdge,
    /// Connect two border edge chains with a strip of faces.
    Bridge,
    /// Offset Edge Loop on the selected edge.
    OffsetLoop { offset: f32 },
    /// Slide the selected edges' loop.
    SlideEdge { amount: f32 },
    /// Mirror Cut about a plane of the object.
    MirrorCut { axis: usize, offset: f32, negative: bool },
    /// Select the border edges of the mesh.
    SelectBorder,
}

/// The mesh shape currently being edited: the component selection's node, else the first selected object.
pub fn current_shape(s: &Session) -> Option<NodeId> {
    let is_mesh = |id: NodeId| matches!(s.scene.node(id).map(|n| &n.data), Some(NodeData::Mesh(_)) | Some(NodeData::NurbsCurve(_)) | Some(NodeData::Lattice(_)));
    if s.comp.mode != SelMode::Object {
        if let Some(n) = s.comp.node {
            if is_mesh(n) {
                return Some(n);
            }
        }
    }
    for id in &s.scene.selection {
        if is_mesh(*id) {
            return Some(*id);
        }
        if let Some(sh) = s.scene.mesh_shapes_under(*id).first() {
            return Some(*sh);
        }
        if let Some(sh) = s.scene.curve_shapes_under(*id).first() {
            return Some(*sh);
        }
        if let Some(sh) = s.scene.lattice_shapes_under(*id).first() {
            return Some(*sh);
        }
    }
    None
}

fn mesh_mut(s: &mut Session, shape: NodeId) -> Option<&mut MeshData> {
    match s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
        Some(NodeData::Mesh(md)) => Some(md),
        _ => None,
    }
}

/// Selection in the current mode, falling back to "everything" when nothing component-wise is selected.
pub(super) fn effective(s: &Session, shape: NodeId, poly: &Poly) -> (Sel, SelMode) {
    let mode = s.comp.mode;
    let c = &s.comp;
    let all = Sel {
        verts: (0..poly.pos.len() as u32).collect(),
        edges: poly.edge_list().into_iter().collect(),
        faces: (0..poly.faces.len() as u32).collect(),
    };
    if mode == SelMode::Object || c.node != Some(shape) {
        return (all, SelMode::Object);
    }
    let ok = c.sel.verts.iter().all(|&v| (v as usize) < poly.pos.len())
        && c.sel.faces.iter().all(|&f| (f as usize) < poly.faces.len());
    if !ok {
        return (Sel::default(), mode);
    }
    (c.sel.clone(), mode)
}

pub fn faces_of(poly: &Poly, sel: &Sel, mode: SelMode) -> BTreeSet<u32> {
    match mode {
        SelMode::Face | SelMode::Object => sel.faces.clone(),
        SelMode::Edge => poly.faces_inside_edges(&sel.edges),
        SelMode::Vertex => poly.faces_inside_verts(&sel.verts),
    }
}

pub fn edges_of(poly: &Poly, sel: &Sel, mode: SelMode) -> BTreeSet<Edge> {
    match mode {
        SelMode::Edge | SelMode::Object => sel.edges.clone(),
        SelMode::Face => poly.faces_to_edges(&sel.faces),
        SelMode::Vertex => poly.edge_list().into_iter().filter(|e| sel.verts.contains(&e.0) && sel.verts.contains(&e.1)).collect(),
    }
}

pub fn verts_of(poly: &Poly, sel: &Sel, mode: SelMode) -> BTreeSet<u32> {
    match mode {
        SelMode::Vertex | SelMode::Object => sel.verts.clone(),
        SelMode::Edge => sel.edges.iter().flat_map(|e| [e.0, e.1]).collect(),
        SelMode::Face => poly.verts_of_faces(&sel.faces),
    }
}

fn transform_of(s: &Session, id: NodeId) -> NodeId {
    match s.scene.node(id) {
        Some(n) if !n.data.is_transform() => n.parent.unwrap_or(id),
        _ => id,
    }
}

fn flat_edges(edges: &BTreeSet<Edge>) -> Vec<u32> {
    edges.iter().flat_map(|e| [e.0, e.1]).collect()
}

pub fn run_op(s: &mut Session, op: ModelOp) -> Result<String, String> {
    match op {
        ModelOp::Combine => return combine(s),
        ModelOp::Separate => return separate(s),
        _ => {}
    }
    // whole-object operations are recorded on every selected mesh
    if let ModelOp::Smooth { levels } = op {
        return object_op(s, PolyKind::Smooth, &[("divisions".to_string(), levels.clamp(1, 3) as f32)], "Smoothed");
    }
    if matches!(op, ModelOp::Triangulate) {
        return object_op(s, PolyKind::Triangulate, &[], "Triangulated");
    }
    if let ModelOp::Mirror { axis } = op {
        return object_op(s, PolyKind::Mirror, &[("axis".to_string(), axis as f32)], "Mirrored");
    }
    let shape = current_shape(s).ok_or("Select a polygon object first")?;
    if matches!(s.scene.node(shape).map(|n| &n.data), Some(NodeData::NurbsCurve(_))) || matches!(s.scene.node(shape).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.nurbs.is_some()) {
        return Err("This is a NURBS object: use Modify > Convert > NURBS to Polygons (nurbsToPoly) before polygon modeling".to_string());
    }
    if s.scene.skin_of(shape).is_some() && !matches!(op, ModelOp::SelectLoop | ModelOp::Grow | ModelOp::Convert(_)) {
        return Err("The mesh is bound to a skeleton; detach the skin before modeling".to_string());
    }
    s.scene.flush_meshes();
    let (poly, sel, mode) = {
        let md = mesh_mut(s, shape).ok_or("Not a mesh")?;
        md.ensure_custom();
        let poly = md.poly();
        let (sel, mode) = effective(s, shape, &poly);
        (poly, sel, mode)
    };
    let mut new_mode = s.comp.mode;
    let msg: String;
    // (kind, ints, plug values) of the history node to append
    let (kind, ints, plugs): (PolyKind, Vec<u32>, Vec<(String, f32)>) = match op.clone() {
        ModelOp::Extrude { ltz } => {
            let kft = if s.prefs.keep_faces_together { 1.0 } else { 0.0 };
            if mode == SelMode::Edge && !sel.edges.is_empty() {
                new_mode = SelMode::Edge;
                msg = format!("Extruded {} edge(s)", sel.edges.len());
                (PolyKind::ExtrudeEdge, flat_edges(&sel.edges), vec![("localTranslateZ".to_string(), ltz), ("keepFacesTogether".to_string(), kft)])
            } else {
                let faces = faces_of(&poly, &sel, mode);
                if faces.is_empty() {
                    return Err("Select faces (or edges) to extrude".to_string());
                }
                new_mode = SelMode::Face;
                msg = format!("Extruded {} face(s)", faces.len());
                (PolyKind::Extrude, faces.into_iter().collect(), vec![("localTranslateZ".to_string(), ltz), ("keepFacesTogether".to_string(), kft)])
            }
        }
        ModelOp::Bevel { offset } => {
            let edges = edges_of(&poly, &sel, mode);
            if edges.is_empty() {
                return Err("Select edges to bevel".to_string());
            }
            new_mode = SelMode::Face;
            msg = format!("Beveled {} edge(s)", edges.len());
            (PolyKind::Bevel, flat_edges(&edges), vec![("offset".to_string(), offset)])
        }
        ModelOp::Delete { .. } => match mode {
            SelMode::Face => {
                if sel.faces.is_empty() {
                    return Err("No faces selected".to_string());
                }
                msg = format!("Deleted {} face(s)", sel.faces.len());
                (PolyKind::DelFace, sel.faces.iter().copied().collect(), vec![])
            }
            SelMode::Edge => {
                if sel.edges.is_empty() {
                    return Err("No edges selected".to_string());
                }
                msg = format!("Deleted {} edge(s)", sel.edges.len());
                (PolyKind::DelEdge, flat_edges(&sel.edges), vec![])
            }
            SelMode::Vertex => {
                if sel.verts.is_empty() {
                    return Err("No vertices selected".to_string());
                }
                msg = format!("Deleted {} vertex(es)", sel.verts.len());
                (PolyKind::DelVertex, sel.verts.iter().copied().collect(), vec![])
            }
            SelMode::Object => return Err("Switch to a component mode and select components first".to_string()),
        },
        ModelOp::Merge { dist } => {
            let verts = verts_of(&poly, &sel, mode);
            if verts.len() < 2 {
                return Err("Select at least two vertices to merge".to_string());
            }
            new_mode = SelMode::Vertex;
            msg = "Merged vertices".to_string();
            (PolyKind::Merge, verts.into_iter().collect(), vec![("distance".to_string(), dist)])
        }
        ModelOp::Collapse => match mode {
            SelMode::Edge if !sel.edges.is_empty() => {
                let mut v = vec![0u32];
                v.extend(flat_edges(&sel.edges));
                msg = "Collapsed edges".to_string();
                (PolyKind::Collapse, v, vec![])
            }
            SelMode::Face if !sel.faces.is_empty() => {
                let mut v = vec![1u32];
                v.extend(sel.faces.iter().copied());
                msg = "Collapsed faces".to_string();
                (PolyKind::Collapse, v, vec![])
            }
            _ => return Err("Select edges or faces to collapse".to_string()),
        },
        ModelOp::Reverse => {
            let faces = faces_of(&poly, &sel, mode);
            msg = "Reversed normals".to_string();
            let ints: Vec<u32> = if mode == SelMode::Object { Vec::new() } else { faces.into_iter().collect() };
            (PolyKind::Reverse, ints, vec![])
        }
        ModelOp::InsertLoop { edge, t } => {
            if edge.0 as usize >= poly.pos.len() || edge.1 as usize >= poly.pos.len() {
                return Err("Invalid edge".to_string());
            }
            new_mode = SelMode::Edge;
            msg = "Inserted edge loop".to_string();
            (PolyKind::InsertLoop, vec![edge.0, edge.1], vec![("weight".to_string(), t.clamp(0.02, 0.98))])
        }
        ModelOp::Split(points) => {
            let mut ints = Vec::new();
            let mut plugs = Vec::new();
            for (i, p) in points.iter().enumerate() {
                match p {
                    SplitPt::Vertex(v) => ints.extend([0, *v, 0]),
                    SplitPt::OnEdge((a, b), t) => {
                        ints.extend([1, *a, *b]);
                        plugs.push((format!("t{}", i), *t));
                    }
                }
            }
            new_mode = SelMode::Edge;
            msg = "Split polygon".to_string();
            (PolyKind::Split, ints, plugs)
        }
        ModelOp::FillHole => {
            let edges = if mode == SelMode::Object { BTreeSet::new() } else { edges_of(&poly, &sel, mode) };
            new_mode = SelMode::Face;
            msg = "Filled hole".to_string();
            (PolyKind::FillHole, flat_edges(&edges), vec![])
        }
        ModelOp::SelectLoop => {
            let edges = edges_of(&poly, &sel, mode);
            if edges.is_empty() {
                return Err("Select an edge first".to_string());
            }
            let mut out = BTreeSet::new();
            for e in edges {
                out.extend(poly.edge_loop(e));
            }
            let n = out.len();
            s.comp.sel = Sel { edges: out, ..Default::default() };
            s.comp.node = Some(shape);
            s.comp.mode = SelMode::Edge;
            return Ok(format!("Selected {} edges", n));
        }
        ModelOp::Grow => {
            s.comp.sel = poly.grow(&sel);
            s.comp.node = Some(shape);
            return Ok("Grew selection".to_string());
        }
        ModelOp::Convert(to) => {
            let verts = verts_of(&poly, &sel, mode);
            let mut ns = Sel::default();
            match to {
                SelMode::Vertex => ns.verts = verts,
                SelMode::Edge => {
                    ns.edges = match mode {
                        SelMode::Face => poly.faces_to_edges(&sel.faces),
                        SelMode::Vertex => poly.edge_list().into_iter().filter(|e| verts.contains(&e.0) || verts.contains(&e.1)).collect(),
                        _ => edges_of(&poly, &sel, mode),
                    }
                }
                SelMode::Face => {
                    ns.faces = match mode {
                        SelMode::Edge => poly.faces_inside_edges(&sel.edges),
                        SelMode::Face => sel.faces.clone(),
                        _ => poly.faces_inside_verts(&verts),
                    };
                    if mode == SelMode::Vertex && ns.faces.is_empty() {
                        for (i, f) in poly.faces.iter().enumerate() {
                            if f.iter().any(|v| verts.contains(v)) {
                                ns.faces.insert(i as u32);
                            }
                        }
                    }
                }
                SelMode::Object => {}
            }
            s.comp.sel = ns;
            s.comp.node = Some(shape);
            s.comp.mode = to;
            return Ok("Converted selection".to_string());
        }
        ModelOp::MergeCenter => {
            let verts = verts_of(&poly, &sel, mode);
            if verts.len() < 2 {
                return Err("Select at least two vertices (or edges or faces) to merge".to_string());
            }
            new_mode = SelMode::Vertex;
            msg = format!("Merged {} vertices into one", verts.len());
            (PolyKind::Merge, verts.into_iter().collect(), vec![("distance".to_string(), 1.0e9)])
        }
        ModelOp::Bridge => {
            if mode != SelMode::Edge || sel.edges.is_empty() {
                return Err("Select the border edges (F10) of two holes or the two sides of a gap".to_string());
            }
            // validate on a copy so the message names the problem
            poly.clone().bridge(&sel.edges)?;
            new_mode = SelMode::Face;
            msg = "Bridged".to_string();
            (PolyKind::Bridge, flat_edges(&sel.edges), vec![])
        }
        ModelOp::OffsetLoop { offset } => {
            if mode != SelMode::Edge || sel.edges.is_empty() {
                return Err("Select an edge of the loop to offset".to_string());
            }
            let e = *sel.edges.iter().next().unwrap();
            new_mode = SelMode::Edge;
            msg = "Offset edge loop".to_string();
            (PolyKind::OffsetLoop, vec![e.0, e.1], vec![("offset".to_string(), offset)])
        }
        ModelOp::SlideEdge { amount } => {
            if mode != SelMode::Edge || sel.edges.is_empty() {
                return Err("Select the edges (or an edge loop) to slide".to_string());
            }
            let mut all = BTreeSet::new();
            for e in &sel.edges {
                all.extend(poly.edge_loop(*e));
            }
            new_mode = SelMode::Edge;
            msg = "Slid edges".to_string();
            (PolyKind::SlideEdge, flat_edges(&all), vec![("amount".to_string(), amount)])
        }
        ModelOp::MirrorCut { axis, offset, negative } => {
            msg = "Mirror cut".to_string();
            new_mode = SelMode::Object;
            (PolyKind::MirrorCut, vec![], vec![("axis".to_string(), axis as f32), ("planeOffset".to_string(), offset), ("keepNegative".to_string(), negative as u8 as f32)])
        }
        ModelOp::SelectBorder => {
            let b = poly.border_edges();
            if b.is_empty() {
                return Err("The mesh has no border edges".to_string());
            }
            let n = b.len();
            s.comp.sel = Sel { edges: b, ..Default::default() };
            s.comp.node = Some(shape);
            s.comp.mode = SelMode::Edge;
            return Ok(format!("Selected {} border edges", n));
        }
        ModelOp::MergeEdge => {
            if mode != SelMode::Edge || sel.edges.len() < 2 {
                return Err("Select at least two border edges (F10) to merge, for example the two edges of a seam".to_string());
            }
            new_mode = SelMode::Vertex;
            msg = "Merged edges".to_string();
            (PolyKind::MergeEdge, flat_edges(&sel.edges), vec![])
        }
        ModelOp::ChipOff { dup } => {
            let faces = faces_of(&poly, &sel, mode);
            if mode == SelMode::Object || faces.is_empty() {
                return Err("Select faces first".to_string());
            }
            new_mode = SelMode::Face;
            msg = format!("{} {} face(s)", if dup { "Duplicated" } else { "Detached" }, faces.len());
            (PolyKind::ChipOff, faces.into_iter().collect(), vec![("duplicate".to_string(), dup as u32 as f32)])
        }
        ModelOp::Combine | ModelOp::Separate | ModelOp::Smooth { .. } | ModelOp::Triangulate | ModelOp::Mirror { .. } => unreachable!(),
    };
    let before = (poly.pos.len(), poly.faces.len());
    let node = s.scene.append_history(shape, kind, ints, &plugs)?;
    let after = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => (md.mesh.positions.len(), md.mesh.faces.len()),
        _ => before,
    };
    let no_effect = after == before && !matches!(kind, PolyKind::Reverse | PolyKind::Merge | PolyKind::Collapse | PolyKind::SlideEdge | PolyKind::MirrorCut);
    // merging edges that cannot be paired leaves the mesh as it was
    if no_effect {
        s.scene.pop_history(shape, node);
        return Err(match kind {
            PolyKind::Bevel => "Nothing could be beveled (border edges are not supported)".to_string(),
            PolyKind::FillHole => "Nothing to fill: select edges on the border of a gap (border edges have a face on one side only)".to_string(),
            PolyKind::MergeEdge => "No pair of border edges could be merged: select two edges on the opposite sides of a gap".to_string(),
            PolyKind::Split => "Cannot split: the points must lie on the same face".to_string(),
            _ => "The operation had no effect".to_string(),
        });
    }
    let out = s.scene.last_op_selection(shape);
    if !s.history_on {
        s.scene.delete_history(shape);
    }
    s.comp.node = Some(shape);
    s.comp.mode = if new_mode == SelMode::Object { SelMode::Face } else { new_mode };
    s.comp.sel = out;
    Ok(msg)
}

/// Record a whole-object operation on every selected mesh.
pub fn object_op(s: &mut Session, kind: PolyKind, plugs: &[(String, f32)], what: &str) -> Result<String, String> {
    let mut shapes: Vec<NodeId> = Vec::new();
    if let Some(sh) = current_shape(s) {
        if s.comp.mode != SelMode::Object {
            shapes.push(sh);
        }
    }
    if shapes.is_empty() {
        for id in s.scene.selection.clone() {
            shapes.extend(s.scene.mesh_shapes_under(id));
        }
    }
    if shapes.is_empty() {
        return Err("Select a polygon object first".to_string());
    }
    for sh in &shapes {
        s.scene.append_history(*sh, kind, Vec::new(), plugs)?;
        if !s.history_on {
            s.scene.delete_history(*sh);
        }
    }
    s.comp.sel = Sel::default();
    Ok(format!("{} {} object(s)", what, shapes.len()))
}

fn combine(s: &mut Session) -> Result<String, String> {
    let sel = s.scene.selection.clone();
    let mut parts: Vec<(NodeId, NodeId)> = Vec::new(); // (transform, shape)
    for id in &sel {
        for sh in s.scene.mesh_shapes_under(*id) {
            parts.push((transform_of(s, sh), sh));
        }
    }
    if parts.len() < 2 {
        return Err("Select two or more polygon objects to combine".to_string());
    }
    let mut pos: Vec<Vec3> = Vec::new();
    let mut faces: Vec<Vec<u32>> = Vec::new();
    let mut material = None;
    for (t, sh) in &parts {
        let world = s.scene.world_matrix(*t);
        if let Some(NodeData::Mesh(md)) = s.scene.node(*sh).map(|n| &n.data) {
            let base = pos.len() as u32;
            pos.extend(md.mesh.positions.iter().map(|p| world.transform_point(*p)));
            faces.extend(md.mesh.faces.iter().map(|f| f.iter().map(|v| v + base).collect()));
            if material.is_none() {
                material = md.material;
            }
        }
    }
    let poly = Poly { pos, faces };
    let t = s.scene.create_node(NodeData::Transform, "polySurface", None, true);
    let shape_name = format!("{}Shape", s.scene.name_of(t));
    let md = MeshData { cache: Default::default(), history: Vec::new(), mesh: poly.to_mesh(), prim: "custom".to_string(), params: Vec::new(), material, ..Default::default() };
    s.scene.create_node(NodeData::Mesh(md), &shape_name, Some(t), false);
    let mut originals: Vec<NodeId> = parts.iter().map(|p| p.0).collect();
    originals.sort();
    originals.dedup();
    for o in originals {
        let _ = s.scene.delete_node(o);
    }
    s.scene.selection = vec![t];
    s.comp.sel = Sel::default();
    s.comp.node = None;
    Ok(s.scene.name_of(t))
}

fn separate(s: &mut Session) -> Result<String, String> {
    let shape = current_shape(s).ok_or("Select a polygon object first")?;
    let t = transform_of(s, shape);
    let (poly, material) = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => (md.poly(), md.material),
        _ => return Err("Not a mesh".to_string()),
    };
    // connected shells through shared vertices
    let n = poly.pos.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let nx = p[c];
            p[c] = r;
            c = nx;
        }
        r
    }
    for f in &poly.faces {
        for w in f.windows(2) {
            let (a, b) = (find(&mut parent, w[0] as usize), find(&mut parent, w[1] as usize));
            if a != b {
                parent[a] = b;
            }
        }
    }
    let mut shells: HashMap<usize, Vec<usize>> = HashMap::new();
    for (fi, f) in poly.faces.iter().enumerate() {
        let r = find(&mut parent, f[0] as usize);
        shells.entry(r).or_default().push(fi);
    }
    if shells.len() < 2 {
        return Err("The object has only one shell".to_string());
    }
    let mut groups: Vec<Vec<usize>> = shells.into_values().collect();
    groups.sort();
    let attrs = ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"];
    let vals: Vec<f32> = attrs.iter().map(|a| s.scene.get_attr(t, a).unwrap_or(0.0)).collect();
    let parent_node = s.scene.node(t).and_then(|n| n.parent);
    let base = s.scene.name_of(t);
    let mut created = Vec::new();
    for g in &groups {
        let mut map: HashMap<u32, u32> = HashMap::new();
        let mut p = Poly::default();
        for &fi in g {
            let nf: Vec<u32> = poly.faces[fi]
                .iter()
                .map(|v| {
                    *map.entry(*v).or_insert_with(|| {
                        p.pos.push(poly.pos[*v as usize]);
                        (p.pos.len() - 1) as u32
                    })
                })
                .collect();
            p.faces.push(nf);
        }
        let nt = s.scene.create_node(NodeData::Transform, &base, parent_node, true);
        for (a, v) in attrs.iter().zip(&vals) {
            let _ = s.scene.set_attr(nt, a, *v);
        }
        let sname = format!("{}Shape", s.scene.name_of(nt));
        let md = MeshData { cache: Default::default(), history: Vec::new(), mesh: p.to_mesh(), prim: "custom".to_string(), params: Vec::new(), material, ..Default::default() };
        s.scene.create_node(NodeData::Mesh(md), &sname, Some(nt), false);
        created.push(nt);
    }
    let _ = s.scene.delete_node(t);
    s.scene.selection = created.clone();
    s.comp.sel = Sel::default();
    s.comp.node = None;
    Ok(format!("Separated into {} objects", created.len()))
}

// ---------------------------------------------------------------------- component specs

/// Parse `name.f[1:3]`, `name.e[*]`, `name.vtx[0]` into (name, mode, indices-or-all).
pub fn parse_component(arg: &str) -> Option<(String, SelMode, Option<Vec<u32>>)> {
    let br = arg.find('[')?;
    let head = &arg[..br];
    let dot = head.rfind('.')?;
    let (name, kind) = (&head[..dot], &head[dot + 1..]);
    let mode = match kind {
        "f" => SelMode::Face,
        "e" => SelMode::Edge,
        "vtx" | "v" => SelMode::Vertex,
        _ => return None,
    };
    let inner = arg[br + 1..].strip_suffix(']')?;
    if inner == "*" {
        return Some((name.to_string(), mode, None));
    }
    let mut ids = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        if let Some((a, b)) = part.split_once(':') {
            let (a, b) = (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?);
            ids.extend(a.min(b)..=a.max(b));
        } else {
            ids.push(part.parse().ok()?);
        }
    }
    Some((name.to_string(), mode, Some(ids)))
}

pub fn select_components(s: &mut Session, args: &[String], add: bool) -> Result<(), String> {
    let expanded = super::cmd_nurbs::expand_cv_args(s, args)?;
    let args = &expanded;
    let mut mode = None;
    let mut node = None;
    let mut verts = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut faces = BTreeSet::new();
    for a in args {
        let (name, m, ids) = parse_component(a).ok_or_else(|| format!("Bad component name: {}", a))?;
        let id = s.scene.find(&name).ok_or_else(|| format!("No object matches name: {}", name))?;
        let shape = if matches!(s.scene.node(id).map(|n| &n.data), Some(NodeData::Mesh(_)) | Some(NodeData::NurbsCurve(_)) | Some(NodeData::Lattice(_))) {
            id
        } else if let Some(sh) = s.scene.mesh_shapes_under(id).first() {
            *sh
        } else if let Some(sh) = s.scene.curve_shapes_under(id).first() {
            *sh
        } else {
            *s.scene.lattice_shapes_under(id).first().ok_or_else(|| format!("{} has no mesh", name))?
        };
        if node.map(|n| n != shape).unwrap_or(false) {
            return Err("Components of one object at a time".to_string());
        }
        node = Some(shape);
        mode = Some(m);
        let poly = match mesh_mut(s, shape) {
            Some(md) => {
                md.ensure_custom();
                md.poly()
            }
            None => s.scene.shape_poly(shape).ok_or("Not a mesh")?.0,
        };
        let edge_list = poly.edge_list();
        match m {
            SelMode::Vertex => {
                let all: Vec<u32> = ids.unwrap_or_else(|| (0..poly.pos.len() as u32).collect());
                for v in all {
                    if (v as usize) >= poly.pos.len() {
                        return Err(format!("vertex {} out of range", v));
                    }
                    verts.insert(v);
                }
            }
            SelMode::Edge => {
                let all: Vec<u32> = ids.unwrap_or_else(|| (0..edge_list.len() as u32).collect());
                for e in all {
                    edges.insert(*edge_list.get(e as usize).ok_or_else(|| format!("edge {} out of range", e))?);
                }
            }
            _ => {
                let all: Vec<u32> = ids.unwrap_or_else(|| (0..poly.faces.len() as u32).collect());
                for f in all {
                    if (f as usize) >= poly.faces.len() {
                        return Err(format!("face {} out of range", f));
                    }
                    faces.insert(f);
                }
            }
        }
    }
    let (node, mode) = match (node, mode) {
        (Some(n), Some(m)) => (n, m),
        _ => return Ok(()),
    };
    if !add || s.comp.node != Some(node) || s.comp.mode != mode {
        s.comp.sel = Sel::default();
    }
    s.comp.sel.verts.extend(verts);
    s.comp.sel.edges.extend(edges);
    s.comp.sel.faces.extend(faces);
    s.comp.node = Some(node);
    s.comp.mode = mode;
    if let Some(t) = s.scene.node(node).and_then(|n| n.parent) {
        s.scene.selection = vec![t];
    }
    Ok(())
}

// ---------------------------------------------------------------------- MEL commands

const OP_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["ltz", "localTranslateZ"], arity: 1 },
    FlagSpec { names: &["offset", "o"], arity: 1 },
    FlagSpec { names: &["d", "distance"], arity: 1 },
    FlagSpec { names: &["dv", "divisions"], arity: 1 },
    FlagSpec { names: &["axis", "ax"], arity: 1 },
    FlagSpec { names: &["t", "position"], arity: 1 },
    FlagSpec { names: &["cv", "cleanVertices"], arity: 0 },
    FlagSpec { names: &["nm", "normalMode"], arity: 1 },
    FlagSpec { names: &["object"], arity: 0 },
    FlagSpec { names: &["vertex"], arity: 0 },
    FlagSpec { names: &["edge"], arity: 0 },
    FlagSpec { names: &["face"], arity: 0 },
    FlagSpec { names: &["toVertex"], arity: 0 },
    FlagSpec { names: &["toEdge"], arity: 0 },
    FlagSpec { names: &["toFace"], arity: 0 },
    FlagSpec { names: &["dup", "duplicate"], arity: 1 },
    FlagSpec { names: &["center", "mergeToCenter"], arity: 0 },
    FlagSpec { names: &["negative", "neg"], arity: 0 },
];

pub fn command(s: &mut Session, name: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, OP_SPEC)?;
    // an optional trailing component argument selects first, e.g. polyBevel -offset 0.1 pCube1.e[0:3]
    let comps: Vec<String> = p.arg_strings().into_iter().filter(|a| a.contains('[')).collect();
    if !comps.is_empty() {
        select_components(s, &comps, false)?;
    }
    let num = |k: &str, d: f32| -> Result<f32, String> { Ok(p.num(k)?.unwrap_or(d)) };
    let op = match name {
        "polyExtrudeFacet" | "polyExtrudeFace" | "polyExtrudeEdge" => ModelOp::Extrude { ltz: num("ltz", 0.0)? },
        "polyBevel" => ModelOp::Bevel { offset: num("offset", 0.2)? },
        "polyDelFacet" | "polyDelEdge" | "polyDelVertex" => {
            let want = match name {
                "polyDelFacet" => SelMode::Face,
                "polyDelEdge" => SelMode::Edge,
                _ => SelMode::Vertex,
            };
            if s.comp.mode != want {
                return Err(format!("{} needs a {:?} selection", name, want).to_lowercase());
            }
            ModelOp::Delete { clean: p.has("cv") || want != SelMode::Edge }
        }
        "polyMergeVertex" => {
            if p.has("center") {
                ModelOp::MergeCenter
            } else {
                ModelOp::Merge { dist: num("d", 0.001)? }
            }
        }
        "polyMergeEdge" => ModelOp::MergeEdge,
        "polyBridgeEdge" => ModelOp::Bridge,
        "polyOffsetLoop" => ModelOp::OffsetLoop { offset: num("offset", 0.25)? },
        "polySlideEdge" => ModelOp::SlideEdge { amount: num("t", 0.0)? },
        "polySelectBorder" => ModelOp::SelectBorder,
        "polyMirrorCut" => {
            let axis = match p.string("axis").unwrap_or_else(|| "x".into()).to_lowercase().as_str() {
                "y" | "1" => 1,
                "z" | "2" => 2,
                _ => 0,
            };
            ModelOp::MirrorCut { axis, offset: num("offset", 0.0)?, negative: p.has("negative") }
        }
        "polyCollapseEdge" | "polyCollapseFacet" => ModelOp::Collapse,
        "polySmooth" => ModelOp::Smooth { levels: num("dv", 1.0)?.max(1.0) as u32 },
        "polyTriangulate" => ModelOp::Triangulate,
        "polyNormal" => ModelOp::Reverse,
        "polyMirrorFace" => {
            let axis = match p.string("axis").unwrap_or_else(|| "x".into()).to_lowercase().as_str() {
                "y" | "1" => 1,
                "z" | "2" => 2,
                _ => 0,
            };
            ModelOp::Mirror { axis }
        }
        "polyCloseBorder" | "polyFillHole" => ModelOp::FillHole,
        "polyUnite" => ModelOp::Combine,
        "polySeparate" => ModelOp::Separate,
        "polyChipOff" => ModelOp::ChipOff { dup: num("dup", 0.0)? > 0.5 },
        "polyExtract" => {
            run_op(s, ModelOp::ChipOff { dup: false })?;
            let msg = run_op(s, ModelOp::Separate)?;
            return Ok(vec![msg]);
        }
        "polySelectEdgeLoop" => ModelOp::SelectLoop,
        "polyGrowSelection" => ModelOp::Grow,
        "polyConvertSelection" => {
            let to = if p.has("toVertex") {
                SelMode::Vertex
            } else if p.has("toEdge") {
                SelMode::Edge
            } else {
                SelMode::Face
            };
            ModelOp::Convert(to)
        }
        "polySplitRing" => {
            let shape = current_shape(s).ok_or("Select an edge first")?;
            let edge = match (s.comp.node == Some(shape), s.comp.sel.edges.iter().next()) {
                (true, Some(e)) => *e,
                _ => return Err("Select an edge first".to_string()),
            };
            ModelOp::InsertLoop { edge, t: num("t", 0.5)? }
        }
        _ => return Err(format!("Unknown modeling command: {}", name)),
    };
    let msg = run_op(s, op)?;
    Ok(vec![msg])
}

pub fn select_mode(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, OP_SPEC)?;
    let mode = if p.has("vertex") {
        SelMode::Vertex
    } else if p.has("edge") {
        SelMode::Edge
    } else if p.has("face") {
        SelMode::Face
    } else {
        SelMode::Object
    };
    // the shape comes from the object selection, not from the component node of the last mode
    s.comp.mode = SelMode::Object;
    let shape = current_shape(s);
    s.comp.mode = mode;
    s.comp.sel = Sel::default();
    if mode != SelMode::Object {
        s.comp.node = shape;
        if s.comp.node.is_none() {
            s.comp.mode = SelMode::Object;
            return Err("Select a polygon object first".to_string());
        }
    }
    Ok(vec![format!("{:?} selection mode", mode)])
}

pub fn poly_count(s: &mut Session) -> Out {
    let shape = current_shape(s).ok_or("Select a polygon object first")?;
    if let Some(NodeData::Mesh(md)) = s.scene.node(shape).map(|n| &n.data) {
        let poly = md.poly();
        let e = poly.edge_list().len();
        return Ok(vec![format!(
            "{}: verts {}  edges {}  faces {}  tris {}",
            s.scene.name_of(shape),
            poly.pos.len(),
            e,
            poly.faces.len(),
            md.mesh.triangle_count()
        )]);
    }
    Err("Not a mesh".to_string())
}

pub const MODEL_COMMANDS: &[&str] = &[
    "polyExtrudeFacet",
    "polyExtrudeFace",
    "polyExtrudeEdge",
    "polyBevel",
    "polyDelFacet",
    "polyDelEdge",
    "polyDelVertex",
    "polyMergeVertex",
    "polyMergeEdge",
    "polyBridgeEdge",
    "polyOffsetLoop",
    "polySlideEdge",
    "polySelectBorder",
    "polyMirrorCut",
    "polyCollapseEdge",
    "polyCollapseFacet",
    "polySmooth",
    "polyTriangulate",
    "polyNormal",
    "polyMirrorFace",
    "polyCloseBorder",
    "polyFillHole",
    "polyUnite",
    "polySeparate",
    "polySelectEdgeLoop",
    "polyGrowSelection",
    "polyConvertSelection",
    "polySplitRing",
    "polyChipOff",
    "polyExtract",
];

// ---------------------------------------------------------------------- history and vertex commands

pub(super) fn comp_verts(s: &mut Session, args: &[String]) -> Result<(NodeId, Vec<u32>), String> {
    let comps: Vec<String> = args.iter().filter(|a| a.contains('[')).cloned().collect();
    if !comps.is_empty() {
        select_components(s, &comps, false)?;
    }
    let shape = current_shape(s).ok_or("Select a polygon object first")?;
    let poly = s.scene.shape_poly(shape).ok_or("Not a mesh")?.0;
    let (sel, mode) = effective(s, shape, &poly);
    let mode = if mode == SelMode::Object { SelMode::Vertex } else { mode };
    let verts: Vec<u32> = verts_of(&poly, &sel, mode).into_iter().collect();
    if verts.is_empty() {
        return Err("Select vertices first".to_string());
    }
    Ok((shape, verts))
}

const MOVE_SPEC: &[FlagSpec] = &[FlagSpec { names: &["t", "translate"], arity: 3 }, FlagSpec { names: &["r", "relative"], arity: 0 }];

pub fn move_vertex(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, MOVE_SPEC)?;
    let t = p.get("t").ok_or("Usage: polyMoveVertex -t x y z [obj.vtx[..]]")?;
    if t.len() < 3 {
        return Err("polyMoveVertex -t needs x y z".to_string());
    }
    let d = Vec3::new(t[0].as_f32()?, t[1].as_f32()?, t[2].as_f32()?);
    let (shape, verts) = comp_verts(s, &p.arg_strings())?;
    s.scene.flush_meshes();
    let pos: Vec<Vec3> = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) if md.nurbs.is_none() => md.mesh.positions.clone(),
        // control points of NURBS shapes
        Some(_) => s.scene.shape_poly(shape).ok_or("Not a mesh")?.0.pos,
        _ => return Err("Not a mesh".to_string()),
    };
    let edits: Vec<(u32, Vec3)> = verts.iter().filter(|v| (**v as usize) < pos.len()).map(|v| (*v, pos[*v as usize] + d)).collect();
    s.scene.tweak_vertices(shape, &edits)?;
    Ok(vec![format!("Moved {} vertices", edits.len())])
}

pub fn key_vertices(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let (shape, verts) = comp_verts(s, &p.arg_strings())?;
    let layer = s.scene.anim.active_layer;
    let n = s.scene.key_vertices(shape, &verts, layer)?;
    Ok(vec![format!("Keyed {} vertex channels", n)])
}

pub fn point_position(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let (shape, verts) = comp_verts(s, &p.arg_strings())?;
    s.scene.flush_meshes();
    let md = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md,
        _ => return Err("Not a mesh".to_string()),
    };
    Ok(verts
        .iter()
        .filter_map(|v| md.mesh.positions.get(*v as usize).map(|p| format!("{} {} {}", super::commands::fmt_f(p.x), super::commands::fmt_f(p.y), super::commands::fmt_f(p.z))))
        .collect())
}

pub fn delete_history(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = p.arg_strings();
    let mut shapes: Vec<NodeId> = Vec::new();
    if names.is_empty() {
        for id in s.scene.selection.clone() {
            shapes.extend(s.scene.mesh_shapes_under(id));
            shapes.extend(s.scene.curve_shapes_under(id));
        }
    } else {
        for n in names {
            let id = s.scene.find(&n).ok_or_else(|| format!("No object matches name: {}", n))?;
            shapes.extend(s.scene.mesh_shapes_under(id));
            shapes.extend(s.scene.curve_shapes_under(id));
        }
    }
    if shapes.is_empty() {
        return Err("Select a polygon object first".to_string());
    }
    let mut n = 0;
    for sh in shapes {
        n += s.scene.delete_history(sh);
    }
    Ok(vec![format!("Deleted {} history node(s)", n)])
}

pub fn list_history(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = p.arg_strings();
    let mut shapes: Vec<NodeId> = Vec::new();
    if names.is_empty() {
        for id in s.scene.selection.clone() {
            shapes.extend(s.scene.mesh_shapes_under(id));
        }
    } else {
        for n in names {
            let id = s.scene.find(&n).ok_or_else(|| format!("No object matches name: {}", n))?;
            shapes.extend(s.scene.mesh_shapes_under(id));
        }
    }
    let mut out = Vec::new();
    for sh in shapes {
        for h in s.scene.history_of(sh).into_iter().rev() {
            out.push(s.scene.name_of(h));
        }
    }
    Ok(out)
}

const TEX_SPEC: &[FlagSpec] = &[FlagSpec { names: &["clear", "c"], arity: 0 }];

/// assignTexture [-clear] [object|material] "file.png"
pub fn assign_texture(s: &mut Session, rest: &[Token]) -> Out {
    if let Some(r) = super::cmd_tex::assign_texture_cmd(s, rest) {
        return r;
    }
    let p = parse_args(rest, TEX_SPEC)?;
    let args = p.arg_strings();
    let clear = p.has("clear");
    let (target, path) = if clear {
        (args.first().cloned(), None)
    } else {
        match args.len() {
            1 => (None, Some(args[0].clone())),
            2 => (Some(args[0].clone()), Some(args[1].clone())),
            _ => return Err("Usage: assignTexture [object|material] \"file.png\"   (or -clear)".to_string()),
        }
    };
    let id = match target {
        Some(n) => s.scene.find(&n).ok_or_else(|| format!("No object matches name: {}", n))?,
        None => *s.scene.selection.first().ok_or("Select an object or material first")?,
    };
    let mat = if matches!(s.scene.node(id).map(|n| &n.data), Some(NodeData::Material)) {
        id
    } else {
        let shapes = s.scene.mesh_shapes_under(id);
        let first = *shapes.first().ok_or("Select a polygon object or a material")?;
        let existing = match s.scene.node(first).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.material,
            _ => None,
        };
        match existing {
            Some(m) => m,
            None => {
                if clear {
                    return Err("The object has no material".to_string());
                }
                let m = s.scene.create_material(None);
                s.scene.assign_material(m, id)?;
                m
            }
        }
    };
    match path {
        Some(path) => {
            crate::render::texture::invalidate(&path);
            if crate::render::texture::load(&s.scene.texture_file(&path)).is_none() {
                return Err(format!("Cannot read image: {}", path));
            }
            let stored = s.scene.texture_path_for_storage(&path);
            s.scene.set_color_map(mat, Some(&stored))?;
            s.view.show_textures();
            Ok(vec![format!("{}: color map {}", s.scene.name_of(mat), path)])
        }
        None => {
            s.scene.set_color_map(mat, None)?;
            Ok(vec![format!("{}: color map cleared", s.scene.name_of(mat))])
        }
    }
}

/// imagePlane "file.png": a vertical plane with the image's aspect ratio and the image as color map.
pub fn image_plane(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let path = p.arg_strings().first().cloned().ok_or("Usage: imagePlane \"file.png\"")?;
    crate::render::texture::invalidate(&path);
    let tex = crate::render::texture::load(&path).ok_or_else(|| format!("Cannot read image: {}", path))?;
    let aspect = tex.w as f32 / tex.h.max(1) as f32;
    let h = 5.0;
    let params = vec![
        ("width".to_string(), h * aspect),
        ("height".to_string(), h),
        ("subdivisions".to_string(), 1.0),
    ];
    let (t, _) = s.scene.create_mesh_object("polyPlane", params, Some("imagePlane"))?;
    s.scene.set_attr(t, "rotateX", 90.0)?;
    s.scene.set_attr(t, "translateY", h * 0.5)?;
    s.scene.selection = vec![t];
    s.view.show_textures();
    let name = s.scene.name_of(t);
    let m = s.scene.create_material(None);
    s.scene.assign_material(m, t)?;
    s.scene.set_color_map(m, Some(&path))?;
    // a flat image reads best with no shading ramp
    s.scene.set_attr(m, "ambient", 0.55)?;
    s.scene.set_attr(m, "diffuse", 0.55)?;
    s.scene.set_attr(m, "specular", 0.0)?;
    Ok(vec![name])
}
