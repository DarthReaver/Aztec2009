//! Skeleton editing: orient, connect, mirror, insert, remove, reroot, limits, preferred angle.

use super::commands::{fmt_f, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::NodeId;
use crate::math::{Mat4, Quat, Vec3};
use crate::session::Session;

fn selected_joints(s: &Session, names: &[String]) -> Result<Vec<NodeId>, String> {
    let ids: Vec<NodeId> = if names.is_empty() {
        s.scene.selection.clone()
    } else {
        names.iter().map(|n| s.scene.find(n).ok_or_else(|| format!("No object matches name: {}", n))).collect::<Result<_, _>>()?
    };
    let joints: Vec<NodeId> = ids.into_iter().filter(|i| s.scene.is_joint(*i)).collect();
    if joints.is_empty() {
        return Err("Select a joint".to_string());
    }
    Ok(joints)
}

fn names_only(p: &super::parser::Parsed) -> Vec<String> {
    p.arg_strings()
}

const ORIENT_SPEC: &[FlagSpec] = &[FlagSpec { names: &["orientJoint", "oj"], arity: 1 }, FlagSpec { names: &["secondaryAxisOrient", "sao"], arity: 1 }, FlagSpec { names: &["children", "ch"], arity: 0 }];

/// `orientJoint [-oj xyz] [-sao yup] [joint]`: Skeleton > Orient Joint. The primary axis points to the first child,
/// the secondary axis toward the given world direction (yup, ydown, xup, xdown, zup, zdown, none).
pub fn orient_joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, ORIENT_SPEC)?;
    let order_s = p.string("orientJoint").unwrap_or_else(|| "xyz".to_string()).to_lowercase();
    let idx = |c: char| match c {
        'x' => Some(0),
        'y' => Some(1),
        'z' => Some(2),
        _ => None,
    };
    let order: Vec<usize> = order_s.chars().filter_map(idx).collect();
    if order.len() != 3 || order[0] == order[1] || order[1] == order[2] || order[0] == order[2] {
        return Err(format!("Bad orientation '{}': use a permutation of xyz such as xyz or yzx", order_s));
    }
    let up = match p.string("secondaryAxisOrient").unwrap_or_else(|| "yup".to_string()).to_lowercase().as_str() {
        "yup" => Some(Vec3::UP),
        "ydown" => Some(-Vec3::UP),
        "xup" => Some(Vec3::new(1.0, 0.0, 0.0)),
        "xdown" => Some(Vec3::new(-1.0, 0.0, 0.0)),
        "zup" => Some(Vec3::new(0.0, 0.0, 1.0)),
        "zdown" => Some(Vec3::new(0.0, 0.0, -1.0)),
        "none" => None,
        o => return Err(format!("Unknown secondary axis '{}': yup, ydown, xup, xdown, zup, zdown or none", o)),
    };
    let roots = selected_joints(s, &names_only(&p))?;
    let mut n = 0;
    for r in roots {
        n += s.scene.orient_joints(r, [order[0], order[1], order[2]], up)?;
    }
    Ok(vec![format!("Oriented {} joint(s)", n)])
}

/// `connectJoint CHILD PARENT` (also `parent`): Skeleton > Connect Joint, Parent Joint mode; the joints keep their place.
pub fn connect_joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = names_only(&p);
    let ids: Vec<NodeId> = if names.len() >= 2 {
        names.iter().map(|n| s.scene.find(n).ok_or_else(|| format!("No object matches name: {}", n))).collect::<Result<_, _>>()?
    } else {
        s.scene.selection.clone()
    };
    if ids.len() < 2 {
        return Err("Select the joint to move, then the joint to connect it to".to_string());
    }
    let (parent, kids) = (*ids.last().unwrap(), &ids[..ids.len() - 1]);
    if !s.scene.is_joint(parent) {
        return Err("The last joint named must be a joint".to_string());
    }
    for k in kids {
        s.scene.set_parent_keep_world(*k, Some(parent))?;
    }
    s.scene.mark_skins_dirty();
    Ok(vec![format!("Connected {} joint(s) to {}", kids.len(), s.scene.name_of(parent))])
}

/// `disconnectJoint [joint]`: the joint becomes the root of its own hierarchy.
pub fn disconnect_joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let joints = selected_joints(s, &names_only(&p))?;
    for j in &joints {
        s.scene.set_parent_keep_world(*j, None)?;
    }
    s.scene.mark_skins_dirty();
    Ok(vec![format!("Disconnected {} joint(s)", joints.len())])
}

