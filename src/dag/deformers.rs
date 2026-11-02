//! Point deformers: wire, cluster, soft modification, jiggle; membership and ordering.

use super::graph::{NodeId, Scene};
use super::nodes::{DeformExt, JiggleState, NodeData, PolyKind};
use crate::geo::edit::Poly;
use crate::geo::nurbs::Curve;
use crate::math::{Mat4, Quat, Vec3};

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn pv(plugs: &[super::plugs::Plug], name: &str, default: f32) -> f32 {
    plugs.iter().find(|p| p.name == name).map(|p| p.value).unwrap_or(default)
}

/// Parameter of the point of `c` nearest to `p`, from a coarse sampling refined locally.
fn nearest_param(c: &Curve, samples: &[(f32, Vec3)], p: Vec3) -> (f32, f32) {
    let mut best = 0;
    let mut bd = f32::MAX;
    for (i, (_, q)) in samples.iter().enumerate() {
        let d = q.distance(p);
        if d < bd {
            bd = d;
            best = i;
        }
    }
    let lo = samples[best.saturating_sub(1)].0;
    let hi = samples[(best + 1).min(samples.len() - 1)].0;
    let (mut a, mut b) = (lo, hi);
    for _ in 0..14 {
        let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
        if c.point(m1).distance(p) < c.point(m2).distance(p) {
            b = m2;
        } else {
            a = m1;
        }
    }
    let t = (a + b) * 0.5;
    (t, c.point(t).distance(p))
}

impl Scene {
    fn mesh_world(&mut self, shape: NodeId) -> Mat4 {
        match self.nodes.get(shape).and_then(|n| n.parent) {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        }
    }

    fn handle_matrix(&mut self, handle: NodeId, bind_inv: Mat4) -> Mat4 {
        self.world_matrix(handle) * bind_inv
    }

