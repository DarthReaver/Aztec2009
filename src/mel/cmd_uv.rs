//! Editable UVs (UV Texture Editor): projections that create a UV set, UV selection, transforms,
//! layout, cut / sew, unfold.

use super::cmd_model::current_shape;
use super::cmd_tex::{face_target, shapes_of_selection};
use super::commands::{fmt_f, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec, Parsed};
use crate::dag::nodes::{MeshData, NodeData};
use crate::dag::NodeId;
use crate::geo::edit::UvMap;
use crate::geo::uv::{self, UvSet};
use crate::math::Vec2;
use crate::session::{SelMode, Session};
use std::collections::BTreeSet;

fn md_mut(s: &mut Session, shape: NodeId) -> Result<&mut MeshData, String> {
    match s.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
        Some(NodeData::Mesh(md)) if md.nurbs.is_some() => Err("NURBS surfaces have parametric UVs; convert to polygons (nurbsToPoly) to edit UVs".to_string()),
        Some(NodeData::Mesh(md)) => Ok(md),
        _ => Err("Not a polygon object".to_string()),
    }
}

/// The shape whose UVs are edited: the selected component node or object.
fn uv_shape(s: &mut Session) -> Result<NodeId, String> {
    let sh = current_shape(s).ok_or("Select a polygon object first")?;
    s.scene.flush_meshes();
    Ok(sh)
}

/// The UV set of the shape, created from the default automatic mapping when it has none yet.
fn ensure_set(md: &mut MeshData) {
    md.ensure_custom();
    if md.uv_set.is_none() {
        let mut set = uv::project(&md.mesh, None, UvMap::default(), None);
        set.sig = Vec::new();
        md.uv_set = Some(set);
        md.touch();
        if let Some(set) = md.uv_set.as_mut() {
            if set.sig.is_empty() {
                md.sync_uvs();
            }
        }
    }
}

const PROJ_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["type", "t"], arity: 1 },
    FlagSpec { names: &["mapDirection", "md"], arity: 1 },
    FlagSpec { names: &["layout", "l"], arity: 0 },
];

fn finish(md: &mut MeshData, set: UvSet) {
    md.uv_set = Some(set);
    md.touch();
}

/// `polyProjection -type planar|cylindrical|spherical [-md x|y|z|b] [obj.f[..]]` creates real UVs by
/// projecting the object (or the selected faces). `polyAutoProjection` flattens along six directions and packs the shells.
pub fn projection(s: &mut Session, name: &str, rest: &[Token]) -> Out {
    let p = parse_args(rest, PROJ_SPEC)?;
    let args = p.arg_strings();
    let auto = name == "polyAutoProjection";
    let (kind, map) = if auto {
        ("automatic".to_string(), UvMap::default())
    } else {
        let kind = p.string("type").unwrap_or_else(|| "planar".into()).to_lowercase();
        let md = p.string("mapDirection").unwrap_or_else(|| "z".into()).to_lowercase();
        let mode = match kind.as_str() {
            "planar" => match md.as_str() {
                "x" => 1,
                "y" => 2,
                "z" => 3,
                "b" | "best" => 6,
                o => return Err(format!("Unknown map direction '{}': x, y, z or b (best plane)", o)),
            },
            "best" | "bestplane" => 6,
            "cylindrical" => 4,
            "spherical" => 5,
            o => return Err(format!("Unknown projection '{}': planar, cylindrical or spherical", o)),
        };
        (kind, UvMap { mode, ..UvMap::default() })
    };
    let target = face_target(s, &args)?;
    let (shapes, faces): (Vec<NodeId>, Option<BTreeSet<u32>>) = match target {
        Some(t) => (vec![t.shape], Some(t.faces.into_iter().collect())),
        None => (shapes_of_selection(s, &args.iter().filter(|a| !a.contains('[')).cloned().collect::<Vec<_>>())?, None),
    };
    for sh in &shapes {
        let md = md_mut(s, *sh)?;
        md.ensure_custom();
        let base = md.uv_set.clone();
        let set = if auto { uv::automatic(&md.mesh, faces.as_ref(), base.as_ref()) } else { uv::project(&md.mesh, faces.as_ref(), map, base.as_ref()) };
        finish(md, set);
    }
    s.comp.uv_sel.clear();
    s.view.show_textures();
    Ok(vec![format!("{} projection on {}", kind, match &faces { Some(f) => format!("{} face(s)", f.len()), None => format!("{} object(s)", shapes.len()) })])
}

/// `polyMapDel`: remove the UV set; the texture goes back to the projection attributes.
pub fn map_del(s: &mut Session) -> Out {
    let sh = uv_shape(s)?;
    let md = md_mut(s, sh)?;
    if md.uv_set.take().is_none() {
        return Err("The object has no UVs of its own".to_string());
    }
    md.touch();
    s.comp.uv_sel.clear();
    Ok(vec!["UVs deleted".to_string()])
}

