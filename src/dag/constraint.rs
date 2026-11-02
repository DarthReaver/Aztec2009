//! Constraints (point, orient, scale, aim, parent) and Set Driven Key curves.

use super::graph::{NodeId, Scene};
use super::nodes::{ConstraintData, ConstraintKind, NodeData, MAX_CONSTRAINT_TARGETS};
use super::plugs::PlugRef;
use crate::anim::keyframes::Curve;
use crate::anim::Interp;
use crate::math::{Mat4, Quat, Vec3};

pub(crate) fn quat_of_world(m: &Mat4) -> Quat {
    let (_, r, _) = m.decompose();
    Quat::from_euler_deg(r)
}

fn quat_to_euler(q: Quat) -> Vec3 {
    Mat4::from_quat(q).decompose().1
}

/// Normalized weighted average of quaternions (nlerp, signs aligned to the first).
fn blend_quats(qs: &[(Quat, f32)]) -> Quat {
    let first = qs[0].0;
    let (mut x, mut y, mut z, mut w) = (0.0, 0.0, 0.0, 0.0);
    for (q, wt) in qs {
        let s = if q.dot(first) < 0.0 { -1.0 } else { 1.0 } * wt;
        x += q.x * s;
        y += q.y * s;
        z += q.z * s;
        w += q.w * s;
    }
    Quat::new(x, y, z, w).normalized()
}

/// World matrix whose columns are the given axes (rotation only).
fn basis(x: Vec3, y: Vec3, z: Vec3) -> Mat4 {
    let mut m = Mat4::identity();
    for (c, v) in [x, y, z].iter().enumerate() {
        m.m[c][0] = v.x;
        m.m[c][1] = v.y;
        m.m[c][2] = v.z;
    }
    m
}

impl Scene {
    /// A motion path that moves `target` along `curve_shape`.
    pub fn create_motion_path(&mut self, curve_shape: NodeId, target: NodeId) -> Result<NodeId, String> {
        if !matches!(self.nodes.get(curve_shape).map(|n| &n.data), Some(NodeData::NurbsCurve(_))) {
            return Err("The path must be a NURBS curve".to_string());
        }
        if !self.nodes.get(target).map(|n| n.data.is_transform()).unwrap_or(false) {
            return Err("Only transforms can follow a path".to_string());
        }
        let data = crate::dag::nodes::MotionPathData { curve: Some(curve_shape), curve_name: String::new() };
        Ok(self.create_node(NodeData::MotionPath(Box::new(data)), "motionPath", Some(target), true))
    }

