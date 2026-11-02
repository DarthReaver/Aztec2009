//! Skeleton commands: joint, ikHandle, bindSkin, detachSkin, mirrorJoint.

use super::commands::{fmt_f, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::{IkData, NodeData};
use crate::dag::NodeId;
use crate::math::Vec3;
use crate::session::Session;

const JOINT_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["p", "position"], arity: 3 },
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["r", "radius"], arity: 1 },
];

/// Create a joint at a world position, parented to the selected joint when there is one.
pub fn create_joint(s: &mut Session, pos: Vec3, name: Option<&str>, parent: Option<NodeId>) -> Result<NodeId, String> {
    let id = match name {
        Some(n) => s.scene.create_node(NodeData::Joint, n, parent, false),
        None => s.scene.create_node(NodeData::Joint, "joint", parent, true),
    };
    let local = match parent {
        Some(p) => s.scene.world_matrix(p).inverse().transform_point(pos),
        None => pos,
    };
    s.scene.set_attr(id, "translateX", local.x)?;
    s.scene.set_attr(id, "translateY", local.y)?;
    s.scene.set_attr(id, "translateZ", local.z)?;
    Ok(id)
}

pub fn joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, JOINT_SPEC)?;
    let pos = match p.get("p") {
        Some(v) if v.len() >= 3 => Vec3::new(v[0].as_f32()?, v[1].as_f32()?, v[2].as_f32()?),
        _ => Vec3::ZERO,
    };
    let parent = s.scene.selection.first().copied().filter(|id| s.scene.is_joint(*id));
    let name = p.string("name");
    let id = create_joint(s, pos, name.as_deref(), parent)?;
    if let Some(r) = p.num("radius")? {
        s.scene.set_attr(id, "radius", r)?;
    }
    s.scene.selection = vec![id];
    Ok(vec![s.scene.name_of(id)])
}

const IK_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["sj", "startJoint"], arity: 1 },
    FlagSpec { names: &["ee", "endEffector"], arity: 1 },
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["solver", "sol"], arity: 1 },
    FlagSpec { names: &["curve", "c"], arity: 1 },
];

pub fn create_ik(s: &mut Session, start: NodeId, end: NodeId, name: Option<&str>) -> Result<NodeId, String> {
    create_ik_ex(s, start, end, name, 0, None)
}

/// An IK handle with a solver (0 rotate plane, 1 single chain, 2 spline with its curve shape).
pub fn create_ik_ex(s: &mut Session, start: NodeId, end: NodeId, name: Option<&str>, solver: u8, curve: Option<NodeId>) -> Result<NodeId, String> {
    if !s.scene.is_joint(start) || !s.scene.is_joint(end) {
        return Err("The IK handle needs two joints".to_string());
    }
    if !s.scene.descendants(start).contains(&end) || start == end {
        return Err("The end joint must be below the start joint".to_string());
    }
    let pos = s.scene.world_matrix(end).translation_part();
    if solver == 2 && curve.is_none() {
        return Err("The spline solver needs a curve: ikHandle -sol ikSplineSolver -curve NAME".to_string());
    }
    let data = IkData { start: Some(start), end: Some(end), solver, curve, ..Default::default() };
    let h = match name {
        Some(n) => s.scene.create_node(NodeData::IkHandle(Box::new(data)), n, None, false),
        None => s.scene.create_node(NodeData::IkHandle(Box::new(data)), "ikHandle", None, true),
    };
    s.scene.set_attr(h, "translateX", pos.x)?;
    s.scene.set_attr(h, "translateY", pos.y)?;
    s.scene.set_attr(h, "translateZ", pos.z)?;
    Ok(h)
}

pub fn ik_handle(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, IK_SPEC)?;
    let find = |s: &Session, n: Option<String>| -> Result<NodeId, String> {
        let n = n.ok_or("Usage: ikHandle -sj startJoint -ee endJoint")?;
        s.scene.find(&n).ok_or_else(|| format!("No object matches name: {}", n))
    };
    let a = find(s, p.string("sj"))?;
    let b = find(s, p.string("ee"))?;
    let name = p.string("name");
    let solver = match p.string("solver").unwrap_or_else(|| "ikRPsolver".to_string()).as_str() {
        "ikRPsolver" | "rp" => 0,
        "ikSCsolver" | "sc" => 1,
        "ikSplineSolver" | "spline" => 2,
        o => return Err(format!("Unknown solver '{}': ikRPsolver, ikSCsolver or ikSplineSolver", o)),
    };
    let curve = match p.string("curve") {
        Some(c) => Some(s.scene.find(&c).and_then(|t| s.scene.curve_shapes_under(t).first().copied()).ok_or_else(|| format!("No curve named '{}'", c))?),
        None => None,
    };
    let h = create_ik_ex(s, a, b, name.as_deref(), solver, curve)?;
    s.scene.selection = vec![h];
    Ok(vec![s.scene.name_of(h)])
}

