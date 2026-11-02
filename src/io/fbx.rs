//! Autodesk FBX 7.x scene import and export (binary and ASCII).
//!
//! Supported: model hierarchy with translate / rotate / scale (pre/post rotation, pivots and
//! rotation orders are folded into the transform), polygon geometry with normals and UVs,
//! materials (diffuse color and texture), LimbNode joints and transform animation curves.
//! Not carried over: skin weights, blend shapes, cameras, lights and per-face materials beyond
//! splitting the mesh by material.

use super::fbx_tree::{self as tree, FNode, Prop};
use super::{sanitize_name, IoCurve, IoKind, IoMaterial, IoNode, IoScene};
use crate::anim::Interp;
use crate::geo::Mesh;
use crate::math::{Mat4, Quat, Vec2, Vec3};
use std::collections::HashMap;

const VERSION: u32 = 7400;
const TICKS_PER_SECOND: f64 = 46_186_158_000.0;

// ====================================================================== writing

fn s(v: &str) -> Prop {
    Prop::Str(v.to_string())
}

fn named(name: &str, class: &str) -> Prop {
    Prop::Str(format!("{}\0\x01{}", name, class))
}

fn p70(rows: Vec<FNode>) -> FNode {
    let mut n = FNode::new("Properties70");
    n.children = rows;
    n
}

fn prow(name: &str, ty: &str, label: &str, flags: &str, vals: Vec<Prop>) -> FNode {
    let mut n = FNode::new("P");
    n.props = vec![s(name), s(ty), s(label), s(flags)];
    n.props.extend(vals);
    n
}

fn vec3_row(name: &str, ty: &str, v: [f64; 3]) -> FNode {
    prow(name, ty, "", "A", v.iter().map(|x| Prop::F64(*x)).collect())
}

fn time_mode(fps: f32) -> (i32, f64) {
    match fps.round() as i32 {
        24 => (11, 24.0),
        25 => (10, 25.0),
        30 => (6, 30.0),
        48 => (5, 48.0),
        50 => (4, 50.0),
        60 => (3, 60.0),
        100 => (2, 100.0),
        120 => (1, 120.0),
        _ => (14, fps as f64),
    }
}

fn ticks(frame: f32, fps: f32) -> i64 {
    ((frame as f64 / fps.max(1.0) as f64) * TICKS_PER_SECOND).round() as i64
}

