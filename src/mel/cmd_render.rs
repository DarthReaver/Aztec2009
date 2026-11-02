//! Rendering commands: Render Settings, render layers and passes, light linking, render stats,
//! rendering images and animations, and the FCheck sequence tool.

use super::commands::{resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::dag::nodes::{NodeData, RenderLayerData};
use crate::dag::NodeId;
use crate::render::pipeline::{self, Settings};
use crate::session::Session;

/// The scene's defaultRenderGlobals node, made when missing.
pub fn globals(s: &mut Session) -> NodeId {
    if let Some(id) = s.scene.ids_where(|d| matches!(d, NodeData::RenderGlobals(_))).first() {
        return *id;
    }
    s.scene.create_node(NodeData::RenderGlobals(Box::default()), "defaultRenderGlobals", None, false)
}

const SET_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["renderer", "r"], arity: 1 },
    FlagSpec { names: &["width", "w"], arity: 1 },
    FlagSpec { names: &["height", "h"], arity: 1 },
    FlagSpec { names: &["quality", "q"], arity: 1 },
    FlagSpec { names: &["start", "s"], arity: 1 },
    FlagSpec { names: &["end", "e"], arity: 1 },
    FlagSpec { names: &["by", "b"], arity: 1 },
    FlagSpec { names: &["prefix"], arity: 1 },
    FlagSpec { names: &["ext"], arity: 1 },
    FlagSpec { names: &["camera", "cam"], arity: 1 },
    FlagSpec { names: &["motionBlur", "mb"], arity: 1 },
    FlagSpec { names: &["blurType"], arity: 1 },
    FlagSpec { names: &["blurLength"], arity: 1 },
    FlagSpec { names: &["blurSamples"], arity: 1 },
    FlagSpec { names: &["alpha"], arity: 1 },
    FlagSpec { names: &["depth"], arity: 1 },
    FlagSpec { names: &["animation"], arity: 1 },
    FlagSpec { names: &["raytracing"], arity: 1 },
    FlagSpec { names: &["layer", "l"], arity: 1 },
    FlagSpec { names: &["info", "i"], arity: 0 },
    FlagSpec { names: &["attr"], arity: 1 },
];

fn flag_num(name: &str, v: &str) -> Result<f32, String> {
    v.parse::<f32>().map_err(|_| format!("-{} needs a number, not '{}'", name, v))
}

