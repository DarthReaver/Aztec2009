//! Dynamics commands: rigid bodies, constraints, fields, particles, emitters, collisions, events, ramps and the solver.

use super::commands::{resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::anim::Interp;
use crate::dag::dynamics::JOINT_KINDS;
use crate::dag::nodes::*;
use crate::dag::NodeId;
use crate::math::Vec3;
use crate::nucleus::fields::{FIELD_KINDS, VOLUMES};
use crate::session::Session;

fn nums(text: &str) -> Vec<f32> {
    text.split(|c: char| c.is_whitespace() || c == ',').filter_map(|x| x.parse().ok()).collect()
}

fn vec3_flag(p: &super::parser::Parsed, k: &str) -> Result<Option<Vec3>, String> {
    match p.string(k) {
        Some(t) => {
            let v = nums(&t);
            if v.len() != 3 {
                return Err(format!("-{} needs \"x y z\"", k));
            }
            Ok(Some(Vec3::new(v[0], v[1], v[2])))
        }
        None => Ok(None),
    }
}

fn transforms(s: &Session, args: &[String]) -> Result<Vec<NodeId>, String> {
    resolve_targets(s, args)
}

const RB_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["active", "a"], arity: 0 },
    FlagSpec { names: &["passive", "p"], arity: 0 },
    FlagSpec { names: &["mass", "m"], arity: 1 },
    FlagSpec { names: &["bounciness", "b"], arity: 1 },
    FlagSpec { names: &["damping", "d"], arity: 1 },
    FlagSpec { names: &["staticFriction", "sf"], arity: 1 },
    FlagSpec { names: &["dynamicFriction", "df"], arity: 1 },
    FlagSpec { names: &["centerOfMass", "com"], arity: 1 },
    FlagSpec { names: &["initialVelocity", "iv"], arity: 1 },
    FlagSpec { names: &["initialSpin", "is"], arity: 1 },
    FlagSpec { names: &["shape"], arity: 1 },
    FlagSpec { names: &["delete"], arity: 0 },
    FlagSpec { names: &["query", "q"], arity: 0 },
];

/// `rigidBody [-active | -passive] [-m MASS] [-b BOUNCINESS] [-d DAMPING] [-sf F] [-df F] [-com "x y z"] [-iv "x y z"] [-is "x y z"] [-shape box|sphere|mesh] obj ...`
/// Soft/Rigid Bodies > Create Active / Passive Rigid Body. `-delete` removes the rigid body, `-q` lists the settings.
pub fn rigid_body(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, RB_SPEC)?;
    let ids = transforms(s, &p.arg_strings())?;
    let mut out = Vec::new();
    for t in ids {
        let t = match s.scene.node(t).map(|n| &n.data) {
            Some(NodeData::Transform) => t,
            _ => s.scene.node(t).and_then(|n| n.parent).unwrap_or(t),
        };
        if p.has("delete") {
            s.scene.remove_rigid_body(t)?;
            continue;
        }
        let rb = match (s.scene.rigid_body_of(t), p.has("query")) {
            (Some(rb), _) => rb,
            (None, true) => return Err(format!("{} is not a rigid body", s.scene.name_of(t))),
            (None, false) => s.scene.create_rigid_body(t, !p.has("passive"))?,
        };
        for (flag, plug) in [("mass", "mass"), ("bounciness", "bounciness"), ("damping", "damping"), ("staticFriction", "staticFriction"), ("dynamicFriction", "dynamicFriction")] {
            if let Some(v) = p.num(flag)? {
                s.scene.set_attr(rb, plug, v)?;
            }
        }
        for (flag, base) in [("centerOfMass", "centerOfMass"), ("initialVelocity", "initialVelocity"), ("initialSpin", "initialSpin")] {
            if let Some(v) = vec3_flag(&p, flag)? {
                for (a, x) in ["X", "Y", "Z"].iter().zip([v.x, v.y, v.z]) {
                    s.scene.set_attr(rb, &format!("{}{}", base, a), x)?;
                }
            }
        }
        if let Some(sh) = p.string("shape") {
            let v = match sh.as_str() {
                "box" | "cube" => 0.0,
                "sphere" => 1.0,
                "mesh" => 2.0,
                o => return Err(format!("Unknown stand-in shape '{}': box, sphere or mesh", o)),
            };
            s.scene.set_attr(rb, "shape", v)?;
        }
        if p.has("active") && !p.has("passive") {
            s.scene.set_attr(rb, "active", 1.0)?;
        } else if p.has("passive") {
            s.scene.set_attr(rb, "active", 0.0)?;
        }
        out.push(s.scene.name_of(rb));
    }
    s.scene.evaluate_frame();
    Ok(out)
}

