//! Non-linear animation and related animation tools: Trax clips, blends, merges, time warps, redirects, animation layer
//! helpers, cameras with aim, the normal constraint, live surfaces and path markers.

use super::commands::{resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::anim::trax::{self, BLEND_SHAPES};
use crate::anim::Interp;
use crate::dag::nodes::{ConstraintKind, NodeData};
use crate::dag::NodeId;
use crate::math::Vec3;
use crate::session::Session;

fn range(v: Option<String>) -> Result<Option<(f32, f32)>, String> {
    match v {
        None => Ok(None),
        Some(r) => {
            let (a, b) = r.split_once(':').ok_or("Time range must look like \"1:48\"")?;
            let (a, b) = (a.trim().parse::<f32>().map_err(|_| "Bad start frame")?, b.trim().parse::<f32>().map_err(|_| "Bad end frame")?);
            Ok(Some(if a <= b { (a, b) } else { (b, a) }))
        }
    }
}

fn character(s: &Session, name: Option<String>) -> Result<NodeId, String> {
    match name {
        Some(n) => {
            let id = s.scene.find(&n).ok_or_else(|| format!("No character set named {}", n))?;
            if matches!(s.scene.node(id).map(|x| &x.data), Some(NodeData::Character(_))) {
                Ok(id)
            } else {
                Err(format!("'{}' is not a character set", n))
            }
        }
        None => s
            .scene
            .ids_where(|d| matches!(d, NodeData::Character(_)))
            .into_iter()
            .find(|c| s.current_character == Some(*c))
            .or_else(|| s.scene.ids_where(|d| matches!(d, NodeData::Character(_))).first().copied())
            .ok_or_else(|| "There is no character set: create one with the character command".to_string()),
    }
}

const CLIP_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["create", "c"], arity: 1 },
    FlagSpec { names: &["edit", "e"], arity: 1 },
    FlagSpec { names: &["delete"], arity: 1 },
    FlagSpec { names: &["list", "l"], arity: 0 },
    FlagSpec { names: &["query", "q"], arity: 1 },
    FlagSpec { names: &["activate"], arity: 1 },
    FlagSpec { names: &["keep"], arity: 0 },
    FlagSpec { names: &["character", "ch"], arity: 1 },
    FlagSpec { names: &["time", "t"], arity: 1 },
    FlagSpec { names: &["leaveKeys"], arity: 0 },
    FlagSpec { names: &["timeWarp"], arity: 0 },
    FlagSpec { names: &["start"], arity: 1 },
    FlagSpec { names: &["scale"], arity: 1 },
    FlagSpec { names: &["cycles"], arity: 1 },
    FlagSpec { names: &["cycleOffset"], arity: 1 },
    FlagSpec { names: &["relative"], arity: 1 },
    FlagSpec { names: &["weight"], arity: 1 },
    FlagSpec { names: &["enable"], arity: 1 },
    FlagSpec { names: &["track"], arity: 1 },
    FlagSpec { names: &["rename"], arity: 1 },
];

fn clip_line(s: &Session, c: &trax::Clip) -> String {
    format!(
        "{}: source {}, character {}, track {}, frames {}..{}, scale {}, cycles {}, {}, weight {}{}{}",
        c.name,
        c.source,
        c.character,
        c.track,
        c.start,
        c.end(),
        c.scale,
        c.cycles,
        if c.relative { "relative" } else { "absolute" },
        c.weight,
        if c.enabled { "" } else { ", disabled" },
        if c.warp.is_some() { ", time warp" } else { "" }
    ) + &{
        let _ = s;
        String::new()
    }
}

