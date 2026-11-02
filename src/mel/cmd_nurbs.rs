//! NURBS curves and surfaces: primitives, curve tools, Revolve / Loft / Extrude, attach, detach, isoparms.

use super::commands::{fmt_f, resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec, Parsed};
use crate::dag::nodes::{CurveData, MeshData, NodeData};
use crate::dag::NodeId;
use crate::geo::nurbs::{self, Curve, Dir, Surface};
use crate::math::Vec3;
use crate::session::Session;

const PRIM_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["radius", "r"], arity: 1 },
    FlagSpec { names: &["heightRatio", "hr"], arity: 1 },
    FlagSpec { names: &["height", "h"], arity: 1 },
    FlagSpec { names: &["sections", "s"], arity: 1 },
    FlagSpec { names: &["spans", "nsp"], arity: 1 },
    FlagSpec { names: &["width", "w"], arity: 1 },
    FlagSpec { names: &["length", "l"], arity: 1 },
    FlagSpec { names: &["patchesU", "u"], arity: 1 },
    FlagSpec { names: &["patchesV", "v"], arity: 1 },
    FlagSpec { names: &["axis", "ax"], arity: 3 },
    FlagSpec { names: &["pivot", "p"], arity: 3 },
    FlagSpec { names: &["constructionHistory", "ch"], arity: 1 },
    FlagSpec { names: &["degree", "d"], arity: 1 },
    FlagSpec { names: &["startSweep", "ssw"], arity: 1 },
    FlagSpec { names: &["endSweep", "esw"], arity: 1 },
    FlagSpec { names: &["tolerance", "tol"], arity: 1 },
    FlagSpec { names: &["object", "o"], arity: 1 },
];

fn set(s: &mut Session, shape: NodeId, attr: &str, v: Option<f32>) -> Result<(), String> {
    if let Some(v) = v {
        s.scene.set_attr(shape, attr, v)?;
    }
    Ok(())
}

/// The shape of a freshly created primitive transform.
fn shape_of(s: &Session, t: NodeId) -> NodeId {
    s.scene.node(t).and_then(|n| n.children.first().copied()).unwrap_or(t)
}

/// `sphere`, `cylinder`, `cone`, `torus`, `nurbsPlane`, `circle`, `nurbsSquare`: Create > NURBS Primitives.
pub fn primitive(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, PRIM_SPEC)?;
    let name = p.string("name");
    let n = |k: &str| p.num(k);
    let t = match cmd {
        "circle" | "nurbsSquare" => {
            let (kind, curve) = if cmd == "circle" { ("circle", nurbs::circle(1.0, 8, 3)) } else { ("square", Curve::open(vec![Vec3::ZERO, Vec3::ZERO], 1)) };
            let t = s.scene.create_nurbs_curve(name.as_deref(), curve, Some((kind, Vec::new())))?;
            let sh = shape_of(s, t);
            set(s, sh, "radius", n("radius")?)?;
            set(s, sh, "sections", n("sections")?)?;
            set(s, sh, "sideLength", n("width")?)?;
            t
        }
        _ => {
            let kind = match cmd {
                "sphere" => "sphere",
                "cylinder" => "cylinder",
                "cone" => "cone",
                "torus" => "torus",
                _ => "plane",
            };
            let t = s.scene.create_nurbs_surface(name.as_deref(), Some(kind), Vec::new(), None)?;
            let sh = shape_of(s, t);
            set(s, sh, "radius", n("radius")?)?;
            set(s, sh, "height", n("height")?)?;
            set(s, sh, "sections", n("sections")?)?;
            set(s, sh, "spans", n("spans")?)?;
            set(s, sh, "width", n("width")?)?;
            set(s, sh, "length", n("length")?)?;
            set(s, sh, "patchesU", n("patchesU")?)?;
            set(s, sh, "patchesV", n("patchesV")?)?;
            if let Some(hr) = n("heightRatio")? {
                let r = s.scene.get_attr(sh, "radius").unwrap_or(1.0);
                if kind == "torus" {
                    // the section radius is radius / heightRatio
                    set(s, sh, "sectionRadius", Some(r / hr.max(0.1)))?;
                } else if kind == "cylinder" || kind == "cone" {
                    // the height is the radius times the ratio
                    set(s, sh, "height", Some(r * hr.max(0.01)))?;
                }
            }
            t
        }
    };
    // -axis turns the primitive's up axis toward the vector, -pivot puts its center there
    if let Some(a) = p.get("axis") {
        let v = Vec3::new(a[0].as_f32()?, a[1].as_f32()?, a[2].as_f32()?);
        if v.length_sq() > 1e-12 {
            let q = crate::dag::Scene::quat_from_to(Vec3::UP, v.normalized(), Vec3::new(1.0, 0.0, 0.0));
            let e = crate::math::Mat4::from_quat(q).decompose().1;
            for (k, x) in ["rotateX", "rotateY", "rotateZ"].iter().zip([e.x, e.y, e.z]) {
                s.scene.set_attr(t, k, x)?;
            }
        }
    }
    if let Some(a) = p.get("pivot") {
        for (k, x) in ["translateX", "translateY", "translateZ"].iter().zip(a.iter()) {
            s.scene.set_attr(t, k, x.as_f32()?)?;
        }
    }
    s.scene.selection = vec![t];
    s.scene.flush_meshes();
    Ok(vec![s.scene.name_of(t)])
}

const CURVE_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["degree", "d"], arity: 1 },
    FlagSpec { names: &["point", "p"], arity: 3 },
    FlagSpec { names: &["editPoint", "ep"], arity: 3 },
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["live"], arity: 0 },
];

/// `curve -d 3 -p x y z -p x y z ...` (control vertices) or `curve -ep x y z ...` (the curve passes through the
/// edit points): CV Curve Tool / EP Curve Tool.
pub fn curve(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, CURVE_SPEC)?;
    let degree = p.num("degree")?.unwrap_or(3.0).round();
    if !(1.0..=7.0).contains(&degree) {
        return Err("-d must be between 1 and 7".to_string());
    }
    let degree = degree as usize;
    let (mut pts, ep) = collect_points(&p, rest)?;
    if p.has("live") {
        // the points fall onto the live surface (straight down, else the closest point)
        let live = s.scene.live.ok_or("Make a surface live first (makeLive)")?;
        for q in pts.iter_mut() {
            if let Some(h) = s.scene.snap_to_surface(live, *q, Vec3::new(0.0, -1.0, 0.0)) {
                *q = h;
            }
        }
    }
    if pts.len() < 2 {
        return Err("A curve needs at least two points".to_string());
    }
    let c = if ep {
        if degree == 1 || pts.len() <= 2 {
            Curve::open(pts, 1)
        } else {
            nurbs::interpolate(&pts, degree.min(pts.len() - 1))
        }
    } else {
        if pts.len() <= degree {
            return Err(format!("A degree {} curve needs at least {} CVs", degree, degree + 1));
        }
        Curve::open(pts, degree)
    };
    let t = s.scene.create_nurbs_curve(p.string("name").as_deref(), c, None)?;
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

