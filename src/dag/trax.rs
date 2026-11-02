//! Trax in the scene: clips made from characters, their evaluation, trimming, splitting, merging, redirects and layer helpers.

use super::graph::{NodeId, Scene};
use super::nodes::NodeData;
use crate::anim::keyframes::Curve;
use crate::anim::trax::*;
use crate::anim::Interp;
use crate::math::{Mat4, Quat, Vec3};

/// An override that moves a whole animation: a control node adds its translation (and rotation about `pivot`).
#[derive(Clone, Debug)]
pub struct Redirect {
    pub node: NodeId,
    pub control: NodeId,
    pub translate: bool,
    pub rotate: bool,
    pub pivot: Vec3,
}

impl Scene {
    fn clip_character_name(&self, ch: NodeId) -> String {
        self.name_of(ch)
    }

    /// Evaluate the clips of every character (after the scene's keys), then the redirects.
    pub fn evaluate_trax(&mut self) {
        if self.trax.clips.is_empty() && self.redirects.is_empty() {
            return;
        }
        let t = self.timeline.current;
        let mut driven: Vec<(NodeId, String)> = Vec::new();
        if !self.trax.clips.is_empty() {
            let results = {
                let base = |node: &str, attr: &str| -> f32 { self.find(node).and_then(|n| self.nodes.get(n)).and_then(|n| n.plug(attr)).map(|p| p.value).unwrap_or(0.0) };
                self.trax.evaluate(None, t, &base)
            };
            for (node, attr, v) in results {
                if let Some(id) = self.find(&node) {
                    self.set_eval(id, &attr, v);
                    driven.push((id, attr));
                }
            }
        }
        // redirects: only on animated or clip driven channels so the offset never piles up
        let reds = self.redirects.clone();
        for r in reds {
            if self.nodes.get(r.node).is_none() || self.nodes.get(r.control).is_none() {
                continue;
            }
            let animated = |s: &Scene, a: &str| driven.iter().any(|(n, x)| *n == r.node && x == a) || s.anim.is_animated(r.node, a);
            // a channel that is not animated starts from its authored value, so the offset never piles up
            let get = |s: &Scene, n: NodeId, a: &str| -> f32 {
                let p = s.nodes.get(n).and_then(|x| x.plug(a));
                if n == r.node && !animated(s, a) { p.map(|p| p.rest).unwrap_or(0.0) } else { p.map(|p| p.value).unwrap_or(0.0) }
            };
            let anim_t = Vec3::new(get(self, r.node, "translateX"), get(self, r.node, "translateY"), get(self, r.node, "translateZ"));
            let ctrl_t = Vec3::new(get(self, r.control, "translateX"), get(self, r.control, "translateY"), get(self, r.control, "translateZ"));
            let mut pos = anim_t + ctrl_t;
            if r.rotate {
                let ctrl_r = Vec3::new(get(self, r.control, "rotateX"), get(self, r.control, "rotateY"), get(self, r.control, "rotateZ"));
                let q = Quat::from_euler_deg(ctrl_r);
                pos = r.pivot + q.rotate(anim_t - r.pivot) + ctrl_t;
                let anim_r = Vec3::new(get(self, r.node, "rotateX"), get(self, r.node, "rotateY"), get(self, r.node, "rotateZ"));
                let combined = Mat4::from_quat(q * Quat::from_euler_deg(anim_r)).decompose().1;
                for (a, v) in ["rotateX", "rotateY", "rotateZ"].iter().zip([combined.x, combined.y, combined.z]) {
                    self.set_eval(r.node, a, v);
                }
            }
            if r.translate || r.rotate {
                for (a, v) in ["translateX", "translateY", "translateZ"].iter().zip([pos.x, pos.y, pos.z]) {
                    self.set_eval(r.node, a, v);
                }
            }
        }
    }

    /// Character > Redirect: a locator that offsets the animation of `node`.
    pub fn create_redirect(&mut self, node: NodeId, translate: bool, rotate: bool) -> Result<NodeId, String> {
        if !self.nodes.get(node).map(|n| n.data.is_transform()).unwrap_or(false) {
            return Err("Select the master node of the character".to_string());
        }
        if self.redirects.iter().any(|r| r.node == node) {
            return Err(format!("{} is already redirected", self.name_of(node)));
        }
        let name = if translate && !rotate { "OffsetTranslateControl" } else if rotate && !translate { "OffsetRotateControl" } else { "OffsetControl" };
        let ctrl = self.create_node(NodeData::Transform, name, None, true);
        self.create_node(NodeData::Locator, &format!("{}Shape", self.name_of(ctrl)), Some(ctrl), false);
        let pivot = Vec3::new(
            self.nodes.get(node).and_then(|n| n.plug("translateX")).map(|p| p.value).unwrap_or(0.0),
            self.nodes.get(node).and_then(|n| n.plug("translateY")).map(|p| p.value).unwrap_or(0.0),
            self.nodes.get(node).and_then(|n| n.plug("translateZ")).map(|p| p.value).unwrap_or(0.0),
        );
        self.redirects.push(Redirect { node, control: ctrl, translate, rotate, pivot });
        Ok(ctrl)
    }