/// `uvSelect -all | -clear | -shell | -faces | -index N ... | -box U0 V0 U1 V1 | -q`: UV selection of the UV Texture Editor.
pub fn uv_select(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(
        rest,
        &[
            FlagSpec { names: &["all"], arity: 0 },
            FlagSpec { names: &["clear", "cl"], arity: 0 },
            FlagSpec { names: &["shell", "sh"], arity: 0 },
            FlagSpec { names: &["faces", "f"], arity: 0 },
            FlagSpec { names: &["box"], arity: 4 },
            FlagSpec { names: &["add"], arity: 0 },
            FlagSpec { names: &["query", "q"], arity: 0 },
            FlagSpec { names: &["invert"], arity: 0 },
        ],
    )?;
    let sh = uv_shape(s)?;
    let faces_sel: BTreeSet<u32> = if s.comp.mode == SelMode::Face && s.comp.node == Some(sh) { s.comp.sel.faces.clone() } else { BTreeSet::new() };
    let mut cur = if s.comp.uv_sel.is_empty() || p.has("add") || p.has("shell") { s.comp.uv_sel.clone() } else { BTreeSet::new() };
    let md = md_mut(s, sh)?;
    ensure_set(md);
    let set = md.uv_set.clone().unwrap();
    if p.has("query") {
        return Ok(s.comp.uv_sel.iter().map(|u| u.to_string()).collect());
    }
    if p.has("clear") {
        cur.clear();
    } else if p.has("all") {
        cur = (0..set.uvs.len() as u32).collect();
    } else if p.has("faces") {
        if faces_sel.is_empty() {
            return Err("Select faces first (the UVs of the faces get selected)".to_string());
        }
        cur = set.uvs_of_faces(&faces_sel);
    } else if let Some(b) = p.get("box") {
        let v: Vec<f32> = b.iter().map(|t| t.as_f32()).collect::<Result<_, _>>()?;
        let (u0, v0, u1, v1) = (v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3]));
        let hit: BTreeSet<u32> = set.uvs.iter().enumerate().filter(|(_, p)| p.x >= u0 && p.x <= u1 && p.y >= v0 && p.y <= v1).map(|(i, _)| i as u32).collect();
        if p.has("add") {
            cur.extend(hit);
        } else {
            cur = hit;
        }
    } else if !p.args.is_empty() {
        let idx: BTreeSet<u32> = p.args.iter().filter_map(|a| a.as_string().parse::<u32>().ok()).filter(|&i| (i as usize) < set.uvs.len()).collect();
        if idx.is_empty() {
            return Err(format!("UV index out of range (the object has {} UVs)", set.uvs.len()));
        }
        if p.has("add") {
            cur.extend(idx);
        } else {
            cur = idx;
        }
    }
    if p.has("shell") {
        cur = set.expand_to_shells(&cur);
    }
    if p.has("invert") {
        cur = (0..set.uvs.len() as u32).filter(|u| !cur.contains(u)).collect();
    }
    s.comp.uv_sel = cur;
    Ok(vec![format!("{} UV(s) selected", s.comp.uv_sel.len())])
}

/// `uvInfo [-q]`: UV count, shells and the bounds of the selection.
pub fn uv_info(s: &mut Session) -> Out {
    let sh = uv_shape(s)?;
    let sel = s.comp.uv_sel.clone();
    let md = md_mut(s, sh)?;
    let Some(set) = md.uv_set.as_ref() else { return Ok(vec!["No UVs of its own (projection attributes in use)".to_string()]) };
    let mut out = vec![format!("{} UVs in {} shell(s)", set.uvs.len(), set.shells().len())];
    if let Some((lo, hi)) = set.bounds(&sel) {
        out.push(format!("selection {} UVs: u {} .. {}  v {} .. {}", sel.len(), fmt_f(lo.x), fmt_f(hi.x), fmt_f(lo.y), fmt_f(hi.y)));
    }
    Ok(out)
}

