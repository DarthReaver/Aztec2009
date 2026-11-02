//! NURBS curves and surfaces in the scene: creation, generators ("history") and control point edits.

use super::graph::{NodeId, Scene};
use super::nodes::{nurbs_gen_plugs, CurveData, MeshData, NodeData, NurbsGen, NurbsSurf};
use crate::math::Mat4;
use super::plugs::Plug;
use crate::geo::mesh::Mesh;
use crate::geo::nurbs::{self, Curve, Surface};
use crate::math::Vec3;
use std::hash::{Hash, Hasher};

/// Tessellation density of the display mesh (samples per span).
const TESS: usize = 4;

fn hash_f(h: &mut impl Hasher, v: f32) {
    v.to_bits().hash(h);
}

impl Scene {
    /// The curve shapes at or under a transform.
    pub fn curve_shapes_under(&self, id: NodeId) -> Vec<NodeId> {
        self.descendants(id).into_iter().filter(|n| matches!(self.nodes.get(*n).map(|x| &x.data), Some(NodeData::NurbsCurve(_)))).collect()
    }

    /// Mesh shapes with a NURBS source at or under a transform.
    pub fn surface_shapes_under(&self, id: NodeId) -> Vec<NodeId> {
        self.descendants(id).into_iter().filter(|n| matches!(self.nodes.get(*n).map(|x| &x.data), Some(NodeData::Mesh(md)) if md.nurbs.is_some())).collect()
    }

    /// Polygon view of a component-editable shape: a mesh's polygons, or the control point hull of NURBS
    /// (a curve's control polygon is a chain of two-point faces).
    pub fn shape_poly(&self, shape: NodeId) -> Option<(crate::geo::edit::Poly, Option<NodeId>)> {
        let n = self.nodes.get(shape)?;
        match &n.data {
            NodeData::Mesh(md) => Some((md.poly(), n.parent)),
            NodeData::NurbsCurve(cd) => {
                let faces = (0..cd.curve.cvs.len().saturating_sub(1)).map(|i| vec![i as u32, i as u32 + 1]).collect();
                Some((crate::geo::edit::Poly { pos: cd.curve.cvs.clone(), faces }, n.parent))
            }
            NodeData::Lattice(ld) => {
                let pos: Vec<Vec3> = (0..ld.count())
                    .map(|idx| {
                        let (i, j, k) = ld.coords(idx);
                        let g = |a: &str| n.plug(&format!("pt_{}_{}_{}_{}", i, j, k, a)).map(|p| p.value).unwrap_or(0.0);
                        ld.rest(idx) + Vec3::new(g("x"), g("y"), g("z"))
                    })
                    .collect();
                // the grid lines as two-point faces
                let mut faces: Vec<Vec<u32>> = Vec::new();
                for i in 0..ld.divs[0] {
                    for j in 0..ld.divs[1] {
                        for k in 0..ld.divs[2] {
                            let a = ld.index(i, j, k) as u32;
                            if i + 1 < ld.divs[0] {
                                faces.push(vec![a, ld.index(i + 1, j, k) as u32]);
                            }
                            if j + 1 < ld.divs[1] {
                                faces.push(vec![a, ld.index(i, j + 1, k) as u32]);
                            }
                            if k + 1 < ld.divs[2] {
                                faces.push(vec![a, ld.index(i, j, k + 1) as u32]);
                            }
                        }
                    }
                }
                Some((crate::geo::edit::Poly { pos, faces }, n.parent))
            }
            _ => None,
        }
    }

    pub fn add_gen_plugs(&mut self, shape: NodeId, kind: &str) {
        if let Some(node) = self.nodes.get_mut(shape) {
            for (k, v) in nurbs_gen_plugs(kind) {
                if node.plug(k).is_none() {
                    node.plugs.push(Plug::new(k, *v));
                }
            }
        }
    }

    fn param(&mut self, id: NodeId, name: &str, default: f32) -> f32 {
        self.get_attr(id, name).unwrap_or(default)
    }

    /// An input curve in world space together with the pivot of its transform.
    pub(crate) fn input_curve_world(&mut self, shape: NodeId) -> Option<(Curve, Vec3)> {
        self.input_curve(shape)
    }

