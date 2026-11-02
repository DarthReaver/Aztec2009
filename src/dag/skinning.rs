//! Skin weight editing: influences, normalize, prune, smooth, mirror, copy, paint, bind pose.

use super::graph::{NodeId, Scene};
use super::nodes::{NodeData, SkinData};
use crate::math::{Mat4, Quat, Vec3};
use std::collections::HashMap;

/// How a paint stroke changes the weight of the active joint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintOp {
    Replace,
    Add,
    Scale,
    Smooth,
}

impl PaintOp {
    pub fn parse(s: &str) -> Option<PaintOp> {
        match s.to_lowercase().as_str() {
            "replace" | "r" => Some(PaintOp::Replace),
            "add" | "a" => Some(PaintOp::Add),
            "scale" | "s" => Some(PaintOp::Scale),
            "smooth" => Some(PaintOp::Smooth),
            _ => None,
        }
    }
}

/// Weight of joint `j` in a vertex's influence list.
pub fn weight_of(w: &[(u16, f32)], j: u16) -> f32 {
    w.iter().find(|(x, _)| *x == j).map(|x| x.1).unwrap_or(0.0)
}

/// Set one joint's weight and rescale the others so the vertex still sums to one.
pub fn set_weight_normalized(w: &mut Vec<(u16, f32)>, j: u16, value: f32) {
    let value = value.clamp(0.0, 1.0);
    w.retain(|(x, _)| *x != j);
    let others: f32 = w.iter().map(|x| x.1).sum();
    if others > 1e-6 {
        let k = (1.0 - value) / others;
        for e in w.iter_mut() {
            e.1 *= k;
        }
    } else if value < 1.0 {
        // nothing to share the rest with: the joint keeps what it was given
    }
    w.retain(|(_, x)| *x > 1e-6);
    if value > 1e-6 {
        w.push((j, value));
    }
}

impl Scene {
    /// Run `f` on the skin data of a bound mesh shape, then re-evaluate the mesh.
    pub fn with_skin<R>(&mut self, shape: NodeId, f: impl FnOnce(&mut SkinData) -> Result<R, String>) -> Result<R, String> {
        let node = self.skin_of(shape).ok_or("The mesh is not bound to a skeleton")?;
        let r = match self.nodes.get_mut(node).map(|n| &mut n.data) {
            Some(NodeData::PolyOp(op)) => f(op.skin.as_mut().ok_or("The mesh is not bound to a skeleton")?)?,
            _ => return Err("The mesh is not bound to a skeleton".to_string()),
        };
        self.mark_poly_dirty(node);
        self.flush_meshes();
        Ok(r)
    }

    pub fn skin_data(&self, shape: NodeId) -> Option<&SkinData> {
        let node = self.skin_of(shape)?;
        match self.nodes.get(node).map(|n| &n.data) {
            Some(NodeData::PolyOp(op)) => op.skin.as_ref(),
            _ => None,
        }
    }

    /// Skin > Edit Smooth Skin > Add Influence: the joint starts with no weights.
    pub fn add_influence(&mut self, shape: NodeId, joint: NodeId) -> Result<(), String> {
        let inv = self.world_matrix(joint).inverse();
        self.with_skin(shape, |sk| {
            if sk.joints.contains(&joint) {
                return Err("The joint already influences this mesh".to_string());
            }
            sk.joints.push(joint);
            sk.bind_inv.push(inv);
            Ok(())
        })
    }

    /// Remove Influence: its weights are shared out among the other joints of each vertex.
    pub fn remove_influence(&mut self, shape: NodeId, joint: NodeId) -> Result<(), String> {
        self.with_skin(shape, |sk| {
            let j = sk.joints.iter().position(|x| *x == joint).ok_or("The joint does not influence this mesh")? as u16;
            if sk.joints.len() == 1 {
                return Err("A skin needs at least one influence".to_string());
            }
            sk.joints.remove(j as usize);
            sk.bind_inv.remove(j as usize);
            for w in sk.weights.iter_mut() {
                w.retain(|(x, _)| *x != j);
                for e in w.iter_mut() {
                    if e.0 > j {
                        e.0 -= 1;
                    }
                }
                let sum: f32 = w.iter().map(|x| x.1).sum();
                if sum > 1e-6 {
                    for e in w.iter_mut() {
                        e.1 /= sum;
                    }
                }
            }
            Ok(())
        })
    }

