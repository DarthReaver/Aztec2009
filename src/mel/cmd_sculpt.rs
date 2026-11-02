//! Sculpt Geometry Tool (Artisan): push, pull, smooth, relax and erase strokes on polygon meshes and NURBS surfaces.

use super::commands::{resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::NodeId;
use crate::math::Vec3;
use crate::session::Session;
use std::collections::HashMap;

pub const OPS: &[&str] = &["push", "pull", "smooth", "relax", "erase"];

#[derive(Clone, Debug)]
pub struct SculptState {
    pub op: String,
    /// Brush radius in world units.
    pub radius: f32,
    pub opacity: f32,
    pub max_disp: f32,
    pub smooth_strength: u32,
    /// The reference surface updates at the start of every stroke, so strokes build on each other.
    pub update_ref: bool,
    refs: HashMap<NodeId, Vec<Vec3>>,
    ref_normals: HashMap<NodeId, Vec<Vec3>>,
    erase_refs: HashMap<NodeId, Vec<Vec3>>,
}

impl Default for SculptState {
    fn default() -> Self {
        SculptState { op: "push".to_string(), radius: 1.0, opacity: 1.0, max_disp: 0.1, smooth_strength: 1, update_ref: true, refs: HashMap::new(), ref_normals: HashMap::new(), erase_refs: HashMap::new() }
    }
}

fn vertex_normals(pos: &[Vec3], faces: &[Vec<u32>]) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; pos.len()];
    for f in faces.iter().filter(|f| f.len() >= 3) {
        let fn_ = crate::geo::edit::face_normal(pos, f);
        for v in f {
            n[*v as usize] += fn_;
        }
    }
    n.into_iter().map(|v| if v.length_sq() > 1e-12 { v.normalized() } else { Vec3::UP }).collect()
}

fn neighbours(n: usize, faces: &[Vec<u32>]) -> Vec<Vec<u32>> {
    let mut nb: Vec<Vec<u32>> = vec![Vec::new(); n];
    for f in faces {
        for k in 0..f.len() {
            let (a, b) = (f[k], f[(k + 1) % f.len()]);
            if a != b {
                if !nb[a as usize].contains(&b) {
                    nb[a as usize].push(b);
                }
                if !nb[b as usize].contains(&a) {
                    nb[b as usize].push(a);
                }
            }
        }
    }
    nb
}

impl SculptState {
    /// Start a stroke on a shape: remember what Erase returns to, and the reference surface the strokes displace from.
    pub fn begin_stroke(&mut self, s: &Session, shape: NodeId) -> Result<(), String> {
        let (poly, _) = s.scene.shape_poly(shape).ok_or("Select a polygon or NURBS surface to sculpt")?;
        self.erase_refs.entry(shape).or_insert_with(|| poly.pos.clone());
        let stale = self.refs.get(&shape).map(|r| r.len() != poly.pos.len()).unwrap_or(true);
        if self.update_ref || stale {
            self.refs.insert(shape, poly.pos.clone());
            self.ref_normals.insert(shape, vertex_normals(&poly.pos, &poly.faces));
        }
        Ok(())
    }

    /// The Update buttons of Sculpt Parameters.
    pub fn update_reference(&mut self, s: &Session, shape: NodeId) {
        if let Some((poly, _)) = s.scene.shape_poly(shape) {
            self.refs.insert(shape, poly.pos.clone());
            self.ref_normals.insert(shape, vertex_normals(&poly.pos, &poly.faces));
        }
    }

    pub fn update_erase_surface(&mut self, s: &Session, shape: NodeId) {
        if let Some((poly, _)) = s.scene.shape_poly(shape) {
            self.erase_refs.insert(shape, poly.pos);
        }
    }