pub fn build_tree(io: &IoScene) -> Vec<FNode> {
    let fps = if io.fps > 0.0 { io.fps } else { 24.0 };
    let (tmode, custom_fps) = time_mode(fps);
    let mut next_id: i64 = 100_000;
    let mut new_id = || {
        next_id += 1;
        next_id
    };

    let mut header = FNode::new("FBXHeaderExtension");
    header.leaf("FBXHeaderVersion", Prop::I32(1003));
    header.leaf("FBXVersion", Prop::I32(VERSION as i32));
    header.leaf("Creator", s("Aztec 2009"));

    let (mut tmin, mut tmax) = (f32::INFINITY, f32::NEG_INFINITY);
    for c in &io.curves {
        for k in &c.keys {
            tmin = tmin.min(k.0);
            tmax = tmax.max(k.0);
        }
    }
    if !tmin.is_finite() {
        tmin = 0.0;
        tmax = 0.0;
    }

    let mut gs = FNode::new("GlobalSettings");
    gs.leaf("Version", Prop::I32(1000));
    let mut gp = vec![];
    for (n, v) in [("UpAxis", 1), ("UpAxisSign", 1), ("FrontAxis", 2), ("FrontAxisSign", 1), ("CoordAxis", 0), ("CoordAxisSign", 1), ("OriginalUpAxis", 1), ("OriginalUpAxisSign", 1)] {
        gp.push(prow(n, "int", "Integer", "", vec![Prop::I32(v)]));
    }
    gp.push(prow("UnitScaleFactor", "double", "Number", "", vec![Prop::F64(1.0)]));
    gp.push(prow("OriginalUnitScaleFactor", "double", "Number", "", vec![Prop::F64(1.0)]));
    gp.push(prow("TimeMode", "enum", "", "", vec![Prop::I32(tmode)]));
    if tmode == 14 {
        gp.push(prow("CustomFrameRate", "double", "Number", "", vec![Prop::F64(custom_fps)]));
    }
    gp.push(prow("TimeSpanStart", "KTime", "Time", "", vec![Prop::I64(ticks(tmin, fps))]));
    gp.push(prow("TimeSpanStop", "KTime", "Time", "", vec![Prop::I64(ticks(tmax, fps))]));
    gs.add(p70(gp));

    let mut docs = FNode::new("Documents");
    docs.leaf("Count", Prop::I32(1));
    let doc_id = new_id();
    {
        let d = docs.add(FNode::new("Document").with(Prop::I64(doc_id)).with(s("Scene")).with(s("Scene")));
        d.add(p70(vec![prow("SourceObject", "object", "", "", vec![])]));
        d.leaf("RootNode", Prop::I64(0));
    }

    let mut objects = FNode::new("Objects");
    let mut conns = FNode::new("Connections");
    let mut connect = |kind: &str, child: i64, parent: i64, prop: Option<&str>| {
        let mut c = FNode::new("C").with(s(kind)).with(Prop::I64(child)).with(Prop::I64(parent));
        if let Some(p) = prop {
            c.props.push(s(p));
        }
        conns.children.push(c);
    };

    // materials first so models can connect to them
    let mut mat_ids: Vec<i64> = Vec::new();
    let mut n_textures = 0;
    for m in &io.materials {
        let id = new_id();
        mat_ids.push(id);
        let mut mn = FNode::new("Material").with(Prop::I64(id)).with(named(&m.name, "Material")).with(s(""));
        mn.leaf("Version", Prop::I32(102));
        mn.leaf("ShadingModel", s("lambert"));
        mn.leaf("MultiLayer", Prop::I32(0));
        let c = [m.color[0] as f64, m.color[1] as f64, m.color[2] as f64];
        mn.add(p70(vec![
            prow("ShadingModel", "KString", "", "", vec![s("lambert")]),
            vec3_row("DiffuseColor", "Color", c),
            prow("DiffuseFactor", "Number", "", "A", vec![Prop::F64(1.0)]),
            vec3_row("AmbientColor", "Color", [0.0, 0.0, 0.0]),
        ]));
        objects.children.push(mn);
        if let Some(t) = &m.texture {
            let tid = new_id();
            n_textures += 1;
            let fname = std::path::Path::new(t).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| t.clone());
            let mut tn = FNode::new("Texture").with(Prop::I64(tid)).with(named(&fname, "Texture")).with(s(""));
            tn.leaf("Type", s("TextureVideoClip"));
            tn.leaf("Version", Prop::I32(202));
            tn.leaf("TextureName", named(&fname, "Texture"));
            tn.add(p70(vec![prow("UseMaterial", "bool", "", "", vec![Prop::I32(1)])]));
            tn.leaf("Media", named(&fname, "Video"));
            tn.leaf("FileName", s(t));
            tn.leaf("RelativeFilename", s(&fname));
            objects.children.push(tn);
            connect("OP", tid, id, Some("DiffuseColor"));
        }
    }

    // models, geometry and joint attributes
    let mut model_ids: Vec<i64> = Vec::new();
    let mut n_geo = 0;
    let mut n_attr = 0;
    for n in &io.nodes {
        let mid = new_id();
        model_ids.push(mid);
        let ty = match n.kind {
            IoKind::Mesh if !n.mesh.faces.is_empty() => "Mesh",
            IoKind::Joint => "LimbNode",
            _ => "Null",
        };
        let mut mn = FNode::new("Model").with(Prop::I64(mid)).with(named(&n.name, "Model")).with(s(ty));
        mn.leaf("Version", Prop::I32(232));
        let mut rows = vec![
            prow("RotationActive", "bool", "", "", vec![Prop::I32(1)]),
            prow("InheritType", "enum", "", "", vec![Prop::I32(1)]),
            vec3_row("Lcl Translation", "Lcl Translation", [n.t.x as f64, n.t.y as f64, n.t.z as f64]),
            vec3_row("Lcl Rotation", "Lcl Rotation", [n.r.x as f64, n.r.y as f64, n.r.z as f64]),
            vec3_row("Lcl Scaling", "Lcl Scaling", [n.s.x as f64, n.s.y as f64, n.s.z as f64]),
            vec3_row("RotationPivot", "Vector3D", [n.rp.x as f64, n.rp.y as f64, n.rp.z as f64]),
            vec3_row("ScalingPivot", "Vector3D", [n.sp.x as f64, n.sp.y as f64, n.sp.z as f64]),
        ];
        rows.insert(0, prow("DefaultAttributeIndex", "int", "Integer", "", vec![Prop::I32(0)]));
        mn.add(p70(rows));
        mn.leaf("Shading", Prop::Bool(true));
        mn.leaf("Culling", s("CullingOff"));
        objects.children.push(mn);
        connect("OO", mid, n.parent.map(|p| model_ids[p]).unwrap_or(0), None);

        if n.kind == IoKind::Joint {
            let aid = new_id();
            n_attr += 1;
            let mut an = FNode::new("NodeAttribute").with(Prop::I64(aid)).with(named(&n.name, "NodeAttribute")).with(s("LimbNode"));
            an.add(p70(vec![prow("Size", "double", "Number", "", vec![Prop::F64(1.0)])]));
            an.leaf("TypeFlags", s("Skeleton"));
            objects.children.push(an);
            connect("OO", aid, mid, None);
        }
        if ty == "Mesh" {
            let gid = new_id();
            n_geo += 1;
            // material slots: the node's own material first, then the others used by faces
            let mut slots: Vec<usize> = n.material.into_iter().collect();
            for m in n.face_material.iter().flatten() {
                if !slots.contains(m) {
                    slots.push(*m);
                }
            }
            let fslot: Vec<i32> = if n.face_material.is_empty() { Vec::new() } else { n.face_material.iter().map(|m| m.and_then(|m| slots.iter().position(|s| *s == m)).unwrap_or(0) as i32).collect() };
            objects.children.push(geometry_node(gid, &n.name, &n.mesh, &fslot));
            connect("OO", gid, mid, None);
            for m in &slots {
                connect("OO", mat_ids[*m], mid, None);
            }
        }
    }

    // animation
    let mut n_stack = 0;
    let mut n_layer = 0;
    let mut n_cnode = 0;
    let mut n_curve = 0;
    if !io.curves.is_empty() {
        let sid = new_id();
        let lid = new_id();
        n_stack = 1;
        n_layer = 1;
        let mut st = FNode::new("AnimationStack").with(Prop::I64(sid)).with(named("Take 001", "AnimStack")).with(s(""));
        st.add(p70(vec![
            prow("LocalStart", "KTime", "Time", "", vec![Prop::I64(ticks(tmin, fps))]),
            prow("LocalStop", "KTime", "Time", "", vec![Prop::I64(ticks(tmax, fps))]),
            prow("ReferenceStart", "KTime", "Time", "", vec![Prop::I64(ticks(tmin, fps))]),
            prow("ReferenceStop", "KTime", "Time", "", vec![Prop::I64(ticks(tmax, fps))]),
        ]));
        objects.children.push(st);
        objects.children.push(FNode::new("AnimationLayer").with(Prop::I64(lid)).with(named("BaseLayer", "AnimLayer")).with(s("")));
        connect("OO", lid, sid, None);
        // group curves by node and channel
        let mut order: Vec<(usize, &str)> = Vec::new();
        for c in &io.curves {
            let ch = if c.attr.starts_with("translate") {
                "T"
            } else if c.attr.starts_with("rotate") {
                "R"
            } else {
                "S"
            };
            if !order.contains(&(c.node, ch)) {
                order.push((c.node, ch));
            }
        }
        for (ni, ch) in order {
            let nd = &io.nodes[ni];
            let (prop, vals) = match ch {
                "T" => ("Lcl Translation", nd.t),
                "R" => ("Lcl Rotation", nd.r),
                _ => ("Lcl Scaling", nd.s),
            };
            let cid = new_id();
            n_cnode += 1;
            let mut cn = FNode::new("AnimationCurveNode").with(Prop::I64(cid)).with(named(ch, "AnimCurveNode")).with(s(""));
            cn.add(p70(vec![
                prow("d|X", "Number", "", "A", vec![Prop::F64(vals.x as f64)]),
                prow("d|Y", "Number", "", "A", vec![Prop::F64(vals.y as f64)]),
                prow("d|Z", "Number", "", "A", vec![Prop::F64(vals.z as f64)]),
            ]));
            objects.children.push(cn);
            connect("OO", cid, lid, None);
            connect("OP", cid, model_ids[ni], Some(prop));
            for (axis, letter) in ["X", "Y", "Z"].iter().enumerate() {
                let want = format!("{}{}", match ch { "T" => "translate", "R" => "rotate", _ => "scale" }, letter);
                let Some(c) = io.curves.iter().find(|c| c.node == ni && c.attr == want) else { continue };
                let _ = axis;
                let kid = new_id();
                n_curve += 1;
                let mut kn = FNode::new("AnimationCurve").with(Prop::I64(kid)).with(named("", "AnimCurve")).with(s(""));
                kn.leaf("Default", Prop::F64(c.keys.first().map(|k| k.1 as f64).unwrap_or(0.0)));
                kn.leaf("KeyVer", Prop::I32(4008));
                kn.leaf("KeyTime", Prop::ArrL(c.keys.iter().map(|k| ticks(k.0, fps)).collect()));
                kn.leaf("KeyValueFloat", Prop::ArrF(c.keys.iter().map(|k| k.1).collect()));
                // attribute runs of equal interpolation
                let mut flags: Vec<i32> = Vec::new();
                let mut data: Vec<f32> = Vec::new();
                let mut counts: Vec<i32> = Vec::new();
                let mut last: Option<Interp> = None;
                for k in &c.keys {
                    if last == Some(k.2) {
                        *counts.last_mut().unwrap() += 1;
                    } else {
                        flags.push(match k.2 {
                            Interp::Linear => 4,
                            Interp::Step => 2,
                            Interp::Bezier => 24840,
                        });
                        data.extend_from_slice(&[0.0, 0.0, 9.419_963_346_924_634e-30, 0.0]);
                        counts.push(1);
                        last = Some(k.2);
                    }
                }
                kn.leaf("KeyAttrFlags", Prop::ArrI(flags));
                kn.leaf("KeyAttrDataFloat", Prop::ArrF(data));
                kn.leaf("KeyAttrRefCount", Prop::ArrI(counts));
                objects.children.push(kn);
                connect("OP", kid, cid, Some(&format!("d|{}", letter)));
            }
        }
    }

    let mut defs = FNode::new("Definitions");
    defs.leaf("Version", Prop::I32(100));
    let mut kinds: Vec<(&str, i32)> = vec![("GlobalSettings", 1), ("Model", io.nodes.len() as i32), ("Geometry", n_geo), ("Material", io.materials.len() as i32), ("Texture", n_textures), ("NodeAttribute", n_attr), ("AnimationStack", n_stack), ("AnimationLayer", n_layer), ("AnimationCurveNode", n_cnode), ("AnimationCurve", n_curve)];
    kinds.retain(|k| k.1 > 0);
    let total: i32 = kinds.iter().map(|k| k.1).sum();
    defs.leaf("Count", Prop::I32(total));
    for (k, c) in kinds {
        let mut ot = FNode::new("ObjectType").with(s(k));
        ot.leaf("Count", Prop::I32(c));
        defs.children.push(ot);
    }

    let mut takes = FNode::new("Takes");
    takes.leaf("Current", s(if io.curves.is_empty() { "" } else { "Take 001" }));

    vec![header, FNode::new("FileId").with(Prop::Raw(vec![0x28, 0xb3, 0x2a, 0xeb, 0xb6, 0x24, 0xcc, 0xc2, 0xbf, 0xc8, 0xb0, 0x2a, 0xa9, 0x2b, 0xfc, 0xf1])), FNode::new("CreationTime").with(s("1970-01-01 10:00:00:000")), FNode::new("Creator").with(s("Aztec 2009")), gs, docs, FNode::new("References"), defs, objects, conns, takes]
}