    // ------------------------------------------------------------------ clips

    /// Create Animation Clip: the keys of a character's channels become a clip source and a clip.
    pub fn create_clip(&mut self, name: &str, character: NodeId, range: Option<(f32, f32)>, leave_keys: bool, time_warp: bool) -> Result<String, String> {
        let channels: Vec<(NodeId, String)> = match self.nodes.get(character).map(|n| &n.data) {
            Some(NodeData::Character(c)) => c.channels.clone(),
            _ => return Err("Select a character set (or name one with -character)".to_string()),
        };
        let mut chans: Vec<(NodeId, String, Curve)> = Vec::new();
        for (id, a) in &channels {
            if let Some(b) = self.anim.bindings.iter().find(|b| b.node == *id && b.attr == *a && b.layer == 0) {
                chans.push((*id, a.clone(), b.curve.clone()));
            }
        }
        if chans.is_empty() {
            return Err("The character set has no animation to make a clip of".to_string());
        }
        let (a, b) = match range {
            Some(r) => r,
            None => {
                let lo = chans.iter().filter_map(|c| c.2.keys.first().map(|k| k.t)).fold(f32::MAX, f32::min);
                let hi = chans.iter().filter_map(|c| c.2.keys.last().map(|k| k.t)).fold(f32::MIN, f32::max);
                (lo, hi)
            }
        };
        if b <= a {
            return Err("The clip needs a time range with at least one frame".to_string());
        }
        let dur = b - a;
        let src_name = self.trax.unique(&format!("{}Source", name));
        let clip_name = self.trax.unique(name);
        let mut src = ClipSource { name: src_name.clone(), character: self.clip_character_name(character), duration: dur, channels: Vec::new() };
        for (id, attr, curve) in &chans {
            let (va, vb) = (curve.eval(a).unwrap_or(0.0), curve.eval(b).unwrap_or(0.0));
            let mut c = Curve::new();
            c.pre = curve.pre;
            c.post = curve.post;
            for k in &curve.keys {
                if k.t > a + 1e-4 && k.t < b - 1e-4 {
                    let mut kk = k.clone();
                    kk.t -= a;
                    c.keys.push(kk);
                }
            }
            c.set_key(0.0, va, Interp::Bezier);
            c.set_key(dur, vb, Interp::Bezier);
            c.recompute_tangents();
            src.channels.push(SrcChannel { node: self.name_of(*id), attr: attr.clone(), curve: c });
        }
        self.trax.sources.push(src);
        let mut clip = Clip::new(&clip_name, &src_name, &self.clip_character_name(character), a, dur);
        // the first track where it does not overlap another clip of the character
        let mut track = 0;
        while self.trax.track_clips(&clip.character, track).iter().any(|c| c.start < clip.end() - 1e-4 && c.end() > clip.start + 1e-4) {
            track += 1;
        }
        clip.track = track;
        if time_warp {
            clip.warp = Some(Self::linear_warp(&clip));
        }
        self.trax.clips.push(clip);
        if !leave_keys {
            let ids: Vec<(NodeId, String)> = chans.iter().map(|(i, a, _)| (*i, a.clone())).collect();
            self.anim.bindings.retain(|b| !(b.layer == 0 && ids.iter().any(|(i, a)| *i == b.node && *a == b.attr)));
        }
        self.evaluate_anim();
        Ok(clip_name)
    }

    fn linear_warp(c: &Clip) -> Curve {
        let mut w = Curve::new();
        w.set_key(0.0, c.src_in, Interp::Linear);
        w.set_key(c.length(), c.src_out, Interp::Linear);
        w
    }

    pub fn create_time_warp(&mut self, clip: &str) -> Result<(), String> {
        let c = self.trax.clip_mut(clip).ok_or_else(|| format!("No clip named {}", clip))?;
        c.cycles = 1.0;
        c.warp = Some(Self::linear_warp(&c.clone()));
        self.evaluate_anim();
        Ok(())
    }

