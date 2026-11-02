pub mod anaglyph;
pub mod netbake;
pub mod passes;
pub mod pipeline;
pub mod rasterizer;
pub mod raytrace;
pub mod texture;

use crate::dag::nodes::NodeData;
use crate::dag::{NodeId, Scene};
use crate::math::{Mat4, Quat, Vec3};
use std::collections::HashMap;

pub use crate::geo::Mesh;

/// Material slot reserved for unassigned meshes.
pub const DEFAULT_MATERIAL: u16 = 0;
/// Material slot reserved for nParticles.
pub const PARTICLE_MATERIAL: u16 = 1;

#[derive(Clone, Debug)]
pub struct Material {
    pub color: Vec3,
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    pub shininess: f32,
    /// Mirror reflection amount used by the ray tracer (0..1).
    pub reflectivity: f32,
    /// 0 Lambert, 1 Blinn, 2 Phong, 3 Surface Shader (unlit).
    pub model: u8,
    pub transparency: f32,
    pub incandescence: Vec3,
    pub spec_color: Vec3,
    pub bump_depth: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Point,
    Directional,
    Spot,
    /// Lights everything equally and casts no shadows.
    Ambient,
    /// Rectangular emitter facing down its -Z axis.
    Area,
}

#[derive(Clone, Debug)]
pub struct RenderLight {
    pub kind: LightKind,
    pub pos: Vec3,
    /// Unit vector pointing from the surface toward a directional light.
    pub dir_to_light: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    /// Direction the light shines in (spot and area lights).
    pub axis: Vec3,
    /// Half angle of the spot cone and the extra fade-out angle past it, in radians.
    pub cone_half: f32,
    pub penumbra: f32,
    pub dropoff: f32,
    /// 0 none, 1 linear, 2 quadratic, 3 cubic, 4 soft.
    pub decay: u8,
    pub shadows: bool,
    /// Light radius for shadow feathering (degrees for directional lights); 0 is a hard shadow.
    pub softness: f32,
    pub samples: u32,
    pub shadow_color: Vec3,
    pub diffuse: bool,
    pub specular: bool,
    /// Half extents of an area light in world space.
    pub area_x: Vec3,
    pub area_y: Vec3,
    pub node: Option<NodeId>,
    /// Light linking: only these objects (transforms or shapes) are lit. None lights everything.
    pub links: Option<Vec<NodeId>>,
    /// Extra blur of depth map shadows in world units.
    pub dm_blur: f32,
}

impl RenderLight {
    pub fn simple(kind: LightKind, pos: Vec3, dir_to_light: Vec3, color: Vec3, intensity: f32) -> RenderLight {
        RenderLight {
            kind,
            pos,
            dir_to_light,
            color,
            intensity,
            axis: -dir_to_light,
            cone_half: 20f32.to_radians(),
            penumbra: 0.0,
            dropoff: 0.0,
            decay: 4,
            shadows: true,
            softness: 0.0,
            samples: 16,
            shadow_color: Vec3::ZERO,
            diffuse: true,
            specular: true,
            area_x: Vec3::ZERO,
            area_y: Vec3::ZERO,
            node: None,
            links: None,
            dm_blur: 0.0,
        }
    }

    /// Direction toward the light, its distance and the attenuation (distance falloff and spot cone)
    /// at a surface point. `None` when the point receives no light from it.
    pub fn incident(&self, pos: Vec3) -> Option<(Vec3, f32, f32)> {
        match self.kind {
            LightKind::Ambient => None,
            LightKind::Directional => Some((self.dir_to_light, f32::INFINITY, 1.0)),
            _ => {
                let v = self.pos - pos;
                let d2 = v.length_sq();
                let d = d2.sqrt().max(1e-6);
                let dir = v * (1.0 / d);
                let mut a = match self.decay {
                    0 => 1.0,
                    1 => 1.0 / d.max(1.0),
                    2 => 1.0 / d2.max(1.0),
                    3 => 1.0 / (d2 * d).max(1.0),
                    _ => 1.0 / (1.0 + 0.002 * d2),
                };
                let cosang = self.axis.dot(-dir);
                match self.kind {
                    LightKind::Spot => {
                        let outer = (self.cone_half + self.penumbra).min(std::f32::consts::PI - 0.01);
                        let (c_out, c_in) = (outer.cos(), self.cone_half.cos());
                        if cosang <= c_out {
                            return None;
                        }
                        if self.penumbra > 1e-5 {
                            let t = ((cosang - c_out) / (c_in - c_out).max(1e-6)).clamp(0.0, 1.0);
                            a *= t * t * (3.0 - 2.0 * t);
                        }
                        a *= cosang.max(0.0).powf(self.dropoff.max(0.0));
                    }
                    LightKind::Area => {
                        if cosang <= 0.0 {
                            return None;
                        }
                        a *= cosang;
                    }
                    _ => {}
                }
                Some((dir, d, a))
            }
        }
    }
}

/// RenderItem.flags bits (the Render Stats of the shape).
pub const F_CAST: u8 = 1;
pub const F_RECEIVE: u8 = 2;
pub const F_PRIMARY: u8 = 4;
pub const F_REFLECT: u8 = 8;
pub const F_BLUR: u8 = 16;

#[derive(Clone)]
pub struct RenderItem {
    pub flags: u8,
    /// The transform above the shape.
    pub owner: NodeId,
    pub node: NodeId,
    pub mesh: std::sync::Arc<Mesh>,
    pub world: Mat4,
    pub material: u16,
    pub selected: bool,
}

/// A colored line segment for viewport helpers such as lights and cameras.
pub struct GizmoLine {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Vec3,
    /// Drawn on top of geometry (joints, IK handles).
    pub xray: bool,
}

pub struct RenderParticle {
    pub pos: Vec3,
    pub radius: f32,
    /// 0 points, 1 spheres, 2 blobby surface, 3 sprites, 4 streaks (points, sprites and streaks only render in hardware).
    pub kind: u8,
    pub color: Vec3,
    pub opacity: f32,
    /// Blobby particles of one group merge.
    pub group: u32,
    pub threshold: f32,
    pub material: u16,
    pub vel: Vec3,
}

impl RenderParticle {
    pub fn sphere(pos: Vec3, radius: f32) -> RenderParticle {
        RenderParticle { pos, radius, kind: 1, color: Vec3::splat(-1.0), opacity: 1.0, group: 0, threshold: 1.0, material: PARTICLE_MATERIAL, vel: Vec3::ZERO }
    }
}

/// Value of a ramp (position, value) at `t`.
pub fn ramp_at(r: &[(f32, Vec3)], t: f32) -> Vec3 {
    if r.is_empty() {
        return Vec3::ZERO;
    }
    if t <= r[0].0 {
        return r[0].1;
    }
    for w in r.windows(2) {
        if t <= w[1].0 {
            let f = ((t - w[0].0) / (w[1].0 - w[0].0).max(1e-6)).clamp(0.0, 1.0);
            return w[0].1.lerp(w[1].1, f);
        }
    }
    r[r.len() - 1].1
}

