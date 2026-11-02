//! Deformers: wire, cluster, soft modification, jiggle, point on curve, membership and order.

use super::cmd_model::comp_verts;
use super::commands::{fmt_f, resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::{CurveDriver, CurveDriverKind, NodeData, PolyKind};
use crate::dag::NodeId;
use crate::math::{Mat4, Vec3};
use crate::session::Session;

fn components(args: &[String]) -> Vec<String> {
    args.iter().filter(|a| a.contains('[')).cloned().collect()
}

fn plain(args: &[String]) -> Vec<String> {
    args.iter().filter(|a| !a.contains('[')).cloned().collect()
}

const WIRE_SPEC: &[FlagSpec] = &[FlagSpec { names: &["dropoffDistance", "dds"], arity: 1 }];

/// `wire [-dds DISTANCE] mesh ... curve`: Create Deformers > Wire. Select the meshes, then the curve.
pub fn wire(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, WIRE_SPEC)?;
    let ids = resolve_targets(s, &plain(&p.arg_strings()))?;
    let mut curve: Option<NodeId> = None;
    let mut meshes: Vec<NodeId> = Vec::new();
    for id in ids {
        let cs = s.scene.curve_shapes_under(id);
        if let Some(c) = cs.first() {
            curve = Some(*c);
        } else {
            meshes.extend(s.scene.mesh_shapes_under(id));
        }
    }
    let curve = curve.ok_or("Select the geometry, then the NURBS curve to use as the wire")?;
    if meshes.is_empty() {
        return Err("Select the geometry, then the NURBS curve to use as the wire".to_string());
    }
    let dds = p.num("dropoffDistance")?.unwrap_or(1.0).max(0.001);
    let mut out = Vec::new();
    for m in meshes {
        let op = s.scene.create_wire(m, curve, dds)?;
        out.push(s.scene.name_of(op));
    }
    Ok(out)
}

/// `cluster [-n NAME] [obj.vtx[..] | curve.cv[..]]`: Create Deformers > Cluster on the selected points or control vertices.
pub fn cluster(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["name", "n"], arity: 1 }])?;
    let (shape, verts) = comp_verts(s, &p.arg_strings())?;
    match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::NurbsCurve(_)) => {
            let curve_world = {
                let t = s.scene.node(shape).and_then(|n| n.parent).ok_or("The curve has no transform")?;
                s.scene.world_matrix(t)
            };
            let members: Vec<usize> = verts.iter().map(|v| *v as usize).collect();
            let c = members.iter().fold(Vec3::ZERO, |a, i| {
                let cv = match s.scene.node(shape).map(|n| &n.data) {
                    Some(NodeData::NurbsCurve(cd)) => cd.curve.cvs.get(*i).copied().unwrap_or(Vec3::ZERO),
                    _ => Vec3::ZERO,
                };
                a + curve_world.transform_point(cv)
            }) * (1.0 / members.len().max(1) as f32);
            let h = {
                let t = s.scene.create_node(NodeData::Transform, "clusterHandle", None, true);
                s.scene.create_node(NodeData::Locator, &format!("{}Shape", s.scene.name_of(t)), Some(t), false);
                for (a, v) in [("translateX", c.x), ("translateY", c.y), ("translateZ", c.z)] {
                    s.scene.set_attr(t, a, v)?;
                }
                t
            };
            let bind_inv = s.scene.world_matrix(h).inverse();
            if let Some(NodeData::NurbsCurve(cd)) = s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
                if cd.driver_base.len() != cd.curve.cvs.len() {
                    cd.driver_base = cd.curve.cvs.clone();
                }
                cd.drivers.push(CurveDriver { kind: CurveDriverKind::Cluster { members, bind_inv }, handle: Some(h), handle_name: String::new() });
            }
            s.scene.flush_meshes();
            s.scene.selection = vec![h];
            Ok(vec![s.scene.name_of(h)])
        }
        _ => {
            let (_, h) = s.scene.create_cluster(shape, verts)?;
            s.scene.selection = vec![h];
            Ok(vec![s.scene.name_of(h)])
        }
    }
}

const SOFT_SPEC: &[FlagSpec] = &[FlagSpec { names: &["radius", "r"], arity: 1 }, FlagSpec { names: &["falloff", "f"], arity: 1 }];

