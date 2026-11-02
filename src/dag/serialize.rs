//! `.aztec` ASCII scene files. The format mirrors Maya ASCII: a plain list of commands
//! (`createNode`, `setAttr`, `connectAttr`, `setKeyframe`, ...) that rebuild the scene when replayed.

use super::graph::Scene;
use super::graph::NodeId;
use super::nodes::{build_mesh, default_plugs, MeshData, NodeData, PolyKind, PolyOpData};
use crate::anim::Interp;
use crate::mel::parser::{parse_args, parse_script, FlagSpec, Parsed};

fn q(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            _ => o.push(c),
        }
    }
    o.push('"');
    o
}

fn n(v: f32) -> String {
    if v.is_finite() {
        format!("{}", v)
    } else {
        "0".to_string()
    }
}

/// Serialize the whole scene to `.aztec` ASCII.
pub fn to_ascii(scene: &Scene) -> String {
    let mut s = String::new();
    s.push_str("//Aztec 2009 ASCII Scene\n");
    s.push_str("requires aztec \"2009\";\n");
    s.push_str("fileInfo \"application\" \"Aztec 2009\";\n");
    let tl = &scene.timeline;
    s.push_str(&format!(
        "playbackOptions -min {} -max {} -fps {} -current {};\n",
        n(tl.start),
        n(tl.end),
        n(tl.fps),
        n(tl.current)
    ));

    let rows = scene.outliner_rows();
    // history nodes of every mesh shape, in creation order
    let mut dump: Vec<NodeId> = Vec::new();
    for (id, _) in &rows {
        dump.push(*id);
        if let Some(NodeData::Mesh(md)) = scene.node(*id).map(|n| &n.data) {
            dump.extend(md.history.iter().copied());
        }
    }

    // 1. nodes (history nodes first so a shape can name them)
    for (id, _) in &rows {
        let node = scene.node(*id).unwrap();
        if let NodeData::Mesh(md) = &node.data {
            for h in &md.history {
                let hn = match scene.node(*h) {
                    Some(n) => n,
                    None => continue,
                };
                if let NodeData::PolyOp(op) = &hn.data {
                    let mut line = format!("createNode polyOp -n {} -kind {}", q(&hn.name), q(op.kind.name()));
                    if !op.ints.is_empty() {
                        let v: Vec<String> = op.ints.iter().map(|i| i.to_string()).collect();
                        line.push_str(&format!(" -ints {}", q(&v.join(" "))));
                    }
                    if let Some(e) = &op.ext {
                        if let Some(h) = e.handle.filter(|h| scene.node(*h).is_some()) {
                            line.push_str(&format!(" -handle {}", q(&scene.name_of(h))));
                        }
                        if let Some(m) = &e.bind_inv {
                            let fl: Vec<String> = (0..16).map(|k| n(m.m[k / 4][k % 4])).collect();
                            line.push_str(&format!(" -bindinv {}", q(&fl.join(" "))));
                        }
                        if let Some(c) = e.curve.filter(|c| scene.node(*c).is_some()) {
                            line.push_str(&format!(" -curve {}", q(&scene.name_of(c))));
                        }
                        if let Some(c) = &e.base_curve {
                            let pts: Vec<String> = c.cvs.iter().flat_map(|p| [n(p.x), n(p.y), n(p.z)]).collect();
                            line.push_str(&format!(" -basecurve {} -basecvs {}", q(&format!("{} {} {} {}", c.dir.degree, c.dir.n, c.dir.periodic as u8, c.dir.knots.iter().map(|k| n(*k)).collect::<Vec<_>>().join(" "))), q(&pts.join(" "))));
                        }
                        if let Some(c) = e.center {
                            line.push_str(&format!(" -center {}", q(&format!("{} {} {}", n(c.x), n(c.y), n(c.z)))));
                        }
                        if !e.weights.is_empty() {
                            line.push_str(&format!(" -dweights {}", q(&e.weights.iter().map(|w| n(*w)).collect::<Vec<_>>().join(" "))));
                        }
                    }
                    if let Some(l) = op.lattice.filter(|l| scene.node(*l).is_some()) {
                        line.push_str(&format!(" -lattice {}", q(&scene.name_of(l))));
                    }
                    if let Some(sk) = &op.skin {
                        let names: Vec<String> = sk.joints.iter().map(|j| scene.name_of(*j)).collect();
                        let mut fl: Vec<String> = Vec::new();
                        for m in sk.bind_inv.iter().chain(std::iter::once(&sk.bind_mesh_world)) {
                            for c in 0..4 {
                                for r in 0..4 {
                                    fl.push(n(m.m[c][r]));
                                }
                            }
                        }
                        let mut w: Vec<String> = Vec::new();
                        for v in &sk.weights {
                            w.push(v.len().to_string());
                            for (j, wt) in v {
                                w.push(j.to_string());
                                w.push(n(*wt));
                            }
                        }
                        line.push_str(&format!(" -skin {} -bind {} -weights {} -skinopts {}", q(&names.join(" ")), q(&fl.join(" ")), q(&w.join(" ")), q(&format!("{} {} {}", sk.max_inf, n(sk.dropoff), sk.rigid as u8))));
                    }
                    if let Some(src) = &op.source {
                        let verts: Vec<String> = src.pos.iter().flat_map(|p| [n(p.x), n(p.y), n(p.z)]).collect();
                        let mut faces: Vec<String> = Vec::new();
                        for f in &src.faces {
                            faces.push(f.len().to_string());
                            faces.extend(f.iter().map(|i| i.to_string()));
                        }
                        line.push_str(&format!(" -verts {} -faces {}", q(&verts.join(" ")), q(&faces.join(" "))));
                    }
                    line.push_str(";\n");
                    s.push_str(&line);
                }
            }
        }
        let mut line = format!("createNode {} -n {}", node.data.type_name(), q(&node.name));
        if let Some(p) = node.parent {
            line.push_str(&format!(" -p {}", q(&scene.name_of(p))));
        }
        if let NodeData::IkHandle(d) = &node.data {
            if let (Some(a), Some(b)) = (d.start, d.end) {
                line.push_str(&format!(" -start {} -end {}", q(&scene.name_of(a)), q(&scene.name_of(b))));
            }
            if d.solver != 0 {
                line.push_str(&format!(" -solver {}", d.solver));
            }
            if let Some(c) = d.curve.filter(|c| scene.node(*c).is_some()) {
                line.push_str(&format!(" -curve {}", q(&scene.name_of(c))));
            }
            if let Some(pl) = d.pole.filter(|c| scene.node(*c).is_some()) {
                line.push_str(&format!(" -pole {}", q(&scene.name_of(pl))));
            }
        }
        if let NodeData::Constraint(c) = &node.data {
            let names: Vec<String> = c.targets.iter().map(|t| scene.name_of(*t)).collect();
            let mut fl: Vec<String> = Vec::new();
            for m in &c.offsets {
                for col in 0..4 {
                    for r in 0..4 {
                        fl.push(n(m.m[col][r]));
                    }
                }
            }
            line.push_str(&format!(" -kind {} -targets {} -offsets {}", q(c.kind.name()), q(&names.join(" ")), q(&fl.join(" "))));
        }
        if let NodeData::Character(c) = &node.data {
            let names: Vec<String> = c.channels.iter().filter(|(i, _)| scene.node(*i).is_some()).map(|(i, a)| format!("{}.{}", scene.name_of(*i), a)).collect();
            line.push_str(&format!(" -channels {}", q(&names.join(" "))));
        }
        if let NodeData::Distance(d) = &node.data {
            if let (Some(a), Some(b)) = (d.start, d.end) {
                line.push_str(&format!(" -start {} -end {}", q(&scene.name_of(a)), q(&scene.name_of(b))));
            }
        }
        if let NodeData::Lattice(ld) = &node.data {
            line.push_str(&format!(" -divs {}", q(&format!("{} {} {}", ld.divs[0], ld.divs[1], ld.divs[2]))));
            if let Some(sk) = &ld.skin {
                let names: Vec<String> = sk.joints.iter().map(|j| scene.name_of(*j)).collect();
                let mut fl: Vec<String> = Vec::new();
                for m in sk.bind_inv.iter().chain(std::iter::once(&sk.bind_world)) {
                    for c in 0..4 {
                        for r in 0..4 {
                            fl.push(n(m.m[c][r]));
                        }
                    }
                }
                let mut w: Vec<String> = Vec::new();
                for v in &sk.weights {
                    w.push(v.len().to_string());
                    for (j, wt) in v {
                        w.push(j.to_string());
                        w.push(n(*wt));
                    }
                }
                line.push_str(&format!(" -skin {} -bind {} -weights {}", q(&names.join(" ")), q(&fl.join(" ")), q(&w.join(" "))));
            }
        }
        if let Some(payload) = dyn_payload(scene, &node.data) {
            line.push_str(&format!(" -dyn {}", q(&payload)));
        }
        if let NodeData::RenderGlobals(rg) = &node.data {
            line.push_str(&format!(" -prefix {} -ext {} -camera {}", q(&rg.prefix), q(&rg.ext), q(&rg.camera)));
        }
        if let NodeData::MotionPath(mp) = &node.data {
            if let Some(c) = mp.curve.filter(|c| scene.node(*c).is_some()) {
                line.push_str(&format!(" -curve {}", q(&scene.name_of(c))));
            }
        }
        if let NodeData::Sound(sd) = &node.data {
            line.push_str(&format!(" -path {}", q(&sd.path)));
        }
        if let NodeData::Texture(td) = &node.data {
            line.push_str(&format!(" -kind {} -path {}", q(&td.kind), q(&td.path)));
        }
        if let NodeData::ObjectSet(sd) = &node.data {
            let names: Vec<String> = sd.members.iter().filter(|m| scene.node(**m).is_some()).map(|m| scene.name_of(*m)).collect();
            line.push_str(&format!(" -members {}", q(&names.join(" "))));
        }
        if let NodeData::DrivenCurve(cv) = &node.data {
            let keys: Vec<String> = cv.keys.iter().flat_map(|k| [n(k.t), n(k.v), k.interp.name().to_string()]).collect();
            line.push_str(&format!(" -keys {}", q(&keys.join(" "))));
        }
        if let NodeData::Mesh(md) = &node.data {
            if let Some(st) = &md.stroke {
                let b = st.brush.filter(|b| scene.node(*b).is_some()).map(|b| scene.name_of(b)).unwrap_or_default();
                let path: Vec<String> = st.path.iter().flat_map(|(p, nr)| [n(p.x), n(p.y), n(p.z), n(nr.x), n(nr.y), n(nr.z)]).collect();
                line.push_str(&format!(" -strokebrush {} -strokeseed {} -strokepath {}", q(&b), st.seed, q(&path.join(" "))));
                let mats: Vec<String> = st.mats.iter().map(|m| m.filter(|m| scene.node(*m).is_some()).map(|m| scene.name_of(m)).unwrap_or_default()).collect();
                line.push_str(&format!(" -strokemats {}", q(&mats.join(" "))));
            }
            if let Some(src) = md.instance_of {
                line.push_str(&format!(" -instanceOf {}", q(&scene.name_of(src))));
            }
            if md.nurbs.is_some() {
                line.push_str(" -prim custom -params \"\"");
            } else if !md.history.is_empty() {
                let names: Vec<String> = md.history.iter().map(|h| scene.name_of(*h)).collect();
                line.push_str(&format!(" -prim history -params \"\" -history {}", q(&names.join(" "))));
            } else {
                let params: Vec<String> = md.params.iter().map(|(k, v)| format!("{}={}", k, n(*v))).collect();
                line.push_str(&format!(" -prim {} -params {}", q(&md.prim), q(&params.join(" "))));
                if md.prim == "custom" {
                    let verts: Vec<String> = md.mesh.positions.iter().flat_map(|p| [n(p.x), n(p.y), n(p.z)]).collect();
                    let mut faces: Vec<String> = Vec::new();
                    for f in &md.mesh.faces {
                        faces.push(f.len().to_string());
                        faces.extend(f.iter().map(|i| i.to_string()));
                    }
                    line.push_str(&format!(" -verts {} -faces {}", q(&verts.join(" ")), q(&faces.join(" "))));
                }
            }
        }
        line.push_str(";\n");
        s.push_str(&line);
    }

    // 1b. NURBS geometry (right after the nodes exist, before their attributes)
    let dir_text = |d: &crate::geo::nurbs::Dir| -> String { format!("{} {} {} {}", d.degree, d.n, d.periodic as u8, d.knots.iter().map(|k| n(*k)).collect::<Vec<_>>().join(" ")) };
    for id in &dump {
        let node = scene.node(*id).unwrap();
        let (gen, cvs, base, dirs): (Option<&crate::dag::nodes::NurbsGen>, &Vec<crate::math::Vec3>, &Vec<crate::math::Vec3>, Vec<String>) = match &node.data {
            NodeData::NurbsCurve(cd) => (cd.gen.as_ref(), &cd.curve.cvs, &cd.base, vec![dir_text(&cd.curve.dir)]),
            NodeData::Mesh(md) => match &md.nurbs {
                Some(ns) => (ns.gen.as_ref(), &ns.surf.cvs, &ns.base, vec![dir_text(&ns.surf.u), dir_text(&ns.surf.v)]),
                None => continue,
            },
            _ => continue,
        };
        let pts = |v: &Vec<crate::math::Vec3>| -> String { v.iter().flat_map(|p| [n(p.x), n(p.y), n(p.z)]).collect::<Vec<_>>().join(" ") };
        let mut line = format!("nurbs {} -u {}", q(&node.name), q(&dirs[0]));
        if let Some(v) = dirs.get(1) {
            line.push_str(&format!(" -v {}", q(v)));
        }
        line.push_str(&format!(" -cvs {}", q(&pts(cvs))));
        if let NodeData::Mesh(md) = &node.data {
            if let Some(ns) = &md.nurbs {
                if !ns.surf.trims.is_empty() {
                    let loops: Vec<String> = ns.surf.trims.iter().map(|t| format!("{} {}", t.keep as u8, t.pts.iter().flat_map(|p| [n(p.0), n(p.1)]).collect::<Vec<_>>().join(" "))).collect();
                    line.push_str(&format!(" -trims {}", q(&loops.join("|"))));
                }
            }
        }
        if let NodeData::NurbsCurve(cd) = &node.data {
            if !cd.drivers.is_empty() && cd.driver_base.len() == cd.curve.cvs.len() {
                line.push_str(&format!(" -dbase {}", q(&pts(&cd.driver_base))));
            }
        }
        if let NodeData::NurbsCurve(cd) = &node.data {
            if let Some(os) = &cd.on_surface {
                if scene.node(os.surface).is_some() {
                    line.push_str(&format!(" -onsurface {} -uvs {}", q(&scene.name_of(os.surface)), q(&os.uv.iter().flat_map(|p| [n(p.0), n(p.1)]).collect::<Vec<_>>().join(" "))));
                }
            }
        }
        if let Some(g) = gen {
            let names: Vec<String> = g.inputs.iter().filter(|i| scene.node(**i).is_some()).map(|i| scene.name_of(*i)).collect();
            line.push_str(&format!(" -kind {} -inputs {} -base {}", q(&g.kind), q(&names.join(" ")), q(&pts(base))));
        }
        line.push_str(";\n");
        s.push_str(&line);
        if let NodeData::NurbsCurve(cd) = &node.data {
            for d in &cd.drivers {
                let Some(h) = d.handle.filter(|h| scene.node(*h).is_some()) else { continue };
                let mut l = format!("curveDriver {} -handle {}", q(&node.name), q(&scene.name_of(h)));
                match &d.kind {
                    crate::dag::nodes::CurveDriverKind::Cluster { members, bind_inv } => {
                        let fl: Vec<String> = (0..16).map(|k| n(bind_inv.m[k / 4][k % 4])).collect();
                        l.push_str(&format!(" -kind cluster -members {} -bind {}", q(&members.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(" ")), q(&fl.join(" "))));
                    }
                    crate::dag::nodes::CurveDriverKind::PointOnCurve { param, rest } => {
                        l.push_str(&format!(" -kind poc -param {} -rest {}", n(*param), q(&format!("{} {} {}", n(rest.x), n(rest.y), n(rest.z)))));
                    }
                }
                l.push_str(";\n");
                s.push_str(&l);
            }
        }
    }

    // 2. custom attributes then authored values
    for id in &dump {
        let node = scene.node(*id).unwrap();
        let defaults = default_plugs(&node.data);
        for p in &node.plugs {
            if p.custom {
                let mut line = format!("addAttr {} -ln {} -dv {}", q(&node.name), q(&p.name), n(p.rest));
                if let Some(v) = p.min {
                    line.push_str(&format!(" -min {}", n(v)));
                }
                if let Some(v) = p.max {
                    line.push_str(&format!(" -max {}", n(v)));
                }
                line.push_str(&format!(" -k {} -at {};\n", p.keyable as u8, ["float", "long", "bool"][(p.dtype as usize).min(2)]));
                s.push_str(&line);
                if p.locked {
                    s.push_str(&format!("setAttr -lock 1 {};\n", q(&format!("{}.{}", node.name, p.name))));
                }
                continue;
            }

            if p.source.is_some() || p.readonly {
                continue;
            }
            let default = defaults.iter().find(|d| d.name == p.name).map(|d| d.rest);
            if default.map(|d| (d - p.rest).abs() > 1e-9).unwrap_or(true) {
                s.push_str(&format!("setAttr {} {};\n", q(&format!("{}.{}", node.name, p.name)), n(p.rest)));
            }
        }
        // Channel Control state of built-in attributes, after their values (a locked attribute cannot be set)
        for p in &node.plugs {
            if p.custom {
                continue;
            }
            if let Some(d) = defaults.iter().find(|d| d.name == p.name) {
                if d.keyable != p.keyable {
                    s.push_str(&format!("setAttr -keyable {} {};\n", p.keyable as u8, q(&format!("{}.{}", node.name, p.name))));
                }
            }
            if p.min.is_some() || p.max.is_some() {
                let mut l = format!("attrRange {}", q(&format!("{}.{}", node.name, p.name)));
                if let Some(v) = p.min {
                    l.push_str(&format!(" -min {}", n(v)));
                }
                if let Some(v) = p.max {
                    l.push_str(&format!(" -max {}", n(v)));
                }
                l.push_str(";\n");
                s.push_str(&l);
            }
            if p.locked {
                s.push_str(&format!("setAttr -lock 1 {};\n", q(&format!("{}.{}", node.name, p.name))));
            }
        }
    }

    // 3. material assignments
    for (id, _) in &rows {
        let node = scene.node(*id).unwrap();
        if let NodeData::Mesh(md) = &node.data {
            if let Some(m) = md.material {
                let target = node.parent.map(|p| scene.name_of(p)).unwrap_or_else(|| node.name.clone());
                s.push_str(&format!("assignMaterial {} {};\n", q(&scene.name_of(m)), q(&target)));
            }
        }
    }

    // 3a'. texture zones (per face materials and projections)
    for (id, _) in &rows {
        let node = scene.node(*id).unwrap();
        if let NodeData::Mesh(md) = &node.data {
            if md.face_zone.is_empty() || md.zones.is_empty() {
                continue;
            }
            let mut zs: Vec<String> = Vec::new();
            for z in &md.zones {
                zs.push(z.material.map(|m| scene.name_of(m)).unwrap_or_else(|| "-".to_string()));
                zs.extend([z.uv.mode.to_string(), n(z.uv.tile_u), n(z.uv.tile_v), n(z.uv.off_u), n(z.uv.off_v), n(z.uv.rot_deg)]);
            }
            let fz: Vec<String> = md.face_zone.iter().map(|z| z.to_string()).collect();
            s.push_str(&format!("meshZones {} -zones {} -faces {};\n", q(&node.name), q(&zs.join(" ")), q(&fz.join(" "))));
        }
    }

    // 3a''. editable UVs
    for (id, _) in &rows {
        let node = scene.node(*id).unwrap();
        if let NodeData::Mesh(md) = &node.data {
            if let Some(set) = &md.uv_set {
                let uvs: Vec<String> = set.uvs.iter().flat_map(|p| [n(p.x), n(p.y)]).collect();
                let rows: Vec<String> = set.fuv.iter().map(|f| f.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(" ")).collect();
                s.push_str(&format!("meshUvs {} -uvs {} -faces {};\n", q(&node.name), q(&uvs.join(" ")), q(&rows.join(","))));
            }
        }
    }

    // 3a. notes
    let mut notes: Vec<(&NodeId, &String)> = scene.notes.iter().collect();
    notes.sort_by(|a, b| scene.name_of(*a.0).cmp(&scene.name_of(*b.0)));
    for (id, text) in notes {
        if scene.node(*id).is_some() && !text.is_empty() {
            s.push_str(&format!("setNotes {} {};\n", q(&scene.name_of(*id)), q(text)));
        }
    }

    // 4. animation layers and curves
    for l in scene.anim.layers.iter().skip(1) {
        let members: Vec<String> = l.members.iter().filter(|(i, _)| scene.node(*i).is_some()).map(|(i, a)| format!("{}.{}", scene.name_of(*i), a)).collect();
        let wkeys = if l.weight_curve.is_empty() { String::new() } else { crate::anim::trax::curve_to_text(&l.weight_curve) };
        s.push_str(&format!(
            "animLayer -n {} -w {} -mute {} -members {} -wkeys {};\n",
            q(&l.name),
            n(l.weight),
            if l.mute { 1 } else { 0 },
            q(&members.join(" ")),
            q(&wkeys)
        ));
    }
    for b in &scene.anim.bindings {
        let layer = &scene.anim.layers[b.layer.min(scene.anim.layers.len() - 1)].name;
        for k in &b.curve.keys {
            let default = (if k.interp == crate::anim::Interp::Linear { crate::anim::keyframes::Tan::Linear } else { crate::anim::keyframes::Tan::Clamped }, k.interp.tan());
            let tans = if (k.in_tan, k.out_tan) != default { format!(" -it {} -ot {}", q(k.in_tan.name()), q(k.out_tan.name())) } else { String::new() };
            s.push_str(&format!(
                "setKeyframe -t {} -v {} -ip {}{} -animLayer {} {};\n",
                n(k.t),
                n(k.v),
                q(k.interp.name()),
                tans,
                q(layer),
                q(&format!("{}.{}", scene.name_of(b.node), b.attr))
            ));
        }
        if b.curve.pre != crate::anim::keyframes::Infinity::Constant || b.curve.post != crate::anim::keyframes::Infinity::Constant {
            s.push_str(&format!("setInfinity -pri {} -poi {} -animLayer {} {};\n", q(b.curve.pre.name()), q(b.curve.post.name()), q(layer), q(&format!("{}.{}", scene.name_of(b.node), b.attr))));
        }
    }

    // non-linear animation: clip sources, clips, blends and redirects
    for src in &scene.trax.sources {
        s.push_str(&format!("clipSource {} -char {} -dur {};\n", q(&src.name), q(&src.character), n(src.duration)));
        for c in &src.channels {
            s.push_str(&format!("clipChannel {} {} {} {};\n", q(&src.name), q(&c.node), q(&c.attr), q(&crate::anim::trax::curve_to_text(&c.curve))));
        }
    }
    for c in &scene.trax.clips {
        let warp = c.warp.as_ref().map(crate::anim::trax::curve_to_text).unwrap_or_default();
        s.push_str(&format!(
            "clip {} -source {} -char {} -track {} -start {} -in {} -out {} -scale {} -cycles {} -cycleOffset {} -relative {} -weight {} -enabled {} -warp {};\n",
            q(&c.name), q(&c.source), q(&c.character), c.track, n(c.start), n(c.src_in), n(c.src_out), n(c.scale), n(c.cycles), c.cycle_offset as u8, c.relative as u8, n(c.weight), c.enabled as u8, q(&warp)
        ));
    }
    for b in &scene.trax.blends {
        s.push_str(&format!("clipBlend {} {} -shape {};\n", q(&b.a), q(&b.b), b.shape));
    }
    for r in &scene.redirects {
        if scene.node(r.node).is_some() && scene.node(r.control).is_some() {
            s.push_str(&format!("redirect {} -control {} -t {} -r {} -pivot {};\n", q(&scene.name_of(r.node)), q(&scene.name_of(r.control)), r.translate as u8, r.rotate as u8, q(&format!("{} {} {}", n(r.pivot.x), n(r.pivot.y), n(r.pivot.z)))));
        }
    }
    if let Some(l) = scene.live.filter(|l| scene.node(*l).is_some()) {
        s.push_str(&format!("makeLive {};\n", q(&scene.name_of(l))));
    }
    // referenced files (their nodes are saved above; the link lets them be unloaded and reloaded)
    for r in &scene.references {
        if r.loaded {
            s.push_str(&format!("referenceInfo {} {};\n", q(&r.path), q(&r.namespace)));
        }
    }

    // 5. connections
    for (src, dst) in scene.connections() {
        // a curve node's input-to-output link is implied by the node itself
        if dst.attr == "output" && matches!(scene.node(dst.node).map(|n| &n.data), Some(NodeData::DrivenCurve(_))) {
            continue;
        }
        s.push_str(&format!(
            "connectAttr {} {};\n",
            q(&format!("{}.{}", scene.name_of(src.node), src.attr)),
            q(&format!("{}.{}", scene.name_of(dst.node), dst.attr))
        ));
    }
    for l in &scene.render_layers {
        let names: Vec<String> = l.members.iter().filter(|m| scene.node(**m).is_some()).map(|m| scene.name_of(*m)).collect();
        let ov: Vec<String> = l.overrides.iter().flat_map(|(k, v)| [k.clone(), n(*v)]).collect();
        let tov: Vec<String> = l.text_overrides.iter().flat_map(|(k, v)| [k.clone(), v.clone()]).collect();
        s.push_str(&format!("renderLayer {} -renderable {} -members {} -passes {} -overrides {} -textoverrides {};\n", q(&l.name), l.renderable as u8, q(&names.join(" ")), q(&l.passes.join(" ")), q(&ov.join(" ")), q(&tov.join("\t"))));
    }
    for e in &scene.expressions {
        s.push_str(&format!("expression {} -s {} -o {} -en {};\n", q(&e.name), q(&e.text), q(&e.object), e.enabled as u8));
    }
    for (light, objs) in &scene.light_links {
        if scene.node(*light).is_none() {
            continue;
        }
        let names: Vec<String> = objs.iter().filter(|m| scene.node(**m).is_some()).map(|m| scene.name_of(*m)).collect();
        s.push_str(&format!("lightLink {} -objects {};\n", q(&scene.name_of(*light)), q(&names.join(" "))));
    }
    s.push_str("//End of scene\n");
    s
}

