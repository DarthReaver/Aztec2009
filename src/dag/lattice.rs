//! Lattice deformers (free form deformation) and their rigid binding to joints.

use super::graph::{NodeId, Scene};
use super::nodes::{LatticeData, LatticeSkin, NodeData, PolyKind};
use crate::geo::edit::Poly;
use crate::math::{Mat4, Vec3};

fn bernstein(n: usize, i: usize, t: f32) -> f32 {
    // C(n, i) t^i (1-t)^(n-i)
    let mut c = 1.0f32;
    for k in 0..i {
        c = c * (n - k) as f32 / (k + 1) as f32;
    }
    c * t.powi(i as i32) * (1.0 - t).powi((n - i) as i32)
}

impl Scene {
    pub fn lattice_shapes_under(&self, id: NodeId) -> Vec<NodeId> {
        self.descendants(id).into_iter().filter(|n| matches!(self.nodes.get(*n).map(|x| &x.data), Some(NodeData::Lattice(_)))).collect()
    }

    /// Current control points of a lattice (local cube space): rest + offsets, moved by the joints it is bound to.
    pub fn lattice_points(&mut self, lat: NodeId) -> Vec<Vec3> {
        let ld = match self.nodes.get(lat).map(|n| &n.data) {
            Some(NodeData::Lattice(d)) => (**d).clone(),
            _ => return Vec::new(),
        };
        let mut pts: Vec<Vec3> = (0..ld.count())
            .map(|idx| {
                let (i, j, k) = ld.coords(idx);
                let g = |s: &mut Scene, a: &str| s.get_attr(lat, &format!("pt_{}_{}_{}_{}", i, j, k, a)).unwrap_or(0.0);
                ld.rest(idx) + Vec3::new(g(self, "x"), g(self, "y"), g(self, "z"))
            })
            .collect();
        self.apply_flexor(lat, &mut pts);
        if let Some(sk) = &ld.skin {
            let parent = self.nodes.get(lat).and_then(|n| n.parent);
            let lw = match parent {
                Some(p) => self.world_matrix(p),
                None => Mat4::identity(),
            };
            let lw_inv = lw.inverse();
            let jm: Vec<Mat4> = sk.joints.iter().enumerate().map(|(i, j)| if self.nodes.get(*j).is_some() { self.world_matrix(*j) * sk.bind_inv[i] } else { Mat4::identity() }).collect();
            for (idx, p) in pts.iter_mut().enumerate() {
                let bind_world = sk.bind_world.transform_point(*p);
                let mut acc = Vec3::ZERO;
                let mut ws = 0.0;
                for (j, w) in sk.weights.get(idx).map(|x| x.as_slice()).unwrap_or(&[]) {
                    if let Some(m) = jm.get(*j as usize) {
                        acc += m.transform_point(bind_world) * *w;
                        ws += *w;
                    }
                }
                let world = if ws > 1e-6 { acc * (1.0 / ws) } else { bind_world };
                *p = lw_inv.transform_point(world);
            }
        }
        pts
    }

    /// The joint a flexor lattice rides on, when `lat` is one (its transform sits under a joint and has the flexor channels).
    pub fn flexor_joint(&self, lat: NodeId) -> Option<NodeId> {
        let t = self.nodes.get(lat)?.parent?;
        let j = self.nodes.get(t)?.parent?;
        if self.is_joint(j) && self.nodes.get(t)?.plug("flexorRounding").is_some() {
            Some(j)
        } else {
            None
        }
    }

