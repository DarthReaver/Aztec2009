use super::commands::{execute_script, fmt_f, resolve_targets, Out, HELP};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec};
use crate::anim::Interp;
use crate::dag::nodes::NodeData;
use crate::dag::serialize;
use crate::dag::NodeId;
use crate::render::passes::{render_text, Format, RenderRequest};
use crate::session::Session;
use crate::ui::viewport::{DisplayMode, ViewState};

const AXES: [&str; 3] = ["X", "Y", "Z"];
const TRANSFORM_ATTRS: [&str; 9] = [
    "translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ",
];

fn compound(path: &str) -> Option<(&str, &'static str)> {
    for c in ["translate", "rotate", "scale"] {
        let suffix = format!(".{}", c);
        if let Some(n) = path.strip_suffix(suffix.as_str()) {
            return Some((n, c));
        }
    }
    None
}

fn find(s: &Session, name: &str) -> Result<NodeId, String> {
    s.scene.find(name).ok_or_else(|| format!("No object matches name: {}", name))
}

/// Expand "node.attr", "node.translate" or a bare transform name into concrete plugs.
fn expand_targets(s: &Session, path: &str) -> Result<Vec<(NodeId, String)>, String> {
    if let Some((n, c)) = compound(path) {
        let id = find(s, n)?;
        return Ok(AXES.iter().map(|a| (id, format!("{}{}", c, a))).collect());
    }
    if path.contains('.') {
        return Ok(vec![s.scene.resolve_plug(path)?]);
    }
    let id = find(s, path)?;
    if !s.scene.node(id).map(|n| n.data.is_transform()).unwrap_or(false) {
        return Err(format!("'{}' is not a transform; give an attribute such as {}.intensity", path, path));
    }
    Ok(TRANSFORM_ATTRS.iter().map(|a| (id, a.to_string())).collect())
}

const SETATTR_SPEC: &[FlagSpec] = &[FlagSpec { names: &["lock", "l"], arity: 1 }, FlagSpec { names: &["keyable", "k"], arity: 1 }];

fn flag_on(v: &str) -> bool {
    !matches!(v.to_lowercase().as_str(), "false" | "0" | "off" | "no")
}

pub fn set_attr(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SETATTR_SPEC)?;
    // setAttr -lock true "node.attr" / -keyable false "node.attr" change the attribute's state only
    if (p.has("lock") || p.has("keyable")) && p.args.len() == 1 {
        let path = p.args[0].as_string();
        let targets = expand_targets(s, &path)?;
        for (id, attr) in targets {
            if let Some(v) = p.string("lock") {
                s.scene.set_locked(id, &attr, flag_on(&v))?;
            }
            if let Some(v) = p.string("keyable") {
                s.scene.set_keyable(id, &attr, flag_on(&v))?;
            }
        }
        return Ok(vec![]);
    }
    if p.args.len() < 2 {
        return Err("Usage: setAttr \"node.attr\" value".to_string());
    }
    let path = p.args[0].as_string();
    let vals: Vec<f32> = p.args[1..].iter().map(|v| v.as_f32()).collect::<Result<_, _>>()?;
    if let Some((n, c)) = compound(&path) {
        if vals.len() != 3 {
            return Err(format!("{} needs 3 values", path));
        }
        let id = find(s, n)?;
        for i in 0..3 {
            s.scene.set_attr(id, &format!("{}{}", c, AXES[i]), vals[i])?;
        }
    } else {
        if vals.len() != 1 {
            return Err("setAttr takes a single value for this attribute".to_string());
        }
        let (id, attr) = s.scene.resolve_plug(&path)?;
        s.scene.set_attr(id, &attr, vals[0])?;
    }
    s.scene.evaluate_frame();
    if path.starts_with("persp.") {
        s.view.sync_from_scene(&mut s.scene);
    }
    Ok(vec![])
}

pub fn get_attr(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let path = match p.args.first() {
        Some(v) => v.as_string(),
        None => return Err("Usage: getAttr \"node.attr\"".to_string()),
    };
    if let Some((n, c)) = compound(&path) {
        let id = find(s, n)?;
        let mut parts = Vec::new();
        for a in AXES {
            parts.push(fmt_f(s.scene.get_attr(id, &format!("{}{}", c, a))?));
        }
        return Ok(vec![parts.join(" ")]);
    }
    let (id, attr) = s.scene.resolve_plug(&path)?;
    Ok(vec![fmt_f(s.scene.get_attr(id, &attr)?)])
}