/// All `-p` / `-ep` groups of a command line, in order.
fn collect_points(_p: &Parsed, rest: &[Token]) -> Result<(Vec<Vec3>, bool), String> {
    let mut pts = Vec::new();
    let mut ep = false;
    let mut i = 0;
    while i < rest.len() {
        if let Token::Flag(f) = &rest[i] {
            let f = f.trim_start_matches('-');
            if f == "p" || f == "point" || f == "ep" || f == "editPoint" {
                if f.starts_with('e') {
                    ep = true;
                }
                if i + 3 >= rest.len() {
                    return Err("-p needs x y z".to_string());
                }
                let v: Vec<f32> = rest[i + 1..=i + 3].iter().map(|t| match t { Token::Num(n) => Ok(*n as f32), _ => Err("-p needs numbers".to_string()) }).collect::<Result<_, String>>()?;
                pts.push(Vec3::new(v[0], v[1], v[2]));
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    Ok((pts, ep))
}

/// Curve shapes of the named objects (or the selection).
fn curves_of(s: &Session, names: &[String]) -> Result<Vec<NodeId>, String> {
    let ids = resolve_targets(s, names)?;
    let mut out = Vec::new();
    for id in ids {
        out.extend(s.scene.curve_shapes_under(id));
    }
    Ok(out)
}

fn surfaces_of(s: &Session, names: &[String]) -> Result<Vec<NodeId>, String> {
    let ids = resolve_targets(s, names)?;
    let mut out = Vec::new();
    for id in ids {
        out.extend(s.scene.surface_shapes_under(id));
    }
    Ok(out)
}

fn names_only(p: &Parsed) -> Vec<String> {
    p.arg_strings().into_iter().filter(|a| !a.contains('[')).collect()
}

fn finish_surface(s: &mut Session, t: NodeId) -> Out {
    s.scene.selection = vec![t];
    s.scene.flush_meshes();
    Ok(vec![s.scene.name_of(t)])
}

const REV_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["axis", "ax"], arity: 3 },
    FlagSpec { names: &["degrees", "ssw"], arity: 1 },
    FlagSpec { names: &["endSweep", "esw"], arity: 1 },
    FlagSpec { names: &["sections", "s"], arity: 1 },
    FlagSpec { names: &["name", "n"], arity: 1 },
];

/// `revolve [-ax X Y Z] [-esw DEGREES] [-s SECTIONS] [curve]`: Surfaces > Revolve (about the curve's pivot).
pub fn revolve(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, REV_SPEC)?;
    let curves = curves_of(s, &names_only(&p))?;
    let c = *curves.first().ok_or("Select a curve to revolve")?;
    let t = s.scene.create_nurbs_surface(p.string("name").as_deref(), Some("revolve"), vec![c], None)?;
    let sh = shape_of(s, t);
    if let Some(a) = p.get("axis") {
        for (k, v) in ["axisX", "axisY", "axisZ"].iter().zip(a) {
            s.scene.set_attr(sh, k, v.as_f32()?)?;
        }
    }
    set(s, sh, "degrees", p.num("endSweep")?)?;
    set(s, sh, "sections", p.num("sections")?)?;
    finish_surface(s, t)
}

/// `loft curve curve ...`: Surfaces > Loft; the curves are used in selection order.
pub fn loft(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["name", "n"], arity: 1 }])?;
    let curves = curves_of(s, &names_only(&p))?;
    if curves.len() < 2 {
        return Err("Select at least two curves (in order) to loft".to_string());
    }
    let t = s.scene.create_nurbs_surface(p.string("name").as_deref(), Some("loft"), curves, None)?;
    finish_surface(s, t)
}

/// `extrude profile path`: Surfaces > Extrude (tube at the path); select the profile, then the path.
pub fn extrude(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["name", "n"], arity: 1 }])?;
    let curves = curves_of(s, &names_only(&p))?;
    if curves.len() != 2 {
        return Err("Select the profile curve, then the path curve".to_string());
    }
    let t = s.scene.create_nurbs_surface(p.string("name").as_deref(), Some("extrude"), curves, None)?;
    finish_surface(s, t)
}

/// World-space copy of a curve shape.
fn world_curve(s: &mut Session, shape: NodeId) -> Option<Curve> {
    let c = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::NurbsCurve(cd)) => cd.curve.clone(),
        _ => return None,
    };
    let parent = s.scene.node(shape).and_then(|n| n.parent)?;
    let w = s.scene.world_matrix(parent);
    let mut c = c;
    c.transform(|p| w.transform_point(p));
    Some(c)
}

/// `attachCurve curveA curveB`: Edit Curves > Attach Curves (connect): one curve; the originals are removed.
pub fn attach_curve(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["keepOriginals", "ko"], arity: 1 }])?;
    let curves = curves_of(s, &names_only(&p))?;
    if curves.len() != 2 {
        return Err("Select two curves to attach".to_string());
    }
    let a = world_curve(s, curves[0]).ok_or("Not a curve")?;
    let mut b = world_curve(s, curves[1]).ok_or("Not a curve")?;
    // join the closest pair of ends
    let mut a = a;
    let d = |x: Vec3, y: Vec3| x.distance(y);
    let (ea, sa) = (a.end(), a.start());
    let (sb, eb) = (b.start(), b.end());
    let best = [(d(ea, sb), 0), (d(ea, eb), 1), (d(sa, sb), 2), (d(sa, eb), 3)].into_iter().min_by(|x, y| x.0.partial_cmp(&y.0).unwrap()).unwrap().1;
    match best {
        1 => b.reverse(),
        2 => a.reverse(),
        3 => {
            a.reverse();
            b.reverse();
        }
        _ => {}
    }
    let joined = a.attach(&b);
    let keep = p.num("keepOriginals")?.map(|v| v > 0.5).unwrap_or(false);
    let parents: Vec<NodeId> = curves.iter().filter_map(|c| s.scene.node(*c).and_then(|n| n.parent)).collect();
    let t = s.scene.create_nurbs_curve(None, joined, None)?;
    if !keep {
        for t in parents {
            let _ = s.scene.delete_node(t);
        }
    }
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