fn geometry_node(id: i64, name: &str, m: &Mesh, face_slot: &[i32]) -> FNode {
    let mut g = FNode::new("Geometry").with(Prop::I64(id)).with(named(name, "Geometry")).with(s("Mesh"));
    g.add(p70(vec![]));
    g.leaf("GeometryVersion", Prop::I32(124));
    g.leaf("Vertices", Prop::ArrD(m.positions.iter().flat_map(|p| [p.x as f64, p.y as f64, p.z as f64]).collect()));
    let mut pvi: Vec<i32> = Vec::new();
    let mut corner_v: Vec<usize> = Vec::new();
    for f in &m.faces {
        for (i, &v) in f.iter().enumerate() {
            corner_v.push(v as usize);
            pvi.push(if i + 1 == f.len() { !(v as i32) } else { v as i32 });
        }
    }
    g.leaf("PolygonVertexIndex", Prop::ArrI(pvi));
    let mut ln = FNode::new("LayerElementNormal").with(Prop::I32(0));
    ln.leaf("Version", Prop::I32(101));
    ln.leaf("Name", s(""));
    ln.leaf("MappingInformationType", s("ByPolygonVertex"));
    ln.leaf("ReferenceInformationType", s("Direct"));
    ln.leaf("Normals", Prop::ArrD(corner_v.iter().flat_map(|&v| [m.normals[v].x as f64, m.normals[v].y as f64, m.normals[v].z as f64]).collect()));
    g.children.push(ln);
    let mut lu = FNode::new("LayerElementUV").with(Prop::I32(0));
    lu.leaf("Version", Prop::I32(101));
    lu.leaf("Name", s("map1"));
    lu.leaf("MappingInformationType", s("ByPolygonVertex"));
    lu.leaf("ReferenceInformationType", s("IndexToDirect"));
    lu.leaf("UV", Prop::ArrD(m.uvs.iter().flat_map(|t| [t.x as f64, t.y as f64]).collect()));
    lu.leaf("UVIndex", Prop::ArrI(corner_v.iter().map(|&v| v as i32).collect()));
    g.children.push(lu);
    let mut lm = FNode::new("LayerElementMaterial").with(Prop::I32(0));
    lm.leaf("Version", Prop::I32(101));
    lm.leaf("Name", s(""));
    if face_slot.is_empty() {
        lm.leaf("MappingInformationType", s("AllSame"));
        lm.leaf("ReferenceInformationType", s("IndexToDirect"));
        lm.leaf("Materials", Prop::ArrI(vec![0]));
    } else {
        lm.leaf("MappingInformationType", s("ByPolygon"));
        lm.leaf("ReferenceInformationType", s("IndexToDirect"));
        lm.leaf("Materials", Prop::ArrI(face_slot.to_vec()));
    }
    g.children.push(lm);
    let mut layer = FNode::new("Layer").with(Prop::I32(0));
    layer.leaf("Version", Prop::I32(100));
    for t in ["LayerElementNormal", "LayerElementUV", "LayerElementMaterial"] {
        let mut le = FNode::new("LayerElement");
        le.leaf("Type", s(t));
        le.leaf("TypedIndex", Prop::I32(0));
        layer.children.push(le);
    }
    g.children.push(layer);
    g
}

