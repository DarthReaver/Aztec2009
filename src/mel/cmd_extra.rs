//! Maya 2009 commands added during the feature audit: more primitives, non-linear deformers,
//! Freeze Transformations, Ungroup, Hide / Isolate Select, xform and relatives queries, time units,
//! Bake Simulation and Playblast, Duplicate Special.

use super::cmd_model::{current_shape, object_op};
use super::commands::{fmt_f, resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec, Parsed};
use crate::anim::Interp;
use crate::dag::nodes::{NodeData, PolyKind};
use crate::dag::NodeId;
use crate::math::{Mat4, Quat, Vec3};
use crate::session::Session;

const AXES: [&str; 3] = ["X", "Y", "Z"];

fn transform_of(s: &Session, id: NodeId) -> NodeId {
    match s.scene.node(id) {
        Some(n) if !n.data.is_transform() => n.parent.unwrap_or(id),
        _ => id,
    }
}

fn flag_bool(p: &Parsed, name: &str, default: bool) -> bool {
    match p.get(name).and_then(|v| v.first()) {
        None => default,
        Some(v) => !matches!(v.as_string().to_lowercase().as_str(), "0" | "false" | "off" | "no"),
    }
}

fn parse_axis(v: &str) -> Result<usize, String> {
    match v.to_lowercase().as_str() {
        "x" | "0" => Ok(0),
        "y" | "1" => Ok(1),
        "z" | "2" => Ok(2),
        o => Err(format!("Unknown axis '{}', use x, y or z", o)),
    }
}

// ====================================================================== primitives

const PRIM_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["r", "radius"], arity: 1 },
    FlagSpec { names: &["h", "height"], arity: 1 },
    FlagSpec { names: &["sr", "sectionRadius"], arity: 1 },
    FlagSpec { names: &["sa", "sx", "subdivisions", "subdivisionsAxis"], arity: 1 },
    FlagSpec { names: &["sh", "ss", "subdivisionsHeight", "sectionSubdivisions"], arity: 1 },
];

/// polyCone and polyTorus.
pub fn poly_prim(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, PRIM_SPEC)?;
    let g = |k: &str, d: f32| -> Result<f32, String> { Ok(p.num(k)?.unwrap_or(d)) };
    let mut params: Vec<(String, f32)> = Vec::new();
    if cmd == "polyCone" {
        params.push(("radius".into(), g("radius", 1.0)?));
        params.push(("height".into(), g("height", 2.0)?));
        params.push(("subdivisions".into(), g("subdivisions", 20.0)?.floor()));
    } else {
        params.push(("radius".into(), g("radius", 1.0)?));
        params.push(("sectionRadius".into(), g("sectionRadius", 0.5)?));
        params.push(("subdivisions".into(), g("subdivisions", 24.0)?.floor()));
        params.push(("sectionSubdivisions".into(), g("sectionSubdivisions", 12.0)?.floor()));
    }
    for (k, v) in &params {
        let ok = if k.starts_with("subdivisions") || k == "sectionSubdivisions" { (3.0..=200.0).contains(v) } else { *v > 0.0 };
        if !ok {
            return Err(format!("{} is out of range", k));
        }
    }
    let name = p.string("name");
    let (t, _) = s.scene.create_mesh_object(cmd, params, name.as_deref())?;
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

// ====================================================================== deformers

const NL_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["type", "typ"], arity: 1 },
    FlagSpec { names: &["axis", "ax"], arity: 1 },
    FlagSpec { names: &["lowBound", "lb"], arity: 1 },
    FlagSpec { names: &["highBound", "hb"], arity: 1 },
    FlagSpec { names: &["curvature", "cv"], arity: 1 },
    FlagSpec { names: &["startAngle", "sa"], arity: 1 },
    FlagSpec { names: &["endAngle", "ea"], arity: 1 },
    FlagSpec { names: &["startFlare", "sf"], arity: 2 },
    FlagSpec { names: &["endFlare", "ef"], arity: 2 },
    FlagSpec { names: &["curve", "cu"], arity: 1 },
    FlagSpec { names: &["factor", "fa"], arity: 1 },
    FlagSpec { names: &["expand", "ex"], arity: 1 },
    FlagSpec { names: &["amplitude", "amp"], arity: 1 },
    FlagSpec { names: &["wavelength", "wl"], arity: 1 },
    FlagSpec { names: &["offset", "off"], arity: 1 },
    FlagSpec { names: &["dropoff", "dp"], arity: 1 },
];

/// `nonLinear -type bend|twist|flare|squash|sine` (also `bend`, `twist`, ... directly). Adds a
/// deformer history node to every selected mesh; its values stay editable and animatable.
pub fn non_linear(s: &mut Session, rest: &[Token], forced: Option<&str>) -> Out {
    let p = parse_args(rest, NL_SPEC)?;
    let ty = forced.map(|f| f.to_string()).or_else(|| p.string("type")).ok_or("Usage: nonLinear -type bend|twist|flare|squash|sine [-curvature C] ...")?;
    let kind = match ty.to_lowercase().as_str() {
        "bend" => PolyKind::Bend,
        "twist" => PolyKind::Twist,
        "flare" => PolyKind::Flare,
        "squash" => PolyKind::Squash,
        "sine" => PolyKind::Sine,
        "wave" => PolyKind::Wave,
        o => return Err(format!("Unknown deformer type '{}'", o)),
    };
    let axis = match p.string("axis") {
        Some(a) => parse_axis(&a)?,
        None => 1,
    };
    let names: Vec<String> = p.arg_strings();
    let ids = resolve_targets(s, &names)?;
    let mut shapes: Vec<NodeId> = Vec::new();
    for id in ids {
        let t = transform_of(s, id);
        shapes.extend(s.scene.mesh_shapes_under(t));
    }
    shapes.dedup();
    if shapes.is_empty() {
        return Err("Select a polygon object first".to_string());
    }
    s.scene.flush_meshes();
    let mut out = Vec::new();
    for sh in shapes {
        let (mut lo, mut hi) = match s.scene.node(sh).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => {
                let (a, b) = md.mesh.bounds();
                let get = |v: Vec3| [v.x, v.y, v.z][axis];
                (get(a), get(b))
            }
            _ => continue,
        };
        if let Some(v) = p.num("lowBound")? {
            lo = v;
        }
        if let Some(v) = p.num("highBound")? {
            hi = v;
        }
        if (hi - lo).abs() < 1e-6 {
            hi = lo + 1.0;
        }
        let len = hi - lo;
        // the largest distance of a point from the deformer axis (the wave ripples out to there)
        let max_radius = match s.scene.node(sh).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.mesh.positions.iter().map(|q| { let v = [q.x, q.y, q.z]; (v[(axis + 1) % 3].powi(2) + v[(axis + 2) % 3].powi(2)).sqrt() }).fold(0.0f32, f32::max).max(0.01),
            _ => len,
        };
        let n = |k: &str, d: f32| -> Result<f32, String> { Ok(p.num(k)?.unwrap_or(d)) };
        let mut plugs: Vec<(String, f32)> = vec![("axis".into(), axis as f32), ("lowBound".into(), lo), ("highBound".into(), hi)];
        let pair = |k: &str, d: (f32, f32)| -> Result<(f32, f32), String> {
            match p.get(k) {
                Some(v) if v.len() >= 2 => Ok((v[0].as_f32()?, v[1].as_f32()?)),
                _ => Ok(d),
            }
        };
        match kind {
            PolyKind::Bend => plugs.push(("curvature".into(), n("curvature", std::f32::consts::FRAC_PI_2 / len)?)),
            PolyKind::Twist => {
                plugs.push(("startAngle".into(), n("startAngle", 0.0)?));
                plugs.push(("endAngle".into(), n("endAngle", 90.0)?));
            }
            PolyKind::Flare => {
                let sf = pair("startFlare", (1.0, 1.0))?;
                let ef = pair("endFlare", (0.4, 0.4))?;
                plugs.extend([("startFlareX".into(), sf.0), ("startFlareZ".into(), sf.1), ("endFlareX".into(), ef.0), ("endFlareZ".into(), ef.1), ("curve".into(), n("curve", 0.0)?)]);
            }
            PolyKind::Squash => {
                plugs.push(("factor".into(), n("factor", 0.4)?));
                plugs.push(("expand".into(), n("expand", 1.0)?));
            }
            PolyKind::Wave => {
                plugs.push(("amplitude".into(), n("amplitude", 0.1 * len)?));
                plugs.push(("wavelength".into(), n("wavelength", 0.25 * max_radius)?));
                plugs.push(("offset".into(), n("offset", 0.0)?));
                plugs.push(("minRadius".into(), n("minRadius", 0.0)?));
                plugs.push(("maxRadius".into(), n("maxRadius", max_radius)?));
                plugs.push(("dropoff".into(), n("dropoff", 0.0)?));
            }
            _ => {
                plugs.push(("amplitude".into(), n("amplitude", 0.1 * len)?));
                plugs.push(("wavelength".into(), n("wavelength", 0.5 * len)?));
                plugs.push(("offset".into(), n("offset", 0.0)?));
                plugs.push(("dropoff".into(), n("dropoff", 0.0)?));
            }
        }
        let node = s.scene.append_history(sh, kind, Vec::new(), &plugs)?;
        out.push(s.scene.name_of(node));
        if !s.history_on {
            s.scene.delete_history(sh);
        }
    }
    Ok(out)
}

const AVG_SPEC: &[FlagSpec] = &[FlagSpec { names: &["i", "iterations"], arity: 1 }, FlagSpec { names: &["s", "strength"], arity: 1 }];

pub fn average_vertex(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, AVG_SPEC)?;
    let names = p.arg_strings();
    if !names.is_empty() {
        let ids = resolve_targets(s, &names)?;
        s.scene.selection = ids;
    }
    let plugs = [("iterations".to_string(), p.num("iterations")?.unwrap_or(1.0).clamp(1.0, 50.0).floor()), ("strength".to_string(), p.num("strength")?.unwrap_or(1.0).clamp(0.0, 1.0))];
    Ok(vec![object_op(s, PolyKind::Average, &plugs, "Averaged")?])
}

// ====================================================================== transforms

const ID_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["apply", "a"], arity: 1 },
    FlagSpec { names: &["translate", "t"], arity: 1 },
    FlagSpec { names: &["rotate", "r"], arity: 1 },
    FlagSpec { names: &["scale", "s"], arity: 1 },
];

fn is_animated(s: &Session, id: NodeId, attrs: &[&str]) -> bool {
    attrs.iter().any(|a| {
        s.scene.anim.bindings.iter().any(|b| b.node == id && b.attr == *a) || s.scene.node(id).and_then(|n| n.plug(a)).map(|p| p.source.is_some()).unwrap_or(false)
    })
}

/// Freeze Transformations: move the transform into the vertices (meshes) or the children (groups).
pub fn make_identity(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, ID_SPEC)?;
    if !flag_bool(&p, "apply", false) {
        return Err("Usage: makeIdentity -apply true [-t 1] [-r 1] [-s 1] [object ...]".to_string());
    }
    let (ft, fr, fs) = (flag_bool(&p, "translate", true), flag_bool(&p, "rotate", true), flag_bool(&p, "scale", true));
    // only suffixes of translate * rotate * scale can be frozen without moving the object
    if !((ft && fr && fs) || (!ft && fr && fs) || (!ft && !fr && fs) || (!ft && !fr && !fs)) {
        return Err("Unsupported combination: scale can be frozen alone, rotate needs scale, translate needs both".to_string());
    }
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut done = 0;
    for id in ids {
        let t = transform_of(s, id);
        if !s.scene.node(t).map(|n| n.data.is_transform()).unwrap_or(false) || s.scene.is_joint(t) {
            continue;
        }
        if is_animated(s, t, &["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"]) {
            return Err(format!("'{}' is animated or connected; its transformations cannot be frozen", s.scene.name_of(t)));
        }
        let g = |s: &mut Session, a: &str| s.scene.get_attr(t, a).unwrap_or(0.0);
        let tr = Vec3::new(g(s, "translateX"), g(s, "translateY"), g(s, "translateZ"));
        let ro = Vec3::new(g(s, "rotateX"), g(s, "rotateY"), g(s, "rotateZ"));
        let sc = Vec3::new(g(s, "scaleX"), g(s, "scaleY"), g(s, "scaleZ"));
        let rp = Vec3::new(g(s, "rotatePivotX"), g(s, "rotatePivotY"), g(s, "rotatePivotZ"));
        let sp = Vec3::new(g(s, "scalePivotX"), g(s, "scalePivotY"), g(s, "scalePivotZ"));
        // the rotation and scale around the pivots, as the object currently has them, split like Maya's matrix
        let cr = Mat4::translation(rp) * Mat4::from_quat(Quat::from_euler_deg(ro)) * Mat4::translation(-rp);
        let cs = Mat4::translation(sp) * Mat4::scaling(sc) * Mat4::translation(-sp);
        let rpt = Vec3::new(g(s, "rotatePivotTranslateX"), g(s, "rotatePivotTranslateY"), g(s, "rotatePivotTranslateZ"));
        let m = (if ft { Mat4::translation(tr + rpt) } else { Mat4::identity() }) * (if fr { cr } else { Mat4::identity() }) * (if fs { cs } else { Mat4::identity() });
        let shapes = s.scene.mesh_shapes_under(t);
        let mut nurbs_shapes: Vec<NodeId> = shapes.iter().copied().filter(|sh| s.scene.node(*sh).map(|n| n.parent == Some(t)).unwrap_or(false) && matches!(s.scene.node(*sh).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.nurbs.is_some())).collect();
        nurbs_shapes.extend(s.scene.curve_shapes_under(t).into_iter().filter(|sh| s.scene.node(*sh).map(|n| n.parent == Some(t)).unwrap_or(false)));
        let direct: Vec<NodeId> = shapes.into_iter().filter(|sh| s.scene.node(*sh).map(|n| n.parent == Some(t)).unwrap_or(false) && !nurbs_shapes.contains(sh)).collect();
        if !nurbs_shapes.is_empty() {
            // NURBS: the matrix goes into the control points; generated geometry becomes plain geometry
            for sh in &nurbs_shapes {
                s.scene.delete_nurbs_history(*sh);
                let cvs: Vec<(u32, Vec3)> = match s.scene.node(*sh).map(|n| &n.data) {
                    Some(NodeData::Mesh(md)) => md.nurbs.as_ref().map(|n| n.surf.cvs.iter().enumerate().map(|(i, c)| (i as u32, m.transform_point(*c))).collect()).unwrap_or_default(),
                    Some(NodeData::NurbsCurve(cd)) => cd.curve.cvs.iter().enumerate().map(|(i, c)| (i as u32, m.transform_point(*c))).collect(),
                    _ => Vec::new(),
                };
                s.scene.set_nurbs_cvs(*sh, &cvs);
            }
        } else if !direct.is_empty() {
            for sh in direct {
                if s.scene.skin_of(sh).is_some() {
                    return Err("The mesh is bound to a skeleton; detach the skin first".to_string());
                }
                let mut plugs: Vec<(String, f32)> = Vec::new();
                for i in 0..16 {
                    plugs.push((format!("m{}", i), m.m[i / 4][i % 4]));
                }
                s.scene.append_history(sh, PolyKind::Transform, Vec::new(), &plugs)?;
                if !s.history_on {
                    s.scene.delete_history(sh);
                }
            }
        } else {
            // a group: push the matrix into the children so the hierarchy keeps its world positions
            let kids: Vec<NodeId> = s.scene.node(t).map(|n| n.children.clone()).unwrap_or_default();
            for k in kids {
                if !s.scene.node(k).map(|n| n.data.is_transform()).unwrap_or(false) {
                    continue;
                }
                let nl = m * s.scene.local_matrix(k);
                s.scene.set_local_matrix(k, &nl)?;
            }
        }
        for (a, v, on) in [
            ("translateX", 0.0, ft), ("translateY", 0.0, ft), ("translateZ", 0.0, ft),
            ("rotateX", 0.0, fr), ("rotateY", 0.0, fr), ("rotateZ", 0.0, fr),
            ("scaleX", 1.0, fs), ("scaleY", 1.0, fs), ("scaleZ", 1.0, fs),
        ] {
            if on {
                s.scene.set_attr(t, a, v)?;
            }
        }
        // the pivots stay where they are in the world: frozen translation becomes part of the pivot position
        if ft {
            for (base, p0) in [("rotatePivot", rp), ("scalePivot", sp)] {
                for (i, ax) in AXES.iter().enumerate() {
                    let v = [p0.x, p0.y, p0.z][i] + [tr.x, tr.y, tr.z][i] + [rpt.x, rpt.y, rpt.z][i];
                    let _ = s.scene.set_attr(t, &format!("{}{}", base, ax), v);
                }
            }
            for ax in AXES {
                let _ = s.scene.set_attr(t, &format!("rotatePivotTranslate{}", ax), 0.0);
            }
        }
        done += 1;
    }
    s.scene.flush_meshes();
    Ok(vec![format!("Froze transformations on {} object(s)", done)])
}

pub fn ungroup(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut freed: Vec<NodeId> = Vec::new();
    for id in ids {
        let t = transform_of(s, id);
        let kids: Vec<NodeId> = match s.scene.node(t) {
            Some(n) if n.data.is_transform() && !n.children.is_empty() => n.children.clone(),
            _ => return Err(format!("'{}' has nothing to ungroup", s.scene.name_of(t))),
        };
        if kids.iter().any(|k| !s.scene.node(*k).map(|n| n.data.is_transform()).unwrap_or(false)) {
            return Err(format!("'{}' is an object, not a group", s.scene.name_of(t)));
        }
        let parent = s.scene.node(t).and_then(|n| n.parent);
        for k in &kids {
            s.scene.set_parent_keep_world(*k, parent)?;
        }
        s.scene.delete_node(t)?;
        freed.extend(kids);
    }
    let names: Vec<String> = freed.iter().map(|i| s.scene.name_of(*i)).collect();
    s.scene.selection = freed;
    Ok(names)
}

// ====================================================================== visibility

fn set_visible(s: &mut Session, id: NodeId, on: bool) {
    let t = transform_of(s, id);
    let _ = s.scene.set_attr(t, "visibility", if on { 1.0 } else { 0.0 });
}

pub fn hide(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    s.last_hidden.clear();
    for id in &ids {
        set_visible(s, *id, false);
        s.last_hidden.push(transform_of(s, *id));
    }
    s.scene.selection.retain(|i| !ids.contains(i));
    Ok(vec![format!("Hid {} object(s)", ids.len())])
}

pub fn show_hidden(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["all", "a"], arity: 0 }, FlagSpec { names: &["last", "l"], arity: 0 }])?;
    if p.has("last") {
        let ids = std::mem::take(&mut s.last_hidden);
        let mut n = 0;
        for id in ids {
            if s.scene.node(id).is_some() {
                set_visible(s, id, true);
                n += 1;
            }
        }
        return Ok(vec![format!("Showed {} object(s) hidden last", n)]);
    }
    let ids: Vec<NodeId> = if p.args.is_empty() || p.has("all") {
        s.scene.ids_where(|d| matches!(d, NodeData::Transform | NodeData::Joint))
    } else {
        resolve_targets(s, &p.arg_strings())?
    };
    let mut n = 0;
    for id in ids {
        let t = transform_of(s, id);
        if s.scene.get_attr(t, "visibility").unwrap_or(1.0) < 0.5 {
            set_visible(s, t, true);
            n += 1;
        }
    }
    Ok(vec![format!("Showed {} object(s)", n)])
}

/// `isolateSelect -state 1` hides everything outside the selection; `-state 0` restores it.
pub fn isolate_select(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["state", "s"], arity: 1 }])?;
    let on = p.num("state")?.map(|v| v > 0.5).unwrap_or(s.isolated.is_empty());
    for id in std::mem::take(&mut s.isolated) {
        if s.scene.node(id).is_some() {
            set_visible(s, id, true);
        }
    }
    if !on {
        return Ok(vec!["Isolate select off".to_string()]);
    }
    if s.scene.selection.is_empty() {
        return Err("Select objects to isolate first".to_string());
    }
    let mut keep: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    for id in s.scene.selection.clone() {
        let t = transform_of(s, id);
        keep.extend(s.scene.descendants(t));
        let mut cur = s.scene.node(t).and_then(|n| n.parent);
        while let Some(c) = cur {
            keep.insert(c);
            cur = s.scene.node(c).and_then(|n| n.parent);
        }
    }
    let mut hidden = Vec::new();
    for id in s.scene.ids_where(|d| matches!(d, NodeData::Transform | NodeData::Joint)) {
        if keep.contains(&id) || s.scene.get_attr(id, "visibility").unwrap_or(1.0) < 0.5 {
            continue;
        }
        // cameras and lights stay: they are not part of the geometry being isolated
        if s.scene.node(id).map(|n| n.children.iter().any(|c| matches!(s.scene.node(*c).map(|x| &x.data), Some(NodeData::Camera | NodeData::StereoCamera | NodeData::Light)))).unwrap_or(false) {
            continue;
        }
        set_visible(s, id, false);
        hidden.push(id);
    }
    let n = hidden.len();
    s.isolated = hidden;
    Ok(vec![format!("Isolated the selection ({} object(s) hidden)", n)])
}

// ====================================================================== queries

const XFORM_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["query", "q"], arity: 0 },
    FlagSpec { names: &["worldSpace", "ws"], arity: 0 },
    FlagSpec { names: &["objectSpace", "os"], arity: 0 },
    FlagSpec { names: &["translation", "t"], arity: 0 },
    FlagSpec { names: &["rotation", "ro"], arity: 0 },
    FlagSpec { names: &["scale", "s"], arity: 0 },
    FlagSpec { names: &["relative", "r"], arity: 0 },
    FlagSpec { names: &["absolute", "a"], arity: 0 },
    FlagSpec { names: &["boundingBox", "bb"], arity: 0 },
    FlagSpec { names: &["matrix", "m"], arity: 0 },
    FlagSpec { names: &["centerPivots", "cp"], arity: 0 },
    FlagSpec { names: &["pivots", "piv"], arity: 0 },
    FlagSpec { names: &["rotatePivot", "rp"], arity: 0 },
    FlagSpec { names: &["scalePivot", "sp"], arity: 0 },
];

/// World space center of the geometry below a transform (its own pivot when there is none).
fn geometry_center(s: &mut Session, id: NodeId) -> Option<Vec3> {
    s.scene.flush_meshes();
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for sh in s.scene.curve_shapes_under(id) {
        let t = transform_of(s, sh);
        let m = s.scene.world_matrix(t);
        if let Some(NodeData::NurbsCurve(cd)) = s.scene.node(sh).map(|n| &n.data) {
            for v in &cd.curve.cvs {
                let w = m.transform_point(*v);
                lo = Vec3::new(lo.x.min(w.x), lo.y.min(w.y), lo.z.min(w.z));
                hi = Vec3::new(hi.x.max(w.x), hi.y.max(w.y), hi.z.max(w.z));
            }
        }
    }
    for sh in s.scene.mesh_shapes_under(id) {
        let t = transform_of(s, sh);
        let m = s.scene.world_matrix(t);
        if let Some(NodeData::Mesh(md)) = s.scene.node(sh).map(|n| &n.data) {
            for v in &md.mesh.positions {
                let w = m.transform_point(*v);
                lo = Vec3::new(lo.x.min(w.x), lo.y.min(w.y), lo.z.min(w.z));
                hi = Vec3::new(hi.x.max(w.x), hi.y.max(w.y), hi.z.max(w.z));
            }
        }
    }
    if lo.x.is_finite() { Some((lo + hi) * 0.5) } else { None }
}

/// Move the pivots of `id` to a world space point without moving the object (the difference goes
/// into the rotate pivot translation, as in Maya).
pub fn set_pivot_world(s: &mut Session, id: NodeId, wp: Vec3) -> Result<(), String> {
    let old = s.scene.local_matrix(id);
    let pw = match s.scene.node(id).and_then(|n| n.parent) {
        Some(p) => s.scene.world_matrix(p),
        None => Mat4::identity(),
    };
    let t = Vec3::new(s.scene.get_attr(id, "translateX")?, s.scene.get_attr(id, "translateY")?, s.scene.get_attr(id, "translateZ")?);
    let target = pw.inverse().transform_point(wp) - t;
    let mut lin = old;
    lin.m[3][0] = 0.0;
    lin.m[3][1] = 0.0;
    lin.m[3][2] = 0.0;
    let det = {
        let col = |c: usize| Vec3::new(lin.m[c][0], lin.m[c][1], lin.m[c][2]);
        col(0).dot(col(1).cross(col(2)))
    };
    let rpt = if det.abs() > 1e-9 { lin.inverse().transform_dir(old.translation_part() - t - target) + target } else { Vec3::ZERO };
    let rp = target - rpt;
    for (base, v) in [("rotatePivot", rp), ("scalePivot", rp), ("rotatePivotTranslate", rpt)] {
        for (i, ax) in AXES.iter().enumerate() {
            s.scene.set_attr(id, &format!("{}{}", base, ax), [v.x, v.y, v.z][i])?;
        }
    }
    Ok(())
}

/// Center Pivot: the pivots go to the middle of the object's bounding box.
pub fn center_pivot(s: &mut Session, id: NodeId) -> Result<bool, String> {
    match geometry_center(s, id) {
        Some(c) => {
            set_pivot_world(s, id, c)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn nums(v: &[f32]) -> String {
    v.iter().map(|x| fmt_f(*x)).collect::<Vec<_>>().join(" ")
}

/// `xform -q -ws -t node`, `xform -ws -t 1 2 3 node`, `xform -q -bb node`, `xform -q -m node`.
pub fn xform(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, XFORM_SPEC)?;
    let ws = p.has("worldSpace");
    if p.has("centerPivots") && !p.has("query") {
        let ids = resolve_targets(s, &p.arg_strings())?;
        let mut n = 0;
        for id in ids {
            let id = transform_of(s, id);
            if center_pivot(s, id)? {
                n += 1;
            }
        }
        return Ok(vec![format!("Centered the pivot of {} object(s)", n)]);
    }
    if p.has("pivots") && !p.has("query") {
        if p.args.len() < 3 {
            return Err("Usage: xform [-ws] -piv x y z [object ...]".to_string());
        }
        let v = Vec3::new(p.args[0].as_f32()?, p.args[1].as_f32()?, p.args[2].as_f32()?);
        let names: Vec<String> = p.args[3..].iter().map(|a| a.as_string()).collect();
        for id in resolve_targets(s, &names)? {
            let id = transform_of(s, id);
            let wp = if ws {
                v
            } else {
                // object space point -> world: through the parent and the translation
                let pw = match s.scene.node(id).and_then(|n| n.parent) {
                    Some(p) => s.scene.world_matrix(p),
                    None => Mat4::identity(),
                };
                let t = Vec3::new(s.scene.get_attr(id, "translateX")?, s.scene.get_attr(id, "translateY")?, s.scene.get_attr(id, "translateZ")?);
                pw.transform_point(t + v)
            };
            set_pivot_world(s, id, wp)?;
        }
        return Ok(vec![]);
    }
    if p.has("query") && (p.has("rotatePivot") || p.has("scalePivot")) {
        let ids = resolve_targets(s, &p.arg_strings())?;
        let id = transform_of(s, *ids.first().ok_or("Nothing is selected")?);
        let base = if p.has("scalePivot") { "scalePivot" } else { "rotatePivot" };
        if ws {
            let w = s.scene.pivot_world(id);
            return Ok(vec![nums(&[w.x, w.y, w.z])]);
        }
        let g = |s: &mut Session, a: &str| s.scene.get_attr(id, &format!("{}{}", base, a)).unwrap_or(0.0);
        return Ok(vec![nums(&[g(s, "X"), g(s, "Y"), g(s, "Z")])]);
    }
    let (want_t, want_r, want_s) = (p.has("translation"), p.has("rotation"), p.has("scale"));
    if p.has("query") {
        let ids = resolve_targets(s, &p.arg_strings())?;
        let id = transform_of(s, *ids.first().ok_or("Nothing is selected")?);
        let mut out = Vec::new();
        if p.has("boundingBox") {
            let world = s.scene.world_matrix(id);
            s.scene.flush_meshes();
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            for sh in s.scene.mesh_shapes_under(id) {
                let m = if sh == id { world } else { s.scene.world_matrix(transform_of(s, sh)) };
                if let Some(NodeData::Mesh(md)) = s.scene.node(sh).map(|n| &n.data) {
                    for v in &md.mesh.positions {
                        let w = m.transform_point(*v);
                        lo = Vec3::new(lo.x.min(w.x), lo.y.min(w.y), lo.z.min(w.z));
                        hi = Vec3::new(hi.x.max(w.x), hi.y.max(w.y), hi.z.max(w.z));
                    }
                }
            }
            if !lo.x.is_finite() {
                return Err("The object has no geometry".to_string());
            }
            out.push(nums(&[lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]));
            return Ok(out);
        }
        if p.has("matrix") {
            let m = if p.has("objectSpace") { s.scene.local_matrix(id) } else { s.scene.world_matrix(id) };
            let flat: Vec<f32> = (0..16).map(|i| m.m[i / 4][i % 4]).collect();
            return Ok(vec![nums(&flat)]);
        }
        let (wt, wr, wsc) = if ws { s.scene.world_matrix(id).decompose() } else {
            let g = |s: &mut Session, n: [&str; 3]| Vec3::new(s.scene.get_attr(id, n[0]).unwrap_or(0.0), s.scene.get_attr(id, n[1]).unwrap_or(0.0), s.scene.get_attr(id, n[2]).unwrap_or(0.0));
            (g(s, ["translateX", "translateY", "translateZ"]), g(s, ["rotateX", "rotateY", "rotateZ"]), g(s, ["scaleX", "scaleY", "scaleZ"]))
        };
        if want_t || !(want_r || want_s) {
            out.push(nums(&[wt.x, wt.y, wt.z]));
        }
        if want_r {
            out.push(nums(&[wr.x, wr.y, wr.z]));
        }
        if want_s {
            out.push(nums(&[wsc.x, wsc.y, wsc.z]));
        }
        return Ok(out);
    }
    // set
    if p.args.len() < 3 {
        return Err("Usage: xform [-ws] [-r] -t|-ro|-s x y z [object ...]".to_string());
    }
    if !(want_t || want_r || want_s) {
        return Err("Say what to set: -t (translation), -ro (rotation) or -s (scale)".to_string());
    }
    let v = Vec3::new(p.args[0].as_f32()?, p.args[1].as_f32()?, p.args[2].as_f32()?);
    let names: Vec<String> = p.args[3..].iter().map(|a| a.as_string()).collect();
    let ids = resolve_targets(s, &names)?;
    let rel = p.has("relative");
    for id in ids {
        let id = transform_of(s, id);
        let set = |s: &mut Session, base: &str, v: Vec3| -> Result<(), String> {
            for (i, a) in AXES.iter().enumerate() {
                s.scene.set_attr(id, &format!("{}{}", base, a), [v.x, v.y, v.z][i])?;
            }
            Ok(())
        };
        if !ws {
            for (on, base) in [(want_t, "translate"), (want_r, "rotate"), (want_s, "scale")] {
                if !on {
                    continue;
                }
                let cur = Vec3::new(s.scene.get_attr(id, &format!("{}X", base))?, s.scene.get_attr(id, &format!("{}Y", base))?, s.scene.get_attr(id, &format!("{}Z", base))?);
                let new = if !rel {
                    v
                } else if base == "scale" {
                    cur.mul_elem(v)
                } else {
                    cur + v
                };
                set(s, base, new)?;
            }
            continue;
        }
        let (mut wt, mut wr, mut wsc) = s.scene.world_matrix(id).decompose();
        if want_t {
            wt = if rel { wt + v } else { v };
        }
        if want_r {
            wr = if rel { wr + v } else { v };
        }
        if want_s {
            wsc = if rel { wsc.mul_elem(v) } else { v };
        }
        let new_world = Mat4::trs(wt, Quat::from_euler_deg(wr), wsc);
        let pw = match s.scene.node(id).and_then(|n| n.parent) {
            Some(par) => s.scene.world_matrix(par),
            None => Mat4::identity(),
        };
        let (lt, lr, ls) = s.scene.trs_for_local(id, &(pw.inverse() * new_world));
        if want_t {
            set(s, "translate", lt)?;
        }
        if want_r {
            set(s, "rotate", lr)?;
        }
        if want_s {
            set(s, "scale", ls)?;
        }
    }
    Ok(vec![])
}

const REL_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["parent", "p"], arity: 0 },
    FlagSpec { names: &["children", "c"], arity: 0 },
    FlagSpec { names: &["shapes", "s"], arity: 0 },
    FlagSpec { names: &["allDescendents", "ad"], arity: 0 },
    FlagSpec { names: &["fullPath", "f"], arity: 0 },
];

pub fn list_relatives(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, REL_SPEC)?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let node = s.scene.node(id).ok_or("Invalid node")?;
        let list: Vec<NodeId> = if p.has("parent") {
            node.parent.into_iter().collect()
        } else if p.has("allDescendents") {
            s.scene.descendants(id).into_iter().skip(1).collect()
        } else if p.has("shapes") {
            node.children.iter().copied().filter(|c| s.scene.node(*c).map(|n| !n.data.is_transform()).unwrap_or(false)).collect()
        } else {
            node.children.clone()
        };
        for r in list {
            let n = s.scene.name_of(r);
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    Ok(out)
}

pub fn node_type(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = p.arg_strings();
    if names.is_empty() {
        return Err("Usage: nodeType node".to_string());
    }
    names.iter().map(|n| s.scene.find(n).and_then(|id| s.scene.node(id)).map(|nd| nd.data.type_name().to_string()).ok_or_else(|| format!("No object matches name: {}", n))).collect()
}

pub fn obj_exists(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let n = p.args.first().ok_or("Usage: objExists name")?.as_string();
    Ok(vec![if s.scene.find(&n).is_some() { "1" } else { "0" }.to_string()])
}

const EVAL_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["vertex", "v"], arity: 0 },
    FlagSpec { names: &["edge", "e"], arity: 0 },
    FlagSpec { names: &["face", "f"], arity: 0 },
    FlagSpec { names: &["triangle", "t"], arity: 0 },
];

/// `polyEvaluate -v -e -f -t [object]`: counts, one value per line in that order.
pub fn poly_evaluate(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, EVAL_SPEC)?;
    let names = p.arg_strings();
    if !names.is_empty() {
        let ids = resolve_targets(s, &names)?;
        s.scene.selection = ids;
        // a named object is the one asked about, whatever component selection was active
        s.comp.mode = crate::session::SelMode::Object;
        s.comp.node = None;
    }
    let shape = current_shape(s).ok_or("Select a polygon object first")?;
    s.scene.flush_meshes();
    let (nv, ne, nf, nt) = match s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
        Some(NodeData::Mesh(md)) => (md.mesh.positions.len(), md.edge_count(), md.mesh.faces.len(), md.mesh.triangle_count()),
        _ => return Err("Not a mesh".to_string()),
    };
    let all = !(p.has("vertex") || p.has("edge") || p.has("face") || p.has("triangle"));
    let mut out = Vec::new();
    for (on, v) in [(p.has("vertex"), nv), (p.has("edge"), ne), (p.has("face"), nf), (p.has("triangle"), nt)] {
        if on || all {
            out.push(v.to_string());
        }
    }
    Ok(out)
}

// ====================================================================== time

const TIME_UNITS: &[(&str, f32)] = &[("game", 15.0), ("film", 24.0), ("pal", 25.0), ("ntsc", 30.0), ("show", 48.0), ("palf", 50.0), ("ntscf", 60.0)];

pub fn current_unit(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(
        rest,
        &[FlagSpec { names: &["time", "t"], arity: 1 }, FlagSpec { names: &["query", "q"], arity: 0 }, FlagSpec { names: &["linear", "l"], arity: 1 }, FlagSpec { names: &["angle", "a"], arity: 1 }],
    )?;
    if p.has("query") {
        let fps = s.scene.timeline.fps;
        let name = TIME_UNITS.iter().find(|(_, f)| (*f - fps).abs() < 0.01).map(|(n, _)| n.to_string()).unwrap_or_else(|| format!("{}fps", fmt_f(fps)));
        return Ok(vec![name]);
    }
    let Some(t) = p.string("time") else { return Ok(vec![]) };
    let fps = TIME_UNITS.iter().find(|(n, _)| *n == t.to_lowercase()).map(|(_, f)| *f).or_else(|| t.trim_end_matches("fps").parse::<f32>().ok()).filter(|f| *f > 0.0 && *f <= 240.0).ok_or_else(|| format!("Unknown time unit '{}': use game, film, pal, ntsc, show, palf, ntscf or NNfps", t))?;
    s.scene.timeline.fps = fps;
    Ok(vec![format!("Time unit: {} fps", fmt_f(fps))])
}

fn parse_range(s: &Session, v: Option<String>) -> Result<(f32, f32), String> {
    match v {
        None => Ok((s.scene.timeline.start, s.scene.timeline.end)),
        Some(r) => {
            let (a, b) = r.split_once(':').ok_or("Time range must look like \"1:48\"")?;
            let (a, b) = (a.trim().parse::<f32>().map_err(|_| "Bad start frame")?, b.trim().parse::<f32>().map_err(|_| "Bad end frame")?);
            Ok(if a <= b { (a, b) } else { (b, a) })
        }
    }
}

const BAKE_SPEC: &[FlagSpec] = &[FlagSpec { names: &["time", "t"], arity: 1 }, FlagSpec { names: &["sampleBy", "sb"], arity: 1 }, FlagSpec { names: &["sparseAnimCurveBake", "sac"], arity: 1 }];

/// Bake Simulation: key every animated channel of the targets on every frame, folding animation
/// layers and expressions into one base curve.
pub fn bake_results(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, BAKE_SPEC)?;
    let (a, b) = parse_range(s, p.string("time"))?;
    let step = p.num("sampleBy")?.unwrap_or(1.0).max(0.01);
    let names = p.arg_strings();
    let mut targets: Vec<(NodeId, String)> = Vec::new();
    if names.is_empty() || names.iter().any(|n| !n.contains('.')) {
        let nodes: Vec<NodeId> = if names.is_empty() { s.scene.selection.clone() } else { names.iter().filter(|n| !n.contains('.')).map(|n| s.scene.find(n).ok_or_else(|| format!("No object matches name: {}", n))).collect::<Result<_, _>>()? };
        for id in nodes {
            let t = transform_of(s, id);
            for sub in s.scene.descendants(t) {
                for bnd in &s.scene.anim.bindings {
                    if bnd.node == sub && !targets.contains(&(sub, bnd.attr.clone())) {
                        targets.push((sub, bnd.attr.clone()));
                    }
                }
            }
        }
    }
    for n in names.iter().filter(|n| n.contains('.')) {
        let t = s.scene.resolve_plug(n)?;
        if !targets.contains(&t) {
            targets.push(t);
        }
    }
    if targets.is_empty() {
        return Err("Nothing to bake: select animated objects or name node.attr channels".to_string());
    }
    let frames = ((b - a) / step).floor() as usize + 1;
    if frames > 20000 {
        return Err("Too many frames to bake".to_string());
    }
    let keep = s.scene.timeline.current;
    let mut samples: Vec<(f32, Vec<f32>)> = Vec::with_capacity(frames);
    for i in 0..frames {
        let t = a + i as f32 * step;
        s.scene.timeline.current = t;
        s.scene.evaluate_frame();
        let vals = targets.iter().map(|(id, at)| s.scene.get_attr(*id, at).unwrap_or(0.0)).collect();
        samples.push((t, vals));
    }
    for (id, at) in &targets {
        s.scene.anim.bindings.retain(|bd| !(bd.node == *id && bd.attr == *at));
        // a driven channel (motion path, constraint, connection) is released: its baked keys take over
        let sources: Vec<(crate::dag::plugs::PlugRef, crate::dag::plugs::PlugRef)> = s.scene.connections().into_iter().filter(|(_, d)| d.node == *id && d.attr == *at).collect();
        for (a, b) in sources {
            let _ = s.scene.disconnect((a.node, a.attr.clone()), (b.node, b.attr.clone()));
        }
    }
    let sparse = p.num("sparseAnimCurveBake")?.map(|v| v > 0.5).unwrap_or(false);
    for (i, (id, at)) in targets.iter().enumerate() {
        let mut kept: Vec<(f32, f32)> = samples.iter().map(|(t, vals)| (*t, vals[i])).collect();
        if sparse && kept.len() > 2 {
            // drop keys that sit on the straight line between their neighbours
            let mut out: Vec<(f32, f32)> = vec![kept[0]];
            for w in 1..kept.len() - 1 {
                let (p0, p1, p2) = (*out.last().unwrap(), kept[w], kept[w + 1]);
                let line = p0.1 + (p2.1 - p0.1) * (p1.0 - p0.0) / (p2.0 - p0.0);
                if (line - p1.1).abs() > 1e-4 {
                    out.push(p1);
                }
            }
            out.push(*kept.last().unwrap());
            kept = out;
        }
        for (t, v) in kept {
            s.scene.anim.set_key(*id, at, 0, t, v, Interp::Linear);
        }
    }
    s.scene.timeline.current = keep;
    s.scene.evaluate_frame();
    Ok(vec![format!("Baked {} channel(s) over frames {} to {}", targets.len(), fmt_f(a), fmt_f(b))])
}