const FORCE_SPEC: &[FlagSpec] = &[FlagSpec { names: &["f", "force"], arity: 0 }];

/// A plug path, or the channels of a compound attribute (`tex.outColor` is outColorR / G / B,
/// `ball.translate` is translateX / Y / Z).
fn expand_plug(s: &Session, path: &str) -> Result<Vec<(NodeId, String)>, String> {
    if let Ok(p) = s.scene.resolve_plug(path) {
        return Ok(vec![p]);
    }
    let idx = path.rfind('.').ok_or_else(|| format!("Invalid attribute path '{}', expected node.attr", path))?;
    let id = s.scene.find(&path[..idx]).ok_or_else(|| format!("No object matches name: {}", &path[..idx]))?;
    let attr = &path[idx + 1..];
    for set in [["R", "G", "B"], ["X", "Y", "Z"]] {
        let names: Vec<String> = set.iter().map(|c| format!("{}{}", attr, c)).collect();
        if names.iter().all(|n| s.scene.node(id).map(|nd| nd.plug(n).is_some()).unwrap_or(false)) {
            return Ok(names.into_iter().map(|n| (id, n)).collect());
        }
    }
    s.scene.resolve_plug(path).map(|p| vec![p])
}

pub fn connect_attr(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, FORCE_SPEC)?;
    if p.args.len() != 2 {
        return Err("Usage: connectAttr [-force] \"src.attr\" \"dst.attr\"".to_string());
    }
    let (a, b) = (p.args[0].as_string(), p.args[1].as_string());
    let (src, dst) = (expand_plug(s, &a)?, expand_plug(s, &b)?);
    if src.len() != dst.len() {
        return Err(format!("'{}' and '{}' do not have the same number of channels", a, b));
    }
    for (x, y) in src.into_iter().zip(dst) {
        s.scene.connect(x, y, p.has("force"))?;
    }
    Ok(vec![format!("Connected {} to {}", a, b)])
}

pub fn disconnect_attr(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    if p.args.len() != 2 {
        return Err("Usage: disconnectAttr \"src.attr\" \"dst.attr\"".to_string());
    }
    let (a, b) = (p.args[0].as_string(), p.args[1].as_string());
    let (src, dst) = (expand_plug(s, &a)?, expand_plug(s, &b)?);
    if src.len() != dst.len() {
        return Err(format!("'{}' and '{}' do not have the same number of channels", a, b));
    }
    for (x, y) in src.into_iter().zip(dst) {
        s.scene.disconnect(x, y)?;
    }
    Ok(vec![format!("Disconnected {} from {}", a, b)])
}

const ADD_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["ln", "longName"], arity: 1 },
    FlagSpec { names: &["dv", "defaultValue"], arity: 1 },
    FlagSpec { names: &["min", "minValue"], arity: 1 },
    FlagSpec { names: &["max", "maxValue"], arity: 1 },
    FlagSpec { names: &["k", "keyable"], arity: 1 },
    FlagSpec { names: &["at", "attributeType"], arity: 1 },
];

pub fn add_attr(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, ADD_SPEC)?;
    let ln = p.string("ln").ok_or("Usage: addAttr -ln NAME [-at float|long|bool] [-dv DEFAULT] [-min N] [-max N] [-k true|false] [node]")?;
    let dv = p.num("dv")?.unwrap_or(0.0);
    let ids = resolve_targets(s, &p.arg_strings())?;
    let id = ids[0];
    let dtype = match p.string("at").unwrap_or_else(|| "float".to_string()).to_lowercase().as_str() {
        "float" | "double" | "doublelinear" | "doubleangle" => 0,
        "long" | "int" | "short" | "integer" => 1,
        "bool" | "boolean" => 2,
        o => return Err(format!("Unknown attribute type '{}': use float, long or bool", o)),
    };
    let keyable = !matches!(p.string("k").unwrap_or_default().to_lowercase().as_str(), "false" | "0" | "off" | "no");
    s.scene.add_attr_typed(id, &ln, dv, p.num("min")?, p.num("max")?, keyable, dtype)?;
    Ok(vec![format!("{}.{}", s.scene.name_of(id), ln)])
}

