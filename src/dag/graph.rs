use super::nodes::*;
use super::plugs::*;
use crate::anim::{AnimSystem, Interp, Timeline};
use crate::geo::Mesh;
use crate::math::{Mat4, Quat, Vec3};
use std::collections::HashMap;

/// Generational handle into the node arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    pub index: u32,
    pub gen: u32,
}

struct Slot<T> {
    gen: u32,
    value: Option<T>,
}

/// Slot-map arena: stable generational ids, O(1) insert/remove/lookup, freed slots are reused.
pub struct SlotMap<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for SlotMap<T> {
    fn default() -> Self {
        SlotMap::new()
    }
}

impl<T> SlotMap<T> {
    pub fn new() -> SlotMap<T> {
        SlotMap { slots: Vec::new(), free: Vec::new(), len: 0 }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn insert(&mut self, v: T) -> NodeId {
        self.len += 1;
        if let Some(i) = self.free.pop() {
            let s = &mut self.slots[i as usize];
            s.value = Some(v);
            NodeId { index: i, gen: s.gen }
        } else {
            self.slots.push(Slot { gen: 0, value: Some(v) });
            NodeId { index: (self.slots.len() - 1) as u32, gen: 0 }
        }
    }
    pub fn remove(&mut self, id: NodeId) -> Option<T> {
        let s = self.slots.get_mut(id.index as usize)?;
        if s.gen != id.gen {
            return None;
        }
        let v = s.value.take()?;
        s.gen += 1;
        self.free.push(id.index);
        self.len -= 1;
        Some(v)
    }
    pub fn get(&self, id: NodeId) -> Option<&T> {
        let s = self.slots.get(id.index as usize)?;
        if s.gen == id.gen {
            s.value.as_ref()
        } else {
            None
        }
    }
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
        let s = self.slots.get_mut(id.index as usize)?;
        if s.gen == id.gen {
            s.value.as_mut()
        } else {
            None
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &T)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.value.as_ref().map(|v| (NodeId { index: i as u32, gen: s.gen }, v)))
    }
}

/// Which kinds of objects the viewports draw (Maya: the viewport Show menu) and which transform
/// helpers are displayed (Display > Transform Display). Not saved with the scene.
#[derive(Clone, Debug)]
pub struct ShowFlags {
    pub polygons: bool,
    pub lights: bool,
    pub cameras: bool,
    pub joints: bool,
    pub locators: bool,
    pub emitters: bool,
    pub ik_handles: bool,
    pub particles: bool,
    pub manipulators: bool,
    /// Draw the pivot point of selected objects.
    pub pivots: bool,
    /// Draw a selection handle at the pivot of every object.
    pub handles: bool,
    /// Display > Component Display > Local Rotation Axes of the selected joints.
    pub local_axes: bool,
    /// Shading > X-Ray Joints: joints show through the surfaces.
    pub xray_joints: bool,
    /// Show > Deformers: lattices and flexors are drawn.
    pub deformers: bool,
}

impl Default for ShowFlags {
    fn default() -> ShowFlags {
        ShowFlags { polygons: true, lights: true, cameras: true, joints: true, locators: true, emitters: true, ik_handles: true, particles: true, manipulators: true, pivots: false, handles: false, local_axes: false, xray_joints: true, deformers: true }
    }
}

/// A file referenced into the scene: its nodes carry the namespace prefix `namespace_`.
#[derive(Clone, Debug)]
pub struct Reference {
    pub path: String,
    pub namespace: String,
    pub loaded: bool,
}

/// The dependency graph and scene state.
pub struct Scene {
    /// Set while rendering a final image: Paint Effects strokes always render as meshes.
    pub final_render: bool,
    pub expressions: Vec<super::expr::Expression>,
    pub in_expression: bool,
    /// Non-linear animation: clip sources, clips and blends.
    pub trax: crate::anim::trax::Trax,
    pub redirects: Vec<super::trax::Redirect>,
    /// The live surface curves are drawn on (Modify > Make Live).
    pub live: Option<NodeId>,
    /// Lighting > Use All Lights: when off the views use the default lights.
    pub use_all_lights: bool,
    /// Render layers (the master layer is every object and is not listed).
    pub render_layers: Vec<super::nodes::RenderLayerData>,
    /// Light linking: a listed light only illuminates its objects.
    pub light_links: Vec<(NodeId, Vec<NodeId>)>,
    pub nodes: SlotMap<Node>,
    pub roots: Vec<NodeId>,
    pub(crate) names: HashMap<String, NodeId>,
    pub anim: AnimSystem,
    pub timeline: Timeline,
    pub selection: Vec<NodeId>,
    pub file_path: Option<String>,
    /// Mesh shapes whose construction history needs re-evaluation.
    pub dirty_shapes: Vec<NodeId>,
    /// History nodes that deform meshes with joints (re-evaluated when any transform changes).
    pub skins: Vec<NodeId>,
    /// Free text notes per node (Attribute Editor > Notes).
    pub notes: HashMap<NodeId, String>,
    /// Draw border edges (edges with a face on one side only) so gaps are easy to see. Not saved.
    pub show_borders: bool,
    /// Display > Polygons > Face Normals.
    pub show_normals: bool,
    /// Move tool Reflection: component edits are mirrored across this local axis plane (0 x, 1 y, 2 z).
    pub reflection: Option<usize>,
    /// Display > Animation > Joint Size: multiplies the drawn size of every joint.
    pub joint_scale: f32,
    /// Weights of a joint shown on a skinned mesh (Paint Skin Weights).
    pub skin_vis: Option<(NodeId, NodeId)>,
    /// While a reference loads, every node name gets this prefix (and names are looked up with it first).
    pub name_prefix: String,
    /// Referenced files (File > Create Reference).
    pub references: Vec<Reference>,
    /// Baked images of texture networks, by node and resolution (not saved).
    pub net_cache: HashMap<(NodeId, usize), (u64, std::sync::Arc<crate::render::texture::Texture>)>,
    pub show: ShowFlags,
    /// Project folder (File > Project): relative image paths are looked up inside it.
    pub project_dir: Option<std::path::PathBuf>,
}

impl Default for Scene {
    fn default() -> Self {
        Scene::new()
    }
}

impl Scene {
    /// A fresh scene containing the default `persp` camera.
    pub fn new() -> Scene {
        let mut s = Scene::empty();
        s.add_default_camera();
        s
    }

    pub fn empty() -> Scene {
        Scene {
            final_render: false,
            expressions: Vec::new(),
            in_expression: false,
            trax: Default::default(),
            redirects: Vec::new(),
            live: None,
            use_all_lights: true,
            render_layers: Vec::new(),
            light_links: Vec::new(),
            nodes: SlotMap::new(),
            roots: Vec::new(),
            names: HashMap::new(),
            anim: AnimSystem::new(),
            timeline: Timeline::new(),
            selection: Vec::new(),
            file_path: None,
            dirty_shapes: Vec::new(),
            skins: Vec::new(),
            notes: HashMap::new(),
            show_borders: false,
            show_normals: false,
            reflection: None,
            joint_scale: 1.0,
            skin_vis: None,
            name_prefix: String::new(),
            references: Vec::new(),
            net_cache: HashMap::new(),
            show: ShowFlags::default(),
            project_dir: None,
        }
    }