pub fn write(io: &IoScene, path: &str, ascii: bool) -> Result<(), String> {
    let t = build_tree(io);
    let bytes = if ascii { tree::write_ascii(&t, VERSION).into_bytes() } else { tree::write_binary(&t, VERSION) };
    std::fs::write(path, bytes).map_err(|e| format!("Cannot write '{}': {}", path, e))
}

// ====================================================================== reading

pub fn read(path: &str) -> Result<IoScene, String> {
    let data = std::fs::read(path).map_err(|e| format!("Cannot read '{}': {}", path, e))?;
    let dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    from_bytes(&data, &dir)
}

struct Obj<'a> {
    node: &'a FNode,
}

impl<'a> Obj<'a> {
    fn class(&self) -> &str {
        self.node.str_at(2)
    }
    fn name(&self) -> &str {
        self.node.str_at(1)
    }
    fn p70(&self) -> Option<&'a FNode> {
        self.node.child("Properties70")
    }
}

fn prop_vals<'a>(p70: Option<&'a FNode>, name: &str) -> Option<&'a [Prop]> {
    p70?.all("P").find(|p| p.str_at(0) == name).map(|p| p.props.get(4..).unwrap_or(&[]))
}

fn prop_vec3(p70: Option<&FNode>, name: &str) -> Option<[f64; 3]> {
    let v = prop_vals(p70, name)?;
    if v.len() < 3 {
        return None;
    }
    Some([v[0].f64()?, v[1].f64()?, v[2].f64()?])
}

fn prop_num(p70: Option<&FNode>, name: &str) -> Option<f64> {
    prop_vals(p70, name)?.first()?.f64()
}

/// Rotation matrix for euler degrees with an FBX rotation order (0 = XYZ, applied X first).
fn euler(r: [f64; 3], order: i32) -> Mat4 {
    let axis = |i: usize| -> Quat {
        let mut a = Vec3::ZERO;
        match i {
            0 => a.x = 1.0,
            1 => a.y = 1.0,
            _ => a.z = 1.0,
        }
        Quat::from_axis_angle(a, (r[i] as f32).to_radians())
    };
    let seq: [usize; 3] = match order {
        1 => [0, 2, 1],
        2 => [1, 2, 0],
        3 => [1, 0, 2],
        4 => [2, 0, 1],
        5 => [2, 1, 0],
        _ => [0, 1, 2],
    };
    Mat4::from_quat(axis(seq[2]) * axis(seq[1]) * axis(seq[0]))
}