/// `clip -create NAME [-character C] [-t "a:b"] [-leaveKeys] [-timeWarp]`, `clip -edit NAME [-start F] [-scale S] [-cycles N] [-relative 0|1]
/// [-weight W] [-enable 0|1] [-track N] [-cycleOffset 0|1] [-rename N]`, `clip -delete NAME`, `clip -activate NAME [-keep]`, `clip -list`,
/// `clip -q NAME`: Trax Editor > Create Animation Clip and the clip's attributes.
pub fn clip(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, CLIP_SPEC)?;
    if let Some(name) = p.string("create") {
        let ch = character(s, p.string("character"))?;
        let r = range(p.string("time"))?;
        let n = s.scene.create_clip(&name, ch, r, p.has("leaveKeys"), p.has("timeWarp"))?;
        return Ok(vec![n]);
    }
    if let Some(name) = p.string("delete") {
        s.scene.delete_clip(&name)?;
        return Ok(vec![]);
    }
    if let Some(name) = p.string("activate") {
        let n = s.scene.activate_clip_keys(&name, !p.has("keep"))?;
        return Ok(vec![format!("{} key(s) put in the timeline", n)]);
    }
    if let Some(name) = p.string("query") {
        let c = s.scene.trax.clip(&name).ok_or_else(|| format!("No clip named {}", name))?;
        return Ok(vec![clip_line(s, c)]);
    }
    if let Some(name) = p.string("edit") {
        let mut new_name = None;
        {
            let c = s.scene.trax.clip_mut(&name).ok_or_else(|| format!("No clip named {}", name))?;
            if let Some(v) = p.num("start")? {
                c.start = v;
            }
            if let Some(v) = p.num("scale")? {
                c.scale = v.max(0.01);
            }
            if let Some(v) = p.num("cycles")? {
                c.cycles = v.max(1.0);
            }
            if let Some(v) = p.num("cycleOffset")? {
                c.cycle_offset = v > 0.5;
            }
            if let Some(v) = p.num("relative")? {
                c.relative = v > 0.5;
            }
            if let Some(v) = p.num("weight")? {
                c.weight = v.clamp(0.0, 1.0);
            }
            if let Some(v) = p.num("enable")? {
                c.enabled = v > 0.5;
            }
            if let Some(v) = p.num("track")? {
                c.track = v.max(0.0) as usize;
            }
            if let Some(n) = p.string("rename") {
                new_name = Some(n);
            }
        }
        if let Some(n) = new_name {
            if s.scene.trax.clip(&n).is_some() {
                return Err(format!("A clip named {} exists", n));
            }
            s.scene.trax.clip_mut(&name).unwrap().name = n.clone();
            for b in s.scene.trax.blends.iter_mut() {
                if b.a == name {
                    b.a = n.clone();
                }
                if b.b == name {
                    b.b = n.clone();
                }
            }
        }
        s.scene.evaluate_frame();
        return Ok(vec![]);
    }
    let ch = p.string("character");
    Ok(s.scene.trax.clips.iter().filter(|c| ch.as_ref().map(|x| *x == c.character).unwrap_or(true)).map(|c| clip_line(s, c)).collect())
}

/// `trimClip -before F | -after F CLIP`: Trim Before / Trim After at a scene frame.
pub fn trim_clip(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["before"], arity: 1 }, FlagSpec { names: &["after"], arity: 1 }])?;
    let name = p.arg_strings().first().cloned().ok_or("Usage: trimClip -before F | -after F CLIP")?;
    match (p.num("before")?, p.num("after")?) {
        (Some(f), _) => s.scene.trim_clip(&name, f, true)?,
        (None, Some(f)) => s.scene.trim_clip(&name, f, false)?,
        _ => return Err("Give -before FRAME or -after FRAME".to_string()),
    }
    s.scene.evaluate_frame();
    Ok(vec![])
}

/// `splitClip -at F CLIP`: Modify > Split at the frame; returns the second clip.
pub fn split_clip(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["at"], arity: 1 }])?;
    let name = p.arg_strings().first().cloned().ok_or("Usage: splitClip -at F CLIP")?;
    let f = p.num("at")?.unwrap_or(s.scene.timeline.current);
    let n = s.scene.split_clip(&name, f)?;
    s.scene.evaluate_frame();
    Ok(vec![n])
}