/// `setRigidKey -active | -passive obj ...`: Soft/Rigid Bodies > Set Active Key / Set Passive Key at the current frame.
pub fn set_rigid_key(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["active", "a"], arity: 0 }, FlagSpec { names: &["passive", "p"], arity: 0 }])?;
    let value = if p.has("passive") { 0.0 } else { 1.0 };
    let ids = transforms(s, &p.arg_strings())?;
    let mut n = 0;
    for t in ids {
        let rb = s.scene.rigid_body_of(t).ok_or_else(|| format!("{} is not a rigid body", s.scene.name_of(t)))?;
        s.scene.set_keyframe(rb, "active", None, Some(value), Interp::Step, 0)?;
        n += 1;
    }
    Ok(vec![format!("Keyed {} rigid body(ies) {}", n, if value > 0.5 { "active" } else { "passive" })])
}

const SOLVER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["cache"], arity: 1 },
    FlagSpec { names: &["deleteCache"], arity: 0 },
    FlagSpec { names: &["state"], arity: 1 },
    FlagSpec { names: &["startFrame"], arity: 1 },
    FlagSpec { names: &["stepSize"], arity: 1 },
    FlagSpec { names: &["tolerance"], arity: 1 },
    FlagSpec { names: &["query", "q"], arity: 0 },
];

/// `rigidSolver [-cache 0|1] [-deleteCache] [-state 0|1] [-startFrame N] [-stepSize S] [-tolerance T] [-q]`: Solvers > Rigid Body Solver Attributes.
pub fn rigid_solver(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SOLVER_SPEC)?;
    let id = s.scene.ensure_dyn_solver();
    if p.has("deleteCache") {
        s.scene.delete_dyn_cache();
    }
    for (flag, plug) in [("cache", "cacheData"), ("state", "state"), ("startFrame", "startFrame"), ("stepSize", "stepSize"), ("tolerance", "collisionTolerance")] {
        if let Some(v) = p.num(flag)? {
            s.scene.set_attr(id, plug, v)?;
        }
    }
    let g = |s: &mut Session, k: &str| s.scene.get_attr(id, k).unwrap_or(0.0);
    Ok(vec![format!("{}: state {}, start frame {}, step size {}, cache {} ({} frame(s) cached)", s.scene.name_of(id), g(s, "state"), g(s, "startFrame"), g(s, "stepSize"), g(s, "cacheData") as u8, g(s, "cachedFrames"))])
}

const INIT_SPEC: &[FlagSpec] = &[FlagSpec { names: &["set"], arity: 0 }, FlagSpec { names: &["clear"], arity: 0 }, FlagSpec { names: &["all"], arity: 0 }];

/// `dynInitialState [-all] [-clear] [obj ...]`: Solvers > Initial State > Set for Selected / Set for All Dynamic / Clear.
pub fn initial_state(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, INIT_SPEC)?;
    if p.has("clear") {
        s.scene.clear_initial_state();
        return Ok(vec!["Initial state cleared".to_string()]);
    }
    let only: Option<Vec<NodeId>> = if p.has("all") {
        None
    } else {
        let ids = resolve_targets(s, &p.arg_strings())?;
        Some(ids.iter().filter_map(|i| s.scene.dyn_target_of(*i)).collect())
    };
    let n = s.scene.set_initial_state(only.as_deref());
    s.scene.evaluate_frame();
    Ok(vec![format!("Initial state set for {} object(s)", n)])
}

const FIELD_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["magnitude", "m"], arity: 1 },
    FlagSpec { names: &["attenuation", "att"], arity: 1 },
    FlagSpec { names: &["direction", "dir"], arity: 1 },
    FlagSpec { names: &["maxDistance", "maxd"], arity: 1 },
    FlagSpec { names: &["volume"], arity: 1 },
    FlagSpec { names: &["speed"], arity: 1 },
    FlagSpec { names: &["frequency"], arity: 1 },
    FlagSpec { names: &["pos"], arity: 1 },
];