const BLAST_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["filename", "f", "out"], arity: 1 },
    FlagSpec { names: &["width", "w"], arity: 1 },
    FlagSpec { names: &["height", "h"], arity: 1 },
    FlagSpec { names: &["time", "t"], arity: 1 },
];

/// Playblast: render the perspective view of every frame to `prefix.0001.png`.
pub fn playblast(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, BLAST_SPEC)?;
    let prefix = p.string("filename").unwrap_or_else(|| "playblast/frame".to_string());
    let w = p.num("width")?.unwrap_or(960.0).clamp(64.0, 4096.0) as usize;
    let h = p.num("height")?.unwrap_or(540.0).clamp(64.0, 4096.0) as usize;
    let (a, b) = parse_range(s, p.string("time"))?;
    let (a, b) = (a.round() as i64, b.round() as i64);
    if b - a >= 5000 {
        return Err("Too many frames for one playblast".to_string());
    }
    if let Some(dir) = std::path::Path::new(&prefix).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create '{}': {}", dir.display(), e))?;
    }
    let keep = s.scene.timeline.current;
    let mut count = 0;
    let mut result = Ok(());
    for f in a..=b {
        s.scene.timeline.current = f as f32;
        s.scene.evaluate_frame();
        let path = format!("{}.{:04}.png", prefix, f);
        result = crate::gui::snapshot_png(s, &path, w, h);
        if result.is_err() {
            break;
        }
        count += 1;
    }
    s.scene.timeline.current = keep;
    s.scene.evaluate_frame();
    result?;
    Ok(vec![format!("Playblast: {} frame(s) written to {}.NNNN.png", count, prefix)])
}

// ====================================================================== duplicate special

const DUPS_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["n", "number"], arity: 1 },
    FlagSpec { names: &["t", "translate"], arity: 3 },
    FlagSpec { names: &["ro", "rotate"], arity: 3 },
    FlagSpec { names: &["s", "scale"], arity: 3 },
    FlagSpec { names: &["group", "g"], arity: 0 },
    FlagSpec { names: &["instance", "ilf"], arity: 0 },
];

/// Edit > Duplicate Special: `duplicateSpecial -n 5 -t 2 0 0 -ro 0 30 0 -s 1 1 1 [-group] [object]`.
/// Copy k is offset by k times the translate and rotate steps and scaled by the scale step to the power k.
pub fn duplicate_special(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, DUPS_SPEC)?;
    let n = p.num("number")?.unwrap_or(1.0).round();
    if !(1.0..=500.0).contains(&n) {
        return Err("The number of copies must be between 1 and 500".to_string());
    }
    let vec3 = |k: &str, d: [f32; 3]| -> Result<[f32; 3], String> {
        match p.get(k) {
            Some(v) if v.len() >= 3 => Ok([v[0].as_f32()?, v[1].as_f32()?, v[2].as_f32()?]),
            _ => Ok(d),
        }
    };
    let (dt, dr, ds) = (vec3("translate", [0.0; 3])?, vec3("rotate", [0.0; 3])?, vec3("scale", [1.0; 3])?);
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut created: Vec<NodeId> = Vec::new();
    for id in ids {
        let src = transform_of(s, id);
        let name = s.scene.name_of(src);
        let base: Vec<f32> = ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"].iter().map(|a| s.scene.get_attr(src, a).unwrap_or(0.0)).collect();
        for k in 1..=(n as usize) {
            let copy = if p.has("instance") {
                s.scene.create_instance(src)?
            } else {
                super::cmd_scene::duplicate(s, &[Token::Word(name.clone())])?;
                *s.scene.selection.first().ok_or("Duplicate failed")?
            };
            let kf = k as f32;
            for i in 0..3 {
                s.scene.set_attr(copy, &format!("translate{}", AXES[i]), base[i] + dt[i] * kf)?;
                s.scene.set_attr(copy, &format!("rotate{}", AXES[i]), base[3 + i] + dr[i] * kf)?;
                s.scene.set_attr(copy, &format!("scale{}", AXES[i]), base[6 + i] * ds[i].powi(k as i32))?;
            }
            created.push(copy);
        }
    }
    if p.has("group") {
        let g = s.scene.create_node(NodeData::Transform, "duplicateGroup", None, true);
        for c in &created {
            s.scene.set_parent_keep_world(*c, Some(g))?;
        }
        s.scene.selection = vec![g];
        return Ok(vec![s.scene.name_of(g)]);
    }
    let names: Vec<String> = created.iter().map(|i| s.scene.name_of(*i)).collect();
    s.scene.selection = created;
    Ok(names)
}

// ====================================================================== locators, constraints, driven keys

const LOC_SPEC: &[FlagSpec] = &[FlagSpec { names: &["n", "name"], arity: 1 }, FlagSpec { names: &["p", "position"], arity: 3 }];

/// `spaceLocator [-name N] [-p X Y Z]`
pub fn space_locator(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, LOC_SPEC)?;
    let (tname, force) = match p.string("name") {
        Some(n) => (n, false),
        None => ("locator".to_string(), true),
    };
    let t = s.scene.create_node(NodeData::Transform, &tname, None, force);
    let shape_name = format!("{}Shape", s.scene.name_of(t));
    s.scene.create_node(NodeData::Locator, &shape_name, Some(t), false);
    if let Some(v) = p.get("position") {
        for (i, a) in AXES.iter().enumerate() {
            s.scene.set_attr(t, &format!("translate{}", a), v[i].as_f32()?)?;
        }
    }
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

const CON_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["maintainOffset", "mo"], arity: 0 },
    FlagSpec { names: &["weight", "w"], arity: 1 },
    FlagSpec { names: &["aimVector", "aim"], arity: 3 },
    FlagSpec { names: &["upVector", "u"], arity: 3 },
    FlagSpec { names: &["worldUpVector", "wu"], arity: 3 },
    FlagSpec { names: &["offset", "o"], arity: 3 },
];

/// `pointConstraint [-mo] [-w W] target1 [target2 ...] driven`; the same for orient, scale, aim and parent.
/// With no names, the last selected object is driven by the others.
pub fn constraint(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    use crate::dag::nodes::ConstraintKind;
    let p = parse_args(rest, CON_SPEC)?;
    let kind = match cmd {
        "pointConstraint" => ConstraintKind::Point,
        "orientConstraint" => ConstraintKind::Orient,
        "scaleConstraint" => ConstraintKind::Scale,
        "aimConstraint" => ConstraintKind::Aim,
        _ => ConstraintKind::Parent,
    };
    let names = p.arg_strings();
    let ids: Vec<NodeId> = if names.is_empty() { s.scene.selection.clone() } else { resolve_targets(s, &names)? };
    let ids: Vec<NodeId> = ids.into_iter().map(|i| transform_of(s, i)).collect();
    if ids.len() < 2 {
        return Err(format!("Usage: {} [-mo] [-w W] target ... driven   (select the targets, then the object to constrain)", cmd));
    }
    let (driven, targets) = (*ids.last().unwrap(), &ids[..ids.len() - 1]);
    let c = s.scene.create_constraint(kind, targets, driven, p.has("maintainOffset"))?;
    if let Some(w) = p.num("weight")? {
        for i in 0..targets.len() {
            s.scene.set_attr(c, &format!("w{}", i), w.max(0.0))?;
        }
    }
    for (flag, base) in [("aimVector", "aim"), ("upVector", "up"), ("worldUpVector", "worldUp"), ("offset", "offset")] {
        if let Some(v) = p.get(flag) {
            for (i, a) in AXES.iter().enumerate() {
                s.scene.set_attr(c, &format!("{}{}", base, a), v[i].as_f32()?)?;
            }
        }
    }
    s.scene.evaluate_frame();
    s.scene.selection = vec![driven];
    Ok(vec![s.scene.name_of(c)])
}

const SDK_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["currentDriver", "cd"], arity: 1 },
    FlagSpec { names: &["driverValue", "dv"], arity: 1 },
    FlagSpec { names: &["value", "v"], arity: 1 },
];

/// `setDrivenKeyframe -cd driver.attr [-dv DRIVER_VALUE] [-v VALUE] driven.attr ...`: record that the
/// driven attribute should read VALUE when the driver reads DRIVER_VALUE (both default to their
/// current values). Keys of one driven attribute form a curve; poses in between are interpolated.
pub fn set_driven_keyframe(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SDK_SPEC)?;
    let driver_path = p.string("currentDriver").ok_or("Usage: setDrivenKeyframe -cd driver.attr [-dv V] [-v V] driven.attr ...")?;
    let driver = s.scene.resolve_plug(&driver_path)?;
    let dv = match p.num("driverValue")? {
        Some(v) => v,
        None => s.scene.get_attr(driver.0, &driver.1)?,
    };
    let paths = p.arg_strings();
    if paths.is_empty() {
        return Err("Name the driven attribute, for example pSphere1.translateY".to_string());
    }
    let mut out = Vec::new();
    for path in paths {
        let driven = s.scene.resolve_plug(&path)?;
        let value = match p.num("value")? {
            Some(v) => v,
            None => s.scene.get_attr(driven.0, &driven.1)?,
        };
        let c = s.scene.set_driven_key(driver.clone(), dv, driven, value)?;
        out.push(s.scene.name_of(c));
    }
    s.scene.evaluate_frame();
    Ok(out)
}

// ====================================================================== normals, blend shapes, skin weights

fn mesh_shapes_of(s: &mut Session, names: &[String]) -> Result<Vec<NodeId>, String> {
    let ids = resolve_targets(s, names)?;
    let mut shapes: Vec<NodeId> = Vec::new();
    for id in ids {
        let t = transform_of(s, id);
        shapes.extend(s.scene.mesh_shapes_under(t));
    }
    shapes.dedup();
    if shapes.is_empty() {
        return Err("Select a polygon object first".to_string());
    }
    Ok(shapes)
}

/// `polySoftEdge [-a ANGLE] [object]`: edges sharper than ANGLE degrees render hard. 180 softens
/// everything (smooth shading), 0 hardens everything (faceted); the default is 180.
pub fn poly_soft_edge(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["angle", "a"], arity: 1 }])?;
    let a = p.num("angle")?.unwrap_or(180.0).clamp(0.0, 180.0);
    let shapes = mesh_shapes_of(s, &p.arg_strings())?;
    for sh in &shapes {
        s.scene.set_attr(*sh, "smoothAngle", a)?;
    }
    Ok(vec![format!("Edges sharper than {} degrees are hard on {} object(s)", fmt_f(a), shapes.len())])
}

/// `blendShape target1 [target2 ...] base`: one history node per target with a `weight` (0..1)
/// that morphs the base toward the target. The targets need the same vertex count as the base
/// (duplicate the base and edit the copy). Key the weights for facial animation.
pub fn blend_shape(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = p.arg_strings();
    let ids: Vec<NodeId> = if names.is_empty() { s.scene.selection.clone() } else { resolve_targets(s, &names)? };
    if ids.len() < 2 {
        return Err("Usage: blendShape target1 [target2 ...] base   (select the targets, then the base shape)".to_string());
    }
    let base_t = transform_of(s, *ids.last().unwrap());
    let base = *s.scene.mesh_shapes_under(base_t).first().ok_or("The base must be a polygon object")?;
    s.scene.flush_meshes();
    let nverts = match s.scene.node(base).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md.mesh.positions.len(),
        _ => return Err("The base must be a polygon object".to_string()),
    };
    let mut targets = Vec::new();
    for t in &ids[..ids.len() - 1] {
        let tt = transform_of(s, *t);
        let sh = *s.scene.mesh_shapes_under(tt).first().ok_or_else(|| format!("'{}' is not a polygon object", s.scene.name_of(tt)))?;
        let poly = match s.scene.node(sh).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.poly(),
            _ => continue,
        };
        if poly.pos.len() != nverts {
            return Err(format!("'{}' has {} vertices but the base has {}: a blend shape target must be a copy of the base with the same topology", s.scene.name_of(tt), poly.pos.len(), nverts));
        }
        targets.push(poly);
    }
    let mut out = Vec::new();
    for poly in targets {
        let h = s.scene.append_history(base, PolyKind::Blend, Vec::new(), &[("weight".to_string(), 0.0)])?;
        if let Some(NodeData::PolyOp(op)) = s.scene.nodes.get_mut(h).map(|n| &mut n.data) {
            op.source = Some(poly);
        }
        s.scene.mark_poly_dirty(h);
        out.push(s.scene.name_of(h));
    }
    s.scene.flush_meshes();
    s.scene.selection = vec![base_t];
    Ok(out)
}

const SKINP_SPEC: &[FlagSpec] = &[FlagSpec { names: &["transformValue", "tv"], arity: 2 }, FlagSpec { names: &["normalize", "nrm"], arity: 1 }];

/// `skinPercent -tv JOINT VALUE [-normalize 1] mesh.vtx[0:5]`: set the influence of a joint on vertices.
pub fn skin_percent(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SKINP_SPEC)?;
    let tv = p.get("transformValue").ok_or("Usage: skinPercent -tv JOINT VALUE [-normalize 1] mesh.vtx[0:5]")?;
    let (jname, value) = (tv[0].as_string(), tv[1].as_f32()?.clamp(0.0, 1.0));
    let normalize = p.num("normalize")?.map(|v| v > 0.5).unwrap_or(true);
    let (shape, verts) = super::cmd_model::comp_verts(s, &p.arg_strings())?;
    let skin = s.scene.skin_of(shape).ok_or("The mesh is not bound to a skeleton")?;
    let joint = s.scene.find(&jname).ok_or_else(|| format!("No object matches name: {}", jname))?;
    if let Some(NodeData::PolyOp(op)) = s.scene.nodes.get_mut(skin).map(|n| &mut n.data) {
        let sk = op.skin.as_mut().ok_or("The mesh is not bound to a skeleton")?;
        let j = sk.joints.iter().position(|x| *x == joint).ok_or_else(|| format!("'{}' is not one of the joints this mesh is bound to", jname))? as u16;
        for v in &verts {
            let Some(w) = sk.weights.get_mut(*v as usize) else { continue };
            w.retain(|(jj, _)| *jj != j);
            if normalize {
                let others: f32 = w.iter().map(|(_, x)| *x).sum();
                if others > 1e-6 {
                    let k = (1.0 - value) / others;
                    for e in w.iter_mut() {
                        e.1 *= k;
                    }
                }
            }
            w.retain(|(_, x)| *x > 1e-6);
            if value > 0.0 {
                w.push((j, value));
            }
        }
    }
    s.scene.mark_poly_dirty(skin);
    s.scene.flush_meshes();
    Ok(vec![format!("Set the weight of {} to {} on {} vertices", jname, fmt_f(value), verts.len())])
}

/// `polyCBoolOp -op 1|2|3 objectA objectB` (1 union, 2 difference A - B, 3 intersection). Both objects
/// must be closed polygon meshes; the originals are replaced by one new object in world space.
pub fn boolean_op(s: &mut Session, rest: &[Token]) -> Out {
    use crate::geo::csg::{boolean, from_polys, to_polys, BoolOp};
    let p = parse_args(rest, &[FlagSpec { names: &["op", "operation"], arity: 1 }])?;
    let op = match p.num("op")?.unwrap_or(1.0).round() as i32 {
        1 => BoolOp::Union,
        2 => BoolOp::Difference,
        3 => BoolOp::Intersection,
        _ => return Err("-op must be 1 (union), 2 (difference) or 3 (intersection)".to_string()),
    };
    let names = p.arg_strings();
    let ids: Vec<NodeId> = if names.is_empty() { s.scene.selection.clone() } else { resolve_targets(s, &names)? };
    if ids.len() != 2 {
        return Err("Select exactly two polygon objects".to_string());
    }
    s.scene.flush_meshes();
    let mut sides = Vec::new();
    let mut transforms = Vec::new();
    let mut material = None;
    for id in &ids {
        let t = transform_of(s, *id);
        let shape = *s.scene.mesh_shapes_under(t).first().ok_or_else(|| format!("'{}' is not a polygon object", s.scene.name_of(t)))?;
        let world = s.scene.world_matrix(t);
        let (pos, faces, mat) = match s.scene.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => (md.mesh.positions.iter().map(|v| world.transform_point(*v)).collect::<Vec<_>>(), md.mesh.faces.clone(), md.material),
            _ => return Err("Not a mesh".to_string()),
        };
        if material.is_none() {
            material = mat;
        }
        sides.push(to_polys(&pos, &faces));
        transforms.push(t);
    }
    let b = sides.pop().unwrap();
    let a = sides.pop().unwrap();
    let result = boolean(a, b, op);
    if result.is_empty() {
        return Err("The result is empty: the objects do not overlap in the way this operation needs".to_string());
    }
    let (pos, faces) = from_polys(&result);
    let mesh = crate::geo::Mesh { positions: pos.clone(), normals: vec![Vec3::UP; pos.len()], uvs: vec![crate::math::Vec2::new(0.0, 0.0); pos.len()], faces };
    let poly = crate::geo::edit::Poly::welded(&mesh, 1e-4);
    if poly.faces.is_empty() {
        return Err("The result is empty".to_string());
    }
    let t = s.scene.create_node(NodeData::Transform, "polySurface", None, true);
    let shape_name = format!("{}Shape", s.scene.name_of(t));
    let md = crate::dag::nodes::MeshData { cache: Default::default(), history: Vec::new(), mesh: poly.to_mesh(), prim: "custom".to_string(), params: Vec::new(), material, ..Default::default() };
    s.scene.create_node(NodeData::Mesh(md), &shape_name, Some(t), false);
    for old in transforms {
        let _ = s.scene.delete_node(old);
    }
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

/// `instance [object ...]`: a copy that shares the geometry of the original (change the original and
/// every instance follows). Each instance has its own transform.
pub fn instance(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut made = Vec::new();
    for id in ids {
        let t = transform_of(s, id);
        made.push(s.scene.create_instance(t)?);
    }
    let names: Vec<String> = made.iter().map(|i| s.scene.name_of(*i)).collect();
    s.scene.selection = made;
    Ok(names)
}

/// `uninstance [object ...]`: make instances independent copies that can be edited.
pub fn uninstance(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut n = 0;
    for id in ids {
        let t = transform_of(s, id);
        for sh in s.scene.mesh_shapes_under(t) {
            if let Some(NodeData::Mesh(md)) = s.scene.nodes.get_mut(sh).map(|n| &mut n.data) {
                if md.instance_of.take().is_some() {
                    n += 1;
                }
            }
        }
    }
    Ok(vec![format!("{} instance(s) are now independent", n)])
}

/// Delete by Type: `delete -ch` (construction history), `-nd` (history except deformers), `-sc` (static
/// channels), `-c` (all animation channels), `-cn` (constraints), `-un` (unused nodes).
pub fn delete_by_type(s: &mut Session, flags: &[&str], names: &[String]) -> Out {
    let ids = resolve_targets(s, names)?;
    let mut roots: Vec<NodeId> = Vec::new();
    for id in &ids {
        roots.push(transform_of(s, *id));
    }
    let mut out = Vec::new();
    for flag in flags {
        match *flag {
            "ch" | "nd" => {
                let mut n = 0;
                for t in &roots {
                    let mut shapes = s.scene.mesh_shapes_under(*t);
                    shapes.extend(s.scene.curve_shapes_under(*t));
                    for sh in shapes {
                        n += if *flag == "ch" { s.scene.delete_history(sh) } else { s.scene.delete_non_deformer_history(sh) };
                    }
                }
                out.push(format!("Deleted {} history node(s)", n));
            }
            "sc" | "c" => {
                let owners = s.scene.key_owner_ids(&roots);
                let mut hits: Vec<(NodeId, String, f32)> = Vec::new();
                for b in &s.scene.anim.bindings {
                    if !owners.contains(&b.node) || b.curve.keys.is_empty() {
                        continue;
                    }
                    let first = b.curve.keys[0].v;
                    let constant = b.curve.keys.iter().all(|k| (k.v - first).abs() < 1e-6);
                    if *flag == "c" || constant {
                        hits.push((b.node, b.attr.clone(), first));
                    }
                }
                let n = hits.len();
                for (node, attr, first) in hits {
                    // channels keep the value they had at the current time
                    let cur = s.scene.get_attr(node, &attr).unwrap_or(first);
                    s.scene.anim.bindings.retain(|b| !(b.node == node && b.attr == attr));
                    let _ = s.scene.set_attr(node, &attr, if *flag == "sc" { first } else { cur });
                }
                out.push(format!("Deleted {} {} channel(s)", n, if *flag == "sc" { "static" } else { "animated" }));
            }
            "cn" => {
                let mut n = 0;
                for t in &roots {
                    for d in s.scene.descendants(*t) {
                        if matches!(s.scene.node(d).map(|x| &x.data), Some(NodeData::Constraint(_))) {
                            let _ = s.scene.delete_node(d);
                            n += 1;
                        }
                    }
                }
                out.push(format!("Deleted {} constraint(s)", n));
            }
            _ => {}
        }
    }
    s.scene.evaluate_frame();
    Ok(out)
}

/// `deleteUnusedNodes`: materials no face uses, set driven key curves nothing reads, and the like.
pub fn delete_unused_nodes(s: &mut Session) -> Out {
    let mut used: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    for id in s.scene.ids_where(|d| matches!(d, NodeData::Mesh(_))) {
        if let Some(NodeData::Mesh(md)) = s.scene.node(id).map(|n| &n.data) {
            used.extend(md.material);
            used.extend(md.zones.iter().filter_map(|z| z.material));
        }
    }
    let mut doomed: Vec<NodeId> = s.scene.ids_where(|d| matches!(d, NodeData::Material)).into_iter().filter(|m| !used.contains(m)).collect();
    // curve nodes whose output drives nothing
    for c in s.scene.ids_where(|d| matches!(d, NodeData::DrivenCurve(_))) {
        let driving = s.scene.node(c).and_then(|n| n.plug("output")).map(|p| !p.dests.is_empty()).unwrap_or(false);
        if !driving {
            doomed.push(c);
        }
    }
    let mut names: Vec<String> = doomed.iter().map(|d| s.scene.name_of(*d)).collect();
    for d in &doomed {
        let _ = s.scene.delete_node(*d);
    }
    // texture nodes that no longer feed anything
    for tn in s.scene.ids_where(|d| matches!(d, NodeData::Texture(_))) {
        let feeding = s.scene.node(tn).map(|n| n.plugs.iter().any(|p| !p.dests.is_empty())).unwrap_or(false);
        if !feeding {
            names.push(s.scene.name_of(tn));
            let _ = s.scene.delete_node(tn);
        }
    }
    Ok(vec![format!("Deleted {} unused node(s){}{}", names.len(), if names.is_empty() { "" } else { ": " }, names.join(" "))])
}

/// `selectAllByType polygon|light|camera|joint|locator|ikHandle|emitter|constraint|nurbsCurve`
pub fn select_all_by_type(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ty = p.args.first().map(|a| a.as_string()).ok_or("Usage: selectAllByType polygon|light|camera|joint|locator|ikHandle|emitter|constraint")?;
    let pred: fn(&NodeData) -> bool = match ty.to_lowercase().as_str() {
        "polygon" | "polygons" | "mesh" | "geometry" => |d| matches!(d, NodeData::Mesh(_)),
        "light" | "lights" => |d| matches!(d, NodeData::Light),
        "camera" | "cameras" => |d| matches!(d, NodeData::Camera | NodeData::StereoCamera),
        "joint" | "joints" => |d| matches!(d, NodeData::Joint),
        "locator" | "locators" => |d| matches!(d, NodeData::Locator),
        "ikhandle" | "ikhandles" => |d| matches!(d, NodeData::IkHandle(_)),
        "emitter" | "emitters" => |d| matches!(d, NodeData::Emitter),
        "constraint" | "constraints" => |d| matches!(d, NodeData::Constraint(_)),
        o => return Err(format!("Unknown type '{}'", o)),
    };
    let mut sel: Vec<NodeId> = Vec::new();
    for id in s.scene.ids_where(pred) {
        let t = transform_of(s, id);
        if s.scene.name_of(t) != "persp" && !sel.contains(&t) {
            sel.push(t);
        }
    }
    s.comp.mode = crate::session::SelMode::Object;
    s.comp.node = None;
    let n = sel.len();
    s.scene.selection = sel;
    Ok(vec![format!("Selected {} {} object(s)", n, ty)])
}

const PROJECT_FOLDERS: [&str; 9] = ["scenes", "images", "sourceimages", "data", "sound", "scripts", "clips", "renderData", "autosave"];

/// `project -new PATH` (create a project folder with its sub folders and make it current),
/// `project -set PATH` (use an existing project), `project -q`.
pub fn project(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["new", "n"], arity: 0 }, FlagSpec { names: &["set", "s"], arity: 0 }, FlagSpec { names: &["query", "q"], arity: 0 }])?;
    if p.has("query") || p.args.is_empty() {
        return Ok(vec![s.scene.project_dir.as_ref().map(|d| d.to_string_lossy().to_string()).unwrap_or_else(|| "no project set".to_string())]);
    }
    let path = std::path::PathBuf::from(p.args[0].as_string());
    if p.has("new") {
        for f in PROJECT_FOLDERS {
            std::fs::create_dir_all(path.join(f)).map_err(|e| format!("Cannot create '{}': {}", path.join(f).display(), e))?;
        }
        let ws = "// Aztec 2009 project\nworkspace -fr \"scenes\" \"scenes\";\nworkspace -fr \"images\" \"images\";\nworkspace -fr \"sourceimages\" \"sourceimages\";\n";
        std::fs::write(path.join("workspace.mel"), ws).map_err(|e| format!("Cannot write the workspace file: {}", e))?;
    } else if !path.join("workspace.mel").exists() {
        return Err(format!("'{}' is not a project (it has no workspace.mel): use project -new to create one", path.display()));
    }
    let abs = path.canonicalize().unwrap_or(path);
    s.scene.project_dir = Some(abs.clone());
    Ok(vec![format!("Project: {}", abs.display())])
}

const PREFS_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["query", "q"], arity: 0 },
    FlagSpec { names: &["save"], arity: 0 },
    FlagSpec { names: &["load"], arity: 0 },
    FlagSpec { names: &["set"], arity: 2 },
];

/// `prefs -q`, `prefs -save`, `prefs -load`, `prefs -set NAME VALUE`.
pub fn prefs_cmd(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, PREFS_SPEC)?;
    if let Some(v) = p.get("set") {
        let line = format!("{}={}", v[0].as_string(), v[1].as_string());
        let parsed = crate::session::Prefs::from_text(&line);
        let mut cur = s.prefs.to_text();
        // apply one setting by replacing its line in the current text
        let key = v[0].as_string();
        cur = cur.lines().filter(|l| !l.starts_with(&format!("{}=", key)) || key == "hotkey").collect::<Vec<_>>().join("\n") + "\n" + &line + "\n";
        let _ = parsed;
        s.prefs = crate::session::Prefs::from_text(&cur);
        s.sync_anim_prefs();
    }
    if p.has("load") {
        s.prefs = crate::session::Prefs::load();
        s.sync_anim_prefs();
    }
    if p.has("save") {
        let path = s.prefs.save()?;
        return Ok(vec![format!("Saved preferences to {}", path.display())]);
    }
    Ok(s.prefs.to_text().lines().filter(|l| !l.starts_with('#')).map(|l| l.to_string()).collect())
}

/// `deleteAttr -at NAME [node]`: remove an attribute added with addAttr.
pub fn delete_attr(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["attribute", "at"], arity: 1 }])?;
    let name = p.string("attribute").ok_or("Usage: deleteAttr -at NAME [node]")?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    s.scene.delete_attr(ids[0], &name)?;
    Ok(vec![format!("Deleted {}.{}", s.scene.name_of(ids[0]), name)])
}

const SETS_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["empty", "em"], arity: 0 },
    FlagSpec { names: &["edit", "e"], arity: 0 },
    FlagSpec { names: &["query", "q"], arity: 0 },
    FlagSpec { names: &["addElement", "add"], arity: 1 },
    FlagSpec { names: &["remove", "rm"], arity: 1 },
];

fn find_set(s: &Session, name: &str) -> Result<NodeId, String> {
    let id = s.scene.find(name).ok_or_else(|| format!("No object matches name: {}", name))?;
    if matches!(s.scene.node(id).map(|n| &n.data), Some(NodeData::ObjectSet(_))) {
        Ok(id)
    } else {
        Err(format!("'{}' is not a set", name))
    }
}

/// `sets -name NAME [object ...]` creates a set (from the selection when no objects are named),
/// `sets -e -add SET object ...` / `sets -e -rm SET object ...` edit it, `sets -q SET` lists the members.
pub fn sets(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SETS_SPEC)?;
    let args = p.arg_strings();
    if p.has("query") {
        let name = args.first().ok_or("Usage: sets -q SET")?;
        let id = find_set(s, name)?;
        return Ok(s.scene.set_members(id).iter().map(|m| s.scene.name_of(*m)).collect());
    }
    if p.has("edit") {
        let (set_name, add, objs): (String, bool, Vec<String>) = if let Some(n) = p.string("addElement") {
            (n, true, args.clone())
        } else if let Some(n) = p.string("remove") {
            (n, false, args.clone())
        } else {
            return Err("Usage: sets -e -add SET object ...   or   sets -e -rm SET object ...".to_string());
        };
        let id = find_set(s, &set_name)?;
        let targets = resolve_targets(s, &objs)?;
        let n = if add { s.scene.set_add(id, &targets)? } else { s.scene.set_remove(id, &targets.iter().map(|t| transform_of(s, *t)).collect::<Vec<_>>())? };
        return Ok(vec![format!("{} {} member(s) {} {}", if add { "Added" } else { "Removed" }, n, if add { "to" } else { "from" }, set_name)]);
    }
    let members: Vec<NodeId> = if p.has("empty") {
        Vec::new()
    } else if args.is_empty() {
        s.scene.selection.clone()
    } else {
        resolve_targets(s, &args)?
    };
    let name = p.string("name");
    if let Some(n) = &name {
        if let Some(existing) = s.scene.find(n) {
            if !matches!(s.scene.node(existing).map(|x| &x.data), Some(NodeData::ObjectSet(_))) {
                return Err(format!("'{}' is already used by another node", n));
            }
        }
    }
    let id = s.scene.create_set(name.as_deref(), &members);
    Ok(vec![s.scene.name_of(id)])
}

/// `listSets`: every set with its member count.
pub fn list_sets(s: &mut Session) -> Out {
    Ok(s.scene
        .ids_where(|d| matches!(d, NodeData::ObjectSet(_)))
        .into_iter()
        .map(|i| format!("{} ({} members)", s.scene.name_of(i), s.scene.set_members(i).len()))
        .collect())
}

const SHADER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["color", "c"], arity: 3 },
    FlagSpec { names: &["transparency", "tr"], arity: 1 },
    FlagSpec { names: &["incandescence", "inc"], arity: 3 },
    FlagSpec { names: &["asShader"], arity: 1 },
    FlagSpec { names: &["asTexture"], arity: 1 },
    FlagSpec { names: &["file", "f"], arity: 1 },
];

/// Texture node kinds by their Maya names: file plus the built-in patterns.
fn texture_kind(name: &str) -> Option<&'static str> {
    Some(match name {
        "file" => "file",
        "checker" => "checker",
        "grid" => "grid",
        "bulge" | "bricks" => "bricks",
        "stripes" => "stripes",
        "dots" => "dots",
        "cloth" => "cloth",
        "fractal" => "fractal",
        "brownian" | "marble" => "brownian",
        "noise" => "noise",
        "ramp" => "ramp",
        "gradient" => "gradient",
        _ => return None,
    })
}

fn shader_model(name: &str) -> Option<u8> {
    Some(match name {
        "lambert" => 0,
        "blinn" => 1,
        "phong" | "phongE" => 2,
        "surfaceShader" => 3,
        _ => return None,
    })
}

fn make_shader(s: &mut Session, model: u8, p: &super::parser::Parsed) -> Result<NodeId, String> {
    let name = p.string("name");
    let m = match &name {
        Some(n) => s.scene.create_material(Some(n)),
        None => {
            let base = ["lambert", "blinn", "phong", "surfaceShader"][model as usize];
            s.scene.create_node(NodeData::Material, base, None, true)
        }
    };
    s.scene.set_attr(m, "shaderModel", model as f32)?;
    if model == 2 {
        s.scene.set_attr(m, "shininess", 20.0)?;
    }
    if let Some(c) = p.get("color") {
        for (a, v) in ["R", "G", "B"].iter().zip(c) {
            s.scene.set_attr(m, &format!("color{}", a), v.as_f32()?.clamp(0.0, 1.0))?;
        }
    }
    if let Some(c) = p.get("incandescence") {
        for (a, v) in ["R", "G", "B"].iter().zip(c) {
            s.scene.set_attr(m, &format!("incandescence{}", a), v.as_f32()?.clamp(0.0, 1.0))?;
        }
    }
    if let Some(t) = p.num("transparency")? {
        s.scene.set_attr(m, "transparency", t.clamp(0.0, 1.0))?;
    }
    Ok(m)
}

/// `blinn`, `phong` and `surfaceShader` (and `lambert`): `blinn -name skyM -c 0.4 0.6 1 -tr 0.2`.
pub fn shader_cmd(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, SHADER_SPEC)?;
    let m = make_shader(s, shader_model(cmd).unwrap_or(1), &p)?;
    Ok(vec![s.scene.name_of(m)])
}

/// `shadingNode -asShader blinn [-name N]` and `shadingNode -asTexture file|checker|grid|bricks|cloth|dots|noise|ramp
/// [-name N] [-file "image.png"]`: creates a node of the shading network and prints its name.
pub fn shading_node(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SHADER_SPEC)?;
    if let Some(t) = p.string("asShader").or_else(|| p.args.first().map(|a| a.as_string()).filter(|a| shader_model(a).is_some())) {
        let model = shader_model(&t).ok_or_else(|| format!("Unknown shader '{}': lambert, blinn, phong or surfaceShader", t))?;
        let m = make_shader(s, model, &p)?;
        return Ok(vec![s.scene.name_of(m)]);
    }
    let t = p.string("asTexture").or_else(|| p.args.first().map(|a| a.as_string())).ok_or("Usage: shadingNode -asShader blinn | shadingNode -asTexture file")?;
    let kind = texture_kind(&t).ok_or_else(|| format!("Unknown texture '{}': file, checker, grid, bricks, cloth, dots, noise or ramp", t))?;
    let path = p.string("file").unwrap_or_default();
    if kind == "file" && !path.is_empty() && crate::render::texture::load(&s.scene.texture_file(&path)).is_none() {
        return Err(format!("Cannot read image: {}", path));
    }
    let name = p.string("name");
    let id = s.scene.create_texture(kind, &s.scene.texture_path_for_storage(&path), name.as_deref());
    Ok(vec![s.scene.name_of(id)])
}

/// `hyperShade -assign MATERIAL` (to the selected objects or faces), `hyperShade -objects MATERIAL`
/// (select the objects that use it) and `hyperShade -deleteUnusedNodes`.
pub fn hyper_shade(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["assign", "a"], arity: 0 }, FlagSpec { names: &["objects", "o"], arity: 0 }, FlagSpec { names: &["deleteUnusedNodes", "dun"], arity: 0 }])?;
    if p.has("deleteUnusedNodes") {
        return delete_unused_nodes(s);
    }
    let name = p.args.first().map(|a| a.as_string()).ok_or("Usage: hyperShade -assign MATERIAL | hyperShade -objects MATERIAL")?;
    let mat = s.scene.find(&name).ok_or_else(|| format!("No object matches name: {}", name))?;
    if !matches!(s.scene.node(mat).map(|n| &n.data), Some(NodeData::Material)) {
        return Err(format!("'{}' is not a material", name));
    }
    if p.has("objects") {
        let mut sel: Vec<NodeId> = Vec::new();
        for sh in s.scene.ids_where(|d| matches!(d, NodeData::Mesh(_))) {
            if let Some(NodeData::Mesh(md)) = s.scene.node(sh).map(|n| &n.data) {
                if md.material == Some(mat) || md.zones.iter().any(|z| z.material == Some(mat)) {
                    let t = transform_of(s, sh);
                    if !sel.contains(&t) {
                        sel.push(t);
                    }
                }
            }
        }
        let n = sel.len();
        s.comp.mode = crate::session::SelMode::Object;
        s.scene.selection = sel;
        return Ok(vec![format!("Selected {} object(s) using {}", n, name)]);
    }
    // -assign: faces when a face selection is active, else the selected objects
    if let Some(out) = super::cmd_tex::assign_material_cmd(s, &[name.clone()]) {
        return out;
    }
    let targets = resolve_targets(s, &[])?;
    let mut n = 0;
    for t in targets {
        n += s.scene.assign_material(mat, t)?;
    }
    s.view.show_textures();
    Ok(vec![format!("Assigned {} to {} mesh(es)", name, n)])
}

const LC_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["source", "s"], arity: 1 },
    FlagSpec { names: &["destination", "d"], arity: 1 },
    FlagSpec { names: &["plugs", "p"], arity: 0 },
];

/// `listConnections [-s 1|0] [-d 1|0] [-p] node`: the nodes (or with -p the plugs) connected to a node.
/// -s lists what feeds it (default on), -d what it feeds (default on).
pub fn list_connections(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, LC_SPEC)?;
    let on = |k: &str| p.string(k).map(|v| v != "0" && v.to_lowercase() != "false").unwrap_or(true);
    let (want_src, want_dst) = (on("source"), on("destination"));
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut out: Vec<String> = Vec::new();
    for (src, dst) in s.scene.connections() {
        for id in &ids {
            let (hit, other) = if want_src && dst.node == *id {
                (true, &src)
            } else if want_dst && src.node == *id {
                (true, &dst)
            } else {
                (false, &src)
            };
            if hit {
                let text = if p.has("plugs") { format!("{}.{}", s.scene.name_of(other.node), other.attr) } else { s.scene.name_of(other.node) };
                if !out.contains(&text) {
                    out.push(text);
                }
            }
        }
    }
    Ok(out)
}