    /// Place every object with a motion path on its curve (and turn it to follow the direction).
    pub fn solve_motion_paths(&mut self) {
        let ids = self.ids_where(|d| matches!(d, NodeData::MotionPath(_)));
        for id in ids {
            let (curve_id, target) = match self.nodes.get(id) {
                Some(n) => match (&n.data, n.parent) {
                    (NodeData::MotionPath(d), Some(p)) => (d.curve, p),
                    _ => continue,
                },
                None => continue,
            };
            let Some(cs) = curve_id else { continue };
            let Some((curve, _)) = self.input_curve_world(cs) else { continue };
            let g = |s: &mut Scene, a: &str, d: f32| s.get_attr(id, a).unwrap_or(d);
            let (a, b) = curve.dir.domain();
            let u = g(self, "uValue", 0.0);
            let param = if g(self, "fractionMode", 0.0) > 0.5 { a + (b - a) * u } else { u };
            let param = if curve.dir.periodic { param } else { param.clamp(a, b) };
            let pos = curve.point(param);
            let pw = match self.nodes.get(target).and_then(|n| n.parent) {
                Some(p) => self.world_matrix(p),
                None => Mat4::identity(),
            };
            let mut world = Mat4::translation(pos);
            let mut rotate_too = false;
            if g(self, "follow", 0.0) > 0.5 {
                rotate_too = true;
                let axis_of = |i: f32, inv: bool| -> Vec3 {
                    let v = match i.round() as i32 {
                        0 => Vec3::new(1.0, 0.0, 0.0),
                        1 => Vec3::new(0.0, 1.0, 0.0),
                        _ => Vec3::new(0.0, 0.0, 1.0),
                    };
                    if inv {
                        -v
                    } else {
                        v
                    }
                };
                let (lf, lu) = (axis_of(g(self, "frontAxis", 0.0), g(self, "inverseFront", 0.0) > 0.5), axis_of(g(self, "upAxis", 1.0), g(self, "inverseUp", 0.0) > 0.5));
                if lf.dot(lu).abs() < 0.99 {
                    let f = curve.tangent(param).normalized();
                    // banking leans the up vector into the turn: the horizontal change of direction ahead
                    let h = (b - a) * 0.01;
                    let (t0, t1) = (curve.tangent(param - h), curve.tangent(param + h));
                    let turn = t0.cross(t1).y;
                    let bank_deg = if g(self, "bank", 0.0) > 0.5 { (turn * 57.3 * g(self, "bankScale", 1.0) * 10.0).clamp(-g(self, "bankLimit", 90.0), g(self, "bankLimit", 90.0)) } else { 0.0 };
                    let up_ref = Vec3::UP;
                    let pick = |v: Vec3, along: Vec3| -> Vec3 {
                        let p = v - along * along.dot(v);
                        if p.length_sq() > 1e-10 {
                            p.normalized()
                        } else {
                            let alt = if along.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
                            (alt - along * along.dot(alt)).normalized()
                        }
                    };
                    let mut wu = pick(up_ref, f);
                    if bank_deg.abs() > 1e-4 {
                        let q = Quat::from_axis_angle(f, (-bank_deg).to_radians());
                        wu = q.rotate(wu);
                    }
                    let wc = f.cross(wu);
                    let lu = {
                        let p = lu - lf * lf.dot(lu);
                        p.normalized()
                    };
                    let lc = lf.cross(lu);
                    let mut r = basis(f, wu, wc) * basis(lf, lu, lc).transpose();
                    // twists about the front, up and side axes
                    for (attr, ax) in [("frontTwist", lf), ("upTwist", lu), ("sideTwist", lc)] {
                        let deg = g(self, attr, 0.0);
                        if deg.abs() > 1e-6 {
                            r = r * Mat4::from_quat(Quat::from_axis_angle(ax, deg.to_radians()));
                        }
                    }
                    world = Mat4::translation(pos) * r;
                } else {
                    rotate_too = false;
                }
            }
            let (lt, lr, _) = self.trs_for_local(target, &(pw.inverse() * world));
            for (a, x) in ["X", "Y", "Z"].iter().zip([lt.x, lt.y, lt.z]) {
                self.set_eval(target, &format!("translate{}", a), x);
            }
            if rotate_too {
                for (a, x) in ["X", "Y", "Z"].iter().zip([lr.x, lr.y, lr.z]) {
                    self.set_eval(target, &format!("rotate{}", a), x);
                }
            }
        }
    }

    fn vec3_attr(&mut self, id: NodeId, base: &str) -> Vec3 {
        Vec3::new(
            self.get_attr(id, &format!("{}X", base)).unwrap_or(0.0),
            self.get_attr(id, &format!("{}Y", base)).unwrap_or(0.0),
            self.get_attr(id, &format!("{}Z", base)).unwrap_or(0.0),
        )
    }

    /// Normalized (target, weight, index) triples of the targets that still exist.
    fn constraint_targets(&mut self, id: NodeId, targets: &[NodeId]) -> Vec<(NodeId, f32, usize)> {
        let mut v: Vec<(NodeId, f32, usize)> = Vec::new();
        for (i, t) in targets.iter().enumerate().take(MAX_CONSTRAINT_TARGETS) {
            if self.nodes.get(*t).is_none() {
                continue;
            }
            let w = self.get_attr(id, &format!("w{}", i)).unwrap_or(0.0).max(0.0);
            if w > 0.0 {
                v.push((*t, w, i));
            }
        }
        let sum: f32 = v.iter().map(|x| x.1).sum();
        if sum > 0.0 {
            for x in v.iter_mut() {
                x.1 /= sum;
            }
        }
        v
    }