/// `gravity | air | drag | newton | radial | turbulence | uniform | vortex [-m MAG] [-att A] [-dir "x y z"] [-maxd D] [-volume none|cube|sphere|cylinder|cone] [-pos "x y z"] [obj ...]`:
/// the Fields menu. A field made with rigid bodies or particles selected affects them.
pub fn field(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, FIELD_SPEC)?;
    let kind = FIELD_KINDS.iter().position(|k| *k == cmd).ok_or("Unknown field")? as u8;
    let (t, f) = s.scene.create_field(kind, p.string("name").as_deref());
    for (flag, plug) in [("magnitude", "magnitude"), ("attenuation", "attenuation"), ("speed", "speed"), ("frequency", "frequency")] {
        if let Some(v) = p.num(flag)? {
            s.scene.set_attr(f, plug, v)?;
        }
    }
    if let Some(d) = p.num("maxDistance")? {
        s.scene.set_attr(f, "maxDistance", d)?;
        s.scene.set_attr(f, "useMaxDistance", 1.0)?;
    }
    if let Some(v) = vec3_flag(&p, "direction")? {
        for (a, x) in ["directionX", "directionY", "directionZ"].iter().zip([v.x, v.y, v.z]) {
            s.scene.set_attr(f, a, x)?;
        }
    }
    if let Some(v) = vec3_flag(&p, "pos")? {
        for (a, x) in ["translateX", "translateY", "translateZ"].iter().zip([v.x, v.y, v.z]) {
            s.scene.set_attr(t, a, x)?;
        }
    }
    if let Some(v) = p.string("volume") {
        let i = VOLUMES.iter().position(|x| *x == v).ok_or_else(|| format!("Unknown volume '{}': {}", v, VOLUMES.join(", ")))?;
        s.scene.set_attr(f, "volumeShape", i as f32)?;
    }
    // objects named, or selected rigid bodies and particles
    let args = p.arg_strings();
    let targets: Vec<NodeId> = if args.is_empty() { s.scene.selection.clone() } else { resolve_targets(s, &args)? };
    let dyns: Vec<NodeId> = targets.iter().filter_map(|t| s.scene.dyn_target_of(*t)).collect();
    if !dyns.is_empty() {
        s.scene.connect_field(f, &dyns, true)?;
    }
    s.scene.selection = vec![t];
    s.scene.evaluate_frame();
    Ok(vec![s.scene.name_of(t)])
}

const CONNECT_SPEC: &[FlagSpec] = &[FlagSpec { names: &["field", "f"], arity: 1 }, FlagSpec { names: &["disconnect", "d"], arity: 0 }, FlagSpec { names: &["query", "q"], arity: 0 }];

/// `connectDynamic -f FIELD [-d] obj ...`: Fields > Affect Selected Object(s); `-d` disconnects; `-q -f FIELD` lists what it affects.
pub fn connect_dynamic(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, CONNECT_SPEC)?;
    let name = p.string("field").ok_or("Usage: connectDynamic -f FIELD [-d] obj ...")?;
    let id = s.scene.find(&name).ok_or_else(|| format!("No field named {}", name))?;
    let f = match s.scene.node(id).map(|n| &n.data) {
        Some(NodeData::Field(_)) => id,
        _ => s.scene.node(id).and_then(|n| n.children.iter().copied().find(|c| matches!(s.scene.node(*c).map(|x| &x.data), Some(NodeData::Field(_))))).ok_or_else(|| format!("'{}' is not a field", name))?,
    };
    if p.has("query") {
        let list = match s.scene.node(f).map(|n| &n.data) {
            Some(NodeData::Field(fd)) => fd.targets.iter().filter(|t| s.scene.node(**t).is_some()).map(|t| s.scene.name_of(*t)).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        return Ok(vec![list.join(" ")]);
    }
    let ids = resolve_targets(s, &p.arg_strings())?;
    let n = s.scene.connect_field(f, &ids, !p.has("disconnect"))?;
    s.scene.evaluate_frame();
    Ok(vec![format!("{} affects {} object(s)", name, n)])
}

const JOINT_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["type", "t"], arity: 1 },
    FlagSpec { names: &["a"], arity: 1 },
    FlagSpec { names: &["b"], arity: 1 },
    FlagSpec { names: &["pos"], arity: 1 },
    FlagSpec { names: &["stiffness"], arity: 1 },
    FlagSpec { names: &["damping"], arity: 1 },
    FlagSpec { names: &["rest"], arity: 1 },
    FlagSpec { names: &["name", "n"], arity: 1 },
];