    /// Flexor behavior. The lattice rides the skeleton like a bound lattice: the points before the joint follow the parent
    /// bone, the points after it follow the joint, blended over a short stretch around the joint, so the box bends with the
    /// geometry. On top of that the points swell away from the bone as the joint bends, which keeps the volume instead of
    /// letting it collapse at the crease. `flexorRounding` scales the swell, `flexorLength` the stretch along the bones,
    /// `flexorCreasing` pinches it instead. The points stay editable and drivable (`pt_i_j_k_x/y/z`) on top of this.
    fn apply_flexor(&mut self, lat: NodeId, pts: &mut [Vec3]) {
        let Some(joint) = self.flexor_joint(lat) else { return };
        let t = self.nodes.get(lat).and_then(|n| n.parent).unwrap();
        let g = |s: &mut Scene, a: &str| s.get_attr(t, a).unwrap_or(0.0);
        let (rounding, length, creasing) = (g(self, "flexorRounding"), g(self, "flexorLength").max(0.05), g(self, "flexorCreasing"));
        let parent = self.nodes.get(joint).and_then(|n| n.parent).filter(|p| self.is_joint(*p));
        let child = self.nodes.get(joint).map(|n| n.children.clone()).unwrap_or_default().into_iter().find(|c| self.is_joint(*c));
        // bind inverses from the skin that holds this joint
        let mut bind: Option<(Vec<NodeId>, Vec<Mat4>)> = None;
        for op in self.skins.clone() {
            if let Some(NodeData::PolyOp(o)) = self.nodes.get(op).map(|n| &n.data) {
                if let Some(sk) = &o.skin {
                    if sk.joints.contains(&joint) {
                        bind = Some((sk.joints.clone(), sk.bind_inv.clone()));
                        break;
                    }
                }
            }
        }
        let Some((joints, binv)) = bind else { return };
        let inv_of = |n: NodeId| joints.iter().position(|j| *j == n).and_then(|i| binv.get(i).copied());
        let Some(bj_inv) = inv_of(joint) else { return };
        let bind_j = bj_inv.inverse();
        let bind_p = parent.and_then(inv_of).map(|m| m.inverse());
        let bind_c = child.and_then(inv_of).map(|m| m.inverse());
        let jm = |s: &mut Scene, n: NodeId, inv: &Mat4| s.world_matrix(n) * *inv;
        let jm_j = jm(self, joint, &bj_inv);
        let jm_p = match (parent, bind_p) {
            (Some(p), Some(bp)) => jm(self, p, &bp.inverse()),
            _ => jm_j,
        };
        let jp_b = bind_j.translation_part();
        let tip_b = bind_c.map(|m| m.translation_part()).unwrap_or(jp_b + Vec3::new(0.0, 1.0, 0.0));
        let back_b = bind_p.map(|m| m.translation_part()).unwrap_or(jp_b - (tip_b - jp_b));
        let axis_b = (tip_b - back_b).normalized();
        let reach = ((tip_b - jp_b).length().max((jp_b - back_b).length())).max(1e-3) * length;
        // how far the joint is bent now, against the bind pose
        let dir_now = |a: Vec3, b: Vec3| (b - a).normalized();
        let jp_n = self.world_matrix(joint).translation_part();
        let tip_n = child.map(|c| self.world_matrix(c).translation_part()).unwrap_or(jm_j.transform_point(tip_b));
        let back_n = parent.map(|p| self.world_matrix(p).translation_part()).unwrap_or(jm_j.transform_point(back_b));
        let (d_out, d_in) = (dir_now(jp_n, tip_n), dir_now(back_n, jp_n));
        let (b_out, b_in) = (dir_now(jp_b, tip_b), dir_now(back_b, jp_b));
        let bend = (d_in.dot(d_out).clamp(-1.0, 1.0).acos() - b_in.dot(b_out).clamp(-1.0, 1.0).acos()).abs();
        let k = (bend / std::f32::consts::PI).min(1.0);
        let local = self.local_matrix(t);
        let bind_lw = bind_j * local;
        let lw_inv = self.world_matrix(t).inverse();
        for p in pts.iter_mut() {
            let r = bind_lw.transform_point(*p);
            let d = (r - jp_b).dot(axis_b);
            let x = (d / (0.5 * reach) * 0.5 + 0.5).clamp(0.0, 1.0);
            let bw = x * x * (3.0 - 2.0 * x);
            let mut w = jm_p.transform_point(r) * (1.0 - bw) + jm_j.transform_point(r) * bw;
            if k > 1e-4 && (rounding != 0.0 || creasing != 0.0) {
                // swell about the bent bone axis, strongest at the joint
                let axis = (d_in * (1.0 - bw) + d_out * bw).normalized();
                let rel = w - jp_n;
                let perp = rel - axis * rel.dot(axis);
                let fall = (1.0 - d.abs() / (0.5 * reach)).max(0.0);
                w = w + perp * ((rounding - creasing) * k * fall * 0.6);
            }
            *p = lw_inv.transform_point(w);
        }
    }