/// Used by polyEditUV: move / scale / rotate / flip the selected UVs about their center (or `-pu -pv`).
pub fn edit_selected_uvs(s: &mut Session, p: &Parsed) -> Result<Option<Vec<String>>, String> {
    if s.comp.uv_sel.is_empty() {
        return Ok(None);
    }
    let Some(sh) = current_shape(s) else { return Ok(None) };
    let sel = s.comp.uv_sel.clone();
    let n = |k: &str| p.num(k);
    let (du, dv) = (n("uValue")?.unwrap_or(0.0), n("vValue")?.unwrap_or(0.0));
    let (su, sv) = (n("scaleU")?.unwrap_or(1.0), n("scaleV")?.unwrap_or(1.0));
    let ang = n("angle")?.unwrap_or(0.0).to_radians();
    let (fu, fv) = (p.has("flipU"), p.has("flipV"));
    if su == 0.0 || sv == 0.0 {
        return Err("A scale of zero would collapse the UVs".to_string());
    }
    let (pu, pv) = (n("pivotU")?, n("pivotV")?);
    let md = md_mut(s, sh)?;
    let Some(set) = md.uv_set.as_mut() else { return Ok(None) };
    let sel: BTreeSet<u32> = sel.into_iter().filter(|u| (*u as usize) < set.uvs.len()).collect();
    let (lo, hi) = set.bounds(&sel).ok_or("No UVs selected")?;
    let piv = Vec2::new(pu.unwrap_or((lo.x + hi.x) * 0.5), pv.unwrap_or((lo.y + hi.y) * 0.5));
    let (sn, cs) = ang.sin_cos();
    set.transform(&sel, |q| {
        let (mut x, mut y) = (q.x - piv.x, q.y - piv.y);
        x *= su * if fu { -1.0 } else { 1.0 };
        y *= sv * if fv { -1.0 } else { 1.0 };
        let (rx, ry) = (x * cs - y * sn, x * sn + y * cs);
        Vec2::new(piv.x + rx + du, piv.y + ry + dv)
    });
    md.touch();
    Ok(Some(vec![format!("Edited {} UV(s)", sel.len())]))
}

/// `polyNormalizeUV [-ar 0|1]`: fit the selected UVs (or all) into the 0..1 square; -ar 1 keeps the aspect.
pub fn normalize_uv(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["centerOnTile", "ct"], arity: 1 }, FlagSpec { names: &["preserveAspectRatio", "pa"], arity: 1 }])?;
    let keep = p.num("preserveAspectRatio")?.map(|v| v > 0.5).unwrap_or(true);
    let sh = uv_shape(s)?;
    let sel = s.comp.uv_sel.clone();
    let md = md_mut(s, sh)?;
    ensure_set(md);
    let set = md.uv_set.as_mut().unwrap();
    let all: BTreeSet<u32> = (0..set.uvs.len() as u32).collect();
    let target = if sel.is_empty() { all } else { sel };
    set.normalize(&target, keep);
    md.touch();
    Ok(vec!["Normalized".to_string()])
}

/// `polyLayoutUV [-sf folds] [-p PADDING]`: separate mirrored (folded) faces into their own shells, then pack
/// the shells of the selection (or all) into the 0..1 square without overlap.
pub fn layout_uv(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["separateShells", "sf"], arity: 0 }, FlagSpec { names: &["padding", "p"], arity: 1 }])?;
    let sh = uv_shape(s)?;
    let sel = s.comp.uv_sel.clone();
    let md = md_mut(s, sh)?;
    ensure_set(md);
    let set = md.uv_set.as_mut().unwrap();
    let before = set.shells().len();
    if p.has("separateShells") {
        set.separate_folds();
    }
    set.layout(&sel, p.num("padding")?.unwrap_or(0.01).clamp(0.0, 0.2));
    let after = set.shells().len();
    md.touch();
    s.comp.uv_sel.clear();
    Ok(vec![format!("Laid out {} shell(s) ({} before separation)", after, before)])
}

/// `unfold [-i ITERATIONS]` (also polyRelaxUV): relax the interior of the selected UVs to reduce stretching.
pub fn unfold(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["iterations", "i"], arity: 1 }])?;
    let it = p.num("iterations")?.unwrap_or(20.0).clamp(1.0, 500.0) as usize;
    let sh = uv_shape(s)?;
    let sel = s.comp.uv_sel.clone();
    let md = md_mut(s, sh)?;
    ensure_set(md);
    md.uv_set.as_mut().unwrap().relax(&sel, it);
    md.touch();
    Ok(vec![format!("Relaxed with {} iterations", it)])
}

/// `polyMapCut` / `polyMapSew` on the selected mesh edges.
pub fn map_cut_sew(s: &mut Session, name: &str) -> Out {
    let sh = uv_shape(s)?;
    if s.comp.mode != SelMode::Edge || s.comp.node != Some(sh) || s.comp.sel.edges.is_empty() {
        return Err("Select the edges to cut or sew (F10) first".to_string());
    }
    let edges = s.comp.sel.edges.clone();
    let md = md_mut(s, sh)?;
    ensure_set(md);
    let weld_ids = uv::weld_ids(&md.mesh);
    let welds = uv::weld_faces(&md.mesh);
    let wedges: BTreeSet<(u32, u32)> = edges
        .iter()
        .filter_map(|&(a, b)| Some((*weld_ids.get(a as usize)?, *weld_ids.get(b as usize)?)))
        .map(|(a, b)| (a.min(b), a.max(b)))
        .collect();
    let set = md.uv_set.as_mut().unwrap();
    let msg = if name == "polyMapCut" {
        set.cut(&welds, &wedges);
        format!("Cut {} edge(s)", wedges.len())
    } else {
        let n = set.sew(&welds, &wedges);
        format!("Sewed {} UV point(s)", n)
    };
    set.compact();
    md.touch();
    s.comp.uv_sel.clear();
    Ok(vec![msg])
}
