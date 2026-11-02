//! Texturing objects and faces: per-face materials and color maps, UV projections, texture zones.

use super::cmd_model::{current_shape, effective, faces_of, select_components};
use super::commands::{fmt_f, resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::NodeData;
use crate::dag::NodeId;
use crate::geo::edit::UvMap;
use crate::session::{SelMode, Session};

/// Faces to work on.
pub struct FaceTarget {
    pub shape: NodeId,
    pub faces: Vec<u32>,
}

/// Faces named by component arguments (`obj.f[0:5]`, or edges / vertices converted to the faces they
/// enclose), else the current component selection; `None` when neither names any (work on whole objects).
pub fn face_target(s: &mut Session, args: &[String]) -> Result<Option<FaceTarget>, String> {
    let comps: Vec<String> = args.iter().filter(|a| a.contains('[')).cloned().collect();
    let explicit = !comps.is_empty();
    if explicit {
        select_components(s, &comps, false)?;
    } else if s.comp.mode == SelMode::Object {
        return Ok(None);
    }
    let Some(shape) = current_shape(s) else { return Ok(None) };
    if s.comp.node != Some(shape) || s.comp.mode == SelMode::Object {
        return Ok(None);
    }
    s.scene.flush_meshes();
    let poly = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md.poly(),
        _ => return Ok(None),
    };
    let (sel, mode) = effective(s, shape, &poly);
    let faces: Vec<u32> = faces_of(&poly, &sel, mode).into_iter().collect();
    if faces.is_empty() {
        return if explicit { Err("No faces in that selection".to_string()) } else { Ok(None) };
    }
    Ok(Some(FaceTarget { shape, faces }))
}

pub(crate) fn shapes_of_selection(s: &mut Session, names: &[String]) -> Result<Vec<NodeId>, String> {
    let ids = resolve_targets(s, names)?;
    let mut shapes: Vec<NodeId> = Vec::new();
    for id in ids {
        let t = match s.scene.node(id) {
            Some(n) if !n.data.is_transform() => n.parent.unwrap_or(id),
            _ => id,
        };
        shapes.extend(s.scene.mesh_shapes_under(t));
    }
    shapes.dedup();
    if shapes.is_empty() {
        return Err("Select a polygon object (or faces) first".to_string());
    }
    Ok(shapes)
}

fn non_component(args: &[String]) -> Vec<String> {
    args.iter().filter(|a| !a.contains('[')).cloned().collect()
}

/// Assign `mat` to faces: `assignMaterial MATERIAL obj.f[0:5]` or, with a face selection, `assignMaterial MATERIAL`.
pub fn assign_material_to_faces(s: &mut Session, mat: NodeId, t: &FaceTarget) -> Result<String, String> {
    let n = s.scene.assign_faces(t.shape, &t.faces, Some(mat), None)?;
    s.view.show_textures();
    Ok(format!("Assigned {} to {} face(s)", s.scene.name_of(mat), n.max(t.faces.len().min(n.max(1)))))
}

const TEX_SPEC: &[FlagSpec] = &[FlagSpec { names: &["clear", "c"], arity: 0 }];

/// `assignTexture "obj.f[0:5]" "file.png"`: a new material with the image on those faces, mapped to fit them.
pub fn assign_texture_to_faces(s: &mut Session, t: &FaceTarget, path: &str) -> Result<String, String> {
    crate::render::texture::invalidate(path);
    if crate::render::texture::load(&s.scene.texture_file(path)).is_none() {
        return Err(format!("Cannot read image: {}", path));
    }
    let owner = s.scene.node(t.shape).and_then(|n| n.parent).map(|p| s.scene.name_of(p)).unwrap_or_else(|| "object".to_string());
    let mat = s.scene.create_material(Some(&format!("{}_map", owner)));
    for a in ["colorR", "colorG", "colorB"] {
        s.scene.set_attr(mat, a, 1.0)?;
    }
    let stored = s.scene.texture_path_for_storage(path);
    s.scene.set_color_map(mat, Some(&stored))?;
    let map = UvMap { mode: 6, ..UvMap::default() };
    s.scene.assign_faces(t.shape, &t.faces, Some(mat), Some(map))?;
    s.view.show_textures();
    Ok(format!("{}: {} on {} face(s)", s.scene.name_of(mat), path, t.faces.len()))
}

