//! Paint Effects nodes: brushes and strokes.

use super::graph::{NodeId, Scene};
use super::nodes::{NodeData, StrokeData, Zone};
use crate::geo::paintfx::{self, Params};
use crate::math::Vec3;
use std::hash::{Hash, Hasher};

const MAX_TUBES: usize = 6000;

impl Scene {
    pub fn brush_params(&self, brush: NodeId) -> Params {
        let vals = match self.nodes.get(brush) {
            Some(n) => n.plugs.iter().map(|p| (p.name.clone(), p.value)).collect(),
            None => Vec::new(),
        };
        Params::new(vals)
    }

    /// The brush values as (name, value) pairs.
    pub fn brush_values(&self, brush: NodeId) -> Vec<(String, f32)> {
        self.nodes.get(brush).map(|n| n.plugs.iter().filter(|p| !p.custom).map(|p| (p.name.clone(), p.value)).collect()).unwrap_or_default()
    }

    pub fn set_brush_values(&mut self, brush: NodeId, vals: &[(String, f32)]) {
        for (k, v) in vals {
            let _ = self.set_attr(brush, k, *v);
        }
    }

    /// Create a brush node; `preset` fills its settings.
    pub fn create_brush(&mut self, preset: Option<&str>) -> Result<NodeId, String> {
        let vals = match preset {
            Some(p) => paintfx::preset_values(p).ok_or_else(|| format!("No brush preset named '{}'", p))?,
            None => paintfx::BRUSH_DEFAULTS.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        };
        let id = self.create_node(NodeData::Brush(Box::new(super::nodes::BrushData { preset: preset.unwrap_or("").to_string() })), preset.unwrap_or("brush"), None, preset.is_none());
        self.set_brush_values(id, &vals);
        Ok(id)
    }

    pub fn brush_ids(&self) -> Vec<NodeId> {
        self.ids_where(|d| matches!(d, NodeData::Brush(_)))
    }