/// `softMod [-radius R] [-falloff smooth|linear|ease] [obj.vtx[..]]`: Create Deformers > Soft Modification.
pub fn soft_mod(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SOFT_SPEC)?;
    let (shape, verts) = comp_verts(s, &p.arg_strings())?;
    let ft = match p.string("falloff").unwrap_or_else(|| "smooth".into()).to_lowercase().as_str() {
        "smooth" => 0,
        "linear" => 1,
        "ease" | "easeout" => 2,
        o => return Err(format!("Unknown falloff '{}': smooth, linear or ease", o)),
    };
    let (_, h) = s.scene.create_soft_mod(shape, &verts, p.num("radius")?.unwrap_or(1.0).max(0.001), ft)?;
    s.scene.selection = vec![h];
    Ok(vec![s.scene.name_of(h)])
}

const JIG_SPEC: &[FlagSpec] = &[FlagSpec { names: &["stiffness", "s"], arity: 1 }, FlagSpec { names: &["damping", "d"], arity: 1 }, FlagSpec { names: &["ignoreTransform", "it"], arity: 1 }];

/// `jiggle [-s STIFFNESS] [-d DAMPING] [-it 0|1] [obj.vtx[..] | mesh]`: Create Deformers > Jiggle Deformer.
pub fn jiggle(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, JIG_SPEC)?;
    let args = p.arg_strings();
    let (shape, verts): (NodeId, Vec<u32>) = if !components(&args).is_empty() {
        comp_verts(s, &args)?
    } else {
        let ids = resolve_targets(s, &args)?;
        let sh = ids.iter().flat_map(|i| s.scene.mesh_shapes_under(*i)).next().ok_or("Select a polygon object or its vertices")?;
        (sh, Vec::new())
    };
    let ignore = p.num("ignoreTransform")?.map(|v| v > 0.5).unwrap_or(false);
    let op = s.scene.create_jiggle(shape, verts, p.num("stiffness")?.unwrap_or(0.1), p.num("damping")?.unwrap_or(0.2), ignore)?;
    Ok(vec![s.scene.name_of(op)])
}

const POC_SPEC: &[FlagSpec] = &[FlagSpec { names: &["editPoint", "ep"], arity: 1 }, FlagSpec { names: &["parameter", "p"], arity: 1 }];

/// `pointOnCurve [-ep N | -p PARAM] curve`: Create Deformers > Point on Curve. A locator is made on the curve; moving it moves
/// that point of the curve (edit points lie at the span boundaries).
pub fn point_on_curve(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, POC_SPEC)?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let shape = ids.iter().flat_map(|i| s.scene.curve_shapes_under(*i)).next().ok_or("Select a NURBS curve")?;
    let (cur, dom) = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::NurbsCurve(cd)) => (cd.curve.clone(), cd.curve.dir.domain()),
        _ => return Err("Select a NURBS curve".to_string()),
    };
    let param = match (p.num("editPoint")?, p.num("parameter")?) {
        (Some(n), _) => {
            let ps = cur.dir.span_params();
            *ps.get(n as usize).ok_or_else(|| format!("The curve has edit points 0 to {}", ps.len() - 1))?
        }
        (None, Some(t)) => t,
        _ => return Err("Usage: pointOnCurve -ep N curve   or   pointOnCurve -p PARAMETER curve".to_string()),
    };
    if param < dom.0 - 1e-4 || param > dom.1 + 1e-4 {
        return Err(format!("The parameter must lie inside {} .. {}", fmt_f(dom.0), fmt_f(dom.1)));
    }
    let owner = s.scene.node(shape).and_then(|n| n.parent).ok_or("The curve has no transform")?;
    let w = s.scene.world_matrix(owner);
    let at = w.transform_point(cur.point(param));
    let loc = s.scene.create_node(NodeData::Transform, "pointOnCurveLocator", None, true);
    s.scene.create_node(NodeData::Locator, &format!("{}Shape", s.scene.name_of(loc)), Some(loc), false);
    for (a, v) in [("translateX", at.x), ("translateY", at.y), ("translateZ", at.z)] {
        s.scene.set_attr(loc, a, v)?;
    }
    if let Some(NodeData::NurbsCurve(cd)) = s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
        if cd.driver_base.len() != cd.curve.cvs.len() {
            cd.driver_base = cd.curve.cvs.clone();
        }
        cd.drivers.push(CurveDriver { kind: CurveDriverKind::PointOnCurve { param, rest: at }, handle: Some(loc), handle_name: String::new() });
    }
    s.scene.selection = vec![loc];
    Ok(vec![s.scene.name_of(loc)])
}