/// Entry used by `assignTexture` when its first argument names faces.
pub fn assign_texture_cmd(s: &mut Session, rest: &[Token]) -> Option<Out> {
    let p = parse_args(rest, TEX_SPEC).ok()?;
    let args = p.arg_strings();
    let faces_given = args.first().map(|a| a.contains('[')).unwrap_or(false);
    let face_sel = !faces_given && s.comp.mode == SelMode::Face && args.len() == 1 && !p.has("clear") && current_shape(s).is_some();
    if !faces_given && !face_sel {
        return None;
    }
    Some((|| {
        let (path, comps) = if faces_given {
            (args.get(1).cloned().ok_or("Usage: assignTexture \"obj.f[0:5]\" \"file.png\"")?, args[..1].to_vec())
        } else {
            (args[0].clone(), Vec::new())
        };
        let t = face_target(s, &comps)?.ok_or("Select faces first")?;
        Ok(vec![assign_texture_to_faces(s, &t, &path)?])
    })())
}

/// Entry used by `assignMaterial` when faces are named or selected.
pub fn assign_material_cmd(s: &mut Session, names: &[String]) -> Option<Out> {
    let comps: Vec<String> = names.iter().skip(1).filter(|a| a.contains('[')).cloned().collect();
    let face_sel = comps.is_empty() && names.len() == 1 && s.comp.mode == SelMode::Face && current_shape(s).is_some();
    if comps.is_empty() && !face_sel {
        return None;
    }
    Some((|| {
        let mat = s.scene.find(&names[0]).ok_or_else(|| format!("No object matches name: {}", names[0]))?;
        if !matches!(s.scene.node(mat).map(|n| &n.data), Some(NodeData::Material)) {
            return Err(format!("'{}' is not a material", names[0]));
        }
        let t = face_target(s, &comps)?.ok_or("Select faces first")?;
        Ok(vec![assign_material_to_faces(s, mat, &t)?])
    })())
}

// ====================================================================== projections

const PROJ_SPEC: &[FlagSpec] = &[FlagSpec { names: &["mapDirection", "md"], arity: 1 }];

/// Set the projection of faces (a texture zone each) or, with no faces, of whole objects.
pub fn apply_projection(s: &mut Session, args: &[String], map: UvMap) -> Result<String, String> {
    if let Some(t) = face_target(s, args)? {
        s.scene.assign_faces(t.shape, &t.faces, None, Some(map))?;
        s.view.show_textures();
        return Ok(format!("Mapped {} face(s)", t.faces.len()));
    }
    let shapes = shapes_of_selection(s, &non_component(args))?;
    for sh in &shapes {
        s.scene.set_shape_uv_map(*sh, &map)?;
        let n = match s.scene.node(*sh).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.zones.len(),
            _ => 0,
        };
        for z in 1..=n {
            s.scene.set_zone(*sh, z as u16, None, Some(map))?;
        }
    }
    s.view.show_textures();
    Ok(format!("Mapped {} object(s)", shapes.len()))
}

/// polyPlanarProjection / polyCylindricalProjection / polySphericalProjection / polyAutoProjection
pub fn projection(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, PROJ_SPEC)?;
    let mode = match cmd {
        "polyCylindricalProjection" => 4,
        "polySphericalProjection" => 5,
        "polyAutoProjection" => 0,
        _ => match p.string("mapDirection").unwrap_or_else(|| "b".to_string()).to_lowercase().as_str() {
            "x" => 1,
            "y" => 2,
            "z" => 3,
            "b" | "best" => 6,
            o => return Err(format!("Unknown map direction '{}': use x, y, z or b (best plane)", o)),
        },
    };
    let msg = apply_projection(s, &p.arg_strings(), UvMap { mode, ..UvMap::default() })?;
    Ok(vec![msg])
}

const UV_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["uValue", "u"], arity: 1 },
    FlagSpec { names: &["vValue", "v"], arity: 1 },
    FlagSpec { names: &["scaleU", "su"], arity: 1 },
    FlagSpec { names: &["scaleV", "sv"], arity: 1 },
    FlagSpec { names: &["angle", "a"], arity: 1 },
    FlagSpec { names: &["tilingU", "tu"], arity: 1 },
    FlagSpec { names: &["tilingV", "tv"], arity: 1 },
    FlagSpec { names: &["offsetU", "ou"], arity: 1 },
    FlagSpec { names: &["offsetV", "ov"], arity: 1 },
    FlagSpec { names: &["rotate", "rot"], arity: 1 },
    FlagSpec { names: &["flipU"], arity: 0 },
    FlagSpec { names: &["flipV"], arity: 0 },
    FlagSpec { names: &["pivotU", "pu"], arity: 1 },
    FlagSpec { names: &["pivotV", "pv"], arity: 1 },
];

