use super::keyframes::{Curve, Interp, Tan};
use crate::dag::graph::NodeId;

/// Layer 0 is always the base layer. Every other layer is additive with a 0..1 weight slider.
#[derive(Clone, Debug)]
pub struct AnimLayer {
    pub name: String,
    pub weight: f32,
    pub mute: bool,
    /// Channels added to the layer (Layers > Add Selected Objects).
    pub members: Vec<(NodeId, String)>,
    /// Keys on the weight slider.
    pub weight_curve: Curve,
}

impl AnimLayer {
    pub fn new(name: &str) -> AnimLayer {
        AnimLayer { name: name.to_string(), weight: 1.0, mute: false, members: Vec::new(), weight_curve: Curve::new() }
    }

    /// The weight at `time`: the keyed slider when it has keys, else the plain value.
    pub fn weight_at(&self, time: f32) -> f32 {
        self.weight_curve.eval(time).map(|w| w.clamp(0.0, 1.0)).unwrap_or(self.weight)
    }
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub node: NodeId,
    pub attr: String,
    pub layer: usize,
    pub curve: Curve,
}

#[derive(Clone, Debug)]
pub struct AnimSystem {
    pub layers: Vec<AnimLayer>,
    pub bindings: Vec<Binding>,
    /// Layer that receives new keyframes by default.
    pub active_layer: usize,
    /// In and out tangent types of newly created keys (Preferences > Animation > Tangents).
    pub default_tans: (Tan, Tan),
}

impl Default for AnimSystem {
    fn default() -> Self {
        AnimSystem::new()
    }
}

impl AnimSystem {
    pub fn new() -> AnimSystem {
        AnimSystem {
            layers: vec![AnimLayer::new("BaseAnimation")],
            bindings: Vec::new(),
            active_layer: 0,
            default_tans: (Tan::Clamped, Tan::Clamped),
        }
    }

    pub fn find_layer(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == name)
    }

    pub fn add_layer(&mut self, name: &str) -> Result<usize, String> {
        if self.find_layer(name).is_some() {
            return Err(format!("Animation layer '{}' already exists", name));
        }
        self.layers.push(AnimLayer::new(name));
        Ok(self.layers.len() - 1)
    }

    pub fn set_weight(&mut self, layer: usize, w: f32) {
        if let Some(l) = self.layers.get_mut(layer) {
            l.weight = w.clamp(0.0, 1.0);
        }
    }

    pub fn set_mute(&mut self, layer: usize, m: bool) {
        if let Some(l) = self.layers.get_mut(layer) {
            l.mute = m;
        }
    }

    pub fn set_key(&mut self, node: NodeId, attr: &str, layer: usize, t: f32, v: f32, interp: Interp) {
        if let Some(b) = self
            .bindings
            .iter_mut()
            .find(|b| b.node == node && b.attr == attr && b.layer == layer)
        {
            b.curve.set_key_with(t, v, interp, Some(self.default_tans));
            return;
        }
        let mut curve = Curve::new();
        curve.set_key_with(t, v, interp, Some(self.default_tans));
        self.bindings.push(Binding { node, attr: attr.to_string(), layer, curve });
    }

    pub fn remove_key(&mut self, node: NodeId, attr: &str, layer: usize, t: f32) -> bool {
        let mut removed = false;
        if let Some(b) = self
            .bindings
            .iter_mut()
            .find(|b| b.node == node && b.attr == attr && b.layer == layer)
        {
            removed = b.curve.remove_key(t);
        }
        self.bindings.retain(|b| !b.curve.is_empty());
        removed
    }

    /// Move every key at one of `times` by `delta` frames (for the given nodes). Keys that land on an
    /// existing key replace it. Returns the number of keys moved.
    pub fn shift_keys(&mut self, only: &[NodeId], times: &[f32], delta: f32) -> usize {
        let mut moved = 0;
        for b in self.bindings.iter_mut() {
            if !only.contains(&b.node) {
                continue;
            }
            let hit: Vec<usize> = (0..b.curve.keys.len())
                .filter(|&i| times.iter().any(|t| (b.curve.keys[i].t - t).abs() < 1e-3))
                .collect();
            if hit.is_empty() {
                continue;
            }
            let mut moving: Vec<_> = hit.iter().map(|&i| b.curve.keys[i].clone()).collect();
            let mut i = 0usize;
            b.curve.keys.retain(|_| {
                let keep = !hit.contains(&i);
                i += 1;
                keep
            });
            for k in moving.iter_mut() {
                k.t = (k.t + delta).max(0.0);
                b.curve.keys.retain(|o| (o.t - k.t).abs() > 1e-3);
                b.curve.keys.push(k.clone());
                moved += 1;
            }
            b.curve.keys.sort_by(|a, c| a.t.partial_cmp(&c.t).unwrap());
            b.curve.recompute_tangents();
        }
        moved
    }

    pub fn remove_node(&mut self, node: NodeId) {
        self.bindings.retain(|b| b.node != node);
    }

    pub fn is_animated(&self, node: NodeId, attr: &str) -> bool {
        self.bindings.iter().any(|b| b.node == node && b.attr == attr)
    }

    /// Sorted unique key times across all curves, or only those of the given nodes.
    pub fn key_times(&self, only: Option<&[NodeId]>) -> Vec<f32> {
        let mut out: Vec<f32> = Vec::new();
        for b in &self.bindings {
            if let Some(list) = only {
                if !list.contains(&b.node) {
                    continue;
                }
            }
            for k in &b.curve.keys {
                if !out.iter().any(|t| (t - k.t).abs() < 1e-4) {
                    out.push(k.t);
                }
            }
        }
        out.sort_by(|a, b| a.partial_cmp(b).unwrap());
        out
    }

    /// Evaluate every animated attribute at `time`.
    /// result = (base curve value, or the rest value if the base has no curve) + sum(weight * additive curves).
    pub fn evaluate<F: Fn(NodeId, &str) -> f32>(&self, time: f32, rest: F) -> Vec<(NodeId, String, f32)> {
        let mut targets: Vec<(NodeId, String)> = Vec::new();
        for b in &self.bindings {
            if !targets.iter().any(|(n, a)| *n == b.node && *a == b.attr) {
                targets.push((b.node, b.attr.clone()));
            }
        }
        let mut out = Vec::with_capacity(targets.len());
        for (node, attr) in targets {
            let mut base: Option<f32> = None;
            let mut add = 0.0f32;
            for b in self.bindings.iter().filter(|b| b.node == node && b.attr == attr) {
                let layer = &self.layers[b.layer.min(self.layers.len() - 1)];
                if layer.mute {
                    continue;
                }
                if let Some(v) = b.curve.eval(time) {
                    if b.layer == 0 {
                        base = Some(v);
                    } else {
                        add += v * layer.weight_at(time);
                    }
                }
            }
            let value = base.unwrap_or_else(|| rest(node, &attr)) + add;
            out.push((node, attr, value));
        }
        out
    }
}