    /// Evaluate a wire, cluster, soft modification or jiggle deformer node on `poly` (mesh local space).
    pub fn eval_point_deformer(&mut self, shape: NodeId, op: NodeId, kind: PolyKind, members: &[u32], plugs: &[super::plugs::Plug], poly: &mut Poly) {
        let ext: DeformExt = match self.nodes.get(op).map(|n| &n.data) {
            Some(NodeData::PolyOp(o)) => o.ext.as_deref().cloned().unwrap_or_default(),
            _ => return,
        };
        let envelope = pv(plugs, "envelope", 1.0).clamp(0.0, 1.0);
        if envelope <= 0.0 {
            return;
        }
        let mw = self.mesh_world(shape);
        let mw_inv = mw.inverse();
        let member_set: std::collections::HashSet<u32> = members.iter().copied().collect();
        let is_member = |i: usize| members.is_empty() || member_set.contains(&(i as u32));
        match kind {
            PolyKind::Cluster => {
                let (Some(h), Some(bi)) = (ext.handle, ext.bind_inv) else { return };
                if self.nodes.get(h).is_none() {
                    return;
                }
                let hm = self.handle_matrix(h, bi);
                for (i, p) in poly.pos.iter_mut().enumerate() {
                    if !is_member(i) {
                        continue;
                    }
                    let w = ext.weights.get(i).copied().unwrap_or(1.0) * envelope;
                    let pw = mw.transform_point(*p);
                    *p = mw_inv.transform_point(pw.lerp(hm.transform_point(pw), w));
                }
            }
            PolyKind::SoftMod => {
                let (Some(h), Some(bi), Some(center)) = (ext.handle, ext.bind_inv, ext.center) else { return };
                if self.nodes.get(h).is_none() {
                    return;
                }
                let hm = self.handle_matrix(h, bi);
                let radius = pv(plugs, "falloffRadius", 1.0).max(1e-4);
                let ftype = pv(plugs, "falloffType", 0.0).round() as i32;
                let cw = mw.transform_point(center);
                for (i, p) in poly.pos.iter_mut().enumerate() {
                    if !is_member(i) {
                        continue;
                    }
                    let pw = mw.transform_point(*p);
                    let x = 1.0 - pw.distance(cw) / radius;
                    if x <= 0.0 {
                        continue;
                    }
                    let f = match ftype {
                        1 => x,
                        2 => 1.0 - (1.0 - x) * (1.0 - x),
                        _ => smoothstep(x),
                    };
                    let w = f * ext.weights.get(i).copied().unwrap_or(1.0) * envelope;
                    *p = mw_inv.transform_point(pw.lerp(hm.transform_point(pw), w));
                }
            }
            PolyKind::Wire => {
                let (Some(cs), Some(base)) = (ext.curve, ext.base_curve.as_ref()) else { return };
                let Some((cur, _)) = self.input_curve_world(cs) else { return };
                let dropoff = pv(plugs, "dropoffDistance", 1.0).max(1e-4);
                let (scale, rot) = (pv(plugs, "scale", 1.0), pv(plugs, "rotation", 1.0).clamp(0.0, 1.0));
                let samples: Vec<(f32, Vec3)> = {
                    let (a, b) = base.dir.domain();
                    let n = base.dir.spans() * 12;
                    (0..=n).map(|i| {
                        let t = a + (b - a) * i as f32 / n as f32;
                        (t, base.point(t))
                    }).collect()
                };
                let (a, b) = base.dir.domain();
                let (ca, cb) = cur.dir.domain();
                for (i, p) in poly.pos.iter_mut().enumerate() {
                    if !is_member(i) {
                        continue;
                    }
                    let pw = mw.transform_point(*p);
                    let (t, d) = nearest_param(base, &samples, pw);
                    if d >= dropoff {
                        continue;
                    }
                    let w = smoothstep(1.0 - d / dropoff) * envelope;
                    // the same fraction along the current curve
                    let tc = if (b - a).abs() > 1e-9 { ca + (cb - ca) * (t - a) / (b - a) } else { ca };
                    let (bp, cp) = (base.point(t), cur.point(tc));
                    let r = Scene::quat_from_to(base.tangent(t), cur.tangent(tc), Vec3::UP);
                    let r = Quat::identity().slerp(r, rot);
                    let moved = cp + r.rotate(pw - bp);
                    let target = pw + (moved - pw) * scale;
                    *p = mw_inv.transform_point(pw.lerp(target, w));
                }
            }
            PolyKind::Jiggle => {
                let stiff = pv(plugs, "stiffness", 0.1).clamp(0.0, 1.0);
                let damp = pv(plugs, "damping", 0.2).clamp(0.0, 1.0);
                let wgt = pv(plugs, "jiggleWeight", 1.0).clamp(0.0, 1.0) * envelope;
                let mult = pv(plugs, "motionMultiplier", 1.0).max(0.0);
                let local = pv(plugs, "ignoreTransform", 0.0) > 0.5;
                let t = self.timeline.current;
                let to_space = |p: Vec3| if local { p } else { mw.transform_point(p) };
                let target: Vec<Vec3> = poly.pos.iter().map(|p| to_space(*p)).collect();
                let mut st = match self.nodes.get(op).map(|n| &n.data) {
                    Some(NodeData::PolyOp(o)) => o.ext.as_ref().and_then(|e| e.jiggle.clone()),
                    _ => None,
                }
                .filter(|s| s.target.len() == target.len());
                let fresh = |target: &Vec<Vec3>| JiggleState { t, target: target.clone(), offset: vec![Vec3::ZERO; target.len()], vel: vec![Vec3::ZERO; target.len()] };
                let mut state = match st.take() {
                    Some(s) => {
                        let dt = t - s.t;
                        if dt.abs() < 1e-4 {
                            s
                        } else if dt > 0.0 && dt <= 2.5 {
                            // one or two frames forward: integrate the springs toward the new targets
                            let steps = dt.round().max(1.0) as usize;
                            let mut s = s;
                            for _ in 0..steps {
                                for v in 0..target.len() {
                                    let mut pos = s.target[v] + s.offset[v];
                                    // the pull of the motion itself: the point trails the target by `mult`
                                    let tgt = target[v];
                                    for _ in 0..4 {
                                        s.vel[v] = s.vel[v] * (1.0 - damp * 0.25).max(0.0) + (tgt - pos) * (stiff * 0.6 / mult.max(0.05));
                                        pos += s.vel[v] * 0.25 * 4.0 * 0.25;
                                    }
                                    s.offset[v] = pos - tgt;
                                }
                                s.target = target.clone();
                            }
                            s.t = t;
                            s
                        } else {
                            fresh(&target)
                        }
                    }
                    None => fresh(&target),
                };
                // the output follows the current target even when the time did not change
                state.target = target.clone();
                for (i, p) in poly.pos.iter_mut().enumerate() {
                    if !is_member(i) {
                        continue;
                    }
                    let w = wgt * ext.weights.get(i).copied().unwrap_or(1.0);
                    let out = target[i] + state.offset[i] * w;
                    *p = if local { out } else { mw_inv.transform_point(out) };
                }
                if let Some(NodeData::PolyOp(o)) = self.nodes.get_mut(op).map(|n| &mut n.data) {
                    if let Some(e) = o.ext.as_mut() {
                        e.jiggle = Some(state);
                    }
                }
            }
            _ => {}
        }
    }

