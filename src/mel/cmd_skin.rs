//! Skinning commands: bind options, influences, weight tools, painting, bind pose.

use super::commands::{fmt_f, resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::NodeData;
use crate::dag::skinning::{weight_of, PaintOp};
use crate::dag::NodeId;
use crate::math::Vec3;
use crate::session::Session;

const BIND_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["rigid", "r"], arity: 0 },
    FlagSpec { names: &["smooth", "s"], arity: 0 },
    FlagSpec { names: &["maxInfluences", "mi"], arity: 1 },
    FlagSpec { names: &["dropoff", "dr"], arity: 1 },
    FlagSpec { names: &["toSelectedBones", "tsb"], arity: 0 },
];

/// `bindSkin [-rigid | -smooth] [-maxInfluences N] [-dropoff D] [-toSelectedBones]`: Skin > Bind Skin. Select the
/// skeleton (root joint) and the meshes. Rigid: every vertex follows its nearest bone with weight 1.
pub fn bind_skin(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, BIND_SPEC)?;
    let rigid = p.has("rigid");
    let max_inf = p.num("maxInfluences")?.unwrap_or(4.0).round().clamp(1.0, 8.0) as usize;
    let dropoff = p.num("dropoff")?.unwrap_or(4.0).clamp(0.5, 10.0);
    let sel = s.scene.selection.clone();
    let joints = if p.has("toSelectedBones") { sel.iter().copied().filter(|i| s.scene.is_joint(*i)).collect() } else { s.scene.joints_under(&sel) };
    let mut shapes: Vec<NodeId> = Vec::new();
    // a lattice is bound rigidly to the joints (Lattice binding)
    let lattices: Vec<NodeId> = sel.iter().flat_map(|id| s.scene.lattice_shapes_under(*id)).collect();
    if !lattices.is_empty() {
        if joints.is_empty() {
            return Err("Select the skeleton root as well as the lattice".to_string());
        }
        for l in &lattices {
            s.scene.bind_lattice(*l, &joints)?;
        }
        return Ok(vec![format!("{} lattice(s) bound rigidly to {} joints", lattices.len(), joints.len())]);
    }
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
        if s.scene.node(sh).map(|n| matches!(&n.data, NodeData::Mesh(md) if md.nurbs.is_some())).unwrap_or(false) {
            return Err("NURBS surfaces cannot be skinned here: convert them to polygons first (nurbsToPoly)".to_string());
        }
        let n = s.scene.bind_skin_ex(sh, &joints, rigid, max_inf, dropoff)?;
        out.push(format!("skinCluster {} bound to {} joints ({})", s.scene.name_of(n), joints.len(), if rigid { "rigid" } else { "smooth" }));
    }
    Ok(out)
}

/// The skinned mesh shape named (or selected).
fn skinned_shape(s: &Session, names: &[String]) -> Result<NodeId, String> {
    let ids = resolve_targets(s, names)?;
    for id in ids {
        for sh in s.scene.mesh_shapes_under(id) {
            if s.scene.skin_of(sh).is_some() {
                return Ok(sh);
            }
        }
    }
    Err("Select a mesh that is bound to a skeleton".to_string())
}

fn names_only(p: &super::parser::Parsed) -> Vec<String> {
    p.arg_strings().into_iter().filter(|a| !a.contains('[')).collect()
}

const INF_SPEC: &[FlagSpec] = &[FlagSpec { names: &["add", "a"], arity: 1 }, FlagSpec { names: &["remove", "r"], arity: 1 }];

/// `skinInfluence -add JOINT | -remove JOINT mesh`: Add / Remove Influence.
pub fn influence(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, INF_SPEC)?;
    let sh = skinned_shape(s, &names_only(&p))?;
    let (name, add) = match (p.string("add"), p.string("remove")) {
        (Some(a), _) => (a, true),
        (None, Some(r)) => (r, false),
        _ => return Err("Usage: skinInfluence -add JOINT mesh   |   skinInfluence -remove JOINT mesh".to_string()),
    };
    let j = s.scene.find(&name).filter(|i| s.scene.is_joint(*i)).ok_or_else(|| format!("'{}' is not a joint", name))?;
    if add {
        s.scene.add_influence(sh, j)?;
    } else {
        s.scene.remove_influence(sh, j)?;
    }
    Ok(vec![format!("{} influence {}", if add { "Added" } else { "Removed" }, name)])
}