    /// Put a key on a clip's time warp: at `local` frames from the clip start the source shows `source_frame`.
    pub fn key_time_warp(&mut self, clip: &str, local: f32, source_frame: f32, flat_end: bool) -> Result<(), String> {
        let c = self.trax.clip_mut(clip).ok_or_else(|| format!("No clip named {}", clip))?;
        let w = c.warp.as_mut().ok_or("The clip has no time warp: create one first")?;
        w.set_key(local, source_frame, Interp::Bezier);
        if flat_end {
            if let Some(k) = w.keys.last() {
                let t = k.t;
                w.set_tangents_at(t, Some(crate::anim::keyframes::Tan::Flat), Some(crate::anim::keyframes::Tan::Flat));
            }
        }
        self.evaluate_anim();
        Ok(())
    }

    pub fn delete_clip(&mut self, name: &str) -> Result<(), String> {
        let before = self.trax.clips.len();
        self.trax.clips.retain(|c| c.name != name);
        if self.trax.clips.len() == before {
            return Err(format!("No clip named {}", name));
        }
        self.trax.blends.retain(|b| b.a != name && b.b != name);
        self.evaluate_anim();
        Ok(())
    }

    /// Trim the clip so it begins (`before`) or ends (`!before`) at the scene frame `f`.
    pub fn trim_clip(&mut self, name: &str, f: f32, before: bool) -> Result<(), String> {
        let c = self.trax.clip_mut(name).ok_or_else(|| format!("No clip named {}", name))?;
        if f <= c.start + 1e-3 && before || f >= c.end() - 1e-3 && !before {
            return Err("The frame is outside the clip".to_string());
        }
        if c.cycles != 1.0 {
            // trimming works on one pass of the source
            let (s, _) = c.source_time(f);
            let _ = s;
            c.cycles = 1.0;
        }
        let (s, _) = c.source_time(f);
        if before {
            let shift = f - c.start;
            c.src_in = s;
            c.start = f;
            if let Some(w) = c.warp.as_mut() {
                let v = w.eval(shift).unwrap_or(s);
                w.keys.retain(|k| k.t > shift + 1e-4);
                for k in w.keys.iter_mut() {
                    k.t -= shift;
                }
                w.set_key(0.0, v, Interp::Linear);
                w.recompute_tangents();
            }
        } else {
            c.src_out = s;
            if let Some(w) = c.warp.as_mut() {
                let local = f - c.start;
                let v = w.eval(local).unwrap_or(s);
                w.keys.retain(|k| k.t < local - 1e-4);
                w.set_key(local, v, Interp::Linear);
                w.recompute_tangents();
            }
        }
        self.evaluate_anim();
        Ok(())
    }

    /// Split the clip at scene frame `f`; returns the name of the new second part.
    pub fn split_clip(&mut self, name: &str, f: f32) -> Result<String, String> {
        let orig = self.trax.clip(name).ok_or_else(|| format!("No clip named {}", name))?.clone();
        if f <= orig.start + 1e-3 || f >= orig.end() - 1e-3 {
            return Err("Move to a frame inside the clip to split it".to_string());
        }
        let mut second = orig.clone();
        second.name = self.trax.unique(&format!("{}", name));
        self.trax.clips.push(second.clone());
        // the second part keeps the end, the first the beginning
        self.trim_clip(&second.name, f, true)?;
        self.trim_clip(name, f, false)?;
        // blends that ended on the original now end on the second part
        for b in self.trax.blends.iter_mut() {
            if b.a == name {
                b.a = second.name.clone();
            }
        }
        Ok(second.name)
    }

    /// Create > Blend between two clips (the first one fading into the second).
    pub fn blend_clips(&mut self, a: &str, b: &str, shape: u8) -> Result<(), String> {
        let (ca, cb) = (self.trax.clip(a).ok_or_else(|| format!("No clip named {}", a))?.clone(), self.trax.clip(b).ok_or_else(|| format!("No clip named {}", b))?.clone());
        if ca.character != cb.character || ca.track != cb.track {
            return Err("Blend two clips of one character on the same track".to_string());
        }
        self.trax.blends.retain(|x| !(x.a == a && x.b == b));
        self.trax.blends.push(Blend { a: a.to_string(), b: b.to_string(), shape });
        self.evaluate_anim();
        Ok(())
    }