/// `rigidConstraint -type nail|pin|hinge|spring|barrier [-a OBJ] [-b OBJ] [-pos "x y z"] [-stiffness K] [-damping D] [-rest L]`:
/// Soft/Rigid Bodies > Create Constraint. A nail holds one body at the point, a pin joins two bodies, a hinge also keeps the
/// axis (the constraint's Z), a spring pulls the two anchors, a barrier is the plane of the constraint's Y.
pub fn rigid_constraint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, JOINT_SPEC)?;
    let ty = p.string("type").ok_or("Usage: rigidConstraint -type nail|pin|hinge|spring|barrier -a OBJ [-b OBJ]")?;
    let kind = JOINT_KINDS.iter().position(|k| *k == ty).ok_or_else(|| format!("Unknown constraint '{}': {}", ty, JOINT_KINDS.join(", ")))? as u8;
    let body = |s: &mut Session, name: Option<String>| -> Result<Option<NodeId>, String> {
        match name {
            Some(n) => {
                let ids = resolve_targets(s, &[n.clone()])?;
                let t = ids[0];
                s.scene.rigid_body_of(t).ok_or_else(|| format!("{} is not a rigid body", n))?;
                Ok(Some(t))
            }
            None => Ok(None),
        }
    };
    let (mut a, mut b) = (body(s, p.string("a"))?, body(s, p.string("b"))?);
    let args = p.arg_strings();
    if a.is_none() && kind != 4 {
        // the selected rigid bodies
        let sel = if args.is_empty() { s.scene.selection.clone() } else { resolve_targets(s, &args)? };
        let rbs: Vec<NodeId> = sel.into_iter().filter(|t| s.scene.rigid_body_of(*t).is_some()).collect();
        a = rbs.first().copied();
        if b.is_none() {
            b = rbs.get(1).copied();
        }
    }
    if kind != 4 && a.is_none() {
        return Err("Select the rigid body (and a second one) to constrain".to_string());
    }
    let pos = match vec3_flag(&p, "pos")? {
        Some(v) => v,
        None => match (a, b) {
            (Some(a), Some(b)) => (s.scene.world_matrix(a).translation_part() + s.scene.world_matrix(b).translation_part()) * 0.5,
            (Some(a), None) => s.scene.world_matrix(a).translation_part(),
            _ => Vec3::ZERO,
        },
    };
    let t = s.scene.create_node(NodeData::Transform, p.string("name").as_deref().unwrap_or(JOINT_KINDS[kind as usize]), None, p.string("name").is_none());
    let j = s.scene.create_node(NodeData::RigidJoint(Box::new(RigidJointData { kind, a, b, names: Vec::new() })), &format!("{}Shape", s.scene.name_of(t)), Some(t), false);
    for (n, v) in ["translateX", "translateY", "translateZ"].iter().zip([pos.x, pos.y, pos.z]) {
        s.scene.set_attr(t, n, v)?;
    }
    for (flag, plug) in [("stiffness", "stiffness"), ("damping", "damping"), ("rest", "restLength")] {
        if let Some(v) = p.num(flag)? {
            s.scene.set_attr(j, plug, v)?;
        }
    }
    s.scene.selection = vec![t];
    s.scene.evaluate_frame();
    Ok(vec![s.scene.name_of(t)])
}

const PARTICLE_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["type"], arity: 1 },
    FlagSpec { names: &["radius", "r"], arity: 1 },
    FlagSpec { names: &["lifespan"], arity: 1 },
    FlagSpec { names: &["lifespanRandom"], arity: 1 },
    FlagSpec { names: &["conserve"], arity: 1 },
    FlagSpec { names: &["start"], arity: 1 },
    FlagSpec { names: &["threshold"], arity: 1 },
    FlagSpec { names: &["material"], arity: 1 },
];

/// `particle [-name N] [-type points|spheres|blobby|sprites|streaks] [-r RADIUS] [-lifespan S] [-lifespanRandom S] [-conserve C] [-start F] [-threshold T]`:
/// an empty particle object (fill it with an emitter). Existing objects: with names, changes their settings instead.
pub fn particle(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, PARTICLE_SPEC)?;
    let args = p.arg_strings();
    let (t, shape) = if args.is_empty() {
        s.scene.create_particle_object(p.string("name").as_deref())
    } else {
        let ids = resolve_targets(s, &args)?;
        let t = ids[0];
        let sh = s.scene.dyn_target_of(t).filter(|x| matches!(s.scene.node(*x).map(|n| &n.data), Some(NodeData::Particle(_)))).ok_or_else(|| format!("{} is not a particle object", args[0]))?;
        (t, sh)
    };
    apply_particle_flags(s, shape, &p)?;
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

fn apply_particle_flags(s: &mut Session, shape: NodeId, p: &super::parser::Parsed) -> Result<(), String> {
    if let Some(t) = p.string("type") {
        let v = match t.as_str() {
            "points" | "point" => 0.0,
            "spheres" | "sphere" => 1.0,
            "blobby" | "blobbySurface" => 2.0,
            "sprites" | "sprite" => 3.0,
            "streaks" | "streak" => 4.0,
            o => return Err(format!("Unknown render type '{}': points, spheres, blobby, sprites or streaks", o)),
        };
        s.scene.set_attr(shape, "renderType", v)?;
    }
    for (flag, plug) in [("radius", "radius"), ("lifespanRandom", "lifespanRandom"), ("conserve", "conserve"), ("start", "startFrame"), ("threshold", "threshold")] {
        if let Some(v) = p.num(flag)? {
            s.scene.set_attr(shape, plug, v)?;
        }
    }
    if let Some(l) = p.num("lifespan")? {
        s.scene.set_attr(shape, "lifespan", l)?;
        s.scene.set_attr(shape, "lifespanMode", if p.num("lifespanRandom")?.unwrap_or(0.0) > 0.0 { 2.0 } else { 1.0 })?;
    }
    if let Some(m) = p.string("material") {
        let mat = s.scene.find(&m).ok_or_else(|| format!("No material named {}", m))?;
        s.scene.assign_material(mat, shape)?;
    }
    Ok(())
}