    /// Create a stroke along `path` (world positions with surface normals).
    pub fn create_stroke(&mut self, brush: NodeId, path: Vec<(Vec3, Vec3)>, seed: u32) -> NodeId {
        let t = self.create_node(NodeData::Transform, "stroke", None, true);
        let name = self.name_of(t);
        let md = super::history::mesh_data_empty();
        let shape = self.create_node(NodeData::Mesh(md), &format!("{}Shape", name), Some(t), false);
        let mats = [
            Some(self.create_material(Some(&format!("{}Stem", name)))),
            Some(self.create_material(Some(&format!("{}Leaf", name)))),
            Some(self.create_material(Some(&format!("{}Flower", name)))),
        ];
        let brush_name = self.name_of(brush);
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            md.prim = "custom".into();
            md.history.clear();
            md.material = mats[0];
            md.zones = vec![Zone { material: mats[1], uv: Default::default() }, Zone { material: mats[2], uv: Default::default() }];
            md.stroke = Some(Box::new(StrokeData { brush: Some(brush), brush_name, path, seed, key: 0, mats, mat_names: Vec::new() }));
        }
        if let Some(n) = self.nodes.get_mut(shape) {
            for (k, v) in [("displayPercent", 100.0), ("drawAsMesh", 1.0), ("surfaceOffset", 0.0)] {
                let mut p = super::plugs::Plug::new(k, v);
                p.custom = true;
                p.keyable = true;
                n.plugs.push(p);
            }
        }
        self.flush_strokes();
        shape
    }

    pub fn stroke_shapes(&self) -> Vec<NodeId> {
        self.ids_where(|d| matches!(d, NodeData::Mesh(md) if md.stroke.is_some()))
    }

    fn stroke_key(&self, shape: NodeId) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(n) = self.nodes.get(shape) {
            if let NodeData::Mesh(md) = &n.data {
                if let Some(st) = &md.stroke {
                    st.seed.hash(&mut h);
                    for (p, nrm) in &st.path {
                        for v in [p.x, p.y, p.z, nrm.x, nrm.y, nrm.z] {
                            v.to_bits().hash(&mut h);
                        }
                    }
                    if let Some(b) = st.brush.and_then(|b| self.nodes.get(b)) {
                        for p in &b.plugs {
                            p.value.to_bits().hash(&mut h);
                        }
                    }
                }
            }
            for k in ["displayPercent", "surfaceOffset"] {
                n.plug(k).map(|p| p.value.to_bits()).hash(&mut h);
            }
        }
        h.finish()
    }

    /// Grow the geometry of strokes whose path, brush or display settings changed.
    pub fn flush_strokes(&mut self) {
        for shape in self.stroke_shapes() {
            // resolve names read from a file
            let (bn, mn) = match self.nodes.get(shape).map(|n| &n.data) {
                Some(NodeData::Mesh(md)) => match &md.stroke {
                    Some(st) if (st.brush.is_none() && !st.brush_name.is_empty()) || !st.mat_names.is_empty() => (st.brush_name.clone(), st.mat_names.clone()),
                    _ => (String::new(), Vec::new()),
                },
                _ => (String::new(), Vec::new()),
            };
            if !bn.is_empty() || !mn.is_empty() {
                let b = self.find(&bn);
                let ms: Vec<Option<NodeId>> = mn.iter().map(|m| self.find(m)).collect();
                if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
                    if let Some(st) = md.stroke.as_mut() {
                        if st.brush.is_none() {
                            st.brush = b;
                        }
                        for (i, m) in ms.into_iter().enumerate().take(3) {
                            st.mats[i] = m;
                        }
                        st.mat_names.clear();
                    }
                }
            }
            let key = self.stroke_key(shape);
            let (brush, path, seed, mats) = match self.nodes.get(shape).map(|n| &n.data) {
                Some(NodeData::Mesh(md)) => match &md.stroke {
                    Some(st) if st.key != key => (st.brush, st.path.clone(), st.seed, st.mats),
                    _ => continue,
                },
                _ => continue,
            };
            let percent = self.nodes.get(shape).and_then(|n| n.plug("displayPercent")).map(|p| p.value).unwrap_or(100.0);
            let offset = self.nodes.get(shape).and_then(|n| n.plug("surfaceOffset")).map(|p| p.value).unwrap_or(0.0);
            let params = match brush.filter(|b| self.nodes.get(*b).is_some()) {
                Some(b) => self.brush_params(b),
                None => Params::new(Vec::new()),
            };
            let lifted: Vec<(Vec3, Vec3)> = path.iter().map(|(p, n)| (*p + *n * offset, *n)).collect();
            let growth = paintfx::grow(&params, &lifted, seed, percent, MAX_TUBES);
            let (mesh, zone) = paintfx::build_mesh(&params, &growth);
            if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
                md.mesh = mesh;
                md.face_zone = zone;
                md.material = mats[0];
                md.zones = vec![Zone { material: mats[1], uv: Default::default() }, Zone { material: mats[2], uv: Default::default() }];
                md.cache.version += 1;
                if let Some(st) = md.stroke.as_mut() {
                    st.key = key;
                }
            }
            let cols = paintfx::colors(&params);
            for (m, c) in mats.iter().zip(cols) {
                if let Some(m) = m {
                    for (k, v) in [("colorR", c.x), ("colorG", c.y), ("colorB", c.z)] {
                        let _ = self.set_attr(*m, k, v);
                    }
                }
            }
        }
    }

    /// Convert strokes to polygons: a new mesh with the stroke's geometry and no link to the brush.
    pub fn stroke_to_poly(&mut self, shape: NodeId) -> Result<NodeId, String> {
        self.flush_strokes();
        let (mesh, face_zone, zones, mat) = match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) if md.stroke.is_some() => (md.mesh.clone(), md.face_zone.clone(), md.zones.clone(), md.material),
            _ => return Err("Select a Paint Effects stroke".to_string()),
        };
        let t = self.create_node(NodeData::Transform, "strokePoly", None, true);
        let name = self.name_of(t);
        let mut md = super::history::mesh_data_empty();
        md.mesh = mesh;
        md.prim = "custom".into();
        md.face_zone = face_zone;
        md.zones = zones;
        md.material = mat;
        self.create_node(NodeData::Mesh(md), &format!("{}Shape", name), Some(t), false);
        Ok(t)
    }
}