/// Immutable snapshot of everything the rasterizer needs.
pub struct RenderScene {
    pub items: Vec<RenderItem>,
    pub lights: Vec<RenderLight>,
    pub materials: Vec<Material>,
    pub particles: Vec<RenderParticle>,
    pub gizmos: Vec<GizmoLine>,
    /// One optional color map per material slot.
    pub textures: Vec<Option<texture::TexInst>>,
    /// Bump maps and specular color maps, one optional per material slot.
    pub bumps: Vec<Option<texture::TexInst>>,
    pub spec_maps: Vec<Option<texture::TexInst>>,
    /// Viewports switch color maps off unless Textured display is on.
    pub use_textures: bool,
}

#[derive(Clone, Debug)]
pub struct StereoParams {
    pub interaxial: f32,
    pub zero_parallax: f32,
}

#[derive(Clone, Debug)]
pub struct CameraInfo {
    pub name: String,
    /// World matrix of the camera transform (camera looks down its local -Z).
    pub world: Mat4,
    pub fov_deg: f32,
    pub near: f32,
    pub far: f32,
    pub stereo: Option<StereoParams>,
    /// Fit Resolution Gate: 0 fill, 1 horizontal, 2 vertical, 3 overscan.
    pub fit: u8,
}

impl CameraInfo {
    pub fn eye(&self) -> Vec3 {
        self.world.translation_part()
    }
    pub fn view(&self) -> Mat4 {
        self.world.inverse()
    }
    /// The vertical field of view for a render of `aspect`, from the field of view and how it fits the resolution gate.
    pub fn vertical_fov(&self, aspect: f32) -> f32 {
        // the film is 3:2; fill matches the film to the gate by its width when the gate is wider
        let horizontal = match self.fit {
            1 => true,
            2 => false,
            3 => aspect < 1.5,
            _ => aspect >= 1.5,
        };
        if horizontal {
            2.0 * ((self.fov_deg.to_radians() * 0.5).tan() / aspect.max(1e-3)).atan()
        } else {
            self.fov_deg.to_radians()
        }
    }

    pub fn projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective(self.vertical_fov(aspect), aspect, self.near, self.far)
    }
}