#[derive(Clone, Copy, Default)]
struct Xf {
    t: [f64; 3],
    r: [f64; 3],
    s: [f64; 3],
    pre: [f64; 3],
    post: [f64; 3],
    roff: [f64; 3],
    rp: [f64; 3],
    soff: [f64; 3],
    sp: [f64; 3],
    order: i32,
}

impl Xf {
    fn read(p70: Option<&FNode>) -> Xf {
        let g = |n: &str, d: [f64; 3]| prop_vec3(p70, n).unwrap_or(d);
        Xf {
            t: g("Lcl Translation", [0.0; 3]),
            r: g("Lcl Rotation", [0.0; 3]),
            s: g("Lcl Scaling", [1.0; 3]),
            pre: g("PreRotation", [0.0; 3]),
            post: g("PostRotation", [0.0; 3]),
            roff: g("RotationOffset", [0.0; 3]),
            rp: g("RotationPivot", [0.0; 3]),
            soff: g("ScalingOffset", [0.0; 3]),
            sp: g("ScalingPivot", [0.0; 3]),
            order: prop_num(p70, "RotationOrder").unwrap_or(0.0) as i32,
        }
    }

    /// Only translate / rotate / scale matter and the order is XYZ.
    fn simple(&self) -> bool {
        self.order == 0 && [self.pre, self.post, self.roff, self.soff].iter().all(|v| v.iter().all(|x| x.abs() < 1e-9))
    }

    fn matrix(&self) -> Mat4 {
        let v = |a: [f64; 3]| Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32);
        let tr = |a: [f64; 3]| Mat4::translation(v(a));
        tr(self.t) * tr(self.roff) * tr(self.rp) * euler(self.pre, 0) * euler(self.r, self.order) * euler(self.post, 0).inverse() * tr(self.rp).inverse() * tr(self.soff) * tr(self.sp) * Mat4::scaling(v(self.s)) * tr(self.sp).inverse()
    }
}

struct Curve {
    /// (seconds, value, interpolation)
    keys: Vec<(f64, f32, Interp)>,
}

fn eval(c: &Curve, t: f64) -> f32 {
    let k = &c.keys;
    if k.is_empty() {
        return 0.0;
    }
    if t <= k[0].0 {
        return k[0].1;
    }
    for w in k.windows(2) {
        if t < w[1].0 {
            return match w[0].2 {
                Interp::Step => w[0].1,
                _ => {
                    let f = ((t - w[0].0) / (w[1].0 - w[0].0).max(1e-12)) as f32;
                    w[0].1 + (w[1].1 - w[0].1) * f
                }
            };
        }
    }
    k[k.len() - 1].1
}

fn fps_of_mode(mode: i32, custom: f64) -> f32 {
    match mode {
        1 => 120.0,
        2 => 100.0,
        3 => 60.0,
        4 => 50.0,
        5 => 48.0,
        6 | 7 => 30.0,
        8 | 9 => 29.97,
        10 => 25.0,
        11 | 13 => 24.0,
        12 => 1000.0,
        14 if custom > 0.0 => custom as f32,
        15 => 96.0,
        16 => 72.0,
        17 => 59.94,
        _ => 30.0,
    }
}