const ISO_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["u"], arity: 1 },
    FlagSpec { names: &["v"], arity: 1 },
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["keepOriginal", "ko"], arity: 1 },
];

/// Which isoparm (direction, parameter) the flags name. `-u T`: the line of constant u.
fn iso_of(p: &Parsed) -> Result<(bool, f32), String> {
    match (p.num("u")?, p.num("v")?) {
        (Some(t), None) => Ok((false, t)),
        (None, Some(t)) => Ok((true, t)),
        _ => Err("Give the isoparm with -u PARAM (constant u) or -v PARAM (constant v)".to_string()),
    }
}

fn surface_data(s: &Session, shape: NodeId) -> Result<Surface, String> {
    match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md.nurbs.as_ref().map(|n| n.surf.clone()).ok_or_else(|| "Not a NURBS surface".to_string()),
        _ => Err("Not a NURBS surface".to_string()),
    }
}

fn check_range(sf: &Surface, in_u_direction_param: bool, t: f32) -> Result<(), String> {
    // `in_u_direction_param`: the parameter belongs to u
    let (a, b) = if in_u_direction_param { sf.u.domain() } else { sf.v.domain() };
    if t <= a + 1e-4 || t >= b - 1e-4 {
        return Err(format!("The isoparm parameter must lie inside {} .. {}", fmt_f(a), fmt_f(b)));
    }
    Ok(())
}

/// `duplicateCurve -u T | -v T surface`: Edit Curves > Duplicate Surface Curves.
pub fn duplicate_surface_curve(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, ISO_SPEC)?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    let sh = *shapes.first().ok_or("Select a NURBS surface")?;
    let sf = surface_data(s, sh)?;
    let (const_v, t) = iso_of(&p)?;
    let (a, b) = if const_v { sf.v.domain() } else { sf.u.domain() };
    if t < a - 1e-4 || t > b + 1e-4 {
        return Err(format!("The isoparm parameter must lie inside {} .. {}", fmt_f(a), fmt_f(b)));
    }
    let mut c = sf.isoparm(const_v, t);
    let owner = s.scene.node(sh).and_then(|n| n.parent).ok_or("Surface has no transform")?;
    let w = s.scene.world_matrix(owner);
    c.transform(|q| w.transform_point(q));
    let out = s.scene.create_nurbs_curve(p.string("name").as_deref().or(Some("curveFromSurface")), c, None)?;
    s.scene.selection = vec![out];
    Ok(vec![s.scene.name_of(out)])
}

/// `insertKnotSurface -u T | -v T surface`: Edit NURBS > Insert Isoparms.
pub fn insert_isoparm(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, ISO_SPEC)?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    let sh = *shapes.first().ok_or("Select a NURBS surface")?;
    let sf = surface_data(s, sh)?;
    let (const_v, t) = iso_of(&p)?;
    // constant u -> knot in the u direction
    check_range(&sf, !const_v, t)?;
    let ns = sf.insert_knot(!const_v, t)?;
    replace_surface(s, sh, ns, true);
    Ok(vec![format!("Inserted an isoparm at {}", fmt_f(t))])
}

/// Replace a surface's geometry; the history is dropped (the structure changed).
fn replace_surface(s: &mut Session, shape: NodeId, new: Surface, drop_history: bool) {
    if drop_history {
        s.scene.delete_nurbs_history(shape);
    }
    if let Some(NodeData::Mesh(md)) = s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
        if let Some(ns) = md.nurbs.as_mut() {
            ns.surf = new;
        }
    }
    s.scene.retessellate(shape);
}

/// `detachSurface -u T | -v T surface`: Edit NURBS > Detach Surfaces: two surfaces (the original becomes the first).
pub fn detach_surface(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, ISO_SPEC)?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    let sh = *shapes.first().ok_or("Select a NURBS surface")?;
    let sf = surface_data(s, sh)?;
    let (const_v, t) = iso_of(&p)?;
    check_range(&sf, !const_v, t)?;
    let (a, b) = sf.split(!const_v, t)?;
    let owner = s.scene.node(sh).and_then(|n| n.parent).ok_or("Surface has no transform")?;
    let name = s.scene.name_of(owner);
    replace_surface(s, sh, a, true);
    let t2 = s.scene.create_nurbs_surface(Some(&format!("{}_detached", name)), None, Vec::new(), Some(b))?;
    // the second piece sits where the first one does
    for attr in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"] {
        let v = s.scene.get_attr(owner, attr)?;
        s.scene.set_attr(t2, attr, v)?;
    }
    if let Some(m) = surface_material(s, sh) {
        let _ = s.scene.assign_material(m, t2);
    }
    s.scene.selection = vec![owner, t2];
    Ok(vec![name, s.scene.name_of(t2)])
}

fn surface_material(s: &Session, shape: NodeId) -> Option<NodeId> {
    match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md.material,
        _ => None,
    }
}

/// `attachSurface [-dir u|v] surfaceA surfaceB`: Edit NURBS > Attach Surfaces.
pub fn attach_surface(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["direction", "dir"], arity: 1 }, FlagSpec { names: &["keepOriginals", "ko"], arity: 1 }])?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    if shapes.len() != 2 {
        return Err("Select two NURBS surfaces to attach".to_string());
    }
    let (a, b) = (surface_data(s, shapes[0])?, surface_data(s, shapes[1])?);
    let in_u = p.string("direction").map(|d| d.to_lowercase() != "v").unwrap_or(true);
    // express both in the first surface's space
    let wa = s.scene.node(shapes[0]).and_then(|n| n.parent).map(|t| s.scene.world_matrix(t)).ok_or("no transform")?;
    let wb = s.scene.node(shapes[1]).and_then(|n| n.parent).map(|t| s.scene.world_matrix(t)).ok_or("no transform")?;
    let rel = wa.inverse() * wb;
    let mut b2 = b.clone();
    b2.transform(|q| rel.transform_point(q));
    // pick the end that is closer: flip the second surface when it is nearer by its far end
    let joined = a.attach(&b2, in_u)?;
    let owner = s.scene.node(shapes[0]).and_then(|n| n.parent).unwrap();
    let other = s.scene.node(shapes[1]).and_then(|n| n.parent).unwrap();
    replace_surface(s, shapes[0], joined, true);
    if !p.num("keepOriginals")?.map(|v| v > 0.5).unwrap_or(false) {
        let _ = s.scene.delete_node(other);
    }
    s.scene.selection = vec![owner];
    Ok(vec![s.scene.name_of(owner)])
}

