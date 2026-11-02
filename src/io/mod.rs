//! Import and export of foreign scene formats (Wavefront OBJ and Autodesk FBX).
//!
//! Both formats go through `IoScene`, a flat list of transforms (parents before children) that
//! carry an optional polygon mesh, a material index and animation curves. `export_scene` fills it
//! from a `Scene`; `import_into` turns one back into scene nodes.

pub mod fbx;
pub mod fbx_tree;
pub mod mayaascii;
pub mod obj;

use crate::anim::Interp;
use crate::dag::nodes::{MeshData, NodeData};
use crate::dag::{NodeId, Scene};
use crate::geo::edit::{shading_split, shading_split_zoned, Poly, UvMap};
use crate::geo::Mesh;
use crate::math::{Mat4, Quat, Vec3};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct IoMaterial {
    pub name: String,
    pub color: [f32; 3],
    pub texture: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoKind {
    /// Plain transform.
    Group,
    /// Transform with a mesh shape.
    Mesh,
    Joint,
}

#[derive(Clone, Debug)]
pub struct IoNode {
    pub name: String,
    pub parent: Option<usize>,
    pub kind: IoKind,
    pub t: Vec3,
    /// Maya style XYZ euler angles in degrees.
    pub r: Vec3,
    pub s: Vec3,
    /// Rotate and scale pivots in the node's own space.
    pub rp: Vec3,
    pub sp: Vec3,
    /// Vertices may be split per corner (normals / uvs); importing welds them.
    pub mesh: Mesh,
    pub material: Option<usize>,
    /// Material of each face when a mesh uses several (empty: the whole mesh uses `material`).
    pub face_material: Vec<Option<usize>>,
}

impl IoNode {
    pub fn new(name: &str, parent: Option<usize>, kind: IoKind) -> IoNode {
        IoNode { name: name.to_string(), parent, kind, t: Vec3::ZERO, r: Vec3::ZERO, s: Vec3::ONE, rp: Vec3::ZERO, sp: Vec3::ZERO, mesh: Mesh::new(), material: None, face_material: Vec::new() }
    }

    pub fn local_matrix(&self) -> Mat4 {
        Scene::pivoted(self.t, self.r, self.s, self.rp, self.sp)
    }
}

/// One animated channel: `attr` is translateX .. scaleZ, keys are (frame, value, interpolation).
#[derive(Clone, Debug)]
pub struct IoCurve {
    pub node: usize,
    pub attr: String,
    pub keys: Vec<(f32, f32, Interp)>,
}

#[derive(Clone, Debug, Default)]
pub struct IoScene {
    pub nodes: Vec<IoNode>,
    pub materials: Vec<IoMaterial>,
    pub curves: Vec<IoCurve>,
    pub fps: f32,
    /// Things the importer could not carry over.
    pub warnings: Vec<String>,
}

impl IoScene {
    pub fn world_matrices(&self) -> Vec<Mat4> {
        let mut w: Vec<Mat4> = Vec::with_capacity(self.nodes.len());
        for n in &self.nodes {
            let l = n.local_matrix();
            let m = match n.parent {
                Some(p) if p < w.len() => w[p] * l,
                _ => l,
            };
            w.push(m);
        }
        w
    }

    pub fn mesh_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.kind == IoKind::Mesh && !n.mesh.faces.is_empty()).count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Obj,
    Fbx,
    /// Maya ASCII (.ma): transforms, polygon meshes, joints, materials and animation curves.
    Maya,
}

pub fn format_of(path: &str) -> Option<Format> {
    let l = path.to_ascii_lowercase();
    if l.ends_with(".obj") {
        Some(Format::Obj)
    } else if l.ends_with(".fbx") {
        Some(Format::Fbx)
    } else if l.ends_with(".ma") {
        Some(Format::Maya)
    } else {
        None
    }
}

/// Turn any string into something usable as a node name.
pub fn sanitize_name(s: &str) -> String {
    let s = s.split('\0').next().unwrap_or("");
    let s = s.rsplit("::").next().unwrap_or(s);
    let mut o: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if o.is_empty() {
        o.push_str("node");
    }
    if o.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        o.insert(0, '_');
    }
    o
}