pub fn from_bytes(data: &[u8], dir: &std::path::Path) -> Result<IoScene, String> {
    let (top, version) = tree::parse(data)?;
    if version != 0 && version < 7000 {
        return Err(format!("FBX version {} is not supported (7.0 or newer needed; re-export from your application)", version));
    }
    let objects_node = top.iter().find(|n| n.name == "Objects").ok_or("Not an FBX 7.x scene: no Objects section")?;
    let mut objs: HashMap<i64, Obj> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();
    for n in &objects_node.children {
        if let Some(id) = n.props.first().and_then(|p| p.i64()) {
            objs.insert(id, Obj { node: n });
            order.push(id);
        }
    }
    // (kind, child, parent, property)
    let mut conns: Vec<(i64, i64, Option<String>)> = Vec::new();
    if let Some(c) = top.iter().find(|n| n.name == "Connections") {
        for n in c.all("C") {
            if let (Some(a), Some(b)) = (n.props.get(1).and_then(|p| p.i64()), n.props.get(2).and_then(|p| p.i64())) {
                conns.push((a, b, n.props.get(3).and_then(|p| p.str()).map(|s| s.to_string())));
            }
        }
    }
    let gs = top.iter().find(|n| n.name == "GlobalSettings").and_then(|g| g.child("Properties70"));
    let up_axis = prop_num(gs, "UpAxis").unwrap_or(1.0) as i32;
    let fps = fps_of_mode(prop_num(gs, "TimeMode").unwrap_or(11.0) as i32, prop_num(gs, "CustomFrameRate").unwrap_or(0.0));
    let kind_of = |id: i64| objs.get(&id).map(|o| o.node.name.as_str()).unwrap_or("");

    // parent -> models, model -> geometry / materials / attribute
    let mut kids: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut geo_of: HashMap<i64, i64> = HashMap::new();
    let mut mats_of: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut skeleton: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut tex_of: HashMap<i64, i64> = HashMap::new();
    for (c, p, prop) in &conns {
        match (kind_of(*c), kind_of(*p)) {
            ("Model", "Model") => kids.entry(*p).or_default().push(*c),
            ("Model", _) if *p == 0 => kids.entry(0).or_default().push(*c),
            ("Geometry", "Model") => {
                geo_of.insert(*p, *c);
            }
            ("Material", "Model") => mats_of.entry(*p).or_default().push(*c),
            ("NodeAttribute", "Model") => {
                if objs[c].class() == "LimbNode" || objs[c].node.child("TypeFlags").map(|t| t.str_at(0) == "Skeleton").unwrap_or(false) {
                    skeleton.insert(*p);
                }
            }
            ("Texture", "Material") => {
                let diffuse = prop.as_deref().map(|p| p.contains("Diffuse") || p.contains("Color") || p.contains("base")).unwrap_or(true);
                if diffuse || !tex_of.contains_key(p) {
                    tex_of.insert(*p, *c);
                }
            }
            _ => {}
        }
    }
    // models that are not connected to anything still count as roots
    let connected: std::collections::HashSet<i64> = conns.iter().filter(|(c, _, _)| kind_of(*c) == "Model").map(|(c, _, _)| *c).collect();
    for id in &order {
        if objs[id].node.name == "Model" && !connected.contains(id) {
            kids.entry(0).or_default().push(*id);
        }
    }

    // animation: model -> (channel letter -> axis -> curve)
    type Chan = [Option<Curve>; 3];
    let mut anim: HashMap<i64, HashMap<char, Chan>> = HashMap::new();
    for (cn_id, model, prop) in &conns {
        if kind_of(*cn_id) != "AnimationCurveNode" || kind_of(*model) != "Model" {
            continue;
        }
        let ch = match prop.as_deref() {
            Some("Lcl Translation") => 'T',
            Some("Lcl Rotation") => 'R',
            Some("Lcl Scaling") => 'S',
            _ => continue,
        };
        let mut chan: Chan = [None, None, None];
        for (cv, cn, p) in &conns {
            if cn != cn_id || kind_of(*cv) != "AnimationCurve" {
                continue;
            }
            let axis = match p.as_deref() {
                Some("d|X") => 0,
                Some("d|Y") => 1,
                Some("d|Z") => 2,
                _ => continue,
            };
            let node = objs[cv].node;
            let times = node.child("KeyTime").and_then(|k| k.props.first()).and_then(|p| p.vec_i64()).unwrap_or_default();
            let vals = node.child("KeyValueFloat").and_then(|k| k.props.first()).and_then(|p| p.vec_f64()).unwrap_or_default();
            let flags = node.child("KeyAttrFlags").and_then(|k| k.props.first()).and_then(|p| p.vec_i64()).unwrap_or_default();
            let counts = node.child("KeyAttrRefCount").and_then(|k| k.props.first()).and_then(|p| p.vec_i64()).unwrap_or_default();
            let mut per_key: Vec<Interp> = Vec::new();
            for (i, c) in counts.iter().enumerate() {
                let f = flags.get(i).copied().unwrap_or(8);
                let it = if f & 0x4 != 0 {
                    Interp::Linear
                } else if f & 0x2 != 0 {
                    Interp::Step
                } else {
                    Interp::Bezier
                };
                per_key.extend(std::iter::repeat(it).take((*c).max(0) as usize));
            }
            let n = times.len().min(vals.len());
            let mut keys: Vec<(f64, f32, Interp)> = (0..n).map(|i| (times[i] as f64 / TICKS_PER_SECOND, vals[i] as f32, per_key.get(i).copied().unwrap_or(Interp::Bezier))).collect();
            keys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            if !keys.is_empty() {
                chan[axis] = Some(Curve { keys });
            }
        }
        if chan.iter().any(|c| c.is_some()) {
            anim.entry(*model).or_default().insert(ch, chan);
        }
    }

    let mut io = IoScene { fps, ..Default::default() };
    let mut mat_index: HashMap<i64, usize> = HashMap::new();
    let mut material = |id: i64, io: &mut IoScene| -> usize {
        if let Some(i) = mat_index.get(&id) {
            return *i;
        }
        let o = &objs[&id];
        let p = o.p70();
        let c = prop_vec3(p, "DiffuseColor").or_else(|| prop_vec3(p, "Diffuse")).unwrap_or([0.8, 0.8, 0.8]);
        let factor = prop_num(p, "DiffuseFactor").unwrap_or(1.0);
        let color = [(c[0] * factor).clamp(0.0, 1.0) as f32, (c[1] * factor).clamp(0.0, 1.0) as f32, (c[2] * factor).clamp(0.0, 1.0) as f32];
        let texture = tex_of.get(&id).and_then(|t| {
            let tn = objs[t].node;
            let f = tn.child("FileName").map(|f| f.str_at(0).to_string()).filter(|f| !f.is_empty()).or_else(|| tn.child("RelativeFilename").map(|f| f.str_at(0).to_string()))?;
            let f = f.replace('\\', "/");
            let p = std::path::Path::new(&f);
            if p.is_absolute() && p.exists() {
                Some(f)
            } else {
                // relative, or an absolute path from another machine: look next to the file
                let name = p.file_name()?;
                let local = dir.join(&f);
                let beside = dir.join(name);
                Some(if local.exists() { local } else { beside }.to_string_lossy().to_string())
            }
        });
        io.materials.push(IoMaterial { name: sanitize_name(o.name()), color, texture });
        mat_index.insert(id, io.materials.len() - 1);
        io.materials.len() - 1
    };

    // depth first so parents precede children
    let mut stack: Vec<(i64, Option<usize>)> = kids.get(&0).map(|v| v.iter().rev().map(|c| (*c, None)).collect()).unwrap_or_default();
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    while let Some((id, parent)) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let o = &objs[&id];
        let ty = o.class();
        let p70 = o.p70();
        let xf = Xf::read(p70);
        let has_geo = geo_of.contains_key(&id);
        let kind = if has_geo {
            IoKind::Mesh
        } else if skeleton.contains(&id) || ty == "LimbNode" || ty == "Limb" {
            IoKind::Joint
        } else {
            IoKind::Group
        };
        let mut node = IoNode::new(&sanitize_name(o.name()), parent, kind);
        let up_fix = if up_axis == 2 && parent.is_none() { Some(Mat4::from_quat(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), (-90.0f32).to_radians()))) } else { None };
        let curves = anim.get(&id);
        let bake = curves.is_some() && (!xf.simple() || up_fix.is_some());
        if !bake {
            let m = match (xf.simple(), &up_fix) {
                (true, None) => None,
                (_, f) => Some(f.map(|f| f * xf.matrix()).unwrap_or_else(|| xf.matrix())),
            };
            match m {
                None => {
                    node.rp = Vec3::new(xf.rp[0] as f32, xf.rp[1] as f32, xf.rp[2] as f32);
                    node.sp = Vec3::new(xf.sp[0] as f32, xf.sp[1] as f32, xf.sp[2] as f32);
                    node.t = Vec3::new(xf.t[0] as f32, xf.t[1] as f32, xf.t[2] as f32);
                    node.r = Vec3::new(xf.r[0] as f32, xf.r[1] as f32, xf.r[2] as f32);
                    node.s = Vec3::new(xf.s[0] as f32, xf.s[1] as f32, xf.s[2] as f32);
                }
                Some(m) => {
                    let (t, r, sc) = m.decompose();
                    node.t = t;
                    node.r = r;
                    node.s = sc;
                }
            }
        }
        let me: usize;
        if has_geo {
            let slots = mats_of.get(&id).cloned().unwrap_or_default();
            let geo = objs[&geo_of[&id]].node;
            let geometric = Xf { t: prop_vec3(p70, "GeometricTranslation").unwrap_or([0.0; 3]), r: prop_vec3(p70, "GeometricRotation").unwrap_or([0.0; 3]), s: prop_vec3(p70, "GeometricScaling").unwrap_or([1.0; 3]), ..Default::default() };
            let gm = if geometric.t.iter().chain(geometric.r.iter()).any(|v| v.abs() > 1e-9) || geometric.s.iter().any(|v| (v - 1.0).abs() > 1e-9) { Some(geometric.matrix()) } else { None };
            let (mesh, fslot) = read_geometry(geo, slots.len().max(1), gm);
            if mesh.faces.is_empty() {
                node.kind = IoKind::Group;
            } else {
                let mats: Vec<Option<usize>> = slots.iter().map(|m| Some(material(*m, &mut io))).collect();
                node.mesh = mesh;
                node.material = mats.first().copied().flatten();
                if fslot.iter().any(|s| *s != 0) {
                    node.face_material = fslot.iter().map(|s| mats.get(*s).copied().flatten()).collect();
                }
            }
            io.nodes.push(node);
            me = io.nodes.len() - 1;
        } else {
            io.nodes.push(node);
            me = io.nodes.len() - 1;
        }
        push_curves(&mut io, me, &xf, curves, bake, up_fix);
        if let Some(k) = kids.get(&id) {
            for c in k.iter().rev() {
                stack.push((*c, Some(me)));
            }
        }
    }
    if io.nodes.is_empty() {
        return Err("The FBX file contains no models".to_string());
    }
    Ok(io)
}

