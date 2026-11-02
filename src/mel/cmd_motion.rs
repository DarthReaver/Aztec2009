//! Motion paths (Animate > Motion Paths) and sound.

use super::commands::{fmt_f, resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::anim::Interp;
use crate::dag::nodes::NodeData;
use crate::dag::NodeId;
use crate::session::Session;

const PATH_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["curve", "c"], arity: 1 },
    FlagSpec { names: &["follow", "f"], arity: 1 },
    FlagSpec { names: &["followAxis", "fa"], arity: 1 },
    FlagSpec { names: &["upAxis", "ua"], arity: 1 },
    FlagSpec { names: &["inverseFront", "if"], arity: 1 },
    FlagSpec { names: &["inverseUp", "iu"], arity: 1 },
    FlagSpec { names: &["bank", "b"], arity: 1 },
    FlagSpec { names: &["bankScale", "bs"], arity: 1 },
    FlagSpec { names: &["bankThreshold", "bt"], arity: 1 },
    FlagSpec { names: &["startTimeU", "stu"], arity: 1 },
    FlagSpec { names: &["endTimeU", "etu"], arity: 1 },
    FlagSpec { names: &["fractionMode", "fm"], arity: 1 },
    FlagSpec { names: &["uValue", "u"], arity: 1 },
    FlagSpec { names: &["name", "n"], arity: 1 },
];

fn axis_index(v: &str) -> Result<f32, String> {
    match v.to_lowercase().as_str() {
        "x" | "0" => Ok(0.0),
        "y" | "1" => Ok(1.0),
        "z" | "2" => Ok(2.0),
        o => Err(format!("Unknown axis '{}': x, y or z", o)),
    }
}

fn flag_on(p: &super::parser::Parsed, k: &str) -> Result<Option<f32>, String> {
    Ok(p.string(k).map(|v| if matches!(v.to_lowercase().as_str(), "false" | "0" | "off" | "no") { 0.0 } else { 1.0 }))
}

/// `pathAnimation [-curve C] [-follow 1] [-followAxis x] [-upAxis y] [-inverseFront 0] [-bank 1] [-startTimeU T]
/// [-endTimeU T] object [curve]`: Animate > Motion Paths > Attach to Motion Path. The object takes the position
/// along the curve from `uValue`, which is keyed from the first to the last parameter over the time range.
pub fn path_animation(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, PATH_SPEC)?;
    let names = p.arg_strings();
    let ids = resolve_targets(s, &names)?;
    let mut curve: Option<NodeId> = p.string("curve").and_then(|c| s.scene.find(&c)).and_then(|c| s.scene.curve_shapes_under(c).first().copied());
    if p.string("curve").is_some() && curve.is_none() {
        return Err(format!("No curve named '{}'", p.string("curve").unwrap()));
    }
    let mut target: Option<NodeId> = None;
    for id in &ids {
        let cs = s.scene.curve_shapes_under(*id);
        if let Some(c) = cs.first() {
            if curve.is_none() {
                curve = Some(*c);
                continue;
            }
        }
        if target.is_none() && s.scene.node(*id).map(|n| n.data.is_transform()).unwrap_or(false) && s.scene.curve_shapes_under(*id).is_empty() {
            target = Some(*id);
        }
    }
    let curve = curve.ok_or("Select the object, then the curve (or give -curve NAME)")?;
    let target = target.ok_or("Select the object to move along the curve")?;
    let mp = s.scene.create_motion_path(curve, target)?;
    if let Some(n) = p.string("name") {
        s.scene.rename(mp, &n)?;
    }
    let set = |s: &mut Session, a: &str, v: f32| s.scene.set_attr(mp, a, v);
    if let Some(v) = flag_on(&p, "follow")? {
        set(s, "follow", v)?;
    }
    if let Some(a) = p.string("followAxis") {
        set(s, "frontAxis", axis_index(&a)?)?;
    }
    if let Some(a) = p.string("upAxis") {
        set(s, "upAxis", axis_index(&a)?)?;
    }
    if let Some(v) = flag_on(&p, "inverseFront")? {
        set(s, "inverseFront", v)?;
    }
    if let Some(v) = flag_on(&p, "inverseUp")? {
        set(s, "inverseUp", v)?;
    }
    if let Some(v) = flag_on(&p, "bank")? {
        set(s, "bank", v)?;
    }
    if let Some(v) = p.num("bankScale")? {
        set(s, "bankScale", v)?;
    }
    if let Some(v) = p.num("bankThreshold")? {
        set(s, "bankLimit", v)?;
    }
    let fraction = flag_on(&p, "fractionMode")?.unwrap_or(0.0);
    set(s, "fractionMode", fraction)?;
    let (lo, hi) = match s.scene.node(curve).map(|n| &n.data) {
        Some(NodeData::NurbsCurve(cd)) => cd.curve.dir.domain(),
        _ => (0.0, 1.0),
    };
    let (u0, u1) = if fraction > 0.5 { (0.0, 1.0) } else { (lo, hi) };
    let layer = s.scene.anim.active_layer.min(s.scene.anim.layers.len() - 1);
    if let Some(u) = p.num("uValue")? {
        set(s, "uValue", u)?;
    } else {
        let t0 = p.num("startTimeU")?.unwrap_or(s.scene.timeline.start);
        let t1 = p.num("endTimeU")?.unwrap_or(s.scene.timeline.end);
        if t1 <= t0 {
            return Err("The end time must be after the start time".to_string());
        }
        s.scene.set_keyframe_noeval(mp, "uValue", Some(t0), Some(u0), Interp::Linear, layer)?;
        s.scene.set_keyframe_noeval(mp, "uValue", Some(t1), Some(u1), Interp::Linear, layer)?;
    }
    s.scene.evaluate_frame();
    Ok(vec![s.scene.name_of(mp)])
}

