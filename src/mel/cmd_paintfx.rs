//! Paint Effects: brushes, strokes on surfaces, the 2D canvas, presets and conversion to polygons.

use super::commands::{resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::NodeData;
use crate::dag::NodeId;
use crate::geo::paintfx::{self, PRESET_FOLDERS};
use crate::math::Vec3;
use crate::session::Session;

/// The 2D Paint Effects canvas: linear RGBA pixels, row 0 at the top.
#[derive(Clone, Debug)]
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub pix: Vec<[f32; 4]>,
}

impl Default for Canvas {
    fn default() -> Canvas {
        Canvas { w: 512, h: 512, pix: vec![[1.0, 1.0, 1.0, 1.0]; 512 * 512] }
    }
}

fn find_brush(s: &mut Session, name: Option<String>) -> Result<NodeId, String> {
    match name {
        Some(n) => {
            let id = s.scene.find(&n).ok_or_else(|| format!("No brush named {}", n))?;
            if matches!(s.scene.node(id).map(|x| &x.data), Some(NodeData::Brush(_))) {
                Ok(id)
            } else {
                Err(format!("'{}' is not a brush", n))
            }
        }
        None => match s.scene.brush_ids().last() {
            Some(b) => Ok(*b),
            None => s.scene.create_brush(Some("gloppy")),
        },
    }
}

fn nums(text: &str) -> Vec<f32> {
    text.split_whitespace().filter_map(|x| x.parse().ok()).collect()
}

/// The first hit of a world space ray on a mesh shape: position and the normal facing the ray.
pub fn surface_hit(s: &mut Session, shape: NodeId, origin: Vec3, dir: Vec3) -> Option<(Vec3, Vec3)> {
    let owner = s.scene.node(shape).and_then(|n| n.parent).unwrap_or(shape);
    let w = s.scene.world_matrix(owner);
    let md = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md,
        _ => return None,
    };
    let mut best: Option<(f32, Vec3)> = None;
    for f in &md.mesh.faces {
        for k in 1..f.len().saturating_sub(1) {
            let a = w.transform_point(md.mesh.positions[f[0] as usize]);
            let b = w.transform_point(md.mesh.positions[f[k] as usize]);
            let c = w.transform_point(md.mesh.positions[f[k + 1] as usize]);
            let (e1, e2) = (b - a, c - a);
            let pv = dir.cross(e2);
            let det = e1.dot(pv);
            if det.abs() < 1e-12 {
                continue;
            }
            let id = 1.0 / det;
            let tv = origin - a;
            let u = tv.dot(pv) * id;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let qv = tv.cross(e1);
            let v = dir.dot(qv) * id;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(qv) * id;
            if t > 1e-5 && best.map(|x| t < x.0).unwrap_or(true) {
                let mut n = e1.cross(e2).normalized();
                if n.dot(dir) > 0.0 {
                    n = n * -1.0;
                }
                best = Some((t, n));
            }
        }
    }
    best.map(|(t, n)| (origin + dir * t, n))
}

fn is_paintable(s: &Session, shape: NodeId) -> bool {
    s.scene.node(shape).and_then(|n| n.plug("paintable")).map(|p| p.value > 0.5).unwrap_or(false)
}

fn surface_arg(s: &mut Session, name: &str) -> Result<NodeId, String> {
    let ids = resolve_targets(s, &[name.to_string()])?;
    let shape = ids.iter().flat_map(|i| s.scene.mesh_shapes_under(*i)).next().ok_or("The surface must be a polygon object")?;
    if !is_paintable(s, shape) {
        return Err(format!("'{}' is not paintable: run makePaintable on it first", name));
    }
    Ok(shape)
}

/// `makePaintable [-off] mesh`: Paint Effects > Make Paintable.
pub fn make_paintable(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["off"], arity: 0 }])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut out = Vec::new();
    for id in ids {
        for sh in s.scene.mesh_shapes_under(id) {
            if let Some(n) = s.scene.nodes.get_mut(sh) {
                if n.plug("paintable").is_none() {
                    let mut pl = crate::dag::plugs::Plug::new("paintable", 0.0);
                    pl.custom = true;
                    n.plugs.push(pl);
                }
            }
            s.scene.set_attr(sh, "paintable", if p.has("off") { 0.0 } else { 1.0 })?;
            out.push(s.scene.name_of(sh));
        }
    }
    if out.is_empty() {
        return Err("Select a polygon object".to_string());
    }
    Ok(out)
}

const BRUSH_SPEC: &[FlagSpec] = &[FlagSpec { names: &["preset", "p"], arity: 1 }, FlagSpec { names: &["list", "l"], arity: 0 }];