fn push_curves(io: &mut IoScene, node: usize, xf: &Xf, curves: Option<&HashMap<char, [Option<Curve>; 3]>>, bake: bool, up_fix: Option<Mat4>) {
    let Some(curves) = curves else { return };
    let fps = io.fps as f64;
    let attrs = |ch: char| -> [&'static str; 3] {
        match ch {
            'T' => ["translateX", "translateY", "translateZ"],
            'R' => ["rotateX", "rotateY", "rotateZ"],
            _ => ["scaleX", "scaleY", "scaleZ"],
        }
    };
    if !bake {
        for (ch, chan) in curves {
            for (axis, c) in chan.iter().enumerate() {
                if let Some(c) = c {
                    io.curves.push(IoCurve { node, attr: attrs(*ch)[axis].to_string(), keys: c.keys.iter().map(|k| ((k.0 * fps) as f32, k.1, k.2)).collect() });
                }
            }
        }
        return;
    }
    // sample the full FBX transform at every key time and store plain translate / rotate / scale keys
    let mut times: Vec<f64> = curves.values().flat_map(|ch| ch.iter().flatten().flat_map(|c| c.keys.iter().map(|k| k.0))).collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let mut rows: [Vec<(f32, f32, Interp)>; 9] = Default::default();
    let mut prev_r = [0.0f32; 3];
    for (ti, t) in times.iter().enumerate() {
        let mut x = *xf;
        for (ch, chan) in curves {
            for axis in 0..3 {
                if let Some(c) = &chan[axis] {
                    let v = eval(c, *t) as f64;
                    match ch {
                        'T' => x.t[axis] = v,
                        'R' => x.r[axis] = v,
                        _ => x.s[axis] = v,
                    }
                }
            }
        }
        let mut m = x.matrix();
        if let Some(f) = up_fix {
            m = f * m;
        }
        let (tr, mut r, sc) = m.decompose();
        let mut rr = [r.x, r.y, r.z];
        if ti > 0 {
            for i in 0..3 {
                while rr[i] - prev_r[i] > 180.0 {
                    rr[i] -= 360.0;
                }
                while rr[i] - prev_r[i] < -180.0 {
                    rr[i] += 360.0;
                }
            }
        }
        r = Vec3::new(rr[0], rr[1], rr[2]);
        prev_r = rr;
        let frame = (*t * fps) as f32;
        for (i, v) in [tr.x, tr.y, tr.z, r.x, r.y, r.z, sc.x, sc.y, sc.z].iter().enumerate() {
            rows[i].push((frame, *v, Interp::Linear));
        }
        if ti == 0 {
            let n = &mut io.nodes[node];
            n.t = tr;
            n.r = r;
            n.s = sc;
        }
    }
    let names = ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"];
    for (i, keys) in rows.into_iter().enumerate() {
        let varies = keys.windows(2).any(|w| (w[0].1 - w[1].1).abs() > 1e-6);
        if varies {
            io.curves.push(IoCurve { node, attr: names[i].to_string(), keys });
        }
    }
}