/// `skinInfo [mesh]`: the influences and how many a vertex uses.
pub fn info(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let sh = skinned_shape(s, &names_only(&p))?;
    let sk = s.scene.skin_data(sh).ok_or("The mesh is not bound to a skeleton")?;
    let names: Vec<String> = sk.joints.iter().map(|j| s.scene.name_of(*j)).collect();
    let used = sk.weights.iter().map(|w| w.len()).max().unwrap_or(0);
    let avg = sk.weights.iter().map(|w| w.len()).sum::<usize>() as f32 / sk.weights.len().max(1) as f32;
    Ok(vec![
        format!("{}: {} influences: {}", s.scene.name_of(sh), names.len(), names.join(" ")),
        format!("{} vertices, at most {} influences per vertex (average {}), {}", sk.weights.len(), used, fmt_f((avg * 100.0).round() / 100.0), if sk.rigid { "rigid" } else { "smooth" }),
    ])
}

/// `skinNormalize [mesh]` / `skinPrune [-t THRESHOLD] [mesh]` / `skinReset [mesh]` / `skinMaxInfluences N [mesh]`.
pub fn weight_tool(s: &mut Session, name: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["threshold", "t"], arity: 1 }, FlagSpec { names: &["iterations", "i"], arity: 1 }, FlagSpec { names: &["strength", "st"], arity: 1 }])?;
    let args = names_only(&p);
    match name {
        "skinMaxInfluences" => {
            let n = args.first().and_then(|a| a.parse::<usize>().ok()).ok_or("Usage: skinMaxInfluences N [mesh]")?;
            let sh = skinned_shape(s, &args[1..])?;
            s.scene.limit_influences(sh, n)?;
            return Ok(vec![format!("At most {} influences per vertex", n.clamp(1, 8))]);
        }
        _ => {}
    }
    let sh = skinned_shape(s, &args)?;
    match name {
        "skinNormalize" => {
            s.scene.normalize_skin(sh)?;
            Ok(vec!["Normalized".to_string()])
        }
        "skinPrune" => {
            let n = s.scene.prune_skin(sh, p.num("threshold")?.unwrap_or(0.01).clamp(0.0, 0.5))?;
            Ok(vec![format!("Pruned {} weight(s)", n)])
        }
        "skinReset" => {
            s.scene.reset_skin(sh)?;
            Ok(vec!["Weights reset to the bind defaults".to_string()])
        }
        _ => unreachable!(),
    }
}

/// `skinSmooth [-i ITERATIONS] [-st STRENGTH] [mesh.vtx[..]]`: Smooth Skin Weights on the selected vertices (or all).
pub fn smooth(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["iterations", "i"], arity: 1 }, FlagSpec { names: &["strength", "st"], arity: 1 }])?;
    let args = p.arg_strings();
    let (sh, verts): (NodeId, Vec<u32>) = if args.iter().any(|a| a.contains('[')) {
        super::cmd_model::comp_verts(s, &args)?
    } else {
        let sh = skinned_shape(s, &args)?;
        let n = s.scene.skin_data(sh).map(|k| k.weights.len()).unwrap_or(0);
        (sh, (0..n as u32).collect())
    };
    let it = p.num("iterations")?.unwrap_or(1.0).clamp(1.0, 50.0) as usize;
    s.scene.smooth_skin(sh, &verts, it, p.num("strength")?.unwrap_or(0.5).clamp(0.0, 1.0))?;
    Ok(vec![format!("Smoothed {} vertices", verts.len())])
}