    fn input_curve(&mut self, shape: NodeId) -> Option<(Curve, Vec3)> {
        let c = match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::NurbsCurve(cd)) => cd.curve.clone(),
            _ => return None,
        };
        let parent = self.nodes.get(shape).and_then(|n| n.parent)?;
        let w = self.world_matrix(parent);
        let pivot = self.pivot_world(parent);
        let mut c = c;
        c.transform(|p| w.transform_point(p));
        Some((c, pivot))
    }

    fn gen_key(&mut self, id: NodeId, gen: &NurbsGen) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        gen.kind.hash(&mut h);
        for (k, v) in nurbs_gen_plugs(&gen.kind) {
            let x = self.param(id, k, *v);
            hash_f(&mut h, x);
        }
        for &i in &gen.inputs {
            match self.input_curve(i) {
                Some((c, pivot)) => {
                    for p in &c.cvs {
                        hash_f(&mut h, p.x);
                        hash_f(&mut h, p.y);
                        hash_f(&mut h, p.z);
                    }
                    hash_f(&mut h, pivot.x);
                    hash_f(&mut h, pivot.y);
                    hash_f(&mut h, pivot.z);
                    c.dir.knots.len().hash(&mut h);
                }
                None => 0u8.hash(&mut h),
            }
        }
        h.finish()
    }

    fn generate_curve(&mut self, id: NodeId, gen: &NurbsGen) -> Result<Curve, String> {
        match gen.kind.as_str() {
            "circle" => {
                let r = self.param(id, "radius", 1.0).max(1e-4);
                let n = self.param(id, "sections", 8.0).round().clamp(4.0, 64.0) as usize;
                Ok(nurbs::circle(r, n, 3))
            }
            "square" => {
                let s = self.param(id, "sideLength", 1.0).max(1e-4) * 0.5;
                let pts = vec![Vec3::new(-s, 0.0, -s), Vec3::new(s, 0.0, -s), Vec3::new(s, 0.0, s), Vec3::new(-s, 0.0, s), Vec3::new(-s, 0.0, -s)];
                Ok(Curve::open(pts, 1))
            }
            o => Err(format!("Unknown curve generator '{}'", o)),
        }
    }

    fn generate_surface(&mut self, id: NodeId, gen: &NurbsGen) -> Result<Surface, String> {
        let g = |s: &mut Scene, n: &str, d: f32| s.param(id, n, d);
        let sections = |s: &mut Scene| g(s, "sections", 8.0).round().clamp(3.0, 64.0) as usize;
        let spans = |s: &mut Scene, d: f32| g(s, "spans", d).round().clamp(1.0, 64.0) as usize;
        let centered = |mut s: Surface, h: f32| {
            s.transform(|p| p - Vec3::new(0.0, h * 0.5, 0.0));
            s
        };
        match gen.kind.as_str() {
            "sphere" => Ok(nurbs::sphere(g(self, "radius", 1.0).max(1e-4), sections(self), spans(self, 4.0))),
            "cylinder" => {
                let h = g(self, "height", 2.0);
                Ok(centered(nurbs::cylinder(g(self, "radius", 1.0).max(1e-4), h, sections(self), spans(self, 1.0)), h))
            }
            "cone" => {
                let h = g(self, "height", 2.0);
                Ok(centered(nurbs::cone(g(self, "radius", 1.0).max(1e-4), h, sections(self), spans(self, 1.0)), h))
            }
            "torus" => Ok(nurbs::torus(g(self, "radius", 1.0).max(1e-4), g(self, "sectionRadius", 0.5).max(1e-4), sections(self), spans(self, 8.0))),
            "plane" => {
                let (pu, pv) = (g(self, "patchesU", 1.0).round().clamp(1.0, 32.0) as usize, g(self, "patchesV", 1.0).round().clamp(1.0, 32.0) as usize);
                Ok(nurbs::plane(g(self, "width", 1.0), g(self, "length", 1.0), pu, pv))
            }
            "revolve" => {
                let (curve, pivot) = gen.inputs.first().and_then(|i| self.input_curve(*i)).ok_or("Revolve needs its profile curve")?;
                let axis = Vec3::new(g(self, "axisX", 0.0), g(self, "axisY", 1.0), g(self, "axisZ", 0.0));
                if axis.length_sq() < 1e-12 {
                    return Err("The revolve axis is zero".to_string());
                }
                Ok(nurbs::revolve(&curve, pivot, axis, g(self, "degrees", 360.0).clamp(1.0, 360.0), sections(self), 3))
            }
            "loft" => {
                let curves: Vec<Curve> = gen.inputs.iter().filter_map(|i| self.input_curve(*i).map(|c| c.0)).collect();
                nurbs::loft(&curves)
            }
            "extrude" => {
                let profile = gen.inputs.first().and_then(|i| self.input_curve(*i)).ok_or("Extrude needs a profile curve")?.0;
                let path = gen.inputs.get(1).and_then(|i| self.input_curve(*i)).ok_or("Extrude needs a path curve")?.0;
                Ok(nurbs::extrude_along(&profile, &path))
            }
            o => Err(format!("Unknown surface generator '{}'", o)),
        }
    }

    /// Cluster and point on curve deformers: the control points follow their handles.
    fn apply_curve_drivers(&mut self) {
        let curves = self.ids_where(|d| matches!(d, NodeData::NurbsCurve(cd) if !cd.drivers.is_empty()));
        for id in curves {
            let (drivers, mut base, cvs_now, dir) = match self.nodes.get(id).map(|n| &n.data) {
                Some(NodeData::NurbsCurve(cd)) => (cd.drivers.clone(), cd.driver_base.clone(), cd.curve.cvs.clone(), cd.curve.dir.clone()),
                _ => continue,
            };
            if base.len() != cvs_now.len() {
                base = cvs_now.clone();
            }
            let parent = self.nodes.get(id).and_then(|n| n.parent);
            let cw = match parent {
                Some(p) => self.world_matrix(p),
                None => Mat4::identity(),
            };
            let cw_inv = cw.inverse();
            let mut cvs = base.clone();
            for d in &drivers {
                let Some(h) = d.handle.filter(|h| self.nodes.get(*h).is_some()) else { continue };
                let hw = self.world_matrix(h);
                match &d.kind {
                    super::nodes::CurveDriverKind::Cluster { members, bind_inv } => {
                        let m = hw * *bind_inv;
                        for i in members {
                            if let Some(c) = cvs.get_mut(*i) {
                                *c = cw_inv.transform_point(m.transform_point(cw.transform_point(*c)));
                            }
                        }
                    }
                    super::nodes::CurveDriverKind::PointOnCurve { param, rest } => {
                        let delta = cw_inv.transform_dir(hw.translation_part() - *rest);
                        let (idx, w) = dir.basis(*param);
                        let norm: f32 = w.iter().map(|x| x * x).sum::<f32>().max(1e-9);
                        for (i, wi) in idx.iter().zip(&w) {
                            cvs[*i] += delta * (wi / norm);
                        }
                    }
                }
            }
            let changed = cvs.iter().zip(&cvs_now).any(|(a, b)| a.distance(*b) > 1e-7);
            if let Some(NodeData::NurbsCurve(cd)) = self.nodes.get_mut(id).map(|n| &mut n.data) {
                cd.driver_base = base;
                if changed {
                    cd.curve.cvs = cvs;
                }
            }
            if changed {
                self.mark_skins_dirty();
            }
        }
    }

    /// Rebuild the generated curves and surfaces whose inputs or parameters changed.
    pub fn flush_nurbs(&mut self) {
        self.apply_curve_drivers();
        let curves = self.ids_where(|d| matches!(d, NodeData::NurbsCurve(cd) if cd.gen.is_some()));
        for id in curves {
            let gen = match self.nodes.get(id).map(|n| &n.data) {
                Some(NodeData::NurbsCurve(cd)) => cd.gen.clone().unwrap(),
                _ => continue,
            };
            if gen.inputs.len() != gen.input_names.len() && !gen.input_names.is_empty() {
                continue;
            }
            let key = self.gen_key(id, &gen);
            if matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::NurbsCurve(cd)) if cd.key == key) {
                continue;
            }
            if let Ok(new) = self.generate_curve(id, &gen) {
                if let Some(NodeData::NurbsCurve(cd)) = self.nodes.get_mut(id).map(|n| &mut n.data) {
                    let tweaks: Option<Vec<Vec3>> = (cd.base.len() == cd.curve.cvs.len() && cd.base.len() == new.cvs.len()).then(|| cd.curve.cvs.iter().zip(&cd.base).map(|(a, b)| *a - *b).collect());
                    cd.base = new.cvs.clone();
                    cd.curve = new;
                    if let Some(t) = tweaks {
                        for (c, d) in cd.curve.cvs.iter_mut().zip(t) {
                            *c += d;
                        }
                    }
                    cd.key = key;
                }
            }
        }
        let surfs = self.ids_where(|d| matches!(d, NodeData::Mesh(md) if md.nurbs.is_some()));
        for id in surfs {
            let (gen, empty) = match self.nodes.get(id).map(|n| &n.data) {
                Some(NodeData::Mesh(md)) => (md.nurbs.as_ref().and_then(|n| n.gen.clone()), md.mesh.faces.is_empty()),
                _ => continue,
            };
            let Some(gen) = gen else {
                if empty {
                    self.retessellate(id);
                }
                continue;
            };
            if gen.inputs.len() < gen.input_names.len() {
                continue;
            }
            let key = self.gen_key(id, &gen);
            let same = matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.nurbs.as_ref().map(|n| n.key == key).unwrap_or(false));
            if same {
                if empty {
                    self.retessellate(id);
                }
                continue;
            }
            if let Ok(new) = self.generate_surface(id, &gen) {
                if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(id).map(|n| &mut n.data) {
                    if let Some(ns) = md.nurbs.as_mut() {
                        let tweaks: Option<Vec<Vec3>> = (ns.base.len() == ns.surf.cvs.len() && ns.base.len() == new.cvs.len()).then(|| ns.surf.cvs.iter().zip(&ns.base).map(|(a, b)| *a - *b).collect());
                        ns.base = new.cvs.clone();
                        let trims = std::mem::take(&mut ns.surf.trims);
                        ns.surf = new;
                        ns.surf.trims = trims;
                        if let Some(t) = tweaks {
                            for (c, d) in ns.surf.cvs.iter_mut().zip(t) {
                                *c += d;
                            }
                        }
                        ns.key = key;
                    }
                }
                self.retessellate(id);
            }
        }
    }

    /// Recompute the display mesh and isoparm lines of a NURBS surface.
    pub fn retessellate(&mut self, shape: NodeId) {
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            if let Some(ns) = md.nurbs.as_mut() {
                md.mesh = ns.surf.tessellate(TESS);
                ns.iso = ns.surf.isoparm_lines(6);
                md.prim = "nurbs".to_string();
                md.touch();
            }
        }
    }

    pub fn create_nurbs_curve(&mut self, name: Option<&str>, curve: Curve, gen: Option<(&str, Vec<NodeId>)>) -> Result<NodeId, String> {
        let base = match &gen {
            Some((k, _)) if *k == "circle" => "nurbsCircle",
            Some((k, _)) if *k == "square" => "nurbsSquare",
            _ => "curve",
        };
        let (tname, force) = match name {
            Some(n) => (n, false),
            None => (base, true),
        };
        let t = self.create_node(NodeData::Transform, tname, None, force);
        let sname = format!("{}Shape", self.name_of(t));
        let kind = gen.as_ref().map(|g| g.0.to_string());
        let data = CurveData { drivers: Vec::new(), driver_base: Vec::new(), on_surface: None, base: curve.cvs.clone(), curve, gen: gen.map(|(k, inputs)| NurbsGen { kind: k.to_string(), inputs, input_names: Vec::new() }), key: 0 };
        let shape = self.create_node(NodeData::NurbsCurve(Box::new(data)), &sname, Some(t), false);
        let _ = kind;
        self.flush_nurbs();
        let _ = shape;
        Ok(t)
    }

    /// A NURBS surface from a generator (`kind` + inputs) or, with `surf`, from explicit geometry.
    pub fn create_nurbs_surface(&mut self, name: Option<&str>, kind: Option<&str>, inputs: Vec<NodeId>, surf: Option<Surface>) -> Result<NodeId, String> {
        let default_name = match kind {
            Some("sphere") => "nurbsSphere",
            Some("cylinder") => "nurbsCylinder",
            Some("cone") => "nurbsCone",
            Some("torus") => "nurbsTorus",
            Some("plane") => "nurbsPlane",
            Some("revolve") => "revolvedSurface",
            Some("loft") => "loftedSurface",
            Some("extrude") => "extrudedSurface",
            _ => "surface",
        };
        let (tname, force) = match name {
            Some(n) => (n, false),
            None => (default_name, true),
        };
        let placeholder = surf.clone().unwrap_or_else(|| nurbs::plane(1.0, 1.0, 1, 1));
        let t = self.create_node(NodeData::Transform, tname, None, force);
        let sname = format!("{}Shape", self.name_of(t));
        let ns = NurbsSurf { base: placeholder.cvs.clone(), surf: placeholder, gen: kind.map(|k| NurbsGen { kind: k.to_string(), inputs, input_names: Vec::new() }), key: 0, iso: Vec::new() };
        let md = MeshData { cache: Default::default(), history: Vec::new(), mesh: Mesh::new(), prim: "nurbs".to_string(), params: Vec::new(), material: None, nurbs: Some(Box::new(ns)), ..Default::default() };
        let shape = self.create_node(NodeData::Mesh(md), &sname, Some(t), false);
        if let Some(k) = kind {
            self.add_gen_plugs(shape, k);
        }
        self.flush_nurbs();
        self.retessellate(shape);
        // a generator that failed leaves the placeholder: report it
        if kind.is_some() {
            if let Some(NodeData::Mesh(md)) = self.nodes.get(shape).map(|n| &n.data) {
                if md.nurbs.as_ref().map(|n| n.key == 0).unwrap_or(false) {
                    let _ = self.delete_node(t);
                    return Err("The surface could not be generated from those curves".to_string());
                }
            }
        }
        Ok(t)
    }

    /// Move control points (local space). Generated shapes keep the edit as an offset from the generator output.
    pub fn set_nurbs_cvs(&mut self, shape: NodeId, edits: &[(u32, Vec3)]) -> bool {
        match self.nodes.get_mut(shape).map(|n| &mut n.data) {
            Some(NodeData::Mesh(md)) if md.nurbs.is_some() => {
                let ns = md.nurbs.as_mut().unwrap();
                for (i, p) in edits {
                    if let Some(c) = ns.surf.cvs.get_mut(*i as usize) {
                        *c = *p;
                    }
                }
                self.retessellate(shape);
                true
            }
            Some(NodeData::NurbsCurve(cd)) => {
                let driven = cd.driver_base.len() == cd.curve.cvs.len();
                for (i, p) in edits {
                    if let Some(c) = cd.curve.cvs.get_mut(*i as usize) {
                        // a driven curve keeps the edit as a change of its undriven control points
                        if driven {
                            cd.driver_base[*i as usize] += *p - *c;
                        }
                        *c = *p;
                    }
                }
                if !self.skins.is_empty() {
                    self.mark_skins_dirty();
                }
                true
            }
            _ => false,
        }
    }

    /// Edit > Delete by Type > History for NURBS shapes: the generated geometry becomes plain geometry
    /// (the generator's channels go away with it).
    pub fn delete_nurbs_history(&mut self, shape: NodeId) -> bool {
        let kind = match self.nodes.get_mut(shape).map(|n| &mut n.data) {
            Some(NodeData::Mesh(md)) => match md.nurbs.as_mut() {
                Some(ns) if ns.gen.is_some() => {
                    let k = ns.gen.take().map(|g| g.kind);
                    ns.base.clear();
                    k
                }
                _ => None,
            },
            Some(NodeData::NurbsCurve(cd)) if cd.gen.is_some() => {
                let k = cd.gen.take().map(|g| g.kind);
                cd.base.clear();
                k
            }
            _ => None,
        };
        let Some(kind) = kind else { return false };
        let names: Vec<&str> = nurbs_gen_plugs(&kind).iter().map(|(k, _)| *k).collect();
        for (src, dst) in self.connections() {
            if (src.node == shape && names.contains(&src.attr.as_str())) || (dst.node == shape && names.contains(&dst.attr.as_str())) {
                let _ = self.disconnect((src.node, src.attr.clone()), (dst.node, dst.attr.clone()));
            }
        }
        if let Some(node) = self.nodes.get_mut(shape) {
            node.plugs.retain(|p| !nurbs_gen_plugs(&kind).iter().any(|(k, _)| *k == p.name));
        }
        true
    }
}
