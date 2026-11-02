//! Non-linear animation (Trax): clip sources, clips on tracks, blends, time warps and the evaluation of their channels.

use super::keyframes::{Curve, Infinity, Interp, Key, Tan};
use std::collections::HashMap;

/// One animated channel of a clip source; key times count from the start of the source.
#[derive(Clone, Debug)]
pub struct SrcChannel {
    pub node: String,
    pub attr: String,
    pub curve: Curve,
}

/// The animation of a character kept apart from the scene's keys (Visor > Character Clips).
#[derive(Clone, Debug, Default)]
pub struct ClipSource {
    pub name: String,
    pub character: String,
    pub duration: f32,
    pub channels: Vec<SrcChannel>,
}

/// A clip: a part of a source placed on a track of the Trax Editor.
#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub source: String,
    pub character: String,
    pub track: usize,
    /// Scene frame where the used part of the source begins.
    pub start: f32,
    /// The used part of the source, in source frames.
    pub src_in: f32,
    pub src_out: f32,
    /// Scene frames per source frame (2 plays at half speed).
    pub scale: f32,
    pub cycles: f32,
    /// Each cycle continues from where the last one ended (like Cycle with Offset).
    pub cycle_offset: bool,
    /// Channels move on from the end of the clip before it instead of using the keyed values.
    pub relative: bool,
    pub weight: f32,
    pub enabled: bool,
    /// Time warp: (frames since the clip start, source frame).
    pub warp: Option<Curve>,
}

impl Clip {
    pub fn new(name: &str, source: &str, character: &str, start: f32, duration: f32) -> Clip {
        Clip { name: name.to_string(), source: source.to_string(), character: character.to_string(), track: 0, start, src_in: 0.0, src_out: duration, scale: 1.0, cycles: 1.0, cycle_offset: false, relative: false, weight: 1.0, enabled: true, warp: None }
    }

    /// Frames the clip takes in the scene.
    pub fn length(&self) -> f32 {
        ((self.src_out - self.src_in) * self.scale * self.cycles).max(0.0)
    }

    pub fn end(&self) -> f32 {
        self.start + self.length()
    }

    /// The source frame shown at scene time `t`, and how many whole cycles came before.
    pub fn source_time(&self, t: f32) -> (f32, f32) {
        let local = (t - self.start).clamp(0.0, self.length());
        if let Some(w) = &self.warp {
            if let Some(s) = w.eval(local) {
                return (s, 0.0);
            }
        }
        let period = (self.src_out - self.src_in).max(1e-6);
        let u = local / self.scale.max(1e-6);
        let mut cyc = (u / period).floor();
        // the very end belongs to the last cycle
        if cyc >= self.cycles {
            cyc = (self.cycles - 1.0).max(0.0);
        }
        (self.src_in + (u - cyc * period), cyc)
    }
}

/// A blend between two clips of one track: across their overlap the first fades into the second.
#[derive(Clone, Debug)]
pub struct Blend {
    pub a: String,
    pub b: String,
    /// 0 linear, 1 ease in, 2 ease out, 3 ease in out.
    pub shape: u8,
}

pub const BLEND_SHAPES: &[&str] = &["linear", "easeIn", "easeOut", "easeInOut"];