/// Convert the evaluated scene into a render snapshot.
pub fn build_scene(scene: &mut Scene) -> RenderScene {
    let mut materials = vec![
        Material { color: Vec3::new(0.62, 0.64, 0.68), ambient: 0.14, diffuse: 0.85, specular: 0.25, shininess: 28.0, reflectivity: 0.0, model: 1, transparency: 0.0, incandescence: Vec3::ZERO, spec_color: Vec3::ONE, bump_depth: 0.0 },
        Material { color: Vec3::new(0.35, 0.75, 1.0), ambient: 0.25, diffuse: 0.8, specular: 0.5, shininess: 40.0, reflectivity: 0.0, model: 1, transparency: 0.0, incandescence: Vec3::ZERO, spec_color: Vec3::ONE, bump_depth: 0.0 },
    ];
    let mut textures: Vec<Option<texture::TexInst>> = vec![None, None];
    let mut bumps: Vec<Option<texture::TexInst>> = vec![None, None];
    let mut spec_maps: Vec<Option<texture::TexInst>> = vec![None, None];
    let mut mat_index: HashMap<NodeId, u16> = HashMap::new();
    for id in scene.ids_where(|d| matches!(d, NodeData::Material)) {
        let g = |s: &mut Scene, a: &str| s.get_attr(id, a).unwrap_or(0.0);
        let m = Material {
            color: Vec3::new(g(scene, "colorR"), g(scene, "colorG"), g(scene, "colorB")),
            ambient: g(scene, "ambient"),
            diffuse: g(scene, "diffuse"),
            specular: g(scene, "specular"),
            shininess: g(scene, "shininess"),
            reflectivity: g(scene, "reflectivity"),
            model: g(scene, "shaderModel").round().clamp(0.0, 3.0) as u8,
            transparency: g(scene, "transparency").clamp(0.0, 1.0),
            incandescence: Vec3::new(g(scene, "incandescenceR"), g(scene, "incandescenceG"), g(scene, "incandescenceB")),
            spec_color: Vec3::new(g(scene, "specularColorR"), g(scene, "specularColorG"), g(scene, "specularColorB")),
            bump_depth: g(scene, "bumpDepth"),
        };
        mat_index.insert(id, materials.len() as u16);
        materials.push(m);
        textures.push(texture_instance(scene, id));
        bumps.push(texture_instance_for(scene, id, "bump"));
        spec_maps.push(texture_instance_for(scene, id, "specularColor"));
    }

    scene.flush_meshes();
    let selection = scene.selection.clone();
    let mut items = Vec::new();
    for id in scene.ids_where(|d| matches!(d, NodeData::Mesh(_))) {
        if !scene.is_visible(id) || !scene.show.polygons {
            continue;
        }
        // strokes drawn as lines in the views still render as meshes
        if !scene.final_render && matches!(scene.node(id).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.stroke.is_some()) && scene.node(id).and_then(|n| n.plug("drawAsMesh")).map(|p| p.value < 0.5).unwrap_or(false) {
            continue;
        }
        let world = scene.world_matrix(id);
        let parent = scene.node(id).and_then(|n| n.parent);
        let flag = |scene: &Scene, k: &str, bit: u8| -> u8 { if scene.node(id).and_then(|n| n.plug(k)).map(|p| p.value > 0.5).unwrap_or(true) { bit } else { 0 } };
        let flags = flag(scene, "castsShadows", F_CAST) | flag(scene, "receiveShadows", F_RECEIVE) | flag(scene, "primaryVisibility", F_PRIMARY) | flag(scene, "visibleInReflections", F_REFLECT) | flag(scene, "motionBlur", F_BLUR);
        let owner = parent.unwrap_or(id);
        let mat = match scene.node(id).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.material,
            _ => continue,
        };
        let material = mat.and_then(|m| mat_index.get(&m).copied()).unwrap_or(DEFAULT_MATERIAL);
        let has_zones = matches!(scene.node(id).map(|n| &n.data), Some(NodeData::Mesh(md)) if !md.face_zone.is_empty());
        if has_zones {
            let base_map = {
                let g = |s: &mut Scene, a: &str, d: f32| s.get_attr(id, a).unwrap_or(d);
                crate::geo::edit::UvMap {
                    mode: g(scene, "uvMode", 0.0) as u8,
                    tile_u: g(scene, "uvTilingU", 1.0),
                    tile_v: g(scene, "uvTilingV", 1.0),
                    off_u: g(scene, "uvOffsetU", 0.0),
                    off_v: g(scene, "uvOffsetV", 0.0),
                    rot_deg: g(scene, "uvRotate", 0.0),
                }
            };
            let angle = scene.get_attr(id, "smoothAngle").unwrap_or(40.0).clamp(0.0, 180.0);
            let selected = selection.contains(&id) || parent.map(|p| selection.contains(&p)).unwrap_or(false);
            // material and projection of every zone; zone 0 is the shape's own
            let mut zmat: Vec<u16> = vec![material];
            let mut zmap: Vec<Option<crate::geo::edit::UvMap>> = Vec::new();
            if let Some(NodeData::Mesh(md)) = scene.node(id).map(|n| &n.data) {
                let tex = |mi: u16| textures.get(mi as usize).map(|t| t.is_some()).unwrap_or(false);
                let explicit = md.uv_set.is_some();
                zmap.push(if tex(material) && !explicit { Some(base_map) } else { None });
                for z in &md.zones {
                    let mi = z.material.or(md.material).and_then(|m| mat_index.get(&m).copied()).unwrap_or(DEFAULT_MATERIAL);
                    zmat.push(mi);
                    zmap.push(if tex(mi) && !explicit { Some(z.uv) } else { None });
                }
            }
            let parts = match scene.nodes.get_mut(id).map(|n| &mut n.data) {
                Some(NodeData::Mesh(md)) => {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    angle.to_bits().hash(&mut h);
                    md.cache.version.hash(&mut h);
                    md.face_zone.hash(&mut h);
                    for m in &zmap {
                        match m {
                            Some(u) => [u.mode as u32, u.tile_u.to_bits(), u.tile_v.to_bits(), u.off_u.to_bits(), u.off_v.to_bits(), u.rot_deg.to_bits()].hash(&mut h),
                            None => 0u8.hash(&mut h),
                        }
                    }
                    let key = h.finish();
                    if !matches!(&md.cache.zoned, Some((k, _)) if *k == key) {
                        let mut sm = crate::geo::edit::shading_split_zoned(&md.mesh, angle, &md.face_zone, &zmap);
                        if let Some(set) = &md.uv_set {
                            set.apply_to(&mut sm);
                        }
                        let mut parts: Vec<(u16, std::sync::Arc<Mesh>)> = Vec::new();
                        for z in 0..zmap.len() {
                            let faces: Vec<&Vec<u32>> = sm.faces.iter().enumerate().filter(|(fi, _)| md.face_zone.get(*fi).copied().unwrap_or(0) as usize == z).map(|(_, f)| f).collect();
                            if faces.is_empty() {
                                continue;
                            }
                            let mut part = Mesh::new();
                            let mut remap: HashMap<u32, u32> = HashMap::new();
                            for f in faces {
                                let nf: Vec<u32> = f
                                    .iter()
                                    .map(|&v| *remap.entry(v).or_insert_with(|| part.add_vertex(sm.positions[v as usize], sm.normals[v as usize], sm.uvs[v as usize])))
                                    .collect();
                                part.add_face(&nf);
                            }
                            parts.push((z as u16, std::sync::Arc::new(part)));
                        }
                        md.cache.zoned = Some((key, parts));
                    }
                    md.cache.zoned.as_ref().map(|(_, p)| p.clone()).unwrap_or_default()
                }
                _ => continue,
            };
            for (z, mesh) in parts {
                items.push(RenderItem { flags, owner, node: id, mesh, world, material: zmat.get(z as usize).copied().unwrap_or(material), selected });
            }
            continue;
        }
        let explicit_uvs = matches!(scene.node(id).map(|n| &n.data), Some(NodeData::Mesh(md)) if md.uv_set.is_some());
        let uvmap = if !explicit_uvs && textures.get(material as usize).map(|t| t.is_some()).unwrap_or(false) {
            let g = |s: &mut Scene, a: &str, d: f32| s.get_attr(id, a).unwrap_or(d);
            Some(crate::geo::edit::UvMap {
                mode: g(scene, "uvMode", 0.0) as u8,
                tile_u: g(scene, "uvTilingU", 1.0),
                tile_v: g(scene, "uvTilingV", 1.0),
                off_u: g(scene, "uvOffsetU", 0.0),
                off_v: g(scene, "uvOffsetV", 0.0),
                rot_deg: g(scene, "uvRotate", 0.0),
            })
        } else {
            None
        };
        let angle = scene.get_attr(id, "smoothAngle").unwrap_or(40.0).clamp(0.0, 180.0);
        // Smooth Mesh Preview shows the subdivided mesh in place of the cage (2 and 3)
        let preview = {
            let d = scene.get_attr(id, "displaySmoothness").unwrap_or(1.0).round();
            if d >= 2.0 {
                scene.get_attr(id, "smoothLevel").unwrap_or(2.0).round().clamp(1.0, 3.0)
            } else {
                0.0
            }
        };
        let key: [f32; 8] = match &uvmap {
            Some(u) => [u.mode as f32 + 1.0, u.tile_u, u.tile_v, u.off_u, u.off_v, u.rot_deg, angle, preview],
            None => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, angle, preview],
        };
        let mesh = match scene.nodes.get_mut(id).map(|n| &mut n.data) {
            Some(NodeData::Mesh(md)) => {
                let fresh = matches!(&md.cache.split, Some((k, _)) if *k == key);
                if !fresh {
                    let mut m = if preview > 0.0 {
                        let mut poly = crate::geo::edit::Poly::from_mesh(&md.mesh);
                        for _ in 0..preview as u32 {
                            poly = poly.smooth();
                        }
                        crate::geo::edit::shading_split(&poly.to_mesh(), angle, uvmap.as_ref())
                    } else {
                        crate::geo::edit::shading_split(&md.mesh, angle, uvmap.as_ref())
                    };
                    if preview == 0.0 {
                        if let Some(set) = &md.uv_set {
                            set.apply_to(&mut m);
                        }
                    }
                    md.cache.split = Some((key, std::sync::Arc::new(m)));
                }
                md.cache.split.as_ref().map(|(_, m)| m.clone()).unwrap()
            }
            _ => continue,
        };
        let selected = selection.contains(&id) || parent.map(|p| selection.contains(&p)).unwrap_or(false);
        items.push(RenderItem { flags, owner, node: id, mesh, world, material, selected });
    }

    let mut lights = Vec::new();
    for id in scene.ids_where(|d| matches!(d, NodeData::Light)) {
        if !scene.is_visible(id) {
            continue;
        }
        let world = scene.world_matrix(id);
        let g = |s: &mut Scene, a: &str| s.get_attr(id, a).unwrap_or(0.0);
        let kind = match g(scene, "lightType").round() as i32 {
            1 => LightKind::Directional,
            2 => LightKind::Spot,
            3 => LightKind::Ambient,
            4 => LightKind::Area,
            _ => LightKind::Point,
        };
        let mut l = RenderLight::simple(
            kind,
            world.translation_part(),
            world.transform_dir(Vec3::new(0.0, 0.0, 1.0)).normalized(),
            Vec3::new(g(scene, "colorR"), g(scene, "colorG"), g(scene, "colorB")),
            g(scene, "intensity"),
        );
        l.cone_half = (g(scene, "coneAngle") * 0.5).clamp(1.0, 89.0).to_radians();
        l.penumbra = g(scene, "penumbraAngle").clamp(0.0, 60.0).to_radians();
        l.dropoff = g(scene, "dropoff").clamp(0.0, 255.0);
        l.decay = g(scene, "decayRate").round().clamp(0.0, 4.0) as u8;
        l.shadows = g(scene, "shadows") > 0.5;
        l.softness = g(scene, "shadowSoftness").max(0.0);
        l.samples = g(scene, "shadowSamples").clamp(1.0, 64.0) as u32;
        l.shadow_color = Vec3::new(g(scene, "shadowColorR"), g(scene, "shadowColorG"), g(scene, "shadowColorB"));
        l.diffuse = g(scene, "emitDiffuse") > 0.5;
        l.specular = g(scene, "emitSpecular") > 0.5;
        l.area_x = world.transform_dir(Vec3::new(0.5 * g(scene, "areaWidth"), 0.0, 0.0));
        l.area_y = world.transform_dir(Vec3::new(0.0, 0.5 * g(scene, "areaHeight"), 0.0));
        l.node = Some(id);
        l.links = scene.light_links.iter().find(|(lid, _)| *lid == id || scene.node(id).and_then(|n| n.parent) == Some(*lid)).map(|(_, o)| o.clone());
        if g(scene, "useDepthMapShadows") > 0.5 {
            // the shadow map's texel, spread by the filter, blurs the edge of the shadow
            l.dm_blur = g(scene, "dmapFilterSize").max(0.0) * 20.0 / g(scene, "dmapResolution").clamp(32.0, 8192.0);
        }
        lights.push(l);
    }

    let mut particles = Vec::new();
    let show_particles = scene.show.particles;
    for id in scene.ids_where(|d| matches!(d, NodeData::Nucleus(_))).into_iter().filter(|_| show_particles) {
        if let Some(NodeData::Nucleus(nd)) = scene.node(id).map(|n| &n.data) {
            for p in &nd.solver.system.items {
                particles.push(RenderParticle::sphere(p.pos, p.radius));
            }
        }
    }
    if show_particles {
        for (gi, id) in scene.ids_where(|d| matches!(d, NodeData::Particle(_))).into_iter().enumerate() {
            if !scene.is_visible(id) {
                continue;
            }
            let g = |s: &Scene, k: &str, d: f32| s.node(id).and_then(|n| n.plug(k)).map(|p| p.value).unwrap_or(d);
            let (base, kind, thr, lifespan) = (g(scene, "radius", 0.5), g(scene, "renderType", 0.0) as u8, g(scene, "threshold", 1.0), g(scene, "lifespan", 1.0));
            let color0 = Vec3::new(g(scene, "colorR", 0.8), g(scene, "colorG", 0.8), g(scene, "colorB", 0.8));
            let op0 = g(scene, "opacity", 1.0);
            if let Some(NodeData::Particle(pd)) = scene.node(id).map(|n| &n.data) {
                let mat = pd.material.and_then(|m| mat_index.get(&m).copied()).unwrap_or(PARTICLE_MATERIAL);
                for p in &pd.sim.items {
                    let span = if p.life > 0.0 { p.life } else { lifespan.max(0.01) };
                    let a = (p.age / span).clamp(0.0, 1.0);
                    let scale = pd.scale_ramp.as_ref().map(|r| ramp_at(r, a).x * pd.scale_max).unwrap_or(1.0);
                    let color = pd.color_ramp.as_ref().map(|r| ramp_at(r, a)).unwrap_or(color0);
                    let opacity = pd.opacity_ramp.as_ref().map(|r| ramp_at(r, a).x).unwrap_or(op0);
                    particles.push(RenderParticle { pos: p.pos, radius: (base * scale).max(1e-4), kind, color, opacity, group: gi as u32, threshold: thr, material: mat, vel: p.vel });
                }
            }
        }
    }
    if !scene.final_render && !scene.use_all_lights {
        lights.clear();
    }
    let gizmos = build_gizmos(scene);
    RenderScene { items, lights, materials, particles, gizmos, textures, bumps, spec_maps, use_textures: true }
}