pub fn list_attr(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["keyable", "k"], arity: 0 }, FlagSpec { names: &["locked", "l"], arity: 0 }, FlagSpec { names: &["userDefined", "ud"], arity: 0 }])?;
    let ids = resolve_targets(s, &p.arg_strings())?;
    let id = ids[0];
    let names: Vec<String> = s
        .scene
        .node(id)
        .map(|n| n.plugs.iter().filter(|pl| (!p.has("keyable") || pl.keyable) && (!p.has("locked") || pl.locked) && (!p.has("userDefined") || pl.custom)).map(|pl| pl.name.clone()).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for a in names {
        let v = s.scene.get_attr(id, &a)?;
        out.push(format!("{} = {}", a, fmt_f(v)));
    }
    Ok(out)
}

const KEY_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["t", "time"], arity: 1 },
    FlagSpec { names: &["v", "value"], arity: 1 },
    FlagSpec { names: &["ip", "interp", "itt"], arity: 1 },
    FlagSpec { names: &["animLayer", "al", "layer"], arity: 1 },
    FlagSpec { names: &["insert", "i"], arity: 0 },
    FlagSpec { names: &["attribute", "at"], arity: 1 },
];

fn layer_index(s: &Session, p: &super::parser::Parsed) -> Result<usize, String> {
    match p.string("animLayer") {
        Some(l) => s.scene.anim.find_layer(&l).ok_or_else(|| format!("No animation layer named '{}'", l)),
        None => Ok(s.scene.anim.active_layer.min(s.scene.anim.layers.len() - 1)),
    }
}

pub fn set_keyframe(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, KEY_SPEC)?;
    let layer = layer_index(s, &p)?;
    let interp = match p.string("ip") {
        Some(i) => Interp::parse(&i).ok_or_else(|| format!("Unknown interpolation '{}', use linear, bezier or step", i))?,
        None => Interp::parse(&s.prefs.key_tangent).unwrap_or(Interp::Bezier),
    };
    let t = p.num("t")?;
    let mut v = p.num("v")?;
    let insert = p.has("insert");
    let mut targets: Vec<(NodeId, String)> = Vec::new();
    let paths = p.arg_strings();
    if paths.is_empty() && p.string("attribute").is_none() {
        // with a current character set, Set Key keys the whole character
        if t.is_none() && v.is_none() {
            if let Some(n) = super::cmd_char::key_current(s, interp) {
                return Ok(vec![format!("{}", n)]);
            }
        }
        let ids = resolve_targets(s, &[])?;
        for id in ids {
            if s.scene.node(id).map(|n| n.data.is_transform()).unwrap_or(false) {
                for a in TRANSFORM_ATTRS {
                    targets.push((id, a.to_string()));
                }
            }
        }
        if targets.is_empty() {
            return Err("Usage: setKeyframe [-t T] [-v V] \"node.attr\"".to_string());
        }
    } else if let Some(at) = p.string("attribute") {
        // -at blink lookAt  (or -at lookAt.blink: the path itself)
        if at.contains('.') {
            targets.extend(expand_targets(s, &at)?);
        } else {
            for path in &paths {
                let ids = resolve_targets(s, &[path.clone()])?;
                for id in ids {
                    targets.extend(expand_targets(s, &format!("{}.{}", s.scene.name_of(id), at))?);
                }
            }
        }
    } else {
        for path in &paths {
            targets.extend(expand_targets(s, path)?);
        }
    }
    if v.is_some() && targets.len() != 1 {
        return Err("-v can only be used when keying a single attribute".to_string());
    }
    let mut count = 0;
    for (id, attr) in targets {
        // insert: the new key sits on the curve as it is, so the shape does not change
        if insert {
            let at = t.unwrap_or(s.scene.timeline.current);
            v = s.scene.anim.bindings.iter().find(|b| b.node == id && b.attr == attr && b.layer == layer).and_then(|b| b.curve.eval(at));
        }
        s.scene.set_keyframe(id, &attr, t, v, interp, layer)?;
        count += 1;
    }
    Ok(vec![format!("{}", count)])
}

pub fn cut_key(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, KEY_SPEC)?;
    let t = p.num("t")?.ok_or("Usage: cutKey -t TIME \"node.attr\"")?;
    let layer = layer_index(s, &p)?;
    let mut count = 0;
    for path in p.arg_strings() {
        for (id, attr) in expand_targets(s, &path)? {
            if s.scene.anim.remove_key(id, &attr, layer, t) {
                count += 1;
                if !s.scene.anim.is_animated(id, &attr) {
                    let rest_v = s.scene.node(id).and_then(|n| n.plug(&attr)).map(|p| p.rest).unwrap_or(0.0);
                    s.scene.set_eval(id, &attr, rest_v);
                }
            }
        }
    }
    s.scene.evaluate_frame();
    Ok(vec![format!("{}", count)])
}