const EMITTER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["type", "t"], arity: 1 },
    FlagSpec { names: &["rate", "r"], arity: 1 },
    FlagSpec { names: &["speed", "spd"], arity: 1 },
    FlagSpec { names: &["speedRandom"], arity: 1 },
    FlagSpec { names: &["spread", "sp"], arity: 1 },
    FlagSpec { names: &["direction", "dir"], arity: 1 },
    FlagSpec { names: &["inherit"], arity: 1 },
    FlagSpec { names: &["particle", "p"], arity: 1 },
    FlagSpec { names: &["source"], arity: 1 },
    FlagSpec { names: &["pos"], arity: 1 },
    FlagSpec { names: &["solver", "s"], arity: 1 },
];

/// `emitter -type omni|directional|volume|surface|curve [-r RATE] [-spd SPEED] [-sp SPREAD] [-dir "x y z"] [-inherit F] [-particle P] [-source OBJ] [-pos "x y z"]`:
/// Particles > Create Emitter / Add Emitter. Without `-particle` a new particle object is made. (Without these flags the old
/// nucleus emitter command runs.)
pub fn emitter(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, EMITTER_SPEC)?;
    if p.string("type").is_none() && p.string("particle").is_none() && p.string("source").is_none() && p.string("rate").is_none() {
        return super::cmd_scene::emitter(s, rest);
    }
    let kind = match p.string("type").unwrap_or_else(|| "omni".to_string()).as_str() {
        "omni" => 0,
        "directional" => 1,
        "volume" => 2,
        "surface" => 3,
        "curve" => 4,
        o => return Err(format!("Unknown emitter type '{}': omni, directional, volume, surface or curve", o)),
    };
    let target = match p.string("particle") {
        Some(n) => {
            let ids = resolve_targets(s, &[n.clone()])?;
            s.scene.dyn_target_of(ids[0]).filter(|x| matches!(s.scene.node(*x).map(|n| &n.data), Some(NodeData::Particle(_)))).ok_or_else(|| format!("{} is not a particle object", n))?
        }
        None => s.scene.create_particle_object(None).1,
    };
    let source = match p.string("source") {
        Some(n) => Some(resolve_targets(s, &[n])?[0]),
        None => None,
    };
    let (t, e) = s.scene.create_particle_emitter(target, source, kind, p.string("name").as_deref());
    for (flag, plug) in [("rate", "rate"), ("speed", "speed"), ("speedRandom", "speedRandom"), ("spread", "spread"), ("inherit", "inheritFactor")] {
        if let Some(v) = p.num(flag)? {
            s.scene.set_attr(e, plug, v)?;
        }
    }
    if let Some(v) = vec3_flag(&p, "direction")? {
        for (a, x) in ["directionX", "directionY", "directionZ"].iter().zip([v.x, v.y, v.z]) {
            s.scene.set_attr(e, a, x)?;
        }
    }
    if let Some(v) = vec3_flag(&p, "pos")? {
        for (a, x) in ["translateX", "translateY", "translateZ"].iter().zip([v.x, v.y, v.z]) {
            s.scene.set_attr(t, a, x)?;
        }
    }
    s.scene.selection = vec![t];
    s.scene.evaluate_frame();
    let particle_t = s.scene.node(target).and_then(|n| n.parent).unwrap_or(target);
    Ok(vec![s.scene.name_of(t), s.scene.name_of(particle_t)])
}

const EMIT_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["type", "t"], arity: 1 },
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["rate", "r"], arity: 1 },
    FlagSpec { names: &["speed", "spd"], arity: 1 },
    FlagSpec { names: &["spread", "sp"], arity: 1 },
    FlagSpec { names: &["inherit"], arity: 1 },
    FlagSpec { names: &["particle", "p"], arity: 1 },
];