/// `moveSeam [-u|-v] -isoparm K surface`: Edit NURBS > Move Seam; the K-th span boundary becomes the seam.
pub fn move_seam(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["isoparm", "i"], arity: 1 }, FlagSpec { names: &["u"], arity: 0 }, FlagSpec { names: &["v"], arity: 0 }])?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    let sh = *shapes.first().ok_or("Select a NURBS surface")?;
    let sf = surface_data(s, sh)?;
    let in_u = if p.has("u") { true } else if p.has("v") { false } else { sf.u.periodic };
    let k = p.num("isoparm")?.ok_or("Usage: moveSeam [-u|-v] -isoparm K surface")?.max(0.0) as usize;
    let ns = sf.move_seam(in_u, k % (if in_u { sf.u.n } else { sf.v.n }))?;
    // a generated surface keeps its history only if the structure is unchanged: moving the seam is a re-indexing
    replace_surface(s, sh, ns, true);
    Ok(vec![format!("The seam moved by {} isoparm(s)", k)])
}

/// `reverseSurface [-u|-v] surface`: Edit NURBS > Reverse Surface Direction; flips the U (default) or V parameter direction, so the normals turn around.
pub fn reverse_surface(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["u"], arity: 0 }, FlagSpec { names: &["v"], arity: 0 }])?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    let sh = *shapes.first().ok_or("Select a NURBS surface")?;
    let mut sf = surface_data(s, sh)?;
    let in_u = !p.has("v");
    let (nu, nv) = (sf.u.n, sf.v.n);
    let old = sf.cvs.clone();
    let flip = |d: &mut crate::geo::nurbs::Dir| {
        let (a, b) = (d.knots[0], d.knots[d.knots.len() - 1]);
        d.knots = d.knots.iter().rev().map(|k| a + b - k).collect();
    };
    if in_u {
        for i in 0..nu {
            for j in 0..nv {
                sf.cvs[i * nv + j] = old[(nu - 1 - i) * nv + j];
            }
        }
        flip(&mut sf.u);
    } else {
        for i in 0..nu {
            for j in 0..nv {
                sf.cvs[i * nv + j] = old[i * nv + (nv - 1 - j)];
            }
        }
        flip(&mut sf.v);
    }
    sf.trims.clear();
    replace_surface(s, sh, sf, true);
    Ok(vec![format!("Reversed the surface in {}", if in_u { "U" } else { "V" })])
}

/// `nurbsToPoly [-s SAMPLES] surface`: Modify > Convert > NURBS to Polygons.
pub fn nurbs_to_poly(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["samples", "s"], arity: 1 }])?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    if shapes.is_empty() {
        return Err("Select a NURBS surface".to_string());
    }
    let per = p.num("samples")?.unwrap_or(4.0).clamp(1.0, 16.0) as usize;
    for sh in &shapes {
        let sf = surface_data(s, *sh)?;
        let mesh = sf.tessellate(per);
        // weld the seam and pole duplicates into a proper polygon mesh
        let poly = crate::geo::edit::Poly::welded(&mesh, 1e-4);
        if let Some(NodeData::Mesh(md)) = s.scene.nodes.get_mut(*sh).map(|n| &mut n.data) {
            md.nurbs = None;
            md.prim = "custom".to_string();
            md.set_poly(&poly);
        }
    }
    Ok(vec![format!("Converted {} surface(s) to polygons", shapes.len())])
}

/// `rebuildCurve -s SPANS [-d DEGREE] curve`: Edit Curves > Rebuild Curve (uniform).
pub fn rebuild_curve(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["spans", "s"], arity: 1 }, FlagSpec { names: &["degree", "d"], arity: 1 }])?;
    let curves = curves_of(s, &names_only(&p))?;
    let c = *curves.first().ok_or("Select a curve")?;
    let degree = p.num("degree")?.unwrap_or(3.0).round().clamp(1.0, 5.0) as usize;
    let spans = p.num("spans")?.unwrap_or(4.0).round().clamp(1.0, 64.0) as usize;
    let cur = match s.scene.node(c).map(|n| &n.data) {
        Some(NodeData::NurbsCurve(cd)) => cd.curve.clone(),
        _ => return Err("Not a curve".to_string()),
    };
    let new = nurbs::rebuild_curve(&cur, spans + degree, degree);
    s.scene.delete_nurbs_history(c);
    if let Some(NodeData::NurbsCurve(cd)) = s.scene.nodes.get_mut(c).map(|n| &mut n.data) {
        cd.curve = new;
    }
    Ok(vec![format!("Rebuilt with {} spans", spans)])
}

/// `closeCurve curve` / `openCurve curve`: Edit Curves > Open/Close Curves (periodic).
pub fn close_curve(s: &mut Session, rest: &[Token], close: bool) -> Out {
    let p = parse_args(rest, &[])?;
    let curves = curves_of(s, &names_only(&p))?;
    let c = *curves.first().ok_or("Select a curve")?;
    s.scene.delete_nurbs_history(c);
    if let Some(NodeData::NurbsCurve(cd)) = s.scene.nodes.get_mut(c).map(|n| &mut n.data) {
        let cur = cd.curve.clone();
        if close == cur.dir.periodic {
            return Err(format!("The curve is already {}", if close { "closed" } else { "open" }));
        }
        cd.curve = if close {
            let mut pts = cur.sample(1);
            pts.pop();
            nurbs::periodic_through(&pts, 3.min(pts.len() - 1))
        } else {
            let pts: Vec<Vec3> = (0..cur.cvs.len()).map(|i| cur.cvs[i]).collect();
            let mut closed = pts.clone();
            closed.push(pts[0]);
            nurbs::interpolate(&closed, 3.min(closed.len() - 1))
        };
    }
    Ok(vec![if close { "Closed" } else { "Opened" }.to_string()])
}

/// `nurbsInfo [object]`: degree, CV counts, spans and parameter ranges.
pub fn nurbs_info(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut out = Vec::new();
    for id in ids {
        for c in s.scene.curve_shapes_under(id) {
            if let Some(NodeData::NurbsCurve(cd)) = s.scene.node(c).map(|n| &n.data) {
                let (a, b) = cd.curve.dir.domain();
                out.push(format!("{}: curve degree {}, {} CVs, {} spans, {}, range {} .. {}", s.scene.name_of(c), cd.curve.dir.degree, cd.curve.cvs.len(), cd.curve.dir.spans(), if cd.curve.dir.periodic { "periodic" } else { "open" }, fmt_f(a), fmt_f(b)));
            }
        }
        for sh in s.scene.surface_shapes_under(id) {
            let sf = surface_data(s, sh)?;
            let (ua, ub) = sf.u.domain();
            let (va, vb) = sf.v.domain();
            out.push(format!(
                "{}: surface {}x{} CVs, degree {}x{}, spans {}x{}, u {}{} .. {}, v {}{} .. {}",
                s.scene.name_of(sh), sf.u.n, sf.v.n, sf.u.degree, sf.v.degree, sf.u.spans(), sf.v.spans(),
                if sf.u.periodic { "periodic " } else { "" }, fmt_f(ua), fmt_f(ub), if sf.v.periodic { "periodic " } else { "" }, fmt_f(va), fmt_f(vb)
            ));
        }
    }
    if out.is_empty() {
        return Err("No NURBS curve or surface selected".to_string());
    }
    Ok(out)
}

