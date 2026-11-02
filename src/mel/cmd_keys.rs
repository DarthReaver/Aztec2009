//! Key clipboard: Copy / Cut / Paste of keyframes (time slider right-click menu, `copyKey`, `pasteKey`, `cutKeys`).

use super::commands::{fmt_f, resolve_targets, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::anim::Interp;
use crate::dag::NodeId;
use crate::session::Session;

#[derive(Clone, Debug)]
pub struct ClipKey {
    pub node: NodeId,
    pub attr: String,
    pub layer: usize,
    /// Time relative to the first copied key.
    pub rel: f32,
    pub value: f32,
    pub interp: Interp,
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// Copy the keys at `times` (every time when `None`) of the channels owned by `owners`
/// (optionally only the named attributes). Returns the number of keys copied.
pub fn copy_keys(s: &mut Session, owners: &[NodeId], times: Option<&[f32]>, attrs: Option<&[(NodeId, String)]>) -> Result<usize, String> {
    let mut clip: Vec<ClipKey> = Vec::new();
    for b in &s.scene.anim.bindings {
        if !owners.contains(&b.node) {
            continue;
        }
        if let Some(list) = attrs {
            if !list.iter().any(|(n, a)| *n == b.node && *a == b.attr) {
                continue;
            }
        }
        for k in &b.curve.keys {
            if times.map(|t| t.iter().any(|x| near(*x, k.t))).unwrap_or(true) {
                clip.push(ClipKey { node: b.node, attr: b.attr.clone(), layer: b.layer, rel: k.t, value: k.v, interp: k.interp });
            }
        }
    }
    if clip.is_empty() {
        return Err("There are no keys to copy".to_string());
    }
    let first = clip.iter().map(|k| k.rel).fold(f32::INFINITY, f32::min);
    for k in clip.iter_mut() {
        k.rel -= first;
    }
    let n = clip.len();
    s.key_clip = clip;
    Ok(n)
}

/// Delete the keys at `times` of the channels owned by `owners`.
pub fn delete_keys(s: &mut Session, owners: &[NodeId], times: &[f32]) -> usize {
    let mut n = 0;
    for b in s.scene.anim.bindings.iter_mut() {
        if !owners.contains(&b.node) {
            continue;
        }
        for t in times {
            if b.curve.remove_key(*t) {
                n += 1;
            }
        }
    }
    s.scene.anim.bindings.retain(|b| !b.curve.keys.is_empty());
    n
}

/// Paste the clipboard so its first key lands on `at`. Keys go back to the channels they were copied
/// from; when `targets` is given and does not contain the source node, they go to the channel of the
/// same name on each target instead (copy a bounce from one ball, paste it on another).
/// Returns the times of the pasted keys.
pub fn paste_keys(s: &mut Session, at: f32, targets: &[NodeId]) -> Result<Vec<f32>, String> {
    if s.key_clip.is_empty() {
        return Err("The key clipboard is empty: copy some keys first".to_string());
    }
    let clip = s.key_clip.clone();
    let mut pasted: Vec<f32> = Vec::new();
    let mut count = 0;
    for k in &clip {
        let mut dests: Vec<NodeId> = Vec::new();
        let alive = s.scene.node(k.node).is_some();
        if alive && (targets.is_empty() || targets.contains(&k.node)) {
            dests.push(k.node);
        } else {
            for t in targets {
                if s.scene.node(*t).and_then(|n| n.plug(&k.attr)).is_some() && !dests.contains(t) {
                    dests.push(*t);
                }
            }
        }
        for d in dests {
            let ok = s.scene.node(d).and_then(|n| n.plug(&k.attr)).map(|p| p.source.is_none() && !p.readonly).unwrap_or(false);
            if !ok {
                continue;
            }
            let layer = if k.layer < s.scene.anim.layers.len() { k.layer } else { 0 };
            let t = at + k.rel;
            s.scene.anim.set_key(d, &k.attr, layer, t, k.value, k.interp);
            count += 1;
            if !pasted.iter().any(|p| near(*p, t)) {
                pasted.push(t);
            }
        }
    }
    if count == 0 {
        return Err("The copied channels do not exist on the selected objects".to_string());
    }
    s.scene.evaluate_frame();
    pasted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Ok(pasted)
}

fn parse_times(v: Option<String>) -> Result<Option<Vec<f32>>, String> {
    match v {
        None => Ok(None),
        Some(r) => {
            let bad = |_| format!("Bad time '{}': use \"12\" or \"1:24\"", r);
            Ok(Some(match r.split_once(':') {
                Some((a, b)) => {
                    let (a, b) = (a.trim().parse::<f32>().map_err(bad)?, b.trim().parse::<f32>().map_err(bad)?);
                    // every whole frame in the range; non-integer keys are matched by the range below
                    let mut v = Vec::new();
                    let mut t = a.min(b);
                    while t <= a.max(b) + 1e-3 && v.len() < 100_000 {
                        v.push(t);
                        t += 1.0;
                    }
                    v
                }
                None => vec![r.trim().parse::<f32>().map_err(bad)?],
            }))
        }
    }
}

pub(crate) fn owners_and_attrs(s: &mut Session, names: &[String]) -> Result<(Vec<NodeId>, Option<Vec<(NodeId, String)>>), String> {
    let mut attrs: Vec<(NodeId, String)> = Vec::new();
    let mut nodes: Vec<String> = Vec::new();
    for n in names {
        if n.contains('.') {
            attrs.push(s.scene.resolve_plug(n)?);
        } else {
            nodes.push(n.clone());
        }
    }
    let mut owners: Vec<NodeId> = Vec::new();
    if nodes.is_empty() && attrs.is_empty() {
        owners = s.scene.key_owner_ids(&s.scene.selection.clone());
    } else {
        let ids = resolve_targets(s, &nodes)?;
        if !nodes.is_empty() {
            owners = s.scene.key_owner_ids(&ids);
        }
        owners.extend(attrs.iter().map(|(n, _)| *n));
    }
    Ok((owners, if attrs.is_empty() { None } else { Some(attrs) }))
}

const COPY_SPEC: &[FlagSpec] = &[FlagSpec { names: &["time", "t"], arity: 1 }];

/// `copyKey [-time "1:24"] [object|node.attr ...]`
pub fn copy_key(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, COPY_SPEC)?;
    let times = parse_times(p.string("time"))?;
    let (owners, attrs) = owners_and_attrs(s, &p.arg_strings())?;
    let n = copy_keys(s, &owners, times.as_deref(), attrs.as_deref())?;
    Ok(vec![format!("Copied {} key(s)", n)])
}

/// `cutKeys [-time "1:24"] [object|node.attr ...]`: copy, then delete.
pub fn cut_keys(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, COPY_SPEC)?;
    let times = parse_times(p.string("time"))?;
    let (owners, attrs) = owners_and_attrs(s, &p.arg_strings())?;
    let n = copy_keys(s, &owners, times.as_deref(), attrs.as_deref())?;
    let all: Vec<f32> = match &times {
        Some(t) => t.clone(),
        None => s.scene.anim.key_times(Some(&owners)),
    };
    delete_keys(s, &owners, &all);
    s.scene.evaluate_frame();
    Ok(vec![format!("Cut {} key(s)", n)])
}

/// `pasteKey [-time T] [object ...]`: paste at T (default: the current time).
pub fn paste_key(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, COPY_SPEC)?;
    let at = match p.num("time")? {
        Some(t) => t,
        None => s.scene.timeline.current,
    };
    let names = p.arg_strings();
    let targets: Vec<NodeId> = if names.is_empty() { s.scene.key_owner_ids(&s.scene.selection.clone()) } else { s.scene.key_owner_ids(&resolve_targets(s, &names)?) };
    let t = paste_keys(s, at, &targets)?;
    Ok(vec![format!("Pasted {} key time(s) starting at frame {}", t.len(), fmt_f(at))])
}
