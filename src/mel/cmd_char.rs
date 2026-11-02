//! Character sets, rename multiple, Distance Tool.

use super::commands::{fmt_f, resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::anim::Interp;
use crate::dag::nodes::{DistanceData, NodeData};
use crate::dag::NodeId;
use crate::session::Session;

const CHAR_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["name", "n"], arity: 1 },
    FlagSpec { names: &["excludeVisibility", "ev"], arity: 0 },
    FlagSpec { names: &["excludeTranslate", "et"], arity: 0 },
    FlagSpec { names: &["excludeRotate", "er"], arity: 0 },
    FlagSpec { names: &["excludeScale", "es"], arity: 0 },
    FlagSpec { names: &["query", "q"], arity: 0 },
    FlagSpec { names: &["add", "a"], arity: 1 },
    FlagSpec { names: &["remove", "r"], arity: 1 },
];

fn channel_list(s: &Session, c: NodeId) -> Vec<String> {
    match s.scene.node(c).map(|n| &n.data) {
        Some(NodeData::Character(d)) => d.channels.iter().filter(|(i, _)| s.scene.node(*i).is_some()).map(|(i, a)| format!("{}.{}", s.scene.name_of(*i), a)).collect(),
        _ => Vec::new(),
    }
}

/// `character [-n NAME] [-ev] [-et] [-er] [-es] object | node.attr ...`: Character > Create Character Set from the selection
/// (or the named objects and channels). `character -q NAME` lists the channels; `-add` / `-remove NAME` edit a set.
pub fn character(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, CHAR_SPEC)?;
    let args = p.arg_strings();
    if p.has("query") {
        if args.is_empty() {
            let all = s.scene.ids_where(|d| matches!(d, NodeData::Character(_)));
            return Ok(all.iter().map(|c| format!("{} ({} channels){}", s.scene.name_of(*c), channel_list(s, *c).len(), if s.current_character == Some(*c) { "  current" } else { "" })).collect());
        }
        let c = s.scene.find(&args[0]).filter(|c| matches!(s.scene.node(*c).map(|n| &n.data), Some(NodeData::Character(_)))).ok_or_else(|| format!("'{}' is not a character set", args[0]))?;
        return Ok(channel_list(s, c));
    }
    // channels from objects and explicit attributes
    let mut channels: Vec<(NodeId, String)> = Vec::new();
    let names: Vec<String> = if args.is_empty() { s.scene.selection.iter().map(|i| s.scene.name_of(*i)).collect() } else { args.clone() };
    for n in &names {
        if n.contains('.') {
            let (id, a) = s.scene.resolve_plug(n)?;
            if !channels.contains(&(id, a.clone())) {
                channels.push((id, a));
            }
        } else {
            let id = s.scene.find(n).ok_or_else(|| format!("No object matches name: {}", n))?;
            for (i, a) in s.scene.default_character_channels(id) {
                let skip = (p.has("excludeVisibility") && a == "visibility") || (p.has("excludeTranslate") && a.starts_with("translate")) || (p.has("excludeRotate") && a.starts_with("rotate")) || (p.has("excludeScale") && a.starts_with("scale"));
                if !skip && !channels.contains(&(i, a.clone())) {
                    channels.push((i, a));
                }
            }
        }
    }
    // editing an existing set
    if let Some(set) = p.string("add").or_else(|| p.string("remove")) {
        let c = s.scene.find(&set).filter(|c| matches!(s.scene.node(*c).map(|n| &n.data), Some(NodeData::Character(_)))).ok_or_else(|| format!("'{}' is not a character set", set))?;
        let adding = p.string("add").is_some();
        if let Some(NodeData::Character(d)) = s.scene.nodes.get_mut(c).map(|n| &mut n.data) {
            for ch in channels {
                if adding {
                    if !d.channels.contains(&ch) {
                        d.channels.push(ch);
                    }
                } else {
                    d.channels.retain(|x| *x != ch);
                }
            }
        }
        return Ok(vec![format!("{} now has {} channels", set, channel_list(s, c).len())]);
    }
    let name = p.string("name").unwrap_or_else(|| "character".to_string());
    let c = s.scene.create_character(&name, channels)?;
    s.current_character = Some(c);
    Ok(vec![s.scene.name_of(c)])
}

