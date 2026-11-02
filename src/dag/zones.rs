//! Texture zones: assigning materials and projections to groups of faces.

use super::graph::{NodeId, Scene};
use super::nodes::{MeshData, NodeData, Zone};
use crate::geo::edit::UvMap;

impl Scene {
    /// The projection stored on the shape itself (the base zone's).
    pub fn shape_uv_map(&mut self, shape: NodeId) -> UvMap {
        let g = |s: &mut Scene, a: &str, d: f32| s.get_attr(shape, a).unwrap_or(d);
        UvMap {
            mode: g(self, "uvMode", 0.0) as u8,
            tile_u: g(self, "uvTilingU", 1.0),
            tile_v: g(self, "uvTilingV", 1.0),
            off_u: g(self, "uvOffsetU", 0.0),
            off_v: g(self, "uvOffsetV", 0.0),
            rot_deg: g(self, "uvRotate", 0.0),
        }
    }

    pub fn set_shape_uv_map(&mut self, shape: NodeId, m: &UvMap) -> Result<(), String> {
        for (a, v) in [("uvMode", m.mode as f32), ("uvTilingU", m.tile_u), ("uvTilingV", m.tile_v), ("uvOffsetU", m.off_u), ("uvOffsetV", m.off_v), ("uvRotate", m.rot_deg)] {
            self.set_attr(shape, a, v)?;
        }
        Ok(())
    }

    fn mesh_data(&self, shape: NodeId) -> Result<&MeshData, String> {
        match self.nodes.get(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => Ok(md),
            _ => Err("Not a mesh".to_string()),
        }
    }

    /// Zone of every face (a vector of zeros when the mesh has no zones).
    pub fn face_zones(&self, shape: NodeId) -> Result<Vec<u16>, String> {
        let md = self.mesh_data(shape)?;
        Ok(if md.face_zone.len() == md.mesh.faces.len() { md.face_zone.clone() } else { vec![0; md.mesh.faces.len()] })
    }

    /// Material and projection of zone `z` (0 is the base zone).
    pub fn zone_info(&mut self, shape: NodeId, z: u16) -> Result<(Option<NodeId>, UvMap), String> {
        if z == 0 {
            let map = self.shape_uv_map(shape);
            return Ok((self.mesh_data(shape)?.material, map));
        }
        let md = self.mesh_data(shape)?;
        let zone = md.zones.get(z as usize - 1).ok_or("No such texture zone")?;
        Ok((zone.material.or(md.material), zone.uv))
    }

    /// Give `faces` a new material and/or projection. Each face keeps whatever it is not changed in:
    /// `material` of `None` keeps the face's material, `map` of `None` keeps its projection.
    pub fn assign_faces(&mut self, shape: NodeId, faces: &[u32], material: Option<NodeId>, map: Option<UvMap>) -> Result<usize, String> {
        self.assign_faces_with(shape, faces, |m, uv| (material.or(m), map.unwrap_or(uv)))
    }

    /// Like `assign_faces` with a function computing each face's new (material, projection) from its old one.
    pub fn assign_faces_with<F: Fn(Option<NodeId>, UvMap) -> (Option<NodeId>, UvMap)>(&mut self, shape: NodeId, faces: &[u32], f: F) -> Result<usize, String> {
        if let Some(m) = self.mesh_data(shape).ok().and_then(|md| md.material) {
            let _ = m;
        }
        let nfaces = self.mesh_data(shape)?.mesh.faces.len();
        if faces.is_empty() {
            return Err("Select faces first".to_string());
        }
        let mut fz = self.face_zones(shape)?;
        // current (material, projection) of every zone in use
        let mut cache: std::collections::HashMap<u16, (Option<NodeId>, UvMap)> = std::collections::HashMap::new();
        let mut changed = 0;
        let mut new_zones: Vec<Zone> = self.mesh_data(shape)?.zones.clone();
        let base_mat = self.mesh_data(shape)?.material;
        let base_map = self.shape_uv_map(shape);
        for &fi in faces {
            if fi as usize >= nfaces {
                continue;
            }
            let z = fz[fi as usize];
            let (m, uv) = *cache.entry(z).or_insert_with(|| {
                if z == 0 {
                    (base_mat, base_map)
                } else {
                    let zone = &new_zones[z as usize - 1];
                    (zone.material.or(base_mat), zone.uv)
                }
            });
            let (nm, nuv) = f(m, uv);
            // the base zone keeps its own projection attributes; everything else is a numbered zone
            let target = if nm == base_mat && nuv == base_map {
                0
            } else {
                let zone = Zone { material: nm, uv: nuv };
                match new_zones.iter().position(|e| *e == zone) {
                    Some(i) => i as u16 + 1,
                    None => {
                        if new_zones.len() >= 255 {
                            return Err("A mesh can have at most 255 texture zones".to_string());
                        }
                        new_zones.push(zone);
                        new_zones.len() as u16
                    }
                }
            };
            if fz[fi as usize] != target {
                fz[fi as usize] = target;
                changed += 1;
            }
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            md.zones = new_zones;
            md.face_zone = fz;
            compact_zones(md);
            md.sync_zones();
            md.touch();
        }
        Ok(changed)
    }

    /// Drop every texture zone: the whole mesh uses the shape's material and projection again.
    pub fn clear_zones(&mut self, shape: NodeId) {
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            md.zones.clear();
            md.face_zone.clear();
            md.touch();
        }
    }

    /// Edit the material and projection of a whole zone (all its faces).
    pub fn set_zone(&mut self, shape: NodeId, z: u16, material: Option<NodeId>, map: Option<UvMap>) -> Result<(), String> {
        if z == 0 {
            if let Some(m) = material {
                if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
                    md.material = Some(m);
                }
            }
            if let Some(m) = map {
                self.set_shape_uv_map(shape, &m)?;
            }
            return Ok(());
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(shape).map(|n| &mut n.data) {
            let zone = md.zones.get_mut(z as usize - 1).ok_or("No such texture zone")?;
            if let Some(m) = material {
                zone.material = Some(m);
            }
            if let Some(m) = map {
                zone.uv = m;
            }
            md.touch();
        }
        Ok(())
    }
}

/// Remove zones that no face uses and renumber the rest.
fn compact_zones(md: &mut MeshData) {
    let mut used = vec![false; md.zones.len() + 1];
    for z in &md.face_zone {
        if let Some(u) = used.get_mut(*z as usize) {
            *u = true;
        }
    }
    let mut remap: Vec<u16> = vec![0; md.zones.len() + 1];
    let mut zones = Vec::new();
    for (i, z) in md.zones.iter().enumerate() {
        if used[i + 1] {
            zones.push(z.clone());
            remap[i + 1] = zones.len() as u16;
        }
    }
    for z in md.face_zone.iter_mut() {
        *z = remap.get(*z as usize).copied().unwrap_or(0);
    }
    md.zones = zones;
}