    /// Evaluate every constraint, after animation. Results are written as evaluated values, so the
    /// authored translate / rotate / scale values of the driven object are untouched.
    pub fn solve_constraints(&mut self) {
        let ids = self.ids_where(|d| matches!(d, NodeData::Constraint(_)));
        for id in ids {
            let (kind, targets, offsets, driven) = match self.nodes.get(id) {
                Some(n) => match (&n.data, n.parent) {
                    (NodeData::Constraint(c), Some(p)) => (c.kind, c.targets.clone(), c.offsets.clone(), p),
                    _ => continue,
                },
                None => continue,
            };
            if self.get_attr(id, "enable").unwrap_or(1.0) < 0.5 {
                continue;
            }
            let tg = self.constraint_targets(id, &targets);
            if tg.is_empty() {
                continue;
            }
            let dworld = self.world_matrix(driven);
            let (wt, _wr, ws) = dworld.decompose();
            let dq = quat_of_world(&dworld);
            let tm: Vec<(Mat4, f32, usize)> = tg.iter().map(|(t, w, i)| (self.world_matrix(*t), *w, *i)).collect();
            let mut avg_pos = Vec3::ZERO;
            for (m, w, _) in &tm {
                avg_pos += m.translation_part() * *w;
            }
            let off = self.vec3_attr(id, "offset");
            let (new_world, channels): (Mat4, [bool; 3]) = match kind {
                ConstraintKind::Point => (Mat4::trs(avg_pos + off, dq, ws), [true, false, false]),
                ConstraintKind::Orient => {
                    let qs: Vec<(Quat, f32)> = tm.iter().map(|(m, w, _)| (quat_of_world(m), *w)).collect();
                    let q = blend_quats(&qs) * Quat::from_euler_deg(off);
                    (Mat4::trs(wt, q.normalized(), ws), [false, true, false])
                }
                ConstraintKind::Scale => {
                    let mut s = Vec3::ZERO;
                    for (m, w, _) in &tm {
                        s += m.decompose().2 * *w;
                    }
                    (Mat4::trs(wt, dq, s.mul_elem(off)), [false, false, true])
                }
                ConstraintKind::Aim | ConstraintKind::Normal => {
                    let aim = self.vec3_attr(id, "aim").normalized();
                    let up = self.vec3_attr(id, "up").normalized();
                    let wut = self.get_attr(id, "worldUpType").unwrap_or(2.0).round() as i32;
                    // with object up the second target is the up object, the first one is aimed at
                    let object_up = wut == 1 && tm.len() >= 2;
                    let aim_pos = if object_up { tm[0].0.translation_part() } else { avg_pos };
                    let f = if kind == ConstraintKind::Normal {
                        // the surface normal at the point of the surface closest to the object
                        match self.surface_normal_at(tg[0].0, wt) {
                            Some(n) => n,
                            None => continue,
                        }
                    } else {
                        (aim_pos - wt).normalized()
                    };
                    let wup = match wut {
                        0 => Vec3::UP,
                        1 if object_up => (tm[1].0.translation_part() - wt).normalized(),
                        3 => up_in_world(&dworld, up),
                        _ => self.vec3_attr(id, "worldUp").normalized(),
                    };
                    if f.length_sq() < 1e-12 || aim.length_sq() < 1e-12 {
                        continue;
                    }
                    let pick = |v: Vec3, along: Vec3| -> Vec3 {
                        let p = v - along * along.dot(v);
                        if p.length_sq() > 1e-10 {
                            p.normalized()
                        } else {
                            let alt = if along.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
                            (alt - along * along.dot(alt)).normalized()
                        }
                    };
                    // local frame (aim, up, side) mapped onto the world frame (to target, world up, side)
                    let lu = pick(up, aim);
                    let lc = aim.cross(lu);
                    let wu = pick(wup, f);
                    let wc = f.cross(wu);
                    let r = basis(f, wu, wc) * basis(aim, lu, lc).transpose();
                    (Mat4::translation(wt) * r * Mat4::scaling(ws), [false, true, false])
                }
                ConstraintKind::Parent => {
                    let mut ts = Vec3::ZERO;
                    let mut ss = Vec3::ZERO;
                    let mut qs: Vec<(Quat, f32)> = Vec::new();
                    for (m, w, i) in &tm {
                        let o = offsets.get(*i).copied().unwrap_or_else(Mat4::identity);
                        let c = *m * o;
                        let (t, _, s) = c.decompose();
                        ts += t * *w;
                        ss += s * *w;
                        qs.push((quat_of_world(&c), *w));
                    }
                    (Mat4::trs(ts, blend_quats(&qs), ss), [true, true, true])
                }
            };
            let pw = match self.nodes.get(driven).and_then(|n| n.parent) {
                Some(p) => self.world_matrix(p),
                None => Mat4::identity(),
            };
            let (lt, lr, ls) = self.trs_for_local(driven, &(pw.inverse() * new_world));
            let sets: [(&str, Vec3, bool); 3] = [("translate", lt, channels[0]), ("rotate", lr, channels[1]), ("scale", ls, channels[2])];
            for (base, v, on) in sets {
                if !on {
                    continue;
                }
                for (a, x) in ["X", "Y", "Z"].iter().zip([v.x, v.y, v.z]) {
                    self.set_eval(driven, &format!("{}{}", base, a), x);
                }
            }
        }
    }

