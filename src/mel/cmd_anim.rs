//! Animation curve editing: tangent types, infinity, scaling and moving keys.

use super::cmd_keys::owners_and_attrs;
use super::commands::Out;
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::anim::keyframes::{Infinity, Tan};
use crate::dag::NodeId;
use crate::session::Session;

/// "a:b" (inclusive) or a single time: which keys a command touches.
fn time_filter(range: Option<String>) -> Result<Box<dyn Fn(f32) -> bool>, String> {
    match range {
        None => Ok(Box::new(|_| true)),
        Some(r) => {
            let bad = |_| format!("Bad time '{}': use \"12\" or \"1:24\"", r);
            match r.split_once(':') {
                Some((a, b)) => {
                    let (a, b) = (a.trim().parse::<f32>().map_err(bad)?, b.trim().parse::<f32>().map_err(bad)?);
                    let (lo, hi) = (a.min(b), a.max(b));
                    Ok(Box::new(move |t| t >= lo - 1e-3 && t <= hi + 1e-3))
                }
                None => {
                    let t0 = r.trim().parse::<f32>().map_err(bad)?;
                    Ok(Box::new(move |t| (t - t0).abs() < 1e-3))
                }
            }
        }
    }
}

/// Indexes of the animation bindings the names refer to (object: all its animated channels).
fn binding_indexes(s: &mut Session, names: &[String]) -> Result<Vec<usize>, String> {
    let (owners, attrs) = owners_and_attrs(s, names)?;
    let idx: Vec<usize> = s
        .scene
        .anim
        .bindings
        .iter()
        .enumerate()
        .filter(|(_, b)| match &attrs {
            Some(a) => a.iter().any(|(n, at)| *n == b.node && *at == b.attr),
            None => owners.contains(&b.node),
        })
        .map(|(i, _)| i)
        .collect();
    if idx.is_empty() {
        return Err("No animated channels (select animated objects or name node.attr)".to_string());
    }
    Ok(idx)
}

const TAN_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["inTangentType", "itt"], arity: 1 },
    FlagSpec { names: &["outTangentType", "ott"], arity: 1 },
    FlagSpec { names: &["time", "t"], arity: 1 },
    FlagSpec { names: &["global", "g"], arity: 0 },
    FlagSpec { names: &["edit", "e"], arity: 0 },
];

/// `keyTangent [-itt TYPE] [-ott TYPE] [-t "1:24"] [object|node.attr ...]`: Graph Editor > Tangents. Types: clamped,
/// spline, linear, flat, plateau, fixed (and step for out). With `-g` the types become the default for new keys.
pub fn key_tangent(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, TAN_SPEC)?;
    let parse = |k: &str| -> Result<Option<Tan>, String> {
        match p.string(k) {
            Some(v) => Tan::parse(&v).map(Some).ok_or_else(|| format!("Unknown tangent type '{}': clamped, spline, linear, flat, plateau, fixed or step", v)),
            None => Ok(None),
        }
    };
    let (itt, ott) = (parse("inTangentType")?, parse("outTangentType")?);
    if itt == Some(Tan::Step) {
        return Err("Step is an out tangent type only (-ott step)".to_string());
    }
    if p.has("global") {
        if let Some(t) = itt {
            s.prefs.tangent_in = t.name().to_string();
        }
        if let Some(t) = ott {
            s.prefs.tangent_out = t.name().to_string();
        }
        s.sync_anim_prefs();
        return Ok(vec![format!("New keys: in {}, out {}", s.prefs.tangent_in, s.prefs.tangent_out)]);
    }
    if itt.is_none() && ott.is_none() {
        return Err("Usage: keyTangent [-itt TYPE] [-ott TYPE] [-t \"1:24\"] [-g] [object|node.attr ...]".to_string());
    }
    let when = time_filter(p.string("time"))?;
    let idx = binding_indexes(s, &p.arg_strings())?;
    let mut n = 0;
    for i in idx {
        let c = &mut s.scene.anim.bindings[i].curve;
        let times: Vec<f32> = c.keys.iter().map(|k| k.t).filter(|t| when(*t)).collect();
        for t in times {
            if c.set_tangents_at(t, itt, ott) {
                n += 1;
            }
        }
    }
    s.scene.evaluate_frame();
    if n == 0 {
        return Err("No keys in that time range".to_string());
    }
    Ok(vec![format!("Set the tangents of {} key(s)", n)])
}

const INF_SPEC: &[FlagSpec] = &[FlagSpec { names: &["preInfinite", "pri"], arity: 1 }, FlagSpec { names: &["postInfinite", "poi"], arity: 1 }];