    pub fn clear_all(&mut self) {
        let path = self.file_path.clone();
        let project = self.project_dir.clone();
        *self = Scene::empty();
        self.file_path = path;
        self.project_dir = project;
    }

    pub fn new_scene(&mut self) {
        self.clear_all();
        self.file_path = None;
        self.add_default_camera();
    }

    pub fn add_default_camera(&mut self) {
        let (t, s) = self.create_camera(false, Some("persp"));
        let _ = self.set_attr(s, "renderable", 1.0);
        self.create_node(NodeData::RenderGlobals(Box::default()), "defaultRenderGlobals", None, false);
        let _ = self.set_attr(t, "translateX", 14.0);
        let _ = self.set_attr(t, "translateY", 10.0);
        let _ = self.set_attr(t, "translateZ", 14.0);
        let _ = self.set_attr(t, "rotateX", -26.8);
        let _ = self.set_attr(t, "rotateY", 45.0);
    }

    // ------------------------------------------------------------------ names and lookup

    /// The file to load for an image path: relative paths are looked up in the project folder.
    pub fn texture_file(&self, path: &str) -> String {
        if path.starts_with("builtin:") || std::path::Path::new(path).is_absolute() {
            return path.to_string();
        }
        match &self.project_dir {
            Some(d) if d.join(path).exists() => d.join(path).to_string_lossy().to_string(),
            _ => path.to_string(),
        }
    }

    /// Store an image path relative to the project when it lies inside it.
    pub fn texture_path_for_storage(&self, path: &str) -> String {
        if let Some(d) = &self.project_dir {
            if let Ok(rel) = std::path::Path::new(path).strip_prefix(d) {
                return rel.to_string_lossy().to_string();
            }
        }
        path.to_string()
    }

    pub fn find(&self, name: &str) -> Option<NodeId> {
        let n = name.trim_start_matches('|');
        let n = n.rsplit('|').next().unwrap_or(n);
        if !self.name_prefix.is_empty() {
            if let Some(id) = self.names.get(&format!("{}{}", self.name_prefix, n)) {
                return Some(*id);
            }
        }
        self.names.get(n).copied()
    }