/// `setCurrentCharacter NAME | -none`: the character set Set Key keys.
pub fn set_current(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["none"], arity: 0 }])?;
    if p.has("none") {
        s.current_character = None;
        return Ok(vec!["No current character".to_string()]);
    }
    let name = p.arg_strings().first().cloned().ok_or("Usage: setCurrentCharacter NAME | -none")?;
    let c = s.scene.find(&name).filter(|c| matches!(s.scene.node(*c).map(|n| &n.data), Some(NodeData::Character(_)))).ok_or_else(|| format!("'{}' is not a character set", name))?;
    s.current_character = Some(c);
    Ok(vec![format!("The current character is {}", name)])
}

/// Key the current character at the current time (used by Set Key); returns the number of channels.
pub fn key_current(s: &mut Session, interp: Interp) -> Option<usize> {
    let c = s.current_character.filter(|c| s.scene.node(*c).is_some())?;
    let layer = s.scene.anim.active_layer;
    Some(s.scene.key_character(c, interp, layer))
}

/// `renameSelection BASE`: rename the selected objects BASE1, BASE2 ... in selection order (Rename multiple objects).
pub fn rename_selection(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let base = p.arg_strings().first().cloned().ok_or("Usage: renameSelection BASE")?;
    if base.is_empty() || base.chars().next().unwrap().is_ascii_digit() || !base.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("'{}' is not a valid name", base));
    }
    let sel = s.scene.selection.clone();
    if sel.is_empty() {
        return Err("Select the objects to rename".to_string());
    }
    // the numbers start above any existing BASEn
    let mut n = 1;
    let mut out = Vec::new();
    for id in sel {
        if s.scene.name_of(id) == "persp" {
            continue;
        }
        loop {
            let cand = format!("{}{}", base, n);
            n += 1;
            if s.scene.find(&cand).is_none() {
                out.push(s.scene.rename(id, &cand)?);
                break;
            }
        }
    }
    Ok(out)
}

const DIST_SPEC: &[FlagSpec] = &[FlagSpec { names: &["startPoint", "sp"], arity: 3 }, FlagSpec { names: &["endPoint", "ep"], arity: 3 }, FlagSpec { names: &["name", "n"], arity: 1 }];

/// `distanceDimension [-sp X Y Z] [-ep X Y Z] [-n NAME]`: Create > Measure Tools > Distance Tool. Two locators and a node whose
/// `distance` channel follows their separation (a locator can be moved, parented or constrained).
pub fn distance_dimension(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, DIST_SPEC)?;
    let pt = |k: &str, d: [f32; 3]| -> Result<[f32; 3], String> {
        Ok(match p.get(k) {
            Some(v) => [v[0].as_f32()?, v[1].as_f32()?, v[2].as_f32()?],
            None => d,
        })
    };
    let (a, b) = (pt("startPoint", [0.0, 0.0, 0.0])?, pt("endPoint", [1.0, 0.0, 0.0])?);
    let mk = |s: &mut Session, name: &str, v: [f32; 3]| -> Result<NodeId, String> {
        let l = s.scene.create_node(NodeData::Transform, name, None, true);
        s.scene.create_node(NodeData::Locator, &format!("{}Shape", s.scene.name_of(l)), Some(l), false);
        for (ax, x) in ["X", "Y", "Z"].iter().zip(v) {
            s.scene.set_attr(l, &format!("translate{}", ax), x)?;
        }
        Ok(l)
    };
    let (la, lb) = (mk(s, "locator", a)?, mk(s, "locator", b)?);
    let name = p.string("name").unwrap_or_else(|| "distanceDimension".to_string());
    let d = s.scene.create_node(NodeData::Distance(Box::new(DistanceData { start: Some(la), end: Some(lb), ..Default::default() })), &name, None, false);
    s.scene.evaluate_frame();
    let dist = s.scene.get_attr(d, "distance").unwrap_or(0.0);
    Ok(vec![s.scene.name_of(d), s.scene.name_of(la), s.scene.name_of(lb), format!("distance {}", fmt_f(dist))])
}

#[allow(dead_code)]
fn _unused(s: &Session) -> Result<Vec<NodeId>, String> {
    resolve_targets(s, &[])
}