    fn deformer_shape_poly(&mut self, shape: NodeId) -> Result<(Poly, Mat4), String> {
        self.flush_meshes();
        let poly = match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) if md.nurbs.is_none() => md.poly(),
            _ => return Err("Select a polygon object".to_string()),
        };
        let mw = self.mesh_world(shape);
        Ok((poly, mw))
    }

    fn add_deformer_op(&mut self, shape: NodeId, kind: PolyKind, members: Vec<u32>, plugs: &[(String, f32)], ext: DeformExt) -> Result<NodeId, String> {
        let op = self.append_history(shape, kind, members, plugs)?;
        if let Some(NodeData::PolyOp(o)) = self.nodes.get_mut(op).map(|n| &mut n.data) {
            o.ext = Some(Box::new(ext));
        }
        self.skins.push(op);
        self.mark_poly_dirty(op);
        self.flush_meshes();
        Ok(op)
    }

    fn make_handle(&mut self, name: &str, at: Vec3) -> NodeId {
        let t = self.create_node(NodeData::Transform, name, None, true);
        self.create_node(NodeData::Locator, &format!("{}Shape", self.name_of(t)), Some(t), false);
        for (a, v) in [("translateX", at.x), ("translateY", at.y), ("translateZ", at.z)] {
            let _ = self.set_attr(t, a, v);
        }
        t
    }

    /// Create Deformers > Cluster on mesh points: a handle at their center moves, rotates and scales them.
    pub fn create_cluster(&mut self, shape: NodeId, members: Vec<u32>) -> Result<(NodeId, NodeId), String> {
        let (poly, mw) = self.deformer_shape_poly(shape)?;
        if members.is_empty() || members.iter().any(|m| *m as usize >= poly.pos.len()) {
            return Err("Select the vertices for the cluster".to_string());
        }
        let c = members.iter().fold(Vec3::ZERO, |a, i| a + mw.transform_point(poly.pos[*i as usize])) * (1.0 / members.len() as f32);
        let h = self.make_handle("clusterHandle", c);
        let bind_inv = self.world_matrix(h).inverse();
        let op = self.add_deformer_op(shape, PolyKind::Cluster, members, &[], DeformExt { handle: Some(h), bind_inv: Some(bind_inv), ..Default::default() })?;
        Ok((op, h))
    }

    /// Create Deformers > Soft Modification: the handle sits at the center of the selected points.
    pub fn create_soft_mod(&mut self, shape: NodeId, center_verts: &[u32], radius: f32, falloff_type: u8) -> Result<(NodeId, NodeId), String> {
        let (poly, mw) = self.deformer_shape_poly(shape)?;
        if center_verts.is_empty() || center_verts.iter().any(|m| *m as usize >= poly.pos.len()) {
            return Err("Select the vertices where the soft modification starts".to_string());
        }
        let center_local = center_verts.iter().fold(Vec3::ZERO, |a, i| a + poly.pos[*i as usize]) * (1.0 / center_verts.len() as f32);
        let h = self.make_handle("softModHandle", mw.transform_point(center_local));
        let bind_inv = self.world_matrix(h).inverse();
        let op = self.add_deformer_op(shape, PolyKind::SoftMod, Vec::new(), &[("falloffRadius".to_string(), radius), ("falloffType".to_string(), falloff_type as f32)], DeformExt { handle: Some(h), bind_inv: Some(bind_inv), center: Some(center_local), ..Default::default() })?;
        Ok((op, h))
    }

    /// Create Deformers > Wire: `curve_shape` drags the points of the mesh within `dropoff` of it.
    pub fn create_wire(&mut self, shape: NodeId, curve_shape: NodeId, dropoff: f32) -> Result<NodeId, String> {
        self.deformer_shape_poly(shape)?;
        let (base, _) = self.input_curve_world(curve_shape).ok_or("The wire must be a NURBS curve")?;
        self.add_deformer_op(shape, PolyKind::Wire, Vec::new(), &[("dropoffDistance".to_string(), dropoff)], DeformExt { curve: Some(curve_shape), base_curve: Some(base), ..Default::default() })
    }

    /// Create Deformers > Jiggle Deformer on the member points (all when empty).
    pub fn create_jiggle(&mut self, shape: NodeId, members: Vec<u32>, stiffness: f32, damping: f32, ignore_transform: bool) -> Result<NodeId, String> {
        self.deformer_shape_poly(shape)?;
        let plugs = [("stiffness".to_string(), stiffness), ("damping".to_string(), damping), ("ignoreTransform".to_string(), ignore_transform as u8 as f32)];
        let op = self.add_deformer_op(shape, PolyKind::Jiggle, members, &plugs, DeformExt::default())?;
        Ok(op)
    }

    /// The deformer history nodes of a shape, in evaluation order.
    pub fn deformer_ops(&self, shape: NodeId) -> Vec<NodeId> {
        self.history_of(shape)
            .into_iter()
            .filter(|h| matches!(self.nodes.get(*h).map(|n| &n.data), Some(NodeData::PolyOp(op)) if matches!(op.kind, PolyKind::Lattice | PolyKind::Wave | PolyKind::Wire | PolyKind::Cluster | PolyKind::SoftMod | PolyKind::Jiggle | PolyKind::Bend | PolyKind::Twist | PolyKind::Flare | PolyKind::Squash | PolyKind::Sine | PolyKind::Skin | PolyKind::Blend)))
            .collect()
    }

    pub fn op_shape(&self, op: NodeId) -> Option<NodeId> {
        match self.nodes.get(op).map(|n| &n.data) {
            Some(NodeData::PolyOp(o)) => o.owner,
            _ => None,
        }
    }

    /// Edit > Inputs > ...: move a deformer one step up or down the history, or to the front or back of the deformers.
    pub fn reorder_deformer(&mut self, op: NodeId, how: &str) -> Result<(), String> {
        let shape = self.op_shape(op).ok_or("Not a history node")?;
        let hist = self.history_of(shape);
        let pos = hist.iter().position(|h| *h == op).ok_or("Not part of the shape's history")?;
        if pos == 0 {
            return Err("The first history node creates the geometry".to_string());
        }
        let new = match how {
            "up" | "later" => (pos + 1).min(hist.len() - 1),
            "down" | "earlier" => pos.saturating_sub(1).max(1),
            "front" => 1,
            "back" => hist.len() - 1,
            o => return Err(format!("Unknown order '{}': up, down, front or back", o)),
        };
        if new == pos {
            return Ok(());
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            let h = md.history.remove(pos);
            md.history.insert(new, h);
        }
        let first = pos.min(new);
        let hist2 = self.history_of(shape);
        if let Some(n) = hist2.get(first) {
            self.mark_poly_dirty(*n);
        }
        self.flush_meshes();
        Ok(())
    }

    /// Edit Deformers > Edit Membership: add or remove vertices of a deformer's set (`set` replaces it).
    pub fn edit_membership(&mut self, op: NodeId, verts: &[u32], mode: &str) -> Result<usize, String> {
        let (shape, count) = {
            let shape = self.op_shape(op).ok_or("Not a deformer node")?;
            let n = match self.nodes.get(shape).map(|n| &n.data) {
                Some(NodeData::Mesh(md)) => md.mesh.positions.len(),
                _ => 0,
            };
            (shape, n)
        };
        let _ = shape;
        // the vertex count of the deformer's input
        let all: Vec<u32> = {
            let hist = self.history_of(self.op_shape(op).unwrap());
            let pos = hist.iter().position(|h| *h == op).unwrap_or(0);
            match pos.checked_sub(1).and_then(|p| self.nodes.get(hist[p])).map(|n| &n.data) {
                Some(NodeData::PolyOp(o)) => (0..o.cache.as_ref().map(|c| c.pos.len()).unwrap_or(count) as u32).collect(),
                _ => (0..count as u32).collect(),
            }
        };
        let n = match self.nodes.get_mut(op).map(|n| &mut n.data) {
            Some(NodeData::PolyOp(o)) => {
                let mut cur: std::collections::BTreeSet<u32> = if o.ints.is_empty() { all.iter().copied().collect() } else { o.ints.iter().copied().collect() };
                match mode {
                    "add" => cur.extend(verts.iter().copied().filter(|v| (*v as usize) < all.len())),
                    "remove" => {
                        for v in verts {
                            cur.remove(v);
                        }
                    }
                    "set" => cur = verts.iter().copied().filter(|v| (*v as usize) < all.len()).collect(),
                    o2 => return Err(format!("Unknown membership mode '{}': add, remove or set", o2)),
                }
                // everything a member again means no restriction
                o.ints = if cur.len() == all.len() { Vec::new() } else { cur.iter().copied().collect() };
                cur.len()
            }
            _ => return Err("Not a deformer node".to_string()),
        };
        self.mark_poly_dirty(op);
        self.flush_meshes();
        Ok(n)
    }

    /// Per point weights of a deformer (jiggle weights, cluster weights): set the weight of `verts` to `value`, or smooth them.
    pub fn paint_deformer_weights(&mut self, op: NodeId, verts: &[u32], value: f32, smooth: bool) -> Result<usize, String> {
        let shape = self.op_shape(op).ok_or("Not a deformer node")?;
        let poly = match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.poly(),
            _ => return Err("Not a mesh".to_string()),
        };
        let mut nb: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
        for (a, b) in poly.edge_list() {
            nb.entry(a).or_default().push(b);
            nb.entry(b).or_default().push(a);
        }
        let n = poly.pos.len();
        match self.nodes.get_mut(op).map(|x| &mut x.data) {
            Some(NodeData::PolyOp(o)) => {
                let ext = o.ext.get_or_insert_with(Default::default);
                if ext.weights.len() != n {
                    ext.weights = vec![1.0; n];
                }
                if smooth {
                    let old = ext.weights.clone();
                    for v in verts {
                        if let Some(ns) = nb.get(v) {
                            let avg = ns.iter().map(|k| old[*k as usize]).sum::<f32>() / ns.len() as f32;
                            ext.weights[*v as usize] = old[*v as usize] * 0.5 + avg * 0.5;
                        }
                    }
                } else {
                    for v in verts {
                        if (*v as usize) < n {
                            ext.weights[*v as usize] = value.clamp(0.0, 1.0);
                        }
                    }
                }
            }
            _ => return Err("Not a deformer node".to_string()),
        }
        self.mark_poly_dirty(op);
        self.flush_meshes();
        Ok(verts.len())
    }

    /// Jiggle deformers recompute every frame (they depend on the time).
    pub fn mark_jiggles_dirty(&mut self) {
        let ops: Vec<NodeId> = self.skins.iter().copied().filter(|o| matches!(self.nodes.get(*o).map(|n| &n.data), Some(NodeData::PolyOp(p)) if p.kind == PolyKind::Jiggle)).collect();
        if ops.is_empty() {
            return;
        }
        for o in ops {
            self.mark_poly_dirty(o);
        }
        self.flush_meshes();
    }
}

#[allow(dead_code)]
fn _unused(_: JiggleState) {}