/// The texture a material's color is mapped with: its image or pattern, placement and gain.
fn texture_instance(scene: &mut Scene, mat: NodeId) -> Option<texture::TexInst> {
    texture_instance_for(scene, mat, "color")
}

/// The texture connected to a compound attribute of a material (color, specularColor, bump).
fn texture_instance_for(scene: &mut Scene, mat: NodeId, base: &str) -> Option<texture::TexInst> {
    let t = scene.texture_source(mat, base)?;
    let (kind, path) = match scene.node(t).map(|n| &n.data) {
        Some(NodeData::Texture(d)) => (d.kind.clone(), d.path.clone()),
        _ => return None,
    };
    // procedural kinds and connected inputs are baked into one image (placement and color balance included)
    if netbake::needs_network(scene, t) {
        let tex = netbake::baked(scene, t, 512)?;
        let _ = (&kind, &path);
        return Some(texture::TexInst::plain(tex));
    }
    let file = if kind == "file" { scene.texture_file(&path) } else { format!("builtin:{}", kind) };
    let tex = texture::load(&file)?;
    let g = |s: &mut Scene, a: &str| s.get_attr(t, a).unwrap_or(0.0);
    let mut inst = texture::TexInst::plain(tex);
    inst.rep = (g(scene, "repeatU"), g(scene, "repeatV"));
    inst.off = (g(scene, "offsetU"), g(scene, "offsetV"));
    inst.rot_deg = g(scene, "rotateUV");
    inst.gain = Vec3::new(g(scene, "colorGainR"), g(scene, "colorGainG"), g(scene, "colorGainB"));
    // transparency driven by the same texture's alpha
    let from_alpha = scene.node(mat).and_then(|n| n.plug("transparency")).and_then(|p| p.source.clone()).map(|s| s.node == t).unwrap_or(false);
    inst.alpha_transparency = from_alpha && inst.tex.alpha.is_some();
    Some(inst)
}

/// Shading of one surface point, shared by the viewport and the ray tracer. `light_tint` decides how
/// much of a light reaches the point (shadows): `None` skips the light.
pub fn shade_surface(m: &Material, base: Vec3, n: Vec3, view: Vec3, pos: Vec3, lights: &[RenderLight], light_tint: &dyn Fn(&RenderLight, Vec3, f32) -> Option<Vec3>) -> Vec3 {
    if m.model == 3 {
        // a surface shader is its color, unlit
        return base + m.incandescence;
    }
    let mut col = base * m.ambient + m.incandescence;
    for l in lights {
        if l.kind == LightKind::Ambient {
            if l.diffuse {
                col += base.mul_elem(l.color) * (l.intensity * m.diffuse);
            }
            continue;
        }
        let Some((ldir, dist, atten)) = l.incident(pos) else { continue };
        let ndl = n.dot(ldir);
        if ndl <= 0.0 {
            continue;
        }
        let Some(tint) = light_tint(l, ldir, dist) else { continue };
        let lc = l.color.mul_elem(tint) * (l.intensity * atten);
        if l.diffuse {
            col += base.mul_elem(lc) * (m.diffuse * ndl);
        }
        if m.model >= 1 && l.specular && m.specular > 0.0 {
            let s = if m.model == 2 {
                // Phong: the reflected light direction against the view
                let r = (n * (2.0 * ndl) - ldir).normalized();
                r.dot(view).max(0.0).powf(m.shininess.max(1.0))
            } else {
                let h = (ldir + view).normalized();
                n.dot(h).max(0.0).powf(m.shininess.max(1.0))
            };
            col += lc.mul_elem(m.spec_color) * (s * m.specular);
        }
    }
    col
}