/// `blendClips [-shape linear|easeIn|easeOut|easeInOut] [-delete] CLIP1 CLIP2`: Create > Blend; the first clip fades into the second across their overlap.
pub fn blend_clips(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["shape"], arity: 1 }, FlagSpec { names: &["delete"], arity: 0 }])?;
    let a = p.arg_strings();
    if a.len() != 2 {
        return Err("Usage: blendClips [-shape easeInOut] CLIP1 CLIP2".to_string());
    }
    if p.has("delete") {
        s.scene.trax.blends.retain(|b| !(b.a == a[0] && b.b == a[1]));
        s.scene.evaluate_frame();
        return Ok(vec![]);
    }
    let shape = match p.string("shape") {
        Some(sh) => BLEND_SHAPES.iter().position(|x| x.eq_ignore_ascii_case(&sh)).ok_or_else(|| format!("Unknown blend shape '{}': {}", sh, BLEND_SHAPES.join(", ")))? as u8,
        None => 0,
    };
    s.scene.blend_clips(&a[0], &a[1], shape)?;
    s.scene.evaluate_frame();
    Ok(vec![])
}

/// `mergeClips -name NAME CLIP CLIP ...`: Edit > Merge: one clip with a key on every frame replaces them.
pub fn merge_clips(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["name", "n"], arity: 1 }])?;
    let names = p.arg_strings();
    let n = s.scene.merge_clips(&names, &p.string("name").unwrap_or_else(|| "mergedClip".to_string()))?;
    s.scene.evaluate_frame();
    Ok(vec![n])
}

const WARP_SPEC: &[FlagSpec] = &[FlagSpec { names: &["create"], arity: 0 }, FlagSpec { names: &["remove"], arity: 0 }, FlagSpec { names: &["key"], arity: 1 }, FlagSpec { names: &["flatEnd"], arity: 0 }, FlagSpec { names: &["query", "q"], arity: 0 }];

/// `clipTimeWarp CLIP -create | -key "CLIPFRAME SOURCEFRAME" [-flatEnd] | -remove | -q`: a curve that controls the speed of the clip.
pub fn clip_time_warp(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, WARP_SPEC)?;
    let name = p.arg_strings().first().cloned().ok_or("Usage: clipTimeWarp CLIP -create | -key \"local source\"")?;
    if p.has("create") {
        s.scene.create_time_warp(&name)?;
    }
    if let Some(k) = p.string("key") {
        let v: Vec<f32> = k.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if v.len() != 2 {
            return Err("-key needs \"frames-from-clip-start source-frame\"".to_string());
        }
        s.scene.key_time_warp(&name, v[0], v[1], p.has("flatEnd"))?;
    }
    if p.has("remove") {
        s.scene.trax.clip_mut(&name).ok_or_else(|| format!("No clip named {}", name))?.warp = None;
    }
    s.scene.evaluate_frame();
    if p.has("query") {
        let c = s.scene.trax.clip(&name).ok_or_else(|| format!("No clip named {}", name))?;
        return Ok(vec![match &c.warp {
            Some(w) => w.keys.iter().map(|k| format!("{} {}", k.t, k.v)).collect::<Vec<_>>().join(", "),
            None => "no time warp".to_string(),
        }]);
    }
    Ok(vec![])
}

fn clip_path(s: &Session, file: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(file);
    if p.is_absolute() || p.components().count() > 1 {
        return p.to_path_buf();
    }
    match &s.scene.project_dir {
        Some(d) => d.join("clips").join(file),
        None => p.to_path_buf(),
    }
}