pub fn keyframe_query(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let mut out = Vec::new();
    for path in p.arg_strings() {
        for (id, attr) in expand_targets(s, &path)? {
            for b in s.scene.anim.bindings.iter().filter(|b| b.node == id && b.attr == attr) {
                let lname = &s.scene.anim.layers[b.layer].name;
                for k in &b.curve.keys {
                    out.push(format!(
                        "{}.{} [{}] t={} v={} {}",
                        s.scene.name_of(id),
                        attr,
                        lname,
                        fmt_f(k.t),
                        fmt_f(k.v),
                        k.interp.name()
                    ));
                }
            }
        }
    }
    if out.is_empty() {
        out.push("No keyframes".to_string());
    }
    Ok(out)
}

const LAYER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["e", "edit"], arity: 0 },
    FlagSpec { names: &["w", "weight"], arity: 1 },
    FlagSpec { names: &["mute"], arity: 1 },
    FlagSpec { names: &["l", "list", "q", "query"], arity: 0 },
    FlagSpec { names: &["active"], arity: 0 },
];

pub fn anim_layer(s: &mut Session, rest: &[Token]) -> Out {
    if let Some(r) = super::cmd_trax::anim_layer_extra(s, rest) {
        return r;
    }
    let p = parse_args(rest, LAYER_SPEC)?;
    if p.has("list") {
        let mut out = Vec::new();
        for (i, l) in s.scene.anim.layers.iter().enumerate() {
            out.push(format!(
                "{}{} weight={} mute={}{}",
                l.name,
                if i == 0 { " (base)" } else { "" },
                fmt_f(l.weight),
                if l.mute { 1 } else { 0 },
                if i == s.scene.anim.active_layer { " *active" } else { "" }
            ));
        }
        return Ok(out);
    }
    let name = p.args.first().map(|a| a.as_string()).ok_or("Usage: animLayer NAME   or   animLayer -e [-w W] NAME")?;
    let idx = if p.has("edit") {
        s.scene.anim.find_layer(&name).ok_or_else(|| format!("No animation layer named '{}'", name))?
    } else {
        let i = s.scene.anim.add_layer(&name)?;
        s.scene.anim.active_layer = i;
        i
    };
    if let Some(w) = p.num("w")? {
        if idx == 0 {
            return Err("The base layer weight is fixed at 1".to_string());
        }
        s.scene.anim.set_weight(idx, w);
    }
    if let Some(m) = p.num("mute")? {
        s.scene.anim.set_mute(idx, m > 0.5);
    }
    if p.has("active") {
        s.scene.anim.active_layer = idx;
    }
    s.scene.evaluate_frame();
    Ok(if p.has("edit") { vec![] } else { vec![name] })
}

pub fn current_time(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[FlagSpec { names: &["query", "q"], arity: 0 }, FlagSpec { names: &["edit", "e"], arity: 0 }, FlagSpec { names: &["update", "u"], arity: 1 }])?;
    if p.has("query") {
        return Ok(vec![fmt_f(s.scene.timeline.current)]);
    }
    match p.args.first() {
        Some(v) => {
            s.scene.set_time(v.as_f32()?);
            s.view.sync_from_scene(&mut s.scene);
            Ok(vec![])
        }
        None => Ok(vec![fmt_f(s.scene.timeline.current)]),
    }
}

pub fn play(s: &mut Session) -> Out {
    s.scene.timeline.play();
    Ok(vec!["Playing".to_string()])
}

pub fn stop(s: &mut Session) -> Out {
    s.scene.timeline.pause();
    Ok(vec![format!("Stopped at frame {}", s.scene.timeline.frame())])
}

pub fn rewind(s: &mut Session) -> Out {
    let st = s.scene.timeline.start;
    s.scene.set_time(st);
    s.view.sync_from_scene(&mut s.scene);
    Ok(vec![])
}

pub fn step(s: &mut Session, forward: bool) -> Out {
    if forward {
        s.scene.timeline.step_forward();
    } else {
        s.scene.timeline.step_back();
    }
    s.scene.evaluate_frame();
    s.view.sync_from_scene(&mut s.scene);
    Ok(vec![])
}

const PLAYBACK_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["min"], arity: 1 },
    FlagSpec { names: &["max"], arity: 1 },
    FlagSpec { names: &["fps"], arity: 1 },
    FlagSpec { names: &["loop"], arity: 1 },
];