/// `renderSettings [-renderer software|hardware|vector] [-w N -h N] [-quality preview|intermediate|production|high] [-s N -e N -b N]
/// [-prefix P] [-ext png] [-cam C] [-mb 0|1 -blurType 2d|3d -blurLength F] [-alpha 0|1] [-depth 0|1] [-layer L]`:
/// the Render Settings window. With `-layer` the change is an override for that render layer only.
pub fn render_settings(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SET_SPEC)?;
    let g = globals(s);
    let layer = p.string("layer");
    let mut nums: Vec<(String, f32)> = Vec::new();
    let mut texts: Vec<(&str, String)> = Vec::new();
    if let Some(r) = p.string("renderer") {
        let v = match r.to_lowercase().as_str() {
            "software" | "0" => 0.0,
            "hardware" | "1" => 1.0,
            "vector" | "2" => 2.0,
            o => return Err(format!("Unknown renderer '{}': software, hardware or vector (mental ray is not available)", o)),
        };
        nums.push(("renderer".to_string(), v));
    }
    if let Some(q) = p.string("quality") {
        let v = match q.to_lowercase().as_str() {
            "preview" | "0" => 0.0,
            "intermediate" | "1" => 1.0,
            "production" | "2" => 2.0,
            "high" | "3" => 3.0,
            o => return Err(format!("Unknown quality '{}': preview, intermediate, production or high", o)),
        };
        nums.push(("quality".to_string(), v));
    }
    for (flag, plug) in [("width", "width"), ("height", "height"), ("start", "startFrame"), ("end", "endFrame"), ("by", "byFrame"), ("motionBlur", "motionBlur"), ("blurLength", "blurLength"), ("blurSamples", "blurSamples"), ("alpha", "alphaChannel"), ("depth", "depthChannel"), ("animation", "animation"), ("raytracing", "raytracing")] {
        if let Some(v) = p.string(flag) {
            nums.push((plug.to_string(), flag_num(flag, &v)?));
        }
    }
    if let Some(b) = p.string("blurType") {
        nums.push(("motionBlurType".to_string(), match b.to_lowercase().as_str() {
            "2d" | "0" => 0.0,
            "3d" | "1" => 1.0,
            o => return Err(format!("Unknown blur type '{}': 2d or 3d", o)),
        }));
    }
    if let Some(a) = p.string("attr") {
        let mut it = a.split_whitespace();
        let (Some(k), Some(v)) = (it.next(), it.next()) else { return Err("-attr needs \"NAME VALUE\"".to_string()) };
        if s.scene.node(g).and_then(|n| n.plug(k)).is_none() {
            return Err(format!("There is no Render Settings attribute '{}'", k));
        }
        nums.push((k.to_string(), flag_num("attr", v)?));
    }
    for (flag, key) in [("prefix", "prefix"), ("ext", "ext"), ("camera", "camera")] {
        if let Some(v) = p.string(flag) {
            texts.push((key, v));
        }
    }
    if let Some(ext) = texts.iter().find(|(k, _)| *k == "ext").map(|(_, v)| v.to_lowercase()) {
        if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "tga") {
            return Err(format!("Image format '{}' is not available: png, jpg, bmp or tga", ext));
        }
    }
    match layer {
        Some(l) => {
            let layer = s.scene.render_layers.iter_mut().find(|x| x.name == l).ok_or_else(|| format!("No render layer named {}", l))?;
            for (k, v) in nums {
                layer.overrides.retain(|(n, _)| *n != k);
                layer.overrides.push((k, v));
            }
            for (k, v) in texts {
                layer.text_overrides.retain(|(n, _)| n != k);
                layer.text_overrides.push((k.to_string(), v));
            }
        }
        None => {
            for (k, v) in nums {
                s.scene.set_attr(g, &k, v)?;
            }
            if let Some(NodeData::RenderGlobals(d)) = s.scene.nodes.get_mut(g).map(|n| &mut n.data) {
                for (k, v) in texts {
                    match k {
                        "prefix" => d.prefix = v,
                        "ext" => d.ext = v.to_lowercase(),
                        _ => d.camera = v,
                    }
                }
            }
        }
    }
    let lname = p.string("layer");
    let layer_data = lname.as_ref().and_then(|l| s.scene.render_layers.iter().find(|x| &x.name == l)).cloned();
    let st = Settings::from_scene(&mut s.scene, layer_data.as_ref());
    Ok(vec![format!(
        "{} {}x{}, quality {}, frames {}..{} by {}, {}.{}, motion blur {}{}, alpha {}, depth {}",
        pipeline::RENDERERS[st.renderer as usize],
        st.w,
        st.h,
        ["preview", "intermediate", "production", "high"][st.quality as usize],
        st.start,
        st.end,
        st.by,
        if st.prefix.is_empty() { "<Scene>" } else { &st.prefix },
        st.ext,
        if st.motion_blur { "on" } else { "off" },
        if st.motion_blur { if st.blur_3d { " (3D)" } else { " (2D)" } } else { "" },
        st.alpha as u8,
        st.depth as u8
    )])
}

const RENDER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["layer", "l"], arity: 1 },
    FlagSpec { names: &["pass", "p"], arity: 1 },
    FlagSpec { names: &["cam"], arity: 1 },
    FlagSpec { names: &["region"], arity: 1 },
    FlagSpec { names: &["out", "o"], arity: 1 },
    FlagSpec { names: &["w"], arity: 1 },
    FlagSpec { names: &["h"], arity: 1 },
    FlagSpec { names: &["allPasses"], arity: 0 },
];

fn find_layer(s: &Session, name: &str) -> Result<RenderLayerData, String> {
    s.scene.render_layers.iter().find(|l| l.name == name).cloned().ok_or_else(|| format!("No render layer named {}", name))
}