/// `clipExport SOURCE FILE`: export a clip source (Visor > Character Clips > Export); a bare name goes to the project's clips folder.
pub fn clip_export(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let a = p.arg_strings();
    if a.len() != 2 {
        return Err("Usage: clipExport SOURCE FILE".to_string());
    }
    let src = s.scene.trax.source(&a[0]).ok_or_else(|| format!("No clip source named {}", a[0]))?;
    let mut path = clip_path(s, &a[1]);
    if path.extension().is_none() {
        path.set_extension("aztclip");
    }
    if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("Cannot create {}: {}", d.display(), e))?;
    }
    std::fs::write(&path, trax::source_to_file(src)).map_err(|e| format!("Cannot write {}: {}", path.display(), e))?;
    Ok(vec![path.display().to_string()])
}

/// `clipImport FILE [-insert] [-start F] [-character C]`: Import Animation Clip into the Visor (and optionally the Trax Editor).
pub fn clip_import(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["insert"], arity: 0 }, FlagSpec { names: &["start"], arity: 1 }, FlagSpec { names: &["character"], arity: 1 }])?;
    let file = p.arg_strings().first().cloned().ok_or("Usage: clipImport FILE")?;
    let mut path = clip_path(s, &file);
    if !path.exists() && path.extension().is_none() {
        path.set_extension("aztclip");
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    let mut src = trax::source_from_file(&text)?;
    src.name = s.scene.trax.unique(&src.name);
    let name = src.name.clone();
    s.scene.trax.sources.push(src);
    let mut out = vec![name.clone()];
    if p.has("insert") {
        let c = s.scene.insert_clip(&name, p.num("start")?.unwrap_or(s.scene.timeline.current), p.string("character").as_deref())?;
        out.push(c);
    }
    s.scene.evaluate_frame();
    Ok(out)
}

/// `clipInsert SOURCE [-start F] [-character C]`: Library > Insert Clip.
pub fn clip_insert(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["start"], arity: 1 }, FlagSpec { names: &["character"], arity: 1 }])?;
    let name = p.arg_strings().first().cloned().ok_or("Usage: clipInsert SOURCE [-start F]")?;
    let c = s.scene.insert_clip(&name, p.num("start")?.unwrap_or(s.scene.timeline.current), p.string("character").as_deref())?;
    s.scene.evaluate_frame();
    Ok(vec![c])
}

/// `clipLibrary`: the clip sources of the scene (the Visor's Character Clips tab).
pub fn clip_library(s: &mut Session, _rest: &[Token]) -> Out {
    Ok(s.scene.trax.sources.iter().map(|c| format!("{}: {} channel(s), {} frames, character {}", c.name, c.channels.len(), c.duration, c.character)).collect())
}

const REDIR_SPEC: &[FlagSpec] = &[FlagSpec { names: &["translation", "t"], arity: 0 }, FlagSpec { names: &["rotation", "r"], arity: 0 }, FlagSpec { names: &["remove"], arity: 0 }];

/// `redirect [-translation] [-rotation] [-remove] node`: Character > Redirect: a control node that offsets the whole animation of the node.
pub fn redirect(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, REDIR_SPEC)?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let node = ids[0];
    if p.has("remove") {
        s.scene.redirects.retain(|r| r.node != node);
        s.scene.evaluate_frame();
        return Ok(vec![]);
    }
    let (t, r) = (p.has("translation"), p.has("rotation"));
    let (t, r) = if !t && !r { (true, true) } else { (t, r) };
    let ctrl = s.scene.create_redirect(node, t, r)?;
    s.scene.selection = vec![ctrl];
    Ok(vec![s.scene.name_of(ctrl)])
}

// ---------------------------------------------------------------------------------------------- cameras

const CAMERA_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["aim"], arity: 0 },
    FlagSpec { names: &["up"], arity: 0 },
    FlagSpec { names: &["fov", "fieldOfView"], arity: 1 },
    FlagSpec { names: &["filmFit"], arity: 1 },
    FlagSpec { names: &["distance"], arity: 1 },
];

