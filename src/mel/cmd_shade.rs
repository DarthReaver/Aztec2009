//! Shading networks beyond the Hypershade basics: duplicate, export / import, convert to a file texture,
//! interactive texture placement.

use super::cmd_model::{current_shape, effective};
use super::commands::{resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::NodeData;
use crate::dag::{NodeId, Scene};
use crate::session::{SelMode, Session};
use std::collections::HashMap;

/// A material (or texture) with every texture node feeding it, upstream first-found order.
pub fn network_of(scene: &Scene, root: NodeId) -> Vec<NodeId> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let id = out[i];
        i += 1;
        if let Some(n) = scene.node(id) {
            for p in &n.plugs {
                if let Some(src) = &p.source {
                    if matches!(scene.node(src.node).map(|x| &x.data), Some(NodeData::Texture(_))) && !out.contains(&src.node) {
                        out.push(src.node);
                    }
                }
            }
        }
    }
    out
}

/// Copy the networks of `roots` from one scene into another (names are made unique); returns old -> new ids.
pub fn copy_network(src: &Scene, roots: &[NodeId], dst: &mut Scene) -> Result<HashMap<NodeId, NodeId>, String> {
    let mut ids: Vec<NodeId> = Vec::new();
    for r in roots {
        for n in network_of(src, *r) {
            if !ids.contains(&n) {
                ids.push(n);
            }
        }
    }
    let mut map: HashMap<NodeId, NodeId> = HashMap::new();
    for id in &ids {
        let node = src.node(*id).ok_or("Invalid node")?;
        let stem = node.name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
        let stem = if stem.is_empty() { node.name.clone() } else { stem };
        let nid = dst.create_node(node.data.clone(), &stem, None, true);
        for p in &node.plugs {
            if p.readonly || p.custom {
                continue;
            }
            if let Some(q) = dst.nodes.get_mut(nid).and_then(|n| n.plug_mut(&p.name)) {
                q.value = p.value;
                q.rest = p.rest;
                q.keyable = p.keyable;
                q.locked = false;
            }
        }
        map.insert(*id, nid);
    }
    for (a, b) in src.connections() {
        if let (Some(na), Some(nb)) = (map.get(&a.node), map.get(&b.node)) {
            dst.connect((*na, a.attr.clone()), (*nb, b.attr.clone()), true)?;
        }
    }
    for id in map.values() {
        if matches!(dst.node(*id).map(|n| &n.data), Some(NodeData::Texture(_))) {
            dst.refresh_texture_outputs(*id);
        }
    }
    Ok(map)
}

fn materials_of(s: &Session, names: &[String]) -> Result<Vec<NodeId>, String> {
    let ids = resolve_targets(s, names)?;
    let mut out = Vec::new();
    for id in ids {
        if matches!(s.scene.node(id).map(|n| &n.data), Some(NodeData::Material)) {
            out.push(id);
            continue;
        }
        // an object: its material(s)
        for sh in s.scene.mesh_shapes_under(id) {
            if let Some(NodeData::Mesh(md)) = s.scene.node(sh).map(|n| &n.data) {
                if let Some(m) = md.material {
                    out.push(m);
                }
            }
        }
    }
    out.dedup();
    if out.is_empty() {
        return Err("Select a material (or an object with one)".to_string());
    }
    Ok(out)
}

/// `duplicateShadingNetwork material|object`: Edit > Duplicate > Shading Network (the copy is not assigned).
pub fn duplicate_network(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let mats = materials_of(s, &p.arg_strings())?;
    let mut out = Vec::new();
    for m in mats {
        let ids = network_of(&s.scene, m);
        let snapshot: Vec<(NodeId, NodeData, Vec<(String, f32, f32, bool)>, String)> = ids
            .iter()
            .filter_map(|i| s.scene.node(*i).map(|n| (*i, n.data.clone(), n.plugs.iter().map(|p| (p.name.clone(), p.value, p.rest, p.readonly || p.custom)).collect(), n.name.clone())))
            .collect();
        let conns = s.scene.connections();
        let mut map: HashMap<NodeId, NodeId> = HashMap::new();
        for (id, data, plugs, name) in &snapshot {
            let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
            let stem = if stem.is_empty() { name.as_str() } else { stem };
            let nid = s.scene.create_node(data.clone(), stem, None, true);
            for (n, v, r, skip) in plugs {
                if *skip {
                    continue;
                }
                if let Some(q) = s.scene.nodes.get_mut(nid).and_then(|x| x.plug_mut(n)) {
                    q.value = *v;
                    q.rest = *r;
                }
            }
            map.insert(*id, nid);
        }
        for (a, b) in conns {
            if let (Some(na), Some(nb)) = (map.get(&a.node), map.get(&b.node)) {
                let _ = s.scene.connect((*na, a.attr.clone()), (*nb, b.attr.clone()), true);
            }
        }
        if let Some(n) = map.get(&m) {
            out.push(s.scene.name_of(*n));
        }
    }
    Ok(out)
}