/// Bind the selected meshes to every joint in the selected skeleton hierarchies.
pub fn bind_skin(s: &mut Session, _rest: &[Token]) -> Out {
    let sel = s.scene.selection.clone();
    let joints = s.scene.joints_under(&sel);
    let mut shapes: Vec<NodeId> = Vec::new();
    for id in &sel {
        if s.scene.is_joint(*id) {
            continue;
        }
        shapes.extend(s.scene.mesh_shapes_under(*id));
    }
    if shapes.is_empty() {
        return Err("Select the skeleton and then the mesh(es) to bind".to_string());
    }
    if joints.is_empty() {
        return Err("Select a joint (the root of the skeleton) as well".to_string());
    }
    let mut out = Vec::new();
    for sh in shapes {
        let n = s.scene.bind_skin(sh, &joints)?;
        out.push(format!("skinCluster {} bound to {} joints", s.scene.name_of(n), joints.len()));
    }
    Ok(out)
}

pub fn detach_skin(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = p.arg_strings();
    let ids: Vec<NodeId> = if names.is_empty() {
        s.scene.selection.clone()
    } else {
        names.iter().filter_map(|n| s.scene.find(n)).collect()
    };
    let mut n = 0;
    for id in ids {
        for sh in s.scene.mesh_shapes_under(id) {
            if s.scene.detach_skin(sh).is_ok() {
                n += 1;
            }
        }
    }
    if n == 0 {
        return Err("No bound mesh selected".to_string());
    }
    Ok(vec![format!("Detached {} mesh(es)", n)])
}

const MIRROR_SPEC: &[FlagSpec] = &[FlagSpec { names: &["axis", "ax"], arity: 1 }];

/// Mirror the selected joint hierarchy across a world plane (x by default).
pub fn mirror_joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, MIRROR_SPEC)?;
    let axis = match p.string("axis").unwrap_or_else(|| "x".into()).to_lowercase().as_str() {
        "y" => 1,
        "z" => 2,
        _ => 0,
    };
    let roots: Vec<NodeId> = s.scene.selection.iter().copied().filter(|i| s.scene.is_joint(*i)).collect();
    if roots.is_empty() {
        return Err("Select a joint to mirror".to_string());
    }
    let mut created = Vec::new();
    for root in roots {
        let parent = s.scene.node(root).and_then(|n| n.parent);
        let mut map: Vec<(NodeId, NodeId)> = Vec::new();
        for j in s.scene.joints_under(&[root]) {
            let w = s.scene.world_matrix(j).translation_part();
            let mut m = w;
            match axis {
                0 => m.x = -m.x,
                1 => m.y = -m.y,
                _ => m.z = -m.z,
            }
            let par = if j == root {
                parent
            } else {
                let jp = s.scene.node(j).and_then(|n| n.parent);
                jp.and_then(|jp| map.iter().find(|(o, _)| *o == jp).map(|(_, n)| *n))
            };
            let base = s.scene.name_of(j);
            let nj = create_joint(s, m, Some(&format!("{}_mirror", base)), par)?;
            map.push((j, nj));
            if j == root {
                created.push(nj);
            }
        }
    }
    s.scene.selection = created.clone();
    Ok(created.iter().map(|i| format!("{}", s.scene.name_of(*i))).collect())
}

pub fn list_joints(s: &mut Session) -> Out {
    let ids = s.scene.ids_where(|d| matches!(d, NodeData::Joint));
    Ok(ids
        .iter()
        .map(|i| {
            let p = s.scene.world_matrix(*i).translation_part();
            format!("{}  ({} {} {})", s.scene.name_of(*i), fmt_f(p.x), fmt_f(p.y), fmt_f(p.z))
        })
        .collect())
}