/// `paintBrush [-preset NAME]` creates a brush node (Visor: pick a preset); `paintBrush -list` shows the Visor folders.
pub fn paint_brush(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, BRUSH_SPEC)?;
    if p.has("list") {
        return Ok(PRESET_FOLDERS.iter().map(|(f, b)| format!("{}: {}", f, b.join(" "))).collect());
    }
    let id = s.scene.create_brush(p.string("preset").as_deref())?;
    s.scene.selection = vec![id];
    Ok(vec![s.scene.name_of(id)])
}

const STROKE_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["brush", "b"], arity: 1 },
    FlagSpec { names: &["surface", "s"], arity: 1 },
    FlagSpec { names: &["dir"], arity: 1 },
    FlagSpec { names: &["pts"], arity: 1 },
    FlagSpec { names: &["seed"], arity: 1 },
];

/// `pfxStroke -b BRUSH [-surface MESH [-dir "dx dy dz"]] -pts "x y z x y z ..."`: paint a stroke. On a paintable
/// surface each point is projected along `dir` (default straight down); free strokes lie in the air facing up.
pub fn paint_stroke(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, STROKE_SPEC)?;
    let brush = find_brush(s, p.string("brush"))?;
    let pts = nums(&p.string("pts").ok_or("Usage: pfxStroke -b BRUSH [-surface MESH] -pts \"x y z ...\"")?);
    if pts.len() < 3 || pts.len() % 3 != 0 {
        return Err("-pts needs groups of three numbers".to_string());
    }
    let dir = p.string("dir").map(|d| nums(&d)).filter(|d| d.len() == 3).map(|d| Vec3::new(d[0], d[1], d[2]).normalized()).unwrap_or(Vec3::new(0.0, -1.0, 0.0));
    let surf = match p.string("surface") {
        Some(n) => Some(surface_arg(s, &n)?),
        None => None,
    };
    let mut path = Vec::new();
    for c in pts.chunks_exact(3) {
        let pt = Vec3::new(c[0], c[1], c[2]);
        match surf {
            Some(sh) => {
                // start well behind the point so the ray crosses the surface
                if let Some(h) = surface_hit(s, sh, pt - dir * 1000.0, dir) {
                    path.push(h);
                }
            }
            None => path.push((pt, Vec3::UP)),
        }
    }
    if path.is_empty() {
        return Err("The stroke does not touch the surface".to_string());
    }
    let seed = p.num("seed")?.map(|x| x as u32).unwrap_or(s.scene.stroke_shapes().len() as u32 + 1);
    let shape = s.scene.create_stroke(brush, path, seed);
    let t = s.scene.node(shape).and_then(|n| n.parent).unwrap_or(shape);
    s.scene.selection = vec![t];
    Ok(vec![s.scene.name_of(t)])
}

const AUTO_SPEC: &[FlagSpec] = &[FlagSpec { names: &["brush", "b"], arity: 1 }, FlagSpec { names: &["count", "c"], arity: 1 }, FlagSpec { names: &["seed"], arity: 1 }];

/// `autoPaint -b BRUSH -count N mesh`: Paint Effects > Auto Paint: N short strokes scattered over a paintable surface.
pub fn auto_paint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, AUTO_SPEC)?;
    let brush = find_brush(s, p.string("brush"))?;
    let name = p.arg_strings().first().cloned().ok_or("Usage: autoPaint -b BRUSH -count N mesh")?;
    let shape = surface_arg(s, &name)?;
    let owner = s.scene.node(shape).and_then(|n| n.parent).unwrap_or(shape);
    let w = s.scene.world_matrix(owner);
    let tris: Vec<[Vec3; 3]> = match s.scene.node(shape).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md.mesh.faces.iter().flat_map(|f| (1..f.len().saturating_sub(1)).map(|k| [w.transform_point(md.mesh.positions[f[0] as usize]), w.transform_point(md.mesh.positions[f[k] as usize]), w.transform_point(md.mesh.positions[f[k + 1] as usize])]).collect::<Vec<_>>()).collect(),
        _ => Vec::new(),
    };
    if tris.is_empty() {
        return Err("The surface has no faces".to_string());
    }
    let count = p.num("count")?.unwrap_or(10.0).clamp(1.0, 500.0) as usize;
    let total: f32 = tris.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length()).sum();
    let mut state = p.num("seed")?.map(|x| x as u64).unwrap_or(7).wrapping_mul(2654435761).wrapping_add(1);
    let mut rnd = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ((state >> 40) as f32) / (1u64 << 24) as f32
    };
    let mut made = Vec::new();
    for i in 0..count {
        // choose a triangle by area
        let mut pick = rnd() * total;
        let mut tri = &tris[0];
        for t in &tris {
            let a = (t[1] - t[0]).cross(t[2] - t[0]).length();
            if pick <= a {
                tri = t;
                break;
            }
            pick -= a;
        }
        let nrm = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalized();
        let mut path = Vec::new();
        let (mut u, mut v) = (rnd(), rnd());
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        for k in 0..3 {
            let (uu, vv) = ((u + 0.05 * k as f32).min(1.0 - v.min(1.0)), v);
            path.push((tri[0] + (tri[1] - tri[0]) * uu + (tri[2] - tri[0]) * vv, nrm));
        }
        let sh = s.scene.create_stroke(brush, path, i as u32 + 11);
        made.push(s.scene.name_of(s.scene.node(sh).and_then(|n| n.parent).unwrap_or(sh)));
    }
    Ok(made)
}