    /// Constrain `driven` to `targets`. With `maintain_offset` the object stays where it is now.
    pub fn create_constraint(&mut self, kind: ConstraintKind, targets: &[NodeId], driven: NodeId, maintain_offset: bool) -> Result<NodeId, String> {
        if targets.is_empty() {
            return Err("Select the target objects first and the object to constrain last".to_string());
        }
        if targets.len() > MAX_CONSTRAINT_TARGETS {
            return Err(format!("A constraint can have at most {} targets", MAX_CONSTRAINT_TARGETS));
        }
        if !self.nodes.get(driven).map(|n| n.data.is_transform()).unwrap_or(false) {
            return Err("The constrained object must be a transform".to_string());
        }
        for t in targets {
            if !self.nodes.get(*t).map(|n| n.data.is_transform()).unwrap_or(false) {
                return Err("Targets must be transforms".to_string());
            }
            if *t == driven || self.descendants(driven).contains(t) {
                return Err("An object cannot be constrained to itself or to its own children".to_string());
            }
        }
        let dworld = self.world_matrix(driven);
        let tm: Vec<Mat4> = targets.iter().map(|t| self.world_matrix(*t)).collect();
        let mut offsets: Vec<Mat4> = Vec::new();
        let mut off = Vec3::ZERO;
        match kind {
            ConstraintKind::Point if maintain_offset => {
                let avg = tm.iter().fold(Vec3::ZERO, |a, m| a + m.translation_part()) * (1.0 / tm.len() as f32);
                off = dworld.translation_part() - avg;
            }
            ConstraintKind::Orient if maintain_offset => {
                let qt = blend_quats(&tm.iter().map(|m| (quat_of_world(m), 1.0)).collect::<Vec<_>>());
                off = quat_to_euler(qt.conjugate() * quat_of_world(&dworld));
            }
            ConstraintKind::Scale => {
                off = if maintain_offset {
                    let avg = tm.iter().fold(Vec3::ZERO, |a, m| a + m.decompose().2) * (1.0 / tm.len() as f32);
                    let d = dworld.decompose().2;
                    Vec3::new(d.x / avg.x.max(1e-6), d.y / avg.y.max(1e-6), d.z / avg.z.max(1e-6))
                } else {
                    Vec3::ONE
                };
            }
            ConstraintKind::Parent => {
                for m in &tm {
                    offsets.push(if maintain_offset { m.inverse() * dworld } else { Mat4::identity() });
                }
            }
            _ => {}
        }
        let base = format!("{}_{}Constraint", self.name_of(driven), kind.name());
        let data = ConstraintData { kind, targets: targets.to_vec(), target_names: Vec::new(), offsets };
        let id = self.create_node(NodeData::Constraint(Box::new(data)), &base, Some(driven), true);
        for (a, v) in [("offsetX", off.x), ("offsetY", off.y), ("offsetZ", off.z)] {
            self.set_attr(id, a, v)?;
        }
        self.evaluate_frame();
        Ok(id)
    }