/// `skinMirror [-axis x|y|z] [-direction pos|neg] [-tolerance T] [mesh]`: Mirror Skin Weights across the mesh's local plane.
pub fn mirror(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["axis", "ax"], arity: 1 }, FlagSpec { names: &["direction", "d"], arity: 1 }, FlagSpec { names: &["tolerance", "t"], arity: 1 }])?;
    let sh = skinned_shape(s, &names_only(&p))?;
    let axis = match p.string("axis").unwrap_or_else(|| "x".into()).to_lowercase().as_str() {
        "y" => 1,
        "z" => 2,
        _ => 0,
    };
    let pos_to_neg = p.string("direction").map(|d| !d.to_lowercase().starts_with("neg")).unwrap_or(true);
    let n = s.scene.mirror_skin(sh, axis, pos_to_neg, p.num("tolerance")?.unwrap_or(0.001).max(1e-5))?;
    Ok(vec![format!("Mirrored the weights of {} vertices", n)])
}

/// `skinCopy SOURCE TARGET`: Copy Skin Weights (nearest vertex, the target gets the source's influences).
pub fn copy(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = p.arg_strings();
    if names.len() != 2 {
        return Err("Usage: skinCopy SOURCE TARGET".to_string());
    }
    let src = skinned_shape(s, &names[..1])?;
    let dst_obj = s.scene.find(&names[1]).ok_or_else(|| format!("No object matches name: {}", names[1]))?;
    let dst = *s.scene.mesh_shapes_under(dst_obj).first().ok_or("The target is not a mesh")?;
    let n = s.scene.copy_skin_weights(src, dst)?;
    Ok(vec![format!("Copied the weights to {} vertices", n)])
}

/// `skinExport -file F mesh` / `skinImport -file F mesh`: Export / Import Skin Weight Maps (text).
pub fn export_import(s: &mut Session, rest: &[Token], export: bool) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["file", "f"], arity: 1 }])?;
    let file = p.string("file").ok_or("Usage: skinExport -file weights.txt mesh")?;
    let sh = skinned_shape(s, &names_only(&p))?;
    if export {
        let sk = s.scene.skin_data(sh).ok_or("The mesh is not bound to a skeleton")?;
        let mut text = String::from("# Aztec 2009 skin weights\n");
        for j in &sk.joints {
            text.push_str(&format!("joint {}\n", s.scene.name_of(*j)));
        }
        for (v, w) in sk.weights.iter().enumerate() {
            let list: Vec<String> = w.iter().map(|(j, x)| format!("{}:{}", j, x)).collect();
            text.push_str(&format!("v {} {}\n", v, list.join(" ")));
        }
        std::fs::write(&file, text).map_err(|e| format!("Cannot write {}: {}", file, e))?;
        return Ok(vec![format!("Exported the weights of {} vertices to {}", sk.weights.len(), file)]);
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("Cannot read {}: {}", file, e))?;
    let mut file_joints: Vec<String> = Vec::new();
    let mut rows: Vec<(usize, Vec<(usize, f32)>)> = Vec::new();
    for l in text.lines() {
        let mut it = l.split_whitespace();
        match it.next() {
            Some("joint") => file_joints.push(it.next().unwrap_or("").to_string()),
            Some("v") => {
                let v: usize = it.next().and_then(|x| x.parse().ok()).ok_or("Bad vertex line")?;
                let w: Vec<(usize, f32)> = it.filter_map(|t| t.split_once(':')).filter_map(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).collect();
                rows.push((v, w));
            }
            _ => {}
        }
    }
    // joints by name; unknown ones that exist in the scene become influences
    let mut map: Vec<Option<u16>> = Vec::new();
    for name in &file_joints {
        let jid = s.scene.find(name).filter(|i| s.scene.is_joint(*i));
        let idx = match jid {
            Some(j) => {
                if !s.scene.skin_data(sh).map(|k| k.joints.contains(&j)).unwrap_or(false) {
                    s.scene.add_influence(sh, j)?;
                }
                s.scene.skin_data(sh).and_then(|k| k.joints.iter().position(|x| *x == j)).map(|x| x as u16)
            }
            None => None,
        };
        map.push(idx);
    }
    let n = s.scene.skin_data(sh).map(|k| k.weights.len()).unwrap_or(0);
    let mut applied = 0;
    s.scene.with_skin(sh, |sk| {
        for (v, w) in &rows {
            if *v >= n {
                continue;
            }
            let list: Vec<(u16, f32)> = w.iter().filter_map(|(j, x)| map.get(*j).copied().flatten().map(|m| (m, *x))).collect();
            if !list.is_empty() {
                sk.weights[*v] = list;
                applied += 1;
            }
        }
        Ok(())
    })?;
    Ok(vec![format!("Imported the weights of {} vertices from {}", applied, file)])
}