fn find_op(s: &Session, name: &str) -> Result<NodeId, String> {
    let id = s.scene.find(name).ok_or_else(|| format!("No node named {}", name))?;
    match s.scene.node(id).map(|n| &n.data) {
        Some(NodeData::PolyOp(_)) => Ok(id),
        _ => Err(format!("'{}' is not a history node", name)),
    }
}

/// `reorderDeformer NODE up|down|front|back`: Edit > Inputs: change the order the deformations are applied in.
pub fn reorder(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let a = p.arg_strings();
    if a.len() != 2 {
        return Err("Usage: reorderDeformer NODE up|down|front|back".to_string());
    }
    let op = find_op(s, &a[0])?;
    s.scene.reorder_deformer(op, &a[1])?;
    let shape = s.scene.op_shape(op).unwrap();
    Ok(vec![s.scene.history_of(shape).iter().map(|h| s.scene.name_of(*h)).collect::<Vec<_>>().join(" -> ")])
}

const MEM_SPEC: &[FlagSpec] = &[FlagSpec { names: &["add"], arity: 0 }, FlagSpec { names: &["remove"], arity: 0 }, FlagSpec { names: &["set"], arity: 0 }, FlagSpec { names: &["query", "q"], arity: 0 }, FlagSpec { names: &["node", "n"], arity: 1 }];

/// `deformerMembership -n NODE -add|-remove|-set obj.vtx[..]` or `-q -n NODE`: Edit Deformers > Edit Membership Tool.
pub fn membership(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, MEM_SPEC)?;
    let op = find_op(s, &p.string("node").ok_or("Usage: deformerMembership -n NODE -add|-remove|-set obj.vtx[..]")?)?;
    if p.has("query") {
        let n = match s.scene.node(op).map(|n| &n.data) {
            Some(NodeData::PolyOp(o)) => o.ints.clone(),
            _ => Vec::new(),
        };
        return Ok(vec![if n.is_empty() { "all points".to_string() } else { format!("{} points: {}", n.len(), n.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ")) }]);
    }
    let mode = if p.has("remove") { "remove" } else if p.has("set") { "set" } else { "add" };
    let (_, verts) = comp_verts(s, &p.arg_strings())?;
    let n = s.scene.edit_membership(op, &verts, mode)?;
    Ok(vec![format!("{} point(s) are members", n)])
}

const WGT_SPEC: &[FlagSpec] = &[FlagSpec { names: &["node", "n"], arity: 1 }, FlagSpec { names: &["value", "v"], arity: 1 }, FlagSpec { names: &["smooth"], arity: 0 }];

/// `deformerWeights -n NODE -v VALUE [-smooth] obj.vtx[..]`: Paint Jiggle Weights / cluster weights: a weight per point.
pub fn weights(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, WGT_SPEC)?;
    let op = find_op(s, &p.string("node").ok_or("Usage: deformerWeights -n NODE -v VALUE [-smooth] obj.vtx[..]")?)?;
    let (_, verts) = comp_verts(s, &p.arg_strings())?;
    let n = s.scene.paint_deformer_weights(op, &verts, p.num("value")?.unwrap_or(1.0), p.has("smooth"))?;
    Ok(vec![format!("Set the weights of {} point(s)", n)])
}

/// `listDeformers [mesh]`: the deformers in the history of the mesh, in the order they act.
pub fn list_deformers(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut out = Vec::new();
    for id in ids {
        for sh in s.scene.mesh_shapes_under(id) {
            for op in s.scene.deformer_ops(sh) {
                let kind = match s.scene.node(op).map(|n| &n.data) {
                    Some(NodeData::PolyOp(o)) => o.kind.name(),
                    _ => "",
                };
                out.push(format!("{}  ({})", s.scene.name_of(op), kind));
            }
        }
    }
    Ok(out)
}

#[allow(dead_code)]
fn _unused(_: Mat4, _: PolyKind) {}