    /// Edit > Merge: bake the clips into one new clip with a key on every frame.
    pub fn merge_clips(&mut self, names: &[String], new_name: &str) -> Result<String, String> {
        if names.len() < 2 {
            return Err("Select at least two clips to merge".to_string());
        }
        let clips: Vec<Clip> = names.iter().map(|n| self.trax.clip(n).cloned().ok_or_else(|| format!("No clip named {}", n))).collect::<Result<_, _>>()?;
        let character = clips[0].character.clone();
        if clips.iter().any(|c| c.character != character) {
            return Err("The clips belong to different characters".to_string());
        }
        let (a, b) = (clips.iter().map(|c| c.start).fold(f32::MAX, f32::min), clips.iter().map(|c| c.end()).fold(f32::MIN, f32::max));
        let mut only = self.trax.clone();
        only.clips.retain(|c| names.contains(&c.name));
        // sample every frame
        let mut chans: Vec<(String, String, Curve)> = Vec::new();
        let n = (b - a).ceil() as i32;
        for i in 0..=n {
            let t = (a + i as f32).min(b);
            for (node, attr, v) in only.evaluate(Some(&character), t, &|_, _| 0.0) {
                match chans.iter_mut().find(|c| c.0 == node && c.1 == attr) {
                    Some(c) => c.2.set_key(t - a, v, Interp::Bezier),
                    None => {
                        let mut c = Curve::new();
                        c.set_key(t - a, v, Interp::Bezier);
                        chans.push((node, attr, c));
                    }
                }
            }
        }
        let src_name = self.trax.unique(&format!("{}Source", new_name));
        let clip_name = self.trax.unique(new_name);
        self.trax.sources.push(ClipSource { name: src_name.clone(), character: character.clone(), duration: b - a, channels: chans.into_iter().map(|(node, attr, curve)| SrcChannel { node, attr, curve }).collect() });
        self.trax.clips.retain(|c| !names.contains(&c.name));
        self.trax.blends.retain(|bl| !names.contains(&bl.a) && !names.contains(&bl.b));
        let mut clip = Clip::new(&clip_name, &src_name, &character, a, b - a);
        clip.track = 0;
        self.trax.clips.push(clip);
        self.evaluate_anim();
        Ok(clip_name)
    }

