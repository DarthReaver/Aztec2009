//! Dynamics: rigid bodies, fields, particles and emitters evaluated frame by frame.
//!
//! The simulation runs from the start frame to the current frame (frame by frame, through the animation of passive
//! bodies and of the objects particles collide with). Any change of a dynamics setting starts it again; with the
//! rigid body cache on, positions of the finished frames are kept until the cache is deleted.

use super::graph::{NodeId, Scene};
use super::nodes::*;
use crate::math::{Mat4, Quat, Vec3};
use crate::nucleus::fields::{self, Collider, Field, PSim};
use crate::nucleus::particles::Rng;
use crate::nucleus::rigid::{Body, Joint, JointKind, Shape, TriMesh, World};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// How a rigid body node maps onto the simulated body.
#[derive(Clone, Debug)]
pub struct Bind {
    pub transform: NodeId,
    pub body_node: NodeId,
    /// Center of mass in the scaled local frame of the object.
    pub com_q: Vec3,
    pub scale: Vec3,
}

#[derive(Clone, Debug, Default)]
pub struct DynRuntime {
    /// Frame the rigid bodies were last simulated to.
    pub last_frame: i32,
    pub started: bool,
    pub sig: u64,
    pub sig_rigid: u64,
    pub world: Option<World>,
    pub binds: Vec<Bind>,
    pub pose_prev: Vec<(Vec3, Quat)>,
    pub pose_cur: Vec<(Vec3, Quat)>,
    pub cache: BTreeMap<i32, Vec<BodyState>>,
}

/// Rigid, particle and field names for the creation commands.
pub const JOINT_KINDS: &[&str] = &["nail", "pin", "hinge", "spring", "barrier"];

fn hash_f(h: &mut std::collections::hash_map::DefaultHasher, v: f32) {
    v.to_bits().hash(h);
}

impl Scene {
    // ------------------------------------------------------------------ creation

    pub fn ensure_dyn_solver(&mut self) -> NodeId {
        if let Some(id) = self.ids_where(|d| matches!(d, NodeData::DynSolver(_))).first() {
            return *id;
        }
        self.create_node(NodeData::DynSolver(Box::default()), "rigidSolver", None, false)
    }

    /// The transform that owns a rigid body node, or the transform given.
    pub fn rigid_body_of(&self, transform: NodeId) -> Option<NodeId> {
        self.nodes.get(transform)?.children.iter().copied().find(|c| matches!(self.nodes.get(*c).map(|n| &n.data), Some(NodeData::RigidBody(_))))
    }