// ====================================================================== scene -> IoScene

const XFORM_ATTRS: [&str; 9] = ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"];

fn is_xform(scene: &Scene, id: NodeId) -> bool {
    matches!(scene.node(id).map(|n| &n.data), Some(NodeData::Transform) | Some(NodeData::Joint))
}

fn mesh_child(scene: &Scene, id: NodeId) -> Option<NodeId> {
    let n = scene.node(id)?;
    n.children.iter().copied().find(|c| matches!(scene.node(*c).map(|x| &x.data), Some(NodeData::Mesh(_))))
}

/// Collect transforms, meshes (in shading-split form with generated UVs), materials and base layer
/// animation. With `selected_only` only the selection (and its children) is exported, plus the
/// parents needed to keep the hierarchy.
pub fn export_scene(scene: &mut Scene, selected_only: bool) -> Result<IoScene, String> {
    scene.flush_meshes();
    let mut wanted: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    if selected_only {
        if scene.selection.is_empty() {
            return Err("Nothing is selected".to_string());
        }
        for &s in &scene.selection.clone() {
            let t = if is_xform(scene, s) { Some(s) } else { scene.node(s).and_then(|n| n.parent) };
            if let Some(t) = t {
                wanted.insert(t);
                wanted.extend(scene.descendants(t));
            }
        }
    }
    fn useful(scene: &Scene, id: NodeId, wanted: &std::collections::HashSet<NodeId>, sel: bool) -> bool {
        if !is_xform(scene, id) {
            return false;
        }
        let own = (!sel || wanted.contains(&id)) && (matches!(scene.node(id).map(|n| &n.data), Some(NodeData::Joint)) || mesh_child(scene, id).is_some());
        own || scene.node(id).map(|n| n.children.iter().any(|c| useful(scene, *c, wanted, sel))).unwrap_or(false)
    }
    let mut out = IoScene { fps: scene.timeline.fps, ..Default::default() };
    let mut index: HashMap<NodeId, usize> = HashMap::new();
    let mut mat_index: HashMap<NodeId, usize> = HashMap::new();
    let roots = scene.roots.clone();
    let mut stack: Vec<(NodeId, Option<usize>)> = roots.into_iter().rev().map(|r| (r, None)).collect();
    while let Some((id, parent)) = stack.pop() {
        if !useful(scene, id, &wanted, selected_only) {
            continue;
        }
        let is_joint = matches!(scene.node(id).map(|n| &n.data), Some(NodeData::Joint));
        let shape = mesh_child(scene, id);
        let kind = if is_joint {
            IoKind::Joint
        } else if shape.is_some() && (!selected_only || wanted.contains(&id)) {
            IoKind::Mesh
        } else {
            IoKind::Group
        };
        let mut node = IoNode::new(&scene.name_of(id), parent, kind);
        let g = |scene: &mut Scene, a: &str, d: f32| scene.get_attr(id, a).unwrap_or(d);
        node.t = Vec3::new(g(scene, "translateX", 0.0), g(scene, "translateY", 0.0), g(scene, "translateZ", 0.0));
        node.r = Vec3::new(g(scene, "rotateX", 0.0), g(scene, "rotateY", 0.0), g(scene, "rotateZ", 0.0));
        node.s = Vec3::new(g(scene, "scaleX", 1.0), g(scene, "scaleY", 1.0), g(scene, "scaleZ", 1.0));
        node.rp = Vec3::new(g(scene, "rotatePivotX", 0.0), g(scene, "rotatePivotY", 0.0), g(scene, "rotatePivotZ", 0.0)) + Vec3::new(g(scene, "rotatePivotTranslateX", 0.0), g(scene, "rotatePivotTranslateY", 0.0), g(scene, "rotatePivotTranslateZ", 0.0));
        node.sp = Vec3::new(g(scene, "scalePivotX", 0.0), g(scene, "scalePivotY", 0.0), g(scene, "scalePivotZ", 0.0));
        if let (IoKind::Mesh, Some(sh)) = (kind, shape) {
            let sg = |scene: &mut Scene, a: &str, d: f32| scene.get_attr(sh, a).unwrap_or(d);
            let uvmap = UvMap {
                mode: sg(scene, "uvMode", 0.0) as u8,
                tile_u: sg(scene, "uvTilingU", 1.0),
                tile_v: sg(scene, "uvTilingV", 1.0),
                off_u: sg(scene, "uvOffsetU", 0.0),
                off_v: sg(scene, "uvOffsetV", 0.0),
                rot_deg: sg(scene, "uvRotate", 0.0),
            };
            let angle = scene.get_attr(sh, "smoothAngle").unwrap_or(40.0).clamp(0.0, 180.0);
            // materials and projections per texture zone: face f uses zone face_zone[f]
            let mut zone_mats: Vec<Option<NodeId>> = Vec::new();
            let mut zone_maps: Vec<Option<UvMap>> = Vec::new();
            let mut fz: Vec<u16> = Vec::new();
            let base_mat = match scene.node(sh).map(|n| &n.data) {
                Some(NodeData::Mesh(md)) => {
                    fz = scene.face_zones(sh).unwrap_or_default();
                    md.material
                }
                _ => None,
            };
            let nz = match scene.node(sh).map(|n| &n.data) {
                Some(NodeData::Mesh(md)) => md.zones.len(),
                _ => 0,
            };
            for z in 0..=nz {
                let (m, uv) = scene.zone_info(sh, z as u16).unwrap_or((base_mat, uvmap));
                zone_mats.push(m);
                zone_maps.push(Some(uv));
            }
            if let Some(NodeData::Mesh(md)) = scene.node(sh).map(|n| &n.data) {
                node.mesh = if nz > 0 { shading_split_zoned(&md.mesh, angle, &fz, &zone_maps) } else { shading_split(&md.mesh, angle, Some(&uvmap)) };
                // editable UVs replace the projection
                if let Some(set) = &md.uv_set {
                    set.apply_to(&mut node.mesh);
                }
            }
            let mut intern = |scene: &mut Scene, out: &mut IoScene, m: NodeId| -> usize {
                if let Some(i) = mat_index.get(&m) {
                    return *i;
                }
                let mg = |scene: &mut Scene, a: &str| scene.get_attr(m, a).unwrap_or(0.7);
                let color = [mg(scene, "colorR"), mg(scene, "colorG"), mg(scene, "colorB")];
                let texture = scene.color_map(m).filter(|p| !p.starts_with("builtin:"));
                out.materials.push(IoMaterial { name: scene.name_of(m), color, texture });
                mat_index.insert(m, out.materials.len() - 1);
                out.materials.len() - 1
            };
            node.material = base_mat.map(|m| intern(scene, &mut out, m));
            if nz > 0 {
                let per_zone: Vec<Option<usize>> = zone_mats.iter().map(|m| m.map(|m| intern(scene, &mut out, m))).collect();
                node.face_material = fz.iter().map(|z| per_zone.get(*z as usize).copied().flatten()).collect();
            }
        }
        out.nodes.push(node);
        let me = out.nodes.len() - 1;
        index.insert(id, me);
        if let Some(n) = scene.node(id) {
            for &c in n.children.iter().rev() {
                stack.push((c, Some(me)));
            }
        }
    }
    for b in &scene.anim.bindings {
        if b.layer != 0 || b.curve.keys.is_empty() {
            continue;
        }
        let Some(&ni) = index.get(&b.node) else { continue };
        if !XFORM_ATTRS.contains(&b.attr.as_str()) {
            continue;
        }
        let keys = b.curve.keys.iter().map(|k| (k.t, k.v, k.interp)).collect();
        out.curves.push(IoCurve { node: ni, attr: b.attr.clone(), keys });
    }
    if out.nodes.is_empty() {
        return Err("Nothing to export: no meshes or joints".to_string());
    }
    Ok(out)
}