/// `insertJoint [joint]`: Skeleton > Insert Joint. A new joint halfway to the joint's first child takes over that child.
pub fn insert_joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["name", "n"], arity: 1 }])?;
    let joints = selected_joints(s, &names_only(&p))?;
    let mut out = Vec::new();
    for j in joints {
        let kids: Vec<NodeId> = s.scene.node(j).map(|n| n.children.iter().copied().filter(|c| s.scene.is_joint(*c)).collect()).unwrap_or_default();
        let jw = s.scene.world_matrix(j);
        let at = match kids.first() {
            Some(k) => (jw.translation_part() + s.scene.world_matrix(*k).translation_part()) * 0.5,
            None => jw.translation_part() + jw.transform_dir(Vec3::new(1.0, 0.0, 0.0)).normalized() * 0.5,
        };
        let nj = super::cmd_rig::create_joint(s, at, p.string("name").as_deref(), Some(j))?;
        // the new joint continues the parent's orientation so the chain looks the same
        let (_, jo, _) = (s.scene.world_rotation(j).inverse() * s.scene.world_rotation(nj)).decompose();
        for (a, v) in ["X", "Y", "Z"].iter().zip([jo.x, jo.y, jo.z]) {
            let _ = s.scene.set_attr(nj, &format!("jointOrient{}", a), v);
        }
        for k in kids {
            s.scene.set_parent_keep_world(k, Some(nj))?;
        }
        out.push(s.scene.name_of(nj));
    }
    s.scene.mark_skins_dirty();
    Ok(out)
}

/// `removeJoint [joint]`: Skeleton > Remove Joint; its children go to its parent and keep their place.
pub fn remove_joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let joints = selected_joints(s, &names_only(&p))?;
    for j in &joints {
        let parent = s.scene.node(*j).and_then(|n| n.parent);
        let kids: Vec<NodeId> = s.scene.node(*j).map(|n| n.children.clone()).unwrap_or_default();
        for k in kids {
            s.scene.set_parent_keep_world(k, parent)?;
        }
        s.scene.delete_node(*j)?;
    }
    s.scene.mark_skins_dirty();
    Ok(vec![format!("Removed {} joint(s)", joints.len())])
}

/// `reroot joint`: Skeleton > Reroot Skeleton. The joint becomes the root; its former ancestors hang below it.
pub fn reroot(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let joint = *selected_joints(s, &names_only(&p))?.first().unwrap();
    // chain from the joint up to the root
    let mut chain = vec![joint];
    let mut cur = joint;
    while let Some(par) = s.scene.node(cur).and_then(|n| n.parent) {
        if !s.scene.is_joint(par) {
            break;
        }
        chain.push(par);
        cur = par;
    }
    if chain.len() < 2 {
        return Err("The joint is already the root".to_string());
    }
    // reverse the parent links, one by one from the top so no joint is parented below itself
    s.scene.set_parent_keep_world(joint, s.scene.node(*chain.last().unwrap()).and_then(|n| n.parent))?;
    for w in chain.windows(2) {
        // w[1] was the parent of w[0]; now w[0] is the parent of w[1]
        s.scene.set_parent_keep_world(w[1], Some(w[0]))?;
    }
    s.scene.mark_skins_dirty();
    Ok(vec![format!("{} is the new root of {} joint(s)", s.scene.name_of(joint), chain.len())])
}

const MIRROR_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["axis", "ax"], arity: 1 },
    FlagSpec { names: &["mirrorYZ", "myz"], arity: 0 },
    FlagSpec { names: &["mirrorXY", "mxy"], arity: 0 },
    FlagSpec { names: &["mirrorXZ", "mxz"], arity: 0 },
    FlagSpec { names: &["mirrorBehavior", "mb"], arity: 0 },
    FlagSpec { names: &["searchReplace", "sr"], arity: 2 },
];

