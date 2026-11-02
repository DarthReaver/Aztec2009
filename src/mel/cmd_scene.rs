use super::commands::{resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::NodeData;
use crate::dag::NodeId;
use crate::session::Session;

const POLY_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["w", "width"], arity: 1 },
    FlagSpec { names: &["h", "height"], arity: 1 },
    FlagSpec { names: &["d", "depth"], arity: 1 },
    FlagSpec { names: &["r", "radius"], arity: 1 },
    FlagSpec { names: &["sx", "sa", "sh", "subdivisions"], arity: 1 },
    // accepted like Maya's own echo of the command: the second subdivision count, UV creation, history and the axis
    FlagSpec { names: &["sy", "sz", "sc", "cuv", "o", "tx"], arity: 1 },
    FlagSpec { names: &["ax", "axis"], arity: 3 },
    FlagSpec { names: &["ch", "constructionHistory"], arity: 1 },
];

pub fn poly(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, POLY_SPEC)?;
    let g = |k: &str, d: f32| -> Result<f32, String> { Ok(p.num(k)?.unwrap_or(d)) };
    let sub_default = match cmd {
        "polyCube" | "polyPlane" => 1.0,
        _ => 20.0,
    };
    let sub = match (p.num("subdivisions")?, p.num("sy")?) {
        (Some(v), _) => v,
        (None, Some(v)) => v,
        _ => sub_default,
    };
    if !(1.0..=200.0).contains(&sub) {
        return Err("Subdivisions must be between 1 and 200".to_string());
    }
    let base: Vec<(&str, f32)> = match cmd {
        "polyCube" => vec![("width", g("width", 1.0)?), ("height", g("height", 1.0)?), ("depth", g("depth", 1.0)?)],
        "polySphere" => vec![("radius", g("radius", 1.0)?)],
        "polyCylinder" => vec![("radius", g("radius", 1.0)?), ("height", g("height", 2.0)?)],
        _ => vec![("width", g("width", 1.0)?), ("height", g("height", 1.0)?)],
    };
    let mut params: Vec<(String, f32)> = Vec::new();
    for (k, v) in base {
        if v <= 0.0 {
            return Err(format!("{} must be greater than zero", k));
        }
        params.push((k.to_string(), v));
    }
    params.push(("subdivisions".to_string(), sub.floor()));
    let name = p.string("name");
    let (t, _) = s.scene.create_mesh_object(cmd, params, name.as_deref())?;
    // -ax turns the primitive's up axis toward the vector
    if let Some(a) = p.get("ax") {
        let v = crate::math::Vec3::new(a[0].as_f32()?, a[1].as_f32()?, a[2].as_f32()?);
        if v.length_sq() > 1e-12 && (v.normalized() - crate::math::Vec3::UP).length_sq() > 1e-8 {
            let q = crate::dag::Scene::quat_from_to(crate::math::Vec3::UP, v.normalized(), crate::math::Vec3::new(1.0, 0.0, 0.0));
            let e = crate::math::Mat4::from_quat(q).decompose().1;
            for (k, x) in ["rotateX", "rotateY", "rotateZ"].iter().zip([e.x, e.y, e.z]) {
                s.scene.set_attr(t, k, x)?;
            }
        }
    }
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

const XFORM_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["r", "relative"], arity: 0 },
    FlagSpec { names: &["a", "absolute"], arity: 0 },
];

pub fn xform(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, XFORM_SPEC)?;
    if p.args.len() < 3 {
        return Err(format!("Usage: {} [-r] x y z [object ...]", cmd));
    }
    let v = [p.args[0].as_f32()?, p.args[1].as_f32()?, p.args[2].as_f32()?];
    let names: Vec<String> = p.args[3..].iter().map(|a| a.as_string()).collect();
    let ids = resolve_targets(s, &names)?;
    let rel = p.has("relative");
    let base = match cmd {
        "move" => "translate",
        "rotate" => "rotate",
        _ => "scale",
    };
    let axes = ["X", "Y", "Z"];
    for id in ids {
        for i in 0..3 {
            let attr = format!("{}{}", base, axes[i]);
            let cur = s.scene.get_attr(id, &attr)?;
            let nv = if rel {
                if cmd == "scale" {
                    cur * v[i]
                } else {
                    cur + v[i]
                }
            } else {
                v[i]
            };
            s.scene.set_attr(id, &attr, nv)?;
        }
    }
    s.view.sync_from_scene(&mut s.scene);
    Ok(vec![])
}