/// Rewrite `name.cv[a:b][c:d]` arguments into `name.vtx[...]` so the component tools treat control points as vertices.
pub fn expand_cv_args(s: &Session, args: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for a in args {
        // lattice points: name.pt[i][j][k]
        if let Some(pp) = a.find(".pt[") {
            let name = &a[..pp];
            let id = s.scene.find(name).ok_or_else(|| format!("No object matches name: {}", name))?;
            let lat = *s.scene.lattice_shapes_under(id).first().ok_or_else(|| format!("{} is not a lattice", name))?;
            let ld = match s.scene.node(lat).map(|n| &n.data) {
                Some(NodeData::Lattice(d)) => (**d).clone(),
                _ => unreachable!(),
            };
            let mut groups: Vec<String> = Vec::new();
            let mut rest = &a[pp + 3..];
            while let Some(stripped) = rest.strip_prefix('[') {
                let end = stripped.find(']').ok_or_else(|| format!("Bad component name: {}", a))?;
                groups.push(stripped[..end].to_string());
                rest = &stripped[end + 1..];
            }
            if groups.len() != 3 {
                return Err(format!("A lattice point needs three indices: {}.pt[s][t][u] ({}x{}x{} points)", name, ld.divs[0], ld.divs[1], ld.divs[2]));
            }
            let range = |g: &str, n: usize| -> Result<Vec<usize>, String> {
                if g == "*" {
                    return Ok((0..n).collect());
                }
                let r: Vec<usize> = match g.split_once(':') {
                    Some((x, y)) => {
                        let (x, y) = (x.trim().parse::<usize>().map_err(|_| format!("Bad component name: {}", a))?, y.trim().parse::<usize>().map_err(|_| format!("Bad component name: {}", a))?);
                        (x.min(y)..=x.max(y)).collect()
                    }
                    None => vec![g.trim().parse::<usize>().map_err(|_| format!("Bad component name: {}", a))?],
                };
                if r.iter().any(|i| *i >= n) {
                    return Err(format!("Lattice point index out of range in {}", a));
                }
                Ok(r)
            };
            let (ri, rj, rk) = (range(&groups[0], ld.divs[0])?, range(&groups[1], ld.divs[1])?, range(&groups[2], ld.divs[2])?);
            let mut ids: Vec<String> = Vec::new();
            for i in &ri {
                for j in &rj {
                    for k in &rk {
                        ids.push(ld.index(*i, *j, *k).to_string());
                    }
                }
            }
            out.push(format!("{}.vtx[{}]", name, ids.join(",")));
            continue;
        }
        let Some(pos) = a.find(".cv[") else {
            out.push(a.clone());
            continue;
        };
        let name = &a[..pos];
        let id = s.scene.find(name).ok_or_else(|| format!("No object matches name: {}", name))?;
        // brackets after ".cv"
        let mut groups: Vec<String> = Vec::new();
        let mut rest = &a[pos + 3..];
        while let Some(stripped) = rest.strip_prefix('[') {
            let end = stripped.find(']').ok_or_else(|| format!("Bad component name: {}", a))?;
            groups.push(stripped[..end].to_string());
            rest = &stripped[end + 1..];
        }
        let range = |g: &str, n: usize| -> Result<Vec<usize>, String> {
            if g == "*" {
                return Ok((0..n).collect());
            }
            let mut v = Vec::new();
            for part in g.split(',') {
                let part = part.trim();
                let r: Vec<usize> = match part.split_once(':') {
                    Some((x, y)) => {
                        let (x, y) = (x.trim().parse::<usize>().map_err(|_| format!("Bad component name: {}", a))?, y.trim().parse::<usize>().map_err(|_| format!("Bad component name: {}", a))?);
                        (x.min(y)..=x.max(y)).collect()
                    }
                    None => vec![part.parse::<usize>().map_err(|_| format!("Bad component name: {}", a))?],
                };
                v.extend(r);
            }
            if v.iter().any(|i| *i >= n) {
                return Err(format!("CV index out of range in {} (0..{})", a, n - 1));
            }
            Ok(v)
        };
        let ids: Vec<usize> = if let Some(sh) = s.scene.surface_shapes_under(id).first() {
            let sf = surface_data(s, *sh)?;
            if groups.len() != 2 {
                return Err(format!("A surface CV needs two indices: {}.cv[u][v] ({}x{} CVs)", name, sf.u.n, sf.v.n));
            }
            let (ri, rj) = (range(&groups[0], sf.u.n)?, range(&groups[1], sf.v.n)?);
            ri.iter().flat_map(|i| rj.iter().map(move |j| i * sf.v.n + j)).collect()
        } else if let Some(c) = s.scene.curve_shapes_under(id).first() {
            let n = match s.scene.node(*c).map(|x| &x.data) {
                Some(NodeData::NurbsCurve(cd)) => cd.curve.cvs.len(),
                _ => 0,
            };
            if groups.len() != 1 {
                return Err(format!("A curve CV needs one index: {}.cv[i] ({} CVs)", name, n));
            }
            range(&groups[0], n)?
        } else {
            return Err(format!("{} is not a NURBS curve or surface", name));
        };
        let list: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
        out.push(format!("{}.vtx[{}]", name, list.join(",")));
    }
    Ok(out)
}

const PROJ_SPEC: &[FlagSpec] = &[FlagSpec { names: &["direction", "dir"], arity: 3 }, FlagSpec { names: &["name", "n"], arity: 1 }];