const CREATE_NODE: &[FlagSpec] = &[
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["p", "parent"], arity: 1 },
    FlagSpec { names: &["prim"], arity: 1 },
    FlagSpec { names: &["params"], arity: 1 },
    FlagSpec { names: &["verts"], arity: 1 },
    FlagSpec { names: &["faces"], arity: 1 },
    FlagSpec { names: &["kind"], arity: 1 },
    FlagSpec { names: &["ints"], arity: 1 },
    FlagSpec { names: &["history"], arity: 1 },
    FlagSpec { names: &["skin"], arity: 1 },
    FlagSpec { names: &["bind"], arity: 1 },
    FlagSpec { names: &["weights"], arity: 1 },
    FlagSpec { names: &["skinopts"], arity: 1 },
    FlagSpec { names: &["start"], arity: 1 },
    FlagSpec { names: &["end"], arity: 1 },
    FlagSpec { names: &["targets"], arity: 1 },
    FlagSpec { names: &["offsets"], arity: 1 },
    FlagSpec { names: &["keys"], arity: 1 },
    FlagSpec { names: &["instanceOf"], arity: 1 },
    FlagSpec { names: &["members"], arity: 1 },
    FlagSpec { names: &["curve"], arity: 1 },
    FlagSpec { names: &["path"], arity: 1 },
    FlagSpec { names: &["solver"], arity: 1 },
    FlagSpec { names: &["pole"], arity: 1 },
    FlagSpec { names: &["divs"], arity: 1 },
    FlagSpec { names: &["lattice"], arity: 1 },
    FlagSpec { names: &["channels"], arity: 1 },
    FlagSpec { names: &["handle"], arity: 1 },
    FlagSpec { names: &["prefix"], arity: 1 },
    FlagSpec { names: &["dyn"], arity: 1 },
    FlagSpec { names: &["ext"], arity: 1 },
    FlagSpec { names: &["camera"], arity: 1 },
    FlagSpec { names: &["strokebrush"], arity: 1 },
    FlagSpec { names: &["strokeseed"], arity: 1 },
    FlagSpec { names: &["strokepath"], arity: 1 },
    FlagSpec { names: &["strokemats"], arity: 1 },
    FlagSpec { names: &["bindinv"], arity: 1 },
    FlagSpec { names: &["basecurve"], arity: 1 },
    FlagSpec { names: &["basecvs"], arity: 1 },
    FlagSpec { names: &["center"], arity: 1 },
    FlagSpec { names: &["dweights"], arity: 1 },
];
const ADD_ATTR: &[FlagSpec] = &[
    FlagSpec { names: &["ln", "longName"], arity: 1 },
    FlagSpec { names: &["dv", "defaultValue"], arity: 1 },
    FlagSpec { names: &["min"], arity: 1 },
    FlagSpec { names: &["max"], arity: 1 },
    FlagSpec { names: &["k"], arity: 1 },
    FlagSpec { names: &["at"], arity: 1 },
];
const ANIM_LAYER: &[FlagSpec] = &[
    FlagSpec { names: &["n", "name"], arity: 1 },
    FlagSpec { names: &["w", "weight"], arity: 1 },
    FlagSpec { names: &["mute"], arity: 1 },
    FlagSpec { names: &["members"], arity: 1 },
    FlagSpec { names: &["wkeys"], arity: 1 },
];
const SET_KEY: &[FlagSpec] = &[
    FlagSpec { names: &["t", "time"], arity: 1 },
    FlagSpec { names: &["v", "value"], arity: 1 },
    FlagSpec { names: &["ip", "interp"], arity: 1 },
    FlagSpec { names: &["it"], arity: 1 },
    FlagSpec { names: &["ot"], arity: 1 },
    FlagSpec { names: &["animLayer", "layer"], arity: 1 },
];
const PLAYBACK: &[FlagSpec] = &[
    FlagSpec { names: &["min"], arity: 1 },
    FlagSpec { names: &["max"], arity: 1 },
    FlagSpec { names: &["fps"], arity: 1 },
    FlagSpec { names: &["current"], arity: 1 },
];