    /// One dab of the brush at a world position (`flood` applies the operation everywhere at full falloff).
    pub fn dab(&mut self, s: &mut Session, shape: NodeId, world: Vec3, flood: bool) -> Result<usize, String> {
        self.begin_stroke_if_needed(s, shape)?;
        let (poly, owner) = s.scene.shape_poly(shape).ok_or("Select a polygon or NURBS surface to sculpt")?;
        let wm = owner.map(|o| s.scene.world_matrix(o)).unwrap_or_else(crate::math::Mat4::identity);
        let center = wm.inverse().transform_point(world);
        // the radius in local units (the object's scale)
        let scale = wm.transform_dir(Vec3::new(1.0, 0.0, 0.0)).length().max(1e-6);
        let radius = self.radius / scale;
        let max = self.max_disp / scale;
        let n = poly.pos.len();
        let refs = self.refs.get(&shape).cloned().unwrap_or_else(|| poly.pos.clone());
        let rn = self.ref_normals.get(&shape).cloned().unwrap_or_else(|| vertex_normals(&poly.pos, &poly.faces));
        let erase = self.erase_refs.get(&shape).cloned().unwrap_or_else(|| poly.pos.clone());
        let mut new_pos = poly.pos.clone();
        let nb = neighbours(n, &poly.faces);
        let mut touched = 0;
        let weight = |d: f32| -> f32 {
            if flood {
                1.0
            } else if d >= radius {
                0.0
            } else {
                let t = 1.0 - (d / radius) * (d / radius);
                t * t
            }
        };
        for i in 0..n {
            let w = weight((poly.pos[i] - center).length()) * self.opacity;
            if w <= 1e-5 {
                continue;
            }
            touched += 1;
            match self.op.as_str() {
                "push" | "pull" => {
                    let sign = if self.op == "push" { -1.0 } else { 1.0 };
                    // displacement along the reference normal, kept inside +-max of the reference surface
                    let off = (poly.pos[i] - refs[i]).dot(rn[i]);
                    let target = (off + sign * max * 0.3 * w).clamp(-max, max);
                    new_pos[i] = poly.pos[i] + rn[i] * (target - off);
                }
                "smooth" | "relax" => {
                    if nb[i].is_empty() {
                        continue;
                    }
                    let avg = nb[i].iter().fold(Vec3::ZERO, |a, k| a + poly.pos[*k as usize]) * (1.0 / nb[i].len() as f32);
                    let mut d = (avg - poly.pos[i]) * (w * 0.5 * self.smooth_strength as f32).min(1.0);
                    if self.op == "relax" {
                        // relax slides the points along the surface only
                        d = d - rn[i] * d.dot(rn[i]);
                    }
                    new_pos[i] = poly.pos[i] + d;
                }
                _ => {
                    new_pos[i] = poly.pos[i].lerp(erase[i], (w * if flood { 1.0 } else { 0.5 }).min(1.0));
                }
            }
        }
        let edits: Vec<(u32, Vec3)> = (0..n).filter(|i| (new_pos[*i] - poly.pos[*i]).length_sq() > 1e-14).map(|i| (i as u32, new_pos[i])).collect();
        if !edits.is_empty() {
            s.scene.tweak_vertices(shape, &edits)?;
        }
        Ok(touched)
    }

    fn begin_stroke_if_needed(&mut self, s: &Session, shape: NodeId) -> Result<(), String> {
        if !self.refs.contains_key(&shape) {
            self.begin_stroke(s, shape)?;
        }
        Ok(())
    }
}

const SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["operation", "op"], arity: 1 },
    FlagSpec { names: &["radius", "r"], arity: 1 },
    FlagSpec { names: &["opacity", "o"], arity: 1 },
    FlagSpec { names: &["maxDisplacement", "md"], arity: 1 },
    FlagSpec { names: &["smoothStrength", "ss"], arity: 1 },
    FlagSpec { names: &["updateReferenceEachStroke", "ur"], arity: 1 },
    FlagSpec { names: &["stroke"], arity: 1 },
    FlagSpec { names: &["flood"], arity: 0 },
    FlagSpec { names: &["updateReference"], arity: 0 },
    FlagSpec { names: &["updateEraseSurface"], arity: 0 },
    FlagSpec { names: &["reset"], arity: 0 },
    FlagSpec { names: &["query", "q"], arity: 0 },
];