const SELECT_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["cl", "clear"], arity: 0 },
    FlagSpec { names: &["add", "a"], arity: 0 },
    FlagSpec { names: &["all"], arity: 0 },
    FlagSpec { names: &["r", "replace"], arity: 0 },
    FlagSpec { names: &["hierarchy", "hi"], arity: 0 },
];

/// Simple `*` wildcard matching for names.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn match_names(s: &Session, pattern: &str) -> Vec<NodeId> {
    if pattern.contains('*') {
        s.scene
            .outliner_rows()
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| wildcard(pattern, &s.scene.name_of(*id)))
            .collect()
    } else {
        s.scene.find(pattern).into_iter().collect()
    }
}

pub fn select(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SELECT_SPEC)?;
    if p.has("clear") {
        s.scene.selection.clear();
        if p.args.is_empty() && !p.has("all") {
            return Ok(vec![]);
        }
    }
    let comp_args: Vec<String> = p.arg_strings().into_iter().filter(|a| a.contains('[')).collect();
    if !comp_args.is_empty() {
        super::cmd_model::select_components(s, &comp_args, p.has("add"))?;
        return Ok(vec![]);
    }
    s.comp.mode = crate::session::SelMode::Object;
    s.comp.sel = Default::default();
    let mut ids: Vec<NodeId> = Vec::new();
    if p.has("all") {
        // the default perspective camera is not part of Select All
        let persp = s.scene.find("persp");
        ids.extend(s.scene.roots.iter().copied().filter(|r| Some(*r) != persp && !matches!(s.scene.node(*r).map(|n| &n.data), Some(NodeData::RenderGlobals(_)) | Some(NodeData::DynSolver(_)))));
    }
    for a in p.arg_strings() {
        let found = match_names(s, &a);
        if found.is_empty() {
            return Err(format!("No object matches name: {}", a));
        }
        // selecting a set selects its members
        for f in found {
            if matches!(s.scene.node(f).map(|n| &n.data), Some(NodeData::ObjectSet(_))) {
                ids.extend(s.scene.set_members(f));
            } else {
                ids.push(f);
            }
        }
    }
    if p.has("hierarchy") {
        let base: Vec<NodeId> = if ids.is_empty() { s.scene.selection.clone() } else { ids.clone() };
        ids = base.iter().flat_map(|i| s.scene.descendants(*i)).collect();
        ids.dedup();
    }
    if p.has("add") {
        for id in ids {
            if !s.scene.selection.contains(&id) {
                s.scene.selection.push(id);
            }
        }
    } else {
        ids.dedup();
        s.scene.selection = ids;
    }
    Ok(vec![])
}

const LS_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["sl", "selection"], arity: 0 },
    FlagSpec { names: &["type", "t"], arity: 1 },
    FlagSpec { names: &["long", "l"], arity: 0 },
];

pub fn ls(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, LS_SPEC)?;
    let ty = p.string("type");
    let pats = p.arg_strings();
    let mut out = Vec::new();
    for (id, depth) in s.scene.outliner_rows() {
        let node = s.scene.node(id).unwrap();
        if p.has("selection") && !s.scene.selection.contains(&id) {
            continue;
        }
        if let Some(t) = &ty {
            if node.data.type_name() != t {
                continue;
            }
        }
        if !pats.is_empty() && !pats.iter().any(|pt| wildcard(pt, &node.name)) {
            continue;
        }
        if p.has("long") {
            out.push(format!("{}{}  ({})", "  ".repeat(depth), node.name, node.data.type_name()));
        } else {
            out.push(node.name.clone());
        }
    }
    Ok(out)
}

const DELETE_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["constructionHistory", "ch"], arity: 0 },
    FlagSpec { names: &["nonDeformerHistory", "nd"], arity: 0 },
    FlagSpec { names: &["staticChannels", "sc"], arity: 0 },
    FlagSpec { names: &["channels", "c"], arity: 0 },
    FlagSpec { names: &["constraints", "cn"], arity: 0 },
];

