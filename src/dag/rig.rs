//! Skeletons: joint chains, smooth skin binding and a CCD IK solver.

use super::graph::{NodeId, Scene};
use super::nodes::{NodeData, PolyKind, SkinData};
use crate::geo::edit::Poly;
use crate::math::{Mat4, Quat, Vec3};
use std::collections::HashMap;

fn seg_dist(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    if l2 < 1e-12 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// World matrix whose columns are the given axes (rotation only).
fn rot_basis(x: Vec3, y: Vec3, z: Vec3) -> Mat4 {
    let mut m = Mat4::identity();
    for (c, v) in [x, y, z].iter().enumerate() {
        m.m[c][0] = v.x;
        m.m[c][1] = v.y;
        m.m[c][2] = v.z;
    }
    m
}

fn perp_unit(v: Vec3, along: Vec3) -> Vec3 {
    let p = v - along * along.dot(v);
    if p.length_sq() > 1e-10 {
        p.normalized()
    } else {
        let alt = if along.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
        (alt - along * along.dot(alt)).normalized()
    }
}

impl Scene {
    /// The rotation part of a node's world matrix (no scale, no translation).
    pub fn world_rotation(&mut self, id: NodeId) -> Mat4 {
        let m = self.world_matrix(id);
        let (_, r, _) = m.decompose();
        Mat4::from_quat(Quat::from_euler_deg(r))
    }

    /// Skeleton > Orient Joint: aim the `order[0]` axis of every joint in the hierarchy at its first child with the
    /// `order[1]` axis toward `up` (a world direction); joints without a child take their parent's orientation.
    /// Rotations go to zero, the orientation to jointOrient, and the joints stay where they are.
    pub fn orient_joints(&mut self, root: NodeId, order: [usize; 3], up: Option<Vec3>) -> Result<usize, String> {
        let joints = self.joints_under(&[root]);
        let pos: Vec<(NodeId, Vec3)> = joints.iter().map(|j| (*j, self.world_matrix(*j).translation_part())).collect();
        let at = |id: NodeId| pos.iter().find(|(j, _)| *j == id).map(|(_, p)| *p).unwrap_or(Vec3::ZERO);
        for &j in &joints {
            let parent = self.nodes.get(j).and_then(|n| n.parent);
            let pw = match parent {
                Some(p) => self.world_matrix(p),
                None => Mat4::identity(),
            };
            let prot = match parent {
                Some(p) => self.world_rotation(p),
                None => Mat4::identity(),
            };
            let kid = self.nodes.get(j).map(|n| n.children.clone()).unwrap_or_default().into_iter().find(|c| self.is_joint(*c));
            let world_rot = match kid {
                Some(c) => {
                    let d = (at(c) - at(j)).normalized();
                    if d.length_sq() < 1e-12 {
                        prot
                    } else {
                        let sec = perp_unit(up.unwrap_or(Vec3::UP), d);
                        let (p, s, tt) = (order[0], order[1], order[2]);
                        let tert = if s == (p + 1) % 3 { d.cross(sec) } else { sec.cross(d) };
                        let mut cols = [Vec3::ZERO; 3];
                        cols[p] = d;
                        cols[s] = sec;
                        cols[tt] = tert;
                        rot_basis(cols[0], cols[1], cols[2])
                    }
                }
                None => prot,
            };
            let local = prot.inverse() * world_rot;
            let (_, jo, _) = local.decompose();
            for (a, v) in ["X", "Y", "Z"].iter().zip([jo.x, jo.y, jo.z]) {
                self.set_attr(j, &format!("jointOrient{}", a), v)?;
                self.set_attr(j, &format!("rotate{}", a), 0.0)?;
            }
            // keep the world position: the translation is relative to the parent's (possibly new) frame
            let local_pos = pw.inverse().transform_point(at(j));
            if parent.is_some() {
                for (a, v) in ["X", "Y", "Z"].iter().zip([local_pos.x, local_pos.y, local_pos.z]) {
                    self.set_attr(j, &format!("translate{}", a), v)?;
                }
            }
        }
        self.mark_skins_dirty();
        Ok(joints.len())
    }

    pub fn is_joint(&self, id: NodeId) -> bool {
        matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::Joint))
    }

    /// All joints in the hierarchies of `roots` (the nodes themselves included), parents first.
    pub fn joints_under(&self, roots: &[NodeId]) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = Vec::new();
        for r in roots {
            for d in self.descendants(*r) {
                if self.is_joint(d) && !out.contains(&d) {
                    out.push(d);
                }
            }
        }
        out
    }

    pub fn mark_skins_dirty(&mut self) {
        let skins: Vec<NodeId> = self.skins.iter().copied().filter(|s| self.nodes.get(*s).is_some()).collect();
        self.skins = skins.clone();
        for s in skins {
            self.mark_poly_dirty(s);
        }
    }

    /// Distance based default weights: every vertex follows its nearest bone (rigid) or up to `max_inf` bones with a
    /// falloff set by `dropoff` (higher: tighter). `joint_world` are the joint matrices of the bind pose.
    pub fn default_weights(joints: &[NodeId], parent_of: &dyn Fn(NodeId) -> Option<NodeId>, joint_world: &[Mat4], points_world: &[Vec3], rigid: bool, max_inf: usize, dropoff: f32) -> Vec<Vec<(u16, f32)>> {
        let jp: Vec<Vec3> = joint_world.iter().map(|m| m.translation_part()).collect();
        // bone segments: each joint to its child joints (or the point itself)
        let mut segs: Vec<Vec<(Vec3, Vec3)>> = vec![Vec::new(); joints.len()];
        for (i, j) in joints.iter().enumerate() {
            let kids: Vec<usize> = joints.iter().enumerate().filter(|(_, c)| parent_of(**c) == Some(*j)).map(|(k, _)| k).collect();
            if kids.is_empty() {
                segs[i].push((jp[i], jp[i]));
            }
            for k in kids {
                segs[i].push((jp[i], jp[k]));
            }
        }
        let expo = (dropoff.max(0.5)) * 0.5;
        points_world
            .iter()
            .map(|w| {
                let mut d: Vec<(usize, f32)> = (0..joints.len()).map(|i| (i, segs[i].iter().map(|(a, b)| seg_dist(*w, *a, *b)).fold(f32::INFINITY, f32::min))).collect();
                d.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                if rigid {
                    return d.first().map(|(i, _)| vec![(*i as u16, 1.0)]).unwrap_or_default();
                }
                d.truncate(max_inf.clamp(1, 8));
                let mut ws: Vec<(u16, f32)> = d.iter().map(|(i, dist)| (*i as u16, 1.0 / (dist * dist + 1e-3).powf(expo))).collect();
                let sum: f32 = ws.iter().map(|x| x.1).sum();
                for x in ws.iter_mut() {
                    x.1 /= sum.max(1e-12);
                }
                ws.retain(|x| x.1 > 0.005);
                let sum2: f32 = ws.iter().map(|x| x.1).sum();
                for x in ws.iter_mut() {
                    x.1 /= sum2.max(1e-12);
                }
                ws
            })
            .collect()
    }

    /// Smooth bind `shape` to `joints` with distance based weights (max four influences).
    pub fn bind_skin(&mut self, shape: NodeId, joints: &[NodeId]) -> Result<NodeId, String> {
        self.bind_skin_ex(shape, joints, false, 4, 4.0)
    }

    /// Bind with options: `rigid` gives every vertex one joint, otherwise up to `max_inf` joints with falloff `dropoff`.
    pub fn bind_skin_ex(&mut self, shape: NodeId, joints: &[NodeId], rigid: bool, max_inf: usize, dropoff: f32) -> Result<NodeId, String> {
        if joints.is_empty() {
            return Err("Select a joint (or the root of a skeleton) as well as the mesh".to_string());
        }
        self.flush_meshes();
        if let Some(last) = self.history_of(shape).last() {
            if matches!(self.nodes.get(*last).map(|n| &n.data), Some(NodeData::PolyOp(op)) if op.kind == PolyKind::Skin) {
                return Err("The mesh is already bound; detach the skin first".to_string());
            }
        }
        let (poly, parent) = match self.nodes.get(shape) {
            Some(n) => match &n.data {
                NodeData::Mesh(md) => (md.poly(), n.parent),
                _ => return Err("Not a mesh".to_string()),
            },
            None => return Err("Invalid node".to_string()),
        };
        let mesh_world = match parent {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        };
        let jw: Vec<Mat4> = joints.iter().map(|j| self.world_matrix(*j)).collect();
        let pw: Vec<Vec3> = poly.pos.iter().map(|p| mesh_world.transform_point(*p)).collect();
        let parents: HashMap<NodeId, Option<NodeId>> = joints.iter().map(|j| (*j, self.nodes.get(*j).and_then(|n| n.parent))).collect();
        let weights = Scene::default_weights(joints, &|j| parents.get(&j).copied().flatten(), &jw, &pw, rigid, max_inf, dropoff);
        let skin = SkinData {
            joints: joints.to_vec(),
            joint_names: Vec::new(),
            bind_inv: jw.iter().map(|m| m.inverse()).collect(),
            bind_mesh_world: mesh_world,
            weights,
            max_inf: max_inf as u8,
            dropoff,
            rigid,
        };
        let id = self.append_history(shape, PolyKind::Skin, Vec::new(), &[])?;
        if let Some(NodeData::PolyOp(op)) = self.nodes.get_mut(id).map(|n| &mut n.data) {
            op.skin = Some(skin);
        }
        self.skins.push(id);
        self.mark_poly_dirty(id);
        self.flush_meshes();
        Ok(id)
    }

    pub fn detach_skin(&mut self, shape: NodeId) -> Result<(), String> {
        let hist = self.history_of(shape);
        let skin = hist
            .iter()
            .copied()
            .find(|h| matches!(self.nodes.get(*h).map(|n| &n.data), Some(NodeData::PolyOp(op)) if op.kind == PolyKind::Skin))
            .ok_or("The mesh is not bound to a skeleton")?;
        self.skins.retain(|s| *s != skin);
        self.delete_history_node(shape, skin)
    }

    pub fn skin_of(&self, shape: NodeId) -> Option<NodeId> {
        self.history_of(shape)
            .into_iter()
            .find(|h| matches!(self.nodes.get(*h).map(|n| &n.data), Some(NodeData::PolyOp(op)) if op.kind == PolyKind::Skin))
    }

    /// Deform `poly` (bind pose geometry) with the current joint transforms.
    pub fn eval_skin(&mut self, shape: NodeId, skin: &SkinData, poly: &mut Poly) {
        if skin.weights.len() != poly.pos.len() || skin.bind_inv.len() != skin.joints.len() {
            return;
        }
        let jm: Vec<Mat4> = skin
            .joints
            .iter()
            .enumerate()
            .map(|(i, j)| {
                if self.nodes.get(*j).is_some() {
                    self.world_matrix(*j) * skin.bind_inv[i]
                } else {
                    Mat4::identity()
                }
            })
            .collect();
        let parent = self.nodes.get(shape).and_then(|n| n.parent);
        let mesh_inv = match parent {
            Some(p) => self.world_matrix(p).inverse(),
            None => Mat4::identity(),
        };
        for (v, p) in poly.pos.iter_mut().enumerate() {
            let pw = skin.bind_mesh_world.transform_point(*p);
            let mut acc = Vec3::ZERO;
            let mut wsum = 0.0;
            for (ji, w) in &skin.weights[v] {
                if let Some(m) = jm.get(*ji as usize) {
                    acc += m.transform_point(pw) * *w;
                    wsum += *w;
                }
            }
            let world = if wsum > 1e-6 { acc * (1.0 / wsum) } else { pw };
            *p = mesh_inv.transform_point(world);
        }
    }

    /// Rotate a joint by `delta` in world space (its children follow); writes the evaluated rotation.
    fn rotate_joint_world(&mut self, j: NodeId, delta: Quat) {
        let parent = self.nodes.get(j).and_then(|n| n.parent);
        let pq = match parent {
            Some(p) => {
                let (_, r, _) = self.world_matrix(p).decompose();
                Quat::from_euler_deg(r)
            }
            None => Quat::identity(),
        };
        let g = |s: &mut Scene, a: &str| s.get_attr(j, a).unwrap_or(0.0);
        let jo = if self.nodes.get(j).map(|n| n.plug("jointOrientX").is_some()).unwrap_or(false) { Quat::from_euler_deg(Vec3::new(g(self, "jointOrientX"), g(self, "jointOrientY"), g(self, "jointOrientZ"))) } else { Quat::identity() };
        let r_local = Quat::from_euler_deg(Vec3::new(g(self, "rotateX"), g(self, "rotateY"), g(self, "rotateZ")));
        let new_local = jo.conjugate() * pq.conjugate() * delta * pq * jo * r_local;
        let (_, e, _) = Mat4::from_quat(new_local.normalized()).decompose();
        self.set_eval(j, "rotateX", e.x);
        self.set_eval(j, "rotateY", e.y);
        self.set_eval(j, "rotateZ", e.z);
    }

    /// The rotation that takes direction `a` to direction `b` (about `fallback` when they are opposite).
    pub(crate) fn quat_from_to(a: Vec3, b: Vec3, fallback: Vec3) -> Quat {
        let (a, b) = (a.normalized(), b.normalized());
        let d = a.dot(b).clamp(-1.0, 1.0);
        if a.length_sq() < 1e-12 || b.length_sq() < 1e-12 || d > 0.999999 {
            return Quat::identity();
        }
        let mut axis = a.cross(b);
        if axis.length_sq() < 1e-12 {
            axis = if fallback.length_sq() > 1e-12 { fallback } else { Vec3::new(0.0, 0.0, 1.0) };
        }
        Quat::from_axis_angle(axis.normalized(), d.acos())
    }

    /// Cyclic coordinate descent over `chain` (end effector first).
    fn solve_ccd(&mut self, chain: &[NodeId], target: Vec3) {
        let end = chain[0];
        for _ in 0..40 {
            let eff = self.world_matrix(end).translation_part();
            if eff.distance(target) < 1e-3 {
                break;
            }
            for j in chain.iter().skip(1) {
                let jp = self.world_matrix(*j).translation_part();
                let eff = self.world_matrix(end).translation_part();
                let delta = Self::quat_from_to(eff - jp, target - jp, Vec3::new(0.0, 0.0, 1.0));
                self.rotate_joint_world(*j, delta);
            }
        }
    }

    /// Exact solution for a two bone chain (root, middle, end): the bend plane holds the pole vector.
    fn solve_two_bone(&mut self, chain: &[NodeId], target: Vec3, pole: Option<Vec3>, twist_deg: f32) {
        let (c, b, a) = (chain[0], chain[1], chain[2]);
        let (pa, pb, pc) = (self.world_matrix(a).translation_part(), self.world_matrix(b).translation_part(), self.world_matrix(c).translation_part());
        let (l1, l2) = (pa.distance(pb), pb.distance(pc));
        if l1 < 1e-6 || l2 < 1e-6 {
            return;
        }
        let to_t = target - pa;
        if to_t.length_sq() < 1e-12 {
            return;
        }
        let u = to_t.normalized();
        let reach_lo = (l1 - l2).abs() + 1e-4;
        let reach_hi = l1 + l2 - 1e-4;
        let d = to_t.length().clamp(reach_lo, reach_hi);
        // the direction the knee points: toward the pole, else where it is now
        let perp = |v: Vec3| v - u * u.dot(v);
        let mut pv = pole.map(|p| perp(p - pa)).unwrap_or(Vec3::ZERO);
        if pv.length_sq() < 1e-10 {
            pv = perp(pb - pa);
        }
        if pv.length_sq() < 1e-10 {
            pv = perp(if u.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) });
        }
        let mut pv = pv.normalized();
        if twist_deg.abs() > 1e-6 {
            pv = Quat::from_axis_angle(u, twist_deg.to_radians()).rotate(pv);
        }
        let cos_a = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0);
        let sin_a = (1.0 - cos_a * cos_a).sqrt();
        let new_b = pa + u * (l1 * cos_a) + pv * (l1 * sin_a);
        let new_c = pa + u * d;
        // root: swing the first bone onto its new direction
        let d1 = Self::quat_from_to(pb - pa, new_b - pa, pv);
        self.rotate_joint_world(a, d1);
        // middle: swing the second bone onto the effector point
        let (pb2, pc2) = (self.world_matrix(b).translation_part(), self.world_matrix(c).translation_part());
        let d2 = Self::quat_from_to(pc2 - pb2, new_c - pb2, pv);
        self.rotate_joint_world(b, d2);
    }

    /// Spline IK: the chain (start to end) lies along the curve with its bone lengths kept.
    fn solve_spline(&mut self, chain_root_first: &[NodeId], curve: &crate::geo::nurbs::Curve, offset: f32, roll_deg: f32) {
        let n = chain_root_first.len();
        if n < 2 {
            return;
        }
        let samples = curve.sample(24);
        if samples.len() < 2 {
            return;
        }
        let root = self.world_matrix(chain_root_first[0]).translation_part();
        // start at the curve point nearest to the root, moved along by the offset (in samples)
        let mut start = (0..samples.len()).min_by(|a, b| samples[*a].distance(root).partial_cmp(&samples[*b].distance(root)).unwrap()).unwrap_or(0);
        start = ((start as f32) + offset * 24.0).round().clamp(0.0, (samples.len() - 1) as f32) as usize;
        let mut targets: Vec<Vec3> = vec![samples[start]];
        let mut cursor = start;
        for i in 1..n {
            let len = {
                let a = self.world_matrix(chain_root_first[i - 1]).translation_part();
                let b = self.world_matrix(chain_root_first[i]).translation_part();
                a.distance(b)
            };
            let from = *targets.last().unwrap();
            let mut found = None;
            for k in cursor..samples.len() - 1 {
                let (d0, d1) = (samples[k].distance(from), samples[k + 1].distance(from));
                if d1 >= len && d0 <= len + 1e-4 {
                    let f = if (d1 - d0).abs() > 1e-9 { ((len - d0) / (d1 - d0)).clamp(0.0, 1.0) } else { 0.0 };
                    found = Some((k, samples[k].lerp(samples[k + 1], f)));
                    break;
                }
            }
            match found {
                Some((k, p)) => {
                    cursor = k;
                    targets.push(p);
                }
                None => {
                    // past the end of the curve: continue straight along its end direction
                    let last = samples[samples.len() - 1];
                    let dir = (last - samples[samples.len() - 2]).normalized();
                    targets.push(from + dir * len);
                }
            }
        }
        for i in 0..n - 1 {
            let j = chain_root_first[i];
            let (pj, pn) = (self.world_matrix(j).translation_part(), self.world_matrix(chain_root_first[i + 1]).translation_part());
            let want = targets[i + 1] - targets[i];
            let d = Self::quat_from_to(pn - pj, want, Vec3::new(0.0, 0.0, 1.0));
            self.rotate_joint_world(j, d);
        }
        if roll_deg.abs() > 1e-6 {
            let j = chain_root_first[0];
            let (pj, pn) = (self.world_matrix(j).translation_part(), self.world_matrix(chain_root_first[1]).translation_part());
            self.rotate_joint_world(j, Quat::from_axis_angle((pn - pj).normalized(), roll_deg.to_radians()));
        }
    }

    /// Solve every enabled IK handle: rotate plane (exact for two bones), single chain (CCD) and spline.
    pub fn solve_iks(&mut self) {
        let handles = self.ids_where(|d| matches!(d, NodeData::IkHandle(_)));
        let mut any = false;
        for h in handles {
            let (start, end, solver, curve, pole) = match self.nodes.get(h).map(|n| &n.data) {
                Some(NodeData::IkHandle(d)) => match (d.start, d.end) {
                    (Some(s), Some(e)) => (s, e, d.solver, d.curve, d.pole),
                    _ => continue,
                },
                _ => continue,
            };
            if self.get_attr(h, "enable").unwrap_or(1.0) < 0.5 {
                continue;
            }
            if self.nodes.get(start).is_none() || self.nodes.get(end).is_none() {
                continue;
            }
            // chain from the end effector up to the start joint
            let mut chain = vec![end];
            let mut cur = end;
            let mut ok = false;
            for _ in 0..64 {
                if cur == start {
                    ok = true;
                    break;
                }
                match self.nodes.get(cur).and_then(|n| n.parent) {
                    Some(p) => {
                        chain.push(p);
                        cur = p;
                    }
                    None => break,
                }
            }
            if !ok || chain.len() < 2 {
                continue;
            }
            let blend = self.get_attr(h, "ikBlend").unwrap_or(1.0).clamp(0.0, 1.0);
            // the FK pose: animated channels hold it now, the others keep their authored value
            let mut fk: Vec<(NodeId, [f32; 3])> = Vec::new();
            for j in chain.iter().skip(1) {
                let mut v = [0.0f32; 3];
                for (k, a) in ["rotateX", "rotateY", "rotateZ"].iter().enumerate() {
                    v[k] = if self.anim.is_animated(*j, a) { self.get_attr(*j, a).unwrap_or(0.0) } else { self.nodes.get(*j).and_then(|n| n.plug(a)).map(|p| p.rest).unwrap_or(0.0) };
                }
                fk.push((*j, v));
            }
            if blend < 1e-4 {
                for (j, v) in &fk {
                    for (k, a) in ["rotateX", "rotateY", "rotateZ"].iter().enumerate() {
                        self.set_eval(*j, a, v[k]);
                    }
                }
                any = true;
                continue;
            }
            let target = self.world_matrix(h).translation_part();
            match solver {
                2 => {
                    if let Some(c) = curve.and_then(|c| self.input_curve_world(c)) {
                        let root_first: Vec<NodeId> = chain.iter().rev().copied().collect();
                        let (off, roll) = (self.get_attr(h, "offset").unwrap_or(0.0), self.get_attr(h, "roll").unwrap_or(0.0));
                        self.solve_spline(&root_first, &c.0, off, roll);
                    }
                }
                0 if chain.len() == 3 => {
                    let pole_pos = pole.filter(|p| self.nodes.get(*p).is_some()).map(|p| self.world_matrix(p).translation_part());
                    let twist = self.get_attr(h, "twist").unwrap_or(0.0);
                    self.solve_two_bone(&chain, target, pole_pos, twist);
                }
                _ => self.solve_ccd(&chain, target),
            }
            if blend < 1.0 - 1e-4 {
                for (j, v) in &fk {
                    for (k, a) in ["rotateX", "rotateY", "rotateZ"].iter().enumerate() {
                        let ik = self.get_attr(*j, a).unwrap_or(0.0);
                        let mut d = ik - v[k];
                        while d > 180.0 {
                            d -= 360.0;
                        }
                        while d < -180.0 {
                            d += 360.0;
                        }
                        self.set_eval(*j, a, v[k] + d * blend);
                    }
                }
            }
            any = true;
        }
        if any {
            self.flush_meshes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_roundtrips_euler_angles() {
        let t = Vec3::new(1.0, -2.0, 3.0);
        let r = Vec3::new(30.0, 50.0, -20.0);
        let s = Vec3::new(1.0, 2.0, 0.5);
        let m = Mat4::trs(t, Quat::from_euler_deg(r), s);
        let (t2, r2, s2) = m.decompose();
        assert!((t2 - t).length() < 1e-4);
        assert!((r2 - r).length() < 1e-2, "{:?}", r2);
        assert!((s2 - s).length() < 1e-4);
    }

    #[test]
    fn ik_reaches_target_and_skin_follows_joint() {
        let mut sc = Scene::new();
        let j1 = sc.create_node(NodeData::Joint, "j1", None, false);
        let j2 = sc.create_node(NodeData::Joint, "j2", Some(j1), false);
        let j3 = sc.create_node(NodeData::Joint, "j3", Some(j2), false);
        sc.set_attr(j2, "translateY", 3.0).unwrap();
        sc.set_attr(j3, "translateY", 3.0).unwrap();
        let (_t, shape) = sc
            .create_mesh_object(
                "polyCylinder",
                vec![("radius".into(), 0.5), ("height".into(), 6.0), ("subdivisions".into(), 8.0)],
                Some("arm"),
            )
            .unwrap();
        sc.bind_skin(shape, &[j1, j2, j3]).unwrap();
        let top_before = match sc.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.mesh.positions.iter().map(|p| p.x).fold(f32::MIN, f32::max),
            _ => 0.0,
        };
        sc.set_attr(j1, "rotateZ", 90.0).unwrap();
        let top_after = match sc.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.mesh.positions.iter().map(|p| p.x).fold(f32::MIN, f32::max),
            _ => 0.0,
        };
        assert!(top_after > top_before + 1.0, "skin did not follow: {} -> {}", top_before, top_after);
        // IK
        sc.set_attr(j1, "rotateZ", 0.0).unwrap();
        let data = crate::dag::nodes::IkData { start: Some(j1), end: Some(j3), ..Default::default() };
        let h = sc.create_node(NodeData::IkHandle(Box::new(data)), "ik", None, false);
        sc.set_attr(h, "translateX", 3.0).unwrap();
        sc.set_attr(h, "translateY", 3.0).unwrap();
        let p = sc.world_matrix(j3).translation_part();
        assert!(p.distance(Vec3::new(3.0, 3.0, 0.0)) < 0.05, "end effector at {:?}", p);
    }
}