/// `cameraAim [-n NAME] [-up] [-fov DEG] [-filmFit fill|horizontal|vertical|overscan] [-distance D]`: Create > Cameras > Camera and Aim
/// (and Camera, Aim and Up with `-up`). The camera looks at its aim node; both can be keyed.
pub fn camera_aim(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, CAMERA_SPEC)?;
    let (t, shape) = s.scene.create_camera(false, p.string("name").as_deref());
    let cam_name = s.scene.name_of(t);
    for (a, v) in [("translateY", 5.0), ("translateZ", 15.0)] {
        s.scene.set_attr(t, a, v)?;
    }
    if let Some(f) = p.num("fov")? {
        s.scene.set_attr(shape, "fieldOfView", f.clamp(1.0, 170.0))?;
    }
    if let Some(fit) = p.string("filmFit") {
        let v = ["fill", "horizontal", "vertical", "overscan"].iter().position(|x| x.eq_ignore_ascii_case(&fit)).ok_or_else(|| format!("Unknown film fit '{}'", fit))?;
        s.scene.set_attr(shape, "filmFit", v as f32)?;
    }
    let dist = p.num("distance")?.unwrap_or(15.0);
    let mk = |s: &mut Session, name: String, at: Vec3| -> Result<NodeId, String> {
        let l = s.scene.create_node(NodeData::Transform, &name, None, false);
        s.scene.create_node(NodeData::Locator, &format!("{}Shape", s.scene.name_of(l)), Some(l), false);
        for (a, v) in ["translateX", "translateY", "translateZ"].iter().zip([at.x, at.y, at.z]) {
            s.scene.set_attr(l, a, v)?;
        }
        Ok(l)
    };
    let aim = mk(s, format!("{}_aim", cam_name), Vec3::new(0.0, 0.0, 15.0 - dist))?;
    let mut targets = vec![aim];
    let mut up = None;
    if p.has("up") {
        let u = mk(s, format!("{}_up", cam_name), Vec3::new(0.0, 5.0 + 10.0, 15.0))?;
        targets.push(u);
        up = Some(u);
    }
    // the camera looks down -Z with +Y up
    let c = s.scene.create_constraint(ConstraintKind::Aim, &targets, t, false)?;
    for (a, v) in [("aimX", 0.0), ("aimY", 0.0), ("aimZ", -1.0), ("upX", 0.0), ("upY", 1.0), ("upZ", 0.0)] {
        s.scene.set_attr(c, a, v)?;
    }
    s.scene.set_attr(c, "worldUpType", if up.is_some() { 1.0 } else { 0.0 })?;
    s.scene.evaluate_frame();
    s.scene.selection = vec![t, aim];
    let mut out = vec![cam_name, s.scene.name_of(aim)];
    if let Some(u) = up {
        out.push(s.scene.name_of(u));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------- constraints and live surfaces

const NORMAL_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["aimVector", "aim"], arity: 3 },
    FlagSpec { names: &["upVector", "u"], arity: 3 },
    FlagSpec { names: &["worldUpType", "wut"], arity: 1 },
    FlagSpec { names: &["worldUpObject", "wuo"], arity: 1 },
    FlagSpec { names: &["worldUpVector", "wu"], arity: 3 },
];