pub fn delete(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, DELETE_SPEC)?;
    let by_type: Vec<&str> = ["ch", "nd", "sc", "c", "cn"].into_iter().filter(|f| p.has(match *f { "ch" => "constructionHistory", "nd" => "nonDeformerHistory", "sc" => "staticChannels", "c" => "channels", _ => "constraints" })).collect();
    if !by_type.is_empty() {
        return super::cmd_extra::delete_by_type(s, &by_type, &p.arg_strings());
    }
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut count = 0;
    for id in ids {
        let node = match s.scene.node(id) {
            Some(n) => n,
            None => continue,
        };
        let target = match node.parent {
            // shapes delete their transform; constraints and motion paths only themselves
            Some(p) if !node.data.is_transform() && !matches!(node.data, NodeData::Constraint(_) | NodeData::MotionPath(_)) => p,
            _ => id,
        };
        if s.scene.node(target).is_none() {
            continue;
        }
        let name = s.scene.name_of(target);
        if name == "persp" {
            return Err("The default viewport camera 'persp' cannot be deleted".to_string());
        }
        s.scene.delete_node(target)?;
        count += 1;
    }
    Ok(vec![format!("Deleted {} object(s)", count)])
}

pub fn rename(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    if p.args.len() != 2 {
        return Err("Usage: rename OLD NEW".to_string());
    }
    let old = p.args[0].as_string();
    let id = s.scene.find(&old).ok_or_else(|| format!("No object matches name: {}", old))?;
    let nn = s.scene.rename(id, &p.args[1].as_string())?;
    Ok(vec![nn])
}

const PARENT_SPEC: &[FlagSpec] = &[FlagSpec { names: &["w", "world"], arity: 0 }];

pub fn parent(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, PARENT_SPEC)?;
    let names = p.arg_strings();
    if p.has("world") {
        let ids = resolve_targets(s, &names)?;
        for id in ids {
            s.scene.set_parent_keep_world(id, None)?;
        }
        return Ok(vec![]);
    }
    if names.len() < 2 {
        return Err("Usage: parent CHILD ... PARENT   or   parent -w CHILD ...".to_string());
    }
    let par = s.scene.find(&names[names.len() - 1]).ok_or_else(|| format!("No object matches name: {}", names[names.len() - 1]))?;
    for n in &names[..names.len() - 1] {
        let c = s.scene.find(n).ok_or_else(|| format!("No object matches name: {}", n))?;
        s.scene.set_parent_keep_world(c, Some(par))?;
    }
    Ok(vec![])
}

const NAME_SPEC: &[FlagSpec] = &[FlagSpec { names: &["n", "name"], arity: 1 }];

pub fn group(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, NAME_SPEC)?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let name = p.string("name");
    let g = match &name {
        Some(n) => s.scene.create_node(NodeData::Transform, n, None, false),
        None => s.scene.create_node(NodeData::Transform, "group", None, true),
    };
    for id in ids {
        s.scene.set_parent_keep_world(id, Some(g))?;
    }
    s.scene.selection = vec![g];
    Ok(vec![s.scene.name_of(g)])
}