// ====================================================================== IoScene -> scene

pub struct ImportReport {
    pub top: Vec<NodeId>,
    pub meshes: usize,
    pub joints: usize,
    pub materials: usize,
    pub curves: usize,
    pub faces: usize,
}

pub fn import_into(scene: &mut Scene, io: &IoScene) -> ImportReport {
    let mut rep = ImportReport { top: Vec::new(), meshes: 0, joints: 0, materials: 0, curves: 0, faces: 0 };
    let mut mats: Vec<NodeId> = Vec::new();
    for m in &io.materials {
        let id = scene.create_material(Some(&sanitize_name(&m.name)));
        for (i, a) in ["colorR", "colorG", "colorB"].iter().enumerate() {
            let _ = scene.set_attr(id, a, m.color[i]);
        }
        if let Some(t) = &m.texture {
            let _ = scene.set_color_map(id, Some(t));
        }
        mats.push(id);
        rep.materials += 1;
    }
    let mut ids: Vec<NodeId> = Vec::with_capacity(io.nodes.len());
    for n in &io.nodes {
        let parent = n.parent.and_then(|p| ids.get(p).copied());
        let name = sanitize_name(&n.name);
        let data = if n.kind == IoKind::Joint { NodeData::Joint } else { NodeData::Transform };
        let id = scene.create_node(data, &name, parent, false);
        if n.kind == IoKind::Joint {
            rep.joints += 1;
        }
        for (a, v) in [
            ("translateX", n.t.x), ("translateY", n.t.y), ("translateZ", n.t.z),
            ("rotateX", n.r.x), ("rotateY", n.r.y), ("rotateZ", n.r.z),
            ("scaleX", n.s.x), ("scaleY", n.s.y), ("scaleZ", n.s.z),
        ] {
            if v.is_finite() {
                let _ = scene.set_attr(id, a, v);
            }
        }
        for (a, v) in [
            ("rotatePivotX", n.rp.x), ("rotatePivotY", n.rp.y), ("rotatePivotZ", n.rp.z),
            ("scalePivotX", n.sp.x), ("scalePivotY", n.sp.y), ("scalePivotZ", n.sp.z),
        ] {
            if v.is_finite() && v != 0.0 {
                let _ = scene.set_attr(id, a, v);
            }
        }
        if n.kind == IoKind::Mesh && !n.mesh.faces.is_empty() {
            let (poly, fmap) = Poly::welded_with_faces(&n.mesh, 1e-5);
            if !poly.faces.is_empty() {
                let sname = format!("{}Shape", scene.name_of(id));
                let md = MeshData { cache: Default::default(), history: Vec::new(), mesh: poly.to_mesh(), prim: "custom".to_string(), params: Vec::new(), material: None, ..Default::default() };
                rep.faces += poly.faces.len();
                let shape = scene.create_node(NodeData::Mesh(md), &sname, Some(id), false);
                // the base material is the one most faces use; the others become texture zones
                let mut count: HashMap<usize, usize> = HashMap::new();
                for m in n.face_material.iter().flatten() {
                    *count.entry(*m).or_default() += 1;
                }
                let base = if n.face_material.is_empty() { n.material } else { count.iter().max_by_key(|(m, c)| (**c, std::cmp::Reverse(**m))).map(|(m, _)| *m).or(n.material) };
                if let Some(m) = base.and_then(|m| mats.get(m)) {
                    let _ = scene.assign_material(*m, id);
                }
                let mut groups: Vec<(usize, Vec<u32>)> = Vec::new();
                for (old, fm) in n.face_material.iter().enumerate() {
                    let (Some(fm), Some(Some(new))) = (fm, fmap.get(old)) else { continue };
                    if Some(*fm) == base {
                        continue;
                    }
                    match groups.iter_mut().find(|g| g.0 == *fm) {
                        Some(g) => g.1.push(*new),
                        None => groups.push((*fm, vec![*new])),
                    }
                }
                for (m, faces) in groups {
                    if let Some(mat) = mats.get(m) {
                        let _ = scene.assign_faces(shape, &faces, Some(*mat), None);
                    }
                }
                // UVs in the file become an editable UV set when the faces survived the weld one to one
                if fmap.iter().enumerate().all(|(i, f)| *f == Some(i as u32)) && n.mesh.uvs.len() == n.mesh.positions.len() && n.mesh.uvs.iter().any(|u| u.x != 0.0 || u.y != 0.0) {
                    if let Some(NodeData::Mesh(md)) = scene.nodes.get_mut(shape).map(|x| &mut x.data) {
                        let weld = crate::geo::uv::weld_faces(&md.mesh);
                        let corners: Vec<Vec<crate::math::Vec2>> = n.mesh.faces.iter().map(|f| f.iter().map(|&v| n.mesh.uvs[v as usize]).collect()).collect();
                        if weld.len() == corners.len() && weld.iter().zip(&corners).all(|(a, b)| a.len() == b.len()) {
                            md.uv_set = Some(crate::geo::uv::UvSet::from_corners(&weld, &corners));
                            md.sync_uvs();
                        }
                    }
                }
                rep.meshes += 1;
            }
        }
        if parent.is_none() {
            rep.top.push(id);
        }
        ids.push(id);
    }
    let mut last = scene.timeline.end;
    let tscale = scene.timeline.fps / io.fps.max(1.0);
    for c in &io.curves {
        let Some(&id) = ids.get(c.node) else { continue };
        for &(t, v, interp) in &c.keys {
            let t = t * tscale;
            if scene.set_keyframe_noeval(id, &c.attr, Some(t), Some(v), interp, 0).is_ok() {
                last = last.max(t);
            }
        }
        rep.curves += 1;
    }
    if rep.curves > 0 {
        let start = scene.timeline.start;
        scene.timeline.set_range(start, last.ceil());
        scene.evaluate_anim();
    }
    scene.selection = rep.top.clone();
    rep
}