pub fn playback_options(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, PLAYBACK_SPEC)?;
    let (mut a, mut b) = (s.scene.timeline.start, s.scene.timeline.end);
    if let Some(v) = p.num("min")? {
        a = v;
    }
    if let Some(v) = p.num("max")? {
        b = v;
    }
    s.scene.timeline.set_range(a, b);
    if let Some(v) = p.num("fps")? {
        if !(1.0..=240.0).contains(&v) {
            return Err("fps must be between 1 and 240".to_string());
        }
        s.scene.timeline.fps = v;
    }
    if let Some(v) = p.num("loop")? {
        s.scene.timeline.looping = v > 0.5;
    }
    s.scene.evaluate_frame();
    let t = &s.scene.timeline;
    Ok(vec![format!(
        "range {} to {}  fps {}  loop {}",
        fmt_f(t.start),
        fmt_f(t.end),
        fmt_f(t.fps),
        if t.looping { "on" } else { "off" }
    )])
}

const RENDER_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["m", "mode"], arity: 1 },
    FlagSpec { names: &["o", "out"], arity: 1 },
    FlagSpec { names: &["f", "format"], arity: 1 },
    FlagSpec { names: &["w", "width"], arity: 1 },
    FlagSpec { names: &["h", "height"], arity: 1 },
    FlagSpec { names: &["cam", "camera"], arity: 1 },
];

pub fn render(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, RENDER_SPEC)?;
    let mode = p.string("mode").unwrap_or_else(|| "beauty".to_string());
    let format = match p.string("format") {
        Some(f) => Some(Format::parse(&f).ok_or_else(|| format!("Unknown format '{}', use ascii, ansi or ppm", f))?),
        None => None,
    };
    let w = p.num("width")?.unwrap_or(100.0) as usize;
    let h = p.num("height")?.unwrap_or(50.0) as usize;
    let cam = p.string("cam");
    let out_path = p.string("out");
    let req = RenderRequest { mode: &mode, format, w, h, camera: cam.as_deref() };
    let wants_color = matches!(format, Some(Format::Ansi) | Some(Format::Ppm)) || mode.eq_ignore_ascii_case("anaglyph");
    if out_path.is_none() && wants_color && format != Some(Format::Ascii) {
        return Err("Colored and PPM output must be written to a file with -out FILE".to_string());
    }
    let text = render_text(&mut s.scene, &req)?;
    match out_path {
        Some(path) => {
            std::fs::write(&path, &text).map_err(|e| format!("Cannot write '{}': {}", path, e))?;
            Ok(vec![format!("Rendered {} to {} ({} bytes)", mode, path, text.len())])
        }
        None => Ok(text.lines().map(|l| l.to_string()).collect()),
    }
}

const FILE_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["save", "s"], arity: 0 },
    FlagSpec { names: &["open", "o"], arity: 0 },
    FlagSpec { names: &["new", "n"], arity: 0 },
    FlagSpec { names: &["force", "f"], arity: 0 },
    FlagSpec { names: &["import", "i"], arity: 0 },
    FlagSpec { names: &["exportAll", "ea", "export", "ex"], arity: 0 },
    FlagSpec { names: &["exportSelected", "es", "selected", "sel"], arity: 0 },
    FlagSpec { names: &["ascii"], arity: 0 },
    FlagSpec { names: &["type", "typ"], arity: 1 },
    FlagSpec { names: &["reference", "r"], arity: 0 },
    FlagSpec { names: &["namespace", "ns"], arity: 1 },
    FlagSpec { names: &["unloadReference", "ur"], arity: 1 },
    FlagSpec { names: &["loadReference", "lr"], arity: 1 },
    FlagSpec { names: &["removeReference", "rr"], arity: 1 },
];

fn with_ext(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if file.contains('.') {
        path.to_string()
    } else {
        format!("{}.aztec", path)
    }
}

/// A bare scene name is looked up in (or saved to) the project's scenes folder when a project is set.
fn project_scene_path(s: &Session, path: &str, for_write: bool) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() || path.contains('/') {
        return path.to_string();
    }
    match &s.scene.project_dir {
        Some(d) => {
            let in_project = d.join("scenes").join(path);
            if for_write || (!p.exists() && in_project.exists()) {
                in_project.to_string_lossy().to_string()
            } else {
                path.to_string()
            }
        }
        None => path.to_string(),
    }
}