/// `mirrorJoint [-mirrorYZ|-mirrorXY|-mirrorXZ] [-mirrorBehavior] [-searchReplace "l_" "r_"] [joint]`: Skeleton > Mirror
/// Joint. The default mirrors across the YZ plane (x flips). Orientation mode (default) keeps the world orientation of every
/// joint, Behavior mirrors the axes too so rotating both sides the same amount moves them symmetrically.
pub fn mirror_joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, MIRROR_SPEC)?;
    let axis = if p.has("mirrorXY") {
        2
    } else if p.has("mirrorXZ") {
        1
    } else {
        match p.string("axis").unwrap_or_else(|| "x".into()).to_lowercase().as_str() {
            "y" => 1,
            "z" => 2,
            _ => 0,
        }
    };
    let behavior = p.has("mirrorBehavior");
    let sr = p.get("searchReplace").map(|v| (v[0].as_string(), v[1].as_string()));
    let roots: Vec<NodeId> = selected_joints(s, &p.arg_strings())?;
    let flip = |mut v: Vec3| {
        match axis {
            0 => v.x = -v.x,
            1 => v.y = -v.y,
            _ => v.z = -v.z,
        }
        v
    };
    let mut created = Vec::new();
    for root in roots {
        let parent = s.scene.node(root).and_then(|n| n.parent);
        let mut map: Vec<(NodeId, NodeId)> = Vec::new();
        for j in s.scene.joints_under(&[root]) {
            let src_pos = s.scene.world_matrix(j).translation_part();
            let src_rot = s.scene.world_rotation(j);
            let cols = [Vec3::new(src_rot.m[0][0], src_rot.m[0][1], src_rot.m[0][2]), Vec3::new(src_rot.m[1][0], src_rot.m[1][1], src_rot.m[1][2]), Vec3::new(src_rot.m[2][0], src_rot.m[2][1], src_rot.m[2][2])];
            let world_rot = if behavior {
                // every axis reflected and reversed: the frame stays right handed and the same rotation moves both sides symmetrically
                let (a, b, c) = (-flip(cols[0]), -flip(cols[1]), -flip(cols[2]));
                let mut m = Mat4::identity();
                for (i, v) in [a, b, c].iter().enumerate() {
                    m.m[i][0] = v.x;
                    m.m[i][1] = v.y;
                    m.m[i][2] = v.z;
                }
                m
            } else {
                src_rot
            };
            let par = if j == root { parent } else { s.scene.node(j).and_then(|n| n.parent).and_then(|jp| map.iter().find(|(o, _)| *o == jp).map(|(_, n)| *n)) };
            let base = s.scene.name_of(j);
            let name = match &sr {
                Some((a, b)) if base.contains(a.as_str()) => base.replacen(a.as_str(), b, 1),
                _ => format!("{}_mirror", base),
            };
            let nj = super::cmd_rig::create_joint(s, flip(src_pos), Some(&name), par)?;
            let prot = match par {
                Some(pp) => s.scene.world_rotation(pp),
                None => Mat4::identity(),
            };
            let (_, jo, _) = (prot.inverse() * world_rot).decompose();
            for (a, v) in ["X", "Y", "Z"].iter().zip([jo.x, jo.y, jo.z]) {
                s.scene.set_attr(nj, &format!("jointOrient{}", a), v)?;
            }
            // the translation was computed before the orientation existed: place the joint again
            let pw = match par {
                Some(pp) => s.scene.world_matrix(pp),
                None => Mat4::identity(),
            };
            let lp = pw.inverse().transform_point(flip(src_pos));
            for (a, v) in ["X", "Y", "Z"].iter().zip([lp.x, lp.y, lp.z]) {
                s.scene.set_attr(nj, &format!("translate{}", a), v)?;
            }
            if let Ok(r) = s.scene.get_attr(j, "radius") {
                let _ = s.scene.set_attr(nj, "radius", r);
            }
            map.push((j, nj));
            if j == root {
                created.push(nj);
            }
        }
    }
    s.scene.selection = created.clone();
    Ok(created.iter().map(|i| s.scene.name_of(*i)).collect())
}

const LIMIT_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["translationX", "tx"], arity: 2 },
    FlagSpec { names: &["translationY", "ty"], arity: 2 },
    FlagSpec { names: &["translationZ", "tz"], arity: 2 },
    FlagSpec { names: &["rotationX", "rx"], arity: 2 },
    FlagSpec { names: &["rotationY", "ry"], arity: 2 },
    FlagSpec { names: &["rotationZ", "rz"], arity: 2 },
    FlagSpec { names: &["scaleX", "sx"], arity: 2 },
    FlagSpec { names: &["scaleY", "sy"], arity: 2 },
    FlagSpec { names: &["scaleZ", "sz"], arity: 2 },
    FlagSpec { names: &["query", "q"], arity: 0 },
];