    pub fn normalize_skin(&mut self, shape: NodeId) -> Result<(), String> {
        self.with_skin(shape, |sk| {
            for w in sk.weights.iter_mut() {
                let sum: f32 = w.iter().map(|x| x.1).sum();
                if sum > 1e-9 {
                    for e in w.iter_mut() {
                        e.1 /= sum;
                    }
                }
            }
            Ok(())
        })
    }

    /// Prune Small Weights: drop weights below `threshold`, then normalize.
    pub fn prune_skin(&mut self, shape: NodeId, threshold: f32) -> Result<usize, String> {
        self.with_skin(shape, |sk| {
            let mut removed = 0;
            for w in sk.weights.iter_mut() {
                let before = w.len();
                // never prune a vertex to nothing
                let best = w.iter().copied().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                w.retain(|(_, x)| *x >= threshold);
                if w.is_empty() {
                    if let Some(b) = best {
                        w.push(b);
                    }
                }
                removed += before - w.len();
                let sum: f32 = w.iter().map(|x| x.1).sum();
                for e in w.iter_mut() {
                    e.1 /= sum.max(1e-9);
                }
            }
            Ok(removed)
        })
    }

    /// Limit the number of influences per vertex (keeps the strongest) and normalize.
    pub fn limit_influences(&mut self, shape: NodeId, max: usize) -> Result<(), String> {
        self.with_skin(shape, |sk| {
            sk.max_inf = max.clamp(1, 8) as u8;
            for w in sk.weights.iter_mut() {
                w.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                w.truncate(max.clamp(1, 8));
                let sum: f32 = w.iter().map(|x| x.1).sum();
                for e in w.iter_mut() {
                    e.1 /= sum.max(1e-9);
                }
            }
            Ok(())
        })
    }

