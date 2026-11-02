//! File > Create Reference: another scene file loaded into this one under a namespace prefix.

use super::graph::{NodeId, Reference, Scene};
use super::serialize::load_into;

/// Nodes every scene file carries that a reference must not duplicate.
const DEFAULT_NODES: [&str; 9] = ["persp", "perspShape", "top", "topShape", "front", "frontShape", "side", "sideShape", "lambert1"];

impl Scene {
    pub fn reference_prefix(namespace: &str) -> String {
        format!("{}_", namespace)
    }

    /// The nodes that belong to a referenced file (their names carry the namespace prefix).
    pub fn reference_nodes(&self, namespace: &str) -> Vec<NodeId> {
        let prefix = Scene::reference_prefix(namespace);
        self.nodes.iter().filter(|(_, n)| n.name.starts_with(&prefix)).map(|(i, _)| i).collect()
    }

    fn load_reference_file(&mut self, path: &str, namespace: &str) -> Result<usize, String> {
        let real = self.texture_file(path);
        let text = std::fs::read_to_string(&real).map_err(|e| format!("Cannot read {}: {}", real, e))?;
        let before = self.nodes.len();
        self.name_prefix = Scene::reference_prefix(namespace);
        let r = load_into(self, &text, true);
        self.name_prefix.clear();
        r?;
        // the cameras and default material of the file are not wanted twice
        for d in DEFAULT_NODES {
            if let Some(id) = self.names.get(&format!("{}_{}", namespace, d)).copied() {
                let _ = self.delete_node(id);
            }
        }
        self.flush_meshes();
        self.evaluate_frame();
        Ok(self.nodes.len().saturating_sub(before))
    }

    /// Load `path` into the scene with every node named `namespace_name`.
    pub fn create_reference(&mut self, path: &str, namespace: &str) -> Result<usize, String> {
        if namespace.is_empty() || !namespace.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || namespace.chars().next().unwrap().is_ascii_digit() {
            return Err(format!("'{}' is not a valid namespace (letters, digits and underscores, not starting with a digit)", namespace));
        }
        if self.references.iter().any(|r| r.namespace == namespace) {
            return Err(format!("The namespace '{}' is already used by a reference", namespace));
        }
        if self.names.keys().any(|n| n.starts_with(&Scene::reference_prefix(namespace))) {
            return Err(format!("Nodes named {}_... already exist", namespace));
        }
        let stored = self.texture_path_for_storage(path);
        let n = self.load_reference_file(&stored, namespace)?;
        self.references.push(Reference { path: stored, namespace: namespace.to_string(), loaded: true });
        Ok(n)
    }

    /// Unload Reference: its nodes leave the scene; the link stays so it can be loaded again.
    pub fn unload_reference(&mut self, namespace: &str) -> Result<usize, String> {
        let idx = self.references.iter().position(|r| r.namespace == namespace).ok_or_else(|| format!("No reference with the namespace '{}'", namespace))?;
        if !self.references[idx].loaded {
            return Err("The reference is already unloaded".to_string());
        }
        let all = self.reference_nodes(namespace);
        // delete the top nodes (their subtrees go with them), then anything left over such as history nodes
        let tops: Vec<NodeId> = all.iter().copied().filter(|n| self.nodes.get(*n).and_then(|x| x.parent).map(|p| !all.contains(&p)).unwrap_or(true)).collect();
        for t in &tops {
            if self.nodes.get(*t).is_some() {
                let _ = self.delete_node(*t);
            }
        }
        for n in self.reference_nodes(namespace) {
            if self.nodes.get(n).is_some() {
                let _ = self.delete_node(n);
            }
        }
        self.references[idx].loaded = false;
        self.selection.retain(|s| self.nodes.get(*s).is_some());
        Ok(all.len())
    }

    pub fn load_reference(&mut self, namespace: &str) -> Result<usize, String> {
        let idx = self.references.iter().position(|r| r.namespace == namespace).ok_or_else(|| format!("No reference with the namespace '{}'", namespace))?;
        if self.references[idx].loaded {
            return Err("The reference is already loaded".to_string());
        }
        let path = self.references[idx].path.clone();
        let n = self.load_reference_file(&path, namespace)?;
        self.references[idx].loaded = true;
        Ok(n)
    }

    /// Remove Reference: unload it and forget the link.
    pub fn remove_reference(&mut self, namespace: &str) -> Result<(), String> {
        let idx = self.references.iter().position(|r| r.namespace == namespace).ok_or_else(|| format!("No reference with the namespace '{}'", namespace))?;
        if self.references[idx].loaded {
            self.unload_reference(namespace)?;
        }
        self.references.remove(idx);
        Ok(())
    }
}