pub fn blend_weight(shape: u8, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    match shape {
        1 => x * x,
        2 => 1.0 - (1.0 - x) * (1.0 - x),
        3 => x * x * (3.0 - 2.0 * x),
        _ => x,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Trax {
    pub sources: Vec<ClipSource>,
    pub clips: Vec<Clip>,
    pub blends: Vec<Blend>,
}

impl Trax {
    pub fn source(&self, name: &str) -> Option<&ClipSource> {
        self.sources.iter().find(|s| s.name == name)
    }

    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.name == name)
    }

    pub fn clip_mut(&mut self, name: &str) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|c| c.name == name)
    }

    /// A name not used by any clip or source.
    pub fn unique(&self, base: &str) -> String {
        if self.clip(base).is_none() && self.source(base).is_none() {
            return base.to_string();
        }
        let mut i = 1;
        loop {
            let n = format!("{}{}", base, i);
            if self.clip(&n).is_none() && self.source(&n).is_none() {
                return n;
            }
            i += 1;
        }
    }

    fn channel(&self, clip: &Clip, node: &str, attr: &str) -> Option<&SrcChannel> {
        self.source(&clip.source)?.channels.iter().find(|c| c.node == node && c.attr == attr)
    }

    /// Value of a clip's channel at `t` with its cycle offsets, but without the relative offset.
    fn raw_value(&self, clip: &Clip, node: &str, attr: &str, t: f32) -> Option<f32> {
        let ch = self.channel(clip, node, attr)?;
        let (s, cyc) = clip.source_time(t);
        let mut v = ch.curve.eval(s)?;
        if clip.cycle_offset && cyc > 0.0 {
            let (a, b) = (ch.curve.eval(clip.src_in)?, ch.curve.eval(clip.src_out)?);
            v += cyc * (b - a);
        }
        Some(v)
    }

    /// Clips of a character and track, ordered by start.
    pub fn track_clips(&self, character: &str, track: usize) -> Vec<&Clip> {
        let mut v: Vec<&Clip> = self.clips.iter().filter(|c| c.enabled && c.track == track && c.character == character).collect();
        v.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        v
    }

    /// The offset a relative clip adds: it starts where the clip before it on the track ended.
    fn relative_offset(&self, clip: &Clip, node: &str, attr: &str, depth: usize) -> f32 {
        if !clip.relative || depth > 16 {
            return 0.0;
        }
        let list = self.track_clips(&clip.character, clip.track);
        let Some(pos) = list.iter().position(|c| c.name == clip.name) else { return 0.0 };
        if pos == 0 {
            return 0.0;
        }
        let prev = list[pos - 1];
        let Some(first) = self.raw_value(clip, node, attr, clip.start) else { return 0.0 };
        match self.value_at(prev, node, attr, prev.end(), depth + 1) {
            Some(end) => end - first,
            None => 0.0,
        }
    }

    /// The clip's channel at `t`, relative offset included.
    fn value_at(&self, clip: &Clip, node: &str, attr: &str, t: f32, depth: usize) -> Option<f32> {
        let raw = self.raw_value(clip, node, attr, t)?;
        Some(raw + self.relative_offset(clip, node, attr, depth))
    }

    /// Track contribution at `t`: the clip under the time, or the blend of two overlapping clips.
    fn track_value(&self, character: &str, track: usize, node: &str, attr: &str, t: f32) -> Option<(f32, f32)> {
        let list = self.track_clips(character, track);
        let active: Vec<&&Clip> = list.iter().filter(|c| t >= c.start - 1e-4 && t <= c.end() + 1e-4).collect();
        match active.len() {
            0 => None,
            1 => {
                let c = active[0];
                self.value_at(c, node, attr, t, 0).map(|v| (v, c.weight))
            }
            _ => {
                // the last two overlapping clips
                let (a, b) = (active[active.len() - 2], active[active.len() - 1]);
                let vb = self.value_at(b, node, attr, t, 0);
                let va = self.value_at(a, node, attr, t, 0);
                match (va, vb) {
                    (Some(va), Some(vb)) => {
                        let blend = self.blends.iter().find(|bl| bl.a == a.name && bl.b == b.name);
                        match blend {
                            Some(bl) => {
                                let x = (t - b.start) / (a.end() - b.start).max(1e-6);
                                let w = blend_weight(bl.shape, x);
                                Some((va + (vb - va) * w, a.weight + (b.weight - a.weight) * w))
                            }
                            None => Some((vb, b.weight)),
                        }
                    }
                    (None, Some(vb)) => Some((vb, b.weight)),
                    (Some(va), None) => Some((va, a.weight)),
                    (None, None) => None,
                }
            }
        }
    }

    /// Values of every channel the enabled clips of `character` (all when empty) drive at `t`. `base` gives the value
    /// the channel has without clips, for clips with a weight below one.
    pub fn evaluate(&self, character: Option<&str>, t: f32, base: &dyn Fn(&str, &str) -> f32) -> Vec<(String, String, f32)> {
        let mut keys: Vec<(String, String, String)> = Vec::new();
        for c in self.clips.iter().filter(|c| c.enabled && character.map(|ch| ch == c.character).unwrap_or(true)) {
            if let Some(src) = self.source(&c.source) {
                for ch in &src.channels {
                    let k = (c.character.clone(), ch.node.clone(), ch.attr.clone());
                    if !keys.contains(&k) {
                        keys.push(k);
                    }
                }
            }
        }
        let max_track = self.clips.iter().map(|c| c.track).max().unwrap_or(0);
        let mut out = Vec::new();
        for (chr, node, attr) in keys {
            let mut acc: Option<f32> = None;
            for track in 0..=max_track {
                if let Some((v, w)) = self.track_value(&chr, track, &node, &attr, t) {
                    let below = acc.unwrap_or_else(|| base(&node, &attr));
                    acc = Some(below + (v - below) * w.clamp(0.0, 1.0));
                }
            }
            if acc.is_none() {
                // outside every clip the channel holds the first or the last pose
                let mut with: Vec<&Clip> = self.clips.iter().filter(|c| c.enabled && c.character == chr && self.channel(c, &node, &attr).is_some()).collect();
                with.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                if let (Some(first), Some(last)) = (with.first(), with.iter().max_by(|a, b| a.end().partial_cmp(&b.end()).unwrap())) {
                    if t < first.start {
                        acc = self.value_at(first, &node, &attr, first.start, 0);
                    } else if t > last.end() {
                        acc = self.value_at(last, &node, &attr, last.end(), 0);
                    }
                }
            }
            if let Some(v) = acc {
                out.push((node, attr, v));
            }
        }
        out
    }
}