/// `emitFromObject [-type surface|curve|omni] [-n NAME] [-r RATE] [-spd SPEED] [-inherit F] [-p PARTICLE] obj ...`: Particles > Emit from Object.
/// One emitter per object, all filling one new particle object.
pub fn emit_from_object(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, EMIT_SPEC)?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let kind: u8 = match p.string("type").unwrap_or_else(|| "surface".to_string()).as_str() {
        "surface" => 3,
        "curve" => 4,
        "omni" => 0,
        "volume" => 2,
        o => return Err(format!("Unknown emitter type '{}'", o)),
    };
    let target = match p.string("particle") {
        Some(n) => {
            let ids = resolve_targets(s, &[n.clone()])?;
            s.scene.dyn_target_of(ids[0]).ok_or_else(|| format!("{} is not a particle object", n))?
        }
        None => s.scene.create_particle_object(None).1,
    };
    let mut out = Vec::new();
    for obj in ids {
        let is_curve = !s.scene.curve_shapes_under(obj).is_empty();
        let k = if kind == 3 && is_curve { 4 } else { kind };
        let ename = p.string("name");
        let (t, e) = s.scene.create_particle_emitter(target, Some(obj), k, ename.as_deref());
        for (flag, plug) in [("rate", "rate"), ("speed", "speed"), ("spread", "spread"), ("inherit", "inheritFactor")] {
            if let Some(v) = p.num(flag)? {
                s.scene.set_attr(e, plug, v)?;
            }
        }
        out.push(s.scene.name_of(t));
    }
    let particle_t = s.scene.node(target).and_then(|n| n.parent).unwrap_or(target);
    out.push(s.scene.name_of(particle_t));
    s.scene.evaluate_frame();
    Ok(out)
}

const COLLIDE_SPEC: &[FlagSpec] = &[FlagSpec { names: &["resilience", "r"], arity: 1 }, FlagSpec { names: &["friction", "f"], arity: 1 }, FlagSpec { names: &["remove"], arity: 0 }, FlagSpec { names: &["query", "q"], arity: 0 }];

/// `makeCollide [-r RESILIENCE] [-f FRICTION] [-remove] particle geometry ...`: Particles > Make Collide (the particles first, the geometry last).
pub fn make_collide(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, COLLIDE_SPEC)?;
    let names = p.arg_strings();
    let ids = resolve_targets(s, &names)?;
    let particle = ids.iter().filter_map(|i| s.scene.dyn_target_of(*i)).find(|x| matches!(s.scene.node(*x).map(|n| &n.data), Some(NodeData::Particle(_)))).ok_or("Select the particles, then the geometry they collide with")?;
    let geos: Vec<NodeId> = ids.iter().copied().filter(|i| !s.scene.mesh_shapes_under(*i).is_empty() && s.scene.dyn_target_of(*i).map(|d| !matches!(s.scene.node(d).map(|n| &n.data), Some(NodeData::Particle(_)))).unwrap_or(true)).collect();
    if p.has("query") {
        let list = match s.scene.node(particle).map(|n| &n.data) {
            Some(NodeData::Particle(pd)) => pd.colliders.iter().map(|c| format!("{} resilience {} friction {}", c.name, c.resilience, c.friction)).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        return Ok(list);
    }
    if geos.is_empty() {
        return Err("Select the geometry the particles should collide with (last)".to_string());
    }
    let names: Vec<(NodeId, String)> = geos.iter().map(|g| (*g, s.scene.name_of(*g))).collect();
    let (res, fric) = (p.num("resilience")?, p.num("friction")?);
    if let Some(NodeData::Particle(pd)) = s.scene.nodes.get_mut(particle).map(|n| &mut n.data) {
        for (g, name) in names {
            if p.has("remove") {
                pd.colliders.retain(|c| c.node != Some(g));
                continue;
            }
            match pd.colliders.iter_mut().find(|c| c.node == Some(g)) {
                Some(c) => {
                    c.resilience = res.unwrap_or(c.resilience);
                    c.friction = fric.unwrap_or(c.friction);
                }
                None => pd.colliders.push(ColliderRef { node: Some(g), name, resilience: res.unwrap_or(0.5), friction: fric.unwrap_or(0.1) }),
            }
        }
    }
    s.scene.evaluate_frame();
    Ok(vec![])
}

const EVENT_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["particle", "p"], arity: 1 },
    FlagSpec { names: &["type"], arity: 1 },
    FlagSpec { names: &["num"], arity: 1 },
    FlagSpec { names: &["random"], arity: 1 },
    FlagSpec { names: &["spread"], arity: 1 },
    FlagSpec { names: &["target"], arity: 1 },
    FlagSpec { names: &["inherit"], arity: 1 },
    FlagSpec { names: &["dies"], arity: 1 },
    FlagSpec { names: &["clear"], arity: 0 },
];