    pub fn name_of(&self, id: NodeId) -> String {
        self.nodes.get(id).map(|n| n.name.clone()).unwrap_or_else(|| "<deleted>".to_string())
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    fn unique_name(&self, base: &str, force_number: bool) -> String {
        if !force_number && !self.names.contains_key(base) {
            return base.to_string();
        }
        let stem = base.trim_end_matches(|c: char| c.is_ascii_digit());
        let stem = if stem.is_empty() { base } else { stem };
        let mut n = 1;
        loop {
            let c = format!("{}{}", stem, n);
            if !self.names.contains_key(&c) {
                return c;
            }
            n += 1;
        }
    }

    /// Split "node.attr" at the last dot and validate both halves.
    pub fn resolve_plug(&self, path: &str) -> Result<(NodeId, String), String> {
        let idx = path.rfind('.').ok_or_else(|| format!("Invalid attribute path '{}', expected node.attr", path))?;
        let (n, a) = (&path[..idx], &path[idx + 1..]);
        let id = self.find(n).ok_or_else(|| format!("No object matches name: {}", n))?;
        let attr = canonical_attr(a);
        let node = self.nodes.get(id).unwrap();
        if node.plug(&attr).is_none() {
            return Err(format!("Attribute '{}' not found on '{}'", attr, node.name));
        }
        Ok((id, attr))
    }

    // ------------------------------------------------------------------ node creation

    pub fn create_node(&mut self, data: NodeData, name: &str, parent: Option<NodeId>, force_number: bool) -> NodeId {
        let prefixed;
        let name = if self.name_prefix.is_empty() {
            name
        } else {
            prefixed = format!("{}{}", self.name_prefix, name);
            &prefixed
        };
        let nm = self.unique_name(name, force_number);
        let plugs = default_plugs(&data);
        let node = Node { name: nm.clone(), data, parent, children: Vec::new(), plugs, world_cache: None };
        let id = self.nodes.insert(node);
        self.names.insert(nm, id);
        match parent {
            Some(p) => {
                if let Some(pn) = self.nodes.get_mut(p) {
                    pn.children.push(id);
                }
            }
            None => self.roots.push(id),
        }
        if matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::DrivenCurve(_))) {
            // the curve's output follows its input, so dirty flags travel through it
            let (i, o) = (PlugRef { node: id, attr: "input".into() }, PlugRef { node: id, attr: "output".into() });
            if let Some(p) = self.nodes.get_mut(id).and_then(|n| n.plug_mut("input")) {
                p.dests.push(o.clone());
            }
            if let Some(p) = self.nodes.get_mut(id).and_then(|n| n.plug_mut("output")) {
                p.source = Some(i);
                p.dirty = true;
            }
        }
        id
    }

    pub fn create_mesh_object(
        &mut self,
        prim: &str,
        params: Vec<(String, f32)>,
        name: Option<&str>,
    ) -> Result<(NodeId, NodeId), String> {
        build_mesh(prim, &params).ok_or_else(|| format!("Unknown primitive '{}'", prim))?;
        let (base, kind) = match prim {
            "polyCube" => ("pCube", PolyKind::Cube),
            "polySphere" => ("pSphere", PolyKind::Sphere),
            "polyCylinder" => ("pCylinder", PolyKind::Cylinder),
            "polyCone" => ("pCone", PolyKind::Cone),
            "polyTorus" => ("pTorus", PolyKind::Torus),
            _ => ("pPlane", PolyKind::Plane),
        };
        let (tname, force) = match name {
            Some(n) => (n, false),
            None => (base, true),
        };
        let t = self.create_node(NodeData::Transform, tname, None, force);
        let shape_name = format!("{}Shape", self.name_of(t));
        let data = MeshData { cache: Default::default(), history: Vec::new(), mesh: Mesh::new(), prim: "history".to_string(), params: Vec::new(), material: None, ..Default::default() };
        let s = self.create_node(NodeData::Mesh(data), &shape_name, Some(t), false);
        let h = self.new_poly_node(kind, Vec::new(), None, Some(s));
        for (k, v) in &params {
            if let Some(p) = self.nodes.get_mut(h).and_then(|n| n.plug_mut(k)) {
                p.value = *v;
                p.rest = *v;
            }
        }
        if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(s).map(|n| &mut n.data) {
            md.history = vec![h];
        }
        self.mark_poly_dirty(h);
        self.flush_meshes();
        Ok((t, s))
    }

    /// A new transform with a shape that instances `src` (a transform with a mesh): it follows the
    /// original's geometry and material but has its own transform.
    pub fn create_instance(&mut self, src: NodeId) -> Result<NodeId, String> {
        let shapes = self.mesh_shapes_under(src);
        let first = *shapes.first().ok_or("Only mesh objects can be instanced")?;
        // instancing an instance instances the original
        let origin = match self.nodes.get(first).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.instance_of.unwrap_or(first),
            _ => return Err("Only mesh objects can be instanced".to_string()),
        };
        let base = self.name_of(src);
        let t = self.create_node(NodeData::Transform, &base, None, true);
        let sname = format!("{}Shape", self.name_of(t));
        let md = MeshData { cache: Default::default(), history: Vec::new(), mesh: Mesh::new(), prim: "custom".to_string(), params: Vec::new(), material: None, instance_of: Some(origin), ..Default::default() };
        self.create_node(NodeData::Mesh(md), &sname, Some(t), false);
        for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ", "rotatePivotX", "rotatePivotY", "rotatePivotZ", "scalePivotX", "scalePivotY", "scalePivotZ", "rotatePivotTranslateX", "rotatePivotTranslateY", "rotatePivotTranslateZ"] {
            let v = self.get_attr(src, a)?;
            self.set_attr(t, a, v)?;
        }
        self.sync_instances();
        Ok(t)
    }

    /// A selection set with the given members (transforms; shapes are replaced by their transform).
    pub fn create_set(&mut self, name: Option<&str>, members: &[NodeId]) -> NodeId {
        let (n, force) = match name {
            Some(n) => (n, false),
            None => ("set", true),
        };
        let id = self.create_node(NodeData::ObjectSet(Box::default()), n, None, force);
        let _ = self.set_add(id, members);
        id
    }

    /// Add objects to a set; returns how many were new.
    pub fn set_add(&mut self, set: NodeId, members: &[NodeId]) -> Result<usize, String> {
        let mut mapped: Vec<NodeId> = Vec::new();
        for m in members {
            if *m == set {
                continue;
            }
            let t = match self.nodes.get(*m) {
                Some(n) if !n.data.is_transform() && !matches!(n.data, NodeData::ObjectSet(_)) => n.parent.unwrap_or(*m),
                Some(_) => *m,
                None => continue,
            };
            mapped.push(t);
        }
        match self.nodes.get_mut(set).map(|n| &mut n.data) {
            Some(NodeData::ObjectSet(sd)) => {
                let mut n = 0;
                for t in mapped {
                    if !sd.members.contains(&t) {
                        sd.members.push(t);
                        n += 1;
                    }
                }
                Ok(n)
            }
            _ => Err("Not a set".to_string()),
        }
    }

    pub fn set_remove(&mut self, set: NodeId, members: &[NodeId]) -> Result<usize, String> {
        match self.nodes.get_mut(set).map(|n| &mut n.data) {
            Some(NodeData::ObjectSet(sd)) => {
                let before = sd.members.len();
                sd.members.retain(|m| !members.contains(m));
                Ok(before - sd.members.len())
            }
            _ => Err("Not a set".to_string()),
        }
    }

    pub fn set_members(&self, set: NodeId) -> Vec<NodeId> {
        match self.nodes.get(set).map(|n| &n.data) {
            Some(NodeData::ObjectSet(sd)) => sd.members.iter().copied().filter(|m| self.nodes.get(*m).is_some()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn create_camera(&mut self, stereo: bool, name: Option<&str>) -> (NodeId, NodeId) {
        let (base, data) = if stereo {
            ("stereoCamera", NodeData::StereoCamera)
        } else {
            ("camera", NodeData::Camera)
        };
        let (tname, force) = match name {
            Some(n) => (n, false),
            None => (base, true),
        };
        let t = self.create_node(NodeData::Transform, tname, None, force);
        let shape_name = format!("{}Shape", self.name_of(t));
        let s = self.create_node(data, &shape_name, Some(t), false);
        (t, s)
    }

    pub fn create_light(&mut self, light_type: f32, name: Option<&str>) -> (NodeId, NodeId) {
        let base = match light_type.round() as i32 {
            1 => "directionalLight",
            2 => "spotLight",
            3 => "ambientLight",
            4 => "areaLight",
            _ => "pointLight",
        };
        let (tname, force) = match name {
            Some(n) => (n, false),
            None => (base, true),
        };
        let t = self.create_node(NodeData::Transform, tname, None, force);
        let shape_name = format!("{}Shape", self.name_of(t));
        let s = self.create_node(NodeData::Light, &shape_name, Some(t), false);
        let _ = self.set_attr(s, "lightType", light_type);
        (t, s)
    }

    /// An emitter (transform plus shape) feeding `solver`: moving or rotating the transform moves and
    /// aims the emission, the shape's attributes drive the solver's emission settings.
    pub fn create_emitter(&mut self, solver: NodeId, name: Option<&str>) -> Result<(NodeId, NodeId), String> {
        let existing = self.nodes.get(solver).and_then(|n| n.plug("emitX")).and_then(|p| p.source.clone());
        if existing.is_some() {
            return Err(format!("'{}' already has an emitter", self.name_of(solver)));
        }
        let (tname, force) = match name {
            Some(n) => (n, false),
            None => ("emitter", true),
        };
        let t = self.create_node(NodeData::Transform, tname, None, force);
        let shape = self.create_node(NodeData::Emitter, &format!("{}Shape", self.name_of(t)), Some(t), false);
        for (a, e) in [("translateX", "emitX"), ("translateY", "emitY"), ("translateZ", "emitZ")] {
            let v = self.get_attr(solver, e)?;
            self.set_attr(t, a, v)?;
            self.connect((t, a.to_string()), (solver, e.to_string()), true)?;
        }
        for (a, e) in [
            ("rate", "emitRate"), ("speed", "emitSpeed"), ("spread", "spread"), ("lifespan", "lifespan"),
            ("maxParticles", "maxParticles"), ("startFrame", "startFrame"), ("seed", "seed"), ("enabled", "enabled"),
        ] {
            let v = self.get_attr(solver, e)?;
            self.set_attr(shape, a, v)?;
            self.connect((shape, a.to_string()), (solver, e.to_string()), true)?;
        }
        Ok((t, shape))
    }

    /// A texture node (`kind` is "file" or a built-in pattern name) with its outputs computed.
    pub fn create_texture(&mut self, kind: &str, path: &str, name: Option<&str>) -> NodeId {
        let data = TextureData { kind: kind.to_string(), path: path.to_string() };
        let (n, force) = match name {
            Some(n) => (n, false),
            None => (kind, true),
        };
        let id = self.create_node(NodeData::Texture(Box::new(data)), n, None, force);
        self.refresh_texture_outputs(id);
        id
    }

    /// Recompute the average color and alpha a texture node reports on its output plugs.
    pub fn refresh_texture_outputs(&mut self, id: NodeId) {
        let (kind, path) = match self.nodes.get(id).map(|n| &n.data) {
            Some(NodeData::Texture(t)) => (t.kind.clone(), t.path.clone()),
            _ => return,
        };
        let file = if kind == "file" { self.texture_file(&path) } else { format!("builtin:{}", kind) };
        let (mut avg, mut alpha) = ([0.5f32; 3], 1.0f32);
        let loaded = if crate::render::netbake::needs_network(self, id) { crate::render::netbake::baked(self, id, 32) } else { crate::render::texture::load(&file) };
        if let Some(tex) = loaded {
            let n = tex.rgb.len().max(1);
            let step = (n / 4096).max(1);
            let (mut sum, mut cnt) = ([0.0f32; 3], 0.0f32);
            for c in tex.rgb.iter().step_by(step) {
                for k in 0..3 {
                    sum[k] += c[k];
                }
                cnt += 1.0;
            }
            if cnt > 0.0 {
                avg = [sum[0] / cnt, sum[1] / cnt, sum[2] / cnt];
            }
            if let Some(a) = &tex.alpha {
                let st = (a.len() / 4096).max(1);
                let v: Vec<f32> = a.iter().step_by(st).copied().collect();
                alpha = v.iter().sum::<f32>() / v.len().max(1) as f32;
            }
        }
        // display values are gamma encoded like the swatches the rest of the program shows
        let enc = |v: f32| v.max(0.0).powf(1.0 / 2.2);
        for (a, v) in [("outColorR", enc(avg[0])), ("outColorG", enc(avg[1])), ("outColorB", enc(avg[2])), ("outAlpha", alpha), ("outTransparency", 1.0 - alpha)] {
            self.set_eval(id, a, v);
        }
    }

    /// The texture node feeding `base` ("color", "incandescence") of a material, if any.
    pub fn texture_source(&self, mat: NodeId, base: &str) -> Option<NodeId> {
        let src = self.nodes.get(mat)?.plug(&format!("{}R", base))?.source.clone()?;
        match self.nodes.get(src.node).map(|n| &n.data) {
            Some(NodeData::Texture(_)) => Some(src.node),
            _ => None,
        }
    }

    /// The image or pattern mapped to the material's color ("builtin:NAME" for patterns).
    pub fn color_map(&self, mat: NodeId) -> Option<String> {
        let t = self.texture_source(mat, "color")?;
        match self.nodes.get(t).map(|n| &n.data) {
            Some(NodeData::Texture(d)) if d.kind == "file" => Some(d.path.clone()),
            Some(NodeData::Texture(d)) => Some(format!("builtin:{}", d.kind)),
            _ => None,
        }
    }

    /// Map an image or pattern to the material's color through a texture node (or remove the map).
    pub fn set_color_map(&mut self, mat: NodeId, path: Option<&str>) -> Result<(), String> {
        let existing = self.texture_source(mat, "color");
        let Some(p) = path else {
            if let Some(t) = existing {
                self.delete_node(t)?;
            }
            return Ok(());
        };
        let (kind, file) = match p.strip_prefix("builtin:") {
            Some(k) => (k.to_string(), String::new()),
            None => ("file".to_string(), p.to_string()),
        };
        if let Some(t) = existing {
            if let Some(NodeData::Texture(d)) = self.nodes.get_mut(t).map(|n| &mut n.data) {
                d.kind = kind;
                d.path = file;
            }
            self.refresh_texture_outputs(t);
            return Ok(());
        }
        let t = self.create_texture(&kind, &file, None);
        for a in ["R", "G", "B"] {
            self.connect((t, format!("outColor{}", a)), (mat, format!("color{}", a)), true)?;
        }
        Ok(())
    }

    pub fn create_material(&mut self, name: Option<&str>) -> NodeId {
        match name {
            Some(n) => self.create_node(NodeData::Material, n, None, false),
            None => self.create_node(NodeData::Material, "lambert", None, true),
        }
    }

    pub fn create_solver(&mut self, name: Option<&str>) -> NodeId {
        let data = NodeData::from_type_name("nucleus").unwrap();
        match name {
            Some(n) => self.create_node(data, n, None, false),
            None => self.create_node(data, "nucleus", None, true),
        }
    }

    pub fn mesh_shapes_under(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if let Some(node) = self.nodes.get(n) {
                if matches!(node.data, NodeData::Mesh(_)) {
                    out.push(n);
                }
                stack.extend(node.children.iter().copied());
            }
        }
        out
    }

    pub fn assign_material(&mut self, mat: NodeId, obj: NodeId) -> Result<usize, String> {
        match self.nodes.get(mat) {
            Some(n) if matches!(n.data, NodeData::Material) => {}
            _ => return Err("Not a material node".to_string()),
        }
        // particles take a material too (blobby surfaces and spheres render with it)
        if let Some(p) = self.dyn_target_of(obj).filter(|p| matches!(self.nodes.get(*p).map(|n| &n.data), Some(NodeData::Particle(_)))) {
            if let Some(NodeData::Particle(pd)) = self.nodes.get_mut(p).map(|n| &mut n.data) {
                pd.material = Some(mat);
            }
            return Ok(1);
        }
        let shapes = self.mesh_shapes_under(obj);
        if shapes.is_empty() {
            return Err("Object has no mesh to assign to".to_string());
        }
        for s in &shapes {
            if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(*s).map(|n| &mut n.data) {
                md.material = Some(mat);
            }
        }
        Ok(shapes.len())
    }

    pub fn ids_where<F: Fn(&NodeData) -> bool>(&self, f: F) -> Vec<NodeId> {
        self.nodes.iter().filter(|(_, n)| f(&n.data)).map(|(i, _)| i).collect()
    }

    // ------------------------------------------------------------------ hierarchy

    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            out.push(n);
            if let Some(node) = self.nodes.get(n) {
                for c in node.children.iter().rev() {
                    stack.push(*c);
                }
            }
        }
        out
    }

    pub fn set_parent(&mut self, child: NodeId, parent: Option<NodeId>) -> Result<(), String> {
        if let Some(p) = parent {
            if self.descendants(child).contains(&p) {
                return Err("Cannot parent an object under itself or its descendants".to_string());
            }
            if !self.nodes.get(p).map(|n| n.data.is_transform()).unwrap_or(false) {
                return Err("Parent must be a transform node".to_string());
            }
        }
        let old = self.nodes.get(child).ok_or("Invalid node")?.parent;
        match old {
            Some(op) => {
                if let Some(n) = self.nodes.get_mut(op) {
                    n.children.retain(|c| *c != child);
                }
            }
            None => self.roots.retain(|c| *c != child),
        }
        match parent {
            Some(p) => {
                if let Some(n) = self.nodes.get_mut(p) {
                    n.children.push(child);
                }
            }
            None => self.roots.push(child),
        }
        if let Some(n) = self.nodes.get_mut(child) {
            n.parent = parent;
        }
        self.invalidate_world(child);
        Ok(())
    }

    /// Reparent while keeping the world transform (Maya's default `parent` behaviour).
    pub fn set_parent_keep_world(&mut self, child: NodeId, parent: Option<NodeId>) -> Result<(), String> {
        let w = self.world_matrix(child);
        self.set_parent(child, parent)?;
        let pw = match parent {
            Some(p) => self.world_matrix(p),
            None => Mat4::identity(),
        };
        let local = pw.inverse() * w;
        self.set_local_matrix(child, &local)
    }

    /// Depth first rows of (node, depth) for the outliner.
    pub fn outliner_rows(&self) -> Vec<(NodeId, usize)> {
        let mut rows = Vec::new();
        fn walk(s: &Scene, id: NodeId, depth: usize, rows: &mut Vec<(NodeId, usize)>) {
            rows.push((id, depth));
            if let Some(n) = s.nodes.get(id) {
                for c in &n.children {
                    walk(s, *c, depth + 1, rows);
                }
            }
        }
        for r in &self.roots {
            walk(self, *r, 0, &mut rows);
        }
        rows
    }

    pub fn delete_node(&mut self, id: NodeId) -> Result<(), String> {
        if self.nodes.get(id).is_none() {
            return Err("Invalid node".to_string());
        }
        // deleting a lattice (a flexor) takes its deformation off the meshes it deforms
        let lattices: Vec<NodeId> = self.descendants(id).into_iter().filter(|n| matches!(self.nodes.get(*n).map(|x| &x.data), Some(NodeData::Lattice(_)))).collect();
        for lat in lattices {
            for sh in self.ids_where(|d| matches!(d, NodeData::Mesh(_))) {
                let ops: Vec<NodeId> = self.history_of(sh).into_iter().filter(|h| matches!(self.nodes.get(*h).map(|n| &n.data), Some(NodeData::PolyOp(o)) if o.kind == PolyKind::Lattice && o.lattice == Some(lat))).collect();
                for op in ops {
                    self.skins.retain(|s| *s != op);
                    let _ = self.delete_history_node(sh, op);
                }
            }
        }
        let mut subtree = self.descendants(id);
        let mut hist: Vec<NodeId> = Vec::new();
        for n in &subtree {
            if let Some(NodeData::Mesh(md)) = self.nodes.get(*n).map(|x| &x.data) {
                hist.extend(md.history.iter().copied());
            }
        }
        subtree.extend(hist);
        for n in &subtree {
            self.disconnect_all(*n);
            self.anim.remove_node(*n);
        }
        // Drop material references held by meshes.
        let doomed: Vec<NodeId> = subtree.clone();
        let mesh_ids = self.ids_where(|d| matches!(d, NodeData::Mesh(_)));
        for m in mesh_ids {
            if let Some(NodeData::Mesh(md)) = self.nodes.get_mut(m).map(|n| &mut n.data) {
                if let Some(mat) = md.material {
                    if doomed.contains(&mat) {
                        md.material = None;
                    }
                }
            }
        }
        let parent = self.nodes.get(id).and_then(|n| n.parent);
        match parent {
            Some(p) => {
                if let Some(pn) = self.nodes.get_mut(p) {
                    pn.children.retain(|c| *c != id);
                }
            }
            None => self.roots.retain(|c| *c != id),
        }
        let gone: Vec<NodeId> = subtree.clone();
        for n in subtree {
            if let Some(node) = self.nodes.remove(n) {
                self.names.remove(&node.name);
            }
            self.selection.retain(|s| *s != n);
        }
        // sets forget members that no longer exist
        for sid in self.ids_where(|d| matches!(d, NodeData::ObjectSet(_))) {
            if let Some(NodeData::ObjectSet(sd)) = self.nodes.get_mut(sid).map(|n| &mut n.data) {
                sd.members.retain(|m| !gone.contains(m));
            }
        }
        Ok(())
    }

    pub fn rename(&mut self, id: NodeId, new_name: &str) -> Result<String, String> {
        let old = self.nodes.get(id).ok_or("Invalid node")?.name.clone();
        if old == new_name {
            return Ok(old);
        }
        let nm = self.unique_name(new_name, false);
        self.names.remove(&old);
        self.names.insert(nm.clone(), id);
        if let Some(n) = self.nodes.get_mut(id) {
            n.name = nm.clone();
        }
        // keep Maya's <transform>Shape naming for the shapes underneath
        let children: Vec<NodeId> = self.nodes.get(id).map(|n| n.children.clone()).unwrap_or_default();
        for c in children {
            let (is_shape, cname) = match self.nodes.get(c) {
                Some(n) => (!n.data.is_transform(), n.name.clone()),
                None => continue,
            };
            if is_shape && cname == format!("{}Shape", old) {
                let want = format!("{}Shape", nm);
                let nn = self.unique_name(&want, false);
                self.names.remove(&cname);
                self.names.insert(nn.clone(), c);
                if let Some(n) = self.nodes.get_mut(c) {
                    n.name = nn;
                }
            }
        }
        Ok(nm)
    }

    // ------------------------------------------------------------------ plugs and connections

    pub fn add_attr(&mut self, id: NodeId, name: &str, default: f32) -> Result<(), String> {
        self.add_attr_ex(id, name, default, None, None, true)
    }

    /// Add Attribute with a range and the keyable choice of the dialog.
    pub fn add_attr_ex(&mut self, id: NodeId, name: &str, default: f32, min: Option<f32>, max: Option<f32>, keyable: bool) -> Result<(), String> {
        self.add_attr_typed(id, name, default, min, max, keyable, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_attr_typed(&mut self, id: NodeId, name: &str, default: f32, min: Option<f32>, max: Option<f32>, keyable: bool, dtype: u8) -> Result<(), String> {
        let valid = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit());
        if !valid {
            return Err(format!("'{}' is not a valid attribute name: use letters, digits and underscores, not starting with a digit", name));
        }
        if let (Some(a), Some(b)) = (min, max) {
            if a > b {
                return Err("The minimum must not be larger than the maximum".to_string());
            }
        }
        let node = self.nodes.get_mut(id).ok_or("Invalid node")?;
        if node.plug(name).is_some() {
            return Err(format!("Attribute '{}' already exists on '{}'", name, node.name));
        }
        let mut p = Plug::new(name, default.clamp(min.unwrap_or(f32::MIN), max.unwrap_or(f32::MAX)));
        p.custom = true;
        p.min = if dtype == 2 { Some(0.0) } else { min };
        p.max = if dtype == 2 { Some(1.0) } else { max };
        p.keyable = keyable;
        p.dtype = dtype;
        if dtype == 1 {
            p.value = p.value.round();
            p.rest = p.value;
        }
        node.plugs.push(p);
        Ok(())
    }

    /// Delete a custom attribute (Modify > Edit Attribute > Delete).
    pub fn delete_attr(&mut self, id: NodeId, name: &str) -> Result<(), String> {
        let (custom, source, dests) = {
            let node = self.nodes.get(id).ok_or("Invalid node")?;
            let p = node.plug(name).ok_or_else(|| format!("Attribute '{}' not found on '{}'", name, node.name))?;
            (p.custom, p.source.clone(), p.dests.clone())
        };
        if !custom {
            return Err("Only attributes added with Add Attribute can be deleted".to_string());
        }
        if let Some(s) = source {
            let _ = self.disconnect((s.node, s.attr), (id, name.to_string()));
        }
        for d in dests {
            let _ = self.disconnect((id, name.to_string()), (d.node, d.attr));
        }
        self.anim.bindings.retain(|b| !(b.node == id && b.attr == name));
        if let Some(n) = self.nodes.get_mut(id) {
            n.plugs.retain(|p| p.name != name);
        }
        Ok(())
    }

    /// Channel Control: show or hide an attribute in the Channel Box.
    pub fn set_keyable(&mut self, id: NodeId, name: &str, keyable: bool) -> Result<(), String> {
        let node = self.nodes.get_mut(id).ok_or("Invalid node")?;
        let nname = node.name.clone();
        let p = node.plug_mut(name).ok_or_else(|| format!("Attribute '{}' not found on '{}'", name, nname))?;
        p.keyable = keyable;
        Ok(())
    }

    pub fn set_locked(&mut self, id: NodeId, name: &str, locked: bool) -> Result<(), String> {
        let node = self.nodes.get_mut(id).ok_or("Invalid node")?;
        let nname = node.name.clone();
        let p = node.plug_mut(name).ok_or_else(|| format!("Attribute '{}' not found on '{}'", nname, name))?;
        p.locked = locked;
        Ok(())
    }

    pub fn get_attr(&mut self, id: NodeId, attr: &str) -> Result<f32, String> {
        let attr = canonical_attr(attr);
        if attr == "output" && matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::DrivenCurve(_))) {
            let x = self.get_attr(id, "input")?;
            let v = match self.nodes.get(id).map(|n| &n.data) {
                Some(NodeData::DrivenCurve(c)) => super::constraint::eval_driven(c, x),
                _ => 0.0,
            };
            if let Some(p) = self.nodes.get_mut(id).and_then(|n| n.plug_mut("output")) {
                p.value = v;
                p.dirty = false;
            }
            return Ok(v);
        }
        let (src, dirty, value) = {
            let node = self.nodes.get(id).ok_or("Invalid node")?;
            let p = node
                .plug(&attr)
                .ok_or_else(|| format!("Attribute '{}' not found on '{}'", attr, node.name))?;
            (p.source.clone(), p.dirty, p.value)
        };
        if !dirty {
            return Ok(value);
        }
        let new_value = match src {
            Some(s) => self.get_attr(s.node, &s.attr)?,
            None => value,
        };
        if let Some(p) = self.nodes.get_mut(id).and_then(|n| n.plug_mut(&attr)) {
            p.value = new_value;
            p.dirty = false;
        }
        Ok(new_value)
    }

    pub fn set_attr(&mut self, id: NodeId, attr: &str, v: f32) -> Result<(), String> {
        let attr = canonical_attr(attr);
        {
            let node = self.nodes.get_mut(id).ok_or("Invalid node")?;
            let nname = node.name.clone();
            let p = node
                .plug_mut(&attr)
                .ok_or_else(|| format!("Attribute '{}' not found on '{}'", attr, nname))?;
            if p.readonly {
                return Err(format!("Attribute '{}.{}' is read only", nname, attr));
            }
            if p.locked {
                return Err(format!("Attribute '{}.{}' is locked", nname, attr));
            }
            if p.source.is_some() {
                return Err(format!("Attribute '{}.{}' is connected and cannot be set", nname, attr));
            }
            let mut v = v.clamp(p.min.unwrap_or(f32::MIN), p.max.unwrap_or(f32::MAX));
            match p.dtype {
                1 => v = v.round(),
                2 => v = if v > 0.5 { 1.0 } else { 0.0 },
                _ => {}
            }
            p.value = v;
            p.rest = v;
            p.dirty = false;
        }
        self.propagate(id, &attr);
        // expressions read attributes: run them after every change
        if !self.expressions.is_empty() && !self.in_expression {
            self.eval_expressions();
        }
        if matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::PolyOp(_))) {
            self.mark_poly_dirty(id);
            self.flush_meshes();
        }
        if is_transform_attr(&attr) {
            if matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::IkHandle(_))) {
                self.solve_iks();
            }
            // moving a constraint target (or anything above it) moves what it drives
            if self.nodes.iter().any(|(_, n)| matches!(n.data, NodeData::Constraint(_))) {
                self.solve_constraints();
            }
            if !self.skins.is_empty() {
                self.mark_skins_dirty();
                self.flush_meshes();
            }
        }
        // moving a lattice point (or turning a flexor's channels) deforms what it deforms
        if !self.skins.is_empty() && (matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::Lattice(_))) || attr.starts_with("flexor")) {
            self.mark_skins_dirty();
            self.flush_meshes();
        }
        Ok(())
    }

    /// Write an evaluated value (animation, solver output) without touching the authored rest value.
    pub fn set_eval(&mut self, id: NodeId, attr: &str, v: f32) {
        let mut ok = false;
        let mut changed = false;
        if let Some(p) = self.nodes.get_mut(id).and_then(|n| n.plug_mut(attr)) {
            // transform limits (joint rotate limits) also bind values set by animation, IK and constraints
            let v = v.clamp(p.min.unwrap_or(f32::MIN), p.max.unwrap_or(f32::MAX));
            if p.source.is_none() {
                changed = (p.value - v).abs() > 1e-9;
                p.value = v;
                p.dirty = false;
                ok = true;
            }
        }
        if ok {
            self.propagate(id, attr);
            if changed && matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::PolyOp(_))) {
                self.mark_poly_dirty(id);
            }
            if changed && (is_transform_attr(attr) || matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::Lattice(_)))) && !self.skins.is_empty() {
                self.mark_skins_dirty();
            }
        }
    }

    fn invalidate_world(&mut self, id: NodeId) {
        for n in self.descendants(id) {
            if let Some(node) = self.nodes.get_mut(n) {
                node.world_cache = None;
            }
        }
    }

    /// Dirty flag propagation: mark every downstream plug dirty (iteratively) and drop stale matrix caches.
    pub(super) fn propagate(&mut self, id: NodeId, attr: &str) {
        if is_transform_attr(attr) {
            self.invalidate_world(id);
        }
        let mut stack: Vec<PlugRef> = self
            .nodes
            .get(id)
            .and_then(|n| n.plug(attr))
            .map(|p| p.dests.clone())
            .unwrap_or_default();
        let mut guard = 0usize;
        while let Some(d) = stack.pop() {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            let mut next = Vec::new();
            let mut transform = false;
            if let Some(p) = self.nodes.get_mut(d.node).and_then(|n| n.plug_mut(&d.attr)) {
                p.dirty = true;
                next = p.dests.clone();
                transform = is_transform_attr(&d.attr);
            }
            if transform {
                self.invalidate_world(d.node);
            }
            stack.extend(next);
        }
    }

    /// True if `to` is reachable from `from` following outgoing connections.
    fn reaches(&self, from: &PlugRef, to: &PlugRef) -> bool {
        let mut stack = vec![from.clone()];
        let mut seen: Vec<PlugRef> = Vec::new();
        while let Some(c) = stack.pop() {
            if c == *to {
                return true;
            }
            if seen.contains(&c) {
                continue;
            }
            seen.push(c.clone());
            if let Some(p) = self.nodes.get(c.node).and_then(|n| n.plug(&c.attr)) {
                stack.extend(p.dests.iter().cloned());
            }
        }
        false
    }

    pub fn connect(&mut self, src: (NodeId, String), dst: (NodeId, String), force: bool) -> Result<(), String> {
        let sref = PlugRef { node: src.0, attr: canonical_attr(&src.1) };
        let dref = PlugRef { node: dst.0, attr: canonical_attr(&dst.1) };
        let dname = self.name_of(dref.node);
        let existing = {
            let sp = self.nodes.get(sref.node).and_then(|n| n.plug(&sref.attr)).ok_or("Source attribute not found")?;
            let _ = sp;
            let dp = self.nodes.get(dref.node).and_then(|n| n.plug(&dref.attr)).ok_or("Destination attribute not found")?;
            if dp.readonly {
                return Err(format!("Destination '{}.{}' is read only", dname, dref.attr));
            }
            dp.source.clone()
        };
        if existing.is_some() && !force {
            return Err(format!("Destination '{}.{}' is already connected (use -force)", dname, dref.attr));
        }
        if self.anim.is_animated(dref.node, &dref.attr) {
            return Err(format!("Destination '{}.{}' is animated and cannot be driven", dname, dref.attr));
        }
        if sref == dref || self.reaches(&dref, &sref) {
            return Err("Connection rejected: it would create a cycle in the dependency graph".to_string());
        }
        if let Some(old) = existing {
            self.disconnect_refs(&old, &dref);
        }
        if let Some(p) = self.nodes.get_mut(sref.node).and_then(|n| n.plug_mut(&sref.attr)) {
            p.dests.push(dref.clone());
        }
        if let Some(p) = self.nodes.get_mut(dref.node).and_then(|n| n.plug_mut(&dref.attr)) {
            p.source = Some(sref);
            p.dirty = true;
        }
        let attr = dref.attr.clone();
        if is_transform_attr(&attr) {
            self.invalidate_world(dref.node);
        }
        self.propagate(dref.node, &attr);
        Ok(())
    }

    fn disconnect_refs(&mut self, src: &PlugRef, dst: &PlugRef) {
        if let Some(p) = self.nodes.get_mut(src.node).and_then(|n| n.plug_mut(&src.attr)) {
            p.dests.retain(|d| d != dst);
        }
        if let Some(p) = self.nodes.get_mut(dst.node).and_then(|n| n.plug_mut(&dst.attr)) {
            if p.source.as_ref() == Some(src) {
                p.source = None;
                p.dirty = false;
            }
        }
    }

    pub fn disconnect(&mut self, src: (NodeId, String), dst: (NodeId, String)) -> Result<(), String> {
        let sref = PlugRef { node: src.0, attr: canonical_attr(&src.1) };
        let dref = PlugRef { node: dst.0, attr: canonical_attr(&dst.1) };
        let linked = self
            .nodes
            .get(dref.node)
            .and_then(|n| n.plug(&dref.attr))
            .map(|p| p.source.as_ref() == Some(&sref))
            .unwrap_or(false);
        if !linked {
            return Err("Those attributes are not connected".to_string());
        }
        self.disconnect_refs(&sref, &dref);
        if is_transform_attr(&dref.attr) {
            self.invalidate_world(dref.node);
        }
        Ok(())
    }

    pub(crate) fn disconnect_all(&mut self, id: NodeId) {
        let snapshot: Vec<(String, Option<PlugRef>, Vec<PlugRef>)> = match self.nodes.get(id) {
            Some(n) => n.plugs.iter().map(|p| (p.name.clone(), p.source.clone(), p.dests.clone())).collect(),
            None => return,
        };
        for (attr, src, dests) in snapshot {
            let me = PlugRef { node: id, attr };
            if let Some(s) = src {
                self.disconnect_refs(&s, &me);
            }
            for d in dests {
                self.disconnect_refs(&me, &d);
            }
        }
    }

    /// All connections as (source, destination) pairs in a stable order.
    pub fn connections(&self) -> Vec<(PlugRef, PlugRef)> {
        let mut out = Vec::new();
        for (id, node) in self.nodes.iter() {
            for p in &node.plugs {
                for d in &p.dests {
                    out.push((PlugRef { node: id, attr: p.name.clone() }, d.clone()));
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------ transforms

    fn read_vec3(&mut self, id: NodeId, names: [&str; 3]) -> Vec3 {
        Vec3::new(
            self.get_attr(id, names[0]).unwrap_or(0.0),
            self.get_attr(id, names[1]).unwrap_or(0.0),
            self.get_attr(id, names[2]).unwrap_or(0.0),
        )
    }

    pub fn local_matrix(&mut self, id: NodeId) -> Mat4 {
        let is_t = self.nodes.get(id).map(|n| n.data.is_transform()).unwrap_or(false);
        if !is_t {
            return Mat4::identity();
        }
        let t = self.read_vec3(id, ["translateX", "translateY", "translateZ"]);
        let r = self.read_vec3(id, ["rotateX", "rotateY", "rotateZ"]);
        let s = self.read_vec3(id, ["scaleX", "scaleY", "scaleZ"]);
        let has_pivot = self.nodes.get(id).map(|n| n.plug("rotatePivotX").is_some()).unwrap_or(false);
        let jo = if self.nodes.get(id).map(|n| n.plug("jointOrientX").is_some()).unwrap_or(false) { self.read_vec3(id, ["jointOrientX", "jointOrientY", "jointOrientZ"]) } else { Vec3::ZERO };
        let m = if !has_pivot {
            Mat4::trs(t, Quat::from_euler_deg(r), s)
        } else {
            let rp = self.read_vec3(id, ["rotatePivotX", "rotatePivotY", "rotatePivotZ"]);
            let sp = self.read_vec3(id, ["scalePivotX", "scalePivotY", "scalePivotZ"]);
            let rpt = self.read_vec3(id, ["rotatePivotTranslateX", "rotatePivotTranslateY", "rotatePivotTranslateZ"]);
            Self::pivoted(t + rpt, r, s, rp, sp)
        };
        if jo.length_sq() < 1e-14 {
            return m;
        }
        // joint orient sits between the translation and the rotation: T * JO * (T^-1 * M)
        let tm = m.translation_part();
        Mat4::translation(tm) * Mat4::from_quat(Quat::from_euler_deg(jo)) * Mat4::translation(-tm) * m
    }

    /// Maya's transform: T * Rp * R * Rp^-1 * Sp * S * Sp^-1 (`t` includes the rotate pivot translation).
    pub fn pivoted(t: Vec3, r: Vec3, s: Vec3, rp: Vec3, sp: Vec3) -> Mat4 {
        if rp.length_sq() < 1e-14 && sp.length_sq() < 1e-14 {
            return Mat4::trs(t, Quat::from_euler_deg(r), s);
        }
        Mat4::translation(t) * Mat4::translation(rp) * Mat4::from_quat(Quat::from_euler_deg(r)) * Mat4::translation(-rp) * Mat4::translation(sp) * Mat4::scaling(s) * Mat4::translation(-sp)
    }

    /// The translate, rotate and scale values that give `id` the local matrix `m` with its pivots.
    pub fn trs_for_local(&mut self, id: NodeId, m: &Mat4) -> (Vec3, Vec3, Vec3) {
        // a joint's orient is applied after its translation: take it out before reading the rotation
        let jo = if self.nodes.get(id).map(|n| n.plug("jointOrientX").is_some()).unwrap_or(false) { self.read_vec3(id, ["jointOrientX", "jointOrientY", "jointOrientZ"]) } else { Vec3::ZERO };
        let stripped;
        let m = if jo.length_sq() > 1e-14 {
            let t0 = m.translation_part();
            stripped = Mat4::translation(t0) * Mat4::from_quat(Quat::from_euler_deg(jo)).inverse() * Mat4::translation(-t0) * *m;
            &stripped
        } else {
            m
        };
        let (tm, r, s) = m.decompose();
        let has_pivot = self.nodes.get(id).map(|n| n.plug("rotatePivotX").is_some()).unwrap_or(false);
        if !has_pivot {
            return (tm, r, s);
        }
        let rp = self.read_vec3(id, ["rotatePivotX", "rotatePivotY", "rotatePivotZ"]);
        let sp = self.read_vec3(id, ["scalePivotX", "scalePivotY", "scalePivotZ"]);
        let rpt = self.read_vec3(id, ["rotatePivotTranslateX", "rotatePivotTranslateY", "rotatePivotTranslateZ"]);
        // the part of the matrix that does not come from T
        let c = Self::pivoted(Vec3::ZERO, r, s, rp, sp).translation_part();
        (tm - c - rpt, r, s)
    }

    /// Where the rotate pivot is in world space.
    pub fn pivot_world(&mut self, id: NodeId) -> Vec3 {
        let t = self.read_vec3(id, ["translateX", "translateY", "translateZ"]);
        let has = self.nodes.get(id).map(|n| n.plug("rotatePivotX").is_some()).unwrap_or(false);
        let off = if has {
            self.read_vec3(id, ["rotatePivotX", "rotatePivotY", "rotatePivotZ"]) + self.read_vec3(id, ["rotatePivotTranslateX", "rotatePivotTranslateY", "rotatePivotTranslateZ"])
        } else {
            Vec3::ZERO
        };
        match self.nodes.get(id).and_then(|n| n.parent) {
            Some(p) => self.world_matrix(p).transform_point(t + off),
            None => t + off,
        }
    }

    /// Set translate / rotate / scale so the object's local matrix becomes `m` (driven channels are left alone).
    pub fn set_local_matrix(&mut self, id: NodeId, m: &Mat4) -> Result<(), String> {
        let (t, r, s) = self.trs_for_local(id, m);
        for (a, v) in [
            ("translateX", t.x), ("translateY", t.y), ("translateZ", t.z),
            ("rotateX", r.x), ("rotateY", r.y), ("rotateZ", r.z),
            ("scaleX", s.x), ("scaleY", s.y), ("scaleZ", s.z),
        ] {
            let driven = self.nodes.get(id).and_then(|n| n.plug(a)).map(|p| p.source.is_some()).unwrap_or(true);
            if !driven {
                self.set_attr(id, a, v)?;
            }
        }
        Ok(())
    }

    /// Lazily evaluated, cached world matrix.
    pub fn world_matrix(&mut self, id: NodeId) -> Mat4 {
        if let Some(m) = self.nodes.get(id).and_then(|n| n.world_cache) {
            return m;
        }
        let parent = self.nodes.get(id).and_then(|n| n.parent);
        let inherits = self.nodes.get(id).and_then(|n| n.plug("inheritsTransform")).map(|p| p.value > 0.5).unwrap_or(true);
        let pm = match parent {
            Some(p) if inherits => self.world_matrix(p),
            _ => Mat4::identity(),
        };
        let m = pm * self.local_matrix(id);
        if let Some(n) = self.nodes.get_mut(id) {
            n.world_cache = Some(m);
        }
        m
    }

    pub fn is_visible(&mut self, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            let is_t = self.nodes.get(c).map(|n| n.data.is_transform()).unwrap_or(false);
            if is_t && self.get_attr(c, "visibility").unwrap_or(1.0) < 0.5 {
                return false;
            }
            cur = self.nodes.get(c).and_then(|n| n.parent);
        }
        true
    }

    // ------------------------------------------------------------------ animation and dynamics

    pub fn set_keyframe(
        &mut self,
        id: NodeId,
        attr: &str,
        t: Option<f32>,
        v: Option<f32>,
        interp: Interp,
        layer: usize,
    ) -> Result<(f32, f32), String> {
        let r = self.set_keyframe_noeval(id, attr, t, v, interp, layer)?;
        self.evaluate_anim();
        Ok(r)
    }

    /// Like `set_keyframe` but does not re-evaluate animation; batch callers evaluate once at the end
    /// so keying several channels does not reset the ones that have not been keyed yet.
    pub fn set_keyframe_noeval(
        &mut self,
        id: NodeId,
        attr: &str,
        t: Option<f32>,
        v: Option<f32>,
        interp: Interp,
        layer: usize,
    ) -> Result<(f32, f32), String> {
        let attr = canonical_attr(attr);
        let time = t.unwrap_or(self.timeline.current);
        let default_v = if layer == 0 { self.get_attr(id, &attr)? } else { 0.0 };
        let value = v.unwrap_or(default_v);
        {
            let node = self.nodes.get(id).ok_or("Invalid node")?;
            let p = node.plug(&attr).ok_or_else(|| format!("Attribute '{}' not found on '{}'", attr, node.name))?;
            if p.source.is_some() {
                return Err(format!("'{}.{}' is driven by a connection and cannot be keyed", node.name, attr));
            }
            if p.readonly {
                return Err(format!("'{}.{}' is read only", node.name, attr));
            }
        }
        if layer >= self.anim.layers.len() {
            return Err("Invalid animation layer".to_string());
        }
        self.anim.set_key(id, &attr, layer, time, value, interp);
        Ok((time, value))
    }

    pub fn evaluate_anim(&mut self) {
        let time = self.timeline.current;
        let results = {
            let nodes = &self.nodes;
            self.anim.evaluate(time, |n, a| {
                nodes.get(n).and_then(|nd| nd.plug(a)).map(|p| p.rest).unwrap_or(0.0)
            })
        };
        for (n, a, v) in results {
            self.set_eval(n, &a, v);
        }
        self.evaluate_trax();
        self.eval_expressions();
        self.flush_meshes();
    }

    pub fn update_solvers(&mut self) {
        let frame = self.timeline.frame();
        let fps = self.timeline.fps;
        let ids = self.ids_where(|d| matches!(d, NodeData::Nucleus(_)));
        for id in ids {
            let mut vals = Vec::with_capacity(28);
            for (name, _) in SOLVER_ATTRS.iter().take(28) {
                vals.push(self.get_attr(id, name).unwrap_or(0.0));
            }
            // an emitter positions and aims the emission in world space (its +Y axis is the direction)
            let emitter = self.nodes.get(id).and_then(|n| n.plug("emitX")).and_then(|p| p.source.as_ref()).map(|s| s.node);
            if let Some(t) = emitter.filter(|t| self.nodes.get(*t).is_some()) {
                let w = self.world_matrix(t);
                let (p, d) = (w.translation_part(), w.transform_dir(Vec3::UP).normalized());
                vals[15..18].copy_from_slice(&[p.x, p.y, p.z]);
                vals[18..21].copy_from_slice(&[d.x, d.y, d.z]);
            }
            let mut count = 0usize;
            if let Some(NodeData::Nucleus(nd)) = self.nodes.get_mut(id).map(|n| &mut n.data) {
                apply_solver_values(nd, &vals);
                nd.solver.advance_to(frame, fps);
                count = nd.solver.particle_count();
            }
            self.set_eval(id, "particleCount", count as f32);
        }
    }

    /// Evaluate animation then dynamics for the current timeline frame.
    pub fn evaluate_frame(&mut self) {
        self.evaluate_anim();
        self.solve_motion_paths();
        self.solve_distances();
        self.mark_jiggles_dirty();
        self.solve_iks();
        self.solve_constraints();
        self.update_solvers();
        self.update_dynamics();
    }

    pub fn set_time(&mut self, t: f32) {
        self.timeline.set_time(t);
        self.evaluate_frame();
    }

    pub fn advance_playback(&mut self, dt: f32) -> bool {
        let changed = self.timeline.tick(dt);
        if changed {
            self.evaluate_frame();
        }
        changed
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}