pub fn duplicate(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut created = Vec::new();
    for id in ids {
        let t = {
            let n = s.scene.node(id).ok_or("Invalid node")?;
            if n.data.is_transform() {
                id
            } else {
                n.parent.ok_or("Cannot duplicate this node")?
            }
        };
        // NURBS curves and surfaces are copied as plain geometry (no history)
        let sshapes = s.scene.surface_shapes_under(t);
        let cshapes = s.scene.curve_shapes_under(t);
        if sshapes.first().is_some() || cshapes.first().is_some() {
            let base = s.scene.name_of(t);
            let nt = if let Some(sh) = sshapes.first() {
                let surf = match s.scene.node(*sh).map(|n| &n.data) {
                    Some(NodeData::Mesh(md)) => md.nurbs.as_ref().map(|n| n.surf.clone()).unwrap(),
                    _ => unreachable!(),
                };
                let nt = s.scene.create_nurbs_surface(Some(&base), None, Vec::new(), Some(surf))?;
                if let Some(NodeData::Mesh(md)) = s.scene.node(*sh).map(|n| &n.data) {
                    if let Some(m) = md.material {
                        s.scene.assign_material(m, nt)?;
                    }
                }
                nt
            } else {
                let c = match s.scene.node(cshapes[0]).map(|n| &n.data) {
                    Some(NodeData::NurbsCurve(cd)) => cd.curve.clone(),
                    _ => unreachable!(),
                };
                s.scene.create_nurbs_curve(Some(&base), c, None)?
            };
            for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ", "visibility", "rotatePivotX", "rotatePivotY", "rotatePivotZ", "scalePivotX", "scalePivotY", "scalePivotZ", "rotatePivotTranslateX", "rotatePivotTranslateY", "rotatePivotTranslateZ"] {
                let v = s.scene.get_attr(t, a)?;
                s.scene.set_attr(nt, a, v)?;
            }
            created.push(nt);
            continue;
        }
        let shapes = s.scene.mesh_shapes_under(t);
        let first = *shapes.first().ok_or("Only mesh objects can be duplicated")?;
        let (mesh, mat, hist) = match s.scene.node(first).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => (md.mesh.clone(), md.material, !md.history.is_empty()),
            _ => return Err("Only mesh objects can be duplicated".to_string()),
        };
        let base = s.scene.name_of(t);
        let nt = s.scene.create_node(NodeData::Transform, &base, None, true);
        let sname = format!("{}Shape", s.scene.name_of(nt));
        let md = crate::dag::nodes::MeshData { cache: Default::default(), history: Vec::new(), mesh, prim: "custom".to_string(), params: Vec::new(), material: None, ..Default::default() };
        let ns = s.scene.create_node(NodeData::Mesh(md), &sname, Some(nt), false);
        if hist {
            s.scene.clone_history(first, ns);
        }
        for a in [
            "translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ",
            "visibility",
        ] {
            let v = s.scene.get_attr(t, a)?;
            s.scene.set_attr(nt, a, v)?;
        }
        if let Some(m) = mat {
            s.scene.assign_material(m, nt)?;
        }
        created.push(nt);
    }
    let names: Vec<String> = created.iter().map(|i| s.scene.name_of(*i)).collect();
    s.scene.selection = created;
    Ok(names)
}

const CAMERA_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["interaxial", "ia"], arity: 1 },
    FlagSpec { names: &["zeroParallax", "zp"], arity: 1 },
    FlagSpec { names: &["fov", "fieldOfView"], arity: 1 },
];

pub fn camera(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, CAMERA_SPEC)?;
    let stereo = cmd == "stereoCamera";
    let name = p.string("name");
    let (t, shape) = s.scene.create_camera(stereo, name.as_deref());
    s.scene.set_attr(t, "translateY", 5.0)?;
    s.scene.set_attr(t, "translateZ", 15.0)?;
    s.scene.set_attr(t, "rotateX", -18.4)?;
    if let Some(f) = p.num("fov")? {
        s.scene.set_attr(shape, "fieldOfView", f.clamp(1.0, 170.0))?;
    }
    if stereo {
        if let Some(v) = p.num("interaxial")? {
            s.scene.set_attr(shape, "interaxialSeparation", v)?;
        }
        if let Some(v) = p.num("zeroParallax")? {
            s.scene.set_attr(shape, "zeroParallax", v.max(0.2))?;
        }
    }
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

const LIGHT_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["i", "intensity"], arity: 1 },
    FlagSpec { names: &["c", "color"], arity: 3 },
    FlagSpec { names: &["ca", "coneAngle"], arity: 1 },
    FlagSpec { names: &["pa", "penumbraAngle"], arity: 1 },
    FlagSpec { names: &["dro", "dropoff"], arity: 1 },
    FlagSpec { names: &["dr", "decayRate"], arity: 1 },
    FlagSpec { names: &["sh", "shadows"], arity: 1 },
    FlagSpec { names: &["ss", "softness", "shadowSoftness"], arity: 1 },
    FlagSpec { names: &["sam", "samples", "shadowSamples"], arity: 1 },
    FlagSpec { names: &["sc", "shadowColor"], arity: 3 },
    FlagSpec { names: &["dif", "emitDiffuse"], arity: 1 },
    FlagSpec { names: &["spc", "emitSpecular"], arity: 1 },
];