const PAINT_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["joint", "j"], arity: 1 },
    FlagSpec { names: &["value", "v"], arity: 1 },
    FlagSpec { names: &["radius", "r"], arity: 1 },
    FlagSpec { names: &["operation", "op"], arity: 1 },
    FlagSpec { names: &["strength", "st"], arity: 1 },
    FlagSpec { names: &["at"], arity: 3 },
];

/// `skinPaint -joint J -at X Y Z [-value 1] [-radius 0.5] [-op replace|add|scale|smooth] [-st 1] mesh`: one dab of the
/// Paint Skin Weights tool at a world point.
pub fn paint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, PAINT_SPEC)?;
    let sh = skinned_shape(s, &names_only(&p))?;
    let jn = p.string("joint").ok_or("Usage: skinPaint -joint J -at X Y Z [-value V] [-radius R] [-op replace] mesh")?;
    let joint = s.scene.find(&jn).filter(|i| s.scene.is_joint(*i)).ok_or_else(|| format!("'{}' is not a joint", jn))?;
    let at = p.get("at").ok_or("-at X Y Z is needed")?;
    let world = Vec3::new(at[0].as_f32()?, at[1].as_f32()?, at[2].as_f32()?);
    let op = PaintOp::parse(&p.string("operation").unwrap_or_else(|| "replace".into())).ok_or("-op must be replace, add, scale or smooth")?;
    let parent = s.scene.node(sh).and_then(|n| n.parent);
    let local = match parent {
        Some(pa) => s.scene.world_matrix(pa).inverse().transform_point(world),
        None => world,
    };
    let n = s.scene.paint_skin(sh, local, p.num("radius")?.unwrap_or(0.5).max(1e-3), joint, p.num("value")?.unwrap_or(1.0), op, p.num("strength")?.unwrap_or(1.0).clamp(0.0, 1.0))?;
    Ok(vec![format!("Painted {} vertices", n)])
}

/// `skinWeight -q -vertex N [mesh]`: the influences of one vertex.
pub fn weights_query(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["vertex", "v"], arity: 1 }, FlagSpec { names: &["joint", "j"], arity: 1 }])?;
    let sh = skinned_shape(s, &names_only(&p))?;
    let v = p.num("vertex")?.ok_or("Usage: skinWeights -v N [-j JOINT] [mesh]")? as usize;
    let sk = s.scene.skin_data(sh).ok_or("The mesh is not bound to a skeleton")?;
    let w = sk.weights.get(v).ok_or_else(|| format!("Vertex {} does not exist ({} vertices)", v, sk.weights.len()))?;
    if let Some(jn) = p.string("joint") {
        let j = s.scene.find(&jn).and_then(|j| sk.joints.iter().position(|x| *x == j)).ok_or_else(|| format!("'{}' is not an influence", jn))?;
        return Ok(vec![fmt_f(weight_of(w, j as u16))]);
    }
    Ok(w.iter().map(|(j, x)| format!("{} {}", s.scene.name_of(sk.joints[*j as usize]), fmt_f(*x))).collect())
}

/// `goToBindPose [mesh]`: Skin > Go to Bind Pose.
pub fn go_to_bind_pose(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let sh = skinned_shape(s, &names_only(&p))?;
    let n = s.scene.go_to_bind_pose(sh)?;
    s.scene.evaluate_frame();
    Ok(vec![format!("{} joint(s) are in the bind pose", n)])
}