const SOUND_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["file", "f"], arity: 1 },
    FlagSpec { names: &["offset", "o"], arity: 1 },
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["query", "q"], arity: 0 },
];

/// `sound -file PATH [-offset FRAME] [-name N]`: Create > Sound / File > Import Audio. `sound -q [name]` lists them.
pub fn sound(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SOUND_SPEC)?;
    if p.has("query") || p.string("file").is_none() {
        let all = s.scene.ids_where(|d| matches!(d, NodeData::Sound(_)));
        if p.string("file").is_none() && !p.has("query") {
            return Err("Usage: sound -file song.wav [-offset FRAME] [-name N]   |   sound -q".to_string());
        }
        let fps = s.scene.timeline.fps;
        let mut out = Vec::new();
        for id in all {
            let off = s.scene.get_attr(id, "offset").unwrap_or(1.0);
            if let Some(NodeData::Sound(sd)) = s.scene.node(id).map(|n| &n.data) {
                out.push(format!("{}: {}  {} s ({} frames at {} fps), starts at frame {}, ends at {}", s.scene.name_of(id), sd.path, fmt_f(sd.seconds), fmt_f(sd.seconds * fps), fmt_f(fps), fmt_f(off), fmt_f(off + sd.seconds * fps)));
            }
        }
        return Ok(out);
    }
    let path = p.string("file").unwrap();
    let resolved = s.scene.texture_file(&path);
    let sd = crate::anim::sound::load_wav(&resolved)?;
    let name = p.string("name");
    let id = match &name {
        Some(n) => s.scene.create_node(NodeData::Sound(Box::new(sd)), n, None, false),
        None => s.scene.create_node(NodeData::Sound(Box::new(sd)), "audio", None, true),
    };
    if let Some(o) = p.num("offset")? {
        s.scene.set_attr(id, "offset", o)?;
    }
    // keep sounds out of the way of the selection
    let fps = s.scene.timeline.fps;
    let (secs, off) = match s.scene.node(id).map(|n| &n.data) {
        Some(NodeData::Sound(sd)) => (sd.seconds, s.scene.get_attr(id, "offset").unwrap_or(1.0)),
        _ => (0.0, 1.0),
    };
    Ok(vec![s.scene.name_of(id), format!("{} s, frames {} to {}", fmt_f(secs), fmt_f(off), fmt_f(off + secs * fps))])
}