    /// Skin > Edit Rigid Skin > Create Flexor: a lattice on the mesh bound to `joint`, under the joint, covering the bending region.
    /// With `position` it is centered on the joint and sized to the bones next to it; otherwise it is fitted to the whole mesh.
    pub fn create_flexor(&mut self, joint: NodeId, divs: [usize; 3], position: bool) -> Result<NodeId, String> {
        if !self.is_joint(joint) {
            return Err("Select a joint".to_string());
        }
        let shape = self
            .ids_where(|d| matches!(d, NodeData::Mesh(_)))
            .into_iter()
            .find(|sh| {
                self.skin_of(*sh)
                    .and_then(|op| match self.nodes.get(op).map(|n| &n.data) {
                        Some(NodeData::PolyOp(o)) => o.skin.as_ref().map(|s| s.joints.iter().any(|j| { let mut c = Some(*j); while let Some(x) = c { if x == joint { return true; } c = self.nodes.get(x).and_then(|n| n.parent); } false })),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .ok_or("No mesh is bound to this joint's skeleton: bind the skin first")?;
        let t = self.create_lattice(shape, divs, Some("flexor"))?;
        let jw = self.world_matrix(joint);
        let jp = jw.translation_part();
        if position {
            // length along the dominant axis of the neighboring bones, the width of the mesh across
            let mut reach = 0.0f32;
            let mut dir = Vec3::new(0.0, 1.0, 0.0);
            let mut nbrs: Vec<NodeId> = self.nodes.get(joint).map(|n| n.children.clone()).unwrap_or_default();
            nbrs.extend(self.nodes.get(joint).and_then(|n| n.parent));
            let nbrs: Vec<NodeId> = nbrs.into_iter().filter(|n| self.is_joint(*n)).collect();
            for n in nbrs {
                let d = self.world_matrix(n).translation_part() - jp;
                if d.length() > reach {
                    reach = d.length();
                    dir = d;
                }
            }
            let ax = if dir.x.abs() >= dir.y.abs() && dir.x.abs() >= dir.z.abs() { "scaleX" } else if dir.y.abs() >= dir.z.abs() { "scaleY" } else { "scaleZ" };
            self.set_attr(t, "translateX", jp.x)?;
            self.set_attr(t, "translateY", jp.y)?;
            self.set_attr(t, "translateZ", jp.z)?;
            if reach > 1e-4 {
                self.set_attr(t, ax, reach * 1.2)?;
            }
        }
        self.set_parent_keep_world(t, Some(joint))?;
        self.add_attr_ex(t, "flexorRounding", 1.0, Some(0.0), Some(5.0), true)?;
        self.add_attr_ex(t, "flexorLength", 1.0, Some(0.05), Some(4.0), true)?;
        self.add_attr_ex(t, "flexorCreasing", 0.0, Some(0.0), Some(5.0), true)?;
        self.mark_skins_dirty();
        self.flush_meshes();
        Ok(t)
    }

    /// Deform `poly` (mesh local space) with the lattice.
    pub fn eval_lattice(&mut self, shape: NodeId, lat: NodeId, poly: &mut Poly) {
        let divs = match self.nodes.get(lat).map(|n| &n.data) {
            Some(NodeData::Lattice(d)) => d.divs,
            _ => return,
        };
        let ld = LatticeData { divs, skin: None };
        let pts = self.lattice_points(lat);
        if pts.len() != ld.count() {
            return;
        }
        let mesh_world = match self.nodes.get(shape).and_then(|n| n.parent) {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        };
        let lat_world = match self.nodes.get(lat).and_then(|n| n.parent) {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        };
        let (to_lat, from_lat, from_mesh) = (lat_world.inverse(), lat_world, mesh_world.inverse());
        let (n0, n1, n2) = (divs[0] - 1, divs[1] - 1, divs[2] - 1);
        for p in poly.pos.iter_mut() {
            let l = to_lat.transform_point(mesh_world.transform_point(*p));
            let (s, t, u) = (l.x + 0.5, l.y + 0.5, l.z + 0.5);
            // points on the lattice boundary stay inside whatever the rounding of the last digits says
            const EPS: f32 = 1e-3;
            if !(-EPS..=1.0 + EPS).contains(&s) || !(-EPS..=1.0 + EPS).contains(&t) || !(-EPS..=1.0 + EPS).contains(&u) {
                continue;
            }
            let (s, t, u) = (s.clamp(0.0, 1.0), t.clamp(0.0, 1.0), u.clamp(0.0, 1.0));
            let mut out = Vec3::ZERO;
            for i in 0..=n0 {
                let bi = bernstein(n0, i, s);
                for j in 0..=n1 {
                    let bj = bernstein(n1, j, t);
                    for k in 0..=n2 {
                        out += pts[ld.index(i, j, k)] * (bi * bj * bernstein(n2, k, u));
                    }
                }
            }
            *p = from_mesh.transform_point(from_lat.transform_point(out));
        }
    }

    /// Create > Deformers > Lattice: a lattice fitted to the mesh, deforming it through a history node.
    pub fn create_lattice(&mut self, shape: NodeId, divs: [usize; 3], name: Option<&str>) -> Result<NodeId, String> {
        if divs.iter().any(|d| *d < 2 || *d > 12) || divs[0] * divs[1] * divs[2] > 500 {
            return Err("A lattice needs 2 to 12 divisions per axis and at most 500 points".to_string());
        }
        self.flush_meshes();
        let (poly, parent) = match self.nodes.get(shape) {
            Some(n) => match &n.data {
                NodeData::Mesh(md) => (md.poly(), n.parent),
                _ => return Err("Not a mesh".to_string()),
            },
            None => return Err("Invalid node".to_string()),
        };
        if poly.pos.is_empty() {
            return Err("The mesh has no points".to_string());
        }
        let mw = match parent {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        };
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for p in &poly.pos {
            let w = mw.transform_point(*p);
            lo = Vec3::new(lo.x.min(w.x), lo.y.min(w.y), lo.z.min(w.z));
            hi = Vec3::new(hi.x.max(w.x), hi.y.max(w.y), hi.z.max(w.z));
        }
        let size = Vec3::new((hi.x - lo.x).max(1e-3), (hi.y - lo.y).max(1e-3), (hi.z - lo.z).max(1e-3));
        let center = (lo + hi) * 0.5;
        let (tname, force) = match name {
            Some(n) => (n, false),
            None => ("ffd1Lattice", true),
        };
        let t = self.create_node(NodeData::Transform, tname, None, force);
        let sname = format!("{}Shape", self.name_of(t));
        let lat = self.create_node(NodeData::Lattice(Box::new(LatticeData { divs, skin: None })), &sname, Some(t), false);
        for (a, v) in [("translateX", center.x), ("translateY", center.y), ("translateZ", center.z), ("scaleX", size.x), ("scaleY", size.y), ("scaleZ", size.z)] {
            self.set_attr(t, a, v)?;
        }
        let op = self.append_history(shape, PolyKind::Lattice, Vec::new(), &[])?;
        if let Some(NodeData::PolyOp(o)) = self.nodes.get_mut(op).map(|n| &mut n.data) {
            o.lattice = Some(lat);
        }
        self.skins.push(op);
        self.mark_poly_dirty(op);
        self.flush_meshes();
        Ok(t)
    }

    /// Rigid bind of a lattice to joints: every lattice point follows its nearest bone.
    pub fn bind_lattice(&mut self, lat: NodeId, joints: &[NodeId]) -> Result<(), String> {
        if joints.is_empty() {
            return Err("Select a joint (the root of the skeleton) as well".to_string());
        }
        if matches!(self.nodes.get(lat).map(|n| &n.data), Some(NodeData::Lattice(d)) if d.skin.is_some()) {
            return Err("The lattice is already bound".to_string());
        }
        let ld = match self.nodes.get(lat).map(|n| &n.data) {
            Some(NodeData::Lattice(d)) => (**d).clone(),
            _ => return Err("Not a lattice".to_string()),
        };
        let lw = match self.nodes.get(lat).and_then(|n| n.parent) {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        };
        let jw: Vec<Mat4> = joints.iter().map(|j| self.world_matrix(*j)).collect();
        let pw: Vec<Vec3> = (0..ld.count()).map(|i| lw.transform_point(ld.rest(i))).collect();
        let parents: std::collections::HashMap<NodeId, Option<NodeId>> = joints.iter().map(|j| (*j, self.nodes.get(*j).and_then(|n| n.parent))).collect();
        let weights = Scene::default_weights(joints, &|j| parents.get(&j).copied().flatten(), &jw, &pw, true, 1, 4.0);
        let skin = LatticeSkin { joints: joints.to_vec(), joint_names: Vec::new(), bind_inv: jw.iter().map(|m| m.inverse()).collect(), bind_world: lw, weights };
        if let Some(NodeData::Lattice(d)) = self.nodes.get_mut(lat).map(|n| &mut n.data) {
            d.skin = Some(skin);
        }
        self.mark_skins_dirty();
        self.flush_meshes();
        Ok(())
    }
}