/// `projectCurve [-dir X Y Z] curve surface`: Edit Curves > Project Curve on Surface. The curve is projected
/// along the direction (default straight down) onto the surface; the result is a curve on that surface.
pub fn project_curve(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, PROJ_SPEC)?;
    let names = names_only(&p);
    let ids = resolve_targets(s, &names)?;
    let mut curve_shape = None;
    let mut surf_shape = None;
    for id in ids {
        curve_shape = curve_shape.or_else(|| s.scene.curve_shapes_under(id).first().copied());
        surf_shape = surf_shape.or_else(|| s.scene.surface_shapes_under(id).first().copied());
    }
    let (cs, ss) = (curve_shape.ok_or("Select a curve and a NURBS surface")?, surf_shape.ok_or("Select a curve and a NURBS surface")?);
    let dir = match p.get("direction") {
        Some(d) => Vec3::new(d[0].as_f32()?, d[1].as_f32()?, d[2].as_f32()?),
        None => Vec3::new(0.0, -1.0, 0.0),
    };
    if dir.length_sq() < 1e-12 {
        return Err("The projection direction is zero".to_string());
    }
    let curve = world_curve(s, cs).ok_or("Not a curve")?;
    let surf = surface_data(s, ss)?;
    let owner = s.scene.node(ss).and_then(|n| n.parent).ok_or("Surface has no transform")?;
    let w = s.scene.world_matrix(owner);
    let inv = w.inverse();
    let ldir = inv.transform_dir(dir);
    let mut uv: Vec<(f32, f32)> = Vec::new();
    let mut pts: Vec<Vec3> = Vec::new();
    for q in curve.sample(8) {
        if let Some((u, v, _)) = surf.project_point(inv.transform_point(q), ldir) {
            if uv.last().map(|l| (l.0 - u).abs() + (l.1 - v).abs() > 1e-4).unwrap_or(true) {
                uv.push((u, v));
                pts.push(w.transform_point(surf.point(u, v)));
            }
        }
    }
    if uv.len() < 4 {
        return Err("The curve does not project onto the surface along that direction".to_string());
    }
    let closed = curve.dir.periodic || curve.start().distance(curve.end()) < 1e-3;
    if closed {
        // the loop closes on itself
        uv.pop();
        pts.pop();
    }
    // a smooth curve through at most 24 of the projected points
    let step = (pts.len() as f32 / 24.0).ceil().max(1.0) as usize;
    let fit: Vec<Vec3> = pts.iter().step_by(step).copied().collect();
    let c = if closed && fit.len() > 3 { nurbs::periodic_through(&fit, 3) } else if fit.len() > 3 { nurbs::interpolate(&fit, 3) } else { Curve::open(fit, 1) };
    let t = s.scene.create_nurbs_curve(p.string("name").as_deref().or(Some("projectionCurve")), c, None)?;
    let sh = shape_of(s, t);
    if let Some(NodeData::NurbsCurve(cd)) = s.scene.nodes.get_mut(sh).map(|n| &mut n.data) {
        cd.on_surface = Some(crate::dag::nodes::OnSurface { surface: ss, name: String::new(), uv: if closed { uv } else { uv } });
    }
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

/// The loops a curve-on-surface cuts out of the surface, as region candidates.
fn candidate_loops(surf: &Surface, uv: &[(f32, f32)], closed: bool) -> Result<Vec<Vec<(f32, f32)>>, String> {
    if closed {
        return Ok(vec![uv.to_vec()]);
    }
    let (ua, ub) = surf.u.domain();
    let (va, vb) = surf.v.domain();
    let (tol_u, tol_v) = ((ub - ua) * 0.03, (vb - va) * 0.03);
    // distance along the rectangle boundary (counter-clockwise from (ua, va)), or None when off the edge
    let on_edge = |p: (f32, f32)| -> Option<f32> {
        let (w, h) = (ub - ua, vb - va);
        if (p.1 - va).abs() < tol_v {
            Some(p.0 - ua)
        } else if (p.0 - ub).abs() < tol_u {
            Some(w + (p.1 - va))
        } else if (p.1 - vb).abs() < tol_v {
            Some(w + h + (ub - p.0))
        } else if (p.0 - ua).abs() < tol_u {
            Some(2.0 * w + h + (vb - p.1))
        } else {
            None
        }
    };
    let (a, b) = (uv[0], *uv.last().unwrap());
    let (sa, sb) = (on_edge(a).ok_or("An open curve must run from one edge of the surface to another")?, on_edge(b).ok_or("An open curve must run from one edge of the surface to another")?);
    let (w, h) = (ub - ua, vb - va);
    let perim = 2.0 * (w + h);
    let at = |d: f32| -> (f32, f32) {
        let d = d.rem_euclid(perim);
        if d < w {
            (ua + d, va)
        } else if d < w + h {
            (ub, va + d - w)
        } else if d < 2.0 * w + h {
            (ub - (d - w - h), vb)
        } else {
            (ua, vb - (d - 2.0 * w - h))
        }
    };
    let corners = [0.0, w, w + h, 2.0 * w + h];
    // walk from b around the boundary to a, both ways
    let walk = |from: f32, to: f32, ccw: bool| -> Vec<(f32, f32)> {
        let mut out = Vec::new();
        let mut cur = from;
        let span = if ccw { (to - from).rem_euclid(perim) } else { (from - to).rem_euclid(perim) };
        for c in corners {
            let d = if ccw { (c - from).rem_euclid(perim) } else { (from - c).rem_euclid(perim) };
            if d > 1e-6 && d < span - 1e-6 {
                out.push((d, at(c)));
            }
        }
        out.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
        let _ = &mut cur;
        out.into_iter().map(|x| x.1).collect()
    };
    let mut l1 = uv.to_vec();
    l1.extend(walk(sb, sa, true));
    let mut l2 = uv.to_vec();
    l2.extend(walk(sb, sa, false));
    Ok(vec![l1, l2])
}

fn loop_area(p: &[(f32, f32)]) -> f32 {
    let n = p.len();
    (0..n).map(|i| p[i].0 * p[(i + 1) % n].1 - p[(i + 1) % n].0 * p[i].1).sum::<f32>().abs() * 0.5
}