/// `shareBrush stroke stroke ...`: Paint Effects > Share One Brush: every stroke uses the brush of the first.
pub fn share_brush(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let shapes: Vec<NodeId> = ids.iter().flat_map(|i| s.scene.mesh_shapes_under(*i)).filter(|sh| matches!(s.scene.node(*sh).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.stroke.is_some())).collect();
    if shapes.len() < 2 {
        return Err("Select at least two strokes".to_string());
    }
    let brush = match s.scene.node(shapes[0]).map(|n| &n.data) {
        Some(NodeData::Mesh(md)) => md.stroke.as_ref().and_then(|st| st.brush),
        _ => None,
    }
    .ok_or("The first stroke has no brush")?;
    let name = s.scene.name_of(brush);
    for sh in &shapes[1..] {
        if let Some(NodeData::Mesh(md)) = s.scene.nodes.get_mut(*sh).map(|n| &mut n.data) {
            if let Some(st) = md.stroke.as_mut() {
                st.brush = Some(brush);
                st.brush_name = name.clone();
                st.key = 0;
            }
        }
    }
    s.scene.flush_strokes();
    Ok(vec![name])
}

/// `brushFromStroke stroke`: Get Settings from Selected Stroke: select the brush the stroke uses.
pub fn brush_from_stroke(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    for sh in ids.iter().flat_map(|i| s.scene.mesh_shapes_under(*i)) {
        if let Some(NodeData::Mesh(md)) = s.scene.node(sh).map(|n| &n.data) {
            if let Some(b) = md.stroke.as_ref().and_then(|st| st.brush) {
                s.scene.selection = vec![b];
                return Ok(vec![s.scene.name_of(b)]);
            }
        }
    }
    Err("Select a Paint Effects stroke".to_string())
}

const BLEND_SPEC: &[FlagSpec] = &[FlagSpec { names: &["a"], arity: 1 }, FlagSpec { names: &["b"], arity: 1 }, FlagSpec { names: &["amount"], arity: 1 }, FlagSpec { names: &["shading"], arity: 0 }];

/// `blendBrush -a BRUSH -b BRUSH [-amount 0.5] [-shading]`: Blend Brush 50% or Blend Shading into a new brush.
pub fn blend_brush(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, BLEND_SPEC)?;
    let a = find_brush(s, p.string("a"))?;
    let b = find_brush(s, p.string("b"))?;
    let vals = paintfx::blend(&s.scene.brush_values(a), &s.scene.brush_values(b), p.num("amount")?.unwrap_or(0.5), p.has("shading"));
    let id = s.scene.create_brush(None)?;
    s.scene.set_brush_values(id, &vals);
    s.scene.selection = vec![id];
    Ok(vec![s.scene.name_of(id)])
}

/// `brushPreset -save BRUSH FILE` / `brushPreset -load FILE`: Save Brush Preset and loading it again.
pub fn brush_preset(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["save"], arity: 1 }, FlagSpec { names: &["load"], arity: 1 }])?;
    if let Some(b) = p.string("save") {
        let id = find_brush(s, Some(b))?;
        let file = p.arg_strings().first().cloned().ok_or("Usage: brushPreset -save BRUSH FILE")?;
        let text: String = s.scene.brush_values(id).iter().map(|(k, v)| format!("{} {}\n", k, v)).collect();
        std::fs::write(&file, text).map_err(|e| format!("Cannot write {}: {}", file, e))?;
        return Ok(vec![file]);
    }
    let file = p.string("load").ok_or("Usage: brushPreset -save BRUSH FILE | -load FILE")?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("Cannot read {}: {}", file, e))?;
    let vals: Vec<(String, f32)> = text.lines().filter_map(|l| { let mut it = l.split_whitespace(); Some((it.next()?.to_string(), it.next()?.parse().ok()?)) }).collect();
    let id = s.scene.create_brush(None)?;
    s.scene.set_brush_values(id, &vals);
    Ok(vec![s.scene.name_of(id)])
}

