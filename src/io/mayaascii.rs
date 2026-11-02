//! Maya ASCII (`.ma`) import: the subset needed to open the scenes the lessons start from.
//!
//! Carried over: the transform hierarchy (translate, rotate, scale, pivots, joint orient on static
//! joints), joints, polygon meshes (explicit `.vt` / `.ed` / `.fc` geometry or the poly primitive
//! generators), lambert / blinn / phong colors with file textures, and animation curves on the
//! transform channels. Not carried over: construction history modifiers, NURBS, deformers, shading
//! group face assignments, cameras and lights, rigid bodies, particles.

use super::{IoCurve, IoKind, IoMaterial, IoNode, IoScene};
use crate::anim::Interp;
use crate::geo::Mesh;
use crate::math::{Mat4, Quat, Vec2, Vec3};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Str(String),
    Word(String),
}

impl Tok {
    fn text(&self) -> &str {
        match self {
            Tok::Str(s) | Tok::Word(s) => s,
        }
    }
}

/// Split into statements (`;` outside quotes) of tokens; `//` comments are dropped.
fn statements(src: &str) -> Vec<Vec<Tok>> {
    let mut out: Vec<Vec<Tok>> = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    let mut chars = src.chars().peekable();
    let mut word = String::new();
    let flush = |word: &mut String, cur: &mut Vec<Tok>| {
        if !word.is_empty() {
            cur.push(Tok::Word(std::mem::take(word)));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                flush(&mut word, &mut cur);
                let mut s = String::new();
                while let Some(d) = chars.next() {
                    match d {
                        '\\' => {
                            if let Some(e) = chars.next() {
                                s.push(match e {
                                    'n' => '\n',
                                    't' => '\t',
                                    o => o,
                                });
                            }
                        }
                        '"' => break,
                        o => s.push(o),
                    }
                }
                cur.push(Tok::Str(s));
            }
            '/' if chars.peek() == Some(&'/') => {
                flush(&mut word, &mut cur);
                for d in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
            }
            ';' => {
                flush(&mut word, &mut cur);
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c if c.is_whitespace() => flush(&mut word, &mut cur),
            c => word.push(c),
        }
    }
    flush(&mut word, &mut cur);
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[derive(Default, Debug)]
struct MaNode {
    ty: String,
    name: String,
    /// Full path ("|group1|pCube1").
    path: String,
    parent: Option<String>,
    /// setAttr values by attribute name without the leading dot ("t", "vt", "ktv[0:2]" ...).
    attrs: HashMap<String, Vec<Tok>>,
}

impl MaNode {
    fn nums(&self, key: &str) -> Vec<f32> {
        self.attrs.get(key).map(|v| v.iter().filter_map(|t| t.text().parse::<f32>().ok()).collect()).unwrap_or_default()
    }
    fn vec3(&self, whole: &str, parts: [&str; 3]) -> Option<Vec3> {
        let w = self.nums(whole);
        let mut v = if w.len() >= 3 { Some(Vec3::new(w[0], w[1], w[2])) } else { None };
        for (i, p) in parts.iter().enumerate() {
            if let Some(x) = self.nums(p).first() {
                let mut c = v.unwrap_or(Vec3::ZERO);
                match i {
                    0 => c.x = *x,
                    1 => c.y = *x,
                    _ => c.z = *x,
                }
                v = Some(c);
            }
        }
        v
    }
    fn flag(&self, key: &str, default: f32) -> f32 {
        self.nums(key).first().copied().unwrap_or(default)
    }
    fn string(&self, key: &str) -> Option<String> {
        self.attrs.get(key).and_then(|v| v.iter().find_map(|t| if let Tok::Str(s) = t { Some(s.clone()) } else { None }))
    }
}

/// The values of a setAttr without its flags (`-type \"float3\"`, `-ch 24`, ...).
fn strip_flags(toks: &[Tok]) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if let Tok::Word(w) = &toks[i] {
            if w.starts_with('-') && w.len() > 1 && !w[1..].starts_with(|c: char| c.is_ascii_digit() || c == '.') {
                i += if matches!(w.as_str(), "-av" | "-ca" | "-cap" | "-un" | "-ni") { 1 } else { 2 };
                continue;
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

/// Attribute name of a setAttr / connectAttr path: ".vt[0:7]" -> "vt[0:7]", "node.attr" -> ("node", "attr").
fn split_plug(p: &str) -> (Option<&str>, &str) {
    match p.rfind('.') {
        Some(0) => (None, &p[1..]),
        Some(i) => {
            // node names may not contain dots; the first dot after the node ends it
            let first = p.find('.').unwrap_or(i);
            (Some(&p[..first]), &p[first + 1..])
        }
        None => (None, p),
    }
}

fn attr_key(a: &str) -> String {
    // "vt[0:7]" and "vt" both address the vertex list; array ranges are not needed to read the data in order
    a.split('[').next().unwrap_or(a).to_string()
}

pub fn read(path: &str) -> Result<IoScene, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read '{}': {}", path, e))?;
    let text = String::from_utf8_lossy(&bytes);
    if bytes.starts_with(b"FOR4") || bytes.starts_with(b"FOR8") {
        return Err("This is a Maya binary (.mb) file: in Maya, File > Save Scene As > Maya ASCII, then import that".to_string());
    }
    parse(&text)
}

struct Scene {
    nodes: Vec<MaNode>,
    by_key: HashMap<String, usize>,
}

impl Scene {
    fn find(&self, name: &str) -> Option<usize> {
        let n = name.trim_start_matches('|');
        self.by_key.get(name).or_else(|| self.by_key.get(n)).copied().or_else(|| {
            let short = n.rsplit('|').next().unwrap_or(n);
            self.by_key.get(short).copied()
        })
    }
}

pub fn parse(text: &str) -> Result<IoScene, String> {
    if !text.contains("createNode") {
        return Err("This does not look like a Maya ASCII scene (no createNode commands)".to_string());
    }
    let mut sc = Scene { nodes: Vec::new(), by_key: HashMap::new() };
    let mut current: Option<usize> = None;
    // connections: (source node, source attr, dest node, dest attr)
    let mut conns: Vec<(String, String, String, String)> = Vec::new();
    let mut fps = 24.0f32;
    let mut angle_radians = false;
    let mut warnings: Vec<String> = Vec::new();
    for st in statements(text) {
        let cmd = st[0].text();
        match cmd {
            "currentUnit" => {
                let mut i = 1;
                while i + 1 < st.len() {
                    match (st[i].text(), st[i + 1].text()) {
                        ("-t", u) | ("-time", u) => {
                            fps = match u {
                                "game" => 15.0,
                                "film" => 24.0,
                                "pal" => 25.0,
                                "ntsc" => 30.0,
                                "show" => 48.0,
                                "palf" => 50.0,
                                "ntscf" => 60.0,
                                _ => fps,
                            }
                        }
                        ("-a", u) | ("-angle", u) => angle_radians = u == "rad" || u == "radian",
                        _ => {}
                    }
                    i += 2;
                }
            }
            "createNode" => {
                let ty = st.get(1).map(|t| t.text().to_string()).unwrap_or_default();
                let mut name = String::new();
                let mut parent: Option<String> = None;
                let mut i = 2;
                while i < st.len() {
                    match st[i].text() {
                        "-n" | "-name" => {
                            name = st.get(i + 1).map(|t| t.text().to_string()).unwrap_or_default();
                            i += 2;
                        }
                        "-p" | "-parent" => {
                            parent = st.get(i + 1).map(|t| t.text().to_string());
                            i += 2;
                        }
                        "-s" | "-shared" | "-ss" => i += 1,
                        _ => i += 1,
                    }
                }
                if name.is_empty() {
                    name = format!("{}1", ty);
                }
                let name = name.trim_start_matches('|').to_string();
                let parent_path = parent.as_ref().and_then(|p| sc.find(p)).map(|i| sc.nodes[i].path.clone());
                let path = match &parent_path {
                    Some(pp) => format!("{}|{}", pp, name),
                    None => format!("|{}", name),
                };
                let idx = sc.nodes.len();
                sc.nodes.push(MaNode { ty, name: name.clone(), path: path.clone(), parent: parent_path, attrs: HashMap::new() });
                sc.by_key.insert(path.trim_start_matches('|').to_string(), idx);
                sc.by_key.entry(name).or_insert(idx);
                current = Some(idx);
            }
            "select" => {
                // "select -ne name" makes a node the target of the following setAttr commands
                if let Some(t) = st.iter().rev().find(|t| matches!(t, Tok::Str(_))) {
                    if let Some(i) = sc.find(t.text()) {
                        current = Some(i);
                    }
                }
            }
            "setAttr" => {
                // flags with a value: -s N, -ch N, -k on, -l on, -cb on, -type "x"
                let mut i = 1;
                let mut path: Option<String> = None;
                while i < st.len() {
                    let t = &st[i];
                    if let Tok::Word(w) = t {
                        if w.starts_with('-') && w.len() > 1 && !w[1..].starts_with(|c: char| c.is_ascii_digit() || c == '.') {
                            i += if matches!(w.as_str(), "-av" | "-ca" | "-cap" | "-un" | "-ni") { 1 } else { 2 };
                            continue;
                        }
                    }
                    if let Tok::Str(s) = t {
                        path = Some(s.clone());
                        i += 1;
                        break;
                    }
                    break;
                }
                let Some(path) = path else { continue };
                let (node, attr) = split_plug(&path);
                let target = match node {
                    Some(n) => sc.find(n),
                    None => current,
                };
                if let Some(ti) = target {
                    let vals: Vec<Tok> = strip_flags(&st[i..]);
                    sc.nodes[ti].attrs.entry(attr_key(attr)).or_default().extend(vals);
                    // keep the exact attribute text too: ".uvst[0].uvsp" style paths must not collide
                    if attr.contains('[') {
                        sc.nodes[ti].attrs.entry(attr.to_string()).or_default();
                    }
                }
            }
            "connectAttr" => {
                let a = st.get(1).map(|t| t.text().to_string()).unwrap_or_default();
                let b = st.get(2).map(|t| t.text().to_string()).unwrap_or_default();
                let (an, aa) = split_plug(&a);
                let (bn, ba) = split_plug(&b);
                if let (Some(an), Some(bn)) = (an, bn) {
                    conns.push((an.to_string(), attr_key(aa), bn.to_string(), attr_key(ba)));
                }
            }
            "rename" | "parent" | "reparent" => {
                warnings.push(format!("{} command ignored", cmd));
            }
            _ => {}
        }
    }
    build(&sc, &conns, fps, angle_radians, warnings)
}

fn euler_matrix(r: Vec3, order: i32) -> Mat4 {
    let a = |i: usize, v: f32| {
        let mut ax = Vec3::ZERO;
        match i {
            0 => ax.x = 1.0,
            1 => ax.y = 1.0,
            _ => ax.z = 1.0,
        }
        Quat::from_axis_angle(ax, v.to_radians())
    };
    let seq: [usize; 3] = match order {
        1 => [1, 2, 0],
        2 => [2, 0, 1],
        3 => [0, 2, 1],
        4 => [1, 0, 2],
        5 => [2, 1, 0],
        _ => [0, 1, 2],
    };
    let v = [r.x, r.y, r.z];
    // Maya rotate orders: xyz, yzx, zxy, xzy, yxz, zyx; the first axis is applied first
    Mat4::from_quat(a(seq[2], v[seq[2]]) * a(seq[1], v[seq[1]]) * a(seq[0], v[seq[0]]))
}

fn build(sc: &Scene, conns: &[(String, String, String, String)], fps: f32, angle_radians: bool, mut warnings: Vec<String>) -> Result<IoScene, String> {
    let mut io = IoScene { fps, ..Default::default() };
    let deg = |v: f32| if angle_radians { v.to_degrees() } else { v };

    // materials: shader -> index, with file textures found through connections to the color
    let mut mat_of_shader: HashMap<usize, usize> = HashMap::new();
    for (i, n) in sc.nodes.iter().enumerate() {
        if !matches!(n.ty.as_str(), "lambert" | "blinn" | "phong" | "phongE") || n.name == "lambert1" {
            continue;
        }
        let c = n.vec3("c", ["cr", "cg", "cb"]).unwrap_or(Vec3::new(0.5, 0.5, 0.5));
        let mut texture = None;
        for (an, aa, bn, ba) in conns {
            if sc.find(bn) == Some(i) && (ba == "c" || ba == "cr") && aa == "oc" {
                if let Some(f) = sc.find(an).map(|f| &sc.nodes[f]) {
                    if f.ty == "file" {
                        texture = f.string("ftn");
                    }
                }
            }
        }
        io.materials.push(IoMaterial { name: n.name.clone(), color: [c.x.clamp(0.0, 1.0), c.y.clamp(0.0, 1.0), c.z.clamp(0.0, 1.0)], texture });
        mat_of_shader.insert(i, io.materials.len() - 1);
    }
    // shape -> shader through the shading group
    let mut sg_shader: HashMap<usize, usize> = HashMap::new();
    for (an, aa, bn, ba) in conns {
        if aa == "oc" && ba == "ss" {
            if let (Some(sh), Some(sg)) = (sc.find(an), sc.find(bn)) {
                sg_shader.insert(sg, sh);
            }
        }
    }
    let mut shape_mat: HashMap<usize, usize> = HashMap::new();
    for (an, aa, bn, ba) in conns {
        if (aa == "iog" || aa == "ogs") && ba == "dsm" {
            if let (Some(shape), Some(sg)) = (sc.find(an), sc.find(bn)) {
                if let Some(m) = sg_shader.get(&sg).and_then(|s| mat_of_shader.get(s)) {
                    shape_mat.insert(shape, *m);
                }
            }
        }
    }
    // poly generators feeding meshes: shape <- generator
    let mut generator_of: HashMap<usize, usize> = HashMap::new();
    for (an, aa, bn, ba) in conns {
        if aa == "out" && (ba == "i" || ba == "inMesh") {
            if let (Some(g), Some(shape)) = (sc.find(an), sc.find(bn)) {
                generator_of.insert(shape, g);
            }
        }
    }
    // animation: curve -> (node, attr)
    let mut curve_target: Vec<(usize, usize, String)> = Vec::new();
    for (an, aa, bn, ba) in conns {
        if aa == "o" || aa == "output" {
            if let (Some(c), Some(t)) = (sc.find(an), sc.find(bn)) {
                if sc.nodes[c].ty.starts_with("animCurve") {
                    curve_target.push((c, t, ba.clone()));
                }
            }
        }
    }

    // transforms and joints in file order (parents come first in Maya files)
    let mut io_index: HashMap<usize, usize> = HashMap::new();
    for (i, n) in sc.nodes.iter().enumerate() {
        if n.ty != "transform" && n.ty != "joint" && n.ty != "locator" {
            continue;
        }
        if n.ty == "locator" {
            continue;
        }
        let parent = n.parent.as_ref().and_then(|p| sc.find(p)).and_then(|pi| io_index.get(&pi).copied());
        let kind = if n.ty == "joint" { IoKind::Joint } else { IoKind::Group };
        let mut node = IoNode::new(&n.name, parent, kind);
        node.t = n.vec3("t", ["tx", "ty", "tz"]).unwrap_or(Vec3::ZERO);
        let r = n.vec3("r", ["rx", "ry", "rz"]).unwrap_or(Vec3::ZERO);
        node.r = Vec3::new(deg(r.x), deg(r.y), deg(r.z));
        node.s = n.vec3("s", ["sx", "sy", "sz"]).unwrap_or(Vec3::ONE);
        node.rp = n.vec3("rp", ["rpx", "rpy", "rpz"]).unwrap_or(Vec3::ZERO);
        node.sp = n.vec3("sp", ["spx", "spy", "spz"]).unwrap_or(Vec3::ZERO);
        let order = n.flag("ro", 0.0) as i32;
        let jo = n.vec3("jo", ["jox", "joy", "joz"]).map(|v| Vec3::new(deg(v.x), deg(v.y), deg(v.z)));
        let animated = curve_target.iter().any(|(_, t, a)| *t == i && a.starts_with('r'));
        if order != 0 || jo.map(|v| v.length_sq() > 1e-10).unwrap_or(false) {
            if animated {
                warnings.push(format!("{}: rotate order or joint orient ignored because the rotation is animated", n.name));
            } else {
                // fold the rotate order and the joint orient into one XYZ rotation
                let mut m = euler_matrix(node.r, order);
                if let Some(j) = jo {
                    m = euler_matrix(j, 0) * m;
                }
                node.r = m.decompose().1;
            }
        }
        io.nodes.push(node);
        io_index.insert(i, io.nodes.len() - 1);
    }

    // meshes: attach to their transform
    for (i, n) in sc.nodes.iter().enumerate() {
        if n.ty != "mesh" {
            continue;
        }
        let Some(&ti) = n.parent.as_ref().and_then(|p| sc.find(p)).and_then(|pi| io_index.get(&pi)) else { continue };
        if n.flag("io", 0.0) > 0.5 {
            continue; // intermediate (original) shape of a deformed mesh
        }
        let mesh = match mesh_of(sc, i, generator_of.get(&i).copied(), &mut warnings) {
            Some(m) => m,
            None => {
                warnings.push(format!("{}: no geometry found (it comes from construction history this importer cannot evaluate)", n.name));
                continue;
            }
        };
        if io.nodes[ti].kind == IoKind::Mesh {
            continue; // one shape per transform
        }
        io.nodes[ti].kind = IoKind::Mesh;
        io.nodes[ti].mesh = mesh;
        io.nodes[ti].material = shape_mat.get(&i).copied();
    }

    // animation curves
    for (c, t, attr) in &curve_target {
        let (Some(&ni), cn) = (io_index.get(t), &sc.nodes[*c]) else { continue };
        let name = match attr.as_str() {
            "tx" | "translateX" => "translateX",
            "ty" | "translateY" => "translateY",
            "tz" | "translateZ" => "translateZ",
            "rx" | "rotateX" => "rotateX",
            "ry" | "rotateY" => "rotateY",
            "rz" | "rotateZ" => "rotateZ",
            "sx" | "scaleX" => "scaleX",
            "sy" | "scaleY" => "scaleY",
            "sz" | "scaleZ" => "scaleZ",
            other => {
                warnings.push(format!("animation of '{}' is not carried over", other));
                continue;
            }
        };
        let ktv = cn.nums("ktv");
        let kot = cn.nums("kot");
        let rot = name.starts_with("rotate");
        let mut keys: Vec<(f32, f32, Interp)> = Vec::new();
        for (k, pair) in ktv.chunks_exact(2).enumerate() {
            let v = if rot && angle_radians { pair[1].to_degrees() } else { pair[1] };
            let ip = match kot.get(k).copied().unwrap_or(0.0) as i32 {
                2 => Interp::Linear,
                5 => Interp::Step,
                _ => Interp::Bezier,
            };
            keys.push((pair[0], v, ip));
        }
        if !keys.is_empty() {
            io.curves.push(IoCurve { node: ni, attr: name.to_string(), keys });
        }
    }
    warnings.sort();
    warnings.dedup();
    io.warnings = warnings;
    if io.nodes.is_empty() {
        return Err("The Maya file has no transforms or geometry this importer understands".to_string());
    }
    Ok(io)
}

/// Polygon geometry of a mesh node: explicit vertices, edges and faces, else its poly primitive.
fn mesh_of(sc: &Scene, i: usize, generator: Option<usize>, warnings: &mut Vec<String>) -> Option<Mesh> {
    let n = &sc.nodes[i];
    let vt = n.nums("vt");
    if vt.len() >= 9 {
        let pos: Vec<Vec3> = vt.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
        let ed: Vec<(i64, i64)> = n.nums("ed").chunks_exact(3).map(|c| (c[0] as i64, c[1] as i64)).collect();
        let fc: Vec<String> = n.attrs.get("fc").map(|v| v.iter().map(|t| t.text().to_string()).collect()).unwrap_or_default();
        let mut mesh = Mesh::new();
        for p in &pos {
            mesh.add_vertex(*p, Vec3::UP, Vec2::new(0.0, 0.0));
        }
        let mut k = 0;
        // faces: f N e0 e1 ... then optional uv / color / hole groups
        let num = |s: &String| s.parse::<i64>().ok();
        while k < fc.len() {
            match fc[k].as_str() {
                "f" => {
                    let cnt = fc.get(k + 1).and_then(num).unwrap_or(0) as usize;
                    let mut verts: Vec<u32> = Vec::new();
                    for j in 0..cnt {
                        let Some(e) = fc.get(k + 2 + j).and_then(num) else { break };
                        // a negative edge index is the edge walked backwards
                        let (ei, rev) = if e < 0 { ((-e - 1) as usize, true) } else { (e as usize, false) };
                        let Some(&(a, b)) = ed.get(ei) else { continue };
                        verts.push(if rev { b } else { a } as u32);
                    }
                    if verts.len() >= 3 && verts.iter().all(|v| (*v as usize) < pos.len()) {
                        mesh.add_face(&verts);
                    }
                    k += 2 + cnt;
                }
                "mu" | "mc" => {
                    let cnt = fc.get(k + 2).and_then(num).unwrap_or(0) as usize;
                    k += 3 + cnt;
                }
                "mf" | "h" => {
                    let cnt = fc.get(k + 1).and_then(num).unwrap_or(0) as usize;
                    k += 2 + cnt;
                }
                _ => k += 1,
            }
        }
        if !mesh.faces.is_empty() {
            mesh.recompute_normals();
            return Some(mesh);
        }
        warnings.push(format!("{}: face list could not be read", n.name));
    }
    let g = &sc.nodes[generator?];
    let get = |k: &str, d: f32| g.nums(k).first().copied().unwrap_or(d);
    let (prim, params): (&str, Vec<(&str, f32)>) = match g.ty.as_str() {
        "polyCube" => ("polyCube", vec![("width", get("w", 1.0)), ("height", get("h", 1.0)), ("depth", get("d", 1.0)), ("subdivisions", get("sw", 1.0).max(1.0))]),
        "polySphere" => ("polySphere", vec![("radius", get("r", 1.0)), ("subdivisions", get("sa", 20.0))]),
        "polyCylinder" => ("polyCylinder", vec![("radius", get("r", 1.0)), ("height", get("h", 2.0)), ("subdivisions", get("sa", 20.0))]),
        "polyPlane" => ("polyPlane", vec![("width", get("w", 1.0)), ("height", get("h", 1.0)), ("subdivisions", get("sw", 1.0).max(1.0))]),
        "polyCone" => ("polyCone", vec![("radius", get("r", 1.0)), ("height", get("h", 2.0)), ("subdivisions", get("sa", 20.0))]),
        "polyTorus" => ("polyTorus", vec![("radius", get("r", 1.0)), ("sectionRadius", get("sr", 0.5)), ("subdivisions", get("sa", 20.0)), ("sectionSubdivisions", get("sh", 20.0))]),
        _ => return None,
    };
    let params: Vec<(String, f32)> = params.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    crate::dag::nodes::build_mesh(prim, &params)
}