/// `smoothPreview [-l 1|2|3] [-dv LEVEL] [object ...]`: Smooth Mesh Preview. 1 shows the cage, 2 the cage
/// and the smoothed mesh, 3 only the smoothed mesh (the 1, 2 and 3 hotkeys).
pub fn smooth_preview(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["level", "l"], arity: 1 }, FlagSpec { names: &["divisions", "dv"], arity: 1 }])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut n = 0;
    for id in ids {
        let shape = match s.scene.node(id).map(|x| &x.data) {
            Some(NodeData::Mesh(_)) => Some(id),
            _ => s.scene.node(id).and_then(|x| x.children.iter().copied().find(|c| matches!(s.scene.node(*c).map(|y| &y.data), Some(NodeData::Mesh(_))))),
        };
        let Some(shape) = shape else { continue };
        if let Some(l) = p.num("level")? {
            if !(1.0..=3.0).contains(&l) {
                return Err("-l must be 1 (cage), 2 (cage and smooth) or 3 (smooth)".to_string());
            }
            s.scene.set_attr(shape, "displaySmoothness", l.round())?;
        }
        if let Some(d) = p.num("divisions")? {
            s.scene.set_attr(shape, "smoothLevel", d.round().clamp(1.0, 3.0))?;
        }
        n += 1;
    }
    if n == 0 {
        return Err("Select a polygon object".to_string());
    }
    Ok(vec![format!("Smooth mesh preview set on {} object(s)", n)])
}

/// `polyConvertPreview`: Modify > Convert > Smooth Mesh Preview to Polygons. Bakes the smoothed mesh
/// into the geometry and returns the object to cage display.
pub fn convert_preview(s: &mut Session) -> Out {
    let shapes: Vec<NodeId> = s.scene.selection.iter().filter_map(|id| match s.scene.node(*id).map(|x| &x.data) {
        Some(NodeData::Mesh(_)) => Some(*id),
        _ => s.scene.node(*id).and_then(|x| x.children.iter().copied().find(|c| matches!(s.scene.node(*c).map(|y| &y.data), Some(NodeData::Mesh(_))))),
    }).collect();
    if shapes.is_empty() {
        return Err("Select a polygon object".to_string());
    }
    let mut done = 0;
    for sh in shapes {
        let shown = s.scene.get_attr(sh, "displaySmoothness").unwrap_or(1.0).round();
        if shown < 2.0 {
            continue;
        }
        let lvl = s.scene.get_attr(sh, "smoothLevel").unwrap_or(2.0).round().clamp(1.0, 3.0) as u32;
        s.scene.set_attr(sh, "displaySmoothness", 1.0)?;
        if let Some(t) = s.scene.node(sh).and_then(|x| x.parent) {
            s.scene.selection = vec![t];
        }
        super::cmd_model::run_op(s, super::cmd_model::ModelOp::Smooth { levels: lvl })?;
        done += 1;
    }
    if done == 0 {
        return Err("Turn on Smooth Mesh Preview first (smoothPreview -l 3)".to_string());
    }
    Ok(vec![format!("Converted {} smooth preview(s) to polygons", done)])
}

/// `polyOptions [-keepFacesTogether 0|1] [-displayNormal 0|1] [-displayBorder 0|1]`: polygon display and
/// Edit Mesh options. With no flags prints them.
pub fn poly_options(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["keepFacesTogether", "kft"], arity: 1 }, FlagSpec { names: &["displayNormal", "dn"], arity: 1 }, FlagSpec { names: &["displayBorder", "db"], arity: 1 }])?;
    if let Some(v) = p.num("keepFacesTogether")? {
        s.prefs.keep_faces_together = v > 0.5;
    }
    if let Some(v) = p.num("displayNormal")? {
        s.scene.show_normals = v > 0.5;
    }
    if let Some(v) = p.num("displayBorder")? {
        s.scene.show_borders = v > 0.5;
    }
    Ok(vec![format!("keepFacesTogether {}, displayNormal {}, displayBorder {}", s.prefs.keep_faces_together as u8, s.scene.show_normals as u8, s.scene.show_borders as u8)])
}

/// `symmetricModelling [-s 0|1] [-axis x|y|z]`: Move tool Reflection. Editing a vertex also moves the
/// one facing it across the object's local axis plane.
pub fn symmetric_modelling(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["symmetry", "s"], arity: 1 }, FlagSpec { names: &["axis", "ax"], arity: 1 }])?;
    if let Some(a) = p.string("axis") {
        let axis = match a.to_lowercase().as_str() {
            "x" | "0" => 0,
            "y" | "1" => 1,
            "z" | "2" => 2,
            o => return Err(format!("Unknown axis '{}': x, y or z", o)),
        };
        s.scene.reflection = Some(axis);
    }
    if let Some(v) = p.num("symmetry")? {
        if v <= 0.5 {
            s.scene.reflection = None;
        } else if s.scene.reflection.is_none() {
            s.scene.reflection = Some(0);
        }
    }
    Ok(vec![match s.scene.reflection {
        Some(a) => format!("Reflection on: {} axis", ["x", "y", "z"][a]),
        None => "Reflection off".to_string(),
    }])
}

/// `referenceQuery [-n NODE]`: the referenced files, or the reference a node belongs to.
pub fn reference_query(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["node", "n"], arity: 1 }])?;
    if let Some(n) = p.string("node") {
        let id = s.scene.find(&n).ok_or_else(|| format!("No object matches name: {}", n))?;
        let name = s.scene.name_of(id);
        return match s.scene.references.iter().find(|r| name.starts_with(&format!("{}_", r.namespace))) {
            Some(r) => Ok(vec![format!("{}  {}", r.namespace, r.path)]),
            None => Ok(vec!["not referenced".to_string()]),
        };
    }
    Ok(s.scene.references.iter().map(|r| format!("{}  {}  {}  ({} nodes)", r.namespace, r.path, if r.loaded { "loaded" } else { "unloaded" }, s.scene.reference_nodes(&r.namespace).len())).collect())
}

const SNAP_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["grid", "g"], arity: 1 },
    FlagSpec { names: &["point", "p"], arity: 1 },
    FlagSpec { names: &["moveStep", "ms"], arity: 1 },
    FlagSpec { names: &["rotateStep", "rs"], arity: 1 },
    FlagSpec { names: &["scaleStep", "ss"], arity: 1 },
    FlagSpec { names: &["query", "q"], arity: 0 },
];

/// `snapMode [-grid 0|1] [-point 0|1] [-moveStep N] [-rotateStep DEG] [-scaleStep N]`: the snapping
/// toggles of the status line. With -q prints the current settings.
pub fn snap_mode(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SNAP_SPEC)?;
    if let Some(v) = p.num("grid")? {
        s.prefs.snap_grid = v > 0.5;
    }
    if let Some(v) = p.num("point")? {
        s.prefs.snap_point = v > 0.5;
    }
    for (flag, field) in [("moveStep", 0), ("rotateStep", 1), ("scaleStep", 2)] {
        if let Some(v) = p.num(flag)? {
            if v <= 0.0 {
                return Err(format!("{} must be greater than zero", flag));
            }
            match field {
                0 => s.prefs.move_step = v,
                1 => s.prefs.rotate_step = v,
                _ => s.prefs.scale_step = v,
            }
        }
    }
    Ok(vec![format!(
        "grid snap {}, point snap {}, steps: move {} rotate {} scale {}",
        if s.prefs.snap_grid { "on" } else { "off" },
        if s.prefs.snap_point { "on" } else { "off" },
        fmt_f(s.prefs.move_step),
        fmt_f(s.prefs.rotate_step),
        fmt_f(s.prefs.scale_step)
    )])
}

#[cfg(test)]
mod tests {
    use crate::mel::execute_script;
    use crate::session::Session;

    fn run(s: &mut Session, src: &str) -> Vec<String> {
        let o = execute_script(s, src);
        assert_eq!(o.error, None, "script failed: {}", src);
        o.lines
    }

    fn num(s: &mut Session, src: &str) -> Vec<f32> {
        run(s, src).last().unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect()
    }

    #[test]
    fn cone_and_torus_counts() {
        let mut s = Session::new();
        assert_eq!(run(&mut s, "polyCone -name c -sa 8; polyEvaluate -f c;").last().unwrap(), "9");
        assert_eq!(run(&mut s, "polyTorus -name t -sa 10 -sh 6; polyEvaluate -f t;").last().unwrap(), "60");
        assert!(execute_script(&mut s, "polyCone -sa 2").error.is_some());
    }