// ====================================================================== entry points

/// Read a file into the intermediate form.
pub fn read_file(path: &str) -> Result<IoScene, String> {
    match format_of(path) {
        Some(Format::Obj) => obj::read(path),
        Some(Format::Fbx) => fbx::read(path),
        Some(Format::Maya) => mayaascii::read(path),
        None => Err(format!("Unsupported file type '{}': use .obj, .fbx or .ma", path)),
    }
}

pub struct ExportOptions {
    pub selected_only: bool,
    pub fbx_ascii: bool,
}

/// Write the scene. Returns a human readable summary.
pub fn write_file(scene: &mut Scene, path: &str, opts: &ExportOptions) -> Result<String, String> {
    let fmt = format_of(path).ok_or_else(|| format!("Unsupported file type '{}': use .obj or .fbx", path))?;
    let io = export_scene(scene, opts.selected_only)?;
    match fmt {
        Format::Obj => obj::write(&io, path)?,
        Format::Fbx => fbx::write(&io, path, opts.fbx_ascii)?,
        Format::Maya => return Err("Maya ASCII files can be imported but not exported: use .fbx or .obj".to_string()),
    }
    let joints = io.nodes.iter().filter(|n| n.kind == IoKind::Joint).count();
    Ok(format!(
        "Exported {} ({} mesh{}{}{})",
        path,
        io.mesh_count(),
        if io.mesh_count() == 1 { "" } else { "es" },
        if fmt == Format::Fbx && joints > 0 { format!(", {} joints", joints) } else { String::new() },
        if fmt == Format::Fbx && !io.curves.is_empty() { format!(", {} animation curves", io.curves.len()) } else { String::new() },
    ))
}

/// Import a file into the scene and select what was created.
pub fn import_file(scene: &mut Scene, path: &str) -> Result<String, String> {
    let io = read_file(path)?;
    if io.nodes.is_empty() {
        return Err(format!("'{}' contains no geometry", path));
    }
    let r = import_into(scene, &io);
    let warn = if io.warnings.is_empty() { String::new() } else { format!(" (note: {})", io.warnings.join("; ")) };
    Ok(format!(
        "Imported {}: {} mesh{} ({} faces), {} joints, {} materials, {} animation curves{}",
        path,
        r.meshes,
        if r.meshes == 1 { "" } else { "es" },
        r.faces,
        r.joints,
        r.materials,
        r.curves,
        warn
    ))
}

#[cfg(test)]
mod tests;