fn build_gizmos(scene: &mut Scene) -> Vec<GizmoLine> {
    let mut g = Vec::new();
    let yellow = Vec3::new(1.0, 0.9, 0.3);
    let sel0 = scene.selection.clone();
    // a circle of radius r around `c` in the plane spanned by the unit vectors u and v
    fn ring(g: &mut Vec<GizmoLine>, c: Vec3, u: Vec3, v: Vec3, r: f32, col: Vec3) {
        let n = 24;
        for i in 0..n {
            let (a0, a1) = (i as f32 / n as f32 * std::f32::consts::TAU, (i + 1) as f32 / n as f32 * std::f32::consts::TAU);
            g.push(GizmoLine { a: c + (u * a0.cos() + v * a0.sin()) * r, b: c + (u * a1.cos() + v * a1.sin()) * r, color: col, xray: false });
        }
    }
    for id in scene.ids_where(|d| matches!(d, NodeData::Light)) {
        if !scene.is_visible(id) || !scene.show.lights {
            continue;
        }
        let w = scene.world_matrix(id);
        let p = w.translation_part();
        let parent_sel = scene.node(id).and_then(|n| n.parent).map(|t| sel0.contains(&t)).unwrap_or(false);
        let col = if parent_sel { Vec3::ONE } else { yellow };
        let s = 0.5;
        for axis in [Vec3::new(s, 0.0, 0.0), Vec3::new(0.0, s, 0.0), Vec3::new(0.0, 0.0, s)] {
            g.push(GizmoLine { a: p - axis, b: p + axis, color: col, xray: false });
        }
        let fwd = -w.transform_dir(Vec3::new(0.0, 0.0, 1.0)).normalized();
        let helper = if fwd.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
        let (u, v) = (fwd.cross(helper).normalized(), fwd.cross(fwd.cross(helper)).normalized());
        match scene.get_attr(id, "lightType").unwrap_or(0.0).round() as i32 {
            1 => {
                for k in [Vec3::ZERO, u * 0.4, -u * 0.4, v * 0.4, -v * 0.4] {
                    g.push(GizmoLine { a: p + k, b: p + k + fwd * 3.0, color: col, xray: false });
                }
            }
            2 => {
                let half = (scene.get_attr(id, "coneAngle").unwrap_or(40.0) * 0.5).clamp(1.0, 89.0).to_radians();
                let (len, r) = (3.0, 3.0 * half.tan());
                let end = p + fwd * len;
                ring(&mut g, end, u, v, r, col);
                for k in [u, -u, v, -v] {
                    g.push(GizmoLine { a: p, b: end + k * r, color: col, xray: false });
                }
                g.push(GizmoLine { a: p, b: end, color: col, xray: false });
            }
            3 => ring(&mut g, p, Vec3::new(1.0, 0.0, 0.0), Vec3::UP, 0.8, col),
            4 => {
                let (ax, ay) = (w.transform_dir(Vec3::new(0.5 * scene.get_attr(id, "areaWidth").unwrap_or(2.0), 0.0, 0.0)), w.transform_dir(Vec3::new(0.0, 0.5 * scene.get_attr(id, "areaHeight").unwrap_or(2.0), 0.0)));
                let c = [p - ax - ay, p + ax - ay, p + ax + ay, p - ax + ay];
                for i in 0..4 {
                    g.push(GizmoLine { a: c[i], b: c[(i + 1) % 4], color: col, xray: false });
                }
                g.push(GizmoLine { a: p, b: p + fwd * 2.0, color: col, xray: false });
            }
            _ => {}
        }
    }
    for id in scene.ids_where(|d| matches!(d, NodeData::Locator)) {
        if !scene.is_visible(id) || !scene.show.locators {
            continue;
        }
        let Some(t) = scene.node(id).and_then(|n| n.parent) else { continue };
        let w = scene.world_matrix(t);
        let p = w.translation_part();
        let s = scene.get_attr(id, "size").unwrap_or(1.0).max(0.01) * 0.5;
        let col = if sel0.contains(&t) { Vec3::ONE } else { Vec3::new(1.0, 0.9, 0.3) };
        for axis in [Vec3::new(1.0, 0.0, 0.0), Vec3::UP, Vec3::new(0.0, 0.0, 1.0)] {
            let a = w.transform_dir(axis).normalized() * s;
            g.push(GizmoLine { a: p - a, b: p + a, color: col, xray: true });
        }
    }
    // transform display: pivots of the selection, selection handles of every object
    if scene.show.pivots || scene.show.handles {
        let targets: Vec<NodeId> = if scene.show.handles { scene.ids_where(|d| matches!(d, NodeData::Transform)) } else { sel0.clone() };
        for id in targets {
            if !scene.node(id).map(|n| n.data.is_transform()).unwrap_or(false) || !scene.is_visible(id) || scene.name_of(id) == "persp" {
                continue;
            }
            let p = scene.pivot_world(id);
            let col = if sel0.contains(&id) { Vec3::new(1.0, 1.0, 1.0) } else { Vec3::new(1.0, 0.85, 0.3) };
            let s = 0.18;
            for axis in [Vec3::new(s, 0.0, 0.0), Vec3::new(0.0, s, 0.0), Vec3::new(0.0, 0.0, s)] {
                g.push(GizmoLine { a: p - axis, b: p + axis, color: col, xray: true });
            }
        }
    }
    // skin weights of the active joint as colored crosses on the vertices
    if let Some((sh, j)) = scene.skin_vis {
        let idx = scene.skin_data(sh).and_then(|sk| sk.joints.iter().position(|x| *x == j));
        let weights: Vec<f32> = match (idx, scene.skin_data(sh)) {
            (Some(i), Some(sk)) => sk.weights.iter().map(|w| crate::dag::skinning::weight_of(w, i as u16)).collect(),
            _ => Vec::new(),
        };
        if let Some(pts) = scene.skin_world_points(sh) {
            for (p, w) in pts.iter().zip(&weights) {
                let col = if *w < 0.5 { Vec3::new(0.1, *w * 2.0, 1.0 - *w * 2.0) } else { Vec3::new((*w - 0.5) * 2.0, 1.0 - (*w - 0.5) * 2.0, 0.0) };
                let s = 0.03 + 0.05 * w;
                for axis in [Vec3::new(s, 0.0, 0.0), Vec3::new(0.0, s, 0.0), Vec3::new(0.0, 0.0, s)] {
                    g.push(GizmoLine { a: *p - axis, b: *p + axis, color: col, xray: true });
                }
            }
        }
    }
    // NURBS: curves as lines with their control polygon when selected, isoparms on surfaces
    for id in scene.ids_where(|d| matches!(d, NodeData::NurbsCurve(_) | NodeData::Mesh(_))) {
        if !scene.is_visible(id) {
            continue;
        }
        let parent = scene.node(id).and_then(|n| n.parent);
        let selected = sel0.contains(&id) || parent.map(|p| sel0.contains(&p)).unwrap_or(false);
        let world = scene.world_matrix(parent.unwrap_or(id));
        match scene.node(id).map(|n| &n.data) {
            Some(NodeData::NurbsCurve(cd)) => {
                let col = if selected { Vec3::new(1.0, 1.0, 0.6) } else { Vec3::new(0.35, 0.85, 1.0) };
                let pts: Vec<Vec3> = cd.curve.sample(8).into_iter().map(|p| world.transform_point(p)).collect();
                for w in pts.windows(2) {
                    g.push(GizmoLine { a: w[0], b: w[1], color: col, xray: true });
                }
                if selected {
                    let cvs: Vec<Vec3> = cd.curve.cvs.iter().map(|p| world.transform_point(*p)).collect();
                    let hull = Vec3::new(0.6, 0.6, 0.9);
                    for w in cvs.windows(2) {
                        g.push(GizmoLine { a: w[0], b: w[1], color: hull, xray: true });
                    }
                    for c in &cvs {
                        let s = 0.06;
                        for axis in [Vec3::new(s, 0.0, 0.0), Vec3::new(0.0, s, 0.0), Vec3::new(0.0, 0.0, s)] {
                            g.push(GizmoLine { a: *c - axis, b: *c + axis, color: Vec3::new(1.0, 0.6, 0.2), xray: true });
                        }
                    }
                }
            }
            Some(NodeData::Mesh(md)) if md.stroke.is_some() => {
                if scene.node(id).and_then(|n| n.plug("drawAsMesh")).map(|p| p.value < 0.5).unwrap_or(false) {
                    let col = if selected { Vec3::new(0.95, 0.95, 1.0) } else { Vec3::new(0.3, 0.8, 0.35) };
                    // each tube's centre line is the ring centres; quads run along the tube, so draw their long edges
                    for f in &md.mesh.faces {
                        if f.len() == 4 {
                            g.push(GizmoLine { a: world.transform_point(md.mesh.positions[f[0] as usize]), b: world.transform_point(md.mesh.positions[f[3] as usize]), color: col, xray: false });
                        }
                    }
                }
            }
            Some(NodeData::Mesh(md)) if md.nurbs.is_some() => {
                let ns = md.nurbs.as_ref().unwrap();
                let col = if selected { Vec3::new(0.95, 0.95, 1.0) } else { Vec3::new(0.25, 0.45, 0.85) };
                for line in &ns.iso {
                    for w in line.windows(2) {
                        g.push(GizmoLine { a: world.transform_point(w[0]), b: world.transform_point(w[1]), color: col, xray: false });
                    }
                }
            }
            _ => {}
        }
    }
    // cage of meshes shown with Smooth Mesh Preview 2, and polygon face normals
    for id in scene.ids_where(|d| matches!(d, NodeData::Mesh(_))) {
        if !scene.is_visible(id) {
            continue;
        }
        let cage = scene.get_attr(id, "displaySmoothness").unwrap_or(1.0).round() as i32 == 2;
        if !cage && !scene.show_normals {
            continue;
        }
        let world = scene.world_matrix(id);
        if let Some(NodeData::Mesh(md)) = scene.node(id).map(|n| &n.data) {
            let poly = crate::geo::edit::Poly::from_mesh(&md.mesh);
            if cage {
                for e in poly.edge_list() {
                    g.push(GizmoLine { a: world.transform_point(poly.pos[e.0 as usize]), b: world.transform_point(poly.pos[e.1 as usize]), color: Vec3::new(0.35, 0.8, 1.0), xray: false });
                }
            }
            if scene.show_normals {
                for f in 0..poly.faces.len() {
                    let c = poly.face_center(f);
                    let n = poly.face_normal(f).normalized();
                    g.push(GizmoLine { a: world.transform_point(c), b: world.transform_point(c + n * 0.3), color: Vec3::new(0.2, 1.0, 0.4), xray: false });
                }
            }
        }
    }
    if scene.show_borders {
        let red = Vec3::new(1.0, 0.25, 0.2);
        for id in scene.ids_where(|d| matches!(d, NodeData::Mesh(_))) {
            if !scene.is_visible(id) {
                continue;
            }
            let world = scene.world_matrix(id);
            if let Some(NodeData::Mesh(md)) = scene.node(id).map(|n| &n.data) {
                let poly = crate::geo::edit::Poly::from_mesh(&md.mesh);
                for (e, faces) in poly.edge_faces() {
                    if faces.len() == 1 {
                        g.push(GizmoLine { a: world.transform_point(poly.pos[e.0 as usize]), b: world.transform_point(poly.pos[e.1 as usize]), color: red, xray: true });
                    }
                }
            }
        }
    }
    // particle emitters: a cone along the emission direction, as wide as the spread
    for id in scene.ids_where(|d| matches!(d, NodeData::Emitter)) {
        if !scene.is_visible(id) || !scene.show.emitters {
            continue;
        }
        let Some(t) = scene.node(id).and_then(|n| n.parent) else { continue };
        let w = scene.world_matrix(t);
        let p = w.translation_part();
        let d = w.transform_dir(Vec3::UP).normalized();
        let helper = if d.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
        let (u, v) = (d.cross(helper).normalized(), d.cross(d.cross(helper)).normalized());
        let col = if sel0.contains(&t) { Vec3::ONE } else { Vec3::new(1.0, 0.65, 0.25) };
        let len = 1.6;
        let r = len * scene.get_attr(id, "spread").unwrap_or(0.35).clamp(0.02, 1.5);
        let end = p + d * len;
        ring(&mut g, end, u, v, r, col);
        ring(&mut g, p, u, v, 0.25, col);
        for k in [u, -u, v, -v] {
            g.push(GizmoLine { a: p, b: end + k * r, color: col, xray: true });
        }
        g.push(GizmoLine { a: p, b: end, color: col, xray: true });
    }
    // the live surface shows as a green wireframe
    if let Some(live) = scene.live.filter(|l| scene.node(*l).is_some()) {
        let green = Vec3::new(0.2, 0.9, 0.3);
        for sh in scene.mesh_shapes_under(live) {
            let owner = scene.node(sh).and_then(|n| n.parent).unwrap_or(sh);
            let w = scene.world_matrix(owner);
            if let Some(NodeData::Mesh(md)) = scene.node(sh).map(|n| &n.data) {
                for f in &md.mesh.faces {
                    for k in 0..f.len() {
                        let (a, b) = (f[k] as usize, f[(k + 1) % f.len()] as usize);
                        if a < b {
                            g.push(GizmoLine { a: w.transform_point(md.mesh.positions[a]), b: w.transform_point(md.mesh.positions[b]), color: green, xray: false });
                        }
                    }
                }
            }
        }
    }
    // dynamics helpers: particle emitters, fields, rigid bodies' centers of mass and constraints
    for id in scene.ids_where(|d| matches!(d, NodeData::PEmitter(_) | NodeData::Field(_) | NodeData::RigidBody(_) | NodeData::RigidJoint(_))) {
        if !scene.is_visible(id) || !scene.show.emitters {
            continue;
        }
        let Some(t) = scene.node(id).and_then(|n| n.parent) else { continue };
        let w = scene.world_matrix(t);
        let p = w.translation_part();
        let col = if sel0.contains(&t) { Vec3::ONE } else { Vec3::new(1.0, 0.65, 0.25) };
        let kind = scene.node(id).map(|n| std::mem::discriminant(&n.data));
        let _ = kind;
        let cross = |g: &mut Vec<GizmoLine>, c: Vec3, s: f32, col: Vec3| {
            for a in [Vec3::new(s, 0.0, 0.0), Vec3::new(0.0, s, 0.0), Vec3::new(0.0, 0.0, s)] {
                g.push(GizmoLine { a: c - a, b: c + a, color: col, xray: true });
            }
        };
        match scene.node(id).map(|n| &n.data) {
            Some(NodeData::PEmitter(_)) => {
                let ty = scene.get_attr(id, "emitterType").unwrap_or(0.0).round() as i32;
                let d = w.transform_dir(Vec3::new(scene.get_attr(id, "directionX").unwrap_or(0.0), scene.get_attr(id, "directionY").unwrap_or(1.0), scene.get_attr(id, "directionZ").unwrap_or(0.0))).normalized();
                cross(&mut g, p, 0.4, col);
                ring(&mut g, p, Vec3::new(1.0, 0.0, 0.0), Vec3::UP, 0.3, col);
                if ty == 1 || ty == 4 {
                    g.push(GizmoLine { a: p, b: p + d * 1.2, color: col, xray: true });
                }
            }
            Some(NodeData::Field(_)) => {
                let col = if sel0.contains(&t) { Vec3::ONE } else { Vec3::new(0.4, 0.7, 1.0) };
                let d = w.transform_dir(Vec3::new(scene.get_attr(id, "directionX").unwrap_or(0.0), scene.get_attr(id, "directionY").unwrap_or(-1.0), scene.get_attr(id, "directionZ").unwrap_or(0.0))).normalized();
                ring(&mut g, p, Vec3::new(1.0, 0.0, 0.0), Vec3::UP, 0.5, col);
                ring(&mut g, p, Vec3::UP, Vec3::new(0.0, 0.0, 1.0), 0.5, col);
                g.push(GizmoLine { a: p, b: p + d * 1.6, color: col, xray: true });
                let helper = if d.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
                let u = d.cross(helper).normalized();
                for k in [u, -u] {
                    g.push(GizmoLine { a: p + d * 1.6, b: p + d * 1.3 + k * 0.2, color: col, xray: true });
                }
            }
            Some(NodeData::RigidJoint(_)) => cross(&mut g, p, 0.4, Vec3::new(0.9, 0.5, 0.9)),
            Some(NodeData::RigidBody(_)) => {
                // the center of mass shows as a small x, only for selected bodies and while the bodies are not hidden
                if sel0.contains(&t) {
                    let off = Vec3::new(scene.get_attr(id, "centerOfMassX").unwrap_or(0.0), scene.get_attr(id, "centerOfMassY").unwrap_or(0.0), scene.get_attr(id, "centerOfMassZ").unwrap_or(0.0));
                    // measured from the center of the object's bounding box, like the simulation does
                    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                    for sh in scene.mesh_shapes_under(t) {
                        if let Some(NodeData::Mesh(md)) = scene.node(sh).map(|n| &n.data) {
                            for v in &md.mesh.positions {
                                lo = Vec3::new(lo.x.min(v.x), lo.y.min(v.y), lo.z.min(v.z));
                                hi = Vec3::new(hi.x.max(v.x), hi.y.max(v.y), hi.z.max(v.z));
                            }
                        }
                    }
                    let center = if lo.x.is_finite() { (lo + hi) * 0.5 } else { Vec3::ZERO };
                    cross(&mut g, w.transform_point(center + off), 0.12, Vec3::new(1.0, 0.3, 0.3));
                }
            }
            _ => {}
        }
    }
    // lattices and flexors: the grid of their current points
    if scene.show.deformers {
        for lat in scene.ids_where(|d| matches!(d, NodeData::Lattice(_))) {
            let Some(t) = scene.node(lat).and_then(|n| n.parent) else { continue };
            if !scene.is_visible(t) {
                continue;
            }
            let divs = match scene.node(lat).map(|n| &n.data) {
                Some(NodeData::Lattice(d)) => d.divs,
                _ => continue,
            };
            let pts = scene.lattice_points(lat);
            let w = scene.world_matrix(t);
            let ld = crate::dag::nodes::LatticeData { divs, skin: None };
            if pts.len() != ld.count() {
                continue;
            }
            let col = if scene.selection.contains(&t) || scene.selection.contains(&lat) { Vec3::new(1.0, 1.0, 1.0) } else { Vec3::new(0.95, 0.55, 0.85) };
            for i in 0..divs[0] {
                for j in 0..divs[1] {
                    for k in 0..divs[2] {
                        let a = w.transform_point(pts[ld.index(i, j, k)]);
                        for (ni, nj, nk) in [(i + 1, j, k), (i, j + 1, k), (i, j, k + 1)] {
                            if ni < divs[0] && nj < divs[1] && nk < divs[2] {
                                g.push(GizmoLine { a, b: w.transform_point(pts[ld.index(ni, nj, nk)]), color: col, xray: false });
                            }
                        }
                    }
                }
            }
        }
    }
    // skeleton: joint crosses and tapered bones, drawn on top
    let sel = scene.selection.clone();
    let xj = scene.show.xray_joints;
    for id in scene.ids_where(|d| matches!(d, NodeData::Joint)) {
        if !scene.is_visible(id) || !scene.show.joints {
            continue;
        }
        let w = scene.world_matrix(id);
        let p = w.translation_part();
        let r = scene.get_attr(id, "radius").unwrap_or(0.5).max(0.01) * 0.5 * scene.joint_scale.max(0.01);
        let selected = sel.contains(&id);
        // local rotation axes of the selected joints (and their children)
        if scene.show.local_axes && (selected || scene.node(id).and_then(|n| n.parent).map(|p| sel.contains(&p)).unwrap_or(false)) {
            let (_, rot, _) = w.decompose();
            let q = Quat::from_euler_deg(rot);
            for (axis, c) in [(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.2, 0.2)), (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.2, 1.0, 0.2)), (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.3, 0.5, 1.0))] {
                g.push(GizmoLine { a: p, b: p + q.rotate(axis) * r * 3.0, color: c, xray: xj });
            }
        }
        let col = if selected { Vec3::new(1.0, 1.0, 1.0) } else { Vec3::new(0.55, 1.0, 0.3) };
        for (u, v) in [
            (Vec3::new(r, 0.0, 0.0), Vec3::new(0.0, r, 0.0)),
            (Vec3::new(0.0, r, 0.0), Vec3::new(0.0, 0.0, r)),
            (Vec3::new(r, 0.0, 0.0), Vec3::new(0.0, 0.0, r)),
        ] {
            // a circle in the plane spanned by u and v
            let n = 16;
            for i in 0..n {
                let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
                let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
                g.push(GizmoLine {
                    a: p + u * a0.cos() + v * a0.sin(),
                    b: p + u * a1.cos() + v * a1.sin(),
                    color: col,
                    xray: xj,
                });
            }
        }
        // bone to the parent joint
        if let Some(par) = scene.node(id).and_then(|n| n.parent) {
            if scene.is_joint(par) {
                let pp = scene.world_matrix(par).translation_part();
                let dir = p - pp;
                let len = dir.length();
                if len > 1e-4 {
                    let d = dir * (1.0 / len);
                    let helper = if d.y.abs() < 0.9 { Vec3::new(0.0, 1.0, 0.0) } else { Vec3::new(1.0, 0.0, 0.0) };
                    let u = d.cross(helper).normalized() * (len * 0.08).min(r * 1.2);
                    let v = d.cross(u).normalized() * u.length();
                    let base = pp + d * (r.min(len * 0.45));
                    let ring = [base + u, base + v, base - u, base - v];
                    for i in 0..4 {
                        g.push(GizmoLine { a: ring[i], b: p - d * r.min(len * 0.45), color: col, xray: xj });
                        g.push(GizmoLine { a: ring[i], b: ring[(i + 1) % 4], color: col, xray: xj });
                    }
                }
            }
        }
    }
    for id in scene.ids_where(|d| matches!(d, NodeData::IkHandle(_))) {
        if !scene.is_visible(id) || !scene.show.ik_handles {
            continue;
        }
        let p = scene.world_matrix(id).translation_part();
        let col = if sel.contains(&id) { Vec3::new(1.0, 1.0, 1.0) } else { Vec3::new(0.2, 0.9, 1.0) };
        let s = 0.35;
        for axis in [Vec3::new(s, 0.0, 0.0), Vec3::new(0.0, s, 0.0), Vec3::new(0.0, 0.0, s)] {
            g.push(GizmoLine { a: p - axis, b: p + axis, color: col, xray: true });
        }
    }
    let cams = scene.ids_where(|d| matches!(d, NodeData::Camera | NodeData::StereoCamera));
    for id in cams {
        let tr = match scene.node(id).and_then(|n| n.parent) {
            Some(t) => t,
            None => continue,
        };
        if scene.name_of(tr) == "persp" || !scene.is_visible(id) || !scene.show.cameras {
            continue;
        }
        let w = scene.world_matrix(tr);
        let fov = scene.get_attr(id, "fieldOfView").unwrap_or(45.0).clamp(5.0, 120.0);
        let stereo = matches!(scene.node(id).map(|n| &n.data), Some(NodeData::StereoCamera));
        let ia = if stereo { scene.get_attr(id, "interaxialSeparation").unwrap_or(0.0) } else { 0.0 };
        let offsets: Vec<f32> = if stereo { vec![-ia * 0.5, ia * 0.5] } else { vec![0.0] };
        let color = if stereo { Vec3::new(0.9, 0.5, 0.9) } else { Vec3::new(0.6, 0.85, 1.0) };
        for off in offsets {
            let d = 1.4;
            let hh = d * (fov.to_radians() * 0.5).tan() * 0.7;
            let hw = hh * 1.5;
            let apex = w.transform_point(Vec3::new(off, 0.0, 0.0));
            let c = [
                w.transform_point(Vec3::new(off - hw, -hh, -d)),
                w.transform_point(Vec3::new(off + hw, -hh, -d)),
                w.transform_point(Vec3::new(off + hw, hh, -d)),
                w.transform_point(Vec3::new(off - hw, hh, -d)),
            ];
            for i in 0..4 {
                g.push(GizmoLine { a: apex, b: c[i], color, xray: false });
                g.push(GizmoLine { a: c[i], b: c[(i + 1) % 4], color, xray: false });
            }
        }
    }
    g
}