fn arg(p: &Parsed, i: usize, line: usize, what: &str) -> Result<String, String> {
    p.args
        .get(i)
        .map(|v| v.as_string())
        .ok_or_else(|| format!("Line {}: missing {}", line, what))
}

fn parse_params(s: &str) -> Vec<(String, f32)> {
    s.split_whitespace()
        .filter_map(|kv| {
            let mut it = kv.splitn(2, '=');
            let k = it.next()?;
            let v = it.next()?.parse::<f32>().ok()?;
            Some((k.to_string(), v))
        })
        .collect()
}

/// Parse `.aztec` ASCII into a brand new scene.
pub fn from_ascii(src: &str) -> Result<Scene, String> {
    let mut scene = Scene::empty();
    load_into(&mut scene, src, false)?;
    Ok(scene)
}

/// Parse `.aztec` ASCII into an existing scene. As a reference (`reference`), the scene's name prefix is in
/// effect so every node gets it, and the file's time settings are not applied.
pub fn load_into(scene: &mut Scene, src: &str, reference: bool) -> Result<(), String> {
    let stmts = parse_script(src)?;
    let (mut pb_min, mut pb_max, mut pb_fps, mut pb_cur) = (None, None, None, None);
    let mut instances: Vec<(String, String)> = Vec::new();
    for st in stmts {
        let line = st.line;
        let cmd = match st.tokens.first() {
            Some(crate::mel::lexer::Token::Word(w)) => w.clone(),
            _ => return Err(format!("Line {}: expected a command", line)),
        };
        let rest = &st.tokens[1..];
        let ctx = |e: String| format!("Line {}: {}", line, e);
        match cmd.as_str() {
            "requires" | "fileInfo" => {}
            "referenceInfo" => {
                // a saved reference link: the nodes are already in the file
                let p = parse_args(rest, &[]).map_err(ctx)?;
                let (path, ns) = (arg(&p, 0, line, "path")?, arg(&p, 1, line, "namespace")?);
                scene.references.push(crate::dag::graph::Reference { path, namespace: ns, loaded: true });
            }
            "playbackOptions" => {
                let p = parse_args(rest, PLAYBACK).map_err(ctx)?;
                pb_min = p.num("min").map_err(ctx)?.or(pb_min);
                pb_max = p.num("max").map_err(ctx)?.or(pb_max);
                pb_fps = p.num("fps").map_err(ctx)?.or(pb_fps);
                pb_cur = p.num("current").map_err(ctx)?.or(pb_cur);
            }
            "createNode" => {
                let p = parse_args(rest, CREATE_NODE).map_err(ctx)?;
                let ty = arg(&p, 0, line, "node type")?;
                let name = p.string("n").ok_or_else(|| format!("Line {}: createNode needs -n", line))?;
                let parent = match p.string("p") {
                    Some(pn) => Some(
                        scene
                            .find(&pn)
                            .ok_or_else(|| format!("Line {}: parent '{}' does not exist", line, pn))?,
                    ),
                    None => None,
                };
                let geometry = |p: &Parsed| -> Result<crate::geo::Mesh, String> {
                    let nums: Vec<f32> = p.string("verts").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                    let mut m = crate::geo::Mesh::new();
                    for c in nums.chunks_exact(3) {
                        m.add_vertex(crate::math::Vec3::new(c[0], c[1], c[2]), crate::math::Vec3::UP, crate::math::Vec2::new(0.0, 0.0));
                    }
                    let ids: Vec<u32> = p.string("faces").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                    let mut i = 0usize;
                    while i < ids.len() {
                        let cnt = ids[i] as usize;
                        let end = i + 1 + cnt;
                        if end > ids.len() {
                            return Err("truncated face list".to_string());
                        }
                        let f = &ids[i + 1..end];
                        if f.iter().any(|&v| v as usize >= m.positions.len()) {
                            return Err("face index out of range".to_string());
                        }
                        m.add_face(f);
                        i = end;
                    }
                    Ok(m)
                };
                let data = if ty == "polyOp" {
                    let kind_name = p.string("kind").unwrap_or_else(|| "source".to_string());
                    let kind = PolyKind::from_name(&kind_name).ok_or_else(|| format!("Line {}: unknown history kind '{}'", line, kind_name))?;
                    let mut op = PolyOpData::new(kind);
                    op.lattice_name = p.string("lattice").unwrap_or_default();
                    if p.string("handle").is_some() || p.string("curve").is_some() || p.string("center").is_some() || p.string("dweights").is_some() {
                        let nums = |k: &str| -> Vec<f32> { p.string(k).unwrap_or_default().split_whitespace().filter_map(|x| x.parse().ok()).collect() };
                        let bi = nums("bindinv");
                        let bind_inv = if bi.len() == 16 {
                            let mut m = crate::math::Mat4::identity();
                            for k in 0..16 {
                                m.m[k / 4][k % 4] = bi[k];
                            }
                            Some(m)
                        } else {
                            None
                        };
                        let base_curve = match (p.string("basecurve"), nums("basecvs")) {
                            (Some(d), cv) if !cv.is_empty() => {
                                let toks: Vec<&str> = d.split_whitespace().collect();
                                if toks.len() > 3 {
                                    let dir = crate::geo::nurbs::Dir { degree: toks[0].parse().unwrap_or(3), n: toks[1].parse().unwrap_or(0), periodic: toks[2] == "1", knots: toks[3..].iter().filter_map(|k| k.parse().ok()).collect() };
                                    Some(crate::geo::nurbs::Curve { dir, cvs: cv.chunks_exact(3).map(|c| crate::math::Vec3::new(c[0], c[1], c[2])).collect() })
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        let c = nums("center");
                        op.ext = Some(Box::new(crate::dag::nodes::DeformExt {
                            handle: None,
                            handle_name: p.string("handle").unwrap_or_default(),
                            bind_inv,
                            curve: None,
                            curve_name: p.string("curve").unwrap_or_default(),
                            base_curve,
                            center: if c.len() == 3 { Some(crate::math::Vec3::new(c[0], c[1], c[2])) } else { None },
                            weights: nums("dweights"),
                            jiggle: None,
                        }));
                    }
                    op.ints = p.string("ints").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                    if kind == PolyKind::Source || kind == PolyKind::Blend {
                        let m = geometry(&p).map_err(|e| format!("Line {}: {}", line, e))?;
                        op.source = Some(crate::geo::edit::Poly::from_mesh(&m));
                    }
                    if kind == PolyKind::Skin {
                        let names: Vec<String> = p.string("skin").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                        let fl: Vec<f32> = p.string("bind").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                        let mat = |i: usize| -> crate::math::Mat4 {
                            let mut m = crate::math::Mat4::identity();
                            for c in 0..4 {
                                for r in 0..4 {
                                    m.m[c][r] = fl.get(i * 16 + c * 4 + r).copied().unwrap_or(if c == r { 1.0 } else { 0.0 });
                                }
                            }
                            m
                        };
                        let bind_inv: Vec<crate::math::Mat4> = (0..names.len()).map(mat).collect();
                        let bind_mesh_world = mat(names.len());
                        let w: Vec<f32> = p.string("weights").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                        let mut weights: Vec<Vec<(u16, f32)>> = Vec::new();
                        let mut i = 0usize;
                        while i < w.len() {
                            let cnt = w[i] as usize;
                            i += 1;
                            let mut v = Vec::new();
                            for _ in 0..cnt {
                                if i + 2 > w.len() {
                                    break;
                                }
                                v.push((w[i] as u16, w[i + 1]));
                                i += 2;
                            }
                            weights.push(v);
                        }
                        let opts: Vec<String> = p.string("skinopts").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                        let (max_inf, dropoff, rigid) = (opts.first().and_then(|x| x.parse().ok()).unwrap_or(4), opts.get(1).and_then(|x| x.parse().ok()).unwrap_or(4.0), opts.get(2).map(|x| x == "1").unwrap_or(false));
                        op.skin = Some(crate::dag::nodes::SkinData { joints: Vec::new(), joint_names: names, bind_inv, bind_mesh_world, weights, max_inf, dropoff, rigid });
                    }
                    NodeData::PolyOp(Box::new(op))
                } else if ty == "constraint" {
                    let kind_name = p.string("kind").unwrap_or_else(|| "point".to_string());
                    let kind = crate::dag::nodes::ConstraintKind::from_name(&kind_name).ok_or_else(|| format!("Line {}: unknown constraint kind '{}'", line, kind_name))?;
                    let target_names: Vec<String> = p.string("targets").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                    let fl: Vec<f32> = p.string("offsets").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                    let offsets: Vec<crate::math::Mat4> = fl
                        .chunks_exact(16)
                        .map(|c| {
                            let mut m = crate::math::Mat4::identity();
                            for i in 0..16 {
                                m.m[i / 4][i % 4] = c[i];
                            }
                            m
                        })
                        .collect();
                    NodeData::Constraint(Box::new(crate::dag::nodes::ConstraintData { kind, targets: Vec::new(), target_names, offsets }))
                } else if ty == "character" {
                    let channel_names: Vec<(String, String)> = p.string("channels").unwrap_or_default().split_whitespace().filter_map(|c| c.rsplit_once('.').map(|(a, b)| (a.to_string(), b.to_string()))).collect();
                    NodeData::Character(Box::new(crate::dag::nodes::CharacterData { channels: Vec::new(), channel_names }))
                } else if ty == "distanceDimension" {
                    NodeData::Distance(Box::new(crate::dag::nodes::DistanceData { start: None, end: None, start_name: p.string("start").unwrap_or_default(), end_name: p.string("end").unwrap_or_default() }))
                } else if ty == "lattice" {
                    let d: Vec<usize> = p.string("divs").unwrap_or_default().split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    if d.len() != 3 || d.iter().any(|x| *x < 2 || *x > 12) {
                        return Err(format!("Line {}: lattice needs -divs S T U", line));
                    }
                    let names: Vec<String> = p.string("skin").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                    let skin = if names.is_empty() {
                        None
                    } else {
                        let fl: Vec<f32> = p.string("bind").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                        let mat = |i: usize| -> crate::math::Mat4 {
                            let mut m = crate::math::Mat4::identity();
                            if fl.len() >= (i + 1) * 16 {
                                for k in 0..16 {
                                    m.m[k / 4][k % 4] = fl[i * 16 + k];
                                }
                            }
                            m
                        };
                        let bind_inv: Vec<crate::math::Mat4> = (0..names.len()).map(&mat).collect();
                        let w: Vec<f32> = p.string("weights").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                        let mut weights: Vec<Vec<(u16, f32)>> = Vec::new();
                        let mut i = 0;
                        while i < w.len() {
                            let c = w[i] as usize;
                            let mut v = Vec::new();
                            i += 1;
                            for _ in 0..c {
                                if i + 1 >= w.len() {
                                    break;
                                }
                                v.push((w[i] as u16, w[i + 1]));
                                i += 2;
                            }
                            weights.push(v);
                        }
                        Some(crate::dag::nodes::LatticeSkin { joints: Vec::new(), joint_names: names.clone(), bind_world: mat(names.len()), bind_inv, weights })
                    };
                    NodeData::Lattice(Box::new(crate::dag::nodes::LatticeData { divs: [d[0], d[1], d[2]], skin }))
                } else if matches!(ty.as_str(), "rigidBody" | "rigidSolver" | "field" | "rigidConstraint" | "particleEmitter" | "particle") {
                    parse_dyn(&ty, &p.string("dyn").unwrap_or_default())
                } else if ty == "renderGlobals" {
                    NodeData::RenderGlobals(Box::new(crate::dag::nodes::RenderGlobalsData { prefix: p.string("prefix").unwrap_or_default(), ext: p.string("ext").unwrap_or_else(|| "png".into()), camera: p.string("camera").unwrap_or_default() }))
                } else if ty == "brush" {
                    NodeData::Brush(Box::default())
                } else if ty == "motionPath" {
                    NodeData::MotionPath(Box::new(crate::dag::nodes::MotionPathData { curve: None, curve_name: p.string("curve").unwrap_or_default() }))
                } else if ty == "sound" {
                    // the waveform is read again from the file; a missing file keeps the node without one
                    let path = p.string("path").unwrap_or_default();
                    NodeData::Sound(Box::new(crate::anim::sound::load_wav(&path).unwrap_or(crate::dag::nodes::SoundData { path, ..Default::default() })))
                } else if ty == "texture" {
                    NodeData::Texture(Box::new(crate::dag::nodes::TextureData { kind: p.string("kind").unwrap_or_else(|| "file".to_string()), path: p.string("path").unwrap_or_default() }))
                } else if ty == "objectSet" {
                    let member_names: Vec<String> = p.string("members").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                    NodeData::ObjectSet(Box::new(crate::dag::nodes::SetData { members: Vec::new(), member_names }))
                } else if ty == "animCurveUU" {
                    let toks: Vec<String> = p.string("keys").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                    let mut cv = crate::anim::keyframes::Curve::new();
                    for c in toks.chunks_exact(3) {
                        if let (Ok(t), Ok(v), Some(ip)) = (c[0].parse::<f32>(), c[1].parse::<f32>(), Interp::parse(&c[2])) {
                            cv.set_key(t, v, ip);
                        }
                    }
                    NodeData::DrivenCurve(Box::new(cv))
                } else if ty == "ikHandle" {
                    NodeData::IkHandle(Box::new(crate::dag::nodes::IkData {
                        start: None,
                        end: None,
                        start_name: p.string("start").unwrap_or_default(),
                        end_name: p.string("end").unwrap_or_default(),
                        solver: p.num("solver").map_err(ctx)?.unwrap_or(0.0) as u8,
                        curve: None,
                        curve_name: p.string("curve").unwrap_or_default(),
                        pole: None,
                        pole_name: p.string("pole").unwrap_or_default(),
                    }))
                } else if ty == "mesh" {
                    let prim = p.string("prim").ok_or_else(|| format!("Line {}: mesh needs -prim", line))?;
                    let params = parse_params(&p.string("params").unwrap_or_default());
                    let mesh = if prim == "history" {
                        crate::geo::Mesh::new()
                    } else if prim == "custom" {
                        let nums: Vec<f32> = p.string("verts").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                        let mut m = crate::geo::Mesh::new();
                        for c in nums.chunks_exact(3) {
                            m.add_vertex(
                                crate::math::Vec3::new(c[0], c[1], c[2]),
                                crate::math::Vec3::UP,
                                crate::math::Vec2::new(0.0, 0.0),
                            );
                        }
                        let ids: Vec<u32> = p.string("faces").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                        let mut i = 0usize;
                        while i < ids.len() {
                            let cnt = ids[i] as usize;
                            let end = i + 1 + cnt;
                            if end > ids.len() {
                                return Err(format!("Line {}: truncated face list", line));
                            }
                            let f = &ids[i + 1..end];
                            if f.iter().any(|&v| v as usize >= m.positions.len()) {
                                return Err(format!("Line {}: face index out of range", line));
                            }
                            m.add_face(f);
                            i = end;
                        }
                        m.recompute_normals();
                        m
                    } else {
                        build_mesh(&prim, &params).ok_or_else(|| format!("Line {}: unknown primitive '{}'", line, prim))?
                    };
                    NodeData::Mesh(MeshData { cache: Default::default(), history: Vec::new(), mesh, prim, params, material: None, ..Default::default() })
                } else {
                    NodeData::from_type_name(&ty).ok_or_else(|| format!("Line {}: unknown node type '{}'", line, ty))?
                };
                let is_poly = matches!(data, NodeData::PolyOp(_));
                let hist_names = p.string("history");
                let legacy: Option<(String, Vec<(String, f32)>)> = match &data {
                    NodeData::Mesh(md) if md.prim != "custom" && md.prim != "history" && hist_names.is_none() => {
                        Some((md.prim.clone(), md.params.clone()))
                    }
                    _ => None,
                };
                let mut data = data;
                if p.string("strokepath").is_some() {
                    let nums: Vec<f32> = p.string("strokepath").unwrap_or_default().split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    if let NodeData::Mesh(md) = &mut data {
                        md.stroke = Some(Box::new(crate::dag::nodes::StrokeData {
                            brush: None,
                            brush_name: p.string("strokebrush").unwrap_or_default(),
                            path: nums.chunks_exact(6).map(|c| (crate::math::Vec3::new(c[0], c[1], c[2]), crate::math::Vec3::new(c[3], c[4], c[5]))).collect(),
                            seed: p.string("strokeseed").and_then(|x| x.parse().ok()).unwrap_or(1),
                            key: 0,
                            mats: [None; 3],
                            mat_names: p.string("strokemats").unwrap_or_default().split(' ').map(|x| x.to_string()).collect(),
                        }));
                    }
                }
                let nid = scene.create_node(data, &name, parent, false);
                if let Some(src) = p.string("instanceOf") {
                    instances.push((scene.name_of(nid), src));
                }
                if is_poly {
                    scene.roots.retain(|r| *r != nid);
                }
                if let Some(h) = hist_names {
                    let ids: Vec<NodeId> = h.split_whitespace().filter_map(|n| scene.find(n)).collect();
                    for hid in &ids {
                        if let Some(NodeData::PolyOp(op)) = scene.nodes.get_mut(*hid).map(|n| &mut n.data) {
                            op.owner = Some(nid);
                        }
                    }
                    if let Some(NodeData::Mesh(md)) = scene.nodes.get_mut(nid).map(|n| &mut n.data) {
                        md.history = ids;
                        md.prim = "history".to_string();
                    }
                } else if let Some((prim, params)) = legacy {
                    // files from before construction history: rebuild the primitive as a history node
                    let kind = match prim.as_str() {
                        "polyCube" => Some(PolyKind::Cube),
                        "polySphere" => Some(PolyKind::Sphere),
                        "polyCylinder" => Some(PolyKind::Cylinder),
                        "polyPlane" => Some(PolyKind::Plane),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        let h = scene.new_poly_node(kind, Vec::new(), None, Some(nid));
                        for (k, v) in &params {
                            if let Some(pl) = scene.nodes.get_mut(h).and_then(|n| n.plug_mut(k)) {
                                pl.value = *v;
                                pl.rest = *v;
                            }
                        }
                        if let Some(NodeData::Mesh(md)) = scene.nodes.get_mut(nid).map(|n| &mut n.data) {
                            md.history = vec![h];
                            md.prim = "history".to_string();
                            md.params.clear();
                        }
                    }
                }
            }
            "attrRange" => {
                let p = parse_args(rest, &[FlagSpec { names: &["min"], arity: 1 }, FlagSpec { names: &["max"], arity: 1 }]).map_err(ctx)?;
                let path = arg(&p, 0, line, "attribute path")?;
                let (id, attr) = scene.resolve_plug(&path).map_err(ctx)?;
                let (lo, hi) = (p.num("min").map_err(ctx)?, p.num("max").map_err(ctx)?);
                if let Some(pl) = scene.nodes.get_mut(id).and_then(|n| n.plug_mut(&attr)) {
                    pl.min = lo;
                    pl.max = hi;
                }
            }
            "clipSource" => {
                let p = parse_args(rest, &[FlagSpec { names: &["char"], arity: 1 }, FlagSpec { names: &["dur"], arity: 1 }]).map_err(ctx)?;
                let name = arg(&p, 0, line, "source name")?;
                scene.trax.sources.push(crate::anim::trax::ClipSource { name, character: p.string("char").unwrap_or_default(), duration: p.num("dur").map_err(ctx)?.unwrap_or(0.0), channels: Vec::new() });
            }
            "clipChannel" => {
                let p = parse_args(rest, &[]).map_err(ctx)?;
                let (src, node, attr, curve) = (arg(&p, 0, line, "source")?, arg(&p, 1, line, "node")?, arg(&p, 2, line, "attribute")?, arg(&p, 3, line, "curve")?);
                if let Some(s) = scene.trax.sources.iter_mut().find(|s| s.name == src) {
                    s.channels.push(crate::anim::trax::SrcChannel { node, attr, curve: crate::anim::trax::curve_from_text(&curve) });
                }
            }
            "clip" => {
                let p = parse_args(rest, &[FlagSpec { names: &["source"], arity: 1 }, FlagSpec { names: &["char"], arity: 1 }, FlagSpec { names: &["track"], arity: 1 }, FlagSpec { names: &["start"], arity: 1 }, FlagSpec { names: &["in"], arity: 1 }, FlagSpec { names: &["out"], arity: 1 }, FlagSpec { names: &["scale"], arity: 1 }, FlagSpec { names: &["cycles"], arity: 1 }, FlagSpec { names: &["cycleOffset"], arity: 1 }, FlagSpec { names: &["relative"], arity: 1 }, FlagSpec { names: &["weight"], arity: 1 }, FlagSpec { names: &["enabled"], arity: 1 }, FlagSpec { names: &["warp"], arity: 1 }]).map_err(ctx)?;
                let name = arg(&p, 0, line, "clip name")?;
                let g = |k: &str, d: f32| -> Result<f32, String> { Ok(p.num(k).map_err(ctx)?.unwrap_or(d)) };
                let mut c = crate::anim::trax::Clip::new(&name, &p.string("source").unwrap_or_default(), &p.string("char").unwrap_or_default(), g("start", 1.0)?, 1.0);
                c.track = g("track", 0.0)? as usize;
                c.src_in = g("in", 0.0)?;
                c.src_out = g("out", 1.0)?;
                c.scale = g("scale", 1.0)?;
                c.cycles = g("cycles", 1.0)?;
                c.cycle_offset = g("cycleOffset", 0.0)? > 0.5;
                c.relative = g("relative", 0.0)? > 0.5;
                c.weight = g("weight", 1.0)?;
                c.enabled = g("enabled", 1.0)? > 0.5;
                c.warp = p.string("warp").filter(|w| !w.is_empty()).map(|w| crate::anim::trax::curve_from_text(&w));
                scene.trax.clips.push(c);
            }
            "clipBlend" => {
                let p = parse_args(rest, &[FlagSpec { names: &["shape"], arity: 1 }]).map_err(ctx)?;
                let (a, b) = (arg(&p, 0, line, "first clip")?, arg(&p, 1, line, "second clip")?);
                scene.trax.blends.push(crate::anim::trax::Blend { a, b, shape: p.num("shape").map_err(ctx)?.unwrap_or(0.0) as u8 });
            }
            "redirect" => {
                let p = parse_args(rest, &[FlagSpec { names: &["control"], arity: 1 }, FlagSpec { names: &["t"], arity: 1 }, FlagSpec { names: &["r"], arity: 1 }, FlagSpec { names: &["pivot"], arity: 1 }]).map_err(ctx)?;
                let node = arg(&p, 0, line, "node")?;
                if let (Some(n), Some(c)) = (scene.find(&node), p.string("control").and_then(|c| scene.find(&c))) {
                    let pv: Vec<f32> = p.string("pivot").unwrap_or_default().split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    scene.redirects.push(crate::dag::trax::Redirect { node: n, control: c, translate: p.num("t").map_err(ctx)?.unwrap_or(1.0) > 0.5, rotate: p.num("r").map_err(ctx)?.unwrap_or(0.0) > 0.5, pivot: if pv.len() == 3 { crate::math::Vec3::new(pv[0], pv[1], pv[2]) } else { crate::math::Vec3::ZERO } });
                }
            }
            "makeLive" => {
                let p = parse_args(rest, &[]).map_err(ctx)?;
                let name = arg(&p, 0, line, "surface")?;
                scene.live = scene.find(&name);
            }
            "expression" => {
                let p = parse_args(rest, &[FlagSpec { names: &["s"], arity: 1 }, FlagSpec { names: &["o"], arity: 1 }, FlagSpec { names: &["en"], arity: 1 }]).map_err(ctx)?;
                let name = arg(&p, 0, line, "expression name")?;
                scene.add_expression_loaded(&name, &p.string("s").unwrap_or_default(), &p.string("o").unwrap_or_default(), p.num("en").map_err(ctx)?.unwrap_or(1.0) > 0.5).map_err(ctx)?;
            }
            "renderLayer" => {
                let p = parse_args(rest, &[FlagSpec { names: &["renderable"], arity: 1 }, FlagSpec { names: &["members"], arity: 1 }, FlagSpec { names: &["passes"], arity: 1 }, FlagSpec { names: &["overrides"], arity: 1 }, FlagSpec { names: &["textoverrides"], arity: 1 }]).map_err(ctx)?;
                let name = arg(&p, 0, line, "layer name")?;
                let toks: Vec<String> = p.string("overrides").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                let overrides = toks.chunks_exact(2).filter_map(|c| Some((c[0].clone(), c[1].parse().ok()?))).collect();
                let tt: Vec<String> = p.string("textoverrides").unwrap_or_default().split('\t').map(|x| x.to_string()).collect();
                let text_overrides = tt.chunks_exact(2).map(|c| (c[0].clone(), c[1].clone())).collect();
                let members = p.string("members").unwrap_or_default().split_whitespace().filter_map(|m| scene.find(m)).collect();
                scene.render_layers.push(crate::dag::nodes::RenderLayerData { name, members, renderable: p.string("renderable").as_deref() != Some("0"), passes: p.string("passes").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect(), overrides, text_overrides });
            }
            "lightLink" => {
                let p = parse_args(rest, &[FlagSpec { names: &["objects"], arity: 1 }]).map_err(ctx)?;
                let name = arg(&p, 0, line, "light")?;
                if let Some(l) = scene.find(&name) {
                    let objs = p.string("objects").unwrap_or_default().split_whitespace().filter_map(|m| scene.find(m)).collect();
                    scene.light_links.push((l, objs));
                }
            }
            "curveDriver" => {
                let p = parse_args(rest, &[FlagSpec { names: &["handle"], arity: 1 }, FlagSpec { names: &["kind"], arity: 1 }, FlagSpec { names: &["members"], arity: 1 }, FlagSpec { names: &["bind"], arity: 1 }, FlagSpec { names: &["param"], arity: 1 }, FlagSpec { names: &["rest"], arity: 1 }]).map_err(ctx)?;
                let name = arg(&p, 0, line, "curve")?;
                let id = scene.find(&name).ok_or_else(|| format!("Line {}: no curve '{}'", line, name))?;
                let nums = |k: &str| -> Vec<f32> { p.string(k).unwrap_or_default().split_whitespace().filter_map(|x| x.parse().ok()).collect() };
                let kind = match p.string("kind").as_deref() {
                    Some("cluster") => {
                        let b = nums("bind");
                        let mut m = crate::math::Mat4::identity();
                        if b.len() == 16 {
                            for k in 0..16 {
                                m.m[k / 4][k % 4] = b[k];
                            }
                        }
                        crate::dag::nodes::CurveDriverKind::Cluster { members: nums("members").iter().map(|x| *x as usize).collect(), bind_inv: m }
                    }
                    _ => {
                        let r = nums("rest");
                        crate::dag::nodes::CurveDriverKind::PointOnCurve { param: p.num("param").map_err(ctx)?.unwrap_or(0.0), rest: if r.len() == 3 { crate::math::Vec3::new(r[0], r[1], r[2]) } else { crate::math::Vec3::ZERO } }
                    }
                };
                if let Some(NodeData::NurbsCurve(cd)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                    cd.drivers.push(crate::dag::nodes::CurveDriver { kind, handle: None, handle_name: p.string("handle").unwrap_or_default() });
                }
            }
            "nurbs" => {
                let p = parse_args(rest, &[FlagSpec { names: &["u"], arity: 1 }, FlagSpec { names: &["v"], arity: 1 }, FlagSpec { names: &["cvs"], arity: 1 }, FlagSpec { names: &["kind"], arity: 1 }, FlagSpec { names: &["inputs"], arity: 1 }, FlagSpec { names: &["base"], arity: 1 }, FlagSpec { names: &["trims"], arity: 1 }, FlagSpec { names: &["onsurface"], arity: 1 }, FlagSpec { names: &["uvs"], arity: 1 }, FlagSpec { names: &["dbase"], arity: 1 }]).map_err(ctx)?;
                let name = arg(&p, 0, line, "shape")?;
                let id = scene.find(&name).ok_or_else(|| format!("Line {}: no shape '{}'", line, name))?;
                let dir = |t: Option<String>| -> Option<crate::geo::nurbs::Dir> {
                    let toks: Vec<&str> = t.as_deref()?.split_whitespace().collect();
                    let degree: usize = toks.first()?.parse().ok()?;
                    let nn: usize = toks.get(1)?.parse().ok()?;
                    let periodic = *toks.get(2)? == "1";
                    let knots: Vec<f32> = toks[3..].iter().filter_map(|k| k.parse().ok()).collect();
                    Some(crate::geo::nurbs::Dir { degree, n: nn, periodic, knots })
                };
                let pts = |t: Option<String>| -> Vec<crate::math::Vec3> {
                    let v: Vec<f32> = t.unwrap_or_default().split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    v.chunks_exact(3).map(|c| crate::math::Vec3::new(c[0], c[1], c[2])).collect()
                };
                let u = dir(p.string("u")).ok_or_else(|| format!("Line {}: bad NURBS direction", line))?;
                let v = dir(p.string("v"));
                let cvs = pts(p.string("cvs"));
                let base = pts(p.string("base"));
                let kind = p.string("kind");
                let input_names: Vec<String> = p.string("inputs").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                let gen = kind.clone().map(|k| crate::dag::nodes::NurbsGen { kind: k, inputs: Vec::new(), input_names });
                match (&v, scene.nodes.get_mut(id).map(|n| &mut n.data)) {
                    (None, Some(NodeData::NurbsCurve(cd))) => {
                        if cvs.len() != u.n || u.knots.len() != u.ext() + u.degree + 1 {
                            return Err(format!("Line {}: inconsistent curve data", line));
                        }
                        cd.driver_base = pts(p.string("dbase"));
                        cd.curve = crate::geo::nurbs::Curve { dir: u, cvs };
                        if let Some(sname) = p.string("onsurface") {
                            let nums: Vec<f32> = p.string("uvs").unwrap_or_default().split_whitespace().filter_map(|x| x.parse().ok()).collect();
                            cd.on_surface = Some(crate::dag::nodes::OnSurface { surface: id, name: sname, uv: nums.chunks_exact(2).map(|c| (c[0], c[1])).collect() });
                        }
                        cd.base = base;
                        cd.gen = gen;
                        cd.key = 0;
                    }
                    (Some(vd), Some(NodeData::Mesh(md))) => {
                        if cvs.len() != u.n * vd.n || u.knots.len() != u.ext() + u.degree + 1 || vd.knots.len() != vd.ext() + vd.degree + 1 {
                            return Err(format!("Line {}: inconsistent surface data", line));
                        }
                        let mut trims = Vec::new();
                        for lp in p.string("trims").unwrap_or_default().split('|').filter(|x| !x.trim().is_empty()) {
                            let nums: Vec<f32> = lp.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                            if nums.len() >= 7 {
                                trims.push(crate::geo::nurbs::Trim { keep: nums[0] > 0.5, pts: nums[1..].chunks_exact(2).map(|c| (c[0], c[1])).collect() });
                            }
                        }
                        md.nurbs = Some(Box::new(crate::dag::nodes::NurbsSurf { surf: crate::geo::nurbs::Surface { u, v: vd.clone(), cvs, trims }, gen, base, key: 0, iso: Vec::new() }));
                        md.prim = "nurbs".to_string();
                    }
                    _ => return Err(format!("Line {}: '{}' is not a NURBS shape", line, name)),
                }
                if let Some(k) = kind {
                    scene.add_gen_plugs(id, &k);
                }
            }
            "addAttr" => {
                let p = parse_args(rest, ADD_ATTR).map_err(ctx)?;
                let node = arg(&p, 0, line, "node name")?;
                let id = scene.find(&node).ok_or_else(|| format!("Line {}: no node '{}'", line, node))?;
                let ln = p.string("ln").ok_or_else(|| format!("Line {}: addAttr needs -ln", line))?;
                let dv = p.num("dv").map_err(ctx)?.unwrap_or(0.0);
                let dtype = match p.string("at").as_deref() {
                    Some("long") => 1,
                    Some("bool") => 2,
                    _ => 0,
                };
                let keyable = p.num("k").map_err(ctx)?.map(|v| v > 0.5).unwrap_or(true);
                scene.add_attr_typed(id, &ln, dv, p.num("min").map_err(ctx)?, p.num("max").map_err(ctx)?, keyable, dtype).map_err(ctx)?;
            }
            "setAttr" => {
                let p = parse_args(rest, &[FlagSpec { names: &["lock", "l"], arity: 1 }, FlagSpec { names: &["keyable", "k"], arity: 1 }]).map_err(ctx)?;
                let path = arg(&p, 0, line, "attribute path")?;
                if p.args.len() == 1 {
                    let (id, attr) = scene.resolve_plug(&path).map_err(ctx)?;
                    if let Some(v) = p.num("lock").map_err(ctx)? {
                        scene.set_locked(id, &attr, v > 0.5).map_err(ctx)?;
                    }
                    if let Some(v) = p.num("keyable").map_err(ctx)? {
                        scene.set_keyable(id, &attr, v > 0.5).map_err(ctx)?;
                    }
                    continue;
                }
                let v = p.args.get(1).ok_or_else(|| format!("Line {}: setAttr needs a value", line))?;
                let (id, attr) = scene.resolve_plug(&path).map_err(ctx)?;
                scene.set_attr(id, &attr, v.as_f32().map_err(ctx)?).map_err(ctx)?;
            }
            "setNotes" => {
                let p = parse_args(rest, &[]).map_err(ctx)?;
                let node = arg(&p, 0, line, "node")?;
                let text = arg(&p, 1, line, "text")?;
                let id = scene.find(&node).ok_or_else(|| format!("Line {}: no node '{}'", line, node))?;
                scene.notes.insert(id, text);
            }
            "assignTexture" => {
                let p = parse_args(rest, &[]).map_err(ctx)?;
                let m = arg(&p, 0, line, "material")?;
                let path = arg(&p, 1, line, "file")?;
                let mid = scene.find(&m).ok_or_else(|| format!("Line {}: no material '{}'", line, m))?;
                // older files mapped images to materials directly: that is a texture node now
                scene.set_color_map(mid, Some(&path)).map_err(ctx)?;
            }
            "meshZones" => {
                let p = parse_args(rest, &[FlagSpec { names: &["zones"], arity: 1 }, FlagSpec { names: &["faces"], arity: 1 }]).map_err(ctx)?;
                let shape = arg(&p, 0, line, "shape")?;
                let sid = scene.find(&shape).ok_or_else(|| format!("Line {}: no shape '{}'", line, shape))?;
                let toks: Vec<String> = p.string("zones").unwrap_or_default().split_whitespace().map(|x| x.to_string()).collect();
                let mut zones = Vec::new();
                for c in toks.chunks_exact(7) {
                    let num = |i: usize| c[i].parse::<f32>().unwrap_or(0.0);
                    zones.push(crate::dag::nodes::Zone {
                        material: if c[0] == "-" { None } else { scene.find(&c[0]) },
                        uv: crate::geo::edit::UvMap { mode: num(1) as u8, tile_u: num(2), tile_v: num(3), off_u: num(4), off_v: num(5), rot_deg: num(6) },
                    });
                }
                let fz: Vec<u16> = p.string("faces").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                if let Some(NodeData::Mesh(md)) = scene.nodes.get_mut(sid).map(|n| &mut n.data) {
                    md.zones = zones;
                    md.face_zone = fz;
                }
            }
            "meshUvs" => {
                let p = parse_args(rest, &[FlagSpec { names: &["uvs"], arity: 1 }, FlagSpec { names: &["faces"], arity: 1 }]).map_err(ctx)?;
                let shape = arg(&p, 0, line, "shape")?;
                let sid = scene.find(&shape).ok_or_else(|| format!("Line {}: no shape '{}'", line, shape))?;
                let nums: Vec<f32> = p.string("uvs").unwrap_or_default().split_whitespace().filter_map(|t| t.parse().ok()).collect();
                let uvs: Vec<crate::math::Vec2> = nums.chunks_exact(2).map(|c| crate::math::Vec2::new(c[0], c[1])).collect();
                let fuv: Vec<Vec<u32>> = p.string("faces").unwrap_or_default().split(',').map(|r| r.split_whitespace().filter_map(|t| t.parse().ok()).collect()).collect();
                if fuv.iter().flatten().any(|&u| u as usize >= uvs.len()) {
                    return Err(format!("Line {}: meshUvs refers to a UV that does not exist", line));
                }
                if let Some(NodeData::Mesh(md)) = scene.nodes.get_mut(sid).map(|n| &mut n.data) {
                    let sig: Vec<u64> = Vec::new();
                    md.uv_set = Some(crate::geo::uv::UvSet { uvs, fuv, sig });
                    md.sync_uvs();
                }
            }
            "assignMaterial" => {
                let p = parse_args(rest, &[]).map_err(ctx)?;
                let m = arg(&p, 0, line, "material")?;
                let o = arg(&p, 1, line, "object")?;
                let mid = scene.find(&m).ok_or_else(|| format!("Line {}: no material '{}'", line, m))?;
                let oid = scene.find(&o).ok_or_else(|| format!("Line {}: no object '{}'", line, o))?;
                scene.assign_material(mid, oid).map_err(ctx)?;
            }
            "animLayer" => {
                let p = parse_args(rest, ANIM_LAYER).map_err(ctx)?;
                let name = p.string("n").ok_or_else(|| format!("Line {}: animLayer needs -n", line))?;
                let idx = match scene.anim.find_layer(&name) {
                    Some(i) if reference => i,
                    _ => scene.anim.add_layer(&name).map_err(ctx)?,
                };
                if let Some(w) = p.num("w").map_err(ctx)? {
                    scene.anim.set_weight(idx, w);
                }
                if let Some(m) = p.num("mute").map_err(ctx)? {
                    scene.anim.set_mute(idx, m > 0.5);
                }
                for m in p.string("members").unwrap_or_default().split_whitespace() {
                    if let Ok((id, attr)) = scene.resolve_plug(m) {
                        scene.anim.layers[idx].members.push((id, attr));
                    }
                }
                if let Some(w) = p.string("wkeys").filter(|w| !w.is_empty()) {
                    scene.anim.layers[idx].weight_curve = crate::anim::trax::curve_from_text(&w);
                }
            }
            "setKeyframe" => {
                let p = parse_args(rest, SET_KEY).map_err(ctx)?;
                let path = arg(&p, 0, line, "attribute path")?;
                let (id, attr) = scene.resolve_plug(&path).map_err(ctx)?;
                let t = p.num("t").map_err(ctx)?;
                let v = p.num("v").map_err(ctx)?;
                let ip = match p.string("ip") {
                    Some(s) => Interp::parse(&s).ok_or_else(|| format!("Line {}: bad interpolation '{}'", line, s))?,
                    None => Interp::Bezier,
                };
                let layer = match p.string("animLayer") {
                    Some(l) => scene
                        .anim
                        .find_layer(&l)
                        .ok_or_else(|| format!("Line {}: no animation layer '{}'", line, l))?,
                    None => 0,
                };
                scene.anim.set_key(id, &attr, layer, t.unwrap_or(1.0), v.unwrap_or(0.0), ip);
                let (it, ot) = (p.string("it").and_then(|x| crate::anim::keyframes::Tan::parse(&x)), p.string("ot").and_then(|x| crate::anim::keyframes::Tan::parse(&x)));
                if it.is_some() || ot.is_some() {
                    if let Some(b) = scene.anim.bindings.iter_mut().find(|b| b.node == id && b.attr == attr && b.layer == layer) {
                        b.curve.set_tangents_at(t.unwrap_or(1.0), it, ot);
                    }
                }
            }
            "setInfinity" => {
                let p = parse_args(rest, &[FlagSpec { names: &["pri"], arity: 1 }, FlagSpec { names: &["poi"], arity: 1 }, FlagSpec { names: &["animLayer"], arity: 1 }]).map_err(ctx)?;
                let path = arg(&p, 0, line, "attribute path")?;
                let (id, attr) = scene.resolve_plug(&path).map_err(ctx)?;
                let layer = p.string("animLayer").and_then(|l| scene.anim.find_layer(&l)).unwrap_or(0);
                if let Some(b) = scene.anim.bindings.iter_mut().find(|b| b.node == id && b.attr == attr && b.layer == layer) {
                    if let Some(m) = p.string("pri").and_then(|x| crate::anim::keyframes::Infinity::parse(&x)) {
                        b.curve.pre = m;
                    }
                    if let Some(m) = p.string("poi").and_then(|x| crate::anim::keyframes::Infinity::parse(&x)) {
                        b.curve.post = m;
                    }
                }
            }
            "connectAttr" => {
                let p = parse_args(rest, &[]).map_err(ctx)?;
                let a = arg(&p, 0, line, "source")?;
                let b = arg(&p, 1, line, "destination")?;
                let src = scene.resolve_plug(&a).map_err(ctx)?;
                let dst = scene.resolve_plug(&b).map_err(ctx)?;
                scene.connect(src, dst, true).map_err(ctx)?;
            }
            other => return Err(format!("Line {}: unknown command '{}' in scene file", line, other)),
        }
    }
    if !reference {
        if let (Some(a), Some(b)) = (pb_min, pb_max) {
            scene.timeline.set_range(a, b);
        }
        if let Some(f) = pb_fps {
            scene.timeline.fps = f.max(1.0);
        }
        if let Some(c) = pb_cur {
            scene.timeline.set_time(c);
        }
    }
    // resolve joint references now that every node exists
    let all: Vec<NodeId> = scene.nodes.iter().map(|(i, _)| i).collect();
    for id in all {
        let mut resolved: Option<Vec<NodeId>> = None;
        let mut ik: Option<(Option<NodeId>, Option<NodeId>)> = None;
        let mut cons: Option<Vec<NodeId>> = None;
        let mut set_members: Option<Vec<NodeId>> = None;
        let mut mp_curve: Option<NodeId> = None;
        let mut ik_extra: Option<(Option<NodeId>, Option<NodeId>)> = None;
        match scene.nodes.get(id).map(|n| &n.data) {
            Some(NodeData::PolyOp(op)) => {
                if let Some(sk) = &op.skin {
                    if sk.joints.is_empty() && !sk.joint_names.is_empty() {
                        resolved = Some(sk.joint_names.iter().filter_map(|n| scene.find(n)).collect());
                    }
                }
            }
            Some(NodeData::IkHandle(d)) if d.start.is_none() && !d.start_name.is_empty() => {
                ik = Some((scene.find(&d.start_name), scene.find(&d.end_name)));
                ik_extra = Some((if d.curve_name.is_empty() { None } else { scene.find(&d.curve_name) }, if d.pole_name.is_empty() { None } else { scene.find(&d.pole_name) }));
            }
            Some(NodeData::ObjectSet(sd)) if sd.members.is_empty() && !sd.member_names.is_empty() => {
                set_members = Some(sd.member_names.iter().filter_map(|n| scene.find(n)).collect());
            }
            Some(NodeData::MotionPath(mp)) if mp.curve.is_none() && !mp.curve_name.is_empty() => {
                let name = mp.curve_name.clone();
                mp_curve = scene.find(&name);
            }
            Some(NodeData::Constraint(c)) if c.targets.is_empty() => {
                cons = Some(c.target_names.iter().filter_map(|n| scene.find(n)).collect());
            }
            _ => {}
        }
        if let Some(js) = resolved {
            if let Some(NodeData::PolyOp(op)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                if let Some(sk) = &mut op.skin {
                    sk.joints = js;
                }
            }
            scene.skins.push(id);
        }
        // dynamics: names to nodes
        resolve_dyn(scene, id);
        // handles of curve deformers
        if let Some(NodeData::NurbsCurve(cd)) = scene.nodes.get(id).map(|n| &n.data) {
            if cd.drivers.iter().any(|d| d.handle.is_none() && !d.handle_name.is_empty()) {
                let found: Vec<Option<NodeId>> = cd.drivers.iter().map(|d| scene.find(&d.handle_name)).collect();
                if let Some(NodeData::NurbsCurve(cd)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                    for (d, h) in cd.drivers.iter_mut().zip(found) {
                        d.handle = h;
                    }
                }
            }
        }
        // character channels and distance dimension locators
        if let Some(NodeData::Character(c)) = scene.nodes.get(id).map(|n| &n.data) {
            if c.channels.is_empty() && !c.channel_names.is_empty() {
                let chs: Vec<(NodeId, String)> = c.channel_names.iter().filter_map(|(n, a)| scene.find(n).map(|i| (i, a.clone()))).collect();
                if let Some(NodeData::Character(c)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                    c.channels = chs;
                }
            }
        }
        if let Some(NodeData::Distance(d)) = scene.nodes.get(id).map(|n| &n.data) {
            if d.start.is_none() && !d.start_name.is_empty() {
                let (a, b) = (scene.find(&d.start_name), scene.find(&d.end_name));
                if let Some(NodeData::Distance(d)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                    d.start = a;
                    d.end = b;
                }
            }
        }
        // lattice joints and the lattice a deformation node points at
        if let Some(NodeData::Lattice(ld)) = scene.nodes.get(id).map(|n| &n.data) {
            if let Some(sk) = &ld.skin {
                if sk.joints.is_empty() && !sk.joint_names.is_empty() {
                    let js: Vec<NodeId> = sk.joint_names.iter().filter_map(|n| scene.find(n)).collect();
                    if let Some(NodeData::Lattice(ld)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                        if let Some(sk) = ld.skin.as_mut() {
                            sk.joints = js;
                        }
                    }
                }
            }
        }
        // wire curves and cluster / soft modification handles
        if let Some(NodeData::PolyOp(op)) = scene.nodes.get(id).map(|n| &n.data) {
            if op.kind == PolyKind::Jiggle && !scene.skins.contains(&id) {
                scene.skins.push(id);
            }
        }
        if let Some(NodeData::PolyOp(op)) = scene.nodes.get(id).map(|n| &n.data) {
            if let Some(e) = &op.ext {
                if (e.handle.is_none() && !e.handle_name.is_empty()) || (e.curve.is_none() && !e.curve_name.is_empty()) {
                    let (h, c) = (scene.find(&e.handle_name), scene.find(&e.curve_name));
                    if let Some(NodeData::PolyOp(op)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                        if let Some(e) = op.ext.as_mut() {
                            e.handle = h;
                            e.curve = c;
                        }
                    }
                    scene.skins.push(id);
                } else if matches!(op.kind, PolyKind::Jiggle) && !scene.skins.contains(&id) {
                    scene.skins.push(id);
                }
            }
        }
        if let Some(NodeData::PolyOp(op)) = scene.nodes.get(id).map(|n| &n.data) {
            if op.kind == PolyKind::Lattice && op.lattice.is_none() && !op.lattice_name.is_empty() {
                let l = scene.find(&op.lattice_name);
                if let Some(NodeData::PolyOp(op)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                    op.lattice = l;
                }
                scene.skins.push(id);
            }
        }
        if let Some(c) = mp_curve {
            if let Some(NodeData::MotionPath(mp)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                mp.curve = Some(c);
            }
        }
        if let Some(m) = set_members {
            if let Some(NodeData::ObjectSet(sd)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                sd.members = m;
            }
        }
        if let Some(t) = cons {
            if let Some(NodeData::Constraint(c)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                c.targets = t;
            }
        }
        if let Some((a, b)) = ik {
            if let Some(NodeData::IkHandle(d)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                d.start = a;
                d.end = b;
                if let Some((c, pl)) = ik_extra {
                    d.curve = c;
                    d.pole = pl;
                }
            }
        }
    }
    // texture nodes report the average color of their image
    for tn in scene.ids_where(|d| matches!(d, NodeData::Texture(_))) {
        scene.refresh_texture_outputs(tn);
    }
    // instances follow the shapes they instance
    for (inst, src) in instances {
        if let (Some(i), Some(s)) = (scene.find(&inst), scene.find(&src)) {
            if let Some(NodeData::Mesh(md)) = scene.nodes.get_mut(i).map(|n| &mut n.data) {
                md.instance_of = Some(s);
                md.instance_ver = 0;
            }
        }
    }
    // NURBS generators: input names become nodes
    for id in scene.ids_where(|d| matches!(d, NodeData::NurbsCurve(_) | NodeData::Mesh(_))) {
        let names: Vec<String> = match scene.nodes.get(id).map(|n| &n.data) {
            Some(NodeData::NurbsCurve(cd)) => cd.gen.as_ref().map(|g| g.input_names.clone()).unwrap_or_default(),
            Some(NodeData::Mesh(md)) => md.nurbs.as_ref().and_then(|n| n.gen.as_ref()).map(|g| g.input_names.clone()).unwrap_or_default(),
            _ => Vec::new(),
        };
        if names.is_empty() {
            continue;
        }
        let ids: Vec<NodeId> = names.iter().filter_map(|n| scene.find(n)).filter_map(|t| scene.curve_shapes_under(t).first().copied()).collect();
        match scene.nodes.get_mut(id).map(|n| &mut n.data) {
            Some(NodeData::NurbsCurve(cd)) => {
                if let Some(g) = cd.gen.as_mut() {
                    g.inputs = ids;
                }
            }
            Some(NodeData::Mesh(md)) => {
                if let Some(g) = md.nurbs.as_mut().and_then(|n| n.gen.as_mut()) {
                    g.inputs = ids;
                }
            }
            _ => {}
        }
    }
    // curves on surfaces point at their surface by name
    for id in scene.ids_where(|d| matches!(d, NodeData::NurbsCurve(cd) if cd.on_surface.is_some())) {
        let name = match scene.nodes.get(id).map(|n| &n.data) {
            Some(NodeData::NurbsCurve(cd)) => cd.on_surface.as_ref().map(|o| o.name.clone()).unwrap_or_default(),
            _ => continue,
        };
        let target = scene.find(&name);
        if let Some(NodeData::NurbsCurve(cd)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
            match (cd.on_surface.as_mut(), target) {
                (Some(o), Some(t)) => o.surface = t,
                _ => cd.on_surface = None,
            }
        }
    }
    scene.rebuild_all_meshes();
    scene.sync_instances();
    // face signatures for the texture zones (matching face counts just record them)
    let meshes = scene.ids_where(|d| matches!(d, NodeData::Mesh(_)));
    for m in meshes {
        if let Some(NodeData::Mesh(md)) = scene.nodes.get_mut(m).map(|n| &mut n.data) {
            md.sync_zones();
        }
    }
    scene.evaluate_frame();
    Ok(())
}



// ---------------------------------------------------------------------------------------------- dynamics nodes

fn nm(scene: &Scene, id: Option<NodeId>) -> String {
    match id.filter(|i| scene.node(*i).is_some()) {
        Some(i) => scene.name_of(i),
        None => "-".to_string(),
    }
}

fn fl(v: &[f32]) -> String {
    v.iter().map(|x| n(*x)).collect::<Vec<_>>().join(" ")
}

fn ramp_text(r: &Option<crate::dag::nodes::Ramp>) -> String {
    match r {
        Some(v) => v.iter().map(|(p, c)| format!("{} {} {} {}", n(*p), n(c.x), n(c.y), n(c.z))).collect::<Vec<_>>().join(";"),
        None => String::new(),
    }
}

fn parse_ramp(t: &str) -> Option<crate::dag::nodes::Ramp> {
    if t.trim().is_empty() {
        return None;
    }
    Some(
        t.split(';')
            .filter_map(|s| {
                let v: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                (v.len() == 4).then(|| (v[0], crate::math::Vec3::new(v[1], v[2], v[3])))
            })
            .collect(),
    )
}

/// Compact text of the data of a dynamics node (settings are plugs); names are resolved after loading.
fn dyn_payload(scene: &Scene, data: &NodeData) -> Option<String> {
    use crate::dag::nodes::*;
    Some(match data {
        NodeData::RigidBody(rb) => match &rb.initial {
            Some(s) => fl(&[s.pos.x, s.pos.y, s.pos.z, s.rot.x, s.rot.y, s.rot.z, s.rot.w, s.vel.x, s.vel.y, s.vel.z, s.spin.x, s.spin.y, s.spin.z]),
            None => String::new(),
        },
        NodeData::Field(f) => format!("{}|{}", f.kind, f.targets.iter().filter(|t| scene.node(**t).is_some()).map(|t| scene.name_of(*t)).collect::<Vec<_>>().join(" ")),
        NodeData::RigidJoint(j) => format!("{}|{}|{}", j.kind, nm(scene, j.a), nm(scene, j.b)),
        NodeData::PEmitter(e) => format!("{}|{}", nm(scene, e.target), nm(scene, e.source)),
        NodeData::Particle(p) => {
            let colliders = p.colliders.iter().filter(|c| c.node.map(|n| scene.node(n).is_some()).unwrap_or(false)).map(|c| format!("{} {} {}", nm(scene, c.node), n(c.resilience), n(c.friction))).collect::<Vec<_>>().join(";");
            let events = p.events.iter().map(|e| format!("{} {} {} {} {} {} {}", e.kind, e.num, e.random as u8, n(e.spread), nm(scene, e.target), n(e.inherit), e.original_dies as u8)).collect::<Vec<_>>().join(";");
            let initial = match &p.initial {
                Some(i) => {
                    let items: Vec<String> = i.items.iter().map(|q| fl(&[q.id as f32, q.pos.x, q.pos.y, q.pos.z, q.vel.x, q.vel.y, q.vel.z, q.age, q.life, q.radius])).collect();
                    format!("{} {} {}", i.next_id, i.rng.0, items.join(";"))
                }
                None => String::new(),
            };
            format!("{}|{}|{}|{}|{}|{}|{}|{}", nm(scene, p.material), colliders, events, ramp_text(&p.scale_ramp), ramp_text(&p.color_ramp), ramp_text(&p.opacity_ramp), n(p.scale_max), initial)
        }
        _ => return None,
    })
}

fn parse_dyn(ty: &str, text: &str) -> NodeData {
    use crate::dag::nodes::*;
    let parts: Vec<&str> = text.split('|').collect();
    let name = |s: &str| if s == "-" || s.is_empty() { String::new() } else { s.to_string() };
    match ty {
        "rigidBody" => {
            let v: Vec<f32> = text.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            let initial = (v.len() == 13).then(|| BodyState { pos: crate::math::Vec3::new(v[0], v[1], v[2]), rot: crate::math::Quat::new(v[3], v[4], v[5], v[6]), vel: crate::math::Vec3::new(v[7], v[8], v[9]), spin: crate::math::Vec3::new(v[10], v[11], v[12]) });
            NodeData::RigidBody(Box::new(RigidData { initial }))
        }
        "rigidSolver" => NodeData::DynSolver(Box::default()),
        "field" => NodeData::Field(Box::new(FieldData { kind: parts.first().and_then(|k| k.parse().ok()).unwrap_or(0), targets: Vec::new(), target_names: parts.get(1).map(|t| t.split_whitespace().map(|x| x.to_string()).collect()).unwrap_or_default() })),
        "rigidConstraint" => NodeData::RigidJoint(Box::new(RigidJointData { kind: parts.first().and_then(|k| k.parse().ok()).unwrap_or(0), a: None, b: None, names: vec![name(parts.get(1).unwrap_or(&"")), name(parts.get(2).unwrap_or(&""))] })),
        "particleEmitter" => NodeData::PEmitter(Box::new(PEmitterData { target: None, source: None, names: vec![name(parts.first().unwrap_or(&"")), name(parts.get(1).unwrap_or(&""))], ..Default::default() })),
        _ => {
            let mut pd = ParticleData::default();
            pd.material_name = name(parts.first().unwrap_or(&""));
            for c in parts.get(1).unwrap_or(&"").split(';').filter(|c| !c.is_empty()) {
                let v: Vec<&str> = c.split_whitespace().collect();
                if v.len() == 3 {
                    pd.colliders.push(ColliderRef { node: None, name: v[0].to_string(), resilience: v[1].parse().unwrap_or(0.5), friction: v[2].parse().unwrap_or(0.1) });
                }
            }
            for e in parts.get(2).unwrap_or(&"").split(';').filter(|e| !e.is_empty()) {
                let v: Vec<&str> = e.split_whitespace().collect();
                if v.len() == 7 {
                    pd.events.push(ParticleEvent { kind: v[0].parse().unwrap_or(0), num: v[1].parse().unwrap_or(1), random: v[2] == "1", spread: v[3].parse().unwrap_or(1.0), target: None, target_name: name(v[4]), inherit: v[5].parse().unwrap_or(1.0), original_dies: v[6] == "1" });
                }
            }
            pd.scale_ramp = parse_ramp(parts.get(3).unwrap_or(&""));
            pd.color_ramp = parse_ramp(parts.get(4).unwrap_or(&""));
            pd.opacity_ramp = parse_ramp(parts.get(5).unwrap_or(&""));
            pd.scale_max = parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(1.0);
            let init = parts.get(7).unwrap_or(&"");
            if !init.trim().is_empty() {
                let mut it = init.splitn(3, ' ');
                let (next, rng, rest) = (it.next().and_then(|x| x.parse().ok()).unwrap_or(0u32), it.next().and_then(|x| x.parse().ok()).unwrap_or(1u64), it.next().unwrap_or(""));
                let mut sim = crate::nucleus::fields::PSim { items: Vec::new(), next_id: next, rng: crate::nucleus::particles::Rng(rng) };
                for item in rest.split(';').filter(|i| !i.trim().is_empty()) {
                    let v: Vec<f32> = item.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    if v.len() == 10 {
                        sim.items.push(crate::nucleus::fields::P { id: v[0] as u32, pos: crate::math::Vec3::new(v[1], v[2], v[3]), vel: crate::math::Vec3::new(v[4], v[5], v[6]), age: v[7], life: v[8], radius: v[9], evented: false });
                    }
                }
                pd.initial = Some(sim);
            }
            NodeData::Particle(Box::new(pd))
        }
    }
}

/// Turn the names read from a file into nodes.
fn resolve_dyn(scene: &mut Scene, id: NodeId) {
    use crate::dag::nodes::*;
    let Some(node) = scene.nodes.get(id) else { return };
    match node.data.clone() {
        NodeData::Field(f) if f.targets.is_empty() && !f.target_names.is_empty() => {
            let t: Vec<NodeId> = f.target_names.iter().filter_map(|n| scene.find(n)).collect();
            if let Some(NodeData::Field(fd)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                fd.targets = t;
                fd.target_names.clear();
            }
        }
        NodeData::RigidJoint(j) if !j.names.is_empty() => {
            let (a, b) = (scene.find(&j.names[0]), j.names.get(1).and_then(|n| scene.find(n)));
            if let Some(NodeData::RigidJoint(jd)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                jd.a = a;
                jd.b = b;
                jd.names.clear();
            }
        }
        NodeData::PEmitter(e) if !e.names.is_empty() => {
            let (t, s) = (scene.find(&e.names[0]), e.names.get(1).and_then(|n| scene.find(n)));
            if let Some(NodeData::PEmitter(ed)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                ed.target = t;
                ed.source = s;
                ed.names.clear();
            }
        }
        NodeData::Particle(p) => {
            let needs = !p.material_name.is_empty() || p.colliders.iter().any(|c| c.node.is_none()) || p.events.iter().any(|e| e.target.is_none() && !e.target_name.is_empty());
            if !needs {
                return;
            }
            let mat = scene.find(&p.material_name);
            let cols: Vec<Option<NodeId>> = p.colliders.iter().map(|c| scene.find(&c.name)).collect();
            let evs: Vec<Option<NodeId>> = p.events.iter().map(|e| scene.find(&e.target_name)).collect();
            if let Some(NodeData::Particle(pd)) = scene.nodes.get_mut(id).map(|n| &mut n.data) {
                if mat.is_some() {
                    pd.material = mat;
                }
                pd.material_name.clear();
                for (c, n) in pd.colliders.iter_mut().zip(cols) {
                    if c.node.is_none() {
                        c.node = n;
                    }
                }
                for (e, n) in pd.events.iter_mut().zip(evs) {
                    if e.target.is_none() {
                        e.target = n;
                    }
                }
            }
        }
        _ => {}
    }
}