/// `exportShaders -file PATH material ...`: File > Export Selected Network (a scene file with just the shaders;
/// a bare file name goes to renderData/shaders in the project).
pub fn export_shaders(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["file", "f"], arity: 1 }])?;
    let file = p.string("file").ok_or("Usage: exportShaders -file NAME.aztec material ...")?;
    let mats = materials_of(s, &p.arg_strings())?;
    let mut tmp = Scene::new();
    copy_network(&s.scene, &mats, &mut tmp)?;
    let mut path = std::path::PathBuf::from(&file);
    if path.parent().map(|d| d.as_os_str().is_empty()).unwrap_or(true) {
        if let Some(d) = &s.scene.project_dir {
            path = d.join("renderData").join("shaders").join(&file);
        }
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("Cannot create {}: {}", d.display(), e))?;
    }
    std::fs::write(&path, crate::dag::serialize::to_ascii(&tmp)).map_err(|e| format!("Cannot write {}: {}", path.display(), e))?;
    Ok(vec![format!("Exported {} shader(s) to {}", mats.len(), path.display())])
}

/// `importShaders PATH`: File > Import of a shader file; returns the new materials.
pub fn import_shaders(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let file = p.arg_strings().first().cloned().ok_or("Usage: importShaders NAME.aztec")?;
    let mut path = std::path::PathBuf::from(&file);
    if !path.exists() {
        if let Some(d) = &s.scene.project_dir {
            path = d.join("renderData").join("shaders").join(&file);
        }
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    let src = crate::dag::serialize::from_ascii(&text)?;
    let roots: Vec<NodeId> = src.ids_where(|d| matches!(d, NodeData::Material)).into_iter().filter(|m| src.name_of(*m) != "lambert1").collect();
    if roots.is_empty() {
        return Err("The file has no shaders".to_string());
    }
    let map = copy_network(&src, &roots, &mut s.scene)?;
    Ok(roots.iter().filter_map(|r| map.get(r)).map(|n| s.scene.name_of(*n)).collect())
}

/// `convertToFileTexture [-res N] [-name FILE] material|object`: Hypershade Edit > Convert to File Texture. The
/// color network is baked over the 0..1 UV range into sourceimages/ and replaced by one file texture.
pub fn convert_to_file_texture(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["res", "r"], arity: 1 }, FlagSpec { names: &["name", "n"], arity: 1 }])?;
    let res = p.num("res")?.unwrap_or(512.0) as usize;
    if ![64, 128, 256, 512, 1024, 2048].contains(&res) {
        return Err("-res must be 64, 128, 256, 512, 1024 or 2048".to_string());
    }
    let mats = materials_of(s, &p.arg_strings())?;
    let mut out = Vec::new();
    for m in mats {
        let root = s.scene.texture_source(m, "color").ok_or_else(|| format!("{} has no texture on its color", s.scene.name_of(m)))?;
        let (net, _) = crate::render::netbake::compile(&mut s.scene, root).ok_or("Cannot read the texture network")?;
        let tex = crate::render::netbake::bake(&net, res);
        let mut img = image::RgbImage::new(res as u32, res as u32);
        for (i, px) in img.pixels_mut().enumerate() {
            let c = tex.rgb[i];
            *px = image::Rgb([(c[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8, (c[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8, (c[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8]);
        }
        let dir = s.scene.project_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(".")).join("sourceimages");
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {}", dir.display(), e))?;
        let fname = p.string("name").unwrap_or_else(|| format!("{}_baked.png", s.scene.name_of(m)));
        let path = dir.join(&fname);
        img.save(&path).map_err(|e| format!("Cannot save {}: {}", path.display(), e))?;
        let path_s = path.to_string_lossy().to_string();
        crate::render::texture::invalidate(&path_s);
        let stored = s.scene.texture_path_for_storage(&path_s);
        let t = s.scene.create_texture("file", &stored, None);
        for a in ["R", "G", "B"] {
            s.scene.connect((t, format!("outColor{}", a)), (m, format!("color{}", a)), true)?;
        }
        s.view.show_textures();
        out.push(format!("{}: {}", s.scene.name_of(m), stored));
    }
    Ok(out)
}

/// `placeTexture [-at U V] [-t U V] [-s SU SV] [-r DEG] [-wrap 0|1] [object]`: Interactive Placement of the object's
/// color texture. `-at` puts the middle of the texture at that point of the surface.
pub fn place_texture(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(
        rest,
        &[
            FlagSpec { names: &["at"], arity: 2 },
            FlagSpec { names: &["translate", "t"], arity: 2 },
            FlagSpec { names: &["scale", "s"], arity: 2 },
            FlagSpec { names: &["rotate", "r"], arity: 1 },
            FlagSpec { names: &["wrap"], arity: 1 },
        ],
    )?;
    let mats = materials_of(s, &p.arg_strings())?;
    let mut n = 0;
    for m in mats {
        let Some(t) = s.scene.texture_source(m, "color") else { continue };
        if let Some(v) = p.get("scale") {
            s.scene.set_attr(t, "repeatU", v[0].as_f32()?)?;
            s.scene.set_attr(t, "repeatV", v[1].as_f32()?)?;
        }
        if let Some(v) = p.num("rotate")? {
            s.scene.set_attr(t, "rotateUV", v)?;
        }
        if let Some(v) = p.get("translate") {
            s.scene.set_attr(t, "offsetU", v[0].as_f32()?)?;
            s.scene.set_attr(t, "offsetV", v[1].as_f32()?)?;
        }
        if let Some(v) = p.get("at") {
            let (ru, rv) = (s.scene.get_attr(t, "repeatU")?, s.scene.get_attr(t, "repeatV")?);
            s.scene.set_attr(t, "offsetU", 0.5 - v[0].as_f32()? * ru)?;
            s.scene.set_attr(t, "offsetV", 0.5 - v[1].as_f32()? * rv)?;
        }
        if let Some(w) = p.num("wrap")? {
            s.scene.set_attr(t, "wrapU", if w > 0.5 { 1.0 } else { 0.0 })?;
            s.scene.set_attr(t, "wrapV", if w > 0.5 { 1.0 } else { 0.0 })?;
        }
        n += 1;
    }
    if n == 0 {
        return Err("The object's material has no texture to place".to_string());
    }
    s.view.show_textures();
    Ok(vec![format!("Placed {} texture(s)", n)])
}

/// `polySelectConstraint -smoothness hard|soft [-border 1]`: Select > Select Using Constraints on edges.
pub fn select_constraint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["smoothness", "sm"], arity: 1 }, FlagSpec { names: &["border", "bo"], arity: 1 }])?;
    let shape = current_shape(s).ok_or("Select a polygon object first")?;
    s.scene.flush_meshes();
    let angle = s.scene.get_attr(shape, "smoothAngle").unwrap_or(40.0);
    let (poly, _) = s.scene.shape_poly(shape).ok_or("Not a mesh")?;
    let ef = poly.edge_faces();
    let want = p.string("smoothness").map(|v| v.to_lowercase());
    let border = p.num("border")?.map(|v| v > 0.5);
    let cos_t = angle.to_radians().cos();
    let mut edges = std::collections::BTreeSet::new();
    for (e, faces) in &ef {
        let is_border = faces.len() == 1;
        if let Some(b) = border {
            if is_border != b {
                continue;
            }
        }
        if let Some(w) = &want {
            let hard = if faces.len() == 2 { poly.face_normal(faces[0]).normalized().dot(poly.face_normal(faces[1]).normalized()) < cos_t } else { true };
            let ok = match w.as_str() {
                "hard" => hard,
                "soft" => !hard,
                "all" => true,
                o => return Err(format!("Unknown smoothness '{}': hard, soft or all", o)),
            };
            if !ok {
                continue;
            }
        }
        edges.insert(*e);
    }
    let _ = effective;
    if edges.is_empty() {
        return Err("No edges match the constraint".to_string());
    }
    let n = edges.len();
    s.comp.mode = SelMode::Edge;
    s.comp.node = Some(shape);
    s.comp.sel = crate::geo::edit::Sel { edges, ..Default::default() };
    Ok(vec![format!("Selected {} edges", n)])
}