/// `particleEvent -p PARTICLE -target PARTICLE [-type emit|split] [-num N] [-random 0|1] [-spread S] [-inherit F] [-dies 0|1]`:
/// the Particle Collision Event Editor. `-clear` removes the events.
pub fn particle_event(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, EVENT_SPEC)?;
    let name = p.string("particle").ok_or("Usage: particleEvent -p PARTICLE -target PARTICLE [-num N] [-spread S] [-inherit F] [-dies 0|1]")?;
    let pid = resolve_targets(s, &[name.clone()])?;
    let shape = s.scene.dyn_target_of(pid[0]).ok_or_else(|| format!("{} is not a particle object", name))?;
    if p.has("clear") {
        if let Some(NodeData::Particle(pd)) = s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
            pd.events.clear();
        }
        return Ok(vec![]);
    }
    let kind = match p.string("type").unwrap_or_else(|| "emit".to_string()).as_str() {
        "emit" => 0,
        "split" => 1,
        o => return Err(format!("Unknown event type '{}': emit or split", o)),
    };
    let (target, target_name) = match p.string("target") {
        Some(t) => {
            let ids = resolve_targets(s, &[t.clone()])?;
            let tg = s.scene.dyn_target_of(ids[0]).filter(|x| matches!(s.scene.node(*x).map(|n| &n.data), Some(NodeData::Particle(_)))).ok_or_else(|| format!("{} is not a particle object", t))?;
            (Some(tg), t)
        }
        // splitting and emitting without a target fill the same object
        None => (Some(shape), name.clone()),
    };
    let ev = ParticleEvent { kind, num: p.num("num")?.unwrap_or(3.0).max(1.0) as u32, random: p.num("random")?.unwrap_or(0.0) > 0.5, spread: p.num("spread")?.unwrap_or(1.0), target, target_name, inherit: p.num("inherit")?.unwrap_or(1.0), original_dies: p.num("dies")?.map(|d| d > 0.5).unwrap_or(true) };
    if let Some(NodeData::Particle(pd)) = s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
        pd.events.push(ev);
    }
    s.scene.evaluate_frame();
    Ok(vec![])
}

const RAMP_SPEC: &[FlagSpec] = &[FlagSpec { names: &["particle", "p"], arity: 1 }, FlagSpec { names: &["attr", "a"], arity: 1 }, FlagSpec { names: &["stops"], arity: 1 }, FlagSpec { names: &["max"], arity: 1 }, FlagSpec { names: &["clear"], arity: 0 }];

/// `particleRamp -p PARTICLE -a scale|rgb|opacity -stops "pos value, pos value ..." [-max M] [-clear]`: per particle attributes driven by a ramp
/// over the age of the particle (0 birth, 1 death). rgb stops are "pos r g b"; `-max` multiplies a scale ramp.
pub fn particle_ramp(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, RAMP_SPEC)?;
    let name = p.string("particle").ok_or("Usage: particleRamp -p PARTICLE -a scale|rgb|opacity -stops \"0 0, 0.5 1, 1 0\"")?;
    let pid = resolve_targets(s, &[name.clone()])?;
    let shape = s.scene.dyn_target_of(pid[0]).ok_or_else(|| format!("{} is not a particle object", name))?;
    let attr = p.string("attr").ok_or("-a scale, rgb or opacity")?;
    let stops: Option<Ramp> = match p.string("stops") {
        Some(t) => {
            let mut v = Ramp::new();
            for part in t.split(',') {
                let n = nums(part);
                match (attr.as_str(), n.len()) {
                    ("rgb", 4) => v.push((n[0], Vec3::new(n[1], n[2], n[3]))),
                    ("scale" | "opacity", 2) => v.push((n[0], Vec3::splat(n[1]))),
                    _ => return Err(format!("Ramp stops: \"pos value\" for scale and opacity, \"pos r g b\" for rgb; got '{}'", part.trim())),
                }
            }
            v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            Some(v)
        }
        None => None,
    };
    if stops.is_none() && !p.has("clear") && p.num("max")?.is_none() {
        return Err("Give -stops, -max or -clear".to_string());
    }
    let max = p.num("max")?;
    if let Some(NodeData::Particle(pd)) = s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
        let slot = match attr.as_str() {
            "scale" => &mut pd.scale_ramp,
            "rgb" | "color" => &mut pd.color_ramp,
            "opacity" => &mut pd.opacity_ramp,
            o => return Err(format!("Unknown ramp '{}': scale, rgb or opacity", o)),
        };
        if p.has("clear") {
            *slot = None;
        } else if let Some(v) = stops {
            *slot = Some(v);
        }
        if let Some(m) = max {
            pd.scale_max = m;
        }
    }
    s.scene.evaluate_frame();
    Ok(vec![])
}

const EFFECT_SPEC: &[FlagSpec] = &[FlagSpec { names: &["type"], arity: 1 }, FlagSpec { names: &["name", "n"], arity: 1 }];