    /// Smooth Skin Weights: blend every selected vertex toward the average of its neighbours.
    pub fn smooth_skin(&mut self, shape: NodeId, verts: &[u32], iterations: usize, strength: f32) -> Result<(), String> {
        let poly = match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.poly(),
            _ => return Err("Not a mesh".to_string()),
        };
        let mut nb: HashMap<u32, Vec<u32>> = HashMap::new();
        for (a, b) in poly.edge_list() {
            nb.entry(a).or_default().push(b);
            nb.entry(b).or_default().push(a);
        }
        self.with_skin(shape, |sk| {
            for _ in 0..iterations.max(1) {
                let old = sk.weights.clone();
                for &v in verts {
                    let Some(ns) = nb.get(&v) else { continue };
                    let mut acc: HashMap<u16, f32> = HashMap::new();
                    for n in ns {
                        for (j, w) in old.get(*n as usize).map(|x| x.as_slice()).unwrap_or(&[]) {
                            *acc.entry(*j).or_default() += w / ns.len() as f32;
                        }
                    }
                    let cur = old.get(v as usize).cloned().unwrap_or_default();
                    let mut out: HashMap<u16, f32> = HashMap::new();
                    for (j, w) in &cur {
                        *out.entry(*j).or_default() += w * (1.0 - strength);
                    }
                    for (j, w) in &acc {
                        *out.entry(*j).or_default() += w * strength;
                    }
                    let mut list: Vec<(u16, f32)> = out.into_iter().filter(|(_, w)| *w > 1e-4).collect();
                    list.sort_by_key(|x| x.0);
                    let sum: f32 = list.iter().map(|x| x.1).sum();
                    for e in list.iter_mut() {
                        e.1 /= sum.max(1e-9);
                    }
                    if let Some(w) = sk.weights.get_mut(v as usize) {
                        *w = list;
                    }
                }
            }
            Ok(())
        })
    }

    /// Reset Weights to Default: the distance based weights of the bind options.
    pub fn reset_skin(&mut self, shape: NodeId) -> Result<(), String> {
        let (poly, parent) = match self.nodes.get(shape) {
            Some(n) => match &n.data {
                NodeData::Mesh(md) => (md.poly(), n.parent),
                _ => return Err("Not a mesh".to_string()),
            },
            None => return Err("Invalid node".to_string()),
        };
        let _ = parent;
        let (joints, bind_inv, mesh_world, rigid, max_inf, dropoff) = {
            let sk = self.skin_data(shape).ok_or("The mesh is not bound to a skeleton")?;
            (sk.joints.clone(), sk.bind_inv.clone(), sk.bind_mesh_world, sk.rigid, if sk.max_inf == 0 { 4 } else { sk.max_inf as usize }, if sk.dropoff <= 0.0 { 4.0 } else { sk.dropoff })
        };
        // the skin's input is the bind pose geometry
        let bind_pos: Vec<Vec3> = self.skin_input_positions(shape).unwrap_or_else(|| poly.pos.clone());
        let pw: Vec<Vec3> = bind_pos.iter().map(|p| mesh_world.transform_point(*p)).collect();
        let jw: Vec<Mat4> = bind_inv.iter().map(|m| m.inverse()).collect();
        let parents: HashMap<NodeId, Option<NodeId>> = joints.iter().map(|j| (*j, self.nodes.get(*j).and_then(|n| n.parent))).collect();
        let weights = Scene::default_weights(&joints, &|j| parents.get(&j).copied().flatten(), &jw, &pw, rigid, max_inf, dropoff);
        self.with_skin(shape, |sk| {
            if sk.weights.len() == weights.len() {
                sk.weights = weights;
            }
            Ok(())
        })
    }

    /// Mirror Skin Weights: copy the weights of one side across the plane through the origin (local space of the mesh).
    /// Joints are paired by their mirrored bind position.
    pub fn mirror_skin(&mut self, shape: NodeId, axis: usize, positive_to_negative: bool, tolerance: f32) -> Result<usize, String> {
        let pos: Vec<Vec3> = self.skin_input_positions(shape).ok_or("The mesh is not bound to a skeleton")?;
        let (joints, bind_inv, mesh_world) = {
            let sk = self.skin_data(shape).ok_or("The mesh is not bound to a skeleton")?;
            (sk.joints.clone(), sk.bind_inv.clone(), sk.bind_mesh_world)
        };
        let comp = |p: Vec3| [p.x, p.y, p.z][axis];
        let flip = |mut p: Vec3| {
            match axis {
                0 => p.x = -p.x,
                1 => p.y = -p.y,
                _ => p.z = -p.z,
            }
            p
        };
        // joint pairing by mirrored bind world position
        let jpos: Vec<Vec3> = bind_inv.iter().map(|m| m.inverse().translation_part()).collect();
        let jmap: Vec<u16> = jpos
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let m = flip(mesh_world.inverse().transform_point(*p));
                let target = mesh_world.transform_point(m);
                jpos.iter().enumerate().min_by(|a, b| a.1.distance(target).partial_cmp(&b.1.distance(target)).unwrap()).map(|(k, _)| k as u16).unwrap_or(i as u16)
            })
            .collect();
        let _ = joints;
        // vertex pairing
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (v, p) in pos.iter().enumerate() {
            let on_source = if positive_to_negative { comp(*p) > tolerance } else { comp(*p) < -tolerance };
            if !on_source {
                continue;
            }
            let target = flip(*p);
            if let Some((m, d)) = pos.iter().enumerate().map(|(k, q)| (k, q.distance(target))).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()) {
                if d <= tolerance.max(1e-3) * 4.0 && m != v {
                    pairs.push((v, m));
                }
            }
        }
        let n = pairs.len();
        self.with_skin(shape, |sk| {
            for (v, m) in &pairs {
                let src = sk.weights.get(*v).cloned().unwrap_or_default();
                let mapped: Vec<(u16, f32)> = src.iter().map(|(j, w)| (jmap.get(*j as usize).copied().unwrap_or(*j), *w)).collect();
                if let Some(dst) = sk.weights.get_mut(*m) {
                    *dst = mapped;
                }
            }
            Ok(())
        })?;
        Ok(n)
    }

    /// Copy Skin Weights from another bound mesh: every vertex takes the weights of the nearest source vertex (world space).
    pub fn copy_skin_weights(&mut self, src: NodeId, dst: NodeId) -> Result<usize, String> {
        let (sjoints, sweights) = {
            let sk = self.skin_data(src).ok_or("The source mesh is not bound to a skeleton")?;
            (sk.joints.clone(), sk.weights.clone())
        };
        let src_world: Vec<Vec3> = self.skin_world_points(src).ok_or("The source mesh has no geometry")?;
        // the target needs the same joints
        if self.skin_of(dst).is_none() {
            self.bind_skin_ex(dst, &sjoints, true, 4, 4.0)?;
        }
        for j in &sjoints {
            let has = self.skin_data(dst).map(|s| s.joints.contains(j)).unwrap_or(false);
            if !has {
                self.add_influence(dst, *j)?;
            }
        }
        let dst_world: Vec<Vec3> = self.skin_world_points(dst).ok_or("The target mesh has no geometry")?;
        let djoints = self.skin_data(dst).map(|s| s.joints.clone()).unwrap_or_default();
        let map: Vec<u16> = sjoints.iter().map(|j| djoints.iter().position(|d| d == j).unwrap_or(0) as u16).collect();
        let count = dst_world.len();
        self.with_skin(dst, |sk| {
            for (v, p) in dst_world.iter().enumerate() {
                let nearest = src_world.iter().enumerate().min_by(|a, b| a.1.distance(*p).partial_cmp(&b.1.distance(*p)).unwrap()).map(|(i, _)| i).unwrap_or(0);
                if let (Some(w), Some(d)) = (sweights.get(nearest), sk.weights.get_mut(v)) {
                    *d = w.iter().map(|(j, x)| (map[*j as usize], *x)).collect();
                }
            }
            Ok(())
        })?;
        Ok(count)
    }

    /// Bind pose positions the skin deforms (the mesh's input before the skin node), in mesh local space.
    fn skin_input_positions(&mut self, shape: NodeId) -> Option<Vec<Vec3>> {
        let skin = self.skin_of(shape)?;
        self.flush_meshes();
        let hist = self.history_of(shape);
        let pos = hist.iter().position(|h| *h == skin)?;
        // the cache of the node before the skin holds the bind pose geometry
        let before = if pos == 0 { return None } else { hist[pos - 1] };
        match self.nodes.get(before).map(|n| &n.data) {
            Some(NodeData::PolyOp(op)) => op.cache.as_ref().map(|c| c.pos.clone()),
            _ => None,
        }
    }

    /// Current (deformed) vertex positions in world space.
    pub fn skin_world_points(&mut self, shape: NodeId) -> Option<Vec<Vec3>> {
        self.flush_meshes();
        let parent = self.nodes.get(shape).and_then(|n| n.parent);
        let w = match parent {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        };
        match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => Some(md.poly().pos.iter().map(|p| w.transform_point(*p)).collect()),
            _ => None,
        }
    }

    /// Paint Skin Weights: one dab at `center` (mesh local space of the deformed mesh) with a soft brush.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_skin(&mut self, shape: NodeId, center: Vec3, radius: f32, joint: NodeId, value: f32, op: PaintOp, strength: f32) -> Result<usize, String> {
        let poly = match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.poly(),
            _ => return Err("Not a mesh".to_string()),
        };
        let mut nb: HashMap<u32, Vec<u32>> = HashMap::new();
        if op == PaintOp::Smooth {
            for (a, b) in poly.edge_list() {
                nb.entry(a).or_default().push(b);
                nb.entry(b).or_default().push(a);
            }
        }
        let hits: Vec<(usize, f32)> = poly
            .pos
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let d = p.distance(center);
                if d <= radius {
                    let f = 1.0 - (d / radius).powi(2);
                    Some((i, f * f))
                } else {
                    None
                }
            })
            .collect();
        let n = hits.len();
        self.with_skin(shape, |sk| {
            let j = sk.joints.iter().position(|x| *x == joint).ok_or("The joint does not influence this mesh (Add Influence first)")? as u16;
            let old = sk.weights.clone();
            for (v, f) in hits {
                let k = (f * strength).clamp(0.0, 1.0);
                let cur = weight_of(&old[v], j);
                let target = match op {
                    PaintOp::Replace => cur + (value - cur) * k,
                    PaintOp::Add => cur + value * k,
                    PaintOp::Scale => cur * (1.0 + (value - 1.0) * k),
                    PaintOp::Smooth => {
                        let ns = nb.get(&(v as u32)).cloned().unwrap_or_default();
                        let avg = if ns.is_empty() { cur } else { ns.iter().map(|n| weight_of(&old[*n as usize], j)).sum::<f32>() / ns.len() as f32 };
                        cur + (avg - cur) * k
                    }
                };
                set_weight_normalized(&mut sk.weights[v], j, target);
            }
            Ok(())
        })?;
        Ok(n)
    }

    /// Go to Bind Pose: every joint of the skin returns to its bind pose transformation.
    pub fn go_to_bind_pose(&mut self, shape: NodeId) -> Result<usize, String> {
        let (joints, bind_inv) = {
            let sk = self.skin_data(shape).ok_or("The mesh is not bound to a skeleton")?;
            (sk.joints.clone(), sk.bind_inv.clone())
        };
        // parents first so each joint sees its parent's bind pose
        let mut order: Vec<usize> = (0..joints.len()).collect();
        order.sort_by_key(|i| self.descendants_depth(joints[*i]));
        for i in order {
            let j = joints[i];
            if self.nodes.get(j).is_none() {
                continue;
            }
            let target_world = bind_inv[i].inverse();
            let pw = match self.nodes.get(j).and_then(|n| n.parent) {
                Some(p) => self.world_matrix(p),
                None => Mat4::identity(),
            };
            let local = pw.inverse() * target_world;
            let (t, r, _) = self.trs_for_local(j, &local);
            for (a, v) in ["X", "Y", "Z"].iter().zip([t.x, t.y, t.z]) {
                self.set_attr(j, &format!("translate{}", a), v)?;
            }
            for (a, v) in ["X", "Y", "Z"].iter().zip([r.x, r.y, r.z]) {
                self.set_attr(j, &format!("rotate{}", a), v)?;
            }
        }
        self.mark_skins_dirty();
        Ok(joints.len())
    }

    fn descendants_depth(&self, id: NodeId) -> usize {
        let mut d = 0;
        let mut cur = self.nodes.get(id).and_then(|n| n.parent);
        while let Some(p) = cur {
            d += 1;
            cur = self.nodes.get(p).and_then(|n| n.parent);
        }
        d
    }

    /// Move Skinned Joints: move `joint` (and what hangs below it) by a world offset and refresh the bind pose of
    /// every skin that uses them, so the skinned geometry does not change.
    pub fn move_skinned_joint(&mut self, joint: NodeId, delta: Vec3) -> Result<usize, String> {
        let wp = self.world_matrix(joint).translation_part() + delta;
        let pw = match self.nodes.get(joint).and_then(|n| n.parent) {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        };
        let lp = pw.inverse().transform_point(wp);
        for (a, v) in ["X", "Y", "Z"].iter().zip([lp.x, lp.y, lp.z]) {
            self.set_attr(joint, &format!("translate{}", a), v)?;
        }
        let moved = self.joints_under(&[joint]);
        let skins: Vec<NodeId> = self.skins.clone();
        let mut n = 0;
        for sk_node in skins {
            if !matches!(self.nodes.get(sk_node).map(|x| &x.data), Some(NodeData::PolyOp(op)) if op.skin.is_some()) {
                continue;
            }
            let joints_of: Vec<(usize, NodeId)> = match self.nodes.get(sk_node).map(|x| &x.data) {
                Some(NodeData::PolyOp(op)) => op.skin.as_ref().map(|sk| sk.joints.iter().enumerate().filter(|(_, j)| moved.contains(j)).map(|(i, j)| (i, *j)).collect()).unwrap_or_default(),
                _ => Vec::new(),
            };
            let invs: Vec<(usize, Mat4)> = joints_of.iter().map(|(i, j)| (*i, self.world_matrix(*j).inverse())).collect();
            if let Some(NodeData::PolyOp(op)) = self.nodes.get_mut(sk_node).map(|x| &mut x.data) {
                if let Some(sk) = op.skin.as_mut() {
                    for (i, m) in invs {
                        sk.bind_inv[i] = m;
                        n += 1;
                    }
                }
            }
            self.mark_poly_dirty(sk_node);
        }
        self.flush_meshes();
        Ok(n)
    }
}

#[allow(dead_code)]
fn _unused(_: Quat) {}