/// Text form of a curve: infinity modes, then `time value interp inTan outTan` for every key, separated by `;`.
pub fn curve_to_text(c: &Curve) -> String {
    let mut s = format!("{} {}", c.pre.name(), c.post.name());
    for k in &c.keys {
        s.push_str(&format!(";{} {} {} {} {}", k.t, k.v, k.interp.name(), k.in_tan.name(), k.out_tan.name()));
    }
    s
}

pub fn curve_from_text(text: &str) -> Curve {
    let mut parts = text.split(';');
    let mut c = Curve::new();
    if let Some(h) = parts.next() {
        let mut it = h.split_whitespace();
        c.pre = it.next().and_then(Infinity::parse).unwrap_or_default();
        c.post = it.next().and_then(Infinity::parse).unwrap_or_default();
    }
    for p in parts {
        let f: Vec<&str> = p.split_whitespace().collect();
        if f.len() < 5 {
            continue;
        }
        let (Ok(t), Ok(v)) = (f[0].parse::<f32>(), f[1].parse::<f32>()) else { continue };
        c.keys.push(Key { t, v, interp: Interp::parse(f[2]).unwrap_or(Interp::Bezier), in_dt: 0.0, in_dv: 0.0, out_dt: 0.0, out_dv: 0.0, in_tan: Tan::parse(f[3]).unwrap_or(Tan::Clamped), out_tan: Tan::parse(f[4]).unwrap_or(Tan::Clamped) });
    }
    c.keys.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    c.recompute_tangents();
    c
}

/// Source and clip text for files: `source NAME CHARACTER DURATION` then `channel NODE ATTR CURVE` lines.
pub fn source_to_file(s: &ClipSource) -> String {
    let mut out = format!("//Aztec 2009 clip\nsource {} {} {}\n", s.name, if s.character.is_empty() { "-" } else { &s.character }, s.duration);
    for c in &s.channels {
        out.push_str(&format!("channel {} {} {}\n", c.node, c.attr, curve_to_text(&c.curve)));
    }
    out
}

pub fn source_from_file(text: &str) -> Result<ClipSource, String> {
    let mut src = ClipSource::default();
    let mut seen = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(r) = line.strip_prefix("source ") {
            let f: Vec<&str> = r.split_whitespace().collect();
            if f.len() < 3 {
                return Err("Bad source line in the clip file".to_string());
            }
            src.name = f[0].to_string();
            src.character = if f[1] == "-" { String::new() } else { f[1].to_string() };
            src.duration = f[2].parse().map_err(|_| "Bad duration in the clip file".to_string())?;
            seen = true;
        } else if let Some(r) = line.strip_prefix("channel ") {
            let mut it = r.splitn(3, ' ');
            let (n, a, c) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
            src.channels.push(SrcChannel { node: n.to_string(), attr: a.to_string(), curve: curve_from_text(c) });
        }
    }
    if !seen {
        return Err("This is not an Aztec clip file".to_string());
    }
    Ok(src)
}

/// Names used by lookup tables in tests and commands.
pub fn index_of<'a>(list: &'a [Clip], name: &str) -> Option<(usize, &'a Clip)> {
    list.iter().enumerate().find(|(_, c)| c.name == name)
}