pub fn file(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, FILE_SPEC)?;
    let arg = p.args.first().map(|a| a.as_string());
    if p.has("new") {
        s.scene.new_scene();
        s.view = ViewState::new();
        s.view.apply(&mut s.scene);
        return Ok(vec!["New scene".to_string()]);
    }
    if p.has("reference") {
        let path = arg.ok_or("Usage: file -reference -namespace NS \"character.aztec\"")?;
        let ns = p.string("namespace").unwrap_or_else(|| {
            let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "ref".to_string());
            stem.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
        });
        let path = project_scene_path(s, &with_ext(&path), false);
        let n = s.scene.create_reference(&path, &ns)?;
        return Ok(vec![format!("Referenced {} as {}_ ({} nodes)", path, ns, n)]);
    }
    if let Some(ns) = p.string("unloadReference") {
        let n = s.scene.unload_reference(&ns)?;
        return Ok(vec![format!("Unloaded {} ({} nodes)", ns, n)]);
    }
    if let Some(ns) = p.string("loadReference") {
        let n = s.scene.load_reference(&ns)?;
        return Ok(vec![format!("Loaded {} ({} nodes)", ns, n)]);
    }
    if let Some(ns) = p.string("removeReference") {
        s.scene.remove_reference(&ns)?;
        return Ok(vec![format!("Removed the reference {}", ns)]);
    }
    if p.has("import") {
        let path = arg.ok_or("Usage: file -import \"model.obj\"  (or .fbx)")?;
        let msg = crate::io::import_file(&mut s.scene, &path)?;
        return Ok(vec![msg]);
    }
    if p.has("exportAll") || p.has("exportSelected") {
        let path = arg.ok_or("Usage: file -export \"model.fbx\" [-selected] [-ascii]  (or .obj)")?;
        let opts = crate::io::ExportOptions { selected_only: p.has("exportSelected"), fbx_ascii: p.has("ascii") };
        let msg = crate::io::write_file(&mut s.scene, &path, &opts)?;
        return Ok(vec![msg]);
    }
    if p.has("open") && arg.as_deref().map(|a| a.to_lowercase().ends_with(".ma")).unwrap_or(false) {
        // a Maya ASCII scene opens as a new scene with its contents imported
        let path = arg.clone().unwrap_or_default();
        s.scene.new_scene();
        s.view = ViewState::new();
        s.view.apply(&mut s.scene);
        let msg = crate::io::import_file(&mut s.scene, &path)?;
        s.scene.selection.clear();
        s.resync_view();
        return Ok(vec![msg]);
    }
    if p.has("open") {
        let path = project_scene_path(s, &with_ext(&arg.ok_or("Usage: file -open \"scene.aztec\"")?), false);
        let src = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read '{}': {}", path, e))?;
        let scene = serialize::from_ascii(&src)?;
        let project = s.scene.project_dir.clone();
        s.scene = scene;
        s.scene.project_dir = project;
        // a scene inside a project's scenes folder selects that project
        if let Some(root) = std::path::Path::new(&path).canonicalize().ok().and_then(|p| p.parent().and_then(|d| if d.file_name().map(|n| n == "scenes").unwrap_or(false) { d.parent().map(|r| r.to_path_buf()) } else { None })) {
            if root.join("workspace.mel").exists() {
                s.scene.project_dir = Some(root);
            }
        }
        s.scene.file_path = Some(path.clone());
        s.resync_view();
        return Ok(vec![format!("Opened {}", path)]);
    }
    if p.has("save") {
        let path = match arg {
            Some(a) => project_scene_path(s, &with_ext(&a), true),
            None => s.scene.file_path.clone().ok_or("No file name: use file -save \"scene.aztec\"")?,
        };
        let text = serialize::to_ascii(&s.scene);
        std::fs::write(&path, &text).map_err(|e| format!("Cannot write '{}': {}", path, e))?;
        s.scene.file_path = Some(path.clone());
        return Ok(vec![format!("Saved {}", path)]);
    }
    Ok(vec![s.scene.file_path.clone().unwrap_or_else(|| "untitled".to_string())])
}

const VIEW_SPEC: &[FlagSpec] = &[
    FlagSpec { names: &["mode", "m"], arity: 1 },
    FlagSpec { names: &["cam", "camera"], arity: 1 },
    FlagSpec { names: &["grid"], arity: 1 },
    FlagSpec { names: &["yaw"], arity: 1 },
    FlagSpec { names: &["pitch"], arity: 1 },
    FlagSpec { names: &["dist"], arity: 1 },
    FlagSpec { names: &["target"], arity: 3 },
    FlagSpec { names: &["stereo"], arity: 1 },
    FlagSpec { names: &["borders"], arity: 1 },
    FlagSpec { names: &["normals"], arity: 1 },
    FlagSpec { names: &["show"], arity: 2 },
];