/// Look up a camera by transform or shape name. `None` picks `persp`, then any camera.
pub fn camera_info(scene: &mut Scene, name: Option<&str>) -> Result<CameraInfo, String> {
    let shape_ids = scene.ids_where(|d| matches!(d, NodeData::Camera | NodeData::StereoCamera));
    let pick = |scene: &Scene, nm: &str| -> Option<NodeId> {
        let id = scene.find(nm)?;
        if shape_ids.contains(&id) {
            return Some(id);
        }
        scene.node(id)?.children.iter().copied().find(|c| shape_ids.contains(c))
    };
    let shape = match name {
        Some(n) => pick(scene, n).ok_or_else(|| format!("No camera matches name: {}", n))?,
        None => pick(scene, "persp")
            .or_else(|| shape_ids.first().copied())
            .ok_or_else(|| "The scene has no camera".to_string())?,
    };
    let is_stereo = matches!(scene.node(shape).map(|n| &n.data), Some(NodeData::StereoCamera));
    let g = |s: &mut Scene, a: &str, d: f32| s.get_attr(shape, a).unwrap_or(d);
    let fov = g(scene, "fieldOfView", 45.0).clamp(1.0, 170.0);
    let near = g(scene, "nearClip", 0.1).max(1e-4);
    let far = g(scene, "farClip", 1000.0).max(near + 1e-3);
    let stereo = if is_stereo {
        Some(StereoParams {
            interaxial: g(scene, "interaxialSeparation", 0.4),
            zero_parallax: g(scene, "zeroParallax", 10.0).max(near * 1.01),
        })
    } else {
        None
    };
    let transform = scene.node(shape).and_then(|n| n.parent).unwrap_or(shape);
    let world = scene.world_matrix(transform);
    let fit = g(scene, "filmFit", 2.0).round().clamp(0.0, 3.0) as u8;
    Ok(CameraInfo { name: scene.name_of(transform), world, fov_deg: fov, near, far, stereo, fit })
}