/// `sculpt [-op push|pull|smooth|relax|erase] [-r RADIUS] [-o OPACITY] [-md MAXDISP] [-ss 1..5] [-ur 0|1] [-stroke "x y z x y z ..."] [-flood]
/// [-updateReference] [-updateEraseSurface] [-reset] [-q] [object]`: the Sculpt Geometry Tool. A stroke is a list of world positions; the points
/// within the brush radius of each are displaced along the reference surface's normal (push / pull, limited to the maximum displacement),
/// smoothed, relaxed, or returned toward the surface as it was before sculpting (erase).
pub fn sculpt(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SPEC)?;
    let mut st = std::mem::take(&mut s.sculpt);
    let result = (|| -> Out {
        if p.has("reset") {
            let keep = (st.refs.clone(), st.ref_normals.clone(), st.erase_refs.clone());
            st = SculptState { refs: keep.0, ref_normals: keep.1, erase_refs: keep.2, ..Default::default() };
        }
        if let Some(op) = p.string("operation") {
            if !OPS.contains(&op.as_str()) {
                return Err(format!("Unknown operation '{}': {}", op, OPS.join(", ")));
            }
            st.op = op;
        }
        if let Some(v) = p.num("radius")? {
            if v <= 0.0 {
                return Err("The brush radius must be above zero".to_string());
            }
            st.radius = v;
        }
        if let Some(v) = p.num("opacity")? {
            st.opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = p.num("maxDisplacement")? {
            st.max_disp = v.max(0.0);
        }
        if let Some(v) = p.num("smoothStrength")? {
            st.smooth_strength = v.clamp(1.0, 5.0) as u32;
        }
        if let Some(v) = p.num("updateReferenceEachStroke")? {
            st.update_ref = v > 0.5;
        }
        let needs_shape = p.has("flood") || p.string("stroke").is_some() || p.has("updateReference") || p.has("updateEraseSurface");
        if p.has("query") || !needs_shape {
            return Ok(vec![format!("{}: radius {}, opacity {}, max displacement {}, smooth strength {}, reference {}", st.op, st.radius, st.opacity, st.max_disp, st.smooth_strength, if st.update_ref { "updates each stroke" } else { "fixed until Update" })]);
        }
        let ids = resolve_targets(s, &p.arg_strings())?;
        let shape = ids.iter().find_map(|i| {
            let sc = &s.scene;
            if sc.shape_poly(*i).is_some() && matches!(sc.node(*i).map(|n| &n.data), Some(crate::dag::nodes::NodeData::Mesh(_))) {
                Some(*i)
            } else {
                sc.mesh_shapes_under(*i).first().copied()
            }
        });
        let shape = shape.ok_or("Select a polygon or NURBS surface to sculpt")?;
        if p.has("updateReference") {
            st.update_reference(s, shape);
        }
        if p.has("updateEraseSurface") {
            st.update_erase_surface(s, shape);
        }
        if p.has("flood") {
            st.begin_stroke(s, shape)?;
            let n = st.dab(s, shape, Vec3::ZERO, true)?;
            s.scene.flush_meshes();
            return Ok(vec![format!("Flooded {} point(s)", n)]);
        }
        if let Some(text) = p.string("stroke") {
            let v: Vec<f32> = text.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() < 3 || v.len() % 3 != 0 {
                return Err("-stroke needs groups of three numbers".to_string());
            }
            st.begin_stroke(s, shape)?;
            let mut touched = 0;
            for c in v.chunks_exact(3) {
                touched += st.dab(s, shape, Vec3::new(c[0], c[1], c[2]), false)?;
            }
            s.scene.flush_meshes();
            return Ok(vec![format!("Stroke of {} dab(s) moved {} point(s)", v.len() / 3, touched)]);
        }
        Ok(vec![])
    })();
    s.sculpt = st;
    result
}