    /// The Set Driven Key curve node that drives `driven.attr`, if any.
    pub fn driven_curve_of(&self, driven: NodeId, attr: &str) -> Option<(NodeId, PlugRef)> {
        let src = self.nodes.get(driven)?.plug(attr)?.source.clone()?;
        match self.nodes.get(src.node).map(|n| &n.data) {
            Some(NodeData::DrivenCurve(_)) => {
                let input = self.nodes.get(src.node)?.plug("input")?.source.clone()?;
                Some((src.node, input))
            }
            _ => None,
        }
    }

    /// Set Driven Key: when `driver` reads `driver_value`, `driven` should read `value`.
    pub fn set_driven_key(&mut self, driver: (NodeId, String), driver_value: f32, driven: (NodeId, String), value: f32) -> Result<NodeId, String> {
        let dattr = super::plugs::canonical_attr(&driven.1);
        let rattr = super::plugs::canonical_attr(&driver.1);
        let curve_node = match self.driven_curve_of(driven.0, &dattr) {
            Some((c, input)) if input.node == driver.0 && input.attr == rattr => c,
            Some(_) => return Err(format!("'{}.{}' is already driven by something else", self.name_of(driven.0), dattr)),
            None => {
                let name = format!("{}_{}", self.name_of(driven.0), dattr);
                let c = self.create_node(NodeData::DrivenCurve(Box::default()), &name, None, false);
                self.connect((driver.0, rattr.clone()), (c, "input".to_string()), true)?;
                if let Err(e) = self.connect((c, "output".to_string()), (driven.0, dattr.clone()), false) {
                    let _ = self.delete_node(c);
                    return Err(e);
                }
                c
            }
        };
        if let Some(NodeData::DrivenCurve(cv)) = self.nodes.get_mut(curve_node).map(|n| &mut n.data) {
            cv.set_key(driver_value, value, Interp::Bezier);
        }
        self.mark_driven_dirty(curve_node);
        Ok(curve_node)
    }

    /// Force everything downstream of a curve node to re-read it.
    pub fn mark_driven_dirty(&mut self, curve_node: NodeId) {
        if let Some(p) = self.nodes.get_mut(curve_node).and_then(|n| n.plug_mut("output")) {
            p.dirty = true;
        }
        self.propagate(curve_node, "output");
    }
}

/// Evaluate a Set Driven Key curve: clamped at its ends, spline or linear between keys.
pub fn eval_driven(c: &Curve, x: f32) -> f32 {
    c.eval(x).unwrap_or(0.0)
}


/// The up vector of an object in world space (used when the world up is "none": the up stays as it is).
fn up_in_world(world: &Mat4, up: Vec3) -> Vec3 {
    world.transform_dir(up).normalized()
}