/// `normalConstraint [-aim x y z] [-u x y z] [-wut scene|object|vector|none] [-wuo OBJECT] [-wu x y z] surface driven`: Constrain > Normal.
/// The aim vector of the object follows the normal of the surface at its closest point.
pub fn normal_constraint(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, NORMAL_SPEC)?;
    let names = p.arg_strings();
    let ids: Vec<NodeId> = if names.is_empty() { s.scene.selection.clone() } else { resolve_targets(s, &names)? };
    if ids.len() < 2 {
        return Err("Usage: normalConstraint [-aim x y z] [-u x y z] [-wut object -wuo OBJ] surface driven   (select the surface, then the object)".to_string());
    }
    let (driven, surface) = (*ids.last().unwrap(), ids[0]);
    if s.scene.mesh_shapes_under(surface).is_empty() {
        return Err("The first object must be a polygon surface".to_string());
    }
    let wut = match p.string("worldUpType").unwrap_or_else(|| "scene".to_string()).as_str() {
        "scene" => 0.0,
        "object" => 1.0,
        "vector" => 2.0,
        "none" => 3.0,
        o => return Err(format!("Unknown world up type '{}': scene, object, vector or none", o)),
    };
    let mut targets = vec![surface];
    if wut == 1.0 {
        let o = p.string("worldUpObject").ok_or("Object up needs -wuo OBJECT")?;
        targets.push(resolve_targets(s, &[o])?[0]);
    }
    let c = s.scene.create_constraint(ConstraintKind::Normal, &targets, driven, false)?;
    s.scene.set_attr(c, "worldUpType", wut)?;
    for (flag, base) in [("aimVector", "aim"), ("upVector", "up"), ("worldUpVector", "worldUp")] {
        if let Some(v) = p.get(flag) {
            for (i, a) in ["X", "Y", "Z"].iter().enumerate() {
                s.scene.set_attr(c, &format!("{}{}", base, a), v[i].as_f32()?)?;
            }
        }
    }
    s.scene.evaluate_frame();
    Ok(vec![s.scene.name_of(c)])
}

/// `makeLive [-off] [surface]`: Modify > Make Live / Make Not Live. Curves made with `curve -live` snap to the live surface.
pub fn make_live(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["off"], arity: 0 }, FlagSpec { names: &["query", "q"], arity: 0 }])?;
    if p.has("query") {
        return Ok(vec![s.scene.live.map(|l| s.scene.name_of(l)).unwrap_or_default()]);
    }
    if p.has("off") {
        s.scene.live = None;
        return Ok(vec![]);
    }
    let ids = resolve_targets(s, &p.arg_strings())?;
    let t = ids.into_iter().find(|i| !s.scene.mesh_shapes_under(*i).is_empty()).ok_or("Select a polygon surface to make live")?;
    s.scene.live = Some(t);
    Ok(vec![s.scene.name_of(t)])
}

// ---------------------------------------------------------------------------------------------- path markers

fn path_node(s: &Session, name: &str) -> Result<NodeId, String> {
    let ids = resolve_targets(s, &[name.to_string()])?;
    let id = ids[0];
    let direct = matches!(s.scene.node(id).map(|n| &n.data), Some(NodeData::MotionPath(_)));
    if direct {
        return Ok(id);
    }
    s.scene.node(id).and_then(|n| n.children.iter().copied().find(|c| matches!(s.scene.node(*c).map(|x| &x.data), Some(NodeData::MotionPath(_))))).ok_or_else(|| format!("{} has no motion path", name))
}

const MARKER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["frame", "f"], arity: 1 },
    FlagSpec { names: &["u"], arity: 1 },
    FlagSpec { names: &["moveTo"], arity: 1 },
    FlagSpec { names: &["remove"], arity: 0 },
    FlagSpec { names: &["query", "q"], arity: 0 },
];

/// `pathMarker -q OBJ` lists the markers (keys of the U value); `pathMarker -f FRAME -u U OBJ` sets a marker (a key); `-moveTo NEWFRAME`
/// moves the marker at -f; `-remove` deletes it.
pub fn path_marker(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, MARKER_SPEC)?;
    let name = p.arg_strings().first().cloned().ok_or("Usage: pathMarker [-q] [-f FRAME] [-u U] [-moveTo FRAME] [-remove] OBJECT")?;
    let mp = path_node(s, &name)?;
    if p.has("query") {
        let keys: Vec<String> = s.scene.anim.bindings.iter().filter(|b| b.node == mp && b.attr == "uValue" && b.layer == 0).flat_map(|b| b.curve.keys.iter().map(|k| format!("{} {}", k.t, k.v))).collect();
        return Ok(vec![keys.join(", ")]);
    }
    let f = p.num("frame")?.unwrap_or(s.scene.timeline.current);
    if p.has("remove") {
        if !s.scene.anim.remove_key(mp, "uValue", 0, f) {
            return Err(format!("There is no marker at frame {}", f));
        }
    } else if let Some(to) = p.num("moveTo")? {
        let v = s.scene.anim.bindings.iter().find(|b| b.node == mp && b.attr == "uValue" && b.layer == 0).and_then(|b| b.curve.keys.iter().find(|k| (k.t - f).abs() < 1e-3).map(|k| k.v)).ok_or_else(|| format!("There is no marker at frame {}", f))?;
        s.scene.anim.remove_key(mp, "uValue", 0, f);
        s.scene.anim.set_key(mp, "uValue", 0, to, v, Interp::Bezier);
    } else {
        let u = match p.num("u")? {
            Some(u) => u,
            None => s.scene.get_attr(mp, "uValue")?,
        };
        s.scene.anim.set_key(mp, "uValue", 0, f, u, Interp::Bezier);
    }
    s.scene.evaluate_frame();
    Ok(vec![])
}