pub fn light(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, LIGHT_SPEC)?;
    let ty = match cmd {
        "directionalLight" => 1.0,
        "spotLight" => 2.0,
        "ambientLight" => 3.0,
        "areaLight" => 4.0,
        _ => 0.0,
    };
    let name = p.string("name");
    let (t, shape) = s.scene.create_light(ty, name.as_deref());
    let aim = |s: &mut Session, from: (f32, f32, f32)| -> Result<(), String> {
        s.scene.set_attr(t, "translateX", from.0)?;
        s.scene.set_attr(t, "translateY", from.1)?;
        s.scene.set_attr(t, "translateZ", from.2)?;
        // a light shines down its local -Z axis: turn it toward the origin
        let d = crate::math::Vec3::new(-from.0, -from.1, -from.2).normalized();
        s.scene.set_attr(t, "rotateX", d.y.asin().to_degrees())?;
        s.scene.set_attr(t, "rotateY", (-d.x).atan2(-d.z).to_degrees())?;
        Ok(())
    };
    match ty as i32 {
        1 => {
            s.scene.set_attr(t, "translateY", 10.0)?;
            s.scene.set_attr(t, "rotateX", -90.0)?;
        }
        2 | 4 => aim(s, (6.0, 10.0, 6.0))?,
        3 => {
            s.scene.set_attr(t, "translateY", 6.0)?;
            s.scene.set_attr(shape, "intensity", 0.3)?;
            s.scene.set_attr(shape, "shadows", 0.0)?;
        }
        _ => {
            s.scene.set_attr(t, "translateX", 6.0)?;
            s.scene.set_attr(t, "translateY", 10.0)?;
            s.scene.set_attr(t, "translateZ", 6.0)?;
        }
    }
    if ty as i32 == 2 {
        s.scene.set_attr(shape, "penumbraAngle", 5.0)?;
    }
    if let Some(i) = p.num("intensity")? {
        s.scene.set_attr(shape, "intensity", i.max(0.0))?;
    }
    for (flag, attr, lo, hi) in [
        ("coneAngle", "coneAngle", 2.0, 179.0),
        ("penumbraAngle", "penumbraAngle", 0.0, 60.0),
        ("dropoff", "dropoff", 0.0, 255.0),
        ("decayRate", "decayRate", 0.0, 4.0),
        ("shadows", "shadows", 0.0, 1.0),
        ("shadowSoftness", "shadowSoftness", 0.0, 1000.0),
        ("shadowSamples", "shadowSamples", 1.0, 64.0),
        ("emitDiffuse", "emitDiffuse", 0.0, 1.0),
        ("emitSpecular", "emitSpecular", 0.0, 1.0),
    ] {
        if let Some(v) = p.num(flag)? {
            s.scene.set_attr(shape, attr, v.clamp(lo, hi))?;
        }
    }
    for (flag, base) in [("color", "color"), ("shadowColor", "shadowColor")] {
        if let Some(c) = p.get(flag) {
            for (i, a) in ["R", "G", "B"].iter().enumerate() {
                s.scene.set_attr(shape, &format!("{}{}", base, a), c[i].as_f32()?.clamp(0.0, 1.0))?;
            }
        }
    }
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

pub fn lambert(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, LIGHT_SPEC)?;
    let name = p.string("name");
    let m = s.scene.create_material(name.as_deref());
    // a Lambert has no specular highlight
    s.scene.set_attr(m, "shaderModel", 0.0)?;
    if let Some(c) = p.get("color") {
        s.scene.set_attr(m, "colorR", c[0].as_f32()?)?;
        s.scene.set_attr(m, "colorG", c[1].as_f32()?)?;
        s.scene.set_attr(m, "colorB", c[2].as_f32()?)?;
    }
    Ok(vec![s.scene.name_of(m)])
}