impl Scene {
    /// The interpolated normal of the surface (a polygon object or its shapes) at the point closest to `p`, in world space.
    pub fn surface_normal_at(&mut self, surface: NodeId, p: Vec3) -> Option<Vec3> {
        let mut best: Option<(f32, Vec3)> = None;
        for sh in self.mesh_shapes_under(surface) {
            let owner = self.nodes.get(sh).and_then(|n| n.parent).unwrap_or(sh);
            let w = self.world_matrix(owner);
            let Some(NodeData::Mesh(md)) = self.nodes.get(sh).map(|n| &n.data) else { continue };
            for t in md.mesh.triangulate() {
                let (a, b, c) = (w.transform_point(md.mesh.positions[t[0] as usize]), w.transform_point(md.mesh.positions[t[1] as usize]), w.transform_point(md.mesh.positions[t[2] as usize]));
                let cp = crate::nucleus::rigid::closest_point_triangle(p, a, b, c);
                let d = (cp - p).length_sq();
                if best.map(|x| d < x.0).unwrap_or(true) {
                    // barycentric weights of the closest point
                    let (v0, v1, v2) = (b - a, c - a, cp - a);
                    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
                    let den = d00 * d11 - d01 * d01;
                    let (u, v) = if den.abs() > 1e-12 { ((d11 * d20 - d01 * d21) / den, (d00 * d21 - d01 * d20) / den) } else { (0.0, 0.0) };
                    let n = |i: u32| md.mesh.normals.get(i as usize).copied().unwrap_or(Vec3::UP);
                    let smooth = w.transform_dir(n(t[0]) * (1.0 - u - v) + n(t[1]) * u + n(t[2]) * v).normalized();
                    let flat = (b - a).cross(c - a).normalized();
                    let nrm = if smooth.length_sq() > 0.5 { smooth } else { flat };
                    best = Some((d, nrm));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// The first hit of a world space ray on the mesh shapes of `surface`.
    pub fn ray_hit_surface(&mut self, surface: NodeId, origin: Vec3, dir: Vec3) -> Option<Vec3> {
        let mut best: Option<f32> = None;
        for sh in self.mesh_shapes_under(surface) {
            let owner = self.nodes.get(sh).and_then(|n| n.parent).unwrap_or(sh);
            let w = self.world_matrix(owner);
            let Some(NodeData::Mesh(md)) = self.nodes.get(sh).map(|n| &n.data) else { continue };
            for t in md.mesh.triangulate() {
                let (a, b, c) = (w.transform_point(md.mesh.positions[t[0] as usize]), w.transform_point(md.mesh.positions[t[1] as usize]), w.transform_point(md.mesh.positions[t[2] as usize]));
                let (e1, e2) = (b - a, c - a);
                let pv = dir.cross(e2);
                let det = e1.dot(pv);
                if det.abs() < 1e-12 {
                    continue;
                }
                let iv = 1.0 / det;
                let tv = origin - a;
                let u = tv.dot(pv) * iv;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let qv = tv.cross(e1);
                let v = dir.dot(qv) * iv;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let tt = e2.dot(qv) * iv;
                if tt > 1e-4 && best.map(|x| tt < x).unwrap_or(true) {
                    best = Some(tt);
                }
            }
        }
        best.map(|t| origin + dir * t)
    }

    /// Ray cast down (or along `dir`) onto the mesh shapes of `surface`: the first hit, else the closest point.
    pub fn snap_to_surface(&mut self, surface: NodeId, p: Vec3, dir: Vec3) -> Option<Vec3> {
        let mut best: Option<(f32, Vec3)> = None;
        let mut closest: Option<(f32, Vec3)> = None;
        let origin = p - dir * 1000.0;
        for sh in self.mesh_shapes_under(surface) {
            let owner = self.nodes.get(sh).and_then(|n| n.parent).unwrap_or(sh);
            let w = self.world_matrix(owner);
            let Some(NodeData::Mesh(md)) = self.nodes.get(sh).map(|n| &n.data) else { continue };
            for t in md.mesh.triangulate() {
                let (a, b, c) = (w.transform_point(md.mesh.positions[t[0] as usize]), w.transform_point(md.mesh.positions[t[1] as usize]), w.transform_point(md.mesh.positions[t[2] as usize]));
                let (e1, e2) = (b - a, c - a);
                let pv = dir.cross(e2);
                let det = e1.dot(pv);
                if det.abs() > 1e-12 {
                    let iv = 1.0 / det;
                    let tv = origin - a;
                    let u = tv.dot(pv) * iv;
                    let qv = tv.cross(e1);
                    let v = dir.dot(qv) * iv;
                    if (0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 {
                        let tt = e2.dot(qv) * iv;
                        if tt > 0.0 && best.map(|x| (tt - 1000.0).abs() < (x.0 - 1000.0).abs()).unwrap_or(true) {
                            best = Some((tt, origin + dir * tt));
                        }
                    }
                }
                let cp = crate::nucleus::rigid::closest_point_triangle(p, a, b, c);
                let d = (cp - p).length_sq();
                if closest.map(|x| d < x.0).unwrap_or(true) {
                    closest = Some((d, cp));
                }
            }
        }
        best.map(|b| b.1).or(closest.map(|c| c.1))
    }
}