    /// Create Active / Passive Rigid Body on a transform with a mesh.
    pub fn create_rigid_body(&mut self, transform: NodeId, active: bool) -> Result<NodeId, String> {
        if !matches!(self.nodes.get(transform).map(|n| &n.data), Some(NodeData::Transform)) {
            return Err("Select a polygon or NURBS object".to_string());
        }
        if self.rigid_body_of(transform).is_some() {
            return Err(format!("{} is already a rigid body", self.name_of(transform)));
        }
        let shapes = self.mesh_shapes_under(transform);
        if shapes.is_empty() {
            return Err(format!("{} has no polygon or NURBS surface", self.name_of(transform)));
        }
        for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ"] {
            if self.nodes.get(transform).and_then(|n| n.plug(a)).map(|p| p.locked).unwrap_or(false) {
                return Err("All translate and rotate attributes must be unlocked to make a rigid body".to_string());
            }
        }
        self.ensure_dyn_solver();
        let prim_sphere = matches!(self.nodes.get(shapes[0]).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.prim == "polySphere" || md.history.first().and_then(|h| self.nodes.get(*h)).map(|n| matches!(&n.data, NodeData::PolyOp(op) if op.kind == PolyKind::Sphere)).unwrap_or(false));
        let faces: usize = shapes.iter().map(|s| match self.nodes.get(*s).map(|n| &n.data) { Some(NodeData::Mesh(md)) => md.mesh.faces.len(), _ => 0 }).sum();
        let rb = self.create_node(NodeData::RigidBody(Box::default()), "rigidBody", Some(transform), true);
        self.set_attr(rb, "active", if active { 1.0 } else { 0.0 })?;
        let shape = if prim_sphere { 1.0 } else if !active && faces > 12 { 2.0 } else { 0.0 };
        self.set_attr(rb, "shape", shape)?;
        Ok(rb)
    }

    /// Delete the rigid body node of a transform.
    pub fn remove_rigid_body(&mut self, transform: NodeId) -> Result<(), String> {
        let rb = self.rigid_body_of(transform).ok_or("Not a rigid body")?;
        self.delete_node(rb)
    }

    /// Field and dynamic target normalization: a transform with a rigid body, or a particle shape.
    pub fn dyn_target_of(&self, id: NodeId) -> Option<NodeId> {
        match self.nodes.get(id).map(|n| &n.data) {
            Some(NodeData::Particle(_)) => Some(id),
            Some(NodeData::Transform) => {
                if self.rigid_body_of(id).is_some() {
                    return Some(id);
                }
                self.nodes.get(id)?.children.iter().copied().find(|c| matches!(self.nodes.get(*c).map(|n| &n.data), Some(NodeData::Particle(_))))
            }
            _ => None,
        }
    }

    /// Create a field. Returns (transform, field node).
    pub fn create_field(&mut self, kind: u8, name: Option<&str>) -> (NodeId, NodeId) {
        let base = format!("{}Field", fields::FIELD_KINDS[kind as usize]);
        let t = match name {
            Some(n) => self.create_node(NodeData::Transform, n, None, false),
            None => self.create_node(NodeData::Transform, &base, None, true),
        };
        let tn = self.name_of(t);
        let f = self.create_node(NodeData::Field(Box::new(FieldData { kind, ..Default::default() })), &format!("{}Shape", tn), Some(t), false);
        let (mag, att, dir): (f32, f32, [f32; 3]) = match kind {
            0 => (9.8, 0.0, [0.0, -1.0, 0.0]),
            1 => (1.0, 0.0, [1.0, 0.0, 0.0]),
            2 => (1.0, 0.0, [0.0, 1.0, 0.0]),
            3 => (5.0, 1.0, [0.0, 1.0, 0.0]),
            4 => (5.0, 1.0, [0.0, 1.0, 0.0]),
            5 => (10.0, 0.0, [0.0, 1.0, 0.0]),
            6 => (10.0, 0.0, [0.0, 1.0, 0.0]),
            _ => (5.0, 1.0, [0.0, 1.0, 0.0]),
        };
        let _ = self.set_attr(f, "magnitude", mag);
        let _ = self.set_attr(f, "attenuation", att);
        for (a, v) in ["directionX", "directionY", "directionZ"].iter().zip(dir) {
            let _ = self.set_attr(f, a, v);
        }
        (t, f)
    }

    /// Fields > Affect Selected Object(s) (`connect`) or the reverse.
    pub fn connect_field(&mut self, field: NodeId, targets: &[NodeId], connect: bool) -> Result<usize, String> {
        let norm: Vec<NodeId> = targets.iter().filter_map(|t| self.dyn_target_of(*t)).collect();
        if norm.is_empty() {
            return Err("Select rigid bodies or particles".to_string());
        }
        let n = match self.nodes.get_mut(field).map(|n| &mut n.data) {
            Some(NodeData::Field(fd)) => {
                for t in &norm {
                    if connect {
                        if !fd.targets.contains(t) {
                            fd.targets.push(*t);
                        }
                    } else {
                        fd.targets.retain(|x| x != t);
                    }
                }
                fd.targets.len()
            }
            _ => return Err("Not a field".to_string()),
        };
        Ok(n)
    }

    /// Create a particle object. Returns (transform, particle node).
    pub fn create_particle_object(&mut self, name: Option<&str>) -> (NodeId, NodeId) {
        self.ensure_dyn_solver();
        let t = match name {
            Some(n) => self.create_node(NodeData::Transform, n, None, false),
            None => self.create_node(NodeData::Transform, "particle", None, true),
        };
        let tn = self.name_of(t);
        let p = self.create_node(NodeData::Particle(Box::default()), &format!("{}Shape", tn), Some(t), false);
        (t, p)
    }

    /// Create an emitter that fills `target`. `source` is the object whose surface or curve it emits from.
    pub fn create_particle_emitter(&mut self, target: NodeId, source: Option<NodeId>, emitter_type: u8, name: Option<&str>) -> (NodeId, NodeId) {
        let t = match name {
            Some(n) => self.create_node(NodeData::Transform, n, None, false),
            None => self.create_node(NodeData::Transform, "emitter", None, true),
        };
        let tn = self.name_of(t);
        let e = self.create_node(NodeData::PEmitter(Box::new(PEmitterData { target: Some(target), source, ..Default::default() })), &format!("{}Shape", tn), Some(t), false);
        let _ = self.set_attr(e, "emitterType", emitter_type as f32);
        (t, e)
    }

    // ------------------------------------------------------------------ the evaluation

    fn dyn_solver_id(&self) -> Option<NodeId> {
        self.ids_where(|d| matches!(d, NodeData::DynSolver(_))).first().copied()
    }

    fn plug_of(&self, id: NodeId, k: &str, d: f32) -> f32 {
        self.nodes.get(id).and_then(|n| n.plug(k)).map(|p| p.value).unwrap_or(d)
    }

    /// Settings that the simulation depends on, as a hash. Rigid and particle parts are hashed apart.
    fn dyn_signatures(&mut self) -> (u64, u64) {
        let mut hr = std::collections::hash_map::DefaultHasher::new();
        let mut hp = std::collections::hash_map::DefaultHasher::new();
        let fps = self.timeline.fps;
        hash_f(&mut hr, fps);
        hash_f(&mut hp, fps);
        for id in self.ids_where(|d| matches!(d, NodeData::DynSolver(_) | NodeData::RigidBody(_) | NodeData::RigidJoint(_))) {
            let Some(n) = self.nodes.get(id) else { continue };
            id.hash(&mut hr);
            for p in &n.plugs {
                if p.name != "cachedFrames" && p.name != "cacheData" {
                    hash_f(&mut hr, p.value);
                }
            }
            if let NodeData::RigidBody(rb) = &n.data {
                if let Some(s) = &rb.initial {
                    for v in [s.pos.x, s.pos.y, s.pos.z, s.rot.x, s.rot.y, s.rot.z, s.rot.w, s.vel.x, s.vel.y, s.vel.z] {
                        hash_f(&mut hr, v);
                    }
                }
                // the object itself: its authored transform and its geometry
                if let Some(t) = n.parent {
                    if let Some(tn) = self.nodes.get(t) {
                        for p in &tn.plugs {
                            hash_f(&mut hr, p.rest);
                        }
                    }
                    for sh in self.mesh_shapes_under(t) {
                        if let Some(NodeData::Mesh(md)) = self.nodes.get(sh).map(|n| &n.data) {
                            md.cache.version.hash(&mut hr);
                            md.mesh.positions.len().hash(&mut hr);
                        }
                    }
                }
            }
            if let NodeData::RigidJoint(j) = &n.data {
                j.a.hash(&mut hr);
                j.b.hash(&mut hr);
            }
        }
        for id in self.ids_where(|d| matches!(d, NodeData::Field(_) | NodeData::PEmitter(_) | NodeData::Particle(_))) {
            let Some(n) = self.nodes.get(id) else { continue };
            id.hash(&mut hp);
            for p in &n.plugs {
                if p.name != "particleCount" {
                    hash_f(&mut hp, p.value);
                }
            }
            match &n.data {
                NodeData::Field(f) => f.targets.hash(&mut hp),
                NodeData::PEmitter(e) => {
                    e.target.hash(&mut hp);
                    e.source.hash(&mut hp);
                }
                NodeData::Particle(pd) => {
                    for c in &pd.colliders {
                        c.node.hash(&mut hp);
                        hash_f(&mut hp, c.resilience);
                        hash_f(&mut hp, c.friction);
                    }
                    for e in &pd.events {
                        (e.kind, e.num, e.random, e.target, e.original_dies).hash(&mut hp);
                        hash_f(&mut hp, e.spread);
                        hash_f(&mut hp, e.inherit);
                    }
                    pd.initial.as_ref().map(|i| (i.items.len(), i.next_id)).hash(&mut hp);
                }
                _ => {}
            }
            // transforms of fields and emitters
            if matches!(n.data, NodeData::Field(_) | NodeData::PEmitter(_)) {
                if let Some(t) = n.parent.and_then(|t| self.nodes.get(t)) {
                    for p in &t.plugs {
                        hash_f(&mut hp, p.rest);
                    }
                }
            }
        }
        (hr.finish(), hp.finish())
    }

    fn evaluate_non_dynamic(&mut self) {
        self.evaluate_anim();
        self.solve_motion_paths();
        self.solve_distances();
        self.mark_jiggles_dirty();
        self.solve_iks();
        self.solve_constraints();
    }

    fn pose_of(&mut self, bind: &Bind) -> (Vec3, Quat) {
        let w = self.world_matrix(bind.transform);
        let (t, r, _) = w.decompose();
        let q = Quat::from_euler_deg(r);
        (t + q.rotate(bind.com_q), q)
    }

    /// Build the simulated bodies from the rigid body nodes at the current poses.
    fn dyn_build_world(&mut self, solver: NodeId) {
        let mut world = World::new();
        world.tolerance = self.plug_of(solver, "collisionTolerance", 0.02).max(1e-4);
        world.iterations = self.plug_of(solver, "contactIterations", 10.0).clamp(1.0, 50.0) as usize;
        let mut binds = Vec::new();
        let mut poses = Vec::new();
        for rb in self.ids_where(|d| matches!(d, NodeData::RigidBody(_))) {
            let Some(t) = self.nodes.get(rb).and_then(|n| n.parent) else { continue };
            let Some((body, bind)) = self.dyn_build_body(rb, t) else { continue };
            poses.push((body.pos, body.rot));
            world.bodies.push(body);
            binds.push(bind);
        }
        // constraints
        for j in self.ids_where(|d| matches!(d, NodeData::RigidJoint(_))) {
            let Some(NodeData::RigidJoint(jd)) = self.nodes.get(j).map(|n| n.data.clone()) else { continue };
            let Some(tr) = self.nodes.get(j).and_then(|n| n.parent) else { continue };
            let w = self.world_matrix(tr);
            let anchor = w.translation_part();
            let index_of = |n: Option<NodeId>| n.and_then(|n| binds.iter().position(|b| b.transform == n));
            let (ia, ib) = (index_of(jd.a), index_of(jd.b));
            if jd.kind == 4 {
                world.barriers.push((anchor, w.transform_dir(Vec3::UP).normalized()));
                continue;
            }
            let (a, b) = match (ia, ib) {
                (Some(a), b) => (a, b),
                (None, Some(b)) => (b, None),
                (None, None) => continue,
            };
            let kind = match jd.kind {
                2 => JointKind::Hinge,
                3 => JointKind::Spring,
                _ => JointKind::Point,
            };
            let axis_w = w.transform_dir(Vec3::new(0.0, 0.0, 1.0)).normalized();
            let ba = &world.bodies[a];
            let local_a = ba.to_local(anchor);
            let (local_b, axis_b) = match b {
                Some(b) => (world.bodies[b].to_local(anchor), world.bodies[b].rot.conjugate().rotate(axis_w)),
                None => (Vec3::ZERO, axis_w),
            };
            let stiff = self.plug_of(j, "stiffness", 10.0);
            let damp = self.plug_of(j, "damping", 0.5);
            let rest = self.plug_of(j, "restLength", 0.0);
            world.joints.push(Joint { kind, a, b, local_a, local_b, world: anchor, axis_a: ba.rot.conjugate().rotate(axis_w), axis_b, stiffness: stiff * 100.0, damping: damp * 10.0, rest_length: rest });
        }
        let rt = self.dyn_rt_mut(solver);
        rt.pose_prev = poses.clone();
        rt.pose_cur = poses;
        rt.world = Some(world);
        rt.binds = binds;
    }

    fn dyn_rt_mut(&mut self, solver: NodeId) -> &mut DynRuntime {
        match self.nodes.get_mut(solver).map(|n| &mut n.data) {
            Some(NodeData::DynSolver(d)) => &mut d.rt,
            _ => unreachable!("the solver node exists"),
        }
    }

    fn dyn_rt(&self, solver: NodeId) -> Option<&DynRuntime> {
        match self.nodes.get(solver).map(|n| &n.data) {
            Some(NodeData::DynSolver(d)) => Some(&d.rt),
            _ => None,
        }
    }

    fn dyn_build_body(&mut self, rb: NodeId, t: NodeId) -> Option<(Body, Bind)> {
        let g = |s: &Scene, k: &str, d: f32| s.plug_of(rb, k, d);
        let active = g(self, "active", 1.0) > 0.5;
        let world = self.world_matrix(t);
        let (tr, rot_e, sc) = world.decompose();
        let rot = Quat::from_euler_deg(rot_e);
        let mut verts: Vec<Vec3> = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        for sh in self.mesh_shapes_under(t) {
            if let Some(NodeData::Mesh(md)) = self.nodes.get(sh).map(|n| &n.data) {
                let base = verts.len() as u32;
                verts.extend(md.mesh.positions.iter().map(|p| p.mul_elem(sc)));
                tris.extend(md.mesh.triangulate().into_iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
            }
        }
        if verts.is_empty() {
            return None;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in &verts {
            lo = Vec3::new(lo.x.min(v.x), lo.y.min(v.y), lo.z.min(v.z));
            hi = Vec3::new(hi.x.max(v.x), hi.y.max(v.y), hi.z.max(v.z));
        }
        let center = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5;
        let com_off = Vec3::new(g(self, "centerOfMassX", 0.0), g(self, "centerOfMassY", 0.0), g(self, "centerOfMassZ", 0.0));
        let com_q = center + com_off.mul_elem(sc);
        let off = center - com_q;
        let shape = match (g(self, "shape", 0.0).round() as i32, active) {
            (1, _) => Shape::Sphere { r: half.x.max(half.y).max(half.z).max(1e-3), off },
            (2, false) => Shape::Mesh(Arc::new(TriMesh::new(verts.iter().map(|v| *v - com_q).collect(), tris))),
            _ => Shape::Box { half: Vec3::new(half.x.max(1e-3), half.y.max(1e-3), half.z.max(1e-3)), off },
        };
        let mut body = Body::new(shape, g(self, "mass", 1.0), active);
        body.bounce = g(self, "bounciness", 0.6).clamp(0.0, 1.0);
        body.damping = g(self, "damping", 0.0).max(0.0);
        body.static_friction = g(self, "staticFriction", 0.2).max(0.0);
        body.dynamic_friction = g(self, "dynamicFriction", 0.2).max(0.0);
        body.collide = g(self, "collisions", 1.0) > 0.5;
        let init = match self.nodes.get(rb).map(|n| &n.data) {
            Some(NodeData::RigidBody(d)) => d.initial.clone(),
            _ => None,
        };
        match init {
            Some(s) => {
                body.pos = s.pos;
                body.rot = s.rot;
                body.vel = s.vel;
                body.spin = s.spin;
            }
            None => {
                body.pos = tr + rot.rotate(com_q);
                body.rot = rot;
                body.vel = Vec3::new(g(self, "initialVelocityX", 0.0), g(self, "initialVelocityY", 0.0), g(self, "initialVelocityZ", 0.0));
                body.spin = Vec3::new(g(self, "initialSpinX", 0.0), g(self, "initialSpinY", 0.0), g(self, "initialSpinZ", 0.0));
            }
        }
        Some((body, Bind { transform: t, body_node: rb, com_q, scale: sc }))
    }

    /// Field list with the objects each affects, in world space.
    fn dyn_fields(&mut self, time: f32) -> Vec<(Field, Vec<NodeId>)> {
        let mut out = Vec::new();
        for id in self.ids_where(|d| matches!(d, NodeData::Field(_))) {
            let Some(NodeData::Field(fd)) = self.nodes.get(id).map(|n| n.data.clone()) else { continue };
            let Some(t) = self.nodes.get(id).and_then(|n| n.parent) else { continue };
            let w = self.world_matrix(t);
            let g = |s: &Scene, k: &str, d: f32| s.plug_of(id, k, d);
            let mut f = Field::new(fd.kind);
            f.magnitude = g(self, "magnitude", 10.0);
            f.attenuation = g(self, "attenuation", 0.0).max(0.0);
            f.dir = w.transform_dir(Vec3::new(g(self, "directionX", 0.0), g(self, "directionY", -1.0), g(self, "directionZ", 0.0))).normalized();
            f.pos = w.translation_part();
            f.max_dist = (g(self, "useMaxDistance", 0.0) > 0.5).then(|| g(self, "maxDistance", 5.0));
            f.volume = g(self, "volumeShape", 0.0).round().clamp(0.0, 4.0) as u8;
            f.inv = w.inverse();
            f.speed = g(self, "speed", 0.0);
            f.frequency = g(self, "frequency", 1.0);
            f.phase = g(self, "phase", 0.0);
            f.time = time;
            out.push((f, fd.targets));
        }
        out
    }

    /// Write the poses of the active bodies to their transforms.
    fn dyn_write_back(&mut self, solver: NodeId) {
        let Some(rt) = self.dyn_rt(solver) else { return };
        let Some(world) = &rt.world else { return };
        let writes: Vec<(NodeId, Vec3, Quat, Vec3, Vec3)> = rt.binds.iter().zip(&world.bodies).filter(|(_, b)| b.active).map(|(bind, b)| (bind.transform, b.pos, b.rot, bind.com_q, bind.scale)).collect();
        for (t, pos, rot, com_q, sc) in writes {
            let new_world = Mat4::trs(pos - rot.rotate(com_q), rot, sc);
            let pw = match self.nodes.get(t).and_then(|n| n.parent) {
                Some(p) => self.world_matrix(p),
                None => Mat4::identity(),
            };
            let (lt, lr, _ls) = self.trs_for_local(t, &(pw.inverse() * new_world));
            for (base, v) in [("translate", lt), ("rotate", lr)] {
                for (a, x) in ["X", "Y", "Z"].iter().zip([v.x, v.y, v.z]) {
                    self.set_eval(t, &format!("{}{}", base, a), x);
                }
            }
        }
    }

    fn dyn_restore_rests(&mut self) {
        for rb in self.ids_where(|d| matches!(d, NodeData::RigidBody(_))) {
            let Some(t) = self.nodes.get(rb).and_then(|n| n.parent) else { continue };
            for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ"] {
                let rest = self.nodes.get(t).and_then(|n| n.plug(a)).map(|p| p.rest);
                if let Some(r) = rest {
                    self.set_eval(t, a, r);
                }
            }
        }
    }

    /// Evaluate the dynamics for the current frame.
    pub fn update_dynamics(&mut self) {
        let has_particles = !self.ids_where(|d| matches!(d, NodeData::Particle(_))).is_empty();
        let Some(solver) = self.dyn_solver_id() else {
            if has_particles {
                let s = self.ensure_dyn_solver();
                let _ = s;
            } else {
                return;
            }
            return self.update_dynamics();
        };
        if self.plug_of(solver, "state", 1.0) < 0.5 {
            return;
        }
        let bodies_exist = !self.ids_where(|d| matches!(d, NodeData::RigidBody(_))).is_empty();
        if !bodies_exist && !has_particles {
            return;
        }
        let fps = self.timeline.fps.max(1.0);
        let frame = self.timeline.frame();
        let cache_on = self.plug_of(solver, "cacheData", 0.0) > 0.5;
        let (sig_r, sig_p) = self.dyn_signatures();
        let rstart = self.plug_of(solver, "startFrame", 1.0).round() as i32;
        let pstarts: Vec<(NodeId, i32)> = self.ids_where(|d| matches!(d, NodeData::Particle(_))).into_iter().map(|p| (p, self.plug_of(p, "startFrame", 1.0).round() as i32)).collect();
        let g0 = pstarts.iter().map(|(_, s)| *s).chain(std::iter::once(rstart)).min().unwrap_or(rstart);
        let (sig_old, sig_r_old, last, started, cached) = {
            let rt = self.dyn_rt_mut(solver);
            (rt.sig, rt.sig_rigid, rt.last_frame, rt.started, !rt.cache.is_empty())
        };
        let rigid_changed = sig_r != sig_r_old && !(cache_on && cached);
        let particles_changed = sig_p != sig_old;
        let mut reset = !started || rigid_changed || particles_changed || frame < last;
        if cache_on && cached && frame < last && !rigid_changed && !particles_changed {
            reset = true;
        }
        if !reset && frame == last {
            return;
        }
        let keep_time = self.timeline.current;
        if reset {
            {
                let rt = self.dyn_rt_mut(solver);
                rt.world = None;
                rt.binds.clear();
                rt.last_frame = g0 - 1;
                rt.started = true;
                rt.sig = sig_p;
                rt.sig_rigid = sig_r;
                if !cache_on || rigid_changed {
                    rt.cache.clear();
                }
            }
            for (p, _) in &pstarts {
                if let Some(NodeData::Particle(pd)) = self.nodes.get_mut(*p).map(|n| &mut n.data) {
                    pd.sim = PSim::default();
                    pd.last_frame = i32::MIN;
                }
            }
            self.dyn_restore_rests();
        }
        let mut f = self.dyn_rt_mut(solver).last_frame + 1;
        if frame < g0 {
            // before the simulation starts: everything sits at its start
            self.timeline.current = keep_time;
            self.evaluate_non_dynamic();
            self.dyn_rt_mut(solver).last_frame = frame;
            return;
        }
        let mut evaluated_elsewhere = false;
        while f <= frame {
            if f != frame {
                self.timeline.current = f as f32;
                if f == g0 {
                    self.dyn_restore_rests();
                }
                self.evaluate_non_dynamic();
                evaluated_elsewhere = true;
            } else if evaluated_elsewhere {
                self.timeline.current = f as f32;
                self.evaluate_non_dynamic();
            } else if f == g0 {
                // the first frame is the current one: put the bodies back to their authored poses first
                self.dyn_restore_rests();
                self.evaluate_non_dynamic();
            }
            self.dyn_frame(solver, f, rstart, &pstarts, cache_on, fps);
            self.dyn_rt_mut(solver).last_frame = f;
            f += 1;
        }
        if evaluated_elsewhere || self.timeline.current != keep_time {
            self.timeline.current = keep_time;
            self.evaluate_non_dynamic();
        }
        self.dyn_write_back(solver);
        let n = self.dyn_rt(solver).map(|r| r.cache.len()).unwrap_or(0);
        self.set_eval(solver, "cachedFrames", n as f32);
        for (p, _) in &pstarts {
            let c = match self.nodes.get(*p).map(|n| &n.data) {
                Some(NodeData::Particle(pd)) => pd.sim.items.len(),
                _ => 0,
            };
            self.set_eval(*p, "particleCount", c as f32);
        }
    }

    /// One frame of rigid bodies and particles.
    fn dyn_frame(&mut self, solver: NodeId, f: i32, rstart: i32, pstarts: &[(NodeId, i32)], cache_on: bool, fps: f32) {
        let dt = 1.0 / fps;
        let fields = self.dyn_fields(f as f32 / fps);
        // ---- rigid bodies
        if f == rstart && self.dyn_rt(solver).map(|r| r.world.is_none()).unwrap_or(false) {
            self.dyn_build_world(solver);
            self.dyn_write_back(solver);
        } else if f > rstart {
            self.dyn_step_rigid(solver, f, dt, cache_on, &fields);
            self.dyn_write_back(solver);
        }
        // ---- particles
        let mut spawns: Vec<(NodeId, Vec<(Vec3, Vec3, f32)>)> = Vec::new();
        for (p, start) in pstarts {
            if f == *start {
                // initial state
                let init = match self.nodes.get(*p).map(|n| &n.data) {
                    Some(NodeData::Particle(pd)) => pd.initial.clone(),
                    _ => None,
                };
                let seed = self.plug_of(*p, "seed", 1.0) as u64;
                if let Some(NodeData::Particle(pd)) = self.nodes.get_mut(*p).map(|n| &mut n.data) {
                    pd.sim = init.unwrap_or_else(|| PSim { rng: Rng::new(seed), ..Default::default() });
                    pd.last_frame = f;
                }
                self.dyn_emitter_baselines(*p);
            } else if f > *start {
                let extra = self.dyn_step_particles(*p, f, dt, fps, &fields, &mut spawns);
                let _ = extra;
            }
        }
        // events and emission into other objects
        for (target, list) in spawns {
            let (life_mode, life, rnd, radius, maxc) = (self.plug_of(target, "lifespanMode", 0.0), self.plug_of(target, "lifespan", 1.0), self.plug_of(target, "lifespanRandom", 0.0), self.plug_of(target, "radius", 0.5), self.plug_of(target, "maxCount", 5000.0) as usize);
            if let Some(NodeData::Particle(pd)) = self.nodes.get_mut(target).map(|n| &mut n.data) {
                for (pos, vel, u) in list {
                    if pd.sim.items.len() >= maxc {
                        break;
                    }
                    let l = match life_mode as i32 {
                        0 => 0.0,
                        1 => life,
                        _ => (life + rnd * (u * 2.0 - 1.0)).max(0.01),
                    };
                    pd.sim.spawn(pos, vel, l, radius);
                }
            }
        }
    }

    fn dyn_emitter_baselines(&mut self, particle: NodeId) {
        for e in self.ids_where(|d| matches!(d, NodeData::PEmitter(_))) {
            let tgt = match self.nodes.get(e).map(|n| &n.data) {
                Some(NodeData::PEmitter(d)) => d.target,
                _ => None,
            };
            if tgt != Some(particle) {
                continue;
            }
            let w = self.emitter_frame_matrix(e);
            if let Some(NodeData::PEmitter(d)) = self.nodes.get_mut(e).map(|n| &mut n.data) {
                d.prev_world = Some(w);
                d.accum = 0.0;
            }
        }
    }

    /// World matrix of the object an emitter's motion is measured on (its source, else itself).
    fn emitter_frame_matrix(&mut self, e: NodeId) -> Mat4 {
        let src = match self.nodes.get(e).map(|n| &n.data) {
            Some(NodeData::PEmitter(d)) => d.source,
            _ => None,
        };
        let t = src.filter(|s| self.nodes.get(*s).is_some()).or_else(|| self.nodes.get(e).and_then(|n| n.parent));
        match t {
            Some(t) => self.world_matrix(t),
            None => Mat4::identity(),
        }
    }

    fn dyn_step_rigid(&mut self, solver: NodeId, f: i32, dt: f32, cache_on: bool, fields: &[(Field, Vec<NodeId>)]) {
        let step_size = self.plug_of(solver, "stepSize", 0.02).clamp(0.002, 0.1);
        let n = (dt / step_size).ceil().max(1.0) as usize;
        let h = dt / n as f32;
        let (binds, mut world) = {
            let rt = self.dyn_rt_mut(solver);
            match rt.world.take() {
                Some(w) => (rt.binds.clone(), w),
                None => return,
            }
        };
        // new poses of passive bodies and the active flags
        let mut cur: Vec<(Vec3, Quat)> = Vec::new();
        for (i, bind) in binds.iter().enumerate() {
            let p = self.pose_of(bind);
            cur.push(p);
            let want = self.plug_of(bind.body_node, "active", 1.0) > 0.5;
            if world.bodies[i].active != want {
                // a body that turns active starts with the motion of its keys
                let prev = self.dyn_rt(solver).map(|r| r.pose_cur[i]).unwrap_or(p);
                let b = &mut world.bodies[i];
                b.set_active(want);
                if want {
                    b.pos = p.0;
                    b.rot = p.1;
                    b.vel = (p.0 - prev.0) * (1.0 / dt);
                }
            }
        }
        let prev_poses = self.dyn_rt(solver).map(|r| r.pose_cur.clone()).unwrap_or_default();
        let cached = if cache_on { self.dyn_rt(solver).and_then(|r| r.cache.get(&f).cloned()) } else { None };
        match cached {
            Some(states) => {
                for (b, s) in world.bodies.iter_mut().zip(&states) {
                    b.pos = s.pos;
                    b.rot = s.rot;
                    b.vel = s.vel;
                    b.spin = s.spin;
                }
            }
            None => {
                let time = f as f32 / self.timeline.fps.max(1.0);
                let _ = time;
                for k in 1..=n {
                    let alpha = k as f32 / n as f32;
                    for (i, b) in world.bodies.iter_mut().enumerate() {
                        if !b.active {
                            let (p0, q0) = prev_poses.get(i).copied().unwrap_or(cur[i]);
                            let (p1, q1) = cur[i];
                            b.pos = p0 + (p1 - p0) * alpha;
                            b.rot = q0.slerp(q1, alpha);
                            b.vel = (p1 - p0) * (1.0 / dt);
                            // spin from the rotation difference
                            let dq = q1 * q0.conjugate();
                            let ang = 2.0 * dq.w.clamp(-1.0, 1.0).acos();
                            let ang = if ang > std::f32::consts::PI { ang - std::f32::consts::TAU } else { ang };
                            let axis = Vec3::new(dq.x, dq.y, dq.z);
                            b.spin = if axis.length_sq() > 1e-12 { axis.normalized() * (ang / dt) } else { Vec3::ZERO };
                        }
                    }
                    let node_of: Vec<NodeId> = binds.iter().map(|b| b.transform).collect();
                    let accel = |i: usize, b: &Body| -> Vec3 {
                        let mut a = Vec3::ZERO;
                        for (fl, targets) in fields {
                            if targets.contains(&node_of[i]) {
                                a += fl.accel(b.pos, b.vel, b.mass);
                            }
                        }
                        a
                    };
                    world.step(h, &accel);
                }
                if cache_on {
                    let states: Vec<BodyState> = world.bodies.iter().map(|b| BodyState { pos: b.pos, rot: b.rot, vel: b.vel, spin: b.spin }).collect();
                    self.dyn_rt_mut(solver).cache.insert(f, states);
                }
            }
        }
        let rt = self.dyn_rt_mut(solver);
        rt.pose_prev = prev_poses;
        rt.pose_cur = cur;
        rt.world = Some(world);
    }

    /// Triangle soup of a collision object in world space.
    fn collider_mesh(&mut self, node: NodeId) -> Option<Arc<TriMesh>> {
        let shapes = self.mesh_shapes_under(node);
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        for sh in shapes {
            let owner = self.nodes.get(sh).and_then(|n| n.parent).unwrap_or(sh);
            let w = self.world_matrix(owner);
            if let Some(NodeData::Mesh(md)) = self.nodes.get(sh).map(|n| &n.data) {
                let base = verts.len() as u32;
                verts.extend(md.mesh.positions.iter().map(|p| w.transform_point(*p)));
                tris.extend(md.mesh.triangulate().into_iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
            }
        }
        if verts.is_empty() {
            None
        } else {
            Some(Arc::new(TriMesh::new(verts, tris)))
        }
    }

    fn dyn_step_particles(&mut self, p: NodeId, f: i32, dt: f32, fps: f32, fields: &[(Field, Vec<NodeId>)], spawns: &mut Vec<(NodeId, Vec<(Vec3, Vec3, f32)>)>) -> usize {
        let Some(NodeData::Particle(pd)) = self.nodes.get(p).map(|n| n.data.clone()) else { return 0 };
        let mut sim = pd.sim;
        let radius = self.plug_of(p, "radius", 0.5);
        let (mode, life, rnd) = (self.plug_of(p, "lifespanMode", 0.0) as i32, self.plug_of(p, "lifespan", 1.0), self.plug_of(p, "lifespanRandom", 0.0));
        let maxc = self.plug_of(p, "maxCount", 5000.0) as usize;
        // emission
        for e in self.ids_where(|d| matches!(d, NodeData::PEmitter(_))) {
            let Some(NodeData::PEmitter(ed)) = self.nodes.get(e).map(|n| n.data.clone()) else { continue };
            if ed.target != Some(p) || self.plug_of(e, "enabled", 1.0) < 0.5 {
                continue;
            }
            let now = self.emitter_frame_matrix(e);
            let rate = self.plug_of(e, "rate", 100.0).max(0.0);
            let mut accum = ed.accum + rate * dt;
            let count = accum.floor().max(0.0) as usize;
            accum -= count as f32;
            let prev = ed.prev_world.unwrap_or(now);
            for _ in 0..count {
                if sim.items.len() >= maxc {
                    break;
                }
                let (pos, vel) = self.emit_sample(e, &ed, &mut sim.rng, &now, &prev, fps);
                let u = sim.rng.next_f32();
                let l = match mode {
                    0 => 0.0,
                    1 => life,
                    _ => (life + rnd * sim.rng.signed()).max(0.01),
                };
                // spread the births over the frame
                sim.spawn(pos + vel * (u * dt), vel, l, radius);
            }
            if let Some(NodeData::PEmitter(d)) = self.nodes.get_mut(e).map(|n| &mut n.data) {
                d.accum = accum;
                d.prev_world = Some(now);
            }
        }
        // colliders
        let mut colliders: Vec<Collider> = Vec::new();
        for c in &pd.colliders {
            if let Some(node) = c.node.filter(|n| self.nodes.get(*n).is_some()) {
                if let Some(mesh) = self.collider_mesh(node) {
                    colliders.push(Collider { mesh, resilience: c.resilience, friction: c.friction });
                }
            }
        }
        let relevant: Vec<&Field> = fields.iter().filter(|(_, t)| t.contains(&p)).map(|(fl, _)| fl).collect();
        let mass = self.plug_of(p, "mass", 1.0);
        let params = fields::Params { conserve: self.plug_of(p, "conserve", 1.0), damping: self.plug_of(p, "damping", 0.0), mass, radius: radius.min(0.3) * 0.5 };
        let n = 4usize;
        let h = dt / n as f32;
        let accel = |pos: Vec3, vel: Vec3| -> Vec3 {
            let mut a = Vec3::ZERO;
            for fl in &relevant {
                a += fl.accel(pos, vel, mass);
            }
            a
        };
        for _ in 0..n {
            let hits = fields::step(&mut sim, &params, h, fps, &accel, &colliders);
            for hit in hits {
                self.dyn_collision_event(&pd.events, &mut sim, hit, spawns);
            }
        }
        // death
        sim.items.retain(|q| q.life <= 0.0 || q.age < q.life);
        if let Some(NodeData::Particle(d)) = self.nodes.get_mut(p).map(|n| &mut n.data) {
            d.sim = sim;
            d.last_frame = f;
        }
        0
    }

    fn dyn_collision_event(&mut self, events: &[ParticleEvent], sim: &mut PSim, hit: fields::Hit, spawns: &mut Vec<(NodeId, Vec<(Vec3, Vec3, f32)>)>) {
        if events.is_empty() || hit.index >= sim.items.len() || sim.items[hit.index].evented {
            return;
        }
        sim.items[hit.index].evented = true;
        let mut kill = false;
        for ev in events {
            let num = if ev.random { 1 + (sim.rng.next_f32() * ev.num as f32) as u32 } else { ev.num };
            let mut list = Vec::new();
            for _ in 0..num {
                let jitter = Vec3::new(sim.rng.signed(), sim.rng.signed(), sim.rng.signed()) * ev.spread;
                let reflect = hit.vel - hit.normal * (2.0 * hit.vel.dot(hit.normal));
                let vel = reflect * ev.inherit + jitter + hit.normal * ev.spread * 0.5;
                list.push((hit.pos, vel, sim.rng.next_f32()));
            }
            // an event into the same object adds particles after the step
            let target = ev.target.filter(|t| self.nodes.get(*t).is_some());
            if let Some(t) = target {
                spawns.push((t, list));
            }
            if ev.original_dies {
                kill = true;
            }
        }
        if kill {
            sim.items[hit.index].life = 1e-6;
            sim.items[hit.index].age = 1.0;
        }
    }

    /// One emitted particle: position and velocity in world space.
    fn emit_sample(&mut self, e: NodeId, ed: &PEmitterData, rng: &mut Rng, now: &Mat4, prev: &Mat4, fps: f32) -> (Vec3, Vec3) {
        let kind = self.plug_of(e, "emitterType", 0.0).round() as i32;
        let speed = (self.plug_of(e, "speed", 1.0) * (1.0 + self.plug_of(e, "speedRandom", 0.0) * rng.signed())).max(0.0);
        let spread = self.plug_of(e, "spread", 0.0).clamp(0.0, 1.0);
        let inherit = self.plug_of(e, "inheritFactor", 0.0);
        let et = self.nodes.get(e).and_then(|n| n.parent).unwrap_or(e);
        let ew = self.world_matrix(et);
        let local_dir = Vec3::new(self.plug_of(e, "directionX", 0.0), self.plug_of(e, "directionY", 1.0), self.plug_of(e, "directionZ", 0.0));
        let dir_w = ew.transform_dir(local_dir).normalized();
        let random_unit = |rng: &mut Rng| -> Vec3 {
            loop {
                let v = Vec3::new(rng.signed(), rng.signed(), rng.signed());
                let l = v.length_sq();
                if l > 1e-4 && l <= 1.0 {
                    return v * (1.0 / l.sqrt());
                }
            }
        };
        let cone = |rng: &mut Rng, axis: Vec3| -> Vec3 {
            if spread <= 0.0 {
                return axis;
            }
            (axis + random_unit(rng) * (spread * 1.6)).normalized()
        };
        let (pos, dir) = match kind {
            1 => (ew.translation_part(), cone(rng, dir_w)),
            2 => {
                // a random point of the unit cube in the emitter's space
                let l = Vec3::new(rng.signed(), rng.signed(), rng.signed()) * 0.5;
                (ew.transform_point(l), random_unit(rng))
            }
            3 => match ed.source.and_then(|s| self.surface_sample(s, rng)) {
                Some((p, n)) => (p, cone(rng, n)),
                None => (ew.translation_part(), random_unit(rng)),
            },
            4 => match ed.source.and_then(|s| self.curve_sample(s, rng)) {
                Some(p) => (p, if spread > 0.0 || local_dir.length_sq() < 1e-8 { cone(rng, dir_w) } else { dir_w }),
                None => (ew.translation_part(), random_unit(rng)),
            },
            _ => (ew.translation_part(), random_unit(rng)),
        };
        let mut vel = dir * speed;
        if inherit != 0.0 {
            // velocity of the emission point from the motion of the object since the last frame
            let p_prev = (*prev * now.inverse()).transform_point(pos);
            vel += (pos - p_prev) * (fps * inherit);
        }
        (pos, vel)
    }

    fn surface_sample(&mut self, src: NodeId, rng: &mut Rng) -> Option<(Vec3, Vec3)> {
        let mut tris: Vec<(Vec3, Vec3, Vec3)> = Vec::new();
        for sh in self.mesh_shapes_under(src) {
            let owner = self.nodes.get(sh).and_then(|n| n.parent).unwrap_or(sh);
            let w = self.world_matrix(owner);
            if let Some(NodeData::Mesh(md)) = self.nodes.get(sh).map(|n| &n.data) {
                for t in md.mesh.triangulate() {
                    tris.push((w.transform_point(md.mesh.positions[t[0] as usize]), w.transform_point(md.mesh.positions[t[1] as usize]), w.transform_point(md.mesh.positions[t[2] as usize])));
                }
            }
        }
        if tris.is_empty() {
            return None;
        }
        let area: Vec<f32> = tris.iter().map(|t| (t.1 - t.0).cross(t.2 - t.0).length() * 0.5).collect();
        let total: f32 = area.iter().sum();
        let mut pick = rng.next_f32() * total;
        let mut idx = tris.len() - 1;
        for (i, a) in area.iter().enumerate() {
            if pick <= *a {
                idx = i;
                break;
            }
            pick -= a;
        }
        let t = tris[idx];
        let (mut u, mut v) = (rng.next_f32(), rng.next_f32());
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        Some((t.0 + (t.1 - t.0) * u + (t.2 - t.0) * v, (t.1 - t.0).cross(t.2 - t.0).normalized()))
    }

    fn curve_sample(&mut self, src: NodeId, rng: &mut Rng) -> Option<Vec3> {
        let shapes = self.curve_shapes_under(src);
        let sh = *shapes.first()?;
        let owner = self.nodes.get(sh).and_then(|n| n.parent).unwrap_or(sh);
        let w = self.world_matrix(owner);
        match self.nodes.get(sh).map(|n| &n.data) {
            Some(NodeData::NurbsCurve(cd)) => {
                let (a, b) = cd.curve.dir.domain();
                Some(w.transform_point(cd.curve.point(a + (b - a) * rng.next_f32())))
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------------ initial state and caches

    /// Solvers > Initial State > Set for Selected / All Dynamic: keep the present state as the start.
    pub fn set_initial_state(&mut self, only: Option<&[NodeId]>) -> usize {
        let Some(solver) = self.dyn_solver_id() else { return 0 };
        let mut n = 0;
        let states: Vec<(NodeId, BodyState)> = match self.dyn_rt(solver) {
            Some(rt) => match &rt.world {
                Some(w) => rt.binds.iter().zip(&w.bodies).map(|(bind, b)| (bind.body_node, BodyState { pos: b.pos, rot: b.rot, vel: b.vel, spin: b.spin })).collect(),
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        for (rb, st) in states {
            let t = self.nodes.get(rb).and_then(|n| n.parent);
            if only.map(|o| !o.iter().any(|x| Some(*x) == t)).unwrap_or(false) {
                continue;
            }
            if let Some(NodeData::RigidBody(d)) = self.nodes.get_mut(rb).map(|n| &mut n.data) {
                d.initial = Some(st);
                n += 1;
            }
        }
        for p in self.ids_where(|d| matches!(d, NodeData::Particle(_))) {
            if only.map(|o| !o.contains(&p)).unwrap_or(false) {
                continue;
            }
            if let Some(NodeData::Particle(pd)) = self.nodes.get_mut(p).map(|n| &mut n.data) {
                let mut s = pd.sim.clone();
                // the particles keep their ages, so they live on as they did
                for q in s.items.iter_mut() {
                    q.evented = false;
                }
                pd.initial = Some(s);
                n += 1;
            }
        }
        self.dyn_rt_mut(solver).started = false;
        n
    }

    pub fn clear_initial_state(&mut self) {
        for rb in self.ids_where(|d| matches!(d, NodeData::RigidBody(_))) {
            if let Some(NodeData::RigidBody(d)) = self.nodes.get_mut(rb).map(|n| &mut n.data) {
                d.initial = None;
            }
        }
        for p in self.ids_where(|d| matches!(d, NodeData::Particle(_))) {
            if let Some(NodeData::Particle(pd)) = self.nodes.get_mut(p).map(|n| &mut n.data) {
                pd.initial = None;
            }
        }
        if let Some(s) = self.dyn_solver_id() {
            self.dyn_rt_mut(s).started = false;
        }
    }

    /// Delete the rigid body cache so the next play simulates again.
    pub fn delete_dyn_cache(&mut self) {
        if let Some(s) = self.dyn_solver_id() {
            let rt = self.dyn_rt_mut(s);
            rt.cache.clear();
            rt.started = false;
        }
    }

    /// The simulated state of the rigid bodies, for tests and the Attribute Editor.
    pub fn rigid_state(&self, transform: NodeId) -> Option<BodyState> {
        let solver = self.dyn_solver_id()?;
        let rt = self.dyn_rt(solver)?;
        let i = rt.binds.iter().position(|b| b.transform == transform)?;
        let b = rt.world.as_ref()?.bodies.get(i)?;
        Some(BodyState { pos: b.pos, rot: b.rot, vel: b.vel, spin: b.spin })
    }
}