/// `renderImage [-layer L] [-pass P | -allPasses] [-cam C] [-region "x0 y0 x1 y1"] [-w N -h N] [-o FILE]`: Render > Render Current Frame
/// with the Render Settings (and the overrides of the layer).
pub fn render_image(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, RENDER_SPEC)?;
    let layer = match p.string("layer") {
        Some(l) => Some(find_layer(s, &l)?),
        None => None,
    };
    let mut st = Settings::from_scene(&mut s.scene, layer.as_ref());
    if let Some(w) = p.num("w")? {
        st.w = w.clamp(16.0, 4096.0) as usize;
    }
    if let Some(h) = p.num("h")? {
        st.h = h.clamp(16.0, 4096.0) as usize;
    }
    let region = match p.string("region") {
        Some(r) => {
            let v: Vec<usize> = r.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() != 4 || v[2] <= v[0] || v[3] <= v[1] {
                return Err("-region needs \"x0 y0 x1 y1\" (pixels, the end is excluded)".to_string());
            }
            Some((v[0], v[1], v[2], v[3]))
        }
        None => None,
    };
    let cams = match p.string("cam") {
        Some(c) => vec![c],
        None => pipeline::render_cameras(&mut s.scene, &st),
    };
    let passes: Vec<String> = if p.has("allPasses") {
        let mut v = vec!["beauty".to_string()];
        if let Some(l) = &layer {
            v.extend(l.passes.iter().cloned());
        }
        v
    } else {
        vec![p.string("pass").unwrap_or_else(|| "beauty".to_string())]
    };
    let frame = s.scene.timeline.current.round() as i32;
    let started = std::time::Instant::now();
    let mut out = Vec::new();
    for cam in &cams {
        for pass in &passes {
            let img = pipeline::render_pass(&mut s.scene, &st, layer.as_ref(), cam, pass, region)?;
            let path = match (p.string("out"), passes.len() * cams.len()) {
                (Some(o), 1) => o,
                (Some(o), _) => format!("{}_{}", o, pass),
                (None, _) => pipeline::image_path(&s.scene, &st, layer.as_ref().map(|l| l.name.as_str()), cam, pass, frame),
            };
            img.save(&path, st.alpha || pass != "beauty")?;
            out.push(format!("{} {}x{} -> {}", pass, st.w, st.h, path));
        }
    }
    out.push(format!("Rendered in {:.2}s", started.elapsed().as_secs_f32()));
    Ok(out)
}

const BATCH_SPEC: &[FlagSpec] = &[FlagSpec { names: &["s", "start"], arity: 1 }, FlagSpec { names: &["e", "end"], arity: 1 }, FlagSpec { names: &["layer", "l"], arity: 1 }, FlagSpec { names: &["cam"], arity: 1 }];

/// `batchRender [-s N] [-e N] [-layer L] [-cam C]`: Render > Batch Render: every frame of the range, every renderable layer
/// and pass, to numbered files in the project's images folder.
pub fn batch_render(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, BATCH_SPEC)?;
    let started = std::time::Instant::now();
    let mut log = Vec::new();
    let files = pipeline::batch_render(&mut s.scene, p.num("s")?.map(|x| x as i32), p.num("e")?.map(|x| x as i32), p.string("layer").as_deref(), p.string("cam").as_deref(), &mut |f| log.push(f.to_string()))?;
    Ok(vec![format!("Batch render: {} image(s) in {:.1}s, first {}, last {}", files.len(), started.elapsed().as_secs_f32(), files.first().cloned().unwrap_or_default(), files.last().cloned().unwrap_or_default())])
}

const LAYER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["create", "c"], arity: 1 },
    FlagSpec { names: &["delete"], arity: 1 },
    FlagSpec { names: &["add"], arity: 1 },
    FlagSpec { names: &["remove"], arity: 1 },
    FlagSpec { names: &["renderable"], arity: 1 },
    FlagSpec { names: &["passes"], arity: 1 },
    FlagSpec { names: &["list", "l"], arity: 0 },
    FlagSpec { names: &["clearOverride"], arity: 1 },
    FlagSpec { names: &["query", "q"], arity: 1 },
];