pub fn viewport(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, VIEW_SPEC)?;
    if let Some(m) = p.string("mode") {
        s.view.mode = DisplayMode::parse(&m).ok_or_else(|| format!("Unknown display mode '{}', use shaded, textured, wireframe or anaglyph", m))?;
    }
    if let Some(c) = p.string("cam") {
        if c == "persp" {
            s.view.camera = None;
        } else {
            crate::render::camera_info(&mut s.scene, Some(&c))?;
            s.view.camera = Some(c);
        }
    }
    if let Some(v) = p.get("show") {
        let name = v[0].as_string();
        let on = v[1].as_f64()? > 0.5;
        let sh = &mut s.scene.show;
        match name.as_str() {
            "polygons" => sh.polygons = on,
            "lights" => sh.lights = on,
            "cameras" => sh.cameras = on,
            "joints" => sh.joints = on,
            "ikHandles" => sh.ik_handles = on,
            "locators" => sh.locators = on,
            "emitters" => sh.emitters = on,
            "particles" => sh.particles = on,
            "manipulators" => sh.manipulators = on,
            "pivots" => sh.pivots = on,
            "handles" => sh.handles = on,
            "localAxes" => sh.local_axes = on,
            other => return Err(format!("Unknown show type '{}': polygons lights cameras joints ikHandles locators emitters particles manipulators pivots handles localAxes", other)),
        }
    }
    if let Some(b) = p.num("normals")? {
        s.scene.show_normals = b > 0.5;
    }
    if let Some(b) = p.num("borders")? {
        s.scene.show_borders = b > 0.5;
    }
    if let Some(g) = p.num("grid")? {
        s.view.grid = g > 0.5;
    }
    if let Some(v) = p.num("yaw")? {
        s.view.yaw = v;
    }
    if let Some(v) = p.num("pitch")? {
        s.view.pitch = v.clamp(-89.0, 89.0);
    }
    if let Some(v) = p.num("dist")? {
        s.view.dist = v.clamp(0.5, 500.0);
    }
    if let Some(t) = p.get("target") {
        s.view.target = crate::math::Vec3::new(t[0].as_f32()?, t[1].as_f32()?, t[2].as_f32()?);
    }
    if let Some(v) = p.num("stereo")? {
        s.view.stereo_scale = v.clamp(0.0, 20.0);
    }
    s.view.apply(&mut s.scene);
    Ok(vec![format!(
        "viewport mode={} camera={}",
        s.view.mode.name(),
        s.view.camera.clone().unwrap_or_else(|| "persp".to_string())
    )])
}

pub fn frame_all(s: &mut Session) -> Out {
    s.view.frame_scene(&mut s.scene);
    Ok(vec![])
}

pub fn stats(s: &mut Session) -> Out {
    let mut transforms = 0;
    let mut meshes = 0;
    let mut verts = 0;
    let mut tris = 0;
    let mut lights = 0;
    let mut cameras = 0;
    let mut mats = 0;
    let mut particles = 0;
    for (_, n) in s.scene.nodes.iter() {
        match &n.data {
            NodeData::Transform => transforms += 1,
            NodeData::Mesh(m) => {
                meshes += 1;
                verts += m.mesh.vertex_count();
                tris += m.mesh.triangle_count();
            }
            NodeData::Light => lights += 1,
            NodeData::Camera | NodeData::StereoCamera => cameras += 1,
            NodeData::Material => mats += 1,
            NodeData::Nucleus(nd) => particles += nd.solver.particle_count(),
            NodeData::PolyOp(_) | NodeData::Joint | NodeData::IkHandle(_) | NodeData::Emitter | NodeData::Locator | NodeData::Constraint(_) | NodeData::DrivenCurve(_) | NodeData::ObjectSet(_) | NodeData::Texture(_) | NodeData::MotionPath(_) | NodeData::Brush(_) | NodeData::RenderGlobals(_) | NodeData::RigidBody(_) | NodeData::DynSolver(_) | NodeData::Field(_) | NodeData::RigidJoint(_) | NodeData::PEmitter(_) | NodeData::Particle(_) | NodeData::Sound(_) | NodeData::Lattice(_) | NodeData::Character(_) | NodeData::Distance(_) => {}
            NodeData::NurbsCurve(_) => {}
        }
    }
    let keys: usize = s.scene.anim.bindings.iter().map(|b| b.curve.keys.len()).sum();
    Ok(vec![
        format!("nodes {}  transforms {}  meshes {}  cameras {}  lights {}  materials {}", s.scene.node_count(), transforms, meshes, cameras, lights, mats),
        format!("vertices {}  triangles {}  particles {}", verts, tris, particles),
        format!("keyframes {}  animation layers {}  connections {}", keys, s.scene.anim.layers.len(), s.scene.connections().len()),
    ])
}