pub fn assign_material(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let names = p.arg_strings();
    if names.is_empty() {
        return Err("Usage: assignMaterial MATERIAL [object ...]   (or obj.f[0:5] to texture faces)".to_string());
    }
    if let Some(r) = super::cmd_tex::assign_material_cmd(s, &names) {
        return r;
    }
    let mat = s.scene.find(&names[0]).ok_or_else(|| format!("No object matches name: {}", names[0]))?;
    let targets = resolve_targets(s, &names[1..])?;
    let mut n = 0;
    for t in targets {
        n += s.scene.assign_material(mat, t)?;
    }
    Ok(vec![format!("Assigned {} to {} mesh(es)", names[0], n)])
}

const SOLVER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["rate"], arity: 1 },
    FlagSpec { names: &["speed"], arity: 1 },
    FlagSpec { names: &["pos", "position"], arity: 3 },
    FlagSpec { names: &["max", "count"], arity: 1 },
    FlagSpec { names: &["radius"], arity: 1 },
];

pub fn solver(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, SOLVER_SPEC)?;
    let name = p.string("name");
    let id = match (&name, cmd) {
        (Some(n), _) => s.scene.create_solver(Some(n)),
        (None, "nParticle") => s.scene.create_node(
            NodeData::from_type_name("nucleus").unwrap(),
            "nParticle",
            None,
            true,
        ),
        (None, _) => s.scene.create_solver(None),
    };
    if let Some(v) = p.num("rate")? {
        s.scene.set_attr(id, "emitRate", v.max(0.0))?;
    }
    if let Some(v) = p.num("speed")? {
        s.scene.set_attr(id, "emitSpeed", v)?;
    }
    if let Some(v) = p.num("max")? {
        s.scene.set_attr(id, "maxParticles", v.max(0.0))?;
    }
    if let Some(v) = p.num("radius")? {
        s.scene.set_attr(id, "particleRadius", v.max(0.001))?;
    }
    let pos = match p.get("pos") {
        Some(v) => Some((v[0].as_f32()?, v[1].as_f32()?, v[2].as_f32()?)),
        None => None,
    };
    if let Some(v) = &pos {
        s.scene.set_attr(id, "emitX", v.0)?;
        s.scene.set_attr(id, "emitY", v.1)?;
        s.scene.set_attr(id, "emitZ", v.2)?;
    }
    // the emitter is a real object: select it, move it, rotate it to aim, key it
    let (et, _) = s.scene.create_emitter(id, None)?;
    s.scene.evaluate_frame();
    s.scene.selection = vec![et];
    Ok(vec![s.scene.name_of(id), s.scene.name_of(et)])
}

/// `emitter [-name N] [-solver NUCLEUS]`: add an emitter to a solver that has none.
pub fn emitter(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["n", "name"], arity: 1 }, FlagSpec { names: &["solver", "s"], arity: 1 }])?;
    let solver = match p.string("solver") {
        Some(n) => s.scene.find(&n).ok_or_else(|| format!("No object matches name: {}", n))?,
        None => s
            .scene
            .ids_where(|d| matches!(d, NodeData::Nucleus(_)))
            .into_iter()
            .find(|id| s.scene.node(*id).and_then(|n| n.plug("emitX")).map(|p| p.source.is_none()).unwrap_or(false))
            .ok_or("No nucleus without an emitter: create one with nParticle")?,
    };
    let name = p.string("name");
    let (t, _) = s.scene.create_emitter(solver, name.as_deref())?;
    s.scene.evaluate_frame();
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