/// `strokeToPoly stroke`: Modify > Convert > Paint Effects to Polygons.
pub fn stroke_to_poly(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut out = Vec::new();
    for sh in ids.iter().flat_map(|i| s.scene.mesh_shapes_under(*i)).collect::<Vec<_>>() {
        if matches!(s.scene.node(sh).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.stroke.is_some()) {
            let t = s.scene.stroke_to_poly(sh)?;
            out.push(s.scene.name_of(t));
        }
    }
    if out.is_empty() {
        return Err("Select a Paint Effects stroke".to_string());
    }
    Ok(out)
}

/// `strokeInfo stroke`: faces, tubes' vertices and the brush of a stroke.
pub fn stroke_info(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let mut out = Vec::new();
    for sh in ids.iter().flat_map(|i| s.scene.mesh_shapes_under(*i)) {
        if let Some(NodeData::Mesh(md)) = s.scene.node(sh).map(|n| &n.data) {
            if let Some(st) = &md.stroke {
                out.push(format!("{}: {} faces, {} vertices, brush {}", s.scene.name_of(sh), md.mesh.faces.len(), md.mesh.positions.len(), st.brush.map(|b| s.scene.name_of(b)).unwrap_or_default()));
            }
        }
    }
    Ok(out)
}

const CANVAS_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["size"], arity: 1 },
    FlagSpec { names: &["clear"], arity: 1 },
    FlagSpec { names: &["brush", "b"], arity: 1 },
    FlagSpec { names: &["pts"], arity: 1 },
    FlagSpec { names: &["save"], arity: 1 },
    FlagSpec { names: &["query", "q"], arity: 0 },
];

/// `paintCanvas`: the Paint Effects canvas. `-size "w h"`, `-clear "r g b"`, `-b BRUSH -pts "x y x y ..."` (pixels, y up),
/// `-save FILE.png`, `-q` for the number of painted pixels.
pub fn paint_canvas(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, CANVAS_SPEC)?;
    if let Some(sz) = p.string("size") {
        let v = nums(&sz);
        if v.len() != 2 || v[0] < 4.0 || v[1] < 4.0 || v[0] > 4096.0 || v[1] > 4096.0 {
            return Err("-size needs \"width height\" between 4 and 4096".to_string());
        }
        s.canvas = Canvas { w: v[0] as usize, h: v[1] as usize, pix: vec![[1.0, 1.0, 1.0, 1.0]; v[0] as usize * v[1] as usize] };
    }
    if let Some(c) = p.string("clear") {
        let v = nums(&c);
        if v.len() != 3 {
            return Err("-clear needs \"r g b\"".to_string());
        }
        for px in s.canvas.pix.iter_mut() {
            *px = [v[0], v[1], v[2], 1.0];
        }
    }
    if let Some(pts) = p.string("pts") {
        let brush = find_brush(s, p.string("brush"))?;
        let v = nums(&pts);
        if v.len() < 2 || v.len() % 2 != 0 {
            return Err("-pts needs groups of two numbers".to_string());
        }
        let path: Vec<(Vec3, Vec3)> = v.chunks_exact(2).map(|c| (Vec3::new(c[0], c[1], 0.0), Vec3::UP)).collect();
        // brush sizes are in canvas pixels: a unit of 1 is 64 pixels
        let params = s.scene.brush_params(brush);
        let scaled: Vec<(String, f32)> = paintfx::BRUSH_DEFAULTS.iter().map(|(k, _)| (k.to_string(), params.g(k))).map(|(k, v)| match k.as_str() {
            "brushWidth" | "lengthMin" | "lengthMax" | "tubeWidth1" | "tubeWidth2" | "leafSize" | "flowerSize" => (k, v * 64.0),
            _ => (k, v),
        }).collect();
        let sp = paintfx::Params::new(scaled);
        // the canvas stroke grows toward +y (the normal of a stroke path pointing up the page)
        let growth = paintfx::grow(&sp, &path, 5, 100.0, 4000);
        let (w, h) = (s.canvas.w, s.canvas.h);
        paintfx::rasterize(&mut s.canvas.pix, w, h, &sp, &growth);
    }
    if let Some(file) = p.string("save") {
        let c = &s.canvas;
        let mut img = image::RgbImage::new(c.w as u32, c.h as u32);
        for (i, px) in img.pixels_mut().enumerate() {
            let v = c.pix[i];
            *px = image::Rgb([(v[0].clamp(0.0, 1.0) * 255.0).round() as u8, (v[1].clamp(0.0, 1.0) * 255.0).round() as u8, (v[2].clamp(0.0, 1.0) * 255.0).round() as u8]);
        }
        img.save(&file).map_err(|e| format!("Cannot save {}: {}", file, e))?;
        return Ok(vec![file]);
    }
    let white = s.canvas.pix.iter().filter(|px| px[0] < 0.999 || px[1] < 0.999 || px[2] < 0.999).count();
    Ok(vec![format!("canvas {}x{}, {} painted pixel(s)", s.canvas.w, s.canvas.h, white)])
}