pub fn print(_s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    Ok(vec![p.arg_strings().join(" ")])
}

pub fn source(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let path = p.args.first().map(|a| a.as_string()).ok_or("Usage: source \"script.mel\"")?;
    // a bare name is looked up in the script folders
    let path = match super::cmd_mel::find_script(s, &path) {
        Some(f) if !std::path::Path::new(&path).exists() => f.to_string_lossy().to_string(),
        _ => path,
    };
    let src = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read '{}': {}", path, e))?;
    if s.source_depth > 8 {
        return Err("source nesting is too deep".to_string());
    }
    s.source_depth += 1;
    let o = execute_script(s, &src);
    s.source_depth -= 1;
    match o.error {
        Some(e) => Err(format!("{}: {}", path, e)),
        None => Ok(o.lines),
    }
}

pub fn help(rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    if let Some(c) = p.args.first() {
        let c = c.as_string();
        return match HELP.iter().find(|(n, _)| *n == c) {
            Some((_, u)) => Ok(vec![u.to_string()]),
            None => Err(format!("No help for '{}'", c)),
        };
    }
    let mut out = vec!["Aztec Shell commands (help COMMAND for usage):".to_string()];
    let names: Vec<&str> = HELP.iter().map(|(n, _)| *n).collect();
    for chunk in names.chunks(5) {
        out.push(format!("  {}", chunk.join("  ")));
    }
    out.push("Variables: $x = 5;   print $x   Comments: // and /* */".to_string());
    Ok(out)
}

const RAY_SPEC: &[super::parser::FlagSpec] = &[
    super::parser::FlagSpec { names: &["w", "width"], arity: 1 },
    super::parser::FlagSpec { names: &["h", "height"], arity: 1 },
    super::parser::FlagSpec { names: &["q", "quality"], arity: 1 },
    super::parser::FlagSpec { names: &["out", "o"], arity: 1 },
    super::parser::FlagSpec { names: &["cam"], arity: 1 },
];

/// rayRender [-w N] [-h N] [-q 1..4] [-cam NAME] -out file.png
pub fn ray_render(s: &mut Session, rest: &[Token]) -> Out {
    let p = super::parser::parse_args(rest, RAY_SPEC)?;
    let w = p.num("w")?.unwrap_or(640.0).clamp(16.0, 4096.0) as usize;
    let h = p.num("h")?.unwrap_or(360.0).clamp(16.0, 4096.0) as usize;
    let q = p.num("q")?.unwrap_or(2.0).clamp(1.0, 4.0) as usize;
    let out = p.string("out").unwrap_or_else(|| format!("raytrace_{}.png", s.scene.timeline.frame()));
    let cam = crate::render::camera_info(&mut s.scene, p.string("cam").as_deref())?;
    s.scene.final_render = true;
    let mut rs = crate::render::build_scene(&mut s.scene);
    s.scene.final_render = false;
    rs.use_textures = true;
    let aspect = w as f32 / h as f32;
    let started = std::time::Instant::now();
    let opt = crate::render::raytrace::Options { grid: q, ..Default::default() };
    let pix = crate::render::raytrace::raytrace(&rs, &cam.view(), &cam.projection(aspect), w, h, &opt, None);
    let mut rgba = vec![255u8; w * h * 4];
    for (i, c) in pix.iter().enumerate() {
        rgba[i * 4] = (c.x * 255.0) as u8;
        rgba[i * 4 + 1] = (c.y * 255.0) as u8;
        rgba[i * 4 + 2] = (c.z * 255.0) as u8;
    }
    std::fs::write(&out, crate::gui::png::encode_rgba(w, h, &rgba)).map_err(|e| format!("Cannot write {}: {}", out, e))?;
    Ok(vec![format!("Raytraced {}x{} to {} in {:.2}s", w, h, out, started.elapsed().as_secs_f32())])
}