    /// Activate Keys: put the keys of a clip into the scene's timeline (the clip stays unless `remove`).
    pub fn activate_clip_keys(&mut self, name: &str, remove: bool) -> Result<usize, String> {
        let clip = self.trax.clip(name).cloned().ok_or_else(|| format!("No clip named {}", name))?;
        let src = self.trax.source(&clip.source).cloned().ok_or("The clip's source is missing")?;
        let mut only = self.trax.clone();
        only.clips.retain(|c| c.name == name);
        let mut times: Vec<f32> = Vec::new();
        if clip.warp.is_some() || clip.cycles != 1.0 {
            let n = clip.length().ceil() as i32;
            times.extend((0..=n).map(|i| (clip.start + i as f32).min(clip.end())));
        } else {
            for ch in &src.channels {
                for k in &ch.curve.keys {
                    if k.t >= clip.src_in - 1e-4 && k.t <= clip.src_out + 1e-4 {
                        times.push(clip.start + (k.t - clip.src_in) * clip.scale);
                    }
                }
            }
            times.push(clip.start);
            times.push(clip.end());
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        let mut n = 0;
        for t in times {
            for (node, attr, v) in only.evaluate(None, t, &|_, _| 0.0) {
                if let Some(id) = self.find(&node) {
                    self.anim.set_key(id, &attr, 0, t, v, Interp::Bezier);
                    n += 1;
                }
            }
        }
        if remove {
            self.delete_clip(name)?;
        } else {
            self.evaluate_anim();
        }
        Ok(n)
    }

    /// Insert a clip of `source` into the Trax Editor (Library > Insert Clip).
    pub fn insert_clip(&mut self, source: &str, start: f32, character: Option<&str>) -> Result<String, String> {
        let src = self.trax.source(source).cloned().ok_or_else(|| format!("No clip source named {}", source))?;
        let chr = character.map(|c| c.to_string()).unwrap_or(src.character.clone());
        let base = source.strip_suffix("Source").unwrap_or(source);
        let name = self.trax.unique(base);
        let mut clip = Clip::new(&name, source, &chr, start, src.duration);
        let mut track = 0;
        while self.trax.track_clips(&clip.character, track).iter().any(|c| c.start < clip.end() - 1e-4 && c.end() > clip.start + 1e-4) {
            track += 1;
        }
        clip.track = track;
        self.trax.clips.push(clip);
        self.evaluate_anim();
        Ok(name)
    }

    // ------------------------------------------------------------------ animation layers

    /// Layers > Add Selected Objects: their keyable channels join the layer.
    pub fn add_to_layer(&mut self, layer: usize, nodes: &[NodeId]) -> Result<usize, String> {
        if layer == 0 || layer >= self.anim.layers.len() {
            return Err("Choose an animation layer other than the base layer".to_string());
        }
        let mut n = 0;
        for id in nodes {
            for (i, a) in self.default_character_channels(*id) {
                let l = &mut self.anim.layers[layer];
                if !l.members.iter().any(|(m, x)| *m == i && *x == a) {
                    l.members.push((i, a));
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// Zero Key Layer: key the layer's channels of `nodes` at their neutral value (0) at the current time.
    pub fn zero_key_layer(&mut self, layer: usize, nodes: &[NodeId]) -> Result<usize, String> {
        if layer == 0 || layer >= self.anim.layers.len() {
            return Err("Zero keys belong on an animation layer, not the base layer".to_string());
        }
        let t = self.timeline.current;
        let members: Vec<(NodeId, String)> = self.anim.layers[layer].members.iter().filter(|(n, _)| nodes.is_empty() || nodes.contains(n)).cloned().collect();
        let bound: Vec<(NodeId, String)> = self.anim.bindings.iter().filter(|b| b.layer == layer && (nodes.is_empty() || nodes.contains(&b.node))).map(|b| (b.node, b.attr.clone())).collect();
        let mut chans = members;
        for b in bound {
            if !chans.contains(&b) {
                chans.push(b);
            }
        }
        if chans.is_empty() {
            return Err("Add the objects to the layer first".to_string());
        }
        for (id, a) in &chans {
            self.anim.set_key(*id, a, layer, t, 0.0, Interp::Bezier);
        }
        self.evaluate_anim();
        Ok(chans.len())
    }

    /// Layers > Merge Layers: bake the layers into the base animation.
    pub fn merge_anim_layers(&mut self, layers: &[usize]) -> Result<usize, String> {
        if layers.iter().any(|l| *l == 0 || *l >= self.anim.layers.len()) {
            return Err("Name animation layers (not the base layer)".to_string());
        }
        let chans: Vec<(NodeId, String)> = {
            let mut v: Vec<(NodeId, String)> = Vec::new();
            for b in self.anim.bindings.iter().filter(|b| layers.contains(&b.layer)) {
                if !v.contains(&(b.node, b.attr.clone())) {
                    v.push((b.node, b.attr.clone()));
                }
            }
            v
        };
        let mut merged = 0;
        for (id, a) in &chans {
            let mut times: Vec<f32> = Vec::new();
            for b in self.anim.bindings.iter().filter(|b| b.node == *id && b.attr == *a && (b.layer == 0 || layers.contains(&b.layer))) {
                times.extend(b.curve.keys.iter().map(|k| k.t));
            }
            // every frame between the first and the last key, so the sum is reproduced closely
            if let (Some(lo), Some(hi)) = (times.iter().cloned().reduce(f32::min), times.iter().cloned().reduce(f32::max)) {
                let mut f = lo.ceil();
                while f <= hi {
                    times.push(f);
                    f += 1.0;
                }
            }
            times.sort_by(|x, y| x.partial_cmp(y).unwrap());
            times.dedup_by(|x, y| (*x - *y).abs() < 1e-3);
            let rest = self.nodes.get(*id).and_then(|n| n.plug(a)).map(|p| p.rest).unwrap_or(0.0);
            // value of the base plus the merged layers at each time (the other layers stay)
            let vals: Vec<(f32, f32)> = times
                .iter()
                .map(|t| {
                    let mut base: Option<f32> = None;
                    let mut add = 0.0;
                    for b in self.anim.bindings.iter().filter(|b| b.node == *id && b.attr == *a) {
                        let l = &self.anim.layers[b.layer];
                        if let Some(v) = b.curve.eval(*t) {
                            if b.layer == 0 {
                                base = Some(v);
                            } else if layers.contains(&b.layer) {
                                add += v * l.weight_at(*t);
                            }
                        }
                    }
                    (*t, base.unwrap_or(rest) + add)
                })
                .collect();
            for (t, v) in vals {
                self.anim.set_key(*id, a, 0, t, v, Interp::Bezier);
            }
            merged += 1;
        }
        // remove merged layers (highest index first)
        let mut order: Vec<usize> = layers.to_vec();
        order.sort_unstable_by(|a, b| b.cmp(a));
        for l in order {
            self.anim.bindings.retain(|b| b.layer != l);
            for b in self.anim.bindings.iter_mut() {
                if b.layer > l {
                    b.layer -= 1;
                }
            }
            self.anim.layers.remove(l);
            if self.anim.active_layer >= self.anim.layers.len() || self.anim.active_layer == l {
                self.anim.active_layer = 0;
            } else if self.anim.active_layer > l {
                self.anim.active_layer -= 1;
            }
        }
        self.evaluate_anim();
        Ok(merged)
    }
}