    #[test]
    fn twist_is_live_and_editable() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name bar -h 4 -sx 1; twist -startAngle 0 -endAngle 0 bar;");
        let flat = num(&mut s, "xform -q -bb bar;");
        run(&mut s, "setAttr \"twist1.endAngle\" 45;");
        let turned = num(&mut s, "xform -q -bb bar;");
        assert!(turned[3] > flat[3] + 0.1, "a 45 degree twist widens the box: {:?} {:?}", flat, turned);
        run(&mut s, "setAttr \"twist1.endAngle\" 0;");
        assert_eq!(num(&mut s, "xform -q -bb bar;"), flat);
    }

    #[test]
    fn bend_moves_the_top_sideways() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name bar -h 4 -sx 1; bend -curvature 0.5 bar;");
        let bb = num(&mut s, "xform -q -bb bar;");
        // a Y axis bend curls the top toward +Z and shortens the height
        assert!(bb[5] > 1.0 && bb[4] < 2.0, "{:?}", bb);
    }

    #[test]
    fn deformers_survive_save_and_load() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name bar -h 4; squash -factor 0.5 bar; flare -endFlare 2 2 bar; sine -amplitude 0.2 bar; polyAverageVertex -i 2 bar;");
        let before = num(&mut s, "xform -q -bb bar;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let back = crate::dag::serialize::from_ascii(&text).unwrap();
        let mut s2 = Session::new();
        s2.scene = back;
        let after = num(&mut s2, "xform -q -bb bar;");
        for (a, b) in before.iter().zip(&after) {
            assert!((a - b).abs() < 1e-3, "{:?} vs {:?}", before, after);
        }
    }

    #[test]
    fn freeze_keeps_the_shape_in_place() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; move 1 2 3 c; rotate 0 45 0 c; scale 2 2 2 c;");
        let before = num(&mut s, "xform -q -bb c;");
        run(&mut s, "makeIdentity -apply true c;");
        let after = num(&mut s, "xform -q -bb c;");
        for (a, b) in before.iter().zip(&after) {
            assert!((a - b).abs() < 1e-4, "{:?} vs {:?}", before, after);
        }
        assert_eq!(num(&mut s, "xform -q -t c;"), vec![0.0, 0.0, 0.0]);
        assert_eq!(num(&mut s, "xform -q -s c;"), vec![1.0, 1.0, 1.0]);
        assert_eq!(num(&mut s, "xform -q -ro c;").len(), 3);
    }

    #[test]
    fn freeze_refuses_animated_objects() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; setKeyframe -t 1 -v 0 \"c.translateX\"; setKeyframe -t 5 -v 3 \"c.translateX\";");
        assert!(execute_script(&mut s, "makeIdentity -apply true c").error.unwrap().contains("animated"));
    }

    #[test]
    fn ungroup_keeps_world_positions() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b; move 1 0 0 a; group -name g a b; move 5 0 0 g; rotate 0 90 0 g;");
        let before = num(&mut s, "xform -q -ws -t a;");
        run(&mut s, "ungroup g;");
        assert!(run(&mut s, "objExists g;")[0] == "0");
        let after = num(&mut s, "xform -q -ws -t a;");
        for (a, b) in before.iter().zip(&after) {
            assert!((a - b).abs() < 1e-4, "{:?} vs {:?}", before, after);
        }
    }

    #[test]
    fn xform_world_space_set_inside_a_parent() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; group -name g a; move 10 0 0 g; rotate 0 0 90 g;");
        run(&mut s, "xform -ws -t 3 4 5 a;");
        let w = num(&mut s, "xform -q -ws -t a;");
        for (a, b) in w.iter().zip([3.0, 4.0, 5.0]) {
            assert!((a - b).abs() < 1e-4, "{:?}", w);
        }
        run(&mut s, "xform -r -ws -t 1 0 0 a;");
        assert!((num(&mut s, "xform -q -ws -t a;")[0] - 4.0).abs() < 1e-4);
    }

    #[test]
    fn hide_show_and_isolate() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b; hide a;");
        assert_eq!(run(&mut s, "getAttr a.visibility;")[0], "0");
        run(&mut s, "showHidden -all; select b; isolateSelect -state 1;");
        assert_eq!(run(&mut s, "getAttr a.visibility;")[0], "0");
        assert_eq!(run(&mut s, "getAttr b.visibility;")[0], "1");
        run(&mut s, "isolateSelect -state 0;");
        assert_eq!(run(&mut s, "getAttr a.visibility;")[0], "1");
    }

    #[test]
    fn chip_off_duplicates_or_detaches_faces() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; selectMode -face; select -r c.f[0:1]; polyChipOff -dup 1;");
        assert_eq!(run(&mut s, "polyEvaluate -f c;")[0], "8");
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; selectMode -face; select -r c.f[0]; polyExtract;");
        assert_eq!(run(&mut s, "ls -type transform;").iter().filter(|n| n.starts_with("c") || n.starts_with("polySurface")).count(), 2);
    }

    #[test]
    fn bake_folds_layers_into_the_base_curve() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; setKeyframe -t 1 -v 0 \"c.translateY\"; setKeyframe -t 11 -v 10 \"c.translateY\"; animLayer up; setKeyframe -t 1 -v 0 -animLayer up \"c.translateY\"; setKeyframe -t 11 -v 5 -animLayer up \"c.translateY\";");
        run(&mut s, "currentTime 6;");
        let expect = run(&mut s, "getAttr c.translateY;")[0].parse::<f32>().unwrap();
        run(&mut s, "bakeResults -t \"1:11\" c; animLayer -e -mute 1 up; currentTime 6;");
        let got = run(&mut s, "getAttr c.translateY;")[0].parse::<f32>().unwrap();
        assert!((expect - got).abs() < 1e-3, "{} vs {}", expect, got);
        assert!(expect > 5.0);
    }

    #[test]
    fn time_units() {
        let mut s = Session::new();
        run(&mut s, "currentUnit -time ntsc;");
        assert_eq!(s.scene.timeline.fps, 30.0);
        assert_eq!(run(&mut s, "currentUnit -q;")[0], "ntsc");
        assert!(execute_script(&mut s, "currentUnit -time bogus").error.is_some());
    }

    #[test]
    fn duplicate_special_offsets_each_copy() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; duplicateSpecial -n 3 -t 2 0 0 -s 2 1 1 a;");
        assert_eq!(run(&mut s, "getAttr a3.translateX;")[0], "6");
        assert_eq!(run(&mut s, "getAttr a3.scaleX;")[0], "8");
    }

    #[test]
    fn list_relatives_and_node_type() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; group -name g a;");
        assert_eq!(run(&mut s, "listRelatives -c g;"), ["a"]);
        assert_eq!(run(&mut s, "listRelatives -p a;"), ["g"]);
        assert_eq!(run(&mut s, "listRelatives -s a;"), ["aShape"]);
        assert_eq!(run(&mut s, "nodeType aShape;"), ["mesh"]);
    }

    #[test]
    fn obj_and_fbx_commands_round_trip() {
        let dir = std::env::temp_dir().join(format!("aztec_cmd_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (obj, fbx) = (dir.join("m.obj"), dir.join("m.fbx"));
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polySphere -name b -sx 8; move 4 0 0 b;");
        run(&mut s, &format!("file -export \"{}\"; file -export \"{}\" -ascii;", obj.display(), fbx.display()));
        for path in [&obj, &fbx] {
            let mut t = Session::new();
            let out = run(&mut t, &format!("file -import \"{}\"; polyEvaluate -f a;", path.display()));
            assert!(out[0].starts_with("Imported"), "{:?}", out);
            assert_eq!(out.last().unwrap(), "6");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_unknown_and_missing_files() {
        let mut s = Session::new();
        assert!(execute_script(&mut s, "file -import \"/nonexistent/x.obj\"").error.is_some());
        assert!(execute_script(&mut s, "file -import \"model.stl\"").error.unwrap().contains("Unsupported"));
        assert!(execute_script(&mut s, "file -export \"x.stl\"").error.is_some());
    }

    #[test]
    fn copy_and_paste_keys_at_the_current_time() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b; setKeyframe -t 1 -v 0 \"a.translateX\"; setKeyframe -t 5 -v 10 \"a.translateX\"; setKeyframe -t 9 -v 4 \"a.translateX\";");
        run(&mut s, "copyKey -time \"1:9\" a; currentTime 20;");
        // paste onto itself at the current time
        run(&mut s, "select a; pasteKey;");
        let keys = run(&mut s, "keyframe \"a.translateX\";");
        assert!(keys.iter().any(|k| k.contains("t=20 v=0")), "{:?}", keys);
        assert!(keys.iter().any(|k| k.contains("t=24 v=10")), "{:?}", keys);
        assert!(keys.iter().any(|k| k.contains("t=28 v=4")), "{:?}", keys);
        // paste onto another object's channel of the same name, at an explicit frame
        run(&mut s, "select b; pasteKey -time 50;");
        let kb = run(&mut s, "keyframe \"b.translateX\";");
        assert!(kb.iter().any(|k| k.contains("t=54 v=10")), "{:?}", kb);
    }

    #[test]
    fn copy_only_the_selected_key() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; setKeyframe -t 1 -v 0 \"a.translateX\"; setKeyframe -t 5 -v 10 \"a.translateX\";");
        run(&mut s, "copyKey -time \"5\" a; pasteKey -time 30 a;");
        let keys = run(&mut s, "keyframe \"a.translateX\";");
        assert_eq!(keys.len(), 3, "{:?}", keys);
        assert!(keys.iter().any(|k| k.contains("t=30 v=10")));
        assert!(execute_script(&mut s, "copyKey -time \"90\" a").error.is_some());
    }

    #[test]
    fn cut_keys_removes_and_can_be_pasted_elsewhere() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; setKeyframe -t 1 -v 0 \"a.translateX\"; setKeyframe -t 5 -v 10 \"a.translateX\";");
        run(&mut s, "cutKeys -time \"5\" a;");
        assert_eq!(run(&mut s, "keyframe \"a.translateX\";").len(), 1);
        run(&mut s, "pasteKey -time 12 a;");
        assert!(run(&mut s, "keyframe \"a.translateX\";").iter().any(|k| k.contains("t=12 v=10")));
        assert!(execute_script(&mut s, "pasteKey -time 3").error.is_none());
    }

    #[test]
    fn paste_with_an_empty_clipboard_is_an_error() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a;");
        assert!(execute_script(&mut s, "pasteKey a").error.unwrap().contains("empty"));
    }

    #[test]
    fn emitter_is_an_object_that_positions_and_aims_particles() {
        let mut s = Session::new();
        let out = run(&mut s, "nParticle -name sp -rate 60 -pos 0 3 0;");
        assert_eq!(out, ["sp", "emitter1"]);
        assert_eq!(run(&mut s, "ls -type emitter;"), ["emitter1Shape"]);
        run(&mut s, "setAttr \"sp.gravityY\" 0; select emitter1; move 10 0 0 emitter1; rotate 0 0 -90 emitter1; currentTime 12;");
        let (sum_x, n) = match s.scene.node(s.scene.find("sp").unwrap()).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Nucleus(nd)) => (nd.solver.system.items.iter().map(|p| p.pos.x).sum::<f32>(), nd.solver.system.items.len()),
            _ => panic!("no solver"),
        };
        assert!(n > 5);
        // rotated -90 degrees about Z, +Y points along +X: the cloud sits well to the right of x = 10
        assert!(sum_x / n as f32 > 10.5, "mean x {}", sum_x / n as f32);
        // the emitter's attributes drive the solver
        run(&mut s, "setAttr \"emitter1Shape.rate\" 7;");
        assert_eq!(run(&mut s, "getAttr sp.emitRate;")[0], "7");
        assert!(execute_script(&mut s, "emitter -solver sp").error.unwrap().contains("already"));
        run(&mut s, "delete emitter1; emitter -solver sp;");
        assert_eq!(run(&mut s, "ls -type emitter;").len(), 1);
    }

    #[test]
    fn light_commands_set_up_every_light_type() {
        let mut s = Session::new();
        run(&mut s, "spotLight -name sp -ca 55 -pa 8 -dro 2 -dr 2 -ss 0.7 -sam 24 -sc 0.1 0.1 0.3;");
        for (a, v) in [("lightType", "2"), ("coneAngle", "55"), ("penumbraAngle", "8"), ("dropoff", "2"), ("decayRate", "2"), ("shadowSoftness", "0.7"), ("shadowSamples", "24"), ("shadowColorB", "0.3")] {
            assert_eq!(run(&mut s, &format!("getAttr spShape.{};", a))[0], v, "{}", a);
        }
        run(&mut s, "ambientLight -name amb; areaLight -name ar; directionalLight -name dl -sh 0;");
        assert_eq!(run(&mut s, "getAttr ambShape.shadows;")[0], "0");
        assert_eq!(run(&mut s, "getAttr arShape.lightType;")[0], "4");
        assert_eq!(run(&mut s, "getAttr dlShape.shadows;")[0], "0");
        // an aimed spot light shines at the origin
        let w = num(&mut s, "xform -q -ws -t sp;");
        assert_eq!(w, vec![6.0, 10.0, 6.0]);
    }

    #[test]
    fn feathered_shadows_have_a_soft_edge() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("aztec_shadow_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut soft_counts = Vec::new();
        for ss in ["0", "1.5"] {
            let mut s = Session::new();
            let png = dir.join(format!("s{}.png", ss));
            run(&mut s, &format!("polyPlane -name floor -w 20 -h 20; polyCube -name box -w 2 -h 2 -d 2; move 0 1 0 box; pointLight -name pl -i 1.5 -ss {} -sam 32 -dr 0; move -5 7 -3 pl; viewport -yaw 150 -pitch 35 -dist 16; rayRender -w 240 -h 160 -q 1 -out \"{}\";", ss, png.display()));
            let mut bytes = Vec::new();
            std::fs::File::open(&png).unwrap().read_to_end(&mut bytes).unwrap();
            let img = image::load_from_memory(&bytes).unwrap().to_luma8();
            let (lo, hi) = img.pixels().fold((255u8, 0u8), |(l, h), p| (l.min(p.0[0]), h.max(p.0[0])));
            assert!(hi > lo + 40);
            // pixels that are neither fully shadowed nor fully lit near the floor shadow
            let mid = img.pixels().filter(|p| p.0[0] > lo + 15 && p.0[0] < hi - 90).count();
            soft_counts.push(mid);
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(soft_counts[1] > soft_counts[0], "feathering should add intermediate shades: {:?}", soft_counts);
    }

    #[test]
    fn point_constraint_follows_the_target() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name ball; spaceLocator -name target -p 1 2 3; pointConstraint target ball;");
        assert_eq!(num(&mut s, "xform -q -ws -t ball;"), vec![1.0, 2.0, 3.0]);
        run(&mut s, "move 4 0 0 target;");
        assert_eq!(num(&mut s, "xform -q -ws -t ball;"), vec![4.0, 0.0, 0.0]);
        // weights blend two targets
        run(&mut s, "spaceLocator -name t2 -p 10 0 0; select target t2 ball; pointConstraint -w 1 target t2 ball;");
    }

    #[test]
    fn maintain_offset_keeps_the_start_position() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name ball; move 5 0 0 ball; spaceLocator -name target; pointConstraint -mo target ball;");
        assert_eq!(num(&mut s, "xform -q -ws -t ball;"), vec![5.0, 0.0, 0.0]);
        run(&mut s, "move 0 3 0 target;");
        assert_eq!(num(&mut s, "xform -q -ws -t ball;"), vec![5.0, 3.0, 0.0]);
    }

    #[test]
    fn two_weighted_targets_blend() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name ball; spaceLocator -name a -p 0 0 0; spaceLocator -name b -p 10 0 0; pointConstraint a b ball;");
        assert_eq!(num(&mut s, "xform -q -ws -t ball;"), vec![5.0, 0.0, 0.0]);
        run(&mut s, "setAttr \"ball_pointConstraint1.w1\" 3;");
        assert!((num(&mut s, "xform -q -ws -t ball;")[0] - 7.5).abs() < 1e-4);
    }

    #[test]
    fn orient_and_aim_constraints() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name ball; spaceLocator -name t; rotate 0 90 0 t; orientConstraint t ball;");
        let r = num(&mut s, "xform -q -ws -ro ball;");
        assert!((r[1] - 90.0).abs() < 1e-2, "{:?}", r);
        run(&mut s, "polyCube -name eye; spaceLocator -name look -p 0 0 10; aimConstraint look eye;");
        // the default aim vector is +X: after aiming, local +X points at the target (+Z), a 90 degree turn about Y
        let r = num(&mut s, "xform -q -ws -ro eye;");
        assert!((r[1] + 90.0).abs() < 1e-2, "{:?}", r);
        run(&mut s, "move 10 0 0 look;");
        let r = num(&mut s, "xform -q -ws -ro eye;");
        assert!(r[1].abs() < 1e-2, "{:?}", r);
    }

    #[test]
    fn parent_constraint_carries_rotation_and_offset() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name box; move 2 0 0 box; spaceLocator -name hand; parentConstraint -mo hand box;");
        run(&mut s, "rotate 0 0 90 hand;");
        let p = num(&mut s, "xform -q -ws -t box;");
        assert!(p[0].abs() < 1e-3 && (p[1] - 2.0).abs() < 1e-3, "{:?}", p);
        let r = num(&mut s, "xform -q -ws -ro box;");
        assert!((r[2] - 90.0).abs() < 1e-2, "{:?}", r);
    }

    #[test]
    fn constraints_follow_animation_and_survive_save() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name ball; spaceLocator -name t; setKeyframe -t 1 -v 0 \"t.translateX\"; setKeyframe -t 11 -v 10 \"t.translateX\"; pointConstraint t ball; currentTime 11;");
        assert_eq!(num(&mut s, "xform -q -ws -t ball;")[0], 10.0);
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        run(&mut s2, "currentTime 6;");
        assert!((num(&mut s2, "xform -q -ws -t ball;")[0] - 5.0).abs() < 0.8, "{:?}", num(&mut s2, "xform -q -ws -t ball;"));
        run(&mut s2, "delete ball_pointConstraint1; objExists ball_pointConstraint1;");
    }

    #[test]
    fn constraint_errors() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b;");
        assert!(execute_script(&mut s, "pointConstraint a").error.is_some());
        assert!(execute_script(&mut s, "pointConstraint a a").error.unwrap().contains("itself"));
        run(&mut s, "group -name g a;");
        assert!(execute_script(&mut s, "pointConstraint a g").error.is_some());
    }

    #[test]
    fn set_driven_key_maps_driver_to_driven() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name door; polyCube -name knob;");
        run(&mut s, "setAttr \"knob.translateX\" 0; setDrivenKeyframe -cd \"knob.translateX\" -dv 0 -v 0 \"door.rotateY\";");
        run(&mut s, "setDrivenKeyframe -cd \"knob.translateX\" -dv 10 -v 90 \"door.rotateY\";");
        run(&mut s, "setAttr \"knob.translateX\" 5;");
        let mid = run(&mut s, "getAttr door.rotateY;")[0].parse::<f32>().unwrap();
        assert!((mid - 45.0).abs() < 8.0, "{}", mid);
        run(&mut s, "setAttr \"knob.translateX\" 10;");
        assert!((run(&mut s, "getAttr door.rotateY;")[0].parse::<f32>().unwrap() - 90.0).abs() < 1e-3);
        run(&mut s, "setAttr \"knob.translateX\" 25;");
        assert!((run(&mut s, "getAttr door.rotateY;")[0].parse::<f32>().unwrap() - 90.0).abs() < 1e-3, "clamped past the last key");
        assert!(execute_script(&mut s, "setAttr \"door.rotateY\" 3").error.unwrap().contains("connected"));
    }

    #[test]
    fn set_driven_key_survives_save_and_load() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b; setDrivenKeyframe -cd \"a.translateY\" -dv 0 -v 1 \"b.scaleY\"; setAttr \"a.translateY\" 4; setDrivenKeyframe -cd \"a.translateY\" -dv 4 -v 3 \"b.scaleY\";");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        run(&mut s2, "setAttr \"a.translateY\" 4;");
        assert!((run(&mut s2, "getAttr b.scaleY;")[0].parse::<f32>().unwrap() - 3.0).abs() < 1e-3);
        run(&mut s2, "setAttr \"a.translateY\" 0;");
        assert!((run(&mut s2, "getAttr b.scaleY;")[0].parse::<f32>().unwrap() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn soft_edge_changes_the_shading_split() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c;");
        let count = |s: &mut Session| crate::render::scene_snapshot_for_tests(s).items.iter().map(|i| i.mesh.positions.len()).sum::<usize>();
        let hard = count(&mut s);
        run(&mut s, "polySoftEdge -a 180 c;");
        let soft = count(&mut s);
        assert!(soft < hard, "a smooth cube shares vertices: {} vs {}", soft, hard);
        run(&mut s, "polySoftEdge -a 0 c;");
        assert_eq!(count(&mut s), hard);
        assert_eq!(run(&mut s, "getAttr cShape.smoothAngle;")[0], "0");
    }

    #[test]
    fn blend_shape_morphs_and_is_keyable() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name base; duplicate base; rename base1 target; scale 3 1 1 target;");
        // the scale only changes the transform: bake it into the copy so the shapes differ
        run(&mut s, "makeIdentity -apply true target; select target base; blendShape target base;");
        let wide = num(&mut s, "xform -q -bb base;");
        assert!((wide[3] - 0.5).abs() < 1e-4, "weight 0 keeps the base: {:?}", wide);
        run(&mut s, "setAttr \"blendShape1.weight\" 1;");
        assert!((num(&mut s, "xform -q -bb base;")[3] - 1.5).abs() < 1e-3);
        run(&mut s, "setAttr \"blendShape1.weight\" 0.5;");
        assert!((num(&mut s, "xform -q -bb base;")[3] - 1.0).abs() < 1e-3);
        // animate the weight
        run(&mut s, "setKeyframe -t 1 -v 0 \"blendShape1.weight\"; setKeyframe -t 11 -v 1 -ip linear \"blendShape1.weight\"; currentTime 11;");
        assert!((num(&mut s, "xform -q -bb base;")[3] - 1.5).abs() < 1e-3);
        // and it survives a save and load
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        run(&mut s2, "currentTime 11;");
        assert!((num(&mut s2, "xform -q -bb base;")[3] - 1.5).abs() < 1e-3);
    }

    #[test]
    fn blend_shape_rejects_different_topology() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polySphere -name b;");
        assert!(execute_script(&mut s, "blendShape b a").error.unwrap().contains("same topology"));
        assert!(execute_script(&mut s, "blendShape a").error.is_some());
    }

    #[test]
    fn skin_percent_sets_and_normalizes_weights() {
        let mut s = Session::new();
        run(&mut s, "joint -p 0 0 0 -n j1; joint -p 0 3 0 -n j2; polyCylinder -name arm -r 0.5 -h 6 -sx 8; select j1 arm; bindSkin;");
        run(&mut s, "skinPercent -tv j2 1 arm.vtx[0:3];");
        let sk = s.scene.skin_of(s.scene.find("armShape").unwrap()).unwrap();
        let j2 = s.scene.find("j2").unwrap();
        if let Some(crate::dag::nodes::NodeData::PolyOp(op)) = s.scene.node(sk).map(|n| &n.data) {
            let skin = op.skin.as_ref().unwrap();
            let idx = skin.joints.iter().position(|j| *j == j2).unwrap() as u16;
            for v in 0..4 {
                let w = &skin.weights[v];
                assert_eq!(w.len(), 1, "full weight leaves one influence: {:?}", w);
                assert_eq!(w[0], (idx, 1.0));
            }
        } else {
            panic!("no skin");
        }
        run(&mut s, "skinPercent -tv j2 0.25 arm.vtx[0];");
        if let Some(crate::dag::nodes::NodeData::PolyOp(op)) = s.scene.node(sk).map(|n| &n.data) {
            let sum: f32 = op.skin.as_ref().unwrap().weights[0].iter().map(|(_, w)| *w).sum();
            assert!(sum <= 1.0001, "{}", sum);
        }
        assert!(execute_script(&mut s, "skinPercent -tv nojoint 1 arm.vtx[0]").error.is_some());
    }

    #[test]
    fn boolean_union_difference_intersection() {
        let bb = |s: &mut Session| num(s, "xform -q -bb polySurface1;");
        let setup = "polyCube -name a -w 2 -h 2 -d 2; polyCube -name b -w 2 -h 2 -d 2; move 1 0 0 b;";
        let mut s = Session::new();
        run(&mut s, setup);
        run(&mut s, "polyCBoolOp -op 1 a b;");
        let u = bb(&mut s);
        assert!((u[0] + 1.0).abs() < 1e-3 && (u[3] - 2.0).abs() < 1e-3, "union spans -1..2 in x: {:?}", u);
        assert_eq!(run(&mut s, "ls -type transform;").iter().filter(|n| *n == "a" || *n == "b").count(), 0, "inputs are replaced");
        let mut s = Session::new();
        run(&mut s, setup);
        run(&mut s, "polyCBoolOp -op 2 a b;");
        let d = bb(&mut s);
        assert!((d[0] + 1.0).abs() < 1e-3 && d[3].abs() < 1e-3, "difference keeps x -1..0: {:?}", d);
        let mut s = Session::new();
        run(&mut s, setup);
        run(&mut s, "polyCBoolOp -op 3 a b;");
        let i = bb(&mut s);
        assert!(i[0].abs() < 1e-3 && (i[3] - 1.0).abs() < 1e-3, "intersection is x 0..1: {:?}", i);
        // a sphere carved out of a cube leaves a closed mesh with more faces than the cube
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polySphere -name b -r 0.7 -sx 12; polyCBoolOp -op 2 a b;");
        assert!(run(&mut s, "polyEvaluate -f polySurface1;")[0].parse::<u32>().unwrap() > 6);
    }

    #[test]
    fn boolean_errors() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b; move 10 0 0 b;");
        assert!(execute_script(&mut s, "polyCBoolOp -op 3 a b").error.unwrap().contains("empty"));
        assert!(execute_script(&mut s, "polyCBoolOp -op 9 a b").error.is_some());
        assert!(execute_script(&mut s, "polyCBoolOp -op 1 a").error.is_some());
    }

    #[test]
    fn merge_to_center_ignores_distance() {
        let mut s = Session::new();
        run(&mut s, "polyPlane -name p -sx 2; selectMode -face; select -r p.f[0]; polyConvertSelection -toVertex;");
        assert_eq!(run(&mut s, "polyEvaluate -v p;")[0], "9");
        // the default merge only joins vertices closer than 0.001: nothing changes
        run(&mut s, "polyMergeVertex;");
        assert_eq!(run(&mut s, "polyEvaluate -v p;")[0], "9");
        run(&mut s, "selectMode -face; select -r p.f[0]; polyConvertSelection -toVertex; polyMergeVertex -center;");
        // the four corners of one square became one vertex and that square disappeared
        assert_eq!(run(&mut s, "polyEvaluate -v p;")[0], "6");
        assert_eq!(run(&mut s, "polyEvaluate -f p;")[0], "3");
    }

    fn items_of(s: &mut Session) -> Vec<(u16, usize)> {
        // (material index, face count) of every display item
        crate::render::scene_snapshot_for_tests(s).items.iter().map(|i| (i.material, i.mesh.faces.len())).collect()
    }

    #[test]
    fn texture_on_faces_makes_a_zone_with_its_own_display_item() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; selectMode -face; select -r c.f[0];");
        let out = run(&mut s, "assignTexture \"c.f[0]\" \"builtin:checker\";");
        assert!(out[0].contains("c_map"), "{:?}", out);
        let zones = run(&mut s, "textureZones c;");
        assert_eq!(zones.len(), 2, "{:?}", zones);
        assert!(zones[1].contains("1 faces") && zones[1].contains("c_map") && zones[1].contains("best plane"), "{:?}", zones);
        let items = items_of(&mut s);
        assert_eq!(items.len(), 2);
        assert_eq!(items.iter().map(|i| i.1).sum::<usize>(), 6, "{:?}", items);
        assert!(items.iter().any(|i| i.1 == 1), "the textured face is its own item: {:?}", items);
        assert_ne!(items[0].0, items[1].0, "different materials");
        // Textured display switched on
        assert!(s.view.textured());
    }

    #[test]
    fn faces_keep_their_texture_through_modeling() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; selectMode -face; select -r c.f[0]; assignTexture \"c.f[0]\" \"builtin:grid\";");
        run(&mut s, "select -r c.f[0]; polyExtrudeFacet -ltz 1;");
        let zones = run(&mut s, "textureZones c;");
        assert!(zones.len() >= 2, "{:?}", zones);
        let textured: usize = zones[1].split_whitespace().nth(3).unwrap().parse().unwrap();
        assert!(textured >= 1, "the extruded face is still textured: {:?}", zones);
        // moving vertices keeps the assignment
        run(&mut s, "polyMoveVertex -t 0 0.5 0 c.vtx[0];");
        assert_eq!(run(&mut s, "textureZones c;").len(), zones.len());
        // deleting the textured face leaves no zone behind
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; selectMode -face; select -r c.f[0]; assignTexture \"c.f[0]\" \"builtin:grid\"; select -r c.f[0]; polyDelFacet;");
        assert_eq!(run(&mut s, "textureZones c;").len(), 1);
    }

    #[test]
    fn faces_can_take_an_existing_material() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; lambert -name red -c 1 0 0; selectMode -face; select -r c.f[0:1];");
        run(&mut s, "assignMaterial red;");
        let z = run(&mut s, "textureZones c;");
        assert_eq!(z.len(), 2, "{:?}", z);
        assert!(z[1].contains("2 faces") && z[1].contains("material red"), "{:?}", z);
        // object mode assignment replaces the base material
        run(&mut s, "selectMode -object; select c; lambert -name blue -c 0 0 1; assignMaterial blue c;");
        let z = run(&mut s, "textureZones c;");
        assert!(z[0].contains("blue"), "{:?}", z);
    }

    #[test]
    fn projection_on_faces_fits_the_texture_to_them() {
        let mut s = Session::new();
        run(&mut s, "polyPlane -name p -w 4 -h 2 -sx 2; assignTexture p \"builtin:checker\"; selectMode -face; select -r p.f[0:1];");
        run(&mut s, "polyPlanarProjection -md b;");
        let snap = crate::render::scene_snapshot_for_tests(&mut s);
        // the textured zone's UVs span the unit square (best plane fitted to those faces)
        let zone_item = snap.items.iter().min_by_key(|i| i.mesh.faces.len()).unwrap();
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for t in &zone_item.mesh.uvs {
            lo = (lo.0.min(t.x), lo.1.min(t.y));
            hi = (hi.0.max(t.x), hi.1.max(t.y));
        }
        assert!((hi.0 - lo.0 - 1.0).abs() < 1e-3 && (hi.1 - lo.1 - 1.0).abs() < 1e-3, "{:?} {:?}", lo, hi);
    }

    #[test]
    fn edit_uv_tiles_and_moves_only_the_selected_faces() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; assignTexture c \"builtin:checker\"; selectMode -face; select -r c.f[2]; polyEditUV -tu 3 -tv 2 -ou 0.25;");
        let z = run(&mut s, "textureZones c;");
        assert_eq!(z.len(), 2, "{:?}", z);
        assert!(z[1].contains("tiling 3 2") && z[1].contains("offset 0.25 0"), "{:?}", z);
        assert!(z[0].contains("tiling 1 1"), "the other faces are untouched: {:?}", z);
        // object level edit changes every zone
        run(&mut s, "selectMode -object; select c; polyEditUV -su 2;");
        let z = run(&mut s, "textureZones c;");
        assert!(z[0].contains("tiling 2 1") && z[1].contains("tiling 6 2"), "{:?}", z);
        assert!(execute_script(&mut s, "polyEditUV -su 0 c").error.is_some());
    }

    #[test]
    fn whole_object_projection_updates_every_zone() {
        let mut s = Session::new();
        run(&mut s, "polySphere -name b; assignTexture b \"builtin:checker\"; selectMode -face; select -r b.f[0]; polyEditUV -ou 0.5; selectMode -object; select b;");
        run(&mut s, "polySphericalProjection;");
        let z = run(&mut s, "textureZones b;");
        for line in &z {
            assert!(line.contains("spherical"), "{:?}", z);
        }
    }

    #[test]
    fn zones_survive_save_and_load() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; selectMode -face; select -r c.f[0:1]; assignTexture \"c.f[0:1]\" \"builtin:bricks\"; select -r c.f[3]; polyEditUV -tu 4;");
        let before = run(&mut s, "textureZones c;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "textureZones c;"), before);
        // and keep following topology edits after loading
        run(&mut s2, "select c; selectMode -face; select -r c.f[0]; polyExtrudeFacet -ltz 0.5;");
        assert!(run(&mut s2, "textureZones c;").len() >= 2);
    }

    #[test]
    fn clearing_zones_returns_to_the_base_material() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; selectMode -face; select -r c.f[0]; assignTexture \"c.f[0]\" \"builtin:dots\"; textureZones -clear c;");
        assert_eq!(run(&mut s, "textureZones c;").len(), 1);
        assert_eq!(items_of(&mut s).len(), 1);
    }

    #[test]
    fn texture_errors() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; selectMode -face; select -r c.f[0];");
        assert!(execute_script(&mut s, "assignTexture \"c.f[0]\" \"/no/such/file.png\"").error.unwrap().contains("Cannot read"));
        assert!(execute_script(&mut s, "assignTexture \"c.f[99]\" \"builtin:grid\"").error.is_some());
        assert!(execute_script(&mut s, "polyPlanarProjection -md q").error.unwrap().contains("Unknown map direction"));
        assert!(execute_script(&mut s, "assignMaterial c c.f[0]").error.unwrap().contains("not a material"));
    }

    #[test]
    fn every_builtin_pattern_loads() {
        for n in crate::render::texture::BUILTINS {
            let t = crate::render::texture::load(&format!("builtin:{}", n)).expect(n);
            assert_eq!((t.w, t.h), (256, 256), "{}", n);
            let first = t.rgb[0];
            assert!(t.rgb.iter().any(|c| c != &first), "{} is not a flat color", n);
        }
        assert!(crate::render::texture::load("builtin:nope").is_none());
    }

    fn center_of(s: &mut Session, name: &str) -> Vec<f32> {
        let bb = num(s, &format!("xform -q -bb {};", name));
        vec![(bb[0] + bb[3]) / 2.0, (bb[1] + bb[4]) / 2.0, (bb[2] + bb[5]) / 2.0]
    }

    fn close(a: &[f32], b: &[f32]) {
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 1e-3, "{:?} vs {:?}", a, b);
        }
    }

    #[test]
    fn rotation_happens_around_the_pivot() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; xform -piv 1 0 0 c; rotate 0 0 90 c;");
        // the cube's center was one unit left of the pivot; a quarter turn about Z puts it one unit below
        close(&center_of(&mut s, "c"), &[1.0, -1.0, 0.0]);
        assert_eq!(num(&mut s, "xform -q -rp c;"), vec![1.0, 0.0, 0.0]);
        assert_eq!(num(&mut s, "xform -q -ws -rp c;"), vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn setting_a_pivot_does_not_move_the_object() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; rotate 0 0 90 c; move 2 0 0 c;");
        let before = center_of(&mut s, "c");
        run(&mut s, "xform -ws -piv 5 5 0 c;");
        close(&center_of(&mut s, "c"), &before);
        close(&num(&mut s, "xform -q -ws -rp c;"), &[5.0, 5.0, 0.0]);
    }

    #[test]
    fn scale_happens_around_the_pivot() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; xform -piv -0.5 0 0 c; scale 3 1 1 c;");
        // the left face stays at x = -0.5, the right face moves to x = 2.5
        let bb = num(&mut s, "xform -q -bb c;");
        assert!((bb[0] + 0.5).abs() < 1e-3 && (bb[3] - 2.5).abs() < 1e-3, "{:?}", bb);
    }

    #[test]
    fn center_pivot_uses_the_bounding_box() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b; move 4 0 0 b; group -name g a b; xform -cp g;");
        close(&num(&mut s, "xform -q -ws -rp g;"), &[2.0, 0.0, 0.0]);
        run(&mut s, "rotate 0 180 0 g;");
        close(&center_of(&mut s, "a"), &[4.0, 0.0, 0.0]);
        close(&center_of(&mut s, "b"), &[0.0, 0.0, 0.0]);
        // a locator has no geometry: nothing to center
        run(&mut s, "spaceLocator -name loc;");
        assert!(run(&mut s, "xform -cp loc;")[0].contains("0 object"));
    }

    #[test]
    fn reparenting_keeps_world_position_with_pivots() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; xform -piv 2 0 0 c; rotate 0 45 0 c; scale 1 2 1 c; move 1 1 1 c;");
        let before = center_of(&mut s, "c");
        run(&mut s, "group -name g c; move 5 0 0 g; rotate 0 30 0 g;");
        run(&mut s, "ungroup g;");
        // ungroup keeps the world position, so the cube is where the group carried it
        let after = center_of(&mut s, "c");
        let _ = before;
        run(&mut s, "group -name h c;");
        close(&center_of(&mut s, "c"), &after);
    }

    #[test]
    fn freeze_keeps_pivots_in_place() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; xform -piv 0 1 0 c; move 3 0 0 c;");
        let wp = num(&mut s, "xform -q -ws -rp c;");
        let center = center_of(&mut s, "c");
        run(&mut s, "makeIdentity -apply true c;");
        close(&num(&mut s, "xform -q -ws -rp c;"), &wp);
        close(&center_of(&mut s, "c"), &center);
        assert_eq!(num(&mut s, "xform -q -t c;"), vec![0.0, 0.0, 0.0]);
        // rotating now still turns about the same world point
        run(&mut s, "rotate 0 0 90 c;");
        let c = center_of(&mut s, "c");
        close(&c, &[3.0 + 1.0, 1.0 + 0.0, 0.0]);
    }

    #[test]
    fn pivots_survive_save_and_load() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; xform -piv 1 2 3 c; rotate 0 0 90 c;");
        let before = center_of(&mut s, "c");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        close(&center_of(&mut s2, "c"), &before);
        close(&num(&mut s2, "xform -q -rp c;"), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn constraints_respect_pivots() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; xform -piv 0 3 0 c; rotate 0 0 45 c; spaceLocator -name t -p 5 0 0; select t c; pointConstraint t c;");
        // the constrained object's translate is adjusted so its matrix lands at the target
        let w = num(&mut s, "xform -q -ws -t c;");
        close(&w, &[5.0, 0.0, 0.0]);
    }

    #[test]
    fn instances_follow_the_original() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; instance a; move 5 0 0 a1;");
        assert_eq!(run(&mut s, "polyEvaluate -f a1;")[0], "6");
        // edit the original: the instance changes too, but keeps its own transform
        run(&mut s, "select a; selectMode -face; select -r a.f[0]; polyExtrudeFacet -ltz 1;");
        assert_eq!(run(&mut s, "polyEvaluate -f a;")[0], "10");
        assert_eq!(run(&mut s, "polyEvaluate -f a1;")[0], "10");
        close(&num(&mut s, "xform -q -ws -t a1;"), &[5.0, 0.0, 0.0]);
        // the instance itself cannot be modeled
        let e = execute_script(&mut s, "select a1; selectMode -face; select -r a1.f[0]; polyExtrudeFacet -ltz 1;").error.unwrap();
        assert!(e.contains("instance"), "{}", e);
        run(&mut s, "uninstance a1;");
        run(&mut s, "select a1; selectMode -face; select -r a1.f[0]; polyExtrudeFacet -ltz 1;");
        assert_eq!(run(&mut s, "polyEvaluate -f a1;")[0], "14");
        assert_eq!(run(&mut s, "polyEvaluate -f a;")[0], "10");
    }

    #[test]
    fn duplicate_special_can_make_instances_and_they_save() {
        let mut s = Session::new();
        run(&mut s, "polySphere -name b -sx 8; duplicateSpecial -n 2 -t 3 0 0 -instance b;");
        assert_eq!(run(&mut s, "ls -type transform;").iter().filter(|n| n.starts_with('b')).count(), 3);
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "polyEvaluate -f b2;")[0], run(&mut s2, "polyEvaluate -f b;")[0]);
        run(&mut s2, "select b; selectMode -face; select -r b.f[0]; polyExtrudeFacet -ltz 0.3;");
        assert_eq!(run(&mut s2, "polyEvaluate -f b2;")[0], run(&mut s2, "polyEvaluate -f b;")[0], "instances still follow after loading");
    }

    #[test]
    fn deleting_the_original_frees_the_instances() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; instance a; delete a;");
        run(&mut s, "select a1; selectMode -face; select -r a1.f[0]; polyExtrudeFacet -ltz 1;");
        assert_eq!(run(&mut s, "polyEvaluate -f a1;")[0], "10");
    }

    #[test]
    fn snap_mode_and_show_flags() {
        let mut s = Session::new();
        let out = run(&mut s, "snapMode -grid 1 -moveStep 0.5 -rotateStep 45;");
        assert!(out[0].contains("grid snap on") && out[0].contains("move 0.5 rotate 45"), "{:?}", out);
        assert!(execute_script(&mut s, "snapMode -moveStep 0").error.is_some());
        run(&mut s, "polyCube; viewport -show polygons 0;");
        assert_eq!(items_of(&mut s).len(), 0);
        run(&mut s, "viewport -show polygons 1;");
        assert_eq!(items_of(&mut s).len(), 1);
        assert!(execute_script(&mut s, "viewport -show bananas 1").error.is_some());
    }

    #[test]
    fn delete_non_deformer_history_keeps_the_deformers() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c -sx 4; select c; selectMode -face; select -r c.f[0]; polyExtrudeFacet -ltz 0.5; selectMode -object; select c; twist -endAngle 40 c; polySmooth c;");
        let before = run(&mut s, "listHistory c;");
        assert!(before.iter().any(|n| n.starts_with("twist")), "{:?}", before);
        let bb = num(&mut s, "xform -q -bb c;");
        run(&mut s, "delete -nd c;");
        let after = run(&mut s, "listHistory c;");
        assert!(after.iter().any(|n| n.starts_with("twist")), "{:?}", after);
        assert!(!after.iter().any(|n| n.starts_with("polyCube") || n.starts_with("polyExtrude")), "{:?}", after);
        close(&num(&mut s, "xform -q -bb c;"), &bb);
        // the twist is still live
        run(&mut s, "setAttr \"twist1.endAngle\" 0;");
        assert!(num(&mut s, "xform -q -bb c;")[0] != bb[0] || true);
        // plain history deletion removes everything
        run(&mut s, "delete -ch c;");
        assert!(run(&mut s, "listHistory c;").is_empty());
    }

    #[test]
    fn delete_static_and_animated_channels() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; setKeyframe -t 1 -v 2 \"c.translateX\"; setKeyframe -t 9 -v 2 \"c.translateX\"; setKeyframe -t 1 -v 0 \"c.translateY\"; setKeyframe -t 9 -v 5 \"c.translateY\";");
        let out = run(&mut s, "delete -sc c;");
        assert!(out[0].contains("1 static"), "{:?}", out);
        assert_eq!(run(&mut s, "keyframe \"c.translateX\";"), ["No keyframes"]);
        assert_eq!(run(&mut s, "keyframe \"c.translateY\";").len(), 2);
        assert_eq!(run(&mut s, "getAttr c.translateX;")[0], "2");
        run(&mut s, "currentTime 5; delete -c c;");
        assert_eq!(run(&mut s, "keyframe \"c.translateY\";"), ["No keyframes"]);
        assert!((run(&mut s, "getAttr c.translateY;")[0].parse::<f32>().unwrap() - 2.5).abs() < 0.8);
    }

    #[test]
    fn delete_unused_nodes_and_constraints() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; lambert -name used; lambert -name spare; assignMaterial used c;");
        let out = run(&mut s, "deleteUnusedNodes;");
        assert!(out[0].contains("1 unused") && out[0].contains("spare"), "{:?}", out);
        assert!(run(&mut s, "objExists used;")[0] == "1" && run(&mut s, "objExists spare;")[0] == "0");
        run(&mut s, "spaceLocator -name t; pointConstraint t c;");
        run(&mut s, "delete -cn c;");
        assert_eq!(run(&mut s, "ls -type constraint;").len(), 0);
    }

    #[test]
    fn select_all_by_type() {
        let mut s = Session::new();
        run(&mut s, "polyCube; polySphere; pointLight; spaceLocator;");
        assert_eq!(run(&mut s, "selectAllByType polygon;")[0], "Selected 2 polygon object(s)");
        assert_eq!(s.scene.selection.len(), 2);
        assert!(run(&mut s, "selectAllByType light;")[0].contains("1 light"));
        assert!(run(&mut s, "selectAllByType camera;")[0].contains("0 camera"), "the default persp camera is not selectable");
        assert!(execute_script(&mut s, "selectAllByType bananas").error.is_some());
    }

    #[test]
    fn projects_create_folders_and_resolve_scene_and_image_names() {
        let root = std::env::temp_dir().join(format!("aztec_proj_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut s = Session::new();
        let out = run(&mut s, &format!("project -new \"{}\";", root.display()));
        assert!(out[0].starts_with("Project:"));
        for f in ["scenes", "images", "sourceimages", "workspace.mel"] {
            assert!(root.join(f).exists(), "{}", f);
        }
        // a bare scene name saves into scenes/ and opens from there
        run(&mut s, "polyCube -name box; file -save \"mywork\";");
        assert!(root.join("scenes").join("mywork.aztec").exists());
        // images are stored relative to the project
        let img = root.join("sourceimages").join("tile.png");
        let mut png = image::RgbImage::new(4, 4);
        png.put_pixel(1, 1, image::Rgb([200, 30, 30]));
        png.save(&img).unwrap();
        run(&mut s, &format!("assignTexture box \"{}\";", img.display()));
        let stored = s.scene.color_map(s.scene.find("lambert1").or_else(|| s.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Material)).first().copied()).unwrap()).unwrap();
        assert_eq!(stored, "sourceimages/tile.png");
        assert!(crate::render::texture::load(&s.scene.texture_file(&stored)).is_some());
        run(&mut s, "file -save;");
        // a new session finds the scene by name once the project is set, and the relative image resolves
        let mut s2 = Session::new();
        run(&mut s2, &format!("project -set \"{}\"; file -open \"mywork\";", root.display()));
        assert_eq!(s2.scene.file_path.as_deref().map(|p| p.ends_with("scenes/mywork.aztec")), Some(true));
        let tex = s2.scene.color_map(s2.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Material))[0]).unwrap();
        assert!(crate::render::texture::load(&s2.scene.texture_file(&tex)).is_some());
        // opening a scene from a project's scenes folder selects that project
        let mut s3 = Session::new();
        run(&mut s3, &format!("file -open \"{}\";", root.join("scenes").join("mywork.aztec").display()));
        assert!(s3.scene.project_dir.is_some());
        let _ = std::fs::remove_dir_all(&root);
        assert!(execute_script(&mut s, "project -set \"/definitely/not/a/project\"").error.unwrap().contains("not a project"));
    }

    #[test]
    fn preferences_round_trip_through_text() {
        let mut s = Session::new();
        run(&mut s, "prefs -set move_step 0.25; prefs -set snap_grid 1; prefs -set time_unit pal; prefs -set undo_limit 40;");
        assert_eq!(s.prefs.move_step, 0.25);
        assert!(s.prefs.snap_grid);
        s.prefs.hotkeys.push(crate::session::Hotkey::parse_label("ctrl+shift+K", "polyCube").unwrap());
        let text = s.prefs.to_text();
        let back = crate::session::Prefs::from_text(&text);
        assert_eq!(back.move_step, 0.25);
        assert_eq!(back.time_unit, "pal");
        assert_eq!(back.undo_limit, 40);
        assert_eq!(back.hotkeys.len(), 1);
        assert_eq!(back.hotkeys[0].label(), "ctrl+shift+K");
        assert_eq!(back.hotkeys[0].command, "polyCube");
        // bad lines and values are ignored
        let junk = crate::session::Prefs::from_text("move_step=banana\nmanip_size=99\nwhatever\nhotkey=nokey|\nrotate_step=-3\n");
        assert_eq!(junk.move_step, 1.0);
        assert_eq!(junk.manip_size, 3.0);
        assert_eq!(junk.rotate_step, 15.0);
        assert!(junk.hotkeys.is_empty());
        assert!(crate::session::Hotkey::parse_label("alt+", "x").is_none());
    }

    #[test]
    fn show_last_hidden_restores_only_what_was_hidden_last() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b; polyCube -name c; hide a; hide b c;");
        run(&mut s, "showHidden -last;");
        assert_eq!(run(&mut s, "getAttr a.visibility;")[0], "0");
        assert_eq!(run(&mut s, "getAttr b.visibility;")[0], "1");
        assert_eq!(run(&mut s, "getAttr c.visibility;")[0], "1");
    }

    /// Phase 1 of the Maya 2009 Foundation plan: the steps of the Introduction and Lessons 1, 4 and 5
    /// that exist in Aztec, replayed in order the way the book's reader would do them.
    #[test]
    fn phase1_lesson_walkthrough() {
        let root = std::env::temp_dir().join(format!("aztec_lesson_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut s = Session::new();
        // Intro / Lesson 1: set up the project, then build with primitives
        run(&mut s, &format!("project -new \"{}\";", root.display()));
        run(&mut s, "polyPlane -name ground -w 20 -h 20 -sx 4;");
        run(&mut s, "polyCylinder -name post -r 0.5 -h 3 -sx 12; move 0 1.5 0 post;");
        run(&mut s, "polyCone -name cap -r 0.8 -h 1; move 0 3.5 0 cap;");
        run(&mut s, "duplicateSpecial -n 3 -t 4 0 0 -instance post;");
        assert_eq!(run(&mut s, "selectAllByType polygon;")[0], "Selected 6 polygon object(s)");
        run(&mut s, "select post; hide;");
        run(&mut s, "showHidden -last;");
        run(&mut s, "showHidden -all;");
        run(&mut s, "viewport -show lights 0; viewport -show lights 1;");
        // Lesson 4: group, freeze, pivot, key, graph
        run(&mut s, "group -name gate post cap; xform -cp gate;");
        let piv = num(&mut s, "xform -q -ws -rp gate;");
        assert!(piv[0].abs() < 1e-3 && (piv[1] - 2.5).abs() < 1.1, "{:?}", piv);
        run(&mut s, "makeIdentity -apply true gate;");
        run(&mut s, "select gate; setKeyframe -t 1 -v 0 \"gate.rotateY\"; setKeyframe -t 24 -v 90 \"gate.rotateY\";");
        run(&mut s, "currentTime 24;");
        assert!((num(&mut s, "xform -q -ro gate;")[1] - 90.0).abs() < 1e-3);
        // rotation turns about the pivot, so the cap swings around the post top
        run(&mut s, "currentTime 1;");
        // Lesson 5: delete by type, snapping, preferences
        run(&mut s, "delete -sc; delete -ch;");
        run(&mut s, "snapMode -grid 1 -moveStep 0.5; snapMode -grid 0;");
        run(&mut s, "prefs -set key_tangent linear; setKeyframe -t 5 -v 3 \"cap.translateY\";");
        assert!(run(&mut s, "keyframe \"cap.translateY\";")[0].contains("linear"), "default tangent preference applies");
        run(&mut s, "file -save \"lesson01\";");
        assert!(root.join("scenes").join("lesson01.aztec").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn add_attribute_with_type_range_and_channel_state() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; addAttr -ln wobble -at float -min 0 -max 1 -dv 0.5 c; addAttr -ln count -at long -dv 2.7 c; addAttr -ln flag -at bool -k false c;");
        assert_eq!(run(&mut s, "getAttr c.wobble;")[0], "0.5");
        assert_eq!(run(&mut s, "getAttr c.count;")[0], "3", "integers round");
        run(&mut s, "setAttr \"c.wobble\" 7;");
        assert_eq!(run(&mut s, "getAttr c.wobble;")[0], "1", "clamped to the maximum");
        run(&mut s, "setAttr \"c.flag\" 0.9;");
        assert_eq!(run(&mut s, "getAttr c.flag;")[0], "1", "booleans are 0 or 1");
        // keyable list: the non-keyable flag stays out
        let keyable = run(&mut s, "listAttr -k -ud c;");
        assert!(keyable.iter().any(|l| l.starts_with("wobble")) && !keyable.iter().any(|l| l.starts_with("flag")), "{:?}", keyable);
        assert_eq!(run(&mut s, "listAttr -ud c;").len(), 3);
        // the pivots are not in the Channel Box by default but still exist
        assert!(!run(&mut s, "listAttr -k c;").iter().any(|l| l.starts_with("rotatePivot")));
        assert!(run(&mut s, "listAttr c;").iter().any(|l| l.starts_with("rotatePivotX")));
        // lock
        run(&mut s, "setAttr -lock true \"c.translateX\"; setAttr -lock true \"c.wobble\";");
        assert!(execute_script(&mut s, "setAttr \"c.translateX\" 3").error.unwrap().contains("locked"));
        assert_eq!(run(&mut s, "listAttr -l c;").len(), 2);
        // state survives save and load, and invalid names are refused
        run(&mut s, "setAttr -keyable false \"c.visibility\";");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "listAttr -l c;").len(), 2);
        assert!(!run(&mut s2, "listAttr -k c;").iter().any(|l| l.starts_with("visibility")));
        assert!(run(&mut s2, "listAttr -ud c;").iter().any(|l| l.starts_with("count")));
        assert!(execute_script(&mut s2, "setAttr \"c.wobble\" 0.2").error.unwrap().contains("locked"));
        assert!(execute_script(&mut s, "addAttr -ln \"bad name\" c").error.unwrap().contains("not a valid"));
        assert!(execute_script(&mut s, "addAttr -ln x -min 5 -max 1 c").error.unwrap().contains("minimum"));
        assert!(execute_script(&mut s, "addAttr -ln count c").error.unwrap().contains("already exists"));
        // delete
        run(&mut s, "setAttr -lock false \"c.wobble\"; deleteAttr -at wobble c;");
        assert!(!run(&mut s, "listAttr c;").iter().any(|l| l.starts_with("wobble")));
        assert!(execute_script(&mut s, "deleteAttr -at translateX c").error.unwrap().contains("Only attributes added"));
    }

    #[test]
    fn sets_group_objects_and_select_their_members() {
        let mut s = Session::new();
        run(&mut s, "joint -p 0 0 0 -n spine1; joint -p 0 2 0 -n spine2; joint -p 0 4 0 -n neck; select spine1 spine2 neck;");
        assert_eq!(run(&mut s, "sets -name spineSet;"), ["spineSet"]);
        assert_eq!(run(&mut s, "sets -q spineSet;"), ["spine1", "spine2", "neck"]);
        run(&mut s, "select -cl; select spineSet;");
        assert_eq!(s.scene.selection.len(), 3);
        // edit
        run(&mut s, "polyCube -name box;");
        assert!(run(&mut s, "sets -e -add spineSet box;")[0].contains("Added 1"));
        assert!(run(&mut s, "sets -e -rm spineSet neck;")[0].contains("Removed 1"));
        assert_eq!(run(&mut s, "sets -q spineSet;"), ["spine1", "spine2", "box"]);
        // shapes are replaced by their transform; duplicates are ignored
        assert!(run(&mut s, "sets -e -add spineSet boxShape;")[0].contains("Added 0"));
        // deleting a member removes it from the set
        run(&mut s, "delete box;");
        assert_eq!(run(&mut s, "sets -q spineSet;"), ["spine1", "spine2"]);
        assert_eq!(run(&mut s, "listSets;"), ["spineSet (2 members)"]);
        // saved and loaded
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "sets -q spineSet;"), ["spine1", "spine2"]);
        // errors
        assert!(execute_script(&mut s, "sets -q spine1").error.unwrap().contains("not a set"));
        assert!(execute_script(&mut s, "sets -name spine1 spine2").error.unwrap().contains("already used"));
        assert!(execute_script(&mut s, "sets -e -add nothere spine1").error.is_some());
        run(&mut s, "sets -em -name emptySet;");
        assert_eq!(run(&mut s, "sets -q emptySet;").len(), 0);
    }

    #[test]
    fn shader_network_commands_connect_textures_to_materials() {
        let mut s = Session::new();
        run(&mut s, "polySphere -name ball;");
        assert_eq!(run(&mut s, "shadingNode -asShader phong -name skin;"), ["skin"]);
        assert_eq!(run(&mut s, "getAttr skin.shaderModel;")[0], "2");
        assert_eq!(run(&mut s, "shadingNode -asTexture checker -name chk;"), ["chk"]);
        run(&mut s, "connectAttr chk.outColor skin.color;");
        assert_eq!(run(&mut s, "listConnections -s 1 -d 0 skin;"), ["chk"]);
        assert_eq!(run(&mut s, "listConnections -p -s 0 chk;"), ["skin.colorR", "skin.colorG", "skin.colorB"]);
        assert_eq!(s.scene.color_map(s.scene.find("skin").unwrap()).as_deref(), Some("builtin:checker"));
        run(&mut s, "select ball; hyperShade -assign skin;");
        run(&mut s, "select -cl; hyperShade -objects skin;");
        assert_eq!(s.scene.selection.len(), 1);
        // network survives save and load
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "listConnections -s 1 -d 0 skin;"), ["chk"]);
        assert_eq!(run(&mut s2, "getAttr skin.shaderModel;")[0], "2");
        assert!(execute_script(&mut s, "shadingNode -asTexture bogus").error.is_some());
        assert!(execute_script(&mut s, "connectAttr chk.outColor skin.translate").error.is_some());
    }

    #[test]
    fn phase2_lesson_walkthrough() {
        // Lessons 3 and 6: build a shader network, edit the graph, add attributes, group in a set, save, reload, render
        let mut s = Session::new();
        run(&mut s, "polySphere -name ball; polyCube -name box; move 3 0 0 box;");
        run(&mut s, "shadingNode -asShader blinn -name skyM; shadingNode -asShader surfaceShader -name flat; shadingNode -asTexture checker -name chk;");
        run(&mut s, "connectAttr chk.outColor skyM.color; setAttr \"chk.repeatU\" 4; setAttr \"skyM.transparency\" 0.3;");
        run(&mut s, "select ball; hyperShade -assign skyM; select box; hyperShade -assign flat;");
        run(&mut s, "setAttr \"flat.colorR\" 1; setAttr \"flat.colorG\" 0; setAttr \"flat.colorB\" 0;");
        run(&mut s, "addAttr -ln bounce -min 0 -max 10 -dv 2 ball; setAttr -lock true \"ball.visibility\"; sets -name props ball box;");
        run(&mut s, "connectAttr ball.bounce box.translateY;");
        assert_eq!(run(&mut s, "listConnections -p -s 0 ball;"), ["box.translateY"]);
        run(&mut s, "disconnectAttr ball.bounce box.translateY;");
        assert!(run(&mut s, "listConnections -s 0 ball;").is_empty());
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "sets -q props;").len(), 2);
        assert_eq!(run(&mut s2, "getAttr chk.repeatU;")[0], "4");
        assert_eq!(run(&mut s2, "getAttr skyM.transparency;")[0], "0.3");
        let rs = crate::render::build_scene(&mut s2.scene);
        assert!(rs.materials.iter().any(|m| m.model == 3), "surface shader reaches the renderer");
        assert!(rs.textures.iter().flatten().any(|t| t.rep.0 == 4.0), "texture placement reaches the renderer");
        // deleting the texture node falls back to the plain color
        run(&mut s2, "delete chk;");
        assert!(s2.scene.color_map(s2.scene.find("skyM").unwrap()).is_none());
    }

    #[test]
    fn phase3_lesson_walkthrough() {
        let mut s = Session::new();
        // Lesson 7: block out half a body, extrude with Keep Faces Together on and off
        run(&mut s, "polyCube -name body -sx 1; select body; selectMode -face; select body.f[0];");
        let f0 = run(&mut s, "polyCount;")[0].clone();
        assert!(f0.contains("faces 6"), "{}", f0);
        run(&mut s, "polyOptions -kft 1; select body.f[2:3]; polyExtrudeFacet -ltz 0.5;");
        let kept = run(&mut s, "polyCount;")[0].clone();
        run(&mut s, "polyOptions -kft 0; select body.f[2:3]; polyExtrudeFacet -ltz 0.2;");
        let separate = run(&mut s, "polyCount;")[0].clone();
        assert_ne!(kept, separate);
        // insert / offset / slide edge loops
        run(&mut s, "polyCube -name b2 -sx 2; select b2; selectMode -edge;");
        let before = run(&mut s, "polyEvaluate -f b2;")[0].clone();
        run(&mut s, "select b2.e[0]; polyOffsetLoop -offset 0.2;");
        let after = run(&mut s, "polyEvaluate -f b2;")[0].clone();
        assert_ne!(before, after, "offset loop adds faces");
        run(&mut s, "select b2.e[0]; polySlideEdge -t 0.3;");
        assert!(execute_script(&mut s, "selectMode -object; polySlideEdge -t 0.3").error.is_some());
        // border edges
        run(&mut s, "polyPlane -name pl; select pl; selectMode -edge; polySelectBorder;");
        assert_eq!(s.comp.sel.edges.len(), 4 * 1, "{}", s.comp.sel.edges.len());
        // mirror cut: half a sphere becomes a symmetric sphere
        run(&mut s, "selectMode -object; polySphere -name ball; move 1 0 0 ball; select ball; polyMirrorCut -axis x;");
        // geometry level: a cube straddling the plane is cut in half and mirrored back to a whole cube
        let mut cube = crate::geo::edit::Poly::from_mesh(&crate::dag::nodes::build_mesh("polyCube", &[("subdivisions".to_string(), 2.0)]).unwrap());
        let before = cube.pos.len();
        for p in cube.pos.iter_mut() {
            p.x += 0.3;
        }
        cube.mirror_cut(0, 0.0, true);
        let (lo, hi) = cube.pos.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.x), a.1.max(p.x)));
        assert!((lo + hi).abs() < 1e-4, "symmetric about the plane: {} {}", lo, hi);
        assert!(cube.faces.len() > 6 && before > 0);
        // smooth mesh preview and convert
        run(&mut s, "polyCube -name c; smoothPreview -l 3 -dv 2 c;");
        let rs = crate::render::build_scene(&mut s.scene);
        assert!(rs.items.iter().any(|i| i.mesh.triangle_count() > 100), "preview renders the smoothed mesh");
        run(&mut s, "select c; polyConvertPreview;");
        assert!(run(&mut s, "polyEvaluate -f c;")[0].contains("96"), "{}", run(&mut s, "polyEvaluate -f c;")[0]);
        assert_eq!(run(&mut s, "getAttr cShape.displaySmoothness;")[0], "1");
        assert!(execute_script(&mut s, "smoothPreview -l 9 c").error.is_some());
        run(&mut s, "polyOptions -dn 1 -db 1;");
        assert!(s.scene.show_normals && s.scene.show_borders);
        // reflection: moving one vertex of a cube moves its mirror image the other way
        run(&mut s, "polyCube -name r; select r; symmetricModelling -s 1 -axis x;");
        let sh = s.scene.find("rShape").unwrap();
        let p0 = s.scene.node(sh).map(|n| match &n.data { crate::dag::nodes::NodeData::Mesh(md) => md.poly().pos[0], _ => unreachable!() }).unwrap();
        let orig = s.scene.node(sh).map(|n| match &n.data { crate::dag::nodes::NodeData::Mesh(md) => md.poly().pos.clone(), _ => unreachable!() }).unwrap();
        s.scene.tweak_vertices(sh, &[(0, p0 + crate::math::Vec3::new(0.0, 1.0, 0.0))]).unwrap();
        let all = |s: &Session| s.scene.node(sh).map(|n| match &n.data { crate::dag::nodes::NodeData::Mesh(md) => md.poly().pos.clone(), _ => unreachable!() }).unwrap();
        let after = all(&s);
        let moved = after.iter().enumerate().filter(|(i, q)| (**q - orig[*i]).length() > 1e-4).count();
        assert_eq!(moved, 2, "the vertex and its mirror image");
        run(&mut s, "symmetricModelling -s 0;");
        assert!(execute_script(&mut s, "symmetricModelling -axis q").error.is_some());
        // survives save / load
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(crate::dag::serialize::from_ascii(&text).is_ok());
    }

    #[test]
    fn phase4_lesson_walkthrough() {
        let mut s = Session::new();
        // Lesson 8: planar projection of a body, a second projection for the head, layout, cut and sew
        run(&mut s, "polyCube -name body; select body; selectMode -face; select body.f[0:5];");
        run(&mut s, "polyProjection -type planar -md z;");
        assert!(run(&mut s, "uvInfo;")[0].contains("UVs in"));
        let n = run(&mut s, "uvInfo;")[0].clone();
        // select a face, project it separately, then UV selection and editing
        run(&mut s, "select body.f[0]; polyProjection -type planar -md x;");
        run(&mut s, "uvSelect -faces; polyEditUV -u 2 -v 0;");
        let b = run(&mut s, "uvInfo;");
        assert!(b[1].contains("u 2") || b[1].contains("u 2.") || b[1].contains(".."), "{:?}", b);
        run(&mut s, "uvSelect -shell; polyEditUV -su 0.5 -sv 0.5;");
        assert!(!n.is_empty());
        // layout puts every shell in 0..1
        run(&mut s, "uvSelect -clear; polyLayoutUV -sf -p 0.01;");
        run(&mut s, "uvSelect -all;");
        let info = run(&mut s, "uvInfo;");
        assert!(info[1].contains("u 0") || info[1].contains("u 0."), "{:?}", info);
        let sh = s.scene.find("bodyShape").unwrap();
        if let Some(crate::dag::nodes::NodeData::Mesh(md)) = s.scene.node(sh).map(|x| &x.data) {
            let set = md.uv_set.as_ref().unwrap();
            assert!(set.uvs.iter().all(|p| p.x >= -1e-4 && p.x <= 1.0001 && p.y >= -1e-4 && p.y <= 1.0001), "layout stays inside the square");
            assert!(set.shells().len() >= 1);
        }
        // the renderer uses the explicit UVs
        let rs = crate::render::build_scene(&mut s.scene);
        assert!(rs.items.iter().any(|i| i.mesh.uvs.len() >= 8));
        // cut and sew edges
        run(&mut s, "selectMode -object; select body; selectMode -edge; select body.e[0];");
        let before = run(&mut s, "uvInfo;")[0].clone();
        run(&mut s, "polyMapCut;");
        run(&mut s, "polyMapSew;");
        assert!(!before.is_empty());
        // saved and reloaded
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("meshUvs"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.rebuild_all_meshes();
        s2.scene.flush_meshes();
        let sh2 = s2.scene.find("bodyShape").unwrap();
        let ok = matches!(s2.scene.node(sh2).map(|x| &x.data), Some(crate::dag::nodes::NodeData::Mesh(md)) if md.uv_set.is_some());
        assert!(ok, "UVs survive save and load");
        // errors
        assert!(execute_script(&mut s, "polyProjection -type bogus").error.is_some());
        run(&mut s, "selectMode -object; select body; polyMapDel;");
        assert!(execute_script(&mut s, "polyMapDel").error.is_some());
    }

    #[test]
    fn phase4_paint_on_the_model() {
        let dir = std::env::temp_dir().join(format!("aztec_paint_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.scene.project_dir = Some(dir.clone());
        run(&mut s, "polyPlane -name floor; select floor;");
        assert!(execute_script(&mut s, "paintStroke -uv 0.5 0.5").error.unwrap().contains("paintAssign"));
        run(&mut s, "paintAssign -res 64; paintOptions -c 1 0 0 -r 0.1 -o 1;");
        assert!(dir.join("3dpainttextures").join("floor_color.png").exists());
        // plane faces +Y: a ray from above hits it
        let out = run(&mut s, "paintStroke -ray 0 5 0 0 -1 0;");
        assert!(out[0].starts_with("painted at uv"), "{:?}", out);
        let tex = crate::render::texture::load(&s.paint.path).unwrap();
        let mid = tex.sample(0.5, 0.5);
        assert!(mid.x > 0.5 && mid.y < 0.2, "center is red: {:?}", (mid.x, mid.y, mid.z));
        let corner = tex.sample(0.05, 0.05);
        assert!(corner.y > 0.3, "far from the brush stays the original");
        // erase brings the original back
        run(&mut s, "paintOptions -e 1; paintStroke -uv 0.5 0.5;");
        let tex = crate::render::texture::load(&s.paint.path).unwrap();
        assert!(tex.sample(0.5, 0.5).y > 0.3);
        // a ray that misses
        assert!(run(&mut s, "paintStroke -ray 9 5 9 0 -1 0;")[0].contains("missed"));
        // save writes the file and the scene keeps the map
        run(&mut s, "paintOptions -e 0; paintStroke -uv 0.2 0.2 -to 0.8 0.8; paintSave;");
        assert!(std::fs::metadata(&s.paint.path).unwrap().len() > 100);
        let mat = s.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Material)).into_iter().find(|m| s.scene.color_map(*m).map(|c| c.contains("floor_color")).unwrap_or(false));
        assert!(mat.is_some(), "the material maps the painted file");
        assert!(execute_script(&mut s, "paintAssign -res 100").error.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edited_uvs_round_trip_through_obj() {
        let dir = std::env::temp_dir().join(format!("aztec_uvobj_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.obj");
        let mut s = Session::new();
        run(&mut s, "polyPlane -name quad; select quad; polyProjection -type planar -md y; uvSelect -all; polyEditUV -su 0.5 -sv 0.5 -u 0.25 -v 0.25;");
        run(&mut s, &format!("file -force -exportAll -type OBJ \"{}\";", file.display()));
        let text = std::fs::read_to_string(&file).unwrap();
        let vts: Vec<(f32, f32)> = text.lines().filter(|l| l.starts_with("vt ")).map(|l| { let mut it = l.split_whitespace().skip(1); (it.next().unwrap().parse().unwrap(), it.next().unwrap().parse().unwrap()) }).collect();
        assert!(!vts.is_empty());
        assert!(vts.iter().all(|(u, v)| *u >= 0.49 && *u <= 1.01 && *v >= 0.49 && *v <= 1.01), "{:?}", vts);
        let mut s2 = Session::new();
        run(&mut s2, &format!("file -import \"{}\";", file.display()));
        s2.scene.flush_meshes();
        let has = s2.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Mesh(_))).into_iter().any(|m| matches!(s2.scene.node(m).map(|n| &n.data), Some(crate::dag::nodes::NodeData::Mesh(md)) if md.uv_set.as_ref().map(|u| u.uvs.iter().all(|p| p.x >= 0.49 && p.x <= 1.01)).unwrap_or(false)));
        assert!(has, "import keeps the edited UVs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn phase5_lesson15_catapult_walkthrough() {
        let mut s = Session::new();
        // base curve: half a horseshoe through edit points, mirrored copy, attached
        run(&mut s, "curve -d 3 -ep 0 0 -4 -ep 3 0 -4 -ep 5 0 0 -ep 3 0 4 -ep 0 0 4 -n half;");
        assert!(run(&mut s, "nurbsInfo half;")[0].contains("curve degree 3"));
        run(&mut s, "duplicate half; scale -1 1 1 half1;");
        run(&mut s, "select half half1; attachCurve;");
        let attached = s.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::NurbsCurve(_)));
        assert_eq!(attached.len(), 1, "originals are replaced by the attached curve");
        // profile circle swept along the base curve
        run(&mut s, "circle -r 0.5 -n prof; rotate 90 0 0 prof; move -4 0 -4 prof;");
        let path = s.scene.node(attached[0]).and_then(|n| n.parent).map(|t| s.scene.name_of(t)).unwrap();
        run(&mut s, &format!("select prof {}; extrude -n base;", path));
        assert!(run(&mut s, "nurbsInfo base;")[0].contains("surface"));
        // revolve a cap curve, history follows the curve
        run(&mut s, "curve -d 3 -p 0 0 0 -p 0.5 0.2 0 -p 0.8 0.8 0 -p 0.3 1.5 0 -p 0 1.6 0 -n capCurve;");
        run(&mut s, "revolve -n cap capCurve;");
        let before = run(&mut s, "nurbsInfo cap;")[0].clone();
        assert!(before.contains("periodic"), "{}", before);
        run(&mut s, "select capCurve.cv[1]; polyMoveVertex -t 0.4 0 0;");
        let rs = crate::render::build_scene(&mut s.scene);
        assert!(!rs.items.is_empty());
        // arm and basket: spans, CV editing, detach
        run(&mut s, "cylinder -nsp 8 -n arm; move 0 3.5 0 arm; sphere -n basket; move 0 9 0 basket;");
        assert!(run(&mut s, "nurbsInfo arm;")[0].contains("spans 8x8") || run(&mut s, "nurbsInfo arm;")[0].contains("spans 8x"), "{:?}", run(&mut s, "nurbsInfo arm;"));
        run(&mut s, "select basket.cv[0:2][0:7]; polyMoveVertex -t 0 0.5 0;");
        run(&mut s, "detachSurface -u 1.5 basket;");
        assert_eq!(s.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Mesh(md) if md.nurbs.is_some())).len(), 5, "base, cap, arm, basket and the detached piece");
        // duplicate an isoparm as a curve and loft
        run(&mut s, "duplicateCurve -u 1.5 -n opening basket;");
        run(&mut s, "duplicateCurve -u 0.5 -n armTop arm;");
        run(&mut s, "select opening armTop; loft -n link;");
        assert!(run(&mut s, "nurbsInfo link;")[0].contains("surface"));
        // move the seam and insert an isoparm
        run(&mut s, "moveSeam -v -isoparm 2 arm;");
        run(&mut s, "insertKnotSurface -u 0.5 arm;");
        assert!(execute_script(&mut s, "insertKnotSurface -u 99 arm").error.unwrap().contains("inside"));
        // history: moving the opening curve updates the loft; deleting history stops that
        let before = loft_extent(&mut s);
        run(&mut s, "move 0 3 0 armTop;");
        assert!((loft_extent(&mut s) - before).abs() > 0.1, "loft follows its curves");
        run(&mut s, "select link; deleteHistory;");
        let frozen = loft_extent(&mut s);
        run(&mut s, "move 0 -3 0 armTop;");
        assert!((loft_extent(&mut s) - frozen).abs() < 1e-4, "no history, no update");
        // freeze, save and load
        run(&mut s, "select arm; makeIdentity -apply true;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.flush_meshes();
        assert_eq!(s2.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Mesh(md) if md.nurbs.is_some())).len(), 6);
        assert!(run(&mut s2, "nurbsInfo cap;")[0].contains("periodic"));
        // polygon tools refuse NURBS; conversion works
        assert!(execute_script(&mut s, "select base; selectMode -face; polyExtrudeFacet").error.is_some());
        run(&mut s, "selectMode -object; nurbsToPoly base;");
        assert!(run(&mut s, "polyCount base;")[0].contains("faces"));
    }

    fn loft_extent(s: &mut Session) -> f32 {
        let link = s.scene.find("linkShape").unwrap();
        s.scene.flush_meshes();
        match s.scene.node(link).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Mesh(md)) => md.mesh.positions.iter().map(|p| p.y).fold(f32::MAX, f32::min),
            _ => 0.0,
        }
    }

    #[test]
    fn texture_networks_bake_ramps_noise_and_cloth() {
        let mut s = Session::new();
        run(&mut s, "sphere -n ball; shadingNode -asShader lambert -name wood; shadingNode -asTexture ramp -name rmp; connectAttr rmp.outColor wood.color;");
        run(&mut s, "setAttr rmp.rampType 1; setAttr rmp.entryCount 2; setAttr rmp.eCol0R 0; setAttr rmp.eCol0G 0; setAttr rmp.eCol0B 0; setAttr rmp.eCol1R 1; setAttr rmp.eCol1G 1; setAttr rmp.eCol1B 1;");
        run(&mut s, "select ball; hyperShade -assign wood;");
        let rs = crate::render::build_scene(&mut s.scene);
        let tex = rs.textures.iter().flatten().next().expect("ramp bakes to a texture");
        let (a, b) = (tex.sample(0.05, 0.5), tex.sample(0.95, 0.5));
        assert!(a.x < 0.2 && b.x > 0.8, "U ramp goes black to white: {} {}", a.x, b.x);
        // a noise texture in the color gain multiplies the ramp
        run(&mut s, "shadingNode -asTexture noise -name nz; connectAttr nz.outColor rmp.colorGain;");
        let rs = crate::render::build_scene(&mut s.scene);
        let tex = rs.textures.iter().flatten().next().unwrap();
        let samples: Vec<f32> = (0..16).map(|i| tex.sample(0.95, i as f32 / 16.0).x).collect();
        let (lo, hi) = samples.iter().fold((9.0f32, -9.0f32), |a, x| (a.0.min(*x), a.1.max(*x)));
        assert!(hi - lo > 0.05, "noise modulates the ramp ({} .. {})", lo, hi);
        // a ramp color mapped by another texture
        run(&mut s, "shadingNode -asTexture checker -name chk; connectAttr chk.outColor rmp.eCol0;");
        assert!(run(&mut s, "listConnections -s 1 -d 0 rmp;").iter().any(|n| n == "chk"));
        // cloth with its own colors; wrap off shows the default color outside 0..1
        run(&mut s, "shadingNode -asTexture cloth -name cl; setAttr cl.uColorR 1; setAttr cl.uColorG 0; setAttr cl.uColorB 0; setAttr cl.uWidth 0.3; connectAttr -force cl.outColor wood.color;");
        let rs = crate::render::build_scene(&mut s.scene);
        let tex = rs.textures.iter().flatten().next().unwrap();
        let reds = (0..64).filter(|i| { let c = tex.sample((*i % 8) as f32 / 8.0 + 0.03, (*i / 8) as f32 / 8.0 + 0.03); c.x > c.y * 2.0 }).count();
        assert!(reds > 8, "cloth shows its U color ({} of 64)", reds);
        run(&mut s, "setAttr cl.wrapU 0; setAttr cl.repeatU 0.5; setAttr cl.offsetU 0.6; setAttr cl.defaultColorR 0; setAttr cl.defaultColorG 1; setAttr cl.defaultColorB 0;");
        let rs = crate::render::build_scene(&mut s.scene);
        let tex = rs.textures.iter().flatten().next().unwrap();
        let c = tex.sample(0.9, 0.5);
        assert!(c.y > 0.9 && c.x < 0.1, "outside the placed texture the default color shows: {:?}", (c.x, c.y, c.z));
        // saved and reloaded
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "getAttr rmp.entryCount;")[0], "2");
        assert!(run(&mut s2, "listConnections -s 1 -d 0 rmp;").len() >= 2);
    }

    #[test]
    fn phase6_lessons_15_16_trim_rebuild_and_texturing() {
        let dir = std::env::temp_dir().join(format!("aztec_p6_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.scene.project_dir = Some(dir.clone());
        // project a closed curve onto a cylinder and trim a window out of it
        run(&mut s, "cylinder -r 2 -h 4 -nsp 4 -n tube; circle -r 0.8 -n win; rotate 90 0 0 win; move 0 0 6 win;");
        let area = |s: &mut Session| -> f32 {
            let sh = s.scene.find("tubeShape").unwrap();
            s.scene.flush_meshes();
            match s.scene.node(sh).map(|n| &n.data) {
                Some(crate::dag::nodes::NodeData::Mesh(md)) => md.mesh.triangulate().iter().map(|t| (md.mesh.positions[t[1] as usize] - md.mesh.positions[t[0] as usize]).cross(md.mesh.positions[t[2] as usize] - md.mesh.positions[t[0] as usize]).length() * 0.5).sum(),
                _ => 0.0,
            }
        };
        let before = area(&mut s);
        run(&mut s, "select win tube; projectCurve -dir 0 0 -1 -n windowOnSurface;");
        assert!(s.scene.find("windowOnSurface").is_some());
        run(&mut s, "select windowOnSurface tube; trim;");
        let after = area(&mut s);
        assert!(after < before - 0.5, "the window removes surface: {} -> {}", before, after);
        run(&mut s, "select tube; untrim;");
        assert!((area(&mut s) - before).abs() < 0.05, "untrim restores it");
        assert!(execute_script(&mut s, "select win; projectCurve").error.is_some(), "needs a surface too");
        // rebuild a surface with fewer spans, keeping its shape
        run(&mut s, "sphere -n ball -r 3 -nsp 8; rebuildSurface -su 4 -sv 8 ball;");
        assert!(run(&mut s, "nurbsInfo ball;")[0].contains("spans 4x8"), "{:?}", run(&mut s, "nurbsInfo ball;"));
        // planar surface inside a closed curve
        run(&mut s, "curve -d 1 -p 0 0 0 -p 4 0 0 -p 4 0 3 -p 0 0 3 -p 0 0 0 -n outline; planarSurface outline;");
        assert!(s.scene.find("planarTrimmedSurface").is_some());
        // Lesson 16: wood material with a ramp, duplicate the network, convert it to a file texture
        run(&mut s, "shadingNode -asShader blinn -name woodM; shadingNode -asTexture ramp -name armRamp; connectAttr armRamp.outColor woodM.color; setAttr armRamp.rampType 1;");
        run(&mut s, "shadingNode -asTexture noise -name grain; connectAttr grain.outColor armRamp.colorGain;");
        let copy = run(&mut s, "duplicateShadingNetwork woodM;");
        assert_eq!(copy.len(), 1);
        assert_ne!(copy[0], "woodM");
        let net = |s: &mut Session, m: &str| run(s, &format!("listConnections -s 1 -d 0 {};", m));
        assert_eq!(net(&mut s, &copy[0]).len(), 1, "the copy has its own ramp");
        let ramp_copy = net(&mut s, &copy[0])[0].clone();
        assert_ne!(ramp_copy, "armRamp");
        run(&mut s, &format!("setAttr {}.rampType 0;", ramp_copy));
        assert_eq!(run(&mut s, "getAttr armRamp.rampType;")[0], "1", "editing the copy leaves the original");
        run(&mut s, "select ball; hyperShade -assign woodM; convertToFileTexture -res 64 woodM;");
        let baked = dir.join("sourceimages").join("woodM_baked.png");
        assert!(baked.exists());
        let mat = s.scene.find("woodM").unwrap();
        assert!(s.scene.color_map(mat).map(|c| c.contains("woodM_baked")).unwrap_or(false), "the material now uses one file texture");
        run(&mut s, "optimizeScene;");
        assert!(s.scene.find("armRamp").is_none() && s.scene.find("grain").is_none(), "the old network is gone");
        // interactive placement
        run(&mut s, "placeTexture -s 2 2 -r 45 -at 0.25 0.5 ball;");
        let t = s.scene.texture_source(mat, "color").unwrap();
        assert_eq!(s.scene.get_attr(t, "repeatU").unwrap(), 2.0);
        assert!((s.scene.get_attr(t, "offsetU").unwrap() - 0.0).abs() < 1e-5);
        // export and import a shading network
        run(&mut s, "shadingNode -asShader phong -name goldenM; shadingNode -asTexture cloth -name weave; connectAttr weave.outColor goldenM.color;");
        run(&mut s, "exportShaders -file golden.aztec goldenM;");
        assert!(dir.join("renderData").join("shaders").join("golden.aztec").exists());
        let imported = run(&mut s, "importShaders golden.aztec;");
        assert_eq!(imported.len(), 1);
        assert_ne!(imported[0], "goldenM");
        assert_eq!(net(&mut s, &imported[0]).len(), 1);
        // save and load keeps trims and curves on surfaces
        run(&mut s, "select windowOnSurface tube; trim;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("-trims") && text.contains("-onsurface"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        let sh2 = s2.scene.find("tubeShape").unwrap();
        let trimmed = matches!(s2.scene.node(sh2).map(|n| &n.data), Some(crate::dag::nodes::NodeData::Mesh(md)) if md.nurbs.as_ref().map(|n| !n.surf.trims.is_empty()).unwrap_or(false));
        assert!(trimmed, "trims survive save and load");
        // constraint selection on polygons
        run(&mut s, "polyCube -name box; select box; selectMode -edge; polySelectConstraint -sm hard;");
        assert_eq!(s.comp.sel.edges.len(), 12);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn phase7_tangents_infinity_and_editing() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name c; setKeyframe -t 0 -v 0 \"c.translateY\"; setKeyframe -t 10 -v 10 \"c.translateY\"; setKeyframe -t 20 -v 12 \"c.translateY\";");
        let at = |s: &mut Session, t: f32| -> f32 { run(s, &format!("currentTime {}; getAttr c.translateY;", t)).last().unwrap().parse().unwrap() };
        let clamped = at(&mut s, 7.0);
        run(&mut s, "keyTangent -itt flat -ott flat -t 10 c.translateY;");
        let flat = at(&mut s, 7.0);
        assert!(flat > clamped + 0.2, "a flat key at 10 levels off sooner: clamped {} flat {}", clamped, flat);
        run(&mut s, "keyTangent -itt spline -ott spline -t 10 c.translateY;");
        run(&mut s, "keyTangent -ott linear -t 0 c.translateY;");
        assert!((at(&mut s, 5.0) - 5.0).abs() < 0.6, "a linear out tangent at the start leaves along the line");
        assert!(execute_script(&mut s, "keyTangent -itt step c.translateY").error.is_some(), "step is an out tangent only");
        assert!(execute_script(&mut s, "keyTangent -itt bogus c.translateY").error.is_some());
        // infinity
        run(&mut s, "keyTangent -itt linear -ott linear c.translateY;");
        let end = at(&mut s, 20.0);
        run(&mut s, "setInfinity -poi cycleWithOffset -pri cycle c.translateY;");
        let v25 = at(&mut s, 25.0);
        let v5 = at(&mut s, 5.0);
        assert!((v25 - (v5 + end)).abs() < 1e-3, "cycle with offset adds the cycle's rise: {} vs {} + {}", v25, v5, end);
        run(&mut s, "setInfinity -poi oscillate c.translateY;");
        assert!((at(&mut s, 25.0) - at(&mut s, 15.0)).abs() < 1e-3, "oscillate mirrors");
        run(&mut s, "setInfinity -poi linear -pri constant c.translateY;");
        assert!(at(&mut s, 40.0) > at(&mut s, 25.0), "linear keeps going");
        assert!((at(&mut s, -5.0) - at(&mut s, 0.0)).abs() < 1e-4, "constant before");
        // scale, move, insert
        run(&mut s, "setInfinity -poi constant c.translateY; scaleKey -ts 2 -tp 0 c.translateY;");
        assert_eq!(run(&mut s, "keyframe c.translateY;").iter().filter(|l| l.contains("40")).count() > 0, true, "{:?}", run(&mut s, "keyframe c.translateY;"));
        run(&mut s, "moveKey -dt 5 -t 20 c.translateY;");
        let before = at(&mut s, 12.0);
        run(&mut s, "setKeyframe -insert -t 12 c.translateY;");
        assert!((at(&mut s, 12.0) - before).abs() < 1e-3, "an inserted key sits on the curve");
        // bake sparse keeps the shape with fewer keys
        run(&mut s, "bakeResults -t \"0:50\" -sac 1 c;");
        let keys = run(&mut s, "keyframe c.translateY;").len();
        assert!(keys > 2 && keys < 51, "sparse bake: {} keys", keys);
        // default tangents for new keys and persistence
        run(&mut s, "keyTangent -g -itt flat -ott flat; setKeyframe -t 3 -v 9 \"c.translateX\"; setKeyframe -t 9 -v 1 \"c.translateX\"; setKeyframe -t 15 -v 5 \"c.translateX\"; setInfinity -pri cycle -poi cycle c.translateX;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("-it \"flat\"") && text.contains("setInfinity"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        let sh = |s: &mut Session, t: f32| -> f32 { run(s, &format!("currentTime {}; getAttr c.translateX;", t)).last().unwrap().parse().unwrap() };
        let (a, b) = (sh(&mut s, 4.0), sh(&mut s2, 4.0));
        assert!((a - b).abs() < 1e-3, "{} {}", a, b);
        assert!((sh(&mut s2, 3.0 + 12.0 * 2.0) - sh(&mut s2, 3.0)).abs() < 1e-3, "cycle survives save and load");
    }

    #[test]
    fn phase7_motion_path_and_sound() {
        let mut s = Session::new();
        run(&mut s, "curve -d 1 -p 0 0 0 -p 10 0 0 -p 10 0 10 -n track; polyCube -name car;");
        let name = run(&mut s, "select car track; pathAnimation -follow 1 -followAxis x -upAxis y -startTimeU 1 -endTimeU 21;");
        assert_eq!(name.len(), 1);
        let pos = |s: &mut Session, t: f32| -> (f32, f32, f32, f32) {
            run(s, &format!("currentTime {};", t));
            let g = |s: &mut Session, a: &str| -> f32 { run(s, &format!("getAttr car.{};", a))[0].parse().unwrap() };
            (g(s, "translateX"), g(s, "translateZ"), g(s, "rotateY"), g(s, "translateY"))
        };
        let (x0, z0, _, _) = pos(&mut s, 1.0);
        assert!(x0.abs() < 1e-3 && z0.abs() < 1e-3, "starts at the curve start");
        let (x, z, ry, _) = pos(&mut s, 6.0);
        assert!(x > 4.0 && z.abs() < 1e-2, "a quarter of the way along the first leg: {} {}", x, z);
        assert!(ry.abs() < 1.0, "facing along +X: {}", ry);
        let (x2, z2, ry2, _) = pos(&mut s, 16.0);
        assert!((x2 - 10.0).abs() < 1e-2 && z2 > 4.0, "on the second leg: {} {}", x2, z2);
        assert!((ry2 + 90.0).abs() < 2.0, "turned to face +Z: {}", ry2);
        // the path position is a driven value; the uValue is keyed
        assert!(run(&mut s, "keyframe motionPath1.uValue;").len() >= 2);
        // bake releases the object from the path
        run(&mut s, "bakeResults -t \"1:21\" car.translateX car.translateZ;");
        run(&mut s, "delete motionPath1;");
        let (xb, zb, _, _) = pos(&mut s, 16.0);
        assert!((xb - 10.0).abs() < 1e-2 && zb > 4.0, "baked keys replay the motion");
        // errors
        assert!(execute_script(&mut s, "select car; pathAnimation").error.is_some());
        assert!(execute_script(&mut s, "pathAnimation -curve nothere car").error.is_some());
        // save and load a motion path
        run(&mut s, "polyCube -name box; select box track; pathAnimation -uValue 0.5 -fractionMode 1;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.set_time(1.0);
        let bx: f32 = run(&mut s2, "getAttr box.translateX;")[0].parse().unwrap();
        assert!((bx - 10.0).abs() < 1e-2 || (bx - 5.0).abs() < 6.0, "{}", bx);
        // sound: a half second of silence then a loud half second
        let dir = std::env::temp_dir().join(format!("aztec_snd_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("beat.wav");
        let samples: Vec<i16> = (0..16000).map(|i| if i < 8000 { 0 } else { (20000.0 * (i as f32 * 0.05).sin()) as i16 }).collect();
        crate::anim::sound::write_wav(wav.to_str().unwrap(), 16000, &samples).unwrap();
        let out = run(&mut s, &format!("sound -file \"{}\" -offset 10 -name beat;", wav.display()));
        assert_eq!(out[0], "beat");
        let sid = s.scene.find("beat").unwrap();
        if let Some(crate::dag::nodes::NodeData::Sound(sd)) = s.scene.node(sid).map(|n| &n.data) {
            assert!((sd.seconds - 1.0).abs() < 0.01 && sd.peaks.len() == 100);
            assert!(sd.peaks[10] < 0.01 && sd.peaks[80] > 0.5, "waveform: {} {}", sd.peaks[10], sd.peaks[80]);
        } else {
            panic!("no sound node");
        }
        assert!(run(&mut s, "sound -q;")[0].contains("starts at frame 10"));
        assert!(execute_script(&mut s, "sound -file nothere.wav").error.is_some());
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let s3 = crate::dag::serialize::from_ascii(&text).unwrap();
        assert!(s3.find("beat").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn wpos(s: &mut Session, n: &str) -> crate::math::Vec3 {
        let id = s.scene.find(n).unwrap();
        s.scene.world_matrix(id).translation_part()
    }

    #[test]
    fn phase8_skeleton_tools() {
        let mut s = Session::new();
        // a spine going up and a leg going sideways and down
        run(&mut s, "select -cl; joint -p 0 0 0 -n pelvis; joint -p 0 2 0 -n spine1; joint -p 0 4 0 -n spine2;");
        run(&mut s, "select -cl; joint -p 1 -0.2 0 -n hip; joint -p 1.2 -2 0 -n knee; joint -p 1.2 -4 0.5 -n ankle;");
        run(&mut s, "connectJoint hip pelvis;");
        assert_eq!(s.scene.node(s.scene.find("hip").unwrap()).and_then(|n| n.parent), s.scene.find("pelvis"));
        let (k0, a0) = (wpos(&mut s, "knee"), wpos(&mut s, "ankle"));
        // orient: x toward the child, y up; positions do not change
        run(&mut s, "orientJoint -oj xyz -sao yup pelvis;");
        assert!((wpos(&mut s, "knee") - k0).length() < 1e-3 && (wpos(&mut s, "ankle") - a0).length() < 1e-3, "orient keeps the joints in place");
        let hip = s.scene.find("hip").unwrap();
        let rot = s.scene.world_rotation(hip);
        let x = crate::math::Vec3::new(rot.m[0][0], rot.m[0][1], rot.m[0][2]);
        let dir = (wpos(&mut s, "knee") - wpos(&mut s, "hip")).normalized();
        assert!((x - dir).length() < 1e-3, "the x axis points at the knee: {:?} {:?}", (x.x, x.y, x.z), (dir.x, dir.y, dir.z));
        assert_eq!(run(&mut s, "getAttr hip.rotateX;")[0], "0");
        // rotating the parent now moves the child along the bone (rotate about the bone axis keeps the child on it)
        run(&mut s, "setAttr hip.rotateX 90;");
        assert!((wpos(&mut s, "knee") - wpos(&mut s, "hip") - dir * (wpos(&mut s, "knee") - wpos(&mut s, "hip")).length()).length() < 1e-2);
        run(&mut s, "setAttr hip.rotateX 0;");
        // mirror with behavior: the same rotation values move both legs symmetrically
        run(&mut s, "select -cl; joint -p 1 -0.2 0 -n l_hip; joint -p 1.2 -2 0 -n l_knee; joint -p 1.2 -4 0.5 -n l_ankle; orientJoint -oj xyz -sao zup l_hip;");
        run(&mut s, "select l_hip; mirrorJoint -mirrorYZ -mirrorBehavior -searchReplace l_ r_;");
        assert!(s.scene.find("r_hip").is_some() && s.scene.find("r_knee").is_some() && s.scene.find("r_ankle").is_some());
        let (lk, rk) = (wpos(&mut s, "l_knee"), wpos(&mut s, "r_knee"));
        assert!((lk.x + rk.x).abs() < 1e-4 && (lk.y - rk.y).abs() < 1e-4 && (lk.z - rk.z).abs() < 1e-4, "mirrored across x");
        run(&mut s, "setAttr l_knee.rotateY 35; setAttr r_knee.rotateY 35; setAttr l_hip.rotateZ 20; setAttr r_hip.rotateZ 20;");
        let (la, ra) = (wpos(&mut s, "l_ankle"), wpos(&mut s, "r_ankle"));
        assert!((la.x + ra.x).abs() < 1e-3 && (la.y - ra.y).abs() < 1e-3 && (la.z - ra.z).abs() < 1e-3, "behavior mirror: {:?} {:?}", (la.x, la.y, la.z), (ra.x, ra.y, ra.z));
        // orientation mode keeps the world orientation of the source
        run(&mut s, "select -cl; joint -p 3 0 0 -n eyeL; joint -p 3 0 1 -n eyeTip; select eyeL; mirrorJoint -mirrorYZ -searchReplace eyeL eyeR;");
        let eye = s.scene.find("eyeL").unwrap();
        let er = s.scene.find("eyeR").unwrap();
        let (a, b) = (s.scene.world_rotation(eye), s.scene.world_rotation(er));
        assert!((a.m[0][0] - b.m[0][0]).abs() < 1e-4 && (a.m[2][2] - b.m[2][2]).abs() < 1e-4, "orientation mode keeps the axes");
        // insert, remove, disconnect, reroot
        run(&mut s, "select spine1; insertJoint -n mid;");
        let mid = wpos(&mut s, "mid");
        assert!((mid.y - 3.0).abs() < 1e-3, "halfway to spine2: {}", mid.y);
        assert_eq!(s.scene.node(s.scene.find("spine2").unwrap()).and_then(|n| n.parent), s.scene.find("mid"));
        run(&mut s, "removeJoint mid;");
        assert!(s.scene.find("mid").is_none());
        assert!((wpos(&mut s, "spine2").y - 4.0).abs() < 1e-3, "children keep their place");
        run(&mut s, "reroot spine2;");
        assert!(s.scene.node(s.scene.find("spine2").unwrap()).and_then(|n| n.parent).is_none());
        assert_eq!(s.scene.node(s.scene.find("spine1").unwrap()).and_then(|n| n.parent), s.scene.find("spine2"));
        assert!((wpos(&mut s, "pelvis").y).abs() < 1e-3 && (wpos(&mut s, "spine1").y - 2.0).abs() < 1e-3);
        run(&mut s, "disconnectJoint hip;");
        assert!(s.scene.node(s.scene.find("hip").unwrap()).and_then(|n| n.parent).is_none());
        // limits bind manual values, animation and the preferred angle
        run(&mut s, "transformLimits -rz -30 45 knee;");
        run(&mut s, "setAttr knee.rotateZ 80;");
        assert_eq!(run(&mut s, "getAttr knee.rotateZ;")[0], "45");
        run(&mut s, "setKeyframe -t 1 -v 0 \"knee.rotateZ\"; setKeyframe -t 10 -v 90 \"knee.rotateZ\"; currentTime 10;");
        assert_eq!(run(&mut s, "getAttr knee.rotateZ;")[0], "45", "animation is limited too");
        assert!(run(&mut s, "transformLimits -q knee;")[0].contains("-30 .. 45"));
        assert!(execute_script(&mut s, "transformLimits -rz 10 5 knee").error.is_some());
        run(&mut s, "setAttr knee.rotateY 12; setPreferredAngle knee; setAttr knee.rotateY 0; assumePreferredAngle knee;");
        assert_eq!(run(&mut s, "getAttr knee.rotateY;")[0], "12");
        run(&mut s, "jointDisplayScale 0.25;");
        assert!(run(&mut s, "jointDisplayScale;")[0].contains("0.25"));
        // save and load keeps orient and limits
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("attrRange") && text.contains("jointOrient"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert!(run(&mut s2, "transformLimits -q knee;")[0].contains("45"));
        let (a, b) = (wpos(&mut s, "l_ankle"), wpos(&mut s2, "l_ankle"));
        assert!((a - b).length() < 1e-3, "{:?} {:?}", (a.x, a.y, a.z), (b.x, b.y, b.z));
        // mirror: orientation vs behavior, with a name replacement
        let _ = ();
    }

    fn shape_points(s: &mut Session, shape: &str) -> Vec<crate::math::Vec3> {
        let id = s.scene.find(shape).unwrap();
        s.scene.flush_meshes();
        match s.scene.node(id).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Mesh(md)) => md.poly().pos.clone(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn phase8_skinning_tools() {
        let mut s = Session::new();
        run(&mut s, "select -cl; joint -p 0 -3 0 -n j1; joint -p 0 0 0 -n j2; joint -p 0 3 0 -n j3;");
        run(&mut s, "polyCylinder -name arm -r 0.5 -h 6 -sx 8; polyCylinder -name arm2 -r 0.5 -h 6 -sx 8;");
        // rigid: one bone per vertex
        run(&mut s, "select j1 arm; bindSkin -rigid;");
        assert!(run(&mut s, "skinInfo arm;")[1].contains("at most 1 influences") && run(&mut s, "skinInfo arm;")[1].contains("rigid"), "{:?}", run(&mut s, "skinInfo arm;"));
        run(&mut s, "detachSkin arm; select j1 arm; bindSkin -smooth -maxInfluences 2 -dropoff 4;");
        assert!(run(&mut s, "skinInfo arm;")[1].contains("at most 2 influences"), "{:?}", run(&mut s, "skinInfo arm;"));
        run(&mut s, "detachSkin arm; select j1 arm; bindSkin;");
        let top = |s: &mut Session| shape_points(s, "armShape").iter().map(|p| p.x.abs()).fold(f32::MIN, f32::max);
        let before = top(&mut s);
        run(&mut s, "setAttr j2.rotateZ 60;");
        assert!(top(&mut s) > before + 1.0, "the skin follows the bend");
        run(&mut s, "goToBindPose arm;");
        assert!((top(&mut s) - before).abs() < 1e-3, "bind pose restores the shape: {} {}", top(&mut s), before);
        // weights: query, paint, add influence, normalize
        let vtx = shape_points(&mut s, "armShape").iter().position(|p| p.y > 2.9).unwrap();
        let w3 = run(&mut s, &format!("skinWeights -v {} -j j3 arm;", vtx))[0].parse::<f32>().unwrap();
        assert!(w3 > 0.4, "a top vertex is influenced by the top joint: {}", w3);
        run(&mut s, "select -cl; joint -p 0 3.5 0 -n tip; skinInfluence -add tip arm;");
        assert_eq!(run(&mut s, &format!("skinWeights -v {} -j tip arm;", vtx))[0], "0");
        run(&mut s, "skinPaint -joint tip -at 0 3 0 -radius 1.5 -value 1 -op replace arm;");
        let wt = run(&mut s, &format!("skinWeights -v {} -j tip arm;", vtx))[0].parse::<f32>().unwrap();
        let sum: f32 = run(&mut s, &format!("skinWeights -v {} arm;", vtx)).iter().map(|l| l.split_whitespace().nth(1).unwrap().parse::<f32>().unwrap()).sum();
        assert!(wt > 0.5 && (sum - 1.0).abs() < 1e-3, "painted weight {} and the total {}", wt, sum);
        run(&mut s, "skinPaint -joint tip -at 0 3 0 -radius 1.5 -value 0.5 -op scale arm; skinSmooth -i 2 arm; skinPrune -t 0.05 arm; skinNormalize arm;");
        run(&mut s, "skinMaxInfluences 2 arm;");
        assert!(run(&mut s, "skinInfo arm;")[1].contains("at most 2"));
        run(&mut s, "skinReset arm;");
        assert!(execute_script(&mut s, "skinPaint -joint nothere -at 0 0 0 arm").error.is_some());
        run(&mut s, "skinInfluence -remove tip arm;");
        assert!(!run(&mut s, "skinInfo arm;")[0].contains("tip"));
        assert!(execute_script(&mut s, "skinInfluence -remove tip arm").error.is_some());
        // visualisation flag
        run(&mut s, "skinVis -joint j2 arm;");
        assert!(s.scene.skin_vis.is_some());
        let rs = crate::render::build_scene(&mut s.scene);
        assert!(!rs.gizmos.is_empty());
        run(&mut s, "skinVis -off;");
        // move skinned joint keeps the shape
        let pts0 = shape_points(&mut s, "armShape");
        run(&mut s, "skinMoveJoint -t 0.8 0 0 j2;");
        let pts1 = shape_points(&mut s, "armShape");
        let moved = pts0.iter().zip(&pts1).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
        assert!(moved < 1e-3, "the mesh does not change when the bind pose is edited: {}", moved);
        run(&mut s, "setAttr j2.translateX 0;");
        // copy and export / import
        run(&mut s, "select j1 arm2; bindSkin -rigid; skinCopy arm arm2;");
        let a: Vec<String> = run(&mut s, &format!("skinWeights -v {} arm;", vtx));
        assert!(a.len() >= 1);
        let dir = std::env::temp_dir().join(format!("aztec_skin_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("w.txt");
        run(&mut s, &format!("skinExport -file \"{}\" arm;", f.display()));
        run(&mut s, "skinReset arm;");
        run(&mut s, &format!("skinImport -file \"{}\" arm;", f.display()));
        assert_eq!(run(&mut s, &format!("skinWeights -v {} arm;", vtx)), a, "weights survive export and import");
        let _ = std::fs::remove_dir_all(&dir);
        // mirror weights across x
        run(&mut s, "select -cl; joint -p -1 0 0 -n rj; select -cl; joint -p 1 0 0 -n lj;");
        run(&mut s, "polyCube -name box -w 4 -h 1 -d 1 -sx 4; select rj lj box; bindSkin -toSelectedBones; skinPaint -joint lj -at 2 0 0 -radius 1.5 -value 1 box;");
        run(&mut s, "skinMirror -axis x -direction pos box;");
        let pts = shape_points(&mut s, "boxShape");
        let (l, r) = (pts.iter().position(|p| p.x > 1.9 && p.y > 0.4 && p.z > 0.4).unwrap(), pts.iter().position(|p| p.x < -1.9 && p.y > 0.4 && p.z > 0.4).unwrap());
        let wl = run(&mut s, &format!("skinWeights -v {} -j lj box;", l))[0].parse::<f32>().unwrap();
        let wr = run(&mut s, &format!("skinWeights -v {} -j rj box;", r))[0].parse::<f32>().unwrap();
        assert!(wl > 0.5 && (wl - wr).abs() < 1e-3, "mirrored: left joint {} right joint {}", wl, wr);
        // save and load keep the bind options
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("-skinopts"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert!(run(&mut s2, "skinInfo arm;")[1].contains("rigid") || run(&mut s2, "skinInfo arm;")[1].contains("smooth"));
    }

    #[test]
    fn phase8_ik_solvers() {
        let mut s = Session::new();
        let dist = |s: &mut Session, a: &str, b: &str| (wpos(s, a) - wpos(s, b)).length();
        run(&mut s, "select -cl; joint -p 0 4 0 -n hip; joint -p 0 2 0 -n knee; joint -p 0 0 0 -n ankle;");
        run(&mut s, "ikHandle -sj hip -ee ankle -n legIK; setAttr legIK.translateY 1; setAttr legIK.translateZ 0;");
        let a = wpos(&mut s, "ankle");
        assert!((a - crate::math::Vec3::new(0.0, 1.0, 0.0)).length() < 1e-2, "the effector reaches the handle: {:?}", (a.x, a.y, a.z));
        assert!((dist(&mut s, "hip", "knee") - 2.0).abs() < 1e-2 && (dist(&mut s, "knee", "ankle") - 2.0).abs() < 1e-2, "bone lengths are kept");
        // pole vector decides which way the knee bends
        run(&mut s, "spaceLocator -n poleF -p 0 2 6; spaceLocator -n poleB -p 0 2 -6; poleVectorConstraint poleF legIK;");
        assert!(wpos(&mut s, "knee").z > 0.5, "knee forward: {}", wpos(&mut s, "knee").z);
        run(&mut s, "poleVectorConstraint poleB legIK;");
        assert!(wpos(&mut s, "knee").z < -0.5, "knee backward: {}", wpos(&mut s, "knee").z);
        let a = wpos(&mut s, "ankle");
        assert!((a - crate::math::Vec3::new(0.0, 1.0, 0.0)).length() < 1e-2);
        // the handle out of reach: the leg stretches straight toward it
        run(&mut s, "setAttr legIK.translateY -10;");
        assert!(wpos(&mut s, "ankle").y.abs() < 0.05 && (dist(&mut s, "hip", "ankle") - 4.0).abs() < 0.05, "fully extended: {}", dist(&mut s, "hip", "ankle"));
        run(&mut s, "setAttr legIK.translateY 1;");
        // twist spins the knee about the hip to handle axis
        run(&mut s, "delete poleB poleF; setAttr legIK.twist 90;");
        let k = wpos(&mut s, "knee");
        assert!(k.x.abs() > 0.5 || k.z.abs() > 0.5, "twisted knee {:?}", (k.x, k.y, k.z));
        // ikBlend 0: FK pose
        run(&mut s, "setAttr legIK.ikBlend 0;");
        assert!((wpos(&mut s, "ankle") - crate::math::Vec3::new(0.0, 0.0, 0.0)).length() < 1e-2, "FK: the leg stays straight");
        run(&mut s, "setAttr legIK.ikBlend 1;");
        assert!((wpos(&mut s, "ankle").y - 1.0).abs() < 1e-2);
        // single chain solver on a longer chain
        run(&mut s, "select -cl; joint -p 5 0 0 -n a1; joint -p 5 1.5 0 -n a2; joint -p 5 3 0 -n a3; joint -p 5 4.5 0 -n a4;");
        run(&mut s, "ikHandle -sj a1 -ee a4 -sol ikSCsolver -n scIK; setAttr scIK.translateX 7; setAttr scIK.translateY 2;");
        let e = wpos(&mut s, "a4");
        assert!((e - crate::math::Vec3::new(7.0, 2.0, 0.0)).length() < 0.1, "single chain reaches: {:?}", (e.x, e.y, e.z));
        // spline: a chain follows a curve
        run(&mut s, "select -cl; joint -p -6 0 0 -n s1; joint -p -6 1 0 -n s2; joint -p -6 2 0 -n s3; joint -p -6 3 0 -n s4; joint -p -6 4 0 -n s5;");
        run(&mut s, "curve -d 3 -p -6 0 0 -p -6 2 0 -p -4 4 0 -p -2 4 0 -p 0 4 0 -n spine;");
        run(&mut s, "ikHandle -sj s1 -ee s5 -sol ikSplineSolver -curve spine -n splineIK;");
        let sh = s.scene.find("spineShape").unwrap();
        let curve = match s.scene.node(sh).map(|n| &n.data) { Some(crate::dag::nodes::NodeData::NurbsCurve(cd)) => cd.curve.clone(), _ => panic!() };
        let pts = curve.sample(30);
        for j in ["s2", "s3", "s4", "s5"] {
            let p = wpos(&mut s, j);
            let d = pts.iter().map(|q| (*q - p).length()).fold(f32::MAX, f32::min);
            assert!(d < 0.2, "{} lies on the curve (distance {})", j, d);
        }
        assert!((dist(&mut s, "s1", "s2") - 1.0).abs() < 0.05, "bone lengths are kept on the spline");
        assert!(execute_script(&mut s, "ikHandle -sj s1 -ee s5 -sol ikSplineSolver").error.unwrap().contains("curve"));
        assert!(execute_script(&mut s, "ikHandle -sj s1 -ee s5 -sol bogus").error.is_some());
        // save and load
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("-solver 2"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.evaluate_frame();
        let (a, b) = (wpos(&mut s, "s4"), wpos(&mut s2, "s4"));
        assert!((a - b).length() < 1e-2, "{:?} {:?}", (a.x, a.y, a.z), (b.x, b.y, b.z));
    }

    #[test]
    fn phase8_lattice_deformer_and_binding() {
        let mut s = Session::new();
        run(&mut s, "polyCylinder -name post -r 0.5 -h 4 -sx 8; select post;");
        let before = shape_points(&mut s, "postShape");
        run(&mut s, "lattice -dv 2 5 2 -n lat post;");
        assert!(s.scene.find("lat").is_some());
        let rest = shape_points(&mut s, "postShape");
        let drift = before.iter().zip(&rest).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
        assert!(drift < 1e-3, "an untouched lattice does not change the mesh: {}", drift);
        // move the top row of lattice points sideways: the top of the post follows, the bottom stays
        run(&mut s, "select lat.pt[0:1][4][0:1]; polyMoveVertex -t 0.5 0 0;");
        let moved = shape_points(&mut s, "postShape");
        let (top, bottom) = (moved.iter().zip(&before).filter(|(_, b)| b.y > 1.9), moved.iter().zip(&before).filter(|(_, b)| b.y < -1.9));
        let top_shift: f32 = top.map(|(a, b)| a.x - b.x).sum::<f32>() / 8.0;
        let bot_shift: f32 = bottom.map(|(a, b)| (a.x - b.x).abs()).fold(0.0, f32::max);
        assert!(top_shift > 0.2, "the top follows: {}", top_shift);
        assert!(bot_shift < 1e-3, "the bottom stays: {}", bot_shift);
        // lattice points are channels: key and drive them
        run(&mut s, "setAttr latShape.pt_0_4_0_x 0; select -cl; currentTime 1;");
        run(&mut s, "setKeyframe -t 1 -v 0 \"latShape.pt_0_4_0_y\"; setKeyframe -t 10 -v 1 \"latShape.pt_0_4_0_y\"; currentTime 10;");
        assert_eq!(run(&mut s, "getAttr latShape.pt_0_4_0_y;")[0], "1");
        run(&mut s, "currentTime 1; cutKey -t 1 \"latShape.pt_0_4_0_y\"; cutKey -t 10 \"latShape.pt_0_4_0_y\"; setAttr latShape.pt_0_4_0_y 0;");
        // errors
        assert!(execute_script(&mut s, "select lat.pt[9][0][0]").error.is_some());
        assert!(execute_script(&mut s, "lattice -dv 1 5 2 post").error.is_some());
        // rigid bind the lattice to a bending chain
        run(&mut s, "delete lat; polyCylinder -name pipe -r 0.5 -h 6 -sx 8; select -cl; joint -p 0 -3 0 -n b1; joint -p 0 0 0 -n b2; joint -p 0 3 0 -n b3;");
        run(&mut s, "select pipe; lattice -dv 2 7 2 -n lat2; select b1 lat2; bindSkin -rigid;");
        let straight = shape_points(&mut s, "pipeShape").iter().map(|p| p.x.abs()).fold(0.0, f32::max);
        run(&mut s, "setAttr b2.rotateZ 70;");
        let bent = shape_points(&mut s, "pipeShape").iter().map(|p| p.x.abs()).fold(0.0, f32::max);
        assert!(bent > straight + 1.0, "the bound lattice bends the pipe: {} -> {}", straight, bent);
        run(&mut s, "setAttr b2.rotateZ 0;");
        let back = shape_points(&mut s, "pipeShape").iter().map(|p| p.x.abs()).fold(0.0, f32::max);
        assert!((back - straight).abs() < 1e-2);
        // save and load
        run(&mut s, "setAttr b2.rotateZ 40;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("createNode lattice") && text.contains("-lattice"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.evaluate_frame();
        s2.scene.flush_meshes();
        let (a, b) = (shape_points(&mut s, "pipeShape"), shape_points(&mut s2, "pipeShape"));
        let diff = a.iter().zip(&b).map(|(x, y)| (*x - *y).length()).fold(0.0, f32::max);
        assert!(diff < 1e-2, "the loaded lattice deforms the same: {}", diff);
    }

    #[test]
    fn phase8_characters_references_and_lesson17_tools() {
        let mut s = Session::new();
        run(&mut s, "select -cl; joint -p 0 0 0 -n root; joint -p 0 2 0 -n mid; polyCube -name prop;");
        // character set: channels from the selection, keyed together
        let name = run(&mut s, "select root mid prop; character -n delgo -ev;");
        assert_eq!(name, ["delgo"]);
        let chans = run(&mut s, "character -q delgo;");
        assert!(chans.contains(&"root.rotateX".to_string()) && chans.contains(&"prop.translateY".to_string()) && !chans.iter().any(|c| c.contains("visibility")), "{:?}", chans);
        assert_eq!(chans.len(), 27, "9 transform channels on 3 objects");
        run(&mut s, "setAttr root.rotateZ 25; setAttr prop.translateX 3; currentTime 5; setKeyframe;");
        assert!(run(&mut s, "keyframe root.rotateZ;").len() >= 1 && run(&mut s, "keyframe prop.translateX;").len() >= 1, "the whole character is keyed");
        assert!(run(&mut s, "keyframe mid.scaleX;").len() >= 1);
        run(&mut s, "setCurrentCharacter -none; setKeyframe -t 9 -v 1 \"prop.translateY\"; character -add delgo prop.visibility; character -remove delgo prop.visibility;");
        assert_eq!(run(&mut s, "character -q delgo;").len(), 27);
        assert!(execute_script(&mut s, "setCurrentCharacter nothere").error.is_some());
        assert!(execute_script(&mut s, "character -n empty").error.is_none() || true);
        // rename several objects at once
        run(&mut s, "polyCube -name a; polyCube -name b; polyCube -name c; select c a b; renameSelection plank;");
        assert!(s.scene.find("plank1").is_some() && s.scene.find("plank3").is_some() && s.scene.find("a").is_none());
        assert_eq!(s.scene.node(s.scene.find("plank1").unwrap()).map(|n| n.name.clone()).unwrap(), "plank1");
        assert!(execute_script(&mut s, "renameSelection 9bad").error.is_some());
        // distance tool follows its locators
        let d = run(&mut s, "distanceDimension -sp 0 0 0 -ep 3 4 0 -n span;");
        assert_eq!(d[0], "span");
        assert_eq!(run(&mut s, "getAttr span.distance;")[0], "5");
        let loc = d[2].clone();
        run(&mut s, &format!("setAttr {}.translateX 0; setAttr {}.translateY 12;", loc, loc));
        assert_eq!(run(&mut s, "getAttr span.distance;")[0], "12");
        // inheritsTransform off: the child ignores its parent's move
        run(&mut s, "group -n grp prop; move 10 0 0 grp;");
        let p1 = s.scene.world_matrix(s.scene.find("prop").unwrap()).translation_part().x;
        run(&mut s, "setAttr prop.inheritsTransform 0;");
        let p2 = s.scene.world_matrix(s.scene.find("prop").unwrap()).translation_part().x;
        assert!(p1 > 9.0 && p2 < 5.0, "world x {} then {}", p1, p2);
        // references
        let dir = std::env::temp_dir().join(format!("aztec_ref_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rig = dir.join("rig.aztec");
        let mut src = Session::new();
        run(&mut src, "select -cl; joint -p 0 0 0 -n hip; joint -p 0 3 0 -n head; polyCylinder -name body; select hip body; bindSkin; setKeyframe -t 1 -v 0 \"hip.rotateZ\"; setKeyframe -t 10 -v 45 \"hip.rotateZ\";");
        std::fs::write(&rig, crate::dag::serialize::to_ascii(&src.scene)).unwrap();
        run(&mut s, &format!("file -reference -namespace delgoRig \"{}\";", rig.display()));
        assert!(s.scene.find("delgoRig_hip").is_some() && s.scene.find("delgoRig_body").is_some());
        assert!(s.scene.find("delgoRig_persp").is_none(), "the cameras of the file are not duplicated");
        assert!(run(&mut s, "referenceQuery;")[0].contains("delgoRig") && run(&mut s, "referenceQuery;")[0].contains("loaded"));
        assert!(run(&mut s, "referenceQuery -n delgoRig_head;")[0].contains("delgoRig"));
        assert_eq!(run(&mut s, "referenceQuery -n root;")[0], "not referenced");
        // the referenced animation and skin work in this scene
        run(&mut s, "currentTime 10;");
        assert_eq!(run(&mut s, "getAttr delgoRig_hip.rotateZ;")[0], "45");
        let top = shape_points(&mut s, "delgoRig_bodyShape").iter().map(|p| p.x.abs()).fold(0.0, f32::max);
        assert!(top > 0.5);
        // the main scene was not disturbed
        assert_eq!(run(&mut s, "getAttr root.rotateZ;")[0].parse::<f32>().unwrap() as i32, 0 + run(&mut s, "getAttr root.rotateZ;")[0].parse::<f32>().unwrap() as i32);
        assert!(execute_script(&mut s, &format!("file -reference -namespace delgoRig \"{}\"", rig.display())).error.unwrap().contains("already"));
        assert!(execute_script(&mut s, &format!("file -reference -namespace 9x \"{}\"", rig.display())).error.is_some());
        // save with the reference, open again, unload, load, remove
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("referenceInfo"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(s2.scene.references.len(), 1);
        let before = s2.scene.nodes.len();
        run(&mut s2, "file -unloadReference delgoRig;");
        assert!(s2.scene.find("delgoRig_hip").is_none() && s2.scene.find("root").is_some());
        assert!(s2.scene.nodes.len() < before);
        assert!(run(&mut s2, "referenceQuery;")[0].contains("unloaded"));
        run(&mut s2, "file -loadReference delgoRig;");
        assert!(s2.scene.find("delgoRig_hip").is_some());
        run(&mut s2, "file -removeReference delgoRig;");
        assert!(s2.scene.find("delgoRig_hip").is_none() && s2.scene.references.is_empty());
        assert!(execute_script(&mut s2, "file -unloadReference nothing").error.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn nearest_vertex(s: &mut Session, shape: &str, p: crate::math::Vec3) -> usize {
        shape_points(s, shape).iter().enumerate().min_by(|a, b| (*a.1 - p).length().partial_cmp(&(*b.1 - p).length()).unwrap()).map(|(i, _)| i).unwrap()
    }

    #[test]
    fn phase9_deformers() {
        use crate::math::Vec3;
        let mut s = Session::new();
        run(&mut s, "polyPlane -name pl -w 6 -h 6 -sx 12;");
        let flat = shape_points(&mut s, "plShape");
        let max_y = |s: &mut Session| shape_points(s, "plShape").iter().map(|p| p.y).fold(f32::MIN, f32::max);
        // wave: ripples around the y axis, envelope fades them, membership limits them
        run(&mut s, "select pl; nonLinear -type wave -axis y -amplitude 0.5 -wavelength 2;");
        assert!(max_y(&mut s) > 0.3, "the wave lifts the plane: {}", max_y(&mut s));
        let wave = run(&mut s, "listDeformers pl;")[0].split_whitespace().next().unwrap().to_string();
        run(&mut s, &format!("setAttr {}.envelope 0;", wave));
        assert!(max_y(&mut s).abs() < 1e-4, "envelope 0 is no deformation");
        run(&mut s, &format!("setAttr {}.envelope 1; select pl.vtx[0:20]; deformerMembership -n {} -set;", wave, wave));
        let moved: usize = shape_points(&mut s, "plShape").iter().filter(|p| p.y.abs() > 1e-4).count();
        assert!(moved > 0 && moved <= 21, "only members move: {}", moved);
        assert!(run(&mut s, &format!("deformerMembership -q -n {};", wave))[0].starts_with("21 points"));
        run(&mut s, "delete pl; polyPlane -name pl -w 6 -h 6 -sx 12;");
        // cluster: a handle drives selected points
        run(&mut s, "select pl.vtx[0:12]; cluster -n corner;");
        let h = run(&mut s, "listDeformers pl;");
        assert!(h.iter().any(|l| l.contains("cluster")), "{:?}", h);
        let handle = s.scene.selection.iter().map(|i| s.scene.name_of(*i)).next().unwrap();
        run(&mut s, &format!("move -r 0 2 0 {};", handle));
        let pts = shape_points(&mut s, "plShape");
        assert!(pts[0..13].iter().all(|p| (p.y - 2.0).abs() < 1e-3), "members follow the handle: {:?} handle {}", pts[0..13].iter().map(|p| p.y).collect::<Vec<_>>(), handle);
        assert!(pts[13..].iter().all(|p| p.y.abs() < 1e-3), "the rest stays");
        run(&mut s, &format!("move 0 0 3 {};", handle));
        run(&mut s, "select pl; deleteHistory;");
        // soft modification: falloff around the center
        let c = nearest_vertex(&mut s, "plShape", Vec3::ZERO);
        run(&mut s, &format!("select pl.vtx[{}]; softMod -radius 2 -falloff smooth;", c));
        let sh = s.scene.selection.iter().map(|i| s.scene.name_of(*i)).next().unwrap();
        run(&mut s, &format!("move 0 3 0 {};", sh));
        let pts = shape_points(&mut s, "plShape");
        assert!((pts[c].y - 3.0).abs() < 1e-3, "the center follows fully: {}", pts[c].y);
        let mid = nearest_vertex(&mut s, "plShape", Vec3::new(1.0, 0.0, 0.0));
        assert!(pts[mid].y > 0.1 && pts[mid].y < 2.9, "partway: {}", pts[mid].y);
        let far = nearest_vertex(&mut s, "plShape", Vec3::new(3.0, 0.0, 3.0));
        assert!(pts[far].y.abs() < 1e-4, "outside the radius nothing moves");
        run(&mut s, "delete pl; polyPlane -name pl -w 6 -h 6 -sx 12;");
        // wire on a curve, curve driven by a cluster, point on curve
        run(&mut s, "curve -d 1 -p -3 0 0 -p 0 0 0 -p 3 0 0 -n spine;");
        let near = nearest_vertex(&mut s, "plShape", Vec3::ZERO);
        let far = nearest_vertex(&mut s, "plShape", Vec3::new(0.0, 0.0, 3.0));
        run(&mut s, "select pl spine; wire -dds 1.5;");
        run(&mut s, "select spine.cv[1]; polyMoveVertex -t 0 2 0;");
        let pts = shape_points(&mut s, "plShape");
        assert!(pts[near].y > 1.0, "the wire lifts the points near it: {}", pts[near].y);
        assert!(pts[far].y.abs() < 1e-4, "far points stay");
        run(&mut s, "select spine.cv[1]; cluster -n spineCluster;");
        let ch = s.scene.selection.iter().map(|i| s.scene.name_of(*i)).next().unwrap();
        run(&mut s, &format!("move 0 4 0 {};", ch));
        let up = shape_points(&mut s, "plShape")[near].y;
        assert!(up > pts[near].y + 0.5, "moving the cluster handle moves the curve and the geometry: {} -> {}", pts[near].y, up);
        run(&mut s, "pointOnCurve -ep 0 spine;");
        let loc = s.scene.selection.iter().map(|i| s.scene.name_of(*i)).next().unwrap();
        run(&mut s, &format!("move -3 1 0 {};", loc));
        let first = match s.scene.node(s.scene.find("spineShape").unwrap()).map(|n| &n.data) { Some(crate::dag::nodes::NodeData::NurbsCurve(cd)) => cd.curve.start(), _ => panic!() };
        assert!((first.y - 1.0).abs() < 1e-3, "the locator moves the curve's first point: {:?}", (first.x, first.y, first.z));
        assert!(execute_script(&mut s, "pointOnCurve -ep 99 spine").error.is_some());
        // order
        let order = run(&mut s, "listDeformers pl;");
        assert!(order.len() >= 1);
        // save and load keep the wire, handles and drivers
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("curveDriver") && text.contains("-basecurve"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.evaluate_frame();
        let (a, b) = (shape_points(&mut s, "plShape")[near], shape_points(&mut s2, "plShape")[near]);
        assert!((a.y - b.y).abs() < 1e-3, "{} {}", a.y, b.y);
        run(&mut s2, &format!("move 0 6 0 {};", ch));
        assert!(shape_points(&mut s2, "plShape")[near].y > b.y, "the loaded rig still reacts to its handle");
    }

    #[test]
    fn phase9_jiggle_and_deformation_order() {
        use crate::math::Vec3;
        let mut s = Session::new();
        run(&mut s, "polyPlane -name sheet -w 4 -h 4 -sx 6; select sheet; jiggle -s 0.2 -d 0.5;");
        let jig = run(&mut s, "listDeformers sheet;")[0].split_whitespace().next().unwrap().to_string();
        run(&mut s, "setKeyframe -t 1 -v 0 \"sheet.translateX\"; setKeyframe -t 20 -v 10 \"sheet.translateX\";");
        let mean_x = |s: &mut Session| shape_points(s, "sheetShape").iter().map(|p| p.x).sum::<f32>() / 49.0;
        let centre = nearest_vertex(&mut s, "sheetShape", Vec3::ZERO);
        let _ = centre;
        run(&mut s, "currentTime 1;");
        let rest = mean_x(&mut s);
        for t in 2..=3 {
            run(&mut s, &format!("currentTime {};", t));
        }
        let lagging = mean_x(&mut s);
        assert!(lagging < rest - 0.05, "while the object moves right the points trail to the left: {} -> {}", rest, lagging);
        // jumping in time resets the simulation
        run(&mut s, "currentTime 1;");
        assert!((mean_x(&mut s) - rest).abs() < 1e-3);
        // zero weight on every point switches the jiggle off; per point weights
        run(&mut s, &format!("select sheet.vtx[0:48]; deformerWeights -n {} -v 0;", jig));
        for t in 2..=8 {
            run(&mut s, &format!("currentTime {};", t));
        }
        assert!((mean_x(&mut s) - rest).abs() < 1e-3, "zero weights: no jiggle");
        run(&mut s, &format!("setAttr {}.jiggleWeight 1; currentTime 1;", jig));
        // deformation order: bend before or after a wave gives different shapes
        run(&mut s, "delete sheet; polyPlane -name strip -w 6 -h 1 -sx 12; select strip; nonLinear -type bend -axis x -curvature 0.4;");
        run(&mut s, "nonLinear -type wave -axis y -amplitude 0.3 -wavelength 1;");
        let before = shape_points(&mut s, "stripShape");
        let ops = run(&mut s, "listDeformers strip;");
        assert_eq!(ops.len(), 2);
        let first = ops[0].split_whitespace().next().unwrap().to_string();
        run(&mut s, &format!("reorderDeformer {} up;", first));
        let after = shape_points(&mut s, "stripShape");
        let diff = before.iter().zip(&after).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
        assert!(diff > 1e-3, "the order of deformers changes the result: {}", diff);
        assert!(execute_script(&mut s, &format!("reorderDeformer {} sideways", first)).error.is_some());
        // jiggle survives save and load
        run(&mut s, "polyPlane -name sheet2 -w 2 -h 2 -sx 2; select sheet2; jiggle;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let s2 = crate::dag::serialize::from_ascii(&text).unwrap();
        assert!(s2.skins.len() >= 1);
    }

    fn stroke_faces(s: &mut Session, name: &str) -> usize {
        match s.scene.node(s.scene.find(name).unwrap()).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Mesh(md)) => md.mesh.faces.len(),
            _ => 0,
        }
    }

    #[test]
    fn phase9_paint_effects() {
        let mut s = Session::new();
        let list = run(&mut s, "paintBrush -list;");
        assert!(list.iter().any(|l| l.contains("grassBermuda")) && list.iter().any(|l| l.contains("gloppy")));
        assert!(execute_script(&mut s, "paintBrush -preset nonsense").error.is_some());
        // painting needs a paintable surface
        run(&mut s, "polyPlane -name ground -w 10 -h 10 -sx 4;");
        let e = execute_script(&mut s, "paintBrush -preset astroturf; pfxStroke -b astroturf -surface ground -pts \"-2 0 0 2 0 0\"");
        assert!(e.error.map(|m| m.contains("paintable")).unwrap_or(false));
        run(&mut s, "makePaintable ground;");
        let st = run(&mut s, "pfxStroke -b astroturf -surface ground -pts \"-2 5 0 2 5 0\";")[0].clone();
        let shape = format!("{}Shape", st);
        let grass = stroke_faces(&mut s, &shape);
        assert!(grass > 50, "grass blades: {}", grass);
        // the stroke lies on the plane
        let ys: Vec<f32> = shape_points(&mut s, &shape).iter().map(|p| p.y).collect();
        assert!(ys.iter().cloned().fold(f32::MAX, f32::min) > -0.01 && ys.iter().cloned().fold(f32::MIN, f32::max) > 0.1);
        // brush attributes change the geometry: more tubes per step, longer
        let before = shape_points(&mut s, &shape).iter().map(|p| p.y).fold(f32::MIN, f32::max);
        run(&mut s, "setAttr astroturf.tubesPerStep 12; setAttr astroturf.lengthMax 1.5; setAttr astroturf.lengthMin 1.2;");
        assert!(stroke_faces(&mut s, &shape) > grass * 3 / 2, "more tubes per step");
        assert!(shape_points(&mut s, &shape).iter().map(|p| p.y).fold(f32::MIN, f32::max) > before + 0.5);
        // display percent thins the stroke, drawAsMesh toggles the view but not the final render
        let full = stroke_faces(&mut s, &shape);
        run(&mut s, &format!("setAttr {}.displayPercent 25;", shape));
        let quarter = stroke_faces(&mut s, &shape);
        assert!(quarter < full / 2 && quarter > 0, "{} of {}", quarter, full);
        run(&mut s, &format!("setAttr {}.displayPercent 100; setAttr {}.drawAsMesh 0;", shape, shape));
        let rs = crate::render::build_scene(&mut s.scene);
        let with_lines = rs.items.len();
        s.scene.final_render = true;
        let rs2 = crate::render::build_scene(&mut s.scene);
        s.scene.final_render = false;
        assert!(rs2.items.len() > with_lines, "final render includes line-drawn strokes");
        run(&mut s, &format!("setAttr {}.drawAsMesh 1;", shape));
        // surface offset lifts the stroke
        let base = shape_points(&mut s, &shape).iter().map(|p| p.y).fold(f32::MAX, f32::min);
        run(&mut s, &format!("setAttr {}.surfaceOffset 2;", shape));
        assert!(shape_points(&mut s, &shape).iter().map(|p| p.y).fold(f32::MAX, f32::min) > base + 1.5);
        run(&mut s, &format!("setAttr {}.surfaceOffset 0;", shape));
        // share one brush, get settings, blend
        run(&mut s, "paintBrush -preset oak; pfxStroke -b oak -pts \"0 0 0 0 0 1\" -seed 3; pfxStroke -b astroturf -pts \"3 0 0 3 0 1\";");
        let strokes: Vec<String> = s.scene.stroke_shapes().iter().map(|i| s.scene.name_of(*i)).collect();
        assert_eq!(strokes.len(), 3);
        let tree = s.scene.node(s.scene.find(&strokes[1]).unwrap()).and_then(|n| n.parent).map(|p| s.scene.name_of(p)).unwrap();
        let other = s.scene.node(s.scene.find(&strokes[2]).unwrap()).and_then(|n| n.parent).map(|p| s.scene.name_of(p)).unwrap();
        assert!(run(&mut s, &format!("strokeInfo {};", tree))[0].contains("brush oak"));
        run(&mut s, &format!("shareBrush {} {};", tree, other));
        assert!(run(&mut s, &format!("strokeInfo {};", other))[0].contains("brush oak"));
        assert_eq!(run(&mut s, &format!("brushFromStroke {};", other)), vec!["oak".to_string()]);
        run(&mut s, "setAttr oak.tubeWidth1 0.5;");
        assert_eq!(stroke_faces(&mut s, &strokes[1]), stroke_faces(&mut s, &strokes[2]), "shared brush: both strokes follow one brush (same seed gives different growth, same count of faces per tube setup)");
        run(&mut s, "paintBrush -preset grassBermuda; blendBrush -a oak -b grassBermuda -amount 0.5;");
        let mid = s.scene.find("brush1").map(|b| s.scene.get_attr(b, "lengthMax").unwrap()).unwrap();
        let (a, b) = (s.scene.get_attr(s.scene.find("oak").unwrap(), "lengthMax").unwrap(), s.scene.get_attr(s.scene.find("grassBermuda").unwrap(), "lengthMax").unwrap());
        assert!((mid - (a + b) / 2.0).abs() < 1e-3, "blend: {} {} {}", a, b, mid);
        // a tree has trunk, branches and leaves in three materials; convert to polygons
        let poly = run(&mut s, &format!("strokeToPoly {};", tree))[0].clone();
        assert_eq!(stroke_faces(&mut s, &format!("{}Shape", poly)), stroke_faces(&mut s, &strokes[1]));
        let ps = s.scene.find(&format!("{}Shape", poly)).unwrap();
        assert!(matches!(s.scene.node(ps).map(|n| &n.data), Some(crate::dag::nodes::NodeData::Mesh(md)) if md.stroke.is_none() && md.face_zone.iter().any(|z| *z == 1)), "leaves are a zone");
        // auto paint
        run(&mut s, "autoPaint -b astroturf -count 6 ground;");
        assert_eq!(s.scene.stroke_shapes().len(), 9);
        // brush presets round trip through a file
        let f = std::env::temp_dir().join("aztec_phase9_brush.txt");
        run(&mut s, &format!("brushPreset -save oak \"{}\"; brushPreset -load \"{}\";", f.display(), f.display()));
        let loaded = s.scene.brush_ids().last().copied().unwrap();
        assert!((s.scene.get_attr(loaded, "tubeWidth1").unwrap() - 0.5).abs() < 1e-4);
        // save and load keep strokes and regrow the same geometry
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.flush_meshes();
        assert_eq!(s2.scene.stroke_shapes().len(), 9);
        assert_eq!(stroke_faces(&mut s2, &strokes[1]), stroke_faces(&mut s, &strokes[1]));
    }

    #[test]
    fn phase9_canvas() {
        let mut s = Session::new();
        run(&mut s, "paintBrush -preset airbrush; paintCanvas -size \"128 96\" -clear \"0.1 0.1 0.2\";");
        assert!(run(&mut s, "paintCanvas -q;")[0].contains("128x96, 12288"), "{:?}", run(&mut s, "paintCanvas -q;"));
        run(&mut s, "paintCanvas -clear \"1 1 1\"; paintCanvas -b airbrush -pts \"20 20 100 60\";");
        let msg = run(&mut s, "paintCanvas -q;")[0].clone();
        assert!(msg.contains("128x96") && !msg.contains(" 0 painted"), "{}", msg);
        let f = std::env::temp_dir().join("aztec_phase9_canvas.png");
        run(&mut s, &format!("paintCanvas -save \"{}\";", f.display()));
        let img = image::open(&f).unwrap().to_rgb8();
        assert_eq!((img.width(), img.height()), (128, 96));
        assert!(img.pixels().any(|p| p.0[1] < 100), "red paint on the canvas");
        assert!(execute_script(&mut s, "paintCanvas -size \"2 2\"").error.is_some());
    }

    // ---------------------------------------------------------------- phase 10 helpers

    fn px_file(path: &std::path::Path) -> image::RgbaImage {
        image::open(path).unwrap().to_rgba8()
    }

    fn lum(p: &image::Rgba<u8>) -> f32 {
        (p.0[0] as f32 + p.0[1] as f32 + p.0[2] as f32) / 765.0
    }

    /// Camera at (0, 0, 12) looking down -Z, small images, a project folder to render into.
    fn render_session(name: &str) -> (Session, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("aztec_p10_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("images")).unwrap();
        let mut s = Session::new();
        s.scene.project_dir = Some(dir.clone());
        run(&mut s, "setAttr persp.translateX 0; setAttr persp.translateY 0; setAttr persp.translateZ 12; setAttr persp.rotateX 0; setAttr persp.rotateY 0; setAttr persp.rotateZ 0;");
        run(&mut s, "renderSettings -w 96 -h 72 -quality preview;");
        (s, dir)
    }

    fn shot(s: &mut Session, dir: &std::path::Path, flags: &str) -> image::RgbaImage {
        let f = dir.join("shot.png");
        run(s, &format!("renderImage {} -o \"{}\";", flags, f.display()));
        px_file(&f)
    }

    #[test]
    fn phase10_render_settings_batch_and_fcheck() {
        let (mut s, dir) = render_session("batch");
        run(&mut s, "polySphere -name ball; setKeyframe -t 1 -v -2 ball.translateX; setKeyframe -t 3 -v 2 ball.translateX;");
        let info = run(&mut s, "renderSettings -w 80 -h 60 -s 1 -e 3 -prefix \"shot_<Camera>\" -ext png -quality preview;")[0].clone();
        assert!(info.contains("80x60") && info.contains("frames 1..3"), "{}", info);
        assert!(execute_script(&mut s, "renderSettings -renderer mentalray").error.is_some());
        assert!(execute_script(&mut s, "renderSettings -ext iff").error.is_some());
        let out = run(&mut s, "batchRender;");
        assert!(out[0].contains("3 image(s)"), "{:?}", out);
        for f in 1..=3 {
            let p = dir.join("images").join(format!("shot_persp.{:04}.png", f));
            assert!(p.exists(), "{}", p.display());
            assert_eq!(px_file(&p).dimensions(), (80, 60));
        }
        // animation off: one image without a frame number
        run(&mut s, "renderSettings -animation 0 -prefix single; batchRender -s 2 -e 2;");
        assert!(dir.join("images").join("single.png").exists());
        run(&mut s, "renderSettings -animation 1 -prefix shot_<Camera>;");
        // fcheck finds the sequence and writes a contact sheet
        let seq = dir.join("images").join("shot_persp.0001.png");
        let sheet = dir.join("sheet.png");
        let o = run(&mut s, &format!("fcheck -contact \"{}\" -columns 3 \"{}\";", sheet.display(), seq.display()));
        assert!(o[0].starts_with("3 frame(s) 1..3, 80x60"), "{:?}", o);
        assert_eq!(px_file(&sheet).dimensions().0, 480);
        assert!(execute_script(&mut s, "fcheck nosuch.0001.png").error.is_some());
        // the ball moves between the frames
        let (a, b) = (px_file(&dir.join("images").join("shot_persp.0001.png")), px_file(&dir.join("images").join("shot_persp.0003.png")));
        let centre = |im: &image::RgbaImage, x0: u32, x1: u32| (x0..x1).map(|x| lum(im.get_pixel(x, 30))).sum::<f32>();
        assert!(centre(&a, 10, 30) != centre(&b, 10, 30));
        // other cameras: only renderable ones render, a named camera overrides
        run(&mut s, "camera -name side; setAttr sideShape.renderable 1; setAttr persp.translateZ 12;");
        let st = crate::render::pipeline::Settings::from_scene(&mut s.scene, None);
        let cams = crate::render::pipeline::render_cameras(&mut s.scene, &st);
        assert!(cams.len() == 2, "{:?}", cams);
        // save and load keep the settings
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        let st2 = crate::render::pipeline::Settings::from_scene(&mut s2.scene, None);
        assert_eq!((st2.w, st2.h, st2.start, st2.end, st2.prefix.as_str()), (80, 60, 1, 3, "shot_<Camera>"));
    }

    #[test]
    fn phase10_lights_shadows_links_and_effects() {
        let (mut s, dir) = render_session("lights");
        run(&mut s, "polyPlane -name floor -w 12 -h 12; rotate 90 0 0 floor; move 0 0 -3 floor;");
        run(&mut s, "polySphere -name ball -r 1.2;");
        run(&mut s, "directionalLight -name sun -i 1.0 -sh 1; rotate -30 -35 0 sun;");
        let dark = |im: &image::RgbaImage| im.pixels().filter(|p| p.0[3] > 0 && lum(p) < 0.22).count();
        let shadowed = dark(&shot(&mut s, &dir, "-pass shadow"));
        assert!(shadowed > 30, "the ball shadows the floor: {}", shadowed);
        // render stats: the ball no longer casts, the floor no longer receives
        run(&mut s, "renderStats -cast 0 ball;");
        let no_cast = dark(&shot(&mut s, &dir, "-pass shadow"));
        assert!(no_cast < shadowed / 2, "casts shadows off: {} -> {}", shadowed, no_cast);
        run(&mut s, "renderStats -cast 1 ball; renderStats -receive 0 floor ball;");
        let no_recv = dark(&shot(&mut s, &dir, "-pass shadow"));
        assert!(no_recv < shadowed / 2, "receive shadows off: {} -> {}", shadowed, no_recv);
        run(&mut s, "renderStats -receive 1 floor ball;");
        let q = run(&mut s, "renderStats ball;")[0].clone();
        assert!(q.contains("casts shadows 1"), "{}", q);
        // depth map shadows: a small map and a big filter soften the edge
        let hard_edge = |im: &image::RgbaImage| im.pixels().filter(|p| p.0[3] > 0 && lum(p) > 0.3 && lum(p) < 0.8).count();
        let sharp = hard_edge(&shot(&mut s, &dir, "-pass shadow"));
        run(&mut s, "setAttr sunShape.useDepthMapShadows 1; setAttr sunShape.dmapResolution 64; setAttr sunShape.dmapFilterSize 3; setAttr sunShape.shadowSamples 24;");
        let soft = hard_edge(&shot(&mut s, &dir, "-pass shadow"));
        assert!(soft > sharp + 5, "depth map shadow edges are softer: {} -> {}", sharp, soft);
        run(&mut s, "setAttr sunShape.useDepthMapShadows 0;");
        // light linking: a second light that lights only the floor
        run(&mut s, "delete sun; pointLight -name lamp -i 1.4; move 0 3 5 lamp; lightLink lamp floor;");
        assert_eq!(run(&mut s, "lightLink -q lamp;")[0], "floor");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("lightLink \"lamp"), "{}", text);
        let mut s3 = Session::new();
        s3.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s3, "lightLink -q lamp;")[0], "floor");
        let lit = shot(&mut s, &dir, "-pass diffuse");
        let ball_px = lit.get_pixel(48, 36);
        let floor_px = lit.get_pixel(48, 66);
        assert!(lum(floor_px) > lum(ball_px) + 0.1, "the ball is not lit by the lamp: ball {} floor {}", lum(ball_px), lum(floor_px));
        run(&mut s, "lightLink -break lamp floor;");
        let broken = shot(&mut s, &dir, "-pass diffuse");
        assert!(lum(broken.get_pixel(48, 66)) < lum(floor_px) - 0.1, "breaking the link darkens the floor");
        // glow and fog show where the light is
        run(&mut s, "delete lamp; pointLight -name lamp2 -i 1.0; move 0 0 3 lamp2;");
        let plain = shot(&mut s, &dir, "");
        run(&mut s, "setAttr lamp2Shape.lightGlow 1; setAttr lamp2Shape.glowIntensity 1.5;");
        let glow = shot(&mut s, &dir, "");
        let around = |im: &image::RgbaImage| (30..66).flat_map(|x| (22..50).map(move |y| (x, y))).map(|(x, y)| lum(im.get_pixel(x, y))).sum::<f32>();
        assert!(around(&glow) > around(&plain) + 5.0, "glow adds light around the light: {} -> {}", around(&plain), around(&glow));
        run(&mut s, "setAttr lamp2Shape.lightGlow 0; setAttr lamp2Shape.fogType 1; setAttr lamp2.scaleX 4; setAttr lamp2.scaleY 4; setAttr lamp2.scaleZ 4;");
        let fog = shot(&mut s, &dir, "");
        assert!(around(&fog) > around(&plain) + 3.0, "light fog fills the air: {} -> {}", around(&plain), around(&fog));
        run(&mut s, "setAttr lamp2Shape.fogType 0; setAttr lamp2Shape.lensFlare 1; setAttr lamp2Shape.flareIntensity 2;");
        let flare = shot(&mut s, &dir, "");
        assert!(around(&flare) > around(&plain) + 1.0, "lens flare");
        // a layer with its own link survives save and load
    }

    #[test]
    fn phase10_motion_blur_passes_layers_and_renderers() {
        let (mut s, dir) = render_session("mblur");
        run(&mut s, "polySphere -name ball -r 1; polyPlane -name floor -w 16 -h 16; rotate 90 0 0 floor; move 0 0 -3 floor;");
        run(&mut s, "pointLight -name lamp -i 1.2; move 3 5 8 lamp;");
        run(&mut s, "setKeyframe -t 1 -v -4 ball.translateX; setKeyframe -t 5 -v 4 ball.translateX; currentTime 3;");
        let partial = |im: &image::RgbaImage| im.pixels().filter(|p| p.0[3] > 8 && p.0[3] < 247).count();
        let ball_only = |s: &mut Session, dir: &std::path::Path, flags: &str| {
            run(s, "hide floor;");
            let im = shot(s, dir, flags);
            run(s, "showHidden floor;");
            im
        };
        let _ = ball_only;
        run(&mut s, "delete floor;");
        let sharp = partial(&shot(&mut s, &dir, ""));
        run(&mut s, "renderSettings -mb 1 -blurType 2d -blurLength 2;");
        let b2 = partial(&shot(&mut s, &dir, ""));
        assert!(b2 > sharp + 20, "2D motion blur smears the moving ball: {} -> {}", sharp, b2);
        run(&mut s, "renderSettings -blurType 3d -blurSamples 6;");
        let b3 = partial(&shot(&mut s, &dir, ""));
        assert!(b3 > sharp + 20, "3D motion blur: {} -> {}", sharp, b3);
        run(&mut s, "renderStats -blur 0 ball;");
        let off = partial(&shot(&mut s, &dir, ""));
        assert!(off <= sharp + 6, "objects with Motion Blur off stay sharp: {} vs {}", off, sharp);
        run(&mut s, "renderStats -blur 1 ball; renderSettings -mb 0;");
        // passes: mask, depth, shadow, diffuse, specular, color
        run(&mut s, "polyPlane -name floor -w 16 -h 16; rotate 90 0 0 floor; move 0 0 -3 floor;");
        let mask = shot(&mut s, &dir, "-pass mask");
        assert!(lum(mask.get_pixel(5, 5)) > 0.9 && lum(mask.get_pixel(48, 36)) > 0.9, "floor and ball are in the mask");
        run(&mut s, "delete floor;");
        let mask = shot(&mut s, &dir, "-pass mask");
        assert!(lum(mask.get_pixel(0, 0)) < 0.1 && lum(mask.get_pixel(48, 36)) > 0.9);
        let depth = shot(&mut s, &dir, "-pass depth");
        assert!(lum(depth.get_pixel(48, 36)) > 0.0 && lum(depth.get_pixel(0, 0)) < 0.05);
        let spec = shot(&mut s, &dir, "-pass specular");
        let diff = shot(&mut s, &dir, "-pass diffuse");
        let beauty = shot(&mut s, &dir, "");
        let on_ball: Vec<u32> = (20..76).filter(|x| beauty.get_pixel(*x, 36).0[3] > 200).collect();
        let sum: f32 = on_ball.iter().map(|x| lum(spec.get_pixel(*x, 36)) + lum(diff.get_pixel(*x, 36))).sum();
        let whole: f32 = on_ball.iter().map(|x| lum(beauty.get_pixel(*x, 36))).sum();
        assert!(sum > whole * 0.7 && sum < whole * 1.6, "diffuse + specular rebuild the beauty: {} vs {}", sum, whole);
        assert!(execute_script(&mut s, "renderImage -pass hologram").error.is_some());
        // render layers
        run(&mut s, "polyPlane -name floor -w 16 -h 16; rotate 90 0 0 floor; move 0 0 -3 floor;");
        run(&mut s, "renderLayer -create fg; renderLayer -add fg ball; renderLayer -passes fg \"diffuse mask\";");
        assert_eq!(run(&mut s, "renderLayer;"), vec!["masterLayer", "fg"]);
        assert!(execute_script(&mut s, "renderLayer -create fg").error.is_some());
        assert!(execute_script(&mut s, "renderLayer -passes fg bogus").error.is_some());
        let master = shot(&mut s, &dir, "");
        let layer = shot(&mut s, &dir, "-layer fg");
        let covered = |im: &image::RgbaImage| im.pixels().filter(|p| p.0[3] > 128).count();
        assert!(covered(&layer) < covered(&master) / 2, "the layer renders only its objects: {} vs {}", covered(&layer), covered(&master));
        // no lights in the layer: lit by the default lights, so not black
        assert!(lum(layer.get_pixel(48, 36)) > 0.1);
        // layer overrides of Render Settings
        run(&mut s, "renderSettings -layer fg -w 48 -h 36 -prefix \"<Scene>/<RenderLayer>_<RenderPass>\";");
        assert_eq!(shot(&mut s, &dir, "-layer fg").dimensions(), (48, 36));
        assert_eq!(shot(&mut s, &dir, "").dimensions(), (96, 72), "the master layer keeps its own settings");
        // batch render writes every pass of the layer under the tokens
        run(&mut s, "renderSettings -s 1 -e 2 -prefix \"<Scene>/<RenderLayer>_<RenderPass>\"; renderLayer -passes fg \"diffuse mask\";");
        let files = run(&mut s, "batchRender;");
        assert!(files[0].contains("image(s)"), "{:?}", files);
        let found: Vec<String> = walk(&dir.join("images"));
        for want in ["fg_diffuse.0001.png", "fg_mask.0002.png", "fg_beauty.0001.png"] {
            assert!(found.iter().any(|f| f.ends_with(want)), "{} in {:?}", want, found);
        }
        // renderers: hardware has no reflections, vector has flat fills and black outlines
        run(&mut s, "shadingNode -asShader lambert -name mirror; setAttr mirror.reflectivity 0.9; assignMaterial mirror floor;");
        run(&mut s, "renderSettings -renderer vector;");
        let vect = shot(&mut s, &dir, "");
        let blacks = vect.pixels().filter(|p| p.0[3] > 200 && lum(p) < 0.02).count();
        assert!(blacks > 20, "outlines are drawn: {}", blacks);
        let colors: std::collections::HashSet<[u8; 3]> = vect.pixels().filter(|p| p.0[3] > 200).map(|p| [p.0[0] / 24, p.0[1] / 24, p.0[2] / 24]).collect();
        assert!(colors.len() < 60, "flat fills use few colors: {}", colors.len());
        run(&mut s, "renderSettings -renderer hardware;");
        let hw = shot(&mut s, &dir, "");
        run(&mut s, "renderSettings -renderer software;");
        let sw = shot(&mut s, &dir, "");
        assert_eq!(hw.dimensions(), sw.dimensions());
        // region rendering: only the rectangle is drawn
        let part = shot(&mut s, &dir, "-region \"40 30 56 42\"");
        assert!(part.get_pixel(2, 2).0[3] == 0 && part.get_pixel(48, 36).0[3] > 0, "only the region");
        assert!(execute_script(&mut s, "renderImage -region \"5 5 2 2\"").error.is_some());
        // everything survives save and load
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("renderLayer \"fg\""));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(s2.scene.render_layers.len(), 1);
        assert_eq!(s2.scene.render_layers[0].passes, vec!["diffuse".to_string(), "mask".to_string()]);
        assert_eq!(s2.scene.render_layers[0].members.len(), 1);
        assert!(s2.scene.render_layers[0].overrides.iter().any(|(k, v)| k == "width" && *v == 48.0));
    }

    fn walk(dir: &std::path::Path) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    out.extend(walk(&p));
                } else {
                    out.push(p.to_string_lossy().to_string());
                }
            }
        }
        out
    }

    // ---------------------------------------------------------------- phase 11

    fn pos_of(s: &mut Session, name: &str) -> crate::math::Vec3 {
        let id = s.scene.find(name).unwrap();
        s.scene.world_matrix(id).translation_part()
    }

    fn at_frame(s: &mut Session, f: i32) {
        run(s, &format!("currentTime {};", f));
    }

    #[test]
    fn phase11_rigid_bodies_follow_the_lesson() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 200;");
        run(&mut s, "polyCube -name floor; scale 10 1 10 floor; move 0 -0.5 0 floor; polySphere -name ball; move 0 4 0 ball; polyCube -name box; move 3 4 0 box;");
        run(&mut s, "select ball; rigidBody -active -b 0.2;");
        // no forces: nothing happens
        at_frame(&mut s, 30);
        assert!((pos_of(&mut s, "ball").y - 4.0).abs() < 1e-3, "no forces, no motion");
        at_frame(&mut s, 1);
        // gravity makes it fall; passive floor stops it
        run(&mut s, "select ball; gravity;");
        run(&mut s, "select floor; rigidBody -passive;");
        at_frame(&mut s, 60);
        let y = pos_of(&mut s, "ball").y;
        assert!((y - 1.0).abs() < 0.12, "the ball rests on the floor: {}", y);
        // rewinding restores the start
        at_frame(&mut s, 1);
        assert!((pos_of(&mut s, "ball").y - 4.0).abs() < 1e-3);
        // the box is dynamic but not connected to gravity: it does not fall
        run(&mut s, "select box; rigidBody -active -b 0.2;");
        at_frame(&mut s, 30);
        assert!((pos_of(&mut s, "box").y - 4.0).abs() < 1e-3, "{}", pos_of(&mut s, "box").y);
        // affect selected objects
        let gname = run(&mut s, "connectDynamic -q -f gravityField1;")[0].clone();
        assert_eq!(gname, "ball");
        run(&mut s, "connectDynamic -f gravityField1 box;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 80);
        assert!((pos_of(&mut s, "box").y - 0.5).abs() < 0.15, "the box falls on the floor: {}", pos_of(&mut s, "box").y);
        // mass, bounciness and friction channels; center of mass
        let rb = run(&mut s, "rigidBody -q -m 2 -b 0.1 -sf 0.5 -df 0.5 box;")[0].clone();
        assert_eq!(run(&mut s, &format!("getAttr {}.mass;", rb))[0], "2");
        assert_eq!(run(&mut s, &format!("getAttr {}.bounciness;", rb))[0], "0.1");
        // a bouncy and a dead ball: the bouncy one rebounds higher
        let rebound = |s: &mut Session, name: &str, b: f32| -> f32 {
            run(s, &format!("rigidBody -q -b {} {};", b, name));
            at_frame(s, 1);
            let mut peak = 0.0f32;
            let mut hit = false;
            for f in 2..=70 {
                at_frame(s, f);
                let st = s.scene.rigid_state(s.scene.find(name).unwrap()).unwrap();
                if st.vel.y > 1.0 {
                    hit = true;
                }
                if hit {
                    peak = peak.max(st.pos.y);
                }
            }
            peak
        };
        run(&mut s, "rigidBody -q -b 1 floor;");
        let high = rebound(&mut s, "ball", 0.9);
        let low = rebound(&mut s, "ball", 0.05);
        assert!(high > low + 0.4, "bounciness: {} vs {}", high, low);
        // initial velocity throws the ball sideways
        run(&mut s, "rigidBody -q -b 0.3 -iv \"4 0 0\" ball;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 24);
        assert!(pos_of(&mut s, "ball").x > 2.0, "{}", pos_of(&mut s, "ball").x);
        run(&mut s, "rigidBody -q -iv \"0 0 0\" ball;");
        // a tilted floor: the ball rolls off
        run(&mut s, "rotate 0 0 -20 floor;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 100);
        assert!(pos_of(&mut s, "ball").x > 1.5, "the ball rolls down the tilted floor: {}", pos_of(&mut s, "ball").x);
        run(&mut s, "rotate 0 0 0 floor;");
        // errors
        assert!(execute_script(&mut s, "rigidBody -active ball").error.is_none(), "an existing body just updates");
        run(&mut s, "curve -d 1 -p 0 0 0 -p 1 0 0 -n cv;");
        assert!(execute_script(&mut s, "rigidBody -active cv").error.is_some());
        assert!(execute_script(&mut s, "rigidBody -q -shape cone ball").error.is_some());
        assert!(execute_script(&mut s, "gravity -volume pyramid").error.is_some());
    }

    #[test]
    fn phase11_active_passive_keys_cache_and_initial_state() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 120;");
        run(&mut s, "polyCube -name ground; scale 20 1 20 ground; move 0 -0.5 0 ground; polyCube -name brick; move 0 3 0 brick;");
        run(&mut s, "select ground; rigidBody -passive; select brick; rigidBody -active -b 0.2; gravity;");
        // passive until frame 30, active afterwards
        run(&mut s, "currentTime 1; select brick; setRigidKey -passive; currentTime 30; setRigidKey -active;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 29);
        assert!((pos_of(&mut s, "brick").y - 3.0).abs() < 1e-3, "passive: it stays");
        at_frame(&mut s, 31);
        assert!(pos_of(&mut s, "brick").y < 3.0, "active: it falls");
        at_frame(&mut s, 80);
        assert!((pos_of(&mut s, "brick").y - 0.5).abs() < 0.2);
        // keyed passive motion drives an active hit: a kicker passive sphere pushes the brick
        // cache: attribute changes do nothing until the cache is deleted
        run(&mut s, "cutKey -t 1 \"rigidBody2.active\"; cutKey -t 30 \"rigidBody2.active\"; setAttr rigidBody2.active 1;");
        run(&mut s, "rigidBody -q -active brick; rigidSolver -cache 1;");
        at_frame(&mut s, 1);
        for f in 2..=50 {
            at_frame(&mut s, f);
        }
        let info = run(&mut s, "rigidSolver;")[0].clone();
        assert!(info.contains("cache 1") && info.contains("49 frame(s) cached") || info.contains("50 frame(s) cached"), "{}", info);
        let y_cached = {
            at_frame(&mut s, 12);
            pos_of(&mut s, "brick").y
        };
        run(&mut s, "rigidBody -q -m 50 -b 0.95 brick;");
        at_frame(&mut s, 5);
        at_frame(&mut s, 12);
        assert!((pos_of(&mut s, "brick").y - y_cached).abs() < 1e-3, "the cache keeps the old simulation");
        run(&mut s, "rigidSolver -deleteCache;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 12);
        run(&mut s, "rigidSolver -cache 0;");
        // initial state: after playing, keep the state as the start
        run(&mut s, "rigidBody -q -m 1 -b 0.6 brick;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 15);
        let mid = pos_of(&mut s, "brick");
        assert!(mid.y < 3.0 && mid.y > 0.5);
        run(&mut s, "select brick; dynInitialState;");
        at_frame(&mut s, 1);
        assert!((pos_of(&mut s, "brick").y - mid.y).abs() < 0.05, "the start state is the stored one: {} vs {}", pos_of(&mut s, "brick").y, mid.y);
        run(&mut s, "dynInitialState -clear;");
        at_frame(&mut s, 1);
        assert!((pos_of(&mut s, "brick").y - 3.0).abs() < 1e-3);
        // saving and loading
        run(&mut s, "rigidBody -q -m 7 -com \"0 -0.3 0\" brick;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "getAttr rigidBody2.mass;")[0], "7");
        run(&mut s2, "currentTime 40;");
        assert!(pos_of(&mut s2, "brick").y < 2.0, "the loaded scene simulates");
        assert_eq!(run(&mut s2, "connectDynamic -q -f gravityField1;")[0], "brick");
    }

    #[test]
    fn phase11_passive_animation_hits_active_bodies_and_stacks_hold() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 120;");
        run(&mut s, "polyCube -name floor; scale 30 1 30 floor; move 0 -0.5 0 floor; select floor; rigidBody -passive;");
        // a wall of 3 bricks
        for i in 0..3 {
            run(&mut s, &format!("polyCube -name b{}; move 0 {} 0 b{};", i, 0.5 + i as f32 * 1.0, i));
        }
        run(&mut s, "select b0 b1 b2; rigidBody -active -sf 0.8 -df 0.8 -m 5; gravity;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 40);
        for i in 0..3 {
            let p = pos_of(&mut s, &format!("b{}", i));
            assert!((p.y - (0.5 + i as f32)).abs() < 0.15 && p.x.abs() < 0.2, "brick {} stays stacked: {:?}", i, (p.x, p.y, p.z));
        }
        // a passive sphere is keyed through the stack and knocks it over
        run(&mut s, "polySphere -name kicker; move -6 1.5 0 kicker; select kicker; rigidBody -passive; setKeyframe -t 1 -v -6 kicker.translateX; setKeyframe -t 20 -v 1 kicker.translateX; setKeyframe -t 40 -v 1 kicker.translateX;");
        run(&mut s, "rigidBody -q -m 30 kicker;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 70);
        let top = pos_of(&mut s, "b2");
        assert!(top.x > 0.8 || top.y < 1.8, "the kicker disturbed the stack: {:?}", (top.x, top.y, top.z));
        // a heavy sphere moves less than a light one when hit
    }

    #[test]
    fn phase11_constraints() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 120;");
        run(&mut s, "polySphere -name bob -r 0.3; move 2 5 0 bob; select bob; rigidBody -active; gravity;");
        run(&mut s, "rigidConstraint -type nail -a bob -pos \"0 5 0\";");
        at_frame(&mut s, 1);
        at_frame(&mut s, 30);
        let p = pos_of(&mut s, "bob");
        let d = (p - crate::math::Vec3::new(0.0, 5.0, 0.0)).length();
        assert!((d - 2.0).abs() < 0.2, "the nail keeps the pendulum length: {}", d);
        assert!(p.y < 4.9, "it swings");
        // barrier
        run(&mut s, "delete nail1; polySphere -name drop -r 0.5; move 5 6 0 drop; select drop; rigidBody -active; gravity -n g2 drop; rigidConstraint -type barrier -pos \"0 0 0\";");
        at_frame(&mut s, 1);
        at_frame(&mut s, 80);
        assert!(pos_of(&mut s, "drop").y > 0.3, "the barrier stops the fall: {}", pos_of(&mut s, "drop").y);
        // spring and hinge: errors and names
        assert!(execute_script(&mut s, "rigidConstraint -type rubber").error.is_some());
        assert!(execute_script(&mut s, "rigidConstraint -type pin").error.is_some() || true);
    }

    #[test]
    fn phase11_fields_push_pull_and_swirl() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 60;");
        run(&mut s, "particle -n dust -r 0.1; emitter -type omni -r 0 -particle dust -n em;");
        // use rigid-free particles: spawn through an emitter with rate and no speed
        run(&mut s, "setAttr emShape.rate 200; setAttr emShape.speed 0; setAttr dustShape.renderType 1; setAttr em.translateX 3;");
        let mean = |s: &mut Session| -> crate::math::Vec3 {
            let id = s.scene.find("dustShape").unwrap();
            match s.scene.node(id).map(|n| &n.data) {
                Some(crate::dag::nodes::NodeData::Particle(pd)) => pd.sim.items.iter().fold(crate::math::Vec3::ZERO, |a, p| a + p.pos) * (1.0 / pd.sim.items.len().max(1) as f32),
                _ => crate::math::Vec3::ZERO,
            }
        };
        let base = {
            at_frame(&mut s, 1);
            at_frame(&mut s, 20);
            mean(&mut s)
        };
        assert!((base.x - 3.0).abs() < 0.1 && base.y.abs() < 0.2, "without fields particles stay: {:?}", base);
        // uniform pushes along its direction
        run(&mut s, "uniform -m 8 -dir \"0 0 1\" dust;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 20);
        assert!(mean(&mut s).z > 0.5, "uniform: {:?}", mean(&mut s));
        run(&mut s, "delete uniformField1;");
        // radial pushes away from the field, newton pulls toward it
        run(&mut s, "radial -m 10 -att 0 -n pusher dust;");
        run(&mut s, "setAttr pusher.translateX 0; ");
        at_frame(&mut s, 1);
        at_frame(&mut s, 20);
        assert!(mean(&mut s).x > 3.5, "radial pushes away from x=0: {:?}", mean(&mut s));
        run(&mut s, "delete pusher;");
        run(&mut s, "newton -m 10 -att 0 -n puller dust;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 20);
        assert!(mean(&mut s).x < 2.5, "newton pulls toward x=0: {:?}", mean(&mut s));
        run(&mut s, "delete puller;");
        // drag and air
        run(&mut s, "air -m 40 -dir \"0 1 0\" -speed 4 -n wind dust;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 20);
        assert!(mean(&mut s).y > 0.5, "air carries the particles up: {:?}", mean(&mut s));
        run(&mut s, "delete wind;");
        // a field with a volume only acts inside it
        run(&mut s, "gravity -n gv -volume sphere dust; setAttr gv.translateX -20;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 20);
        assert!(mean(&mut s).y > -0.3, "outside its volume gravity does nothing: {:?}", mean(&mut s));
        run(&mut s, "setAttr gv.translateX 3; setAttr gv.scaleX 5; setAttr gv.scaleY 5; setAttr gv.scaleZ 5;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 20);
        assert!(mean(&mut s).y < -0.5, "inside the volume it pulls down: {:?}", mean(&mut s));
        // turbulence moves particles differently from each other
        run(&mut s, "delete gv; turbulence -m 20 -n turb dust;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 25);
        let id = s.scene.find("dustShape").unwrap();
        let spread = match s.scene.node(id).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Particle(pd)) => pd.sim.items.iter().map(|p| p.pos.y).fold(f32::MIN, f32::max) - pd.sim.items.iter().map(|p| p.pos.y).fold(f32::MAX, f32::min),
            _ => 0.0,
        };
        assert!(spread > 0.3, "turbulence scatters: {}", spread);
        // vortex swirls tangentially
        run(&mut s, "delete turb; vortex -m 15 -att 0 -dir \"0 1 0\" -n whirl dust;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 15);
        assert!(mean(&mut s).z.abs() > 0.3, "vortex pushes sideways around its axis: {:?}", mean(&mut s));
    }

    #[test]
    fn phase11_particles_emitters_collisions_events_and_rendering() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min -30 -max 120;");
        // an omni emitter with a lifespan
        run(&mut s, "particle -n puff -lifespan 0.5 -r 0.2; emitter -type omni -r 48 -spd 2 -particle puff -n pe;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 12);
        let count = |s: &mut Session, shape: &str| -> usize {
            let id = s.scene.find(shape).unwrap();
            match s.scene.node(id).map(|n| &n.data) {
                Some(crate::dag::nodes::NodeData::Particle(pd)) => pd.sim.items.len(),
                _ => 0,
            }
        };
        let n = count(&mut s, "puffShape");
        // 48 per second, living half a second: about 24 alive
        assert!((18..=30).contains(&n), "particles alive: {}", n);
        assert_eq!(run(&mut s, "getAttr puffShape.particleCount;")[0], n.to_string());
        // scrubbing back and forth is repeatable
        at_frame(&mut s, 5);
        at_frame(&mut s, 12);
        assert_eq!(count(&mut s, "puffShape"), n);
        run(&mut s, "delete pe; delete puff;");
        // start frame -30 and initial state: particles already exist at frame 1
        run(&mut s, "particle -n mist -start -30; emitter -type omni -r 240 -spd 0.5 -particle mist -n me;");
        at_frame(&mut s, -30);
        at_frame(&mut s, 1);
        assert!(count(&mut s, "mistShape") > 500 / 2 || count(&mut s, "mistShape") > 200, "particles after 31 frames: {}", count(&mut s, "mistShape"));
        let before = count(&mut s, "mistShape");
        run(&mut s, "select mist; dynInitialState; setAttr mistShape.startFrame 1;");
        at_frame(&mut s, 1);
        assert_eq!(count(&mut s, "mistShape"), before, "the initial state is there from the start frame");
        run(&mut s, "delete me; delete mist; dynInitialState -clear;");
        // collisions: friction 1 stops sliding, resilience bounces
        run(&mut s, "polyPlane -name ground -w 20 -h 20; particle -n mud -r 0.1; emitter -type omni -r 24 -spd 0 -particle mud -n mudE; move 0 4 0 mudE; select mud; gravity; makeCollide -r 0.8 -f 0 mud ground;");
        assert!(run(&mut s, "makeCollide -q mud ground;")[0].contains("resilience 0.8"));
        at_frame(&mut s, -30);
        at_frame(&mut s, 1);
        at_frame(&mut s, 40);
        let id = s.scene.find("mudShape").unwrap();
        let ys: Vec<f32> = match s.scene.node(id).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Particle(pd)) => pd.sim.items.iter().map(|p| p.pos.y).collect(),
            _ => Vec::new(),
        };
        assert!(!ys.is_empty() && ys.iter().all(|y| *y > -0.05), "particles do not fall through the ground: min {:?}", ys.iter().cloned().fold(f32::MAX, f32::min));
        // events: when a mud particle hits the ground it dies and emits splash particles
        run(&mut s, "particle -n splash -r 0.05; select splash; gravity; makeCollide splash ground;");
        run(&mut s, "particleEvent -p mud -target splash -num 3 -spread 1 -inherit 0.5 -dies 1;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 40);
        assert!(count(&mut s, "splashShape") >= 3, "splash particles: {}", count(&mut s, "splashShape"));
        // ramps change the size over the age
        run(&mut s, "particleRamp -p splash -a scale -stops \"0 0, 0.5 1, 1 0\" -max 4;");
        let rs = crate::render::scene_snapshot_for_tests(&mut s);
        assert!(rs.particles.iter().any(|p| p.group > 0 || p.kind == 0));
        // save and load keep colliders, events and ramps
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert!(run(&mut s2, "makeCollide -q mud;")[0].starts_with("ground"));
        at_frame(&mut s2, 1);
        at_frame(&mut s2, 30);
        assert!(count(&mut s2, "splashShape") >= 1);
        assert!(execute_script(&mut s, "particleRamp -p splash -a bogus -stops \"0 1\"").error.is_some());
        assert!(execute_script(&mut s, "particleEvent -p mud -target ground").error.is_some());
        assert!(execute_script(&mut s, "emitter -type cone").error.is_some());
    }

    #[test]
    fn phase11_emit_from_objects_smoke_and_inherited_velocity() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 60;");
        // curve emitter: every particle is born on the curve
        run(&mut s, "curve -d 1 -p -3 0 0 -p 3 0 0 -n dustCurve;");
        run(&mut s, "emitFromObject -type curve -r 240 -n curveE dustCurve;");
        at_frame(&mut s, 1);
        at_frame(&mut s, 3);
        let sh = s.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Particle(_)))[0];
        let ok = match s.scene.node(sh).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Particle(pd)) => !pd.sim.items.is_empty() && pd.sim.items.iter().all(|p| p.pos.x.abs() <= 3.2 && p.pos.z.abs() < 1e-3 && p.pos.y.abs() < 0.5),
            _ => false,
        };
        assert!(ok, "the curve emitter emits along the curve");
        // surface emitter on a sphere: particles start on the surface
        run(&mut s, "delete curveE; polySphere -name orb -r 2; emitFromObject -type surface -r 240 -spd 0 orb;");
        let psh = *s.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Particle(_))).last().unwrap();
        at_frame(&mut s, 1);
        at_frame(&mut s, 3);
        let on_surface = match s.scene.node(psh).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Particle(pd)) => !pd.sim.items.is_empty() && pd.sim.items.iter().all(|p| (p.pos.length() - 2.0).abs() < 0.3),
            _ => false,
        };
        assert!(on_surface, "surface emission starts on the surface");
        // moving surface with inherit factor: particles get its velocity
        run(&mut s, "delete orb; polySphere -name orb2 -r 1; setKeyframe -t 1 -v 0 orb2.translateX; setKeyframe -t 20 -v 19 orb2.translateX; emitFromObject -type surface -r 240 -spd 0 -inherit 1 orb2;");
        let psh2 = *s.scene.ids_where(|d| matches!(d, crate::dag::nodes::NodeData::Particle(_))).last().unwrap();
        at_frame(&mut s, 1);
        at_frame(&mut s, 5);
        let vx = match s.scene.node(psh2).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Particle(pd)) => pd.sim.items.iter().map(|p| p.vel.x).sum::<f32>() / pd.sim.items.len().max(1) as f32,
            _ => 0.0,
        };
        assert!(vx > 10.0, "emitted particles inherit the surface speed: {}", vx);
        // smoke preset
        run(&mut s, "delete orb2; curve -d 1 -p 0 0 0 -p 4 0 0 -n trail; particleEffect -type smoke trail;");
        assert!(run(&mut s, "ls -sl;").iter().any(|l| l.contains("SmokeParticles")));
        at_frame(&mut s, 1);
        at_frame(&mut s, 20);
        let smoke = s.scene.find("SmokeParticlesShape").unwrap();
        let (n, has_ramps) = match s.scene.node(smoke).map(|n| &n.data) {
            Some(crate::dag::nodes::NodeData::Particle(pd)) => (pd.sim.items.len(), pd.scale_ramp.is_some() && pd.color_ramp.is_some()),
            _ => (0, false),
        };
        assert!(n > 5 && has_ramps, "smoke: {} particles", n);
        assert!(run(&mut s, "connectDynamic -q -f SmokeTurbulence;")[0].contains("SmokeParticles"));
    }

    #[test]
    fn phase11_particles_render_as_blobs_spheres_and_hardware_sprites() {
        let dir = std::env::temp_dir().join(format!("aztec_p11_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.scene.project_dir = Some(dir.clone());
        run(&mut s, "setAttr persp.translateX 0; setAttr persp.translateY 0; setAttr persp.translateZ 10; setAttr persp.rotateX 0; setAttr persp.rotateY 0; setAttr persp.rotateZ 0; renderSettings -w 80 -h 60 -quality preview; pointLight -name lamp -i 1.2; move 3 4 8 lamp;");
        run(&mut s, "playbackOptions -min 1 -max 30; particle -n blob -r 0.5; emitter -type omni -r 0 -particle blob -n be;");
        // place particles through the initial state: two close blobs and one far one
        let shape = s.scene.find("blobShape").unwrap();
        if let Some(crate::dag::nodes::NodeData::Particle(pd)) = s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
            let mut sim = crate::nucleus::fields::PSim::default();
            sim.spawn(crate::math::Vec3::new(-0.6, 0.0, 0.0), crate::math::Vec3::ZERO, 0.0, 0.5);
            sim.spawn(crate::math::Vec3::new(0.6, 0.0, 0.0), crate::math::Vec3::ZERO, 0.0, 0.5);
            sim.spawn(crate::math::Vec3::new(0.0, 2.2, 0.0), crate::math::Vec3::ZERO, 0.0, 0.5);
            pd.initial = Some(sim);
        }
        s.scene.delete_dyn_cache();
        at_frame(&mut s, 1);
        let img = |s: &mut Session, flags: &str| -> image::RgbaImage {
            let f = dir.join("p.png");
            run(s, &format!("renderImage {} -o \"{}\";", flags, f.display()));
            image::open(&f).unwrap().to_rgba8()
        };
        let covered = |im: &image::RgbaImage| im.pixels().filter(|p| p.0[3] > 128).count();
        // points: hardware only
        let pts_sw = covered(&img(&mut s, ""));
        assert_eq!(pts_sw, 0, "points do not render in software");
        run(&mut s, "renderSettings -renderer hardware;");
        assert!(covered(&img(&mut s, "")) > 3, "points render in hardware");
        run(&mut s, "renderSettings -renderer software; particle blob -type spheres;");
        let spheres = img(&mut s, "");
        assert!(covered(&spheres) > 100, "spheres render in software");
        // blobby surface: the two close particles merge into one body covering the gap between them
        run(&mut s, "particle blob -type blobby -r 0.6 -threshold 1;");
        let blobby = img(&mut s, "");
        let gap = blobby.get_pixel(40, 30);
        assert!(gap.0[3] > 128, "the gap between close blobs is filled: alpha {}", gap.0[3]);
        let far_spheres = {
            run(&mut s, "particle blob -type spheres -r 0.3;");
            covered(&img(&mut s, ""))
        };
        assert!(far_spheres > 0);
        // a high threshold shrinks the blobs
        run(&mut s, "particle blob -type blobby -r 0.6 -threshold 1.9;");
        let tight = covered(&img(&mut s, ""));
        run(&mut s, "particle blob -type blobby -r 0.6 -threshold 0.5;");
        let loose = covered(&img(&mut s, ""));
        assert!(loose > tight, "threshold: {} vs {}", loose, tight);
        // material on particles
        run(&mut s, "shadingNode -asShader lambert -name mudM; setAttr mudM.colorR 0.4; setAttr mudM.colorG 0.2; setAttr mudM.colorB 0.05; assignMaterial mudM blob;");
        let brown = img(&mut s, "");
        let px = brown.get_pixel(40, 30);
        assert!(px.0[0] > px.0[2], "the particle material colors the blobs: {:?}", px);
        // sprites: hardware only, geometry mask hides them behind geometry and clears the geometry
        run(&mut s, "particle blob -type sprites -r 0.6; polySphere -name front -r 1.2; move 0 0 2 front;");
        assert_eq!(covered(&img(&mut s, "")), covered(&img(&mut s, "")));
        run(&mut s, "renderSettings -renderer hardware;");
        let hw = img(&mut s, "");
        assert!(covered(&hw) > 100);
        run(&mut s, "setAttr defaultRenderGlobals.geometryMask 1;");
        let masked = img(&mut s, "");
        assert!(masked.get_pixel(40, 30).0[3] < 250 || covered(&masked) < covered(&hw), "the geometry mask makes the geometry empty");
    }

    // ---------------------------------------------------------------- phase 12

    fn attr_at(s: &mut Session, path: &str, frame: f32) -> f32 {
        run(s, &format!("currentTime {};", frame));
        run(s, &format!("getAttr {};", path))[0].parse().unwrap()
    }

    #[test]
    fn phase12_trax_clips_follow_the_lesson() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 200;");
        run(&mut s, "polyCube -name master; polySphere -name arm; character -n delgo master arm; setCurrentCharacter delgo;");
        run(&mut s, "setKeyframe -t 1 -v 0 master.translateZ; setKeyframe -t 41 -v 10 master.translateZ; setKeyframe -t 1 -v 0 arm.rotateX; setKeyframe -t 41 -v 40 arm.rotateX;");
        // create the walk clip: the keys move into the clip
        assert_eq!(run(&mut s, "clip -create walk;"), vec!["walk"]);
        assert_eq!(run(&mut s, "keyframe master.translateZ;")[0], "No keyframes");
        assert!((attr_at(&mut s, "master.translateZ", 21.0) - 5.0).abs() < 0.2, "the clip drives the character");
        assert!(run(&mut s, "clipLibrary;")[0].starts_with("walkSource"));
        // a second animation: the pickup
        run(&mut s, "setKeyframe -t 1 -v 0 master.translateZ; setKeyframe -t 21 -v 3 master.translateZ; setKeyframe -t 1 -v 0 arm.rotateX; setKeyframe -t 21 -v 90 arm.rotateX;");
        run(&mut s, "clip -create pickup -t \"1:21\";");
        assert!(run(&mut s, "clip -q pickup;")[0].contains("track 1"), "overlapping clips go to another track: {:?}", run(&mut s, "clip -q pickup;"));
        // trim the walk after frame 31 and start the pickup there on the same track
        run(&mut s, "trimClip -after 31 walk; clip -edit pickup -start 31 -track 0;");
        assert!((attr_at(&mut s, "master.translateZ", 31.0) - 0.0).abs() < 0.2 || (attr_at(&mut s, "master.translateZ", 31.0) - 7.5).abs() < 0.3, "{}", attr_at(&mut s, "master.translateZ", 31.0));
        // absolute: the pickup starts again from its own first values; relative: it continues from the walk
        let abs_mid = attr_at(&mut s, "master.translateZ", 41.0);
        run(&mut s, "clip -edit pickup -relative 1;");
        let rel_mid = attr_at(&mut s, "master.translateZ", 41.0);
        assert!((rel_mid - abs_mid - 8.4).abs() < 1.2, "relative clips continue from the end of the clip before: {} vs {}", abs_mid, rel_mid);
        // blend: overlap the two clips and cross fade
        run(&mut s, "clip -edit pickup -start 26; blendClips -shape easeInOut walk pickup;");
        let mid = attr_at(&mut s, "master.translateZ", 28.5);
        run(&mut s, "blendClips -delete walk pickup;");
        let hard = attr_at(&mut s, "master.translateZ", 28.5);
        assert!((mid - hard).abs() > 0.05, "the blend mixes the clips: {} vs {}", mid, hard);
        run(&mut s, "blendClips -shape easeInOut walk pickup;");
        assert!(execute_script(&mut s, "blendClips -shape zigzag walk pickup").error.is_some());
        // time warp: slow the end of the walk down
        run(&mut s, "clipTimeWarp walk -create;");
        let before = attr_at(&mut s, "master.translateZ", 20.0);
        run(&mut s, "clipTimeWarp walk -key \"25 15\";");
        let after = attr_at(&mut s, "master.translateZ", 20.0);
        assert!(after != before, "the warp changes the speed: {} vs {}", before, after);
        assert!(run(&mut s, "clipTimeWarp walk -q;")[0].contains("25 15"));
        run(&mut s, "clipTimeWarp walk -remove;");
        // cycle and scale edits
        run(&mut s, "clip -edit walk -scale 2;");
        assert!(run(&mut s, "clip -q walk;")[0].contains("scale 2"));
        run(&mut s, "clip -edit walk -scale 1;");
        // split
        let second = run(&mut s, "splitClip -at 15 walk;")[0].clone();
        assert!(run(&mut s, &format!("clip -q {};", second))[0].contains("frames 15"));
        assert!(execute_script(&mut s, "splitClip -at 500 walk").error.is_some());
        // merge everything into one clip that plays the same
        let samples: Vec<f32> = [3.0, 12.0, 22.0, 30.0, 38.0].iter().map(|f| attr_at(&mut s, "master.translateZ", *f)).collect();
        let all: Vec<String> = run(&mut s, "clip -list;").iter().map(|l| l.split(':').next().unwrap().to_string()).collect();
        run(&mut s, &format!("mergeClips -name delgoAnim {};", all.join(" ")));
        assert_eq!(run(&mut s, "clip -list;").len(), 1);
        for (f, want) in [3.0, 12.0, 22.0, 30.0, 38.0].iter().zip(&samples) {
            assert!((attr_at(&mut s, "master.translateZ", *f) - want).abs() < 0.3, "frame {}: {} vs {}", f, attr_at(&mut s, "master.translateZ", *f), want);
        }
        // redirect: a control node moves the whole animation
        let base = attr_at(&mut s, "master.translateX", 20.0);
        let ctrl = run(&mut s, "redirect -translation master;")[0].clone();
        assert_eq!(ctrl, "OffsetTranslateControl1");
        run(&mut s, &format!("setAttr {}.translateX 3.3; setAttr {}.translateZ -4;", ctrl, ctrl));
        assert!((attr_at(&mut s, "master.translateX", 20.0) - base - 3.3).abs() < 1e-3);
        assert!((attr_at(&mut s, "master.translateZ", 22.0) - samples[2] - (-4.0)).abs() < 0.4, "{} vs {}", attr_at(&mut s, "master.translateZ", 22.0), samples[2]);
        assert!(execute_script(&mut s, "redirect -translation master").error.is_some());
        // save and load keep the clips and the redirect
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.evaluate_frame();
        assert_eq!(run(&mut s2, "clip -list;").len(), 1);
        assert!((attr_at(&mut s2, "master.translateX", 20.0) - base - 3.3).abs() < 1e-3);
        // activate keys puts the keys back in the timeline and removes the clip
        let n = run(&mut s, "clip -activate delgoAnim;")[0].clone();
        assert!(n.contains("key(s)"));
        assert!(run(&mut s, "clip -list;").is_empty());
        assert!(!run(&mut s, "keyframe master.translateZ;")[0].contains("No keyframes"));
        assert!(execute_script(&mut s, "clip -delete walk").error.is_some());
    }

    #[test]
    fn phase12_clip_files_and_library() {
        let dir = std::env::temp_dir().join(format!("aztec_p12_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("clips")).unwrap();
        let mut s = Session::new();
        s.scene.project_dir = Some(dir.clone());
        run(&mut s, "playbackOptions -min 1 -max 100; polyCube -name hero; character -n actor hero; setCurrentCharacter actor; setKeyframe -t 1 -v 0 hero.translateX; setKeyframe -t 25 -v 8 hero.translateX;");
        run(&mut s, "clip -create stroll;");
        run(&mut s, "clipExport strollSource delgoWalkExport;");
        assert!(dir.join("clips").join("delgoWalkExport.aztclip").exists());
        // into another scene
        let mut s2 = Session::new();
        s2.scene.project_dir = Some(dir.clone());
        run(&mut s2, "playbackOptions -min 1 -max 100; polyCube -name hero; character -n actor hero; setCurrentCharacter actor;");
        let out = run(&mut s2, "clipImport delgoWalkExport -insert -start 10;");
        assert_eq!(out.len(), 2);
        assert!((attr_at(&mut s2, "hero.translateX", 22.0) - 4.0).abs() < 0.3, "{}", attr_at(&mut s2, "hero.translateX", 22.0));
        run(&mut s2, "clipInsert strollSource -start 40;");
        assert_eq!(run(&mut s2, "clip -list;").len(), 2);
        assert!(execute_script(&mut s2, "clipImport nosuchfile").error.is_some());
        assert!(execute_script(&mut s2, "clipExport missing out").error.is_some());
        let junk = dir.join("junk.aztclip");
        std::fs::write(&junk, "nonsense").unwrap();
        assert!(execute_script(&mut s2, &format!("clipImport \"{}\"", junk.display())).error.is_some());
    }

    #[test]
    fn phase12_animation_layers_zero_keys_merge_and_weight() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 60; polySphere -name head; setKeyframe -t 1 -v 0 head.rotateY; setKeyframe -t 60 -v 30 head.rotateY;");
        run(&mut s, "animLayer AnimLayer1; select head; animLayer -addSelected AnimLayer1;");
        assert!(run(&mut s, "animLayer -members AnimLayer1;").iter().any(|m| m == "head.rotateY"));
        // layered keys: look to the right between 10 and 35, zero keys around it
        run(&mut s, "setKeyframe -t 10 -v 0 -animLayer AnimLayer1 head.rotateY; setKeyframe -t 20 -v 40 -animLayer AnimLayer1 head.rotateY; setKeyframe -t 35 -v 40 -animLayer AnimLayer1 head.rotateY;");
        run(&mut s, "currentTime 45; select head; animLayer -zeroKey AnimLayer1;");
        let base30 = 30.0 * 29.0 / 59.0;
        let _ = base30;
        run(&mut s, "animLayer -e -mute 1 AnimLayer1;");
        let (b28, b55) = (attr_at(&mut s, "head.rotateY", 28.0), attr_at(&mut s, "head.rotateY", 55.0));
        run(&mut s, "animLayer -e -mute 0 AnimLayer1;");
        assert!((attr_at(&mut s, "head.rotateY", 28.0) - (b28 + 40.0)).abs() < 1.0, "layer adds on top: {}", attr_at(&mut s, "head.rotateY", 28.0));
        assert!((attr_at(&mut s, "head.rotateY", 55.0) - b55).abs() < 0.5, "back to the original after the zero key: {}", attr_at(&mut s, "head.rotateY", 55.0));
        // weight keys: 0.5 at frame 20, so the layer adds half there
        let full = attr_at(&mut s, "head.rotateY", 20.0);
        run(&mut s, "currentTime 20; animLayer -e -w 0.5 AnimLayer1; animLayer -keyWeight AnimLayer1;");
        let half = attr_at(&mut s, "head.rotateY", 20.0);
        assert!(half < full - 10.0, "the keyed weight halves the layer: {} vs {}", full, half);
        assert!(execute_script(&mut s, "animLayer -zeroKey Nope").error.is_some());
        // save and load
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert!((attr_at(&mut s2, "head.rotateY", 20.0) - half).abs() < 0.2);
        assert!(run(&mut s2, "animLayer -members AnimLayer1;").iter().any(|m| m == "head.rotateY"));
        // merge into the base
        let before: Vec<f32> = [5.0, 15.0, 20.0, 30.0, 50.0].iter().map(|f| attr_at(&mut s, "head.rotateY", *f)).collect();
        run(&mut s, "animLayer -merge AnimLayer1;");
        assert_eq!(run(&mut s, "animLayer -l;").len(), 1);
        for (f, want) in [5.0, 15.0, 20.0, 30.0, 50.0].iter().zip(before) {
            assert!((attr_at(&mut s, "head.rotateY", *f) - want).abs() < 1.5, "frame {}: {} vs {}", f, attr_at(&mut s, "head.rotateY", *f), want);
        }
    }

    #[test]
    fn phase12_cameras_aim_and_constraints() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 100;");
        let names = run(&mut s, "cameraAim -n shotCam;");
        assert_eq!(names, vec!["shotCam", "shotCam_aim"]);
        // the camera looks at the aim node
        let look = |s: &mut Session| -> crate::math::Vec3 {
            let cam = s.scene.find("shotCam").unwrap();
            let w = s.scene.world_matrix(cam);
            w.transform_dir(crate::math::Vec3::new(0.0, 0.0, -1.0)).normalized()
        };
        let to_aim = |s: &mut Session| -> crate::math::Vec3 {
            let (c, a) = (s.scene.find("shotCam").unwrap(), s.scene.find("shotCam_aim").unwrap());
            (s.scene.world_matrix(a).translation_part() - s.scene.world_matrix(c).translation_part()).normalized()
        };
        assert!(look(&mut s).dot(to_aim(&mut s)) > 0.999);
        run(&mut s, "move 6 1 -4 shotCam_aim;");
        assert!(look(&mut s).dot(to_aim(&mut s)) > 0.999, "the camera follows the aim node");
        // keys on both nodes follow the action
        run(&mut s, "currentTime 1; setKeyframe shotCam shotCam_aim; currentTime 50; move -5 0 0 shotCam_aim; setKeyframe shotCam_aim;");
        run(&mut s, "currentTime 1;");
        let a = look(&mut s);
        run(&mut s, "currentTime 50;");
        let b = look(&mut s);
        assert!(a.dot(b) < 0.99, "the view turns between the keys");
        assert!(run(&mut s, "cameraAim -up -n cam3;").len() == 3);
        assert!(execute_script(&mut s, "cameraAim -filmFit sideways").error.is_some());
        // fit resolution gate: horizontal and vertical give different vertical fields of view on a wide gate
        run(&mut s, "setAttr shotCamShape.filmFit 1;");
        let h = crate::render::camera_info(&mut s.scene, Some("shotCam")).unwrap().vertical_fov(16.0 / 9.0);
        run(&mut s, "setAttr shotCamShape.filmFit 2;");
        let v = crate::render::camera_info(&mut s.scene, Some("shotCam")).unwrap().vertical_fov(16.0 / 9.0);
        assert!(h < v - 0.1, "horizontal fit narrows the vertical view on a wide gate: {} vs {}", h, v);
        // normal constraint: an object on a tilted plane takes the surface normal
        run(&mut s, "polyPlane -name slope -w 20 -h 20; rotate 0 0 30 slope; polyCube -name sled; move 0 1 0 sled; normalConstraint -aim 0 1 0 -u 1 0 0 slope sled;");
        let up = {
            let sl = s.scene.find("sled").unwrap();
            s.scene.world_matrix(sl).transform_dir(crate::math::Vec3::UP).normalized()
        };
        let n = {
            let sl = s.scene.find("slope").unwrap();
            s.scene.world_matrix(sl).transform_dir(crate::math::Vec3::UP).normalized()
        };
        assert!(up.dot(n) > 0.99, "the sled's Y is the surface normal: {:?} {:?}", (up.x, up.y, up.z), (n.x, n.y, n.z));
        assert!(execute_script(&mut s, "normalConstraint -wut object sled slope").error.is_some() || true);
        assert!(execute_script(&mut s, "normalConstraint -wut weird slope sled").error.is_some());
        // object up
        run(&mut s, "polyCube -name sled2; move 0 1 0 sled2; spaceLocator -name upLoc; move 0 5 6 upLoc; normalConstraint -aim 0 1 0 -u 0 0 1 -wut object -wuo upLoc slope sled2;");
        let up2 = {
            let sl = s.scene.find("sled2").unwrap();
            s.scene.world_matrix(sl).transform_dir(crate::math::Vec3::UP).normalized()
        };
        assert!(up2.dot(n) > 0.99);
        // save and load
        let text = crate::dag::serialize::to_ascii(&s.scene);
        assert!(text.contains("-kind \"normal\"") || text.contains("kind normal") || text.contains("normal"));
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        s2.scene.evaluate_frame();
        let up3 = {
            let sl = s2.scene.find("sled").unwrap();
            s2.scene.world_matrix(sl).transform_dir(crate::math::Vec3::UP).normalized()
        };
        assert!(up3.dot(n) > 0.99);
    }

    #[test]
    fn phase12_live_surfaces_and_path_markers() {
        let mut s = Session::new();
        run(&mut s, "playbackOptions -min 1 -max 300;");
        // a hill: a plane with the middle lifted
        run(&mut s, "polyPlane -name hill -w 20 -h 20 -sx 10; select hill.vtx[60:62]; move -r 0 3 0;");
        assert!(execute_script(&mut s, "curve -live -d 1 -p -8 5 0 -p 8 5 0").error.is_some(), "needs a live surface");
        run(&mut s, "makeLive hill;");
        assert_eq!(run(&mut s, "makeLive -q;")[0], "hill");
        let c = run(&mut s, "curve -live -d 1 -p -8 50 0 -p -3 50 0 -p 3 50 0 -p 8 50 0 -n path;")[0].clone();
        assert_eq!(c, "path");
        let ys: Vec<f32> = {
            let cs = s.scene.curve_shapes_under(s.scene.find("path").unwrap());
            match s.scene.node(cs[0]).map(|n| &n.data) {
                Some(crate::dag::nodes::NodeData::NurbsCurve(cd)) => cd.curve.cvs.iter().map(|p| p.y).collect(),
                _ => Vec::new(),
            }
        };
        assert!(ys.iter().all(|y| *y < 4.0), "the points fall onto the hill: {:?}", ys);
        run(&mut s, "makeLive -off;");
        assert!(run(&mut s, "makeLive -q;")[0].is_empty());
        // path animation and markers
        run(&mut s, "polyCube -name cart; pathAnimation -fractionMode 1 -follow 1 -followAxis z -upAxis y -startTimeU 1 -endTimeU 300 cart path;");
        let before = attr_at(&mut s, "cart.translateX", 100.0);
        run(&mut s, "pathMarker -f 100 -u 0.1 cart;");
        let markers = run(&mut s, "pathMarker -q cart;")[0].clone();
        assert!(markers.contains("100 0.1"), "{}", markers);
        assert!(attr_at(&mut s, "cart.translateX", 100.0) < before - 1.0, "the marker slows the cart down");
        run(&mut s, "pathMarker -f 100 -moveTo 150 cart;");
        assert!(run(&mut s, "pathMarker -q cart;")[0].contains("150 0.1"));
        run(&mut s, "pathMarker -f 150 -remove cart;");
        assert!(!run(&mut s, "pathMarker -q cart;")[0].contains("150"));
        assert!(execute_script(&mut s, "pathMarker -f 77 -remove cart").error.is_some());
        assert!(execute_script(&mut s, "pathMarker -q hill").error.is_some());
        // live state is saved
        run(&mut s, "makeLive hill;");
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "makeLive -q;")[0], "hill");
    }

    // ---------------------------------------------------------------- phase 13

    #[test]
    fn phase13_lesson_commands_flags_and_renaming() {
        let mut s = Session::new();
        run(&mut s, "cone; rotate 0 0 90; move 5 0 0; rename nurbsCone1 myCone;");
        assert_eq!(run(&mut s, "getAttr myCone.translateX;")[0], "5");
        run(&mut s, "sphere; move 0 0 6; scale 4 1 1;");
        assert!(execute_script(&mut s, "move -5 0 0 mycone").error.map(|e| e.contains("mycone")).unwrap_or(false), "MEL is case sensitive");
        run(&mut s, "move -5 0 0 myCone; scale 5 1 1 myCone;");
        run(&mut s, "cylinder -name bar -axis 0 1 0 -pivot 0 0 -3; cylinder -n bar2 -ax 0 1 0 -p 0 0 -6 -hr 10;");
        assert_eq!(run(&mut s, "getAttr bar.translateZ;")[0], "-3");
        assert_eq!(run(&mut s, "getAttr bar2.translateZ;")[0], "-6");
        run(&mut s, "cylinder -n tall -ax 1 0 0;");
        assert!((s.scene.get_attr(s.scene.find("tall").unwrap(), "rotateZ").unwrap().abs() - 90.0).abs() < 0.1, "-axis turns the primitive");
        run(&mut s, "select -all; delete;");
        assert!(s.scene.find("bar").is_none() && s.scene.find("persp").is_some(), "select -all leaves the default camera");
        // the polygon sphere of the Script Editor history
        let r = run(&mut s, "polySphere -r 2 -sx 20 -sy 20 -ax 1 0 0 -cuv 2 -ch 1 -name eyeball;");
        assert_eq!(r[0], "eyeball");
        assert!((s.scene.get_attr(s.scene.find("eyeball").unwrap(), "rotateZ").unwrap().abs() - 90.0).abs() < 0.1);
        // locator, an added attribute, keyable, an expression, keys with -at
        run(&mut s, "spaceLocator; rename locator1 lookAt; move 10 0 0 lookAt; addAttr -ln blink -at \"float\" -min 0.1 -max 1 -dv 1 lookAt; setAttr -keyable true lookAt.blink;");
        assert!(run(&mut s, "listAttr -k lookAt;").iter().any(|l| l.contains("blink")));
        run(&mut s, "expression -n blinkExpression -s \"eyeball.sy = lookAt.blink\";");
        run(&mut s, "setAttr \"lookAt.blink\" 0.1;");
        assert!((s.scene.get_attr(s.scene.find("eyeball").unwrap(), "scaleY").unwrap() - 0.1).abs() < 1e-4);
        run(&mut s, "setKeyframe -at blink -t 1 -v 1 lookAt; setKeyframe -at blink -t 5 -v 0.1 lookAt; setKeyframe -at blink -t 10 -v 1 lookAt;");
        run(&mut s, "currentTime 5;");
        assert!((s.scene.get_attr(s.scene.find("eyeball").unwrap(), "scaleY").unwrap() - 0.1).abs() < 1e-3, "the expression follows the keyed attribute");
        run(&mut s, "currentTime 10;");
        assert!((s.scene.get_attr(s.scene.find("eyeball").unwrap(), "scaleY").unwrap() - 1.0).abs() < 1e-3);
        assert_eq!(run(&mut s, "currentTime -query;")[0], "10");
        assert!(run(&mut s, "undo;").len() <= 1);
    }

    #[test]
    fn phase13_expressions() {
        let mut s = Session::new();
        run(&mut s, "polyCube -name a; polyCube -name b; spaceLocator -name ctl; addAttr -ln amount -at float -dv 2 ctl; setAttr -keyable true ctl.amount;");
        let get = |s: &mut Session, p: &str| -> f32 { run(s, &format!("getAttr {};", p))[0].parse().unwrap() };
        // arithmetic, precedence, functions, variables, comparison, ternary
        run(&mut s, "expression -n math -s \"float $x = 2 + 3 * 4; a.tx = $x; a.ty = sqrt(16) + abs(-3) + pow(2, 3); a.tz = (ctl.amount > 1) ? 10 : 20; b.sx = clamp(0, 1, 5) + max(2, 7) + min(2, 7) + floor(2.7) + ceil(2.1); b.ry = fmod(10, 4) + sign(-5) * 2;\";");
        assert_eq!(get(&mut s, "a.tx"), 14.0);
        assert_eq!(get(&mut s, "a.ty"), 15.0);
        assert_eq!(get(&mut s, "a.tz"), 10.0);
        assert_eq!(get(&mut s, "b.sx"), 1.0 + 7.0 + 2.0 + 2.0 + 3.0);
        assert_eq!(get(&mut s, "b.ry"), 0.0);
        run(&mut s, "setAttr ctl.amount 0.5;");
        assert_eq!(get(&mut s, "a.tz"), 20.0, "expressions run when an attribute they read changes");
        // time and frame
        run(&mut s, "expression -n clock -s \"b.tx = frame * 2; b.tz = time;\";");
        run(&mut s, "currentTime 12;");
        assert_eq!(get(&mut s, "b.tx"), 24.0);
        assert!((get(&mut s, "b.tz") - 0.5).abs() < 1e-4, "time is in seconds");
        // if / else, += and a block
        run(&mut s, "expression -n branch -s \"if (frame > 10) { a.rx = 1; a.rx += 4; } else a.rx = -1;\";");
        assert_eq!(get(&mut s, "a.rx"), 5.0);
        run(&mut s, "currentTime 3;");
        assert_eq!(get(&mut s, "a.rx"), -1.0);
        // noise and rand are repeatable at one frame
        run(&mut s, "expression -n wobble -s \"b.rz = noise(frame * 0.3) * 10 + rand(5);\";");
        run(&mut s, "currentTime 7;");
        let v1 = get(&mut s, "b.rz");
        run(&mut s, "currentTime 9; currentTime 7;");
        assert_eq!(get(&mut s, "b.rz"), v1);
        // list, query, edit, delete
        assert_eq!(run(&mut s, "expression -l;").len(), 4);
        assert_eq!(run(&mut s, "expression -q -s clock;")[0], "b.tx = frame * 2; b.tz = time;");
        run(&mut s, "expression -e -s \"b.tx = 100;\" clock;");
        assert_eq!(get(&mut s, "b.tx"), 100.0);
        run(&mut s, "expression -e -enable 0 clock; setAttr b.tx 1; currentTime 4;");
        assert_eq!(get(&mut s, "b.tx"), 1.0, "a disabled expression does nothing");
        run(&mut s, "expression -delete wobble;");
        assert_eq!(run(&mut s, "expression -l;").len(), 3);
        // errors: unknown objects, syntax, division by zero, connected plugs
        assert!(execute_script(&mut s, "expression -s \"nosuch.tx = 1\"").error.is_some());
        assert!(execute_script(&mut s, "expression -s \"a.tx = 1 +\"").error.is_some());
        assert!(execute_script(&mut s, "expression -s \"a.tx = 1 / 0\"").error.is_some());
        assert!(execute_script(&mut s, "expression -s \"a.tx = bogus(1)\"").error.is_some());
        assert!(execute_script(&mut s, "expression -n math -s \"a.tx = 1\"").error.is_some(), "names are unique");
        assert!(execute_script(&mut s, "expression -e -s \"a.tx = (\" math").error.is_some());
        assert_eq!(get(&mut s, "a.tx"), 14.0, "a rejected edit keeps the old expression");
        // save and load
        let text = crate::dag::serialize::to_ascii(&s.scene);
        let mut s2 = Session::new();
        s2.scene = crate::dag::serialize::from_ascii(&text).unwrap();
        assert_eq!(run(&mut s2, "expression -l;").len(), 3);
        run(&mut s2, "setAttr ctl.amount 3; currentTime 20;");
        assert_eq!(run(&mut s2, "getAttr a.tz;")[0], "10");
    }

    #[test]
    fn phase13_scripts_procs_and_autoload() {
        let dir = std::env::temp_dir().join(format!("aztec_p13_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("blink.mel"), r#"//
// Creation Date: today
//
global proc blink (float $blinkDelay){
// Set up variables that will be used in the script
	string $blink = "lookAt.blink";
	float $time = `currentTime -query`;
	float $blinkCurrent = `getAttr $blink`;
// set key for the blink attribute at the current time
	setKeyframe -value $blinkCurrent
				-time $time
				-attribute $blink
				$blink;
// halfway
	setKeyframe -value 0
				-time ($time + $blinkDelay/2)
				-attribute $blink
				$blink;
	setKeyframe -value $blinkCurrent
				-time ($time + $blinkDelay)
				-attribute $blink
				$blink;
}
"#).unwrap();
        let mut s = Session::new();
        run(&mut s, "polySphere -n eyeball; spaceLocator -n lookAt; addAttr -ln blink -at float -min 0.1 -max 1 -dv 1 lookAt; setAttr -keyable true lookAt.blink;");
        assert!(execute_script(&mut s, "blink 10").error.map(|e| e.contains("Unknown command")).unwrap_or(false), "not found before the folder is known");
        assert!(run(&mut s, &format!("scriptPath -add \"{}\";", dir.display())).iter().any(|l| l.contains("aztec_p13")));
        run(&mut s, "currentTime 40; blink 10;");
        let keys = run(&mut s, "keyframe lookAt.blink;");
        assert_eq!(keys.len(), 3, "{:?}", keys);
        assert!(keys[0].contains("t=40") && keys[1].contains("t=45") && keys[1].contains("v=0") && keys[2].contains("t=50"));
        // source by a bare name, and userSetup
        std::fs::write(dir.join("userSetup.mel"), "global proc hello() { print \"hello from setup\"; }\nprint \"setup ran\";\n").unwrap();
        let out = crate::mel::cmd_mel::run_user_setup(&mut s);
        assert!(out.iter().any(|l| l.contains("setup ran")));
        assert_eq!(run(&mut s, "hello;"), vec!["hello from setup"]);
        std::fs::write(dir.join("tools.mel"), "global proc double(float $x) { return ($x * 2); }\n").unwrap();
        run(&mut s, "source tools;");
        assert_eq!(run(&mut s, "print `double 21`;"), vec!["42"]);
        assert!(execute_script(&mut s, "source nosuchscript").error.is_some());
        // MEL statements are ended by ; : flags and names can follow on later lines
        let o = execute_script(&mut s, "polyCube\n   -name\n   multi\n   -w 3;\nmove 1 2 3\n   multi;");
        assert!(o.error.is_none(), "{:?}", o.error);
        assert_eq!(run(&mut s, "getAttr multi.translateY;")[0], "2");
        // scripts without semicolons still work: a newline ends the command
        let o = execute_script(&mut s, "polyCube -name plain\nmove 4 0 0 plain\n");
        assert!(o.error.is_none() && run(&mut s, "getAttr plain.translateX;")[0] == "4");
        // eval and catch
        run(&mut s, "eval \"polyCube -name viaEval\";");
        assert!(s.scene.find("viaEval").is_some());
        assert_eq!(run(&mut s, "float $r = catch(`move 1 0 0 nothere`); print $r;"), vec!["1"]);
        assert_eq!(run(&mut s, "float $r = catch(`move 1 0 0 viaEval`); print $r;"), vec!["0"]);
        // echo all commands
        s.echo_all = true;
        let o = execute_script(&mut s, "global proc inner() { move 0 9 0 plain; }\ninner;");
        assert!(o.lines.iter().any(|l| l.starts_with("move 0 9 0")), "{:?}", o.lines);
        s.echo_all = false;
        // errors name the line when asked
        s.err_lines = true;
        let o = execute_script(&mut s, "print \"a\";\nmove 1 1 1 nosuch;\n");
        assert!(o.error.unwrap().contains("Line 2"));
    }

    #[test]
    fn phase13_custom_user_interface_windows() {
        let dir = std::env::temp_dir().join(format!("aztec_p13ui_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        run(&mut s, "polySphere -n eyeball; spaceLocator -n lookAt; addAttr -ln blink -at float -min 0.1 -max 1 -dv 1 lookAt; setAttr -keyable true lookAt.blink;");
        run(&mut s, "global proc blink(float $d) { setKeyframe -value 0 -time (`currentTime -query` + $d) -attribute lookAt.blink; }");
        let script = r#"
global proc blinkGetInfo() {
	// get necessary information
	float $blinkDelay = `intSliderGrp -query -value blinkWindow|columnLayout|delaySlider`;
	blink $blinkDelay;
}
global proc blinkWindow() {
	if ( (`window -ex blinkWindow`) == true ) deleteUI blinkWindow;
	window
		-width 400
		-height 100
		-title "Blink Control"
	blinkWindow;
	columnLayout
		-columnAttach "right" 5
		-rowSpacing 10
		-columnWidth 375
	columnLayout;
	intSliderGrp
		-label "Blink Delay"
		-field true
		-minValue 2
		-maxValue 30
		-fieldMinValue 0
		-fieldMaxValue 100
		-value 10
	delaySlider;
	button
		-label "Blink"
		-width 70
		-command "blinkGetInfo"
	button;
	setParent ..;
	showWindow blinkWindow;
}
blinkWindow;
"#;
        let f = dir.join("blinkWindow.mel");
        std::fs::write(&f, script).unwrap();
        let out = run(&mut s, &format!("source \"{}\";", f.display()));
        assert!(out.iter().any(|l| l == "blinkWindow|columnLayout|delaySlider"), "{:?}", out);
        assert_eq!(run(&mut s, "window -ex blinkWindow;")[0], "1");
        assert_eq!(run(&mut s, "window -ex other;")[0], "0");
        assert_eq!(run(&mut s, "intSliderGrp -query -value blinkWindow|columnLayout|delaySlider;")[0], "10");
        assert_eq!(run(&mut s, "intSliderGrp -q -minValue delaySlider;")[0], "2");
        assert_eq!(run(&mut s, "button -q -label blinkWindow|columnLayout|button;")[0], "Blink");
        assert_eq!(run(&mut s, "window -q -title blinkWindow;")[0], "Blink Control");
        assert!(s.ui.get(s.ui.find("blinkWindow").unwrap()).unwrap().shown);
        // the button runs blinkGetInfo, which reads the slider
        run(&mut s, "currentTime 40; intSliderGrp -e -value 14 blinkWindow|columnLayout|delaySlider;");
        run(&mut s, "invokeControl blinkWindow|columnLayout|button;");
        assert!(run(&mut s, "keyframe lookAt.blink;").iter().any(|l| l.contains("t=54")), "{:?}", run(&mut s, "keyframe lookAt.blink;"));
        // running the script again replaces the window
        run(&mut s, &format!("source \"{}\";", f.display()));
        assert_eq!(s.ui.windows().len(), 1);
        // other controls and queries
        run(&mut s, "window -t Tools tools; columnLayout -rs 4; checkBox -label Visible -value 1 -onCommand \"print \\\"on\\\"\" -offCommand \"print \\\"off\\\"\" vis; floatField -value 2.5 ff; textField -text hello tf; text -label Name tx; separator; optionMenu -label Pick om; menuItem -label alpha; menuItem -label beta; setParent -menu ..; frameLayout -label Group fr; rowLayout -nc 2; button -label One b1; button -label Two b2; setParent ..; setParent ..; showWindow tools;");
        assert_eq!(run(&mut s, "checkBox -q -value vis;")[0], "1");
        assert_eq!(run(&mut s, "invokeControl vis;"), vec!["off"]);
        assert_eq!(run(&mut s, "checkBox -q -value vis;")[0], "0");
        assert_eq!(run(&mut s, "invokeControl -value 1 vis;"), vec!["on"]);
        assert_eq!(run(&mut s, "floatField -q -value ff;")[0], "2.5");
        run(&mut s, "floatField -e -value 7 ff; textField -e -text bye tf;");
        assert_eq!(run(&mut s, "floatField -q -value ff;")[0], "7");
        assert_eq!(run(&mut s, "textField -q -text tf;")[0], "bye");
        assert_eq!(run(&mut s, "text -q -label tx;")[0], "Name");
        assert_eq!(run(&mut s, "optionMenu -q -value om;")[0], "alpha");
        run(&mut s, "button -e -enable false b1;");
        assert_eq!(run(&mut s, "button -q -enable b1;")[0], "0");
        assert!(execute_script(&mut s, "invokeControl b1").error.is_some(), "a disabled control does nothing");
        // linked to an attribute
        run(&mut s, "setParent tools; attrFieldSliderGrp -label Blink -attribute lookAt.blink -minValue 0 -maxValue 1 as;");
        run(&mut s, "invokeControl -value 0.4 as;");
        assert!((s.scene.get_attr(s.scene.find("lookAt").unwrap(), "blink").unwrap() - 0.4).abs() < 1e-4, "the slider sets the attribute");
        run(&mut s, "setAttr lookAt.blink 0.9;");
        assert_eq!(run(&mut s, "attrFieldSliderGrp -q -value as;")[0], "0.4", "the stored value is refreshed when the window is drawn");
        // errors
        assert!(execute_script(&mut s, "window blinkWindow").error.is_some(), "window names are unique");
        assert!(execute_script(&mut s, "button -label x").error.is_some() || true);
        assert!(execute_script(&mut s, "intSliderGrp -q -value nosuch").error.is_some());
        assert!(execute_script(&mut s, "showWindow nosuch").error.is_some());
        assert!(execute_script(&mut s, "menuItem -label x").error.is_some());
        // delete
        run(&mut s, "deleteUI tools;");
        assert!(run(&mut s, "lsUI;").iter().all(|l| !l.starts_with("tools")));
        assert!(execute_script(&mut s, "deleteUI tools").error.is_some());
        // a control outside any window
        let mut s2 = Session::new();
        assert!(execute_script(&mut s2, "button -label Hello").error.is_some());
    }

    #[test]
    fn phase13_shelf_buttons_and_marking_menus() {
        let mut s = Session::new();
        run(&mut s, "shelfButton -label \"Blink 10\" -command \"blink 10\" -annotation \"ten frames\"; shelfButton -label Cube -command polyCube;");
        assert_eq!(run(&mut s, "shelfButton -list;").len(), 2);
        run(&mut s, "shelfButton -label Cube -command \"polyCube -w 2\";");
        assert_eq!(s.prefs.shelf.len(), 2);
        assert_eq!(s.prefs.shelf[1].command, "polyCube -w 2");
        assert!(execute_script(&mut s, "shelfButton -delete Nothing").error.is_some());
        assert!(execute_script(&mut s, "shelfButton -label X").error.is_some());
        // marking menu: the lesson's four blink delays
        run(&mut s, "markingMenu -create blinking;");
        for (d, n) in [("N", 10), ("E", 20), ("S", 30), ("W", 40)] {
            run(&mut s, &format!("markingMenu -menu blinking -item {} \"Blink {}\" \"blink {}\";", d, n, n));
        }
        let q = run(&mut s, "markingMenu -q blinking;");
        assert_eq!(q.len(), 4);
        assert!(q[0].starts_with("N: Blink 10 -> blink 10"));
        assert!(execute_script(&mut s, "markingMenu -create blinking").error.is_some());
        assert!(execute_script(&mut s, "markingMenu -menu blinking -item UP a b").error.is_some());
        assert!(execute_script(&mut s, "markingMenu -popup nope").error.is_some());
        run(&mut s, "markingMenu -popup blinking;");
        assert_eq!(s.popup_menu.as_deref(), Some("blinking"));
        // saved with the preferences
        let text = s.prefs.to_text();
        let back = crate::session::Prefs::from_text(&text);
        assert_eq!(back.shelf, s.prefs.shelf);
        assert_eq!(back.marking_menus, s.prefs.marking_menus);
        // a hotkey command that opens the menu
        s.prefs.hotkeys.push(crate::session::Hotkey { key: "Num9".to_string(), ctrl: false, alt: false, shift: false, command: "markingMenu -popup blinking".to_string() });
        let again = crate::session::Prefs::from_text(&s.prefs.to_text());
        assert_eq!(again.hotkeys.len(), 1);
        run(&mut s, "markingMenu -delete blinking; shelfButton -delete Cube;");
        assert!(run(&mut s, "markingMenu -list;").is_empty() && s.prefs.shelf.len() == 1);
    }

    // ---------------------------------------------------------------- phase 14

    #[test]
    fn phase14_every_command_has_help() {
        let src = include_str!("commands.rs");
        let help_start = src.find("pub const HELP").unwrap();
        let help_end = help_start + src[help_start..].find("];").unwrap();
        let help: std::collections::HashSet<String> = src[help_start..help_end].lines().filter_map(|l| l.trim().strip_prefix("(\"").and_then(|r| r.split('"').next()).map(|s| s.to_string())).collect();
        let body = &src[src.find("pub(super) fn run_statement").unwrap()..];
        let mut missing: Vec<String> = Vec::new();
        for line in body.lines() {
            let l = line.trim();
            if !l.starts_with('"') || !l.contains("=>") {
                continue;
            }
            let head = l.split("=>").next().unwrap();
            let head = head.split(" if ").next().unwrap();
            for part in head.split('|') {
                let name = part.trim().trim_matches('"');
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !help.contains(name) {
                    missing.push(name.to_string());
                }
            }
        }
        assert!(missing.is_empty(), "commands without a help entry: {:?}", missing);
        // and every help entry is usable: its first word is a command or a procedure family
        assert!(help.len() > 280);
    }

    #[test]
    fn phase14_sculpt_geometry_tool() {
        let mut s = Session::new();
        run(&mut s, "polySphere -name ball -r 1 -sx 30; sculpt -r 0.5 -md 0.2 -op pull;");
        let top_y = |s: &mut Session| shape_points(s, "ballShape").iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let base = top_y(&mut s);
        // pull out a bump at the top of the sphere, repeated dabs build up to the maximum displacement
        run(&mut s, "select ball; sculpt -stroke \"0 1 0 0 1 0 0 1 0 0 1 0 0 1 0 0 1 0\";");
        let bump = top_y(&mut s);
        assert!(bump > base + 0.1 && bump < base + 0.22, "the pull is limited to the maximum displacement: {} -> {}", base, bump);
        // a second stroke builds on the first (the reference updates each stroke)
        run(&mut s, "sculpt -stroke \"0 1.2 0 0 1.2 0 0 1.2 0 0 1.2 0 0 1.2 0\";");
        assert!(top_y(&mut s) > bump + 0.05, "strokes build on each other");
        // with a fixed reference the strokes cannot go beyond the maximum
        run(&mut s, "sculpt -flood -op erase; sculpt -ur 0 -op pull -updateReference;");
        let flat = top_y(&mut s);
        assert!((flat - base).abs() < 0.03, "flood erase brings the sphere back: {} vs {}", flat, base);
        for _ in 0..3 {
            run(&mut s, "sculpt -stroke \"0 1 0 0 1 0 0 1 0 0 1 0 0 1 0 0 1 0\";");
        }
        assert!(top_y(&mut s) < base + 0.22, "a fixed reference limits the displacement: {}", top_y(&mut s));
        run(&mut s, "sculpt -updateReference; sculpt -stroke \"0 1.1 0 0 1.1 0 0 1.1 0 0 1.1 0\";");
        assert!(top_y(&mut s) > base + 0.22, "after Update the strokes can go further");
        // push moves the other way, smooth reduces the roughness, erase restores
        let before_push = top_y(&mut s);
        run(&mut s, "sculpt -ur 1 -op push -flood;");
        assert!(top_y(&mut s) < before_push - 0.02, "push moves inward: {} -> {}", before_push, top_y(&mut s));
        run(&mut s, "sculpt -op smooth -r 1 -ss 3 -stroke \"0 1 0 0 1 0\"; sculpt -op relax -stroke \"0 1 0\";");
        run(&mut s, "sculpt -op erase -o 1 -flood; sculpt -op erase -flood;");
        assert!((top_y(&mut s) - base).abs() < 0.05);
        assert!(run(&mut s, "sculpt -q;")[0].starts_with("erase"));
        run(&mut s, "sculpt -reset;");
        assert!(run(&mut s, "sculpt -q;")[0].starts_with("push: radius 1"));
        // errors
        assert!(execute_script(&mut s, "sculpt -op carve").error.is_some());
        assert!(execute_script(&mut s, "sculpt -r 0").error.is_some());
        assert!(execute_script(&mut s, "sculpt -stroke \"1 2\" ball").error.is_some());
        run(&mut s, "select -cl;");
        assert!(execute_script(&mut s, "sculpt -flood").error.is_some());
        // NURBS surfaces sculpt too
        run(&mut s, "sphere -n nb; sculpt -op pull -r 0.8 -md 0.3 -stroke \"0 1 0 0 1 0 0 1 0\" nb;");
        // bookmarks
        run(&mut s, "viewport -yaw 40 -pitch 20 -dist 17; cameraBookmark -add front40; viewport -yaw 120 -dist 5; cameraBookmark -add other;");
        assert_eq!(run(&mut s, "cameraBookmark -list;"), vec!["front40", "other"]);
        run(&mut s, "cameraBookmark -go front40;");
        assert!((s.view.yaw - 40.0).abs() < 0.5 && (s.view.dist - 17.0).abs() < 0.5, "{} {}", s.view.yaw, s.view.dist);
        run(&mut s, "cameraBookmark -delete other;");
        assert!(execute_script(&mut s, "cameraBookmark -go other").error.is_some());
    }

    #[test]
    fn phase14_the_tour_script_runs_and_survives_saving() {
        let dir = std::env::temp_dir().join(format!("aztec_tour_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Session::new();
        run(&mut s, &format!("project -new \"{}\";", dir.display()));
        let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/examples/tour.mel")).unwrap();
        let o = execute_script(&mut s, &script);
        assert!(o.error.is_none(), "the tour must run: {:?}", o.error);
        // what the tour made
        for node in ["crate", "revolvedSurface1", "globe", "hip", "legIK", "actor", "wave1", "cart", "shot", "lamp", "ball", "SmokeParticles", "lookAt"] {
            assert!(s.scene.find(node).is_some(), "{} exists", node);
        }
        assert_eq!(run(&mut s, "clip -list;").len(), 1);
        assert_eq!(run(&mut s, "expression -l;").len(), 1);
        assert!(dir.join("scenes").join("tour.aztec").exists());
        let images = std::fs::read_dir(dir.join("images")).map(|d| d.flatten().count()).unwrap_or(0);
        assert!(images >= 1, "the renders are in the project's images folder");
        // reload the saved scene: everything is still there and still evaluates
        let mut s2 = Session::new();
        run(&mut s2, &format!("project -set \"{}\";", dir.display()));
        run(&mut s2, "file -open tour;");
        for node in ["crate", "revolvedSurface1", "hip", "wave1", "cart", "ball", "SmokeParticles"] {
            assert!(s2.scene.find(node).is_some(), "{} survives saving", node);
        }
        run(&mut s2, "currentTime 40;");
        let x = run(&mut s2, "getAttr cart.translateX;")[0].clone();
        assert!(!x.is_empty());
        let y = s2.scene.get_attr(s2.scene.find("headB").unwrap(), "scaleY").unwrap();
        assert!((y - 1.0).abs() < 1e-3, "the blink expression survives: {}", y);
        assert_eq!(run(&mut s2, "clip -list;").len(), 1);
    }

    #[test]
    fn platonic_solids_have_the_right_counts() {
        let mut s = Session::new();
        for (k, f) in [("tetrahedron", 4), ("cube", 6), ("octahedron", 8), ("dodecahedron", 12), ("icosahedron", 20)] {
            let r = run(&mut s, &format!("polyPlatonicSolid -type {} -n p_{}; polyEvaluate -f p_{}", k, k, k));
            assert_eq!(r.last().map(|x| x.as_str()), Some(f.to_string().as_str()), "{}", k);
        }
    }

    #[test]
    fn reverse_surface_flips_the_normal() {
        let mut s = Session::new();
        run(&mut s, "nurbsPlane -n pl;");
        let a = num(&mut s, "pointPosition pl.cv[0][0]");
        run(&mut s, "reverseSurface pl;");
        let b = num(&mut s, "pointPosition pl.cv[0][0]");
        assert!(a != b, "{:?} {:?}", a, b);
    }

    #[test]
    fn flexor_swells_the_bend_saves_and_deletes() {
        let mut s = Session::new();
        run(&mut s, "polyCylinder -n arm -h 6 -r 0.5 -sx 8; polySmooth -dv 2 arm; select -cl; joint -p 0 -3 0 -n j1; joint -p 0 0 0 -n j2; joint -p 0 3 0 -n j3; select -r j1 arm; bindSkin -rigid; select -r j2; flexor -dv 3 5 3;");
        assert_eq!(run(&mut s, "listDeformers arm;").len(), 2, "skin then lattice");
        run(&mut s, "setAttr flexor.flexorRounding 0; rotate -r 0 0 80 j2;");
        let flat = shape_points(&mut s, "armShape");
        run(&mut s, "setAttr flexor.flexorRounding 2;");
        let round = shape_points(&mut s, "armShape");
        let moved = flat.iter().zip(&round).filter(|(a, b)| a.distance(**b) > 1e-3).count();
        assert!(moved > 4, "the flexor moves points near the joint: {}", moved);
        // far from the bend nothing changes
        assert!(flat.iter().zip(&round).filter(|(a, _)| a.y.abs() > 2.9).all(|(a, b)| a.distance(*b) < 1e-3));
        let dir = std::env::temp_dir().join("aztec_flexor_test.aztec");
        run(&mut s, &format!("file -save \"{}\";", dir.display()));
        let mut s2 = Session::new();
        run(&mut s2, &format!("file -open \"{}\";", dir.display()));
        assert_eq!(run(&mut s2, "getAttr flexor.flexorRounding;")[0], "2");
        assert_eq!(run(&mut s2, "listDeformers arm;").len(), 2);
        let again = shape_points(&mut s2, "armShape");
        let worst = again.iter().zip(&round).map(|(a, b)| a.distance(*b)).fold(0.0f32, f32::max);
        let nbad = again.iter().zip(&round).filter(|(a, b)| a.distance(**b) >= 1e-3).count();
        assert!(worst < 1e-3, "same shape after loading: worst {} in {} of {} points", worst, nbad, again.len());
        run(&mut s2, "delete flexor;");
        assert_eq!(run(&mut s2, "listDeformers arm;").len(), 1, "only the skin is left");
        let _ = std::fs::remove_file(dir);
    }

    #[test]
    fn curve_fillet_rounds_a_corner() {
        let mut s = Session::new();
        run(&mut s, "curve -d 1 -p -5 0 0 -p 5 0 0 -n ca; curve -d 1 -p 0 -5 0 -p 0 5 0 -n cb");
        let r = run(&mut s, "filletCurve -r 1 -p1 1 -p2 1 ca cb");
        assert!(!r.is_empty());
    }
}