/// `polyEditUV [-u DU] [-v DV] [-su S] [-sv S] [-a DEG] [-flipU] [-flipV] [obj.f[..]]` moves, scales, rotates or
/// mirrors the texture on faces (or whole objects); `-tu -tv -ou -ov -rot` set the values outright.
pub fn poly_edit_uv(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, UV_SPEC)?;
    if let Some(r) = super::cmd_uv::edit_selected_uvs(s, &p)? {
        return Ok(r);
    }
    let n = |k: &str| p.num(k);
    let (du, dv) = (n("uValue")?.unwrap_or(0.0), n("vValue")?.unwrap_or(0.0));
    let (su, sv) = (n("scaleU")?.unwrap_or(1.0), n("scaleV")?.unwrap_or(1.0));
    let da = n("angle")?.unwrap_or(0.0);
    let (tu, tv, ou, ov, rot) = (n("tilingU")?, n("tilingV")?, n("offsetU")?, n("offsetV")?, n("rotate")?);
    if su == 0.0 || sv == 0.0 || tu == Some(0.0) || tv == Some(0.0) {
        return Err("A scale of zero would collapse the texture".to_string());
    }
    let (fu, fv) = (p.has("flipU"), p.has("flipV"));
    let edit = move |mut m: UvMap| -> UvMap {
        m.off_u += du;
        m.off_v += dv;
        m.tile_u *= su * if fu { -1.0 } else { 1.0 };
        m.tile_v *= sv * if fv { -1.0 } else { 1.0 };
        m.rot_deg += da;
        if let Some(v) = tu {
            m.tile_u = v;
        }
        if let Some(v) = tv {
            m.tile_v = v;
        }
        if let Some(v) = ou {
            m.off_u = v;
        }
        if let Some(v) = ov {
            m.off_v = v;
        }
        if let Some(v) = rot {
            m.rot_deg = v;
        }
        m
    };
    let args = p.arg_strings();
    if let Some(t) = face_target(s, &args)? {
        s.scene.assign_faces_with(t.shape, &t.faces, |m, uv| (m, edit(uv)))?;
        s.view.show_textures();
        return Ok(vec![format!("Edited the texture on {} face(s)", t.faces.len())]);
    }
    let shapes = shapes_of_selection(s, &non_component(&args))?;
    for sh in &shapes {
        let base = s.scene.shape_uv_map(*sh);
        s.scene.set_shape_uv_map(*sh, &edit(base))?;
        let count = match s.scene.node(*sh).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.zones.len(),
            _ => 0,
        };
        for z in 1..=count {
            let (_, uv) = s.scene.zone_info(*sh, z as u16)?;
            s.scene.set_zone(*sh, z as u16, None, Some(edit(uv)))?;
        }
    }
    Ok(vec![format!("Edited the texture on {} object(s)", shapes.len())])
}

const MODE_NAMES: [&str; 7] = ["automatic (box)", "planar X", "planar Y", "planar Z", "cylindrical", "spherical", "best plane"];

/// `textureZones [-clear] [object]`: list the face groups that have their own material or projection.
pub fn texture_zones(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, TEX_SPEC)?;
    let shapes = shapes_of_selection(s, &non_component(&p.arg_strings()))?;
    let mut out = Vec::new();
    for sh in shapes {
        let name = s.scene.name_of(sh);
        if p.has("clear") {
            s.scene.clear_zones(sh);
            out.push(format!("{}: texture zones cleared", name));
            continue;
        }
        let fz = s.scene.face_zones(sh)?;
        let count = match s.scene.node(sh).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.zones.len(),
            _ => 0,
        };
        for z in 0..=count {
            let faces = fz.iter().filter(|x| **x as usize == z).count();
            if z > 0 && faces == 0 {
                continue;
            }
            let (m, uv) = s.scene.zone_info(sh, z as u16)?;
            let mat = m.map(|m| s.scene.name_of(m)).unwrap_or_else(|| "(default)".to_string());
            out.push(format!(
                "{} zone {}{}: {} faces, material {}, {} projection, tiling {} {}, offset {} {}, rotate {}",
                name,
                z,
                if z == 0 { " (base)" } else { "" },
                faces,
                mat,
                MODE_NAMES[(uv.mode as usize).min(6)],
                fmt_f(uv.tile_u),
                fmt_f(uv.tile_v),
                fmt_f(uv.off_u),
                fmt_f(uv.off_v),
                fmt_f(uv.rot_deg)
            ));
        }
    }
    Ok(out)
}