const LAYER_X_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["addSelected"], arity: 1 },
    FlagSpec { names: &["zeroKey"], arity: 1 },
    FlagSpec { names: &["merge"], arity: 0 },
    FlagSpec { names: &["keyWeight"], arity: 1 },
    FlagSpec { names: &["members"], arity: 1 },
];

/// Extra `animLayer` work: `animLayer -addSelected LAYER [obj ...]` (Add Selected Objects), `-zeroKey LAYER [obj ...]` (Zero Key Layer),
/// `-merge LAYER ...` (Merge Layers), `-keyWeight LAYER` (the K button: key the weight at the current time), `-members LAYER`.
pub fn anim_layer_extra(s: &mut Session, rest: &[Token]) -> Option<Out> {
    let p = parse_args(rest, LAYER_X_SPEC).ok()?;
    let handled = p.has("merge") || p.string("addSelected").is_some() || p.string("zeroKey").is_some() || p.string("keyWeight").is_some() || p.string("members").is_some();
    if !handled {
        return None;
    }
    Some((|| -> Out {
        let args = p.arg_strings();
        let layer = |s: &Session, n: &str| s.scene.anim.find_layer(n).ok_or_else(|| format!("No animation layer named '{}'", n));
        let nodes = |s: &mut Session| -> Result<Vec<NodeId>, String> { if args.is_empty() { Ok(s.scene.selection.clone()) } else { resolve_targets(s, &args) } };
        if let Some(n) = p.string("addSelected") {
            let l = layer(s, &n)?;
            let ids = nodes(s)?;
            let added = s.scene.add_to_layer(l, &ids)?;
            return Ok(vec![format!("{} channel(s) added to {}", added, n)]);
        }
        if let Some(n) = p.string("zeroKey") {
            let l = layer(s, &n)?;
            let ids = nodes(s)?;
            let k = s.scene.zero_key_layer(l, &ids)?;
            return Ok(vec![format!("Zero keys set on {} channel(s)", k)]);
        }
        if let Some(n) = p.string("keyWeight") {
            let l = layer(s, &n)?;
            let t = s.scene.timeline.current;
            let w = s.scene.anim.layers[l].weight_at(t);
            s.scene.anim.layers[l].weight_curve.set_key(t, w, Interp::Bezier);
            s.scene.evaluate_frame();
            return Ok(vec![]);
        }
        if let Some(n) = p.string("members") {
            let l = layer(s, &n)?;
            return Ok(s.scene.anim.layers[l].members.iter().filter(|(i, _)| s.scene.node(*i).is_some()).map(|(i, a)| format!("{}.{}", s.scene.name_of(*i), a)).collect());
        }
        // merge: all names given
        let mut idx = Vec::new();
        for a in &args {
            idx.push(layer(s, a)?);
        }
        let n = s.scene.merge_anim_layers(&idx)?;
        s.scene.evaluate_frame();
        Ok(vec![format!("Merged {} channel(s) into the base animation", n)])
    })())
}