/// `setInfinity [-pri MODE] [-poi MODE] [object|node.attr ...]`: Curves > Pre / Post Infinity. Modes: constant,
/// linear, cycle, cycleWithOffset, oscillate.
pub fn set_infinity(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, INF_SPEC)?;
    let parse = |k: &str| -> Result<Option<Infinity>, String> {
        match p.string(k) {
            Some(v) => Infinity::parse(&v).map(Some).ok_or_else(|| format!("Unknown infinity '{}': constant, linear, cycle, cycleWithOffset or oscillate", v)),
            None => Ok(None),
        }
    };
    let (pre, post) = (parse("preInfinite")?, parse("postInfinite")?);
    if pre.is_none() && post.is_none() {
        return Err("Usage: setInfinity [-pri MODE] [-poi MODE] [object|node.attr ...]".to_string());
    }
    let idx = binding_indexes(s, &p.arg_strings())?;
    for &i in &idx {
        let c = &mut s.scene.anim.bindings[i].curve;
        if let Some(m) = pre {
            c.pre = m;
        }
        if let Some(m) = post {
            c.post = m;
        }
    }
    s.scene.evaluate_frame();
    Ok(vec![format!("Infinity set on {} curve(s)", idx.len())])
}

const SCALE_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["timeScale", "ts"], arity: 1 },
    FlagSpec { names: &["timePivot", "tp"], arity: 1 },
    FlagSpec { names: &["valueScale", "vs"], arity: 1 },
    FlagSpec { names: &["valuePivot", "vp"], arity: 1 },
    FlagSpec { names: &["time", "t"], arity: 1 },
];

/// `scaleKey [-ts S] [-tp PIVOT] [-vs S] [-vp PIVOT] [-t "1:24"] [object|node.attr ...]`: Dope Sheet / Graph Editor
/// scale tool. The time pivot defaults to the first selected key.
pub fn scale_key(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SCALE_SPEC)?;
    let (ts, vs) = (p.num("timeScale")?.unwrap_or(1.0), p.num("valueScale")?.unwrap_or(1.0));
    if ts <= 0.0 {
        return Err("The time scale must be positive".to_string());
    }
    let when = time_filter(p.string("time"))?;
    let idx = binding_indexes(s, &p.arg_strings())?;
    let first = idx.iter().flat_map(|i| s.scene.anim.bindings[*i].curve.keys.iter().map(|k| k.t)).filter(|t| when(*t)).fold(f32::INFINITY, f32::min);
    if !first.is_finite() {
        return Err("No keys in that time range".to_string());
    }
    let (tp, vp) = (p.num("timePivot")?.unwrap_or(first), p.num("valuePivot")?.unwrap_or(0.0));
    let mut n = 0;
    for i in idx {
        let c = &mut s.scene.anim.bindings[i].curve;
        for k in c.keys.iter_mut().filter(|k| when(k.t)) {
            k.t = (tp + (k.t - tp) * ts).max(0.0);
            k.v = vp + (k.v - vp) * vs;
            n += 1;
        }
        c.keys.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        c.recompute_tangents();
    }
    s.scene.evaluate_frame();
    Ok(vec![format!("Scaled {} key(s)", n)])
}

const MOVE_SPEC: &[FlagSpec] = &[FlagSpec { names: &["timeChange", "tc", "dt"], arity: 1 }, FlagSpec { names: &["valueChange", "vc", "dv"], arity: 1 }, FlagSpec { names: &["time", "t"], arity: 1 }];

/// `moveKey [-dt FRAMES] [-dv VALUE] [-t "1:24"] [object|node.attr ...]`: slide keys in time and / or value.
pub fn move_key(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, MOVE_SPEC)?;
    let (dt, dv) = (p.num("timeChange")?.unwrap_or(0.0), p.num("valueChange")?.unwrap_or(0.0));
    let when = time_filter(p.string("time"))?;
    let idx = binding_indexes(s, &p.arg_strings())?;
    let mut n = 0;
    for i in idx {
        let c = &mut s.scene.anim.bindings[i].curve;
        let moving: Vec<f32> = c.keys.iter().map(|k| k.t).filter(|t| when(*t)).collect();
        if moving.is_empty() {
            continue;
        }
        // keys that land on a key that stays replace it
        let landing: Vec<f32> = moving.iter().map(|t| (t + dt).max(0.0)).collect();
        c.keys.retain(|k| moving.iter().any(|m| (k.t - m).abs() < 1e-3) || !landing.iter().any(|l| (k.t - l).abs() < 1e-3));
        for k in c.keys.iter_mut().filter(|k| moving.iter().any(|m| (k.t - m).abs() < 1e-3)) {
            k.t = (k.t + dt).max(0.0);
            k.v += dv;
            n += 1;
        }
        c.keys.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        c.recompute_tangents();
    }
    s.scene.evaluate_frame();
    Ok(vec![format!("Moved {} key(s)", n)])
}

#[allow(dead_code)]
fn _unused(_: NodeId) {}