/// `trim [-pt U V] [-discard] [-keep] curve ... surface`: Edit NURBS > Trim Tool with curves on the surface.
/// The region containing -pt is kept (or discarded); without -pt a closed curve cuts a hole (-keep keeps the
/// inside) and an open curve keeps the larger part.
pub fn trim(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["point", "pt"], arity: 2 }, FlagSpec { names: &["discard", "d"], arity: 0 }, FlagSpec { names: &["keep", "k"], arity: 0 }])?;
    let names = names_only(&p);
    let ids = resolve_targets(s, &names)?;
    let mut surf_shape = None;
    let mut curves: Vec<NodeId> = Vec::new();
    for id in &ids {
        surf_shape = surf_shape.or_else(|| s.scene.surface_shapes_under(*id).first().copied());
        curves.extend(s.scene.curve_shapes_under(*id));
    }
    let ss = surf_shape.ok_or("Select the curves on the surface and the surface")?;
    let surf = surface_data(s, ss)?;
    let pick = match p.get("point") {
        Some(v) => Some((v[0].as_f32()?, v[1].as_f32()?)),
        None => None,
    };
    let mut added = 0;
    for c in curves {
        let (uv, closed) = match s.scene.node(c).map(|n| &n.data) {
            Some(NodeData::NurbsCurve(cd)) => match &cd.on_surface {
                Some(o) if o.surface == ss => (o.uv.clone(), cd.curve.dir.periodic || cd.curve.start().distance(cd.curve.end()) < 1e-3),
                _ => continue,
            },
            _ => continue,
        };
        let cands = candidate_loops(&surf, &uv, closed)?;
        let trim = if closed {
            let inside_pick = pick.map(|q| nurbs::inside_loop(&cands[0], q.0, q.1));
            let keep_inside = match inside_pick {
                Some(ins) => ins != p.has("discard"),
                None => p.has("keep"),
            };
            nurbs::Trim { pts: cands[0].clone(), keep: keep_inside }
        } else {
            // the region to keep: the one containing the pick (or the larger one)
            let mut idx = if loop_area(&cands[0]) >= loop_area(&cands[1]) { 0 } else { 1 };
            if let Some(q) = pick {
                if let Some(i) = cands.iter().position(|l| nurbs::inside_loop(l, q.0, q.1)) {
                    idx = if p.has("discard") { 1 - i } else { i };
                }
            }
            nurbs::Trim { pts: cands[idx].clone(), keep: true }
        };
        if let Some(NodeData::Mesh(md)) = s.scene.nodes.get_mut(ss).map(|n| &mut n.data) {
            if let Some(ns) = md.nurbs.as_mut() {
                ns.surf.trims.push(trim);
                added += 1;
            }
        }
    }
    if added == 0 {
        return Err("No curve on this surface is selected (use projectCurve first)".to_string());
    }
    s.scene.retessellate(ss);
    Ok(vec![format!("Trimmed with {} curve(s)", added)])
}

/// `untrim surface`: Edit NURBS > Untrim Surfaces.
pub fn untrim(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    let mut n = 0;
    for sh in shapes {
        if let Some(NodeData::Mesh(md)) = s.scene.nodes.get_mut(sh).map(|x| &mut x.data) {
            if let Some(ns) = md.nurbs.as_mut() {
                n += ns.surf.trims.len();
                ns.surf.trims.clear();
            }
        }
        s.scene.retessellate(sh);
    }
    Ok(vec![format!("Removed {} trim(s)", n)])
}

/// `planarSurface curve`: Surfaces > Planar, a trimmed flat surface inside a closed planar curve.
pub fn planar_surface(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["name", "n"], arity: 1 }])?;
    let curves = curves_of(s, &names_only(&p))?;
    let c = *curves.first().ok_or("Select a closed planar curve")?;
    let cur = world_curve(s, c).ok_or("Not a curve")?;
    let mut pts = cur.sample(6);
    if cur.start().distance(cur.end()) < 1e-3 || cur.dir.periodic {
        pts.pop();
    } else {
        return Err("The curve must be closed (closeCurve) to make a planar surface".to_string());
    }
    let surf = Surface::planar_from(&pts).ok_or("The curve is not planar")?;
    // the curve must lie in one plane
    let n = {
        let (a, b, d) = (pts[0], pts[pts.len() / 3], pts[2 * pts.len() / 3]);
        (b - a).cross(d - a).normalized()
    };
    if pts.iter().any(|q| (*q - pts[0]).dot(n).abs() > 0.02 * cur.length().max(1.0)) {
        return Err("The curve is not planar".to_string());
    }
    let t = s.scene.create_nurbs_surface(p.string("name").as_deref().or(Some("planarTrimmedSurface")), None, Vec::new(), Some(surf))?;
    finish_surface(s, t)
}

/// `rebuildSurface [-su N] [-sv N] [-du D] [-dv D] surface`: Edit NURBS > Rebuild Surfaces.
pub fn rebuild_surface(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["spansU", "su"], arity: 1 }, FlagSpec { names: &["spansV", "sv"], arity: 1 }, FlagSpec { names: &["degreeU", "du"], arity: 1 }, FlagSpec { names: &["degreeV", "dv"], arity: 1 }])?;
    let shapes = surfaces_of(s, &names_only(&p))?;
    if shapes.is_empty() {
        return Err("Select a NURBS surface".to_string());
    }
    let mut n = 0;
    for sh in shapes {
        let sf = surface_data(s, sh)?;
        let (su, sv) = (p.num("spansU")?.map(|v| v as usize).unwrap_or(sf.u.spans()), p.num("spansV")?.map(|v| v as usize).unwrap_or(sf.v.spans()));
        let (du, dv) = (p.num("degreeU")?.map(|v| v as usize).unwrap_or(sf.u.degree).clamp(1, 5), p.num("degreeV")?.map(|v| v as usize).unwrap_or(sf.v.degree).clamp(1, 5));
        if su == 0 || sv == 0 || su > 64 || sv > 64 {
            return Err("Spans must be between 1 and 64".to_string());
        }
        let new = sf.rebuild(su, sv, du, dv);
        replace_surface(s, sh, new, true);
        n += 1;
    }
    Ok(vec![format!("Rebuilt {} surface(s)", n)])
}

#[allow(dead_code)]
fn _unused(_: CurveData, _: MeshData, _: Dir) {}

const FILLET_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["radius", "r"], arity: 1 },
    FlagSpec { names: &["pick1", "p1"], arity: 1 },
    FlagSpec { names: &["pick2", "p2"], arity: 1 },
    FlagSpec { names: &["trim", "t"], arity: 1 },
    FlagSpec { names: &["join", "j"], arity: 1 },
    FlagSpec { names: &["name", "n"], arity: 1 },
];