/// `skinMoveJoint -t X Y Z joint`: Move Skinned Joints Tool; the skin keeps its shape.
pub fn move_joint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["translate", "t"], arity: 3 }])?;
    let t = p.get("translate").ok_or("Usage: skinMoveJoint -t X Y Z joint")?;
    let d = Vec3::new(t[0].as_f32()?, t[1].as_f32()?, t[2].as_f32()?);
    let ids = resolve_targets(s, &p.arg_strings())?;
    let j = ids.into_iter().find(|i| s.scene.is_joint(*i)).ok_or("Select a joint")?;
    let n = s.scene.move_skinned_joint(j, d)?;
    Ok(vec![format!("Moved {} and updated {} bind matrices", s.scene.name_of(j), n)])
}

/// `skinVis [-joint J] [mesh]` shows the weights of a joint on the mesh (blue: none, red: full); `skinVis -off` hides them.
pub fn vis(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["joint", "j"], arity: 1 }, FlagSpec { names: &["off"], arity: 0 }])?;
    if p.has("off") {
        s.scene.skin_vis = None;
        return Ok(vec!["Weights hidden".to_string()]);
    }
    let sh = skinned_shape(s, &names_only(&p))?;
    let jn = p.string("joint").ok_or("Usage: skinVis -joint J [mesh]  |  skinVis -off")?;
    let j = s.scene.find(&jn).filter(|i| s.scene.is_joint(*i)).ok_or_else(|| format!("'{}' is not a joint", jn))?;
    s.scene.skin_vis = Some((sh, j));
    Ok(vec![format!("Showing the weights of {}", jn)])
}

const FLEX_SPEC: &[FlagSpec] = &[FlagSpec { names: &["divisions", "dv"], arity: 3 }, FlagSpec { names: &["type", "t"], arity: 1 }, FlagSpec { names: &["position", "pos"], arity: 1 }];

/// `flexor [-t lattice] [-dv S T U] [-pos 0|1] [joint]`: Skin > Edit Rigid Skin > Create Flexor. A lattice flexor is made under the
/// joint (default: the selected one); scale and move it over the bending region. Channels on the flexor transform:
/// flexorRounding, flexorLength, flexorCreasing. Its points (`flexor.pt[..]`) can be keyed or driven.
/// Sculpt and Joint Cluster flexors are not supported.
pub fn flexor(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, FLEX_SPEC)?;
    if let Some(k) = p.string("type") {
        if k != "lattice" {
            return Err(format!("Only the lattice flexor is available, not '{}'", k));
        }
    }
    let divs = match p.get("divisions") {
        Some(v) => [v[0].as_f32()? as usize, v[1].as_f32()? as usize, v[2].as_f32()? as usize],
        None => [2, 5, 2],
    };
    let ids = resolve_targets(s, &p.arg_strings())?;
    let joint = ids.into_iter().find(|i| s.scene.is_joint(*i)).ok_or("Select a joint")?;
    let position = p.num("position")?.map(|v| v > 0.5).unwrap_or(true);
    let t = s.scene.create_flexor(joint, divs, position)?;
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

const LAT_SPEC: &[FlagSpec] = &[FlagSpec { names: &["divisions", "dv"], arity: 3 }, FlagSpec { names: &["name", "n"], arity: 1 }];

/// `lattice [-dv S T U] [-n NAME] [mesh]`: Deform > Create Lattice. The lattice is fitted to the mesh; edit its points
/// (`ffd1Lattice.pt[1][2][0]`), key them, or bind the lattice to joints with `bindSkin -rigid`.
pub fn lattice(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, LAT_SPEC)?;
    let divs = match p.get("divisions") {
        Some(v) => [v[0].as_f32()? as usize, v[1].as_f32()? as usize, v[2].as_f32()? as usize],
        None => [2, 5, 2],
    };
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut shapes: Vec<NodeId> = Vec::new();
    for id in ids {
        shapes.extend(s.scene.mesh_shapes_under(id));
    }
    let sh = *shapes.first().ok_or("Select a polygon object")?;
    let t = s.scene.create_lattice(sh, divs, p.string("name").as_deref())?;
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}