#[cfg(test)]
mod light_tests {
    use super::*;

    fn at(kind: LightKind) -> RenderLight {
        // light 10 units above the origin shining down
        let mut l = RenderLight::simple(kind, Vec3::new(0.0, 10.0, 0.0), Vec3::UP, Vec3::ONE, 1.0);
        l.axis = Vec3::new(0.0, -1.0, 0.0);
        l
    }

    #[test]
    fn decay_modes() {
        let p = Vec3::new(0.0, 0.0, 0.0);
        let mut l = at(LightKind::Point);
        l.decay = 0;
        assert_eq!(l.incident(p).unwrap().2, 1.0);
        l.decay = 1;
        assert!((l.incident(p).unwrap().2 - 0.1).abs() < 1e-5);
        l.decay = 2;
        assert!((l.incident(p).unwrap().2 - 0.01).abs() < 1e-5);
        l.decay = 3;
        assert!((l.incident(p).unwrap().2 - 0.001).abs() < 1e-6);
    }

    #[test]
    fn spot_cone_and_penumbra() {
        let mut l = at(LightKind::Spot);
        l.decay = 0;
        l.cone_half = 20f32.to_radians();
        let inside = Vec3::new(3.0, 0.0, 0.0); // about 16.7 degrees off axis
        let outside = Vec3::new(5.0, 0.0, 0.0); // about 26.6 degrees
        assert!(l.incident(inside).is_some());
        assert!(l.incident(outside).is_none(), "hard cone edge");
        l.penumbra = 15f32.to_radians();
        let soft = l.incident(outside).expect("the penumbra extends the cone").2;
        assert!(soft > 0.0 && soft < 1.0, "{}", soft);
        let core = l.incident(Vec3::ZERO).unwrap().2;
        assert!((core - 1.0).abs() < 1e-4);
        // a higher dropoff darkens the edge of the cone
        let edge_before = l.incident(inside).unwrap().2;
        l.dropoff = 10.0;
        assert!(l.incident(inside).unwrap().2 < edge_before);
    }

    #[test]
    fn ambient_has_no_direction_and_area_is_one_sided() {
        assert!(at(LightKind::Ambient).incident(Vec3::ZERO).is_none());
        let mut a = at(LightKind::Area);
        a.decay = 0;
        assert!(a.incident(Vec3::ZERO).is_some());
        assert!(a.incident(Vec3::new(0.0, 20.0, 0.0)).is_none(), "behind the emitter");
    }
}

/// The viewport snapshot of a session's scene, for tests that look at the display meshes.
#[cfg(test)]
pub fn scene_snapshot_for_tests(s: &mut crate::session::Session) -> RenderScene {
    build_scene(&mut s.scene)
}
