//! Character sets and the Distance Tool.

use super::graph::{NodeId, Scene};
use super::nodes::NodeData;
use crate::anim::Interp;

/// Attributes a character set takes from an object by default.
const CHAR_ATTRS: [&str; 10] = ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ", "visibility"];

impl Scene {
    /// The keyable channels of a node a character set would take: transform channels and added attributes.
    pub fn default_character_channels(&self, id: NodeId) -> Vec<(NodeId, String)> {
        let Some(n) = self.nodes.get(id) else { return Vec::new() };
        n.plugs
            .iter()
            .filter(|p| p.keyable && !p.readonly && p.source.is_none() && (CHAR_ATTRS.contains(&p.name.as_str()) || p.custom))
            .map(|p| (id, p.name.clone()))
            .collect()
    }

    pub fn create_character(&mut self, name: &str, channels: Vec<(NodeId, String)>) -> Result<NodeId, String> {
        if channels.is_empty() {
            return Err("There is nothing to put in the character set: select objects or name node.attr channels".to_string());
        }
        let id = self.create_node(NodeData::Character(Box::new(super::nodes::CharacterData { channels: Vec::new(), channel_names: Vec::new() })), name, None, false);
        if let Some(NodeData::Character(c)) = self.nodes.get_mut(id).map(|n| &mut n.data) {
            c.channels = channels;
        }
        Ok(id)
    }

    /// Key every channel of the character set at the current time; returns the number of channels keyed.
    pub fn key_character(&mut self, ch: NodeId, interp: Interp, layer: usize) -> usize {
        let channels: Vec<(NodeId, String)> = match self.nodes.get(ch).map(|n| &n.data) {
            Some(NodeData::Character(c)) => c.channels.clone(),
            _ => return 0,
        };
        let mut n = 0;
        for (id, a) in channels {
            if self.nodes.get(id).is_some() && self.set_keyframe_noeval(id, &a, None, None, interp, layer).is_ok() {
                n += 1;
            }
        }
        self.evaluate_anim();
        n
    }

    /// Update the `distance` channel of every distance dimension from its locators.
    pub fn solve_distances(&mut self) {
        let ids = self.ids_where(|d| matches!(d, NodeData::Distance(_)));
        for id in ids {
            let (a, b) = match self.nodes.get(id).map(|n| &n.data) {
                Some(NodeData::Distance(d)) => match (d.start, d.end) {
                    (Some(a), Some(b)) => (a, b),
                    _ => continue,
                },
                _ => continue,
            };
            if self.nodes.get(a).is_none() || self.nodes.get(b).is_none() {
                continue;
            }
            let (pa, pb) = (self.world_matrix(a).translation_part(), self.world_matrix(b).translation_part());
            self.set_eval(id, "distance", pa.distance(pb));
        }
    }
}