#[allow(dead_code)]
fn _unused(_: HashMap<u8, u8>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(a: f32, b: f32, dur: f32) -> Curve {
        let mut c = Curve::new();
        c.set_key(0.0, a, Interp::Linear);
        c.set_key(dur, b, Interp::Linear);
        c
    }

    fn trax_with_two() -> Trax {
        let mut t = Trax::default();
        for (n, a, b) in [("walkSource", 0.0, 10.0), ("pickSource", 0.0, 4.0)] {
            t.sources.push(ClipSource { name: n.into(), character: "c".into(), duration: 10.0, channels: vec![SrcChannel { node: "m".into(), attr: "translateX".into(), curve: ramp(a, b, 10.0) }] });
        }
        t.clips.push(Clip::new("walk", "walkSource", "c", 1.0, 10.0));
        t.clips.push(Clip::new("pick", "pickSource", "c", 11.0, 10.0));
        t
    }

    fn val(t: &Trax, time: f32) -> Option<f32> {
        t.evaluate(Some("c"), time, &|_, _| -99.0).first().map(|x| x.2)
    }

    #[test]
    fn clips_play_their_source_at_their_start() {
        let t = trax_with_two();
        assert_eq!(val(&t, 1.0), Some(0.0));
        assert!((val(&t, 6.0).unwrap() - 5.0).abs() < 1e-3);
        assert_eq!(val(&t, 0.0), Some(0.0), "before the first clip the first pose holds");
        assert!((val(&t, 16.0).unwrap() - 2.0).abs() < 1e-3, "{:?}", val(&t, 16.0));
    }

    #[test]
    fn absolute_clips_jump_and_relative_clips_continue() {
        let mut t = trax_with_two();
        // at the end of walk (11) the value is 10; pick starts at 0 when absolute
        assert!((val(&t, 11.0).unwrap() - 0.0).abs() < 1e-3);
        t.clip_mut("pick").unwrap().relative = true;
        assert!((val(&t, 11.0).unwrap() - 10.0).abs() < 1e-3, "{:?}", val(&t, 11.0));
        assert!((val(&t, 21.0).unwrap() - 14.0).abs() < 1e-3);
    }

    #[test]
    fn scale_cycle_and_time_warp() {
        let mut t = trax_with_two();
        t.clips.truncate(1);
        t.clip_mut("walk").unwrap().scale = 2.0;
        assert!((val(&t, 11.0).unwrap() - 5.0).abs() < 1e-3, "half speed");
        t.clip_mut("walk").unwrap().scale = 1.0;
        t.clip_mut("walk").unwrap().cycles = 2.0;
        assert!((val(&t, 16.0).unwrap() - 5.0).abs() < 1e-3);
        assert!((val(&t, 21.0).unwrap() - 0.0).abs() < 1e-3 || (val(&t, 20.9).unwrap() - 9.9).abs() < 0.2, "the second cycle starts again");
        t.clip_mut("walk").unwrap().cycle_offset = true;
        assert!((val(&t, 16.0).unwrap() - 15.0).abs() < 1e-3 && (val(&t, 20.5).unwrap() - 19.5).abs() < 0.2, "{:?}", val(&t, 20.5));
        // a time warp that holds the first half longer
        let c = t.clip_mut("walk").unwrap();
        c.cycles = 1.0;
        c.cycle_offset = false;
        let mut w = Curve::new();
        w.set_key(0.0, 0.0, Interp::Linear);
        w.set_key(10.0, 5.0, Interp::Linear);
        c.warp = Some(w);
        assert!((val(&t, 6.0).unwrap() - 2.5).abs() < 1e-3, "{:?}", val(&t, 6.0));
    }

    #[test]
    fn blends_crossfade_across_the_overlap() {
        let mut t = trax_with_two();
        t.clip_mut("pick").unwrap().start = 6.0;
        // overlap 6..11: without a blend the later clip wins
        assert!((val(&t, 8.5).unwrap() - 1.0).abs() < 1e-3, "{:?}", val(&t, 8.5));
        t.blends.push(Blend { a: "walk".into(), b: "pick".into(), shape: 0 });
        // at 8.5: walk 7.5, pick 1.0, halfway: 4.25
        assert!((val(&t, 8.5).unwrap() - 4.25).abs() < 0.05, "{:?}", val(&t, 8.5));
        assert!((val(&t, 6.0).unwrap() - 5.0).abs() < 1e-3 && (val(&t, 11.0).unwrap() - 2.0).abs() < 0.05, "{:?}", val(&t, 11.0));
        assert!(blend_weight(3, 0.25) < 0.25 && blend_weight(3, 0.75) > 0.75);
    }

    #[test]
    fn weight_and_tracks() {
        let mut t = trax_with_two();
        t.clips.truncate(1);
        t.clip_mut("walk").unwrap().weight = 0.5;
        let v = t.evaluate(Some("c"), 6.0, &|_, _| 100.0)[0].2;
        assert!((v - 52.5).abs() < 1e-3, "{}", v);
        let mut c = Clip::new("top", "pickSource", "c", 1.0, 10.0);
        c.track = 1;
        t.clips.push(c);
        t.clip_mut("walk").unwrap().weight = 1.0;
        let v = t.evaluate(Some("c"), 6.0, &|_, _| 0.0)[0].2;
        assert!((v - 2.0).abs() < 1e-3, "the upper track wins: {}", v);
    }

    #[test]
    fn files_round_trip() {
        let t = trax_with_two();
        let text = source_to_file(&t.sources[0]);
        let back = source_from_file(&text).unwrap();
        assert_eq!(back.name, "walkSource");
        assert_eq!(back.channels.len(), 1);
        assert!((back.channels[0].curve.eval(5.0).unwrap() - 5.0).abs() < 1e-3);
        assert!(source_from_file("hello").is_err());
    }
}