/// Closest points of two curves: (parameter on a, parameter on b, distance).
fn curve_crossing(a: &Curve, b: &Curve) -> (f32, f32, f32) {
    let (a0, a1) = a.dir.domain();
    let (b0, b1) = b.dir.domain();
    let n = 80;
    let mut best = (a0, b0, f32::MAX);
    for i in 0..=n {
        let ta = a0 + (a1 - a0) * i as f32 / n as f32;
        let pa = a.point(ta);
        for j in 0..=n {
            let tb = b0 + (b1 - b0) * j as f32 / n as f32;
            let d = pa.distance(b.point(tb));
            if d < best.2 {
                best = (ta, tb, d);
            }
        }
    }
    // refine by shrinking windows around the best pair
    let (mut ra, mut rb) = ((a1 - a0) / n as f32, (b1 - b0) / n as f32);
    for _ in 0..30 {
        let (ca, cb) = (best.0, best.1);
        for i in -4i32..=4 {
            for j in -4i32..=4 {
                let ta = (ca + ra * i as f32 / 4.0).clamp(a0, a1);
                let tb = (cb + rb * j as f32 / 4.0).clamp(b0, b1);
                let d = a.point(ta).distance(b.point(tb));
                if d < best.2 {
                    best = (ta, tb, d);
                }
            }
        }
        ra *= 0.6;
        rb *= 0.6;
    }
    best
}

/// The parameter at arc length `len` from `t` toward the larger (`forward`) or smaller parameters.
fn walk_curve(c: &Curve, t: f32, len: f32, forward: bool) -> f32 {
    let (d0, d1) = c.dir.domain();
    let steps = 400;
    let step = (d1 - d0) / steps as f32 * if forward { 1.0 } else { -1.0 };
    let (mut cur, mut acc) = (t, 0.0f32);
    for _ in 0..steps {
        let next = (cur + step).clamp(d0, d1);
        acc += c.point(cur).distance(c.point(next));
        if acc >= len || next == cur {
            return next;
        }
        cur = next;
    }
    cur
}

/// `filletCurve [-r RADIUS] [-p1 PARAM] [-p2 PARAM] [-trim 0|1] [-join 0|1] curve1 curve2`: Edit Curves > Curve Fillet. Two curves that cross get a
/// circular arc of the radius between them; the picks (curve parameters) say which side of each curve to keep. With Trim the curves end at the
/// arc, with Join everything becomes one curve. The curves should lie in one plane.
pub fn fillet_curve(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, FILLET_SPEC)?;
    let curves = curves_of(s, &names_only(&p))?;
    if curves.len() != 2 {
        return Err("Select the two curves to fillet".to_string());
    }
    let (a, b) = (world_curve(s, curves[0]).ok_or("Not a curve")?, world_curve(s, curves[1]).ok_or("Not a curve")?);
    let radius = p.num("radius")?.unwrap_or(1.0);
    if radius <= 0.0 {
        return Err("The radius must be above zero".to_string());
    }
    let (ta, tb, dist) = curve_crossing(&a, &b);
    let size = a.length().max(b.length());
    if dist > size * 0.02 + 1e-3 {
        return Err("The curves do not cross: bring them together first".to_string());
    }
    let pt = (a.point(ta) + b.point(tb)) * 0.5;
    // the side of each curve that stays: toward the pick, else toward the longer part
    let side = |c: &Curve, t: f32, pick: Option<f32>| -> bool {
        let (d0, d1) = c.dir.domain();
        match pick {
            Some(k) => k > t,
            None => (d1 - t) >= (t - d0),
        }
    };
    let (fa, fb) = (side(&a, ta, p.num("pick1")?), side(&b, tb, p.num("pick2")?));
    let dir = |c: &Curve, t: f32, fwd: bool| -> Vec3 {
        let tg = c.tangent(t).normalized();
        if fwd {
            tg
        } else {
            -tg
        }
    };
    let (d1, d2) = (dir(&a, ta, fa), dir(&b, tb, fb));
    let cosang = d1.dot(d2).clamp(-1.0, 1.0);
    let theta = cosang.acos();
    if theta < 0.05 || theta > std::f32::consts::PI - 0.05 {
        return Err("The curves meet at a straight or hairpin angle: no fillet fits".to_string());
    }
    let tlen = radius / (theta * 0.5).tan();
    let qa = walk_curve(&a, ta, tlen, fa);
    let qb = walk_curve(&b, tb, tlen, fb);
    let (q1, q2) = (a.point(qa), b.point(qb));
    // the arc: center along the bisector, through the two tangent points
    let bis = (d1 + d2).normalized();
    let center = pt + bis * (radius / (theta * 0.5).sin());
    let (r1, r2) = ((q1 - center).normalized(), (q2 - center).normalized());
    let axis = r1.cross(r2).normalized();
    let ang = r1.dot(r2).clamp(-1.0, 1.0).acos();
    let mut pts = Vec::new();
    for k in 0..=6 {
        let f = ang * k as f32 / 6.0;
        // rotate r1 about the axis by f
        let rot = r1 * f.cos() + axis.cross(r1) * f.sin() + axis * axis.dot(r1) * (1.0 - f.cos());
        pts.push(center + rot * radius);
    }
    pts[0] = q1;
    pts[6] = q2;
    let arc = nurbs::interpolate(&pts, 3);
    let trim = p.num("trim")?.map(|v| v > 0.5).unwrap_or(true);
    let join = p.num("join")?.map(|v| v > 0.5).unwrap_or(true);
    let name = p.string("name");
    let parents: Vec<NodeId> = curves.iter().filter_map(|c| s.scene.node(*c).and_then(|n| n.parent)).collect();
    let mut out = Vec::new();
    if !trim {
        let t = s.scene.create_nurbs_curve(name.as_deref(), arc, None)?;
        s.scene.selection = vec![t];
        return Ok(vec![s.scene.name_of(t)]);
    }
    // keep the far sides: the piece of each curve that lies beyond its tangent point
    let keep = |c: &Curve, q: f32, fwd: bool| -> Result<Curve, String> {
        let (l, r) = c.split(q)?;
        Ok(if fwd { r } else { l })
    };
    let (mut ka, mut kb) = (keep(&a, qa, fa)?, keep(&b, qb, fb)?);
    if join {
        // ka ends or starts at q1: orient so that ka ... arc ... kb reads in one direction
        if ka.end().distance(q1) > ka.start().distance(q1) {
            ka.reverse();
        }
        if kb.start().distance(q2) > kb.end().distance(q2) {
            kb.reverse();
        }
        let joined = ka.attach(&arc).attach(&kb);
        let t = s.scene.create_nurbs_curve(name.as_deref(), joined, None)?;
        out.push(s.scene.name_of(t));
        s.scene.selection = vec![t];
    } else {
        let (n1, n2, n3) = (s.scene.create_nurbs_curve(None, ka, None)?, s.scene.create_nurbs_curve(None, arc, None)?, s.scene.create_nurbs_curve(None, kb, None)?);
        out.extend([n1, n2, n3].iter().map(|t| s.scene.name_of(*t)));
        s.scene.selection = vec![n2];
    }
    for t in parents {
        let _ = s.scene.delete_node(t);
    }
    Ok(out)
}