/// `transformLimits [-rx MIN MAX] [-ry ...] [-tx ...] [-sx ...] [-q] object`: Limit Information. `none` removes a bound.
/// Limits bind values set by hand, by animation, IK and constraints.
pub fn transform_limits(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, LIMIT_SPEC)?;
    let ids = if p.arg_strings().is_empty() { s.scene.selection.clone() } else { p.arg_strings().iter().map(|n| s.scene.find(n).ok_or_else(|| format!("No object matches name: {}", n))).collect::<Result<_, _>>()? };
    if ids.is_empty() {
        return Err("Select an object".to_string());
    }
    let table = [("translationX", "translateX"), ("translationY", "translateY"), ("translationZ", "translateZ"), ("rotationX", "rotateX"), ("rotationY", "rotateY"), ("rotationZ", "rotateZ"), ("scaleX", "scaleX"), ("scaleY", "scaleY"), ("scaleZ", "scaleZ")];
    let mut out = Vec::new();
    for id in ids {
        for (flag, attr) in table {
            if p.has("query") {
                if let Some(pl) = s.scene.node(id).and_then(|n| n.plug(attr)) {
                    if pl.min.is_some() || pl.max.is_some() {
                        out.push(format!("{}.{}: {} .. {}", s.scene.name_of(id), attr, pl.min.map(fmt_f).unwrap_or_else(|| "none".into()), pl.max.map(fmt_f).unwrap_or_else(|| "none".into())));
                    }
                }
                continue;
            }
            if let Some(v) = p.get(flag) {
                let parse = |t: &super::parser::Value| -> Result<Option<f32>, String> {
                    let st = t.as_string();
                    if st == "none" {
                        Ok(None)
                    } else {
                        Ok(Some(t.as_f32()?))
                    }
                };
                let (lo, hi) = (parse(&v[0])?, parse(&v[1])?);
                if let (Some(a), Some(b)) = (lo, hi) {
                    if a > b {
                        return Err(format!("The minimum of {} is above its maximum", flag));
                    }
                }
                if let Some(pl) = s.scene.nodes.get_mut(id).and_then(|n| n.plug_mut(attr)) {
                    pl.min = lo;
                    pl.max = hi;
                    let clamped = pl.value.clamp(lo.unwrap_or(f32::MIN), hi.unwrap_or(f32::MAX));
                    let rest = pl.rest.clamp(lo.unwrap_or(f32::MIN), hi.unwrap_or(f32::MAX));
                    pl.value = clamped;
                    pl.rest = rest;
                }
            }
        }
    }
    s.scene.evaluate_frame();
    if p.has("query") {
        return Ok(out);
    }
    Ok(vec!["Limits set".to_string()])
}

/// `setPreferredAngle [joint]` stores the current rotation as the preferred angle; `assumePreferredAngle` goes back to it.
pub fn preferred_angle(s: &mut Session, rest: &[Token], assume: bool) -> Out {
    let p = parse_args(rest, &[])?;
    let joints = selected_joints(s, &names_only(&p))?;
    let mut n = 0;
    for j in s.scene.joints_under(&joints) {
        for a in ["X", "Y", "Z"] {
            let (from, to) = if assume { (format!("preferredAngle{}", a), format!("rotate{}", a)) } else { (format!("rotate{}", a), format!("preferredAngle{}", a)) };
            let v = s.scene.get_attr(j, &from)?;
            s.scene.set_attr(j, &to, v)?;
        }
        n += 1;
    }
    s.scene.mark_skins_dirty();
    s.scene.evaluate_frame();
    Ok(vec![format!("{} {} joint(s)", if assume { "Assumed the preferred angle of" } else { "Stored the preferred angle of" }, n)])
}

/// `jointDisplayScale SCALE`: Display > Animation > Joint Size (0.01 to 100).
pub fn joint_display_scale(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    match p.args.first() {
        Some(v) => {
            let x = v.as_f32()?;
            if !(0.01..=100.0).contains(&x) {
                return Err("The joint size must be between 0.01 and 100".to_string());
            }
            s.scene.joint_scale = x;
            Ok(vec![format!("Joint size {}", fmt_f(x))])
        }
        None => Ok(vec![format!("Joint size {}", fmt_f(s.scene.joint_scale))]),
    }
}

/// `poleVectorConstraint target ikHandle`: the knee of a rotate plane IK chain points toward the target.
pub fn pole_vector(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = p.arg_strings();
    let ids: Vec<NodeId> = if names.len() >= 2 { names.iter().map(|n| s.scene.find(n).ok_or_else(|| format!("No object matches name: {}", n))).collect::<Result<_, _>>()? } else { s.scene.selection.clone() };
    if ids.len() < 2 {
        return Err("Select the pole target (a locator), then the IK handle".to_string());
    }
    let (target, handle) = (ids[0], *ids.last().unwrap());
    if !matches!(s.scene.node(handle).map(|n| &n.data), Some(crate::dag::nodes::NodeData::IkHandle(_))) {
        return Err("The last object must be an IK handle".to_string());
    }
    if let Some(crate::dag::nodes::NodeData::IkHandle(d)) = s.scene.nodes.get_mut(handle).map(|n| &mut n.data) {
        if d.solver != 0 {
            return Err("Pole vectors work with the rotate plane solver (ikRPsolver)".to_string());
        }
        d.pole = Some(target);
    }
    s.scene.evaluate_frame();
    Ok(vec![format!("{} is the pole vector of {}", s.scene.name_of(target), s.scene.name_of(handle))])
}

#[allow(dead_code)]
fn _unused(_: Quat) {}