/// `renderLayer -create L | -delete L | -add L obj... | -remove L obj... | -renderable L 0|1 | -passes L "diffuse specular" | -l | -q L`.
/// Layers render only what they contain; add the lights too or the default lights are used. Overrides: `renderSettings -layer L ...`.
pub fn render_layer(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, LAYER_SPEC)?;
    let args = p.arg_strings();
    if let Some(n) = p.string("create") {
        if n == "masterLayer" || s.scene.render_layers.iter().any(|l| l.name == n) {
            return Err(format!("A render layer named {} exists", n));
        }
        s.scene.render_layers.push(RenderLayerData { name: n.clone(), renderable: true, ..Default::default() });
        return Ok(vec![n]);
    }
    if let Some(n) = p.string("delete") {
        let before = s.scene.render_layers.len();
        s.scene.render_layers.retain(|l| l.name != n);
        if s.scene.render_layers.len() == before {
            return Err(format!("No render layer named {}", n));
        }
        return Ok(vec![]);
    }
    for (flag, adding) in [("add", true), ("remove", false)] {
        if let Some(n) = p.string(flag) {
            let ids = if args.is_empty() { s.scene.selection.clone() } else { resolve_targets(s, &args)? };
            let layer = s.scene.render_layers.iter_mut().find(|l| l.name == n).ok_or_else(|| format!("No render layer named {}", n))?;
            for id in &ids {
                if adding {
                    if !layer.members.contains(id) {
                        layer.members.push(*id);
                    }
                } else {
                    layer.members.retain(|m| m != id);
                }
            }
            return Ok(vec![format!("{} now holds {} object(s)", n, layer.members.len())]);
        }
    }
    if let Some(n) = p.string("renderable") {
        let on = args.first().map(|a| a != "0").unwrap_or(true);
        s.scene.render_layers.iter_mut().find(|l| l.name == n).ok_or_else(|| format!("No render layer named {}", n))?.renderable = on;
        return Ok(vec![]);
    }
    if let Some(n) = p.string("passes") {
        let list: Vec<String> = args.iter().flat_map(|a| a.split_whitespace().map(|x| x.to_string()).collect::<Vec<_>>()).collect();
        for pass in &list {
            if !pipeline::PASS_NAMES.contains(&pass.as_str()) || pass == "beauty" {
                return Err(format!("Unknown render pass '{}': {}", pass, pipeline::PASS_NAMES[1..].join(", ")));
            }
        }
        s.scene.render_layers.iter_mut().find(|l| l.name == n).ok_or_else(|| format!("No render layer named {}", n))?.passes = list;
        return Ok(vec![]);
    }
    if let Some(n) = p.string("clearOverride") {
        let attr = args.first().cloned().ok_or("Usage: renderLayer -clearOverride LAYER ATTR")?;
        let l = s.scene.render_layers.iter_mut().find(|l| l.name == n).ok_or_else(|| format!("No render layer named {}", n))?;
        l.overrides.retain(|(k, _)| *k != attr);
        l.text_overrides.retain(|(k, _)| *k != attr);
        return Ok(vec![]);
    }
    if let Some(n) = p.string("query") {
        let l = find_layer(s, &n)?;
        let names: Vec<String> = l.members.iter().filter(|m| s.scene.node(**m).is_some()).map(|m| s.scene.name_of(*m)).collect();
        return Ok(vec![format!("{}: {}; passes: {}; overrides: {}; objects: {}", l.name, if l.renderable { "renderable" } else { "not renderable" }, l.passes.join(" "), l.overrides.iter().map(|(k, v)| format!("{}={}", k, v)).chain(l.text_overrides.iter().map(|(k, v)| format!("{}={}", k, v))).collect::<Vec<_>>().join(" "), names.join(" "))]);
    }
    Ok(std::iter::once("masterLayer".to_string()).chain(s.scene.render_layers.iter().map(|l| l.name.clone())).collect())
}

const LINK_SPEC: &[FlagSpec] = &[FlagSpec { names: &["break"], arity: 0 }, FlagSpec { names: &["query", "q"], arity: 0 }];

/// `lightLink LIGHT obj ...` makes the light illuminate only those objects; `lightLink -break LIGHT obj ...` removes objects
/// from it (and the first break on an unlinked light links every other object); `lightLink -q LIGHT`.
pub fn light_link(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, LINK_SPEC)?;
    let args = p.arg_strings();
    let lname = args.first().cloned().ok_or("Usage: lightLink [-break] LIGHT obj ...")?;
    let ids = resolve_targets(s, &[lname.clone()])?;
    let light = ids.iter().flat_map(|i| std::iter::once(*i).chain(s.scene.node(*i).map(|n| n.children.clone()).unwrap_or_default())).find(|i| matches!(s.scene.node(*i).map(|n| &n.data), Some(NodeData::Light))).ok_or_else(|| format!("'{}' is not a light", lname))?;
    if p.has("query") {
        let l = s.scene.light_links.iter().find(|(l, _)| *l == light);
        return Ok(vec![match l {
            Some((_, o)) => o.iter().filter(|m| s.scene.node(**m).is_some()).map(|m| s.scene.name_of(*m)).collect::<Vec<_>>().join(" "),
            None => "illuminates everything".to_string(),
        }]);
    }
    if args.len() < 2 {
        return Err("Give the objects to link".to_string());
    }
    let objs = resolve_targets(s, &args[1..])?;
    if p.has("break") {
        if !s.scene.light_links.iter().any(|(l, _)| *l == light) {
            // from "everything" to "everything but these"
            let all: Vec<NodeId> = s.scene.ids_where(|d| matches!(d, NodeData::Mesh(_))).into_iter().map(|m| s.scene.node(m).and_then(|n| n.parent).unwrap_or(m)).collect();
            s.scene.light_links.push((light, all));
        }
        let entry = s.scene.light_links.iter_mut().find(|(l, _)| *l == light).unwrap();
        let shapes: Vec<NodeId> = objs.clone();
        entry.1.retain(|m| !shapes.contains(m));
    } else {
        match s.scene.light_links.iter_mut().find(|(l, _)| *l == light) {
            Some(e) => {
                for o in &objs {
                    if !e.1.contains(o) {
                        e.1.push(*o);
                    }
                }
            }
            None => s.scene.light_links.push((light, objs)),
        }
    }
    Ok(vec![])
}