/// `particleEffect [-type smoke|fire] [-n NAME] [obj]`: Effects > Create Smoke / Create Fire. Makes a particle object, an emitter (from
/// the surface or curve of the object, else at the origin), a turbulence field and ramps for scale and color.
pub fn particle_effect(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, EFFECT_SPEC)?;
    let ty = p.string("type").unwrap_or_else(|| "smoke".to_string());
    let (pname, ename) = match ty.as_str() {
        "smoke" => ("SmokeParticles", "SmokeEmitter"),
        "fire" => ("FireParticles", "FireEmitter"),
        o => return Err(format!("Unknown effect '{}': smoke or fire", o)),
    };
    let args = p.arg_strings();
    let source = if args.is_empty() {
        s.scene.selection.iter().copied().find(|i| !s.scene.mesh_shapes_under(*i).is_empty() || !s.scene.curve_shapes_under(*i).is_empty())
    } else {
        Some(resolve_targets(s, &args)?[0])
    };
    let (pt, pshape) = s.scene.create_particle_object(Some(pname));
    let kind = match source {
        Some(o) if !s.scene.curve_shapes_under(o).is_empty() => 4,
        Some(_) => 3,
        None => 0,
    };
    let (et, e) = s.scene.create_particle_emitter(pshape, source, kind, Some(ename));
    let smoke = ty == "smoke";
    for (k, v) in [("rate", if smoke { 25.0 } else { 80.0 }), ("speed", if smoke { 1.0 } else { 2.5 }), ("spread", 0.3), ("directionY", 1.0)] {
        s.scene.set_attr(e, k, v)?;
    }
    for (k, v) in [("renderType", 3.0), ("lifespanMode", 1.0), ("lifespan", if smoke { 0.5 } else { 0.8 }), ("radius", if smoke { 0.4 } else { 0.25 })] {
        s.scene.set_attr(pshape, k, v)?;
    }
    if let Some(NodeData::Particle(pd)) = s.scene.nodes.get_mut(pshape).map(|n| &mut n.data) {
        pd.scale_ramp = Some(vec![(0.0, Vec3::splat(0.0)), (0.75, Vec3::splat(1.0)), (1.0, Vec3::splat(0.0))]);
        pd.scale_max = if smoke { 3.0 } else { 2.0 };
        pd.color_ramp = Some(if smoke { vec![(0.0, Vec3::new(0.35, 0.25, 0.15)), (1.0, Vec3::new(0.75, 0.75, 0.75))] } else { vec![(0.0, Vec3::new(1.0, 0.95, 0.4)), (0.5, Vec3::new(1.0, 0.45, 0.1)), (1.0, Vec3::new(0.3, 0.05, 0.0))] });
        pd.opacity_ramp = Some(vec![(0.0, Vec3::splat(0.0)), (0.2, Vec3::splat(0.8)), (1.0, Vec3::splat(0.0))]);
    }
    let (ft, f) = s.scene.create_field(5, Some(if smoke { "SmokeTurbulence" } else { "FireTurbulence" }));
    s.scene.set_attr(f, "magnitude", if smoke { 6.0 } else { 3.0 })?;
    s.scene.connect_field(f, &[pshape], true)?;
    let _ = ft;
    s.scene.selection = vec![pt];
    s.scene.evaluate_frame();
    Ok(vec![s.scene.name_of(pt), s.scene.name_of(et)])
}

/// `dynInfo [obj ...]`: particle counts and rigid body states, for scripts.
pub fn dyn_info(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let args = p.arg_strings();
    let ids: Vec<NodeId> = if args.is_empty() {
        s.scene.ids_where(|d| matches!(d, NodeData::Particle(_))).into_iter().filter_map(|i| s.scene.node(i).and_then(|n| n.parent)).chain(s.scene.ids_where(|d| matches!(d, NodeData::RigidBody(_))).into_iter().filter_map(|i| s.scene.node(i).and_then(|n| n.parent))).collect()
    } else {
        resolve_targets(s, &args)?
    };
    let mut out = Vec::new();
    for t in ids {
        if let Some(sh) = s.scene.dyn_target_of(t) {
            match s.scene.node(sh).map(|n| &n.data) {
                Some(NodeData::Particle(pd)) => out.push(format!("{}: {} particle(s)", s.scene.name_of(t), pd.sim.items.len())),
                _ => {
                    if let Some(st) = s.scene.rigid_state(t) {
                        out.push(format!("{}: position {:.3} {:.3} {:.3}, velocity {:.3} {:.3} {:.3}", s.scene.name_of(t), st.pos.x, st.pos.y, st.pos.z, st.vel.x, st.vel.y, st.vel.z));
                    }
                }
            }
        }
    }
    Ok(out)
}