/// Vertices (on the unit sphere) and faces of the five Platonic solids.
fn platonic(kind: &str) -> Option<(Vec<crate::math::Vec3>, Vec<Vec<u32>>)> {
    use crate::math::Vec3;
    let phi = (1.0 + 5f32.sqrt()) / 2.0;
    let (v, f): (Vec<[f32; 3]>, Vec<Vec<u32>>) = match kind {
        "tetrahedron" => (vec![[1.0, 1.0, 1.0], [1.0, -1.0, -1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, 1.0]], vec![vec![0, 2, 1], vec![0, 1, 3], vec![0, 3, 2], vec![1, 2, 3]]),
        "cube" => (
            vec![[-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]],
            vec![vec![0, 3, 2, 1], vec![4, 5, 6, 7], vec![0, 1, 5, 4], vec![3, 7, 6, 2], vec![0, 4, 7, 3], vec![1, 2, 6, 5]],
        ),
        "octahedron" => (
            vec![[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]],
            vec![vec![0, 2, 4], vec![2, 1, 4], vec![1, 3, 4], vec![3, 0, 4], vec![2, 0, 5], vec![1, 2, 5], vec![3, 1, 5], vec![0, 3, 5]],
        ),
        "icosahedron" => {
            let v = vec![[-1.0, phi, 0.0], [1.0, phi, 0.0], [-1.0, -phi, 0.0], [1.0, -phi, 0.0], [0.0, -1.0, phi], [0.0, 1.0, phi], [0.0, -1.0, -phi], [0.0, 1.0, -phi], [phi, 0.0, -1.0], [phi, 0.0, 1.0], [-phi, 0.0, -1.0], [-phi, 0.0, 1.0]];
            let f = [[0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8], [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1]];
            (v, f.iter().map(|t| t.to_vec()).collect())
        }
        "dodecahedron" => {
            // the dual of the icosahedron: one vertex at the center of each icosahedron face, one face around each icosahedron vertex
            let (iv, ifs) = platonic("icosahedron")?;
            let centers: Vec<Vec3> = ifs.iter().map(|f| (iv[f[0] as usize] + iv[f[1] as usize] + iv[f[2] as usize]).normalized()).collect();
            let mut faces: Vec<Vec<u32>> = Vec::new();
            for (vi, v) in iv.iter().enumerate() {
                let mut around: Vec<u32> = ifs.iter().enumerate().filter(|(_, f)| f.contains(&(vi as u32))).map(|(i, _)| i as u32).collect();
                // order the face centers around the vertex
                let n = v.normalized();
                let helper = if n.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
                let (a, b) = (n.cross(helper).normalized(), n.cross(n.cross(helper)).normalized());
                around.sort_by(|x, y| {
                    let ang = |i: u32| {
                        let c = centers[i as usize];
                        c.dot(b).atan2(c.dot(a))
                    };
                    ang(*x).partial_cmp(&ang(*y)).unwrap()
                });
                around.reverse();
                faces.push(around);
            }
            return Some((centers, faces));
        }
        _ => return None,
    };
    Some((v.into_iter().map(|p| Vec3::new(p[0], p[1], p[2]).normalized()).collect(), f))
}

const PLATONIC_SPEC: &[FlagSpec] = &[FlagSpec { names: &["type", "t"], arity: 1 }, FlagSpec { names: &["radius", "r"], arity: 1 }, FlagSpec { names: &["name", "n"], arity: 1 }];

/// `polyPlatonicSolid [-type tetrahedron|cube|octahedron|dodecahedron|icosahedron] [-r RADIUS] [-n NAME]`: Create > Polygon Primitives > Platonic Solids.
/// The solid is made as a plain mesh (no live Solid Type channel).
pub fn poly_platonic(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, PLATONIC_SPEC)?;
    let kind = p.string("type").unwrap_or_else(|| "dodecahedron".to_string());
    let (pos, faces) = platonic(&kind).ok_or_else(|| format!("Unknown solid '{}': tetrahedron, cube, octahedron, dodecahedron or icosahedron", kind))?;
    let r = p.num("radius")?.unwrap_or(1.0);
    if r <= 0.0 {
        return Err("The radius must be above zero".to_string());
    }
    let poly = crate::geo::edit::Poly { pos: pos.into_iter().map(|v| v * r).collect(), faces };
    let name = p.string("name");
    let t = s.scene.create_node(NodeData::Transform, name.as_deref().unwrap_or("polyPlatonicSolid"), None, name.is_none());
    let shape_name = format!("{}Shape", s.scene.name_of(t));
    let md = crate::dag::nodes::MeshData { cache: Default::default(), history: Vec::new(), mesh: poly.to_mesh(), prim: "custom".to_string(), params: Vec::new(), material: None, ..Default::default() };
    s.scene.create_node(NodeData::Mesh(md), &shape_name, Some(t), false);
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}