const STATS_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["cast"], arity: 1 },
    FlagSpec { names: &["receive"], arity: 1 },
    FlagSpec { names: &["blur"], arity: 1 },
    FlagSpec { names: &["primary"], arity: 1 },
    FlagSpec { names: &["reflect"], arity: 1 },
];

/// `renderStats [-cast 0|1] [-receive 0|1] [-blur 0|1] [-primary 0|1] [-reflect 0|1] obj ...`: the Render Stats of shapes
/// (Casts Shadows, Receive Shadows, Motion Blur, Primary Visibility, Visible In Reflections).
pub fn render_stats(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, STATS_SPEC)?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let shapes: Vec<NodeId> = ids.iter().flat_map(|i| s.scene.mesh_shapes_under(*i)).collect();
    if shapes.is_empty() {
        return Err("Select polygon objects".to_string());
    }
    for (flag, plug) in [("cast", "castsShadows"), ("receive", "receiveShadows"), ("blur", "motionBlur"), ("primary", "primaryVisibility"), ("reflect", "visibleInReflections")] {
        if let Some(v) = p.num(flag)? {
            for sh in &shapes {
                s.scene.set_attr(*sh, plug, if v > 0.5 { 1.0 } else { 0.0 })?;
            }
        }
    }
    Ok(shapes.iter().map(|sh| {
        let g = |k: &str| s.scene.node(*sh).and_then(|n| n.plug(k)).map(|p| p.value as u8).unwrap_or(1);
        format!("{}: casts shadows {}, receives shadows {}, motion blur {}, primary visibility {}, visible in reflections {}", s.scene.name_of(*sh), g("castsShadows"), g("receiveShadows"), g("motionBlur"), g("primaryVisibility"), g("visibleInReflections"))
    }).collect())
}

const FCHECK_SPEC: &[FlagSpec] = &[FlagSpec { names: &["contact"], arity: 1 }, FlagSpec { names: &["columns"], arity: 1 }];

/// `fcheck [-contact SHEET.png] [-columns N] FRAME_OF_SEQUENCE`: lists an image sequence (like FCheck's Open Sequence) and can
/// write a contact sheet of its frames.
pub fn fcheck(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, FCHECK_SPEC)?;
    let arg = p.arg_strings().first().cloned().ok_or("Usage: fcheck [-contact SHEET.png] path/name.0001.png")?;
    let path = if std::path::Path::new(&arg).is_absolute() { arg.clone() } else { match &s.scene.project_dir {
        Some(d) if !std::path::Path::new(&arg).exists() => d.join("images").join(&arg).to_string_lossy().to_string(),
        _ => arg.clone(),
    } };
    let seq = pipeline::find_sequence(&path);
    if seq.is_empty() {
        return Err(format!("No image sequence found for {}", arg));
    }
    let first = image::open(&seq[0].1).map_err(|e| format!("Cannot read {}: {}", seq[0].1, e))?;
    let mut out = vec![format!("{} frame(s) {}..{}, {}x{}", seq.len(), seq[0].0, seq[seq.len() - 1].0, first.width(), first.height())];
    if let Some(sheet) = p.string("contact") {
        let cols = p.num("columns")?.unwrap_or(4.0).clamp(1.0, 16.0) as usize;
        let (tw, th) = (160u32, (160.0 * first.height() as f32 / first.width().max(1) as f32).max(1.0) as u32);
        let rows = seq.len().div_ceil(cols);
        let mut img = image::RgbaImage::new(tw * cols as u32, th * rows as u32);
        for (i, (_, f)) in seq.iter().enumerate() {
            if let Ok(fr) = image::open(f) {
                let small = fr.resize_exact(tw, th, image::imageops::FilterType::Triangle).to_rgba8();
                image::imageops::overlay(&mut img, &small, ((i % cols) as u32 * tw) as i64, ((i / cols) as u32 * th) as i64);
            }
        }
        img.save(&sheet).map_err(|e| format!("Cannot write {}: {}", sheet, e))?;
        out.push(format!("Contact sheet written to {}", sheet));
    }
    Ok(out)
}