/// The geometry node as one corner-split mesh plus the material slot of every face.
fn read_geometry(g: &FNode, slots: usize, xf: Option<Mat4>) -> (Mesh, Vec<usize>) {
    let verts: Vec<f64> = g.child("Vertices").and_then(|v| v.props.first()).and_then(|p| p.vec_f64()).unwrap_or_default();
    let pvi: Vec<i64> = g.child("PolygonVertexIndex").and_then(|v| v.props.first()).and_then(|p| p.vec_i64()).unwrap_or_default();
    let pos: Vec<Vec3> = verts
        .chunks_exact(3)
        .map(|c| {
            let p = Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32);
            xf.map(|m| m.transform_point(p)).unwrap_or(p)
        })
        .collect();
    // faces as lists of corner positions
    let mut faces: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for &i in &pvi {
        let end = i < 0;
        let v = if end { !i } else { i } as usize;
        cur.push(v);
        if end {
            faces.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        faces.push(cur);
    }

    // generic layer element lookup: value index for (corner, vertex, face)
    struct Layer {
        mapping: String,
        indexed: bool,
        index: Vec<i64>,
    }
    let layer = |name: &str, idx_name: &str| -> Option<(Layer, Vec<f64>)> {
        let l = g.child(name)?;
        let data_name = match name {
            "LayerElementNormal" => "Normals",
            _ => "UV",
        };
        let data = l.child(data_name)?.props.first()?.vec_f64()?;
        let indexed = l.child("ReferenceInformationType").map(|r| r.str_at(0) != "Direct").unwrap_or(false);
        let index = l.child(idx_name).and_then(|n| n.props.first()).and_then(|p| p.vec_i64()).unwrap_or_default();
        Some((Layer { mapping: l.child("MappingInformationType").map(|m| m.str_at(0).to_string()).unwrap_or_default(), indexed, index }, data))
    };
    let normals = layer("LayerElementNormal", "NormalsIndex");
    let uvs = layer("LayerElementUV", "UVIndex");
    let pick = |l: &Layer, corner: usize, vert: usize, face: usize| -> usize {
        let slot = match l.mapping.as_str() {
            "ByPolygonVertex" => corner,
            "ByPolygon" => face,
            "AllSame" => 0,
            _ => vert,
        };
        if l.indexed {
            l.index.get(slot).copied().unwrap_or(0).max(0) as usize
        } else {
            slot
        }
    };
    // material slot per face
    let mat = g.child("LayerElementMaterial");
    let mat_map = mat.and_then(|m| m.child("MappingInformationType")).map(|m| m.str_at(0)).unwrap_or("AllSame");
    let mat_idx: Vec<i64> = mat.and_then(|m| m.child("Materials")).and_then(|m| m.props.first()).and_then(|p| p.vec_i64()).unwrap_or_default();

    let mut out = Mesh::new();
    let mut fslot: Vec<usize> = Vec::new();
    let mut corner = 0usize;
    for (fi, f) in faces.iter().enumerate() {
        let slot = match mat_map {
            "ByPolygon" => mat_idx.get(fi).copied().unwrap_or(0).max(0) as usize,
            "AllSame" => mat_idx.first().copied().unwrap_or(0).max(0) as usize,
            _ => 0,
        }
        .min(slots - 1);
        let m = &mut out;
        let mut idx = Vec::with_capacity(f.len());
        for &v in f {
            let p = pos.get(v).copied().unwrap_or(Vec3::ZERO);
            let n = normals
                .as_ref()
                .and_then(|(l, d)| {
                    let i = pick(l, corner, v, fi);
                    d.get(i * 3..i * 3 + 3).map(|c| Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32))
                })
                .unwrap_or(Vec3::UP);
            let uv = uvs
                .as_ref()
                .and_then(|(l, d)| {
                    let i = pick(l, corner, v, fi);
                    d.get(i * 2..i * 2 + 2).map(|c| Vec2::new(c[0] as f32, c[1] as f32))
                })
                .unwrap_or(Vec2::new(0.0, 0.0));
            idx.push(m.add_vertex(p, n, uv));
            corner += 1;
        }
        if idx.len() >= 3 {
            m.add_face(&idx);
            fslot.push(slot);
        }
    }
    (out, fslot)
}
