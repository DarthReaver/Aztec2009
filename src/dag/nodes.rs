use super::graph::NodeId;
use super::plugs::Plug;
use crate::geo::{primitives, Mesh};
use crate::math::Mat4;
use crate::nucleus::Solver;

/// Kinds of construction history nodes (Maya: polyCube, polyExtrudeFace, polyTweak, ...).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PolyKind {
    Cube,
    Sphere,
    Cylinder,
    Plane,
    Cone,
    Torus,
    /// Frozen geometry that starts a history chain (what remains after Delete History).
    Source,
    Tweak,
    Extrude,
    ExtrudeEdge,
    Bevel,
    DelFace,
    DelEdge,
    DelVertex,
    Merge,
    Collapse,
    Smooth,
    Triangulate,
    Reverse,
    Mirror,
    InsertLoop,
    Split,
    FillHole,
    /// Smooth skin binding: deforms the geometry before it with joint transforms.
    Skin,
    /// Non-linear deformers (Maya: bend, twist, flare, squash, sine).
    Bend,
    Twist,
    Flare,
    Squash,
    Sine,
    /// Average Vertices (Laplacian smoothing).
    Average,
    /// Blend shape: moves the vertices toward a stored target shape by `weight`.
    Blend,
    /// Merge Edges: joins pairs of selected border edges into single edges.
    MergeEdge,
    /// Bridge: a strip of faces between two border edge chains.
    Bridge,
    /// Extract / Duplicate Face: detach (or copy) faces as their own shell.
    ChipOff,
    /// Freeze Transformations: bakes a matrix into the vertices.
    Transform,
    /// Lattice deformer (FFD): moves the points with a lattice shape.
    Lattice,
    /// Wave deformer: concentric ripples.
    Wave,
    /// Wire deformer: a curve drags the geometry near it.
    Wire,
    /// Cluster: a handle moves, rotates and scales a group of points.
    Cluster,
    /// Soft Modification: a handle pulls the points around its center with a falloff.
    SoftMod,
    /// Jiggle: points lag behind their motion on a spring.
    Jiggle,
    /// Offset Edge Loop: a loop on each side of the selected one.
    OffsetLoop,
    /// Slide Edge: move an edge loop along the surface.
    SlideEdge,
    /// Mirror Cut: cut at a plane and mirror the kept half.
    MirrorCut,
}

impl PolyKind {
    pub fn name(self) -> &'static str {
        match self {
            PolyKind::Cube => "cube",
            PolyKind::Sphere => "sphere",
            PolyKind::Cylinder => "cylinder",
            PolyKind::Plane => "plane",
            PolyKind::Cone => "cone",
            PolyKind::Torus => "torus",
            PolyKind::Source => "source",
            PolyKind::Tweak => "tweak",
            PolyKind::Extrude => "extrude",
            PolyKind::ExtrudeEdge => "extrudeEdge",
            PolyKind::Bevel => "bevel",
            PolyKind::DelFace => "delFace",
            PolyKind::DelEdge => "delEdge",
            PolyKind::DelVertex => "delVertex",
            PolyKind::Merge => "merge",
            PolyKind::Collapse => "collapse",
            PolyKind::Smooth => "smooth",
            PolyKind::Triangulate => "triangulate",
            PolyKind::Reverse => "reverse",
            PolyKind::Mirror => "mirror",
            PolyKind::InsertLoop => "insertLoop",
            PolyKind::Split => "split",
            PolyKind::FillHole => "fillHole",
            PolyKind::Skin => "skin",
            PolyKind::Bend => "bend",
            PolyKind::Twist => "twist",
            PolyKind::Flare => "flare",
            PolyKind::Squash => "squash",
            PolyKind::Sine => "sine",
            PolyKind::Average => "average",
            PolyKind::Blend => "blend",
            PolyKind::MergeEdge => "mergeEdge",
            PolyKind::Bridge => "bridge",
            PolyKind::Lattice => "latticeDeform",
            PolyKind::Wave => "wave",
            PolyKind::Wire => "wire",
            PolyKind::Cluster => "cluster",
            PolyKind::SoftMod => "softMod",
            PolyKind::Jiggle => "jiggle",
            PolyKind::OffsetLoop => "offsetLoop",
            PolyKind::SlideEdge => "slideEdge",
            PolyKind::MirrorCut => "mirrorCut",
            PolyKind::ChipOff => "chipOff",
            PolyKind::Transform => "transform",
        }
    }

    pub fn from_name(s: &str) -> Option<PolyKind> {
        use PolyKind::*;
        [
            Cube, Sphere, Cylinder, Plane, Source, Tweak, Extrude, ExtrudeEdge, Bevel, DelFace, DelEdge, DelVertex, Merge,
            Collapse, Smooth, Triangulate, Reverse, Mirror, InsertLoop, Split, FillHole, Skin, Cone, Torus, Bend, Twist, Flare,
            Squash, Sine, Average, ChipOff, Transform, Blend, MergeEdge, Bridge, Lattice, Wave, Wire, Cluster, SoftMod, Jiggle, OffsetLoop, SlideEdge, MirrorCut,
        ]
        .into_iter()
        .find(|k| k.name() == s)
    }

    pub fn is_generator(self) -> bool {
        matches!(self, PolyKind::Cube | PolyKind::Sphere | PolyKind::Cylinder | PolyKind::Plane | PolyKind::Cone | PolyKind::Torus | PolyKind::Source)
    }

    /// Node name stem, like Maya's polyCube1 / polyExtrudeFace1.
    pub fn node_base(self) -> &'static str {
        match self {
            PolyKind::Cube => "polyCube",
            PolyKind::Sphere => "polySphere",
            PolyKind::Cylinder => "polyCylinder",
            PolyKind::Plane => "polyPlane",
            PolyKind::Cone => "polyCone",
            PolyKind::Torus => "polyTorus",
            PolyKind::Source => "polySource",
            PolyKind::Tweak => "polyTweak",
            PolyKind::Extrude => "polyExtrudeFace",
            PolyKind::ExtrudeEdge => "polyExtrudeEdge",
            PolyKind::Bevel => "polyBevel",
            PolyKind::DelFace => "polyDelFacet",
            PolyKind::DelEdge => "polyDelEdge",
            PolyKind::DelVertex => "polyDelVertex",
            PolyKind::Merge => "polyMergeVertex",
            PolyKind::Collapse => "polyCollapse",
            PolyKind::Smooth => "polySmoothFace",
            PolyKind::Triangulate => "polyTriangulate",
            PolyKind::Reverse => "polyNormal",
            PolyKind::Mirror => "polyMirror",
            PolyKind::InsertLoop => "polySplitRing",
            PolyKind::Split => "polySplit",
            PolyKind::FillHole => "polyCloseBorder",
            PolyKind::Skin => "skinCluster",
            PolyKind::Bend => "bend",
            PolyKind::Twist => "twist",
            PolyKind::Flare => "flare",
            PolyKind::Squash => "squash",
            PolyKind::Sine => "sine",
            PolyKind::Average => "polyAverageVertex",
            PolyKind::Blend => "blendShape",
            PolyKind::MergeEdge => "polyMergeEdge",
            PolyKind::Bridge => "polyBridgeEdge",
            PolyKind::Lattice => "ffd",
            PolyKind::Wave => "wave",
            PolyKind::Wire => "wire",
            PolyKind::Cluster => "cluster",
            PolyKind::SoftMod => "softMod",
            PolyKind::Jiggle => "jiggle",
            PolyKind::OffsetLoop => "polyOffsetLoop",
            PolyKind::SlideEdge => "polySlideEdge",
            PolyKind::MirrorCut => "polyMirrorCut",
            PolyKind::ChipOff => "polyChipOff",
            PolyKind::Transform => "transformGeometry",
        }
    }
}

/// One construction history node. `ints` holds the component indices the operation was applied to
/// (faces, vertices, or flattened edge pairs).
/// Smooth skin binding data (Maya: skinCluster).
#[derive(Clone, Debug, Default)]
pub struct SkinData {
    pub joints: Vec<NodeId>,
    /// Joint names while a file is loading; resolved to `joints` afterwards.
    pub joint_names: Vec<String>,
    /// Inverse world matrix of every joint at bind time.
    pub bind_inv: Vec<Mat4>,
    /// World matrix of the mesh at bind time.
    pub bind_mesh_world: Mat4,
    /// Per vertex influences: (joint index, weight).
    pub weights: Vec<Vec<(u16, f32)>>,
    /// Bind options kept so the default weights can be recomputed: influences per vertex (0 = four),
    /// dropoff (0 = 4) and rigid binding.
    pub max_inf: u8,
    pub dropoff: f32,
    pub rigid: bool,
}

/// IK handle: solves the chain from `start` down to `end` toward the handle position.
#[derive(Clone, Debug, Default)]
pub struct IkData {
    pub start: Option<NodeId>,
    pub end: Option<NodeId>,
    pub start_name: String,
    pub end_name: String,
    /// 0 rotate plane (two bone chains are solved exactly, with a pole vector and twist), 1 single chain, 2 spline.
    pub solver: u8,
    /// Spline IK: the curve the chain follows.
    pub curve: Option<NodeId>,
    pub curve_name: String,
    /// Pole vector constraint target: the bend of a rotate plane chain points toward it.
    pub pole: Option<NodeId>,
    pub pole_name: String,
}

#[derive(Clone, Debug)]
pub struct PolyOpData {
    pub skin: Option<SkinData>,
    pub kind: PolyKind,
    pub ints: Vec<u32>,
    pub source: Option<crate::geo::edit::Poly>,
    pub owner: Option<NodeId>,
    pub cache: Option<crate::geo::edit::Poly>,
    pub out: crate::geo::edit::Sel,
    pub dirty: bool,
    /// Lattice deformation: the lattice shape that moves the points.
    pub lattice: Option<NodeId>,
    pub lattice_name: String,
    /// Handles, curves and state of the point deformers (wire, cluster, soft mod, jiggle).
    pub ext: Option<Box<DeformExt>>,
}

/// Spring state of a jiggle deformer between frames.
#[derive(Clone, Debug, Default)]
pub struct JiggleState {
    pub t: f32,
    pub target: Vec<crate::math::Vec3>,
    pub offset: Vec<crate::math::Vec3>,
    pub vel: Vec<crate::math::Vec3>,
}

/// What a point deformer needs besides its channels.
#[derive(Clone, Debug, Default)]
pub struct DeformExt {
    pub handle: Option<NodeId>,
    pub handle_name: String,
    /// Inverse world matrix of the handle when the deformer was made.
    pub bind_inv: Option<Mat4>,
    pub curve: Option<NodeId>,
    pub curve_name: String,
    /// The wire's curve at bind time (world space).
    pub base_curve: Option<crate::geo::nurbs::Curve>,
    /// Soft modification center (mesh local space) at bind time.
    pub center: Option<crate::math::Vec3>,
    /// Per point weights (jiggle weights, cluster weights); points without an entry weigh 1.
    pub weights: Vec<f32>,
    pub jiggle: Option<JiggleState>,
}

impl PolyOpData {
    pub fn new(kind: PolyKind) -> PolyOpData {
        PolyOpData { skin: None, kind, ints: Vec::new(), source: None, owner: None, cache: None, out: Default::default(), dirty: true, lattice: None, lattice_name: String::new(), ext: None }
    }
}

/// Derived data kept next to a mesh; dropped whenever the mesh changes.
#[derive(Clone, Debug, Default)]
pub struct MeshCache {
    pub version: u64,
    pub edge_count: Option<usize>,
    /// Display mesh (shading split) with the key it was built for.
    pub split: Option<([f32; 8], std::sync::Arc<Mesh>)>,
    /// Display meshes of a mesh with texture zones: one per zone, with the key they were built for.
    pub zoned: Option<(u64, Vec<(u16, std::sync::Arc<Mesh>)>)>,
}

/// A group of faces with its own material and texture mapping (Maya: a shading group on faces plus a
/// projection). Face `f` of a mesh belongs to zone `face_zone[f]`; 0 is the base zone, which uses the
/// shape's `material` and its `uv*` attributes, and `k > 0` is `zones[k - 1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub material: Option<NodeId>,
    pub uv: crate::geo::edit::UvMap,
}

/// Quantized, order independent fingerprint of a face (used to carry zones across topology edits).
fn face_signature(m: &Mesh, f: &[u32]) -> u64 {
    let mut keys: Vec<(i64, i64, i64)> = f
        .iter()
        .map(|&v| {
            let p = m.positions.get(v as usize).copied().unwrap_or(crate::math::Vec3::ZERO);
            ((p.x * 2000.0).round() as i64, (p.y * 2000.0).round() as i64, (p.z * 2000.0).round() as i64)
        })
        .collect();
    keys.sort_unstable();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for k in keys {
        for x in [k.0, k.1, k.2] {
            h ^= x as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// A Paint Effects stroke: the path it was painted along and the brush that grows tubes along it.
#[derive(Clone, Debug, Default)]
pub struct StrokeData {
    pub brush: Option<NodeId>,
    pub brush_name: String,
    /// World positions and surface normals along the stroke.
    pub path: Vec<(crate::math::Vec3, crate::math::Vec3)>,
    pub seed: u32,
    pub key: u64,
    /// Materials of the stem, leaves and flowers.
    pub mats: [Option<NodeId>; 3],
    /// Names to resolve after a file load.
    pub mat_names: Vec<String>,
}

// ---------------------------------------------------------------------------------------------- dynamics

/// Position, orientation and motion of a rigid body's center of mass.
#[derive(Clone, Debug)]
pub struct BodyState {
    pub pos: crate::math::Vec3,
    pub rot: crate::math::Quat,
    pub vel: crate::math::Vec3,
    pub spin: crate::math::Vec3,
}

/// A rigid body node under the transform it simulates; its settings are plugs.
#[derive(Clone, Debug, Default)]
pub struct RigidData {
    /// Initial State: where the body starts (Solvers > Initial State).
    pub initial: Option<BodyState>,
}

/// The rigid body solver (one per scene): its settings are plugs, the simulation state lives here.
#[derive(Clone, Debug, Default)]
pub struct DynSolverData {
    pub rt: Box<super::dynamics::DynRuntime>,
}

/// A dynamic field; `targets` are the transforms and particle shapes it affects.
#[derive(Clone, Debug, Default)]
pub struct FieldData {
    pub kind: u8,
    pub targets: Vec<NodeId>,
    pub target_names: Vec<String>,
}

/// Rigid body constraint: 0 nail, 1 pin, 2 hinge, 3 spring, 4 barrier.
#[derive(Clone, Debug, Default)]
pub struct RigidJointData {
    pub kind: u8,
    pub a: Option<NodeId>,
    pub b: Option<NodeId>,
    pub names: Vec<String>,
}

/// A particle emitter. `target` is the particle shape it fills, `source` the object whose surface or curve it emits from.
#[derive(Clone, Debug, Default)]
pub struct PEmitterData {
    pub target: Option<NodeId>,
    pub source: Option<NodeId>,
    pub names: Vec<String>,
    pub accum: f32,
    pub prev_world: Option<Mat4>,
}

#[derive(Clone, Debug)]
pub struct ColliderRef {
    pub node: Option<NodeId>,
    pub name: String,
    pub resilience: f32,
    pub friction: f32,
}

/// Collision event: 0 emit, 1 split.
#[derive(Clone, Debug)]
pub struct ParticleEvent {
    pub kind: u8,
    pub num: u32,
    pub random: bool,
    pub spread: f32,
    pub target: Option<NodeId>,
    pub target_name: String,
    pub inherit: f32,
    pub original_dies: bool,
}

/// A ramp of values along the normalized age of a particle: (position, value).
pub type Ramp = Vec<(f32, crate::math::Vec3)>;

#[derive(Clone, Debug)]
pub struct ParticleData {
    pub sim: crate::nucleus::fields::PSim,
    pub initial: Option<crate::nucleus::fields::PSim>,
    pub colliders: Vec<ColliderRef>,
    pub events: Vec<ParticleEvent>,
    pub scale_ramp: Option<Ramp>,
    pub scale_max: f32,
    pub color_ramp: Option<Ramp>,
    pub opacity_ramp: Option<Ramp>,
    pub material: Option<NodeId>,
    pub material_name: String,
    /// Frame the particles were last simulated to.
    pub last_frame: i32,
}

impl Default for ParticleData {
    fn default() -> Self {
        ParticleData { sim: Default::default(), initial: None, colliders: Vec::new(), events: Vec::new(), scale_ramp: None, scale_max: 1.0, color_ramp: None, opacity_ramp: None, material: None, material_name: String::new(), last_frame: i32::MIN }
    }
}

/// Render Settings (defaultRenderGlobals): the numbers are plugs, the names live here.
#[derive(Clone, Debug)]
pub struct RenderGlobalsData {
    /// File name prefix with tokens <Scene>, <RenderLayer>, <Camera>, <RenderPass>.
    pub prefix: String,
    /// Image format: an extension such as png, jpg, bmp or tga.
    pub ext: String,
    /// Camera to render; empty means the renderable cameras.
    pub camera: String,
}

impl Default for RenderGlobalsData {
    fn default() -> Self {
        RenderGlobalsData { prefix: String::new(), ext: "png".into(), camera: String::new() }
    }
}

/// A render layer: the objects (and lights) it renders, its passes and its Render Settings overrides.
#[derive(Clone, Debug, Default)]
pub struct RenderLayerData {
    pub name: String,
    pub members: Vec<NodeId>,
    pub renderable: bool,
    pub passes: Vec<String>,
    /// Overridden numeric Render Settings, by plug name.
    pub overrides: Vec<(String, f32)>,
    /// Overridden text settings: prefix, ext, camera.
    pub text_overrides: Vec<(String, String)>,
}

/// A Paint Effects brush; its settings are plugs.
#[derive(Clone, Debug, Default)]
pub struct BrushData {
    pub preset: String,
}

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    /// Paint Effects stroke geometry source.
    pub stroke: Option<Box<StrokeData>>,
    pub cache: MeshCache,
    /// Texture zones beyond the base zone and the zone of every face (empty: everything is base zone).
    pub zones: Vec<Zone>,
    pub face_zone: Vec<u16>,
    /// This shape is an instance of another: it follows that shape's geometry, zones and material.
    pub instance_of: Option<NodeId>,
    /// Version of the source's geometry this instance last copied.
    pub instance_ver: u64,
    pub face_sig: Vec<u64>,
    /// Editable UVs (UV Texture Editor). None: the texture is placed by the projection attributes.
    pub uv_set: Option<crate::geo::uv::UvSet>,
    /// NURBS source: this mesh is the tessellation of a NURBS surface.
    pub nurbs: Option<Box<NurbsSurf>>,
    /// Construction history, generator first. Empty means a static mesh ("history deleted").
    pub history: Vec<NodeId>,
    pub mesh: Mesh,
    /// Primitive generator name (polyCube, polySphere, polyCylinder, polyPlane) and its parameters.
    pub prim: String,
    pub params: Vec<(String, f32)>,
    pub material: Option<NodeId>,
}

impl MeshData {
    /// Turn a procedural primitive into an editable welded mesh ("custom"); no-op if already custom.
    pub fn ensure_custom(&mut self) {
        if self.nurbs.is_some() || !self.history.is_empty() {
            return;
        }
        if self.prim != "custom" {
            let poly = crate::geo::edit::Poly::welded(&self.mesh, 1e-4);
            self.mesh = poly.to_mesh();
            self.touch();
            self.prim = "custom".to_string();
            self.params.clear();
        }
    }

    pub fn poly(&self) -> crate::geo::edit::Poly {
        if let Some(n) = &self.nurbs {
            return n.hull_poly();
        }
        crate::geo::edit::Poly::from_mesh(&self.mesh)
    }

    pub fn set_poly(&mut self, p: &crate::geo::edit::Poly) {
        self.mesh = p.to_mesh();
        self.touch();
    }

    /// Keep `face_zone` in step with the mesh after its faces changed. The same number of faces means
    /// the faces were only moved; otherwise faces are matched by their vertex positions, and new faces
    /// (extrusions, fills) take the zone most common among their neighbours.
    /// Keep the UV set matched to the faces after the geometry changed: faces that survive keep their UVs
    /// (found by fingerprint, else by index), new faces get a small flat patch beside the 0..1 square.
    pub fn sync_uvs(&mut self) {
        let Some(set) = self.uv_set.as_mut() else { return };
        if self.mesh.faces.is_empty() {
            // geometry not built yet (a file is loading): keep the set for when it is
            return;
        }
        let sigs: Vec<u64> = self.mesh.faces.iter().map(|f| face_signature(&self.mesh, f)).collect();
        if set.matches(&self.mesh.faces) {
            set.sig = sigs;
            return;
        }
        let old_rows = std::mem::take(&mut set.fuv);
        let old_sig = std::mem::take(&mut set.sig);
        let by_sig: std::collections::HashMap<u64, usize> = if old_sig.len() == old_rows.len() { old_sig.iter().copied().enumerate().map(|(i, s)| (s, i)).collect() } else { Default::default() };
        let now: std::collections::HashSet<u64> = sigs.iter().copied().collect();
        let mut fuv: Vec<Vec<u32>> = Vec::with_capacity(self.mesh.faces.len());
        let mut fresh = 0usize;
        for (i, f) in self.mesh.faces.iter().enumerate() {
            let pick = by_sig.get(&sigs[i]).copied().filter(|&o| old_rows[o].len() == f.len()).or_else(|| {
                if i < old_rows.len() && old_rows[i].len() == f.len() && old_sig.get(i).map(|s| !now.contains(s)).unwrap_or(true) {
                    Some(i)
                } else {
                    None
                }
            });
            match pick {
                Some(o) => fuv.push(old_rows[o].clone()),
                None => {
                    // a flat patch of the face, 0.2 of its size, in a column right of the square
                    let n = crate::geo::edit::face_normal(&self.mesh.positions, f);
                    let helper = if n.y.abs() < 0.9 { crate::math::Vec3::UP } else { crate::math::Vec3::new(1.0, 0.0, 0.0) };
                    let u = helper.cross(n).normalized();
                    let v = n.cross(u);
                    let c = f.iter().map(|&x| self.mesh.positions[x as usize]).fold(crate::math::Vec3::ZERO, |a, b| a + b) * (1.0 / f.len() as f32);
                    let base = crate::math::Vec2::new(1.2 + (fresh % 8) as f32 * 0.25, (fresh / 8) as f32 * 0.25);
                    fresh += 1;
                    let mut row = Vec::new();
                    for &x in f {
                        let d = self.mesh.positions[x as usize] - c;
                        set.uvs.push(crate::math::Vec2::new(base.x + d.dot(u) * 0.2, base.y + d.dot(v) * 0.2));
                        row.push((set.uvs.len() - 1) as u32);
                    }
                    fuv.push(row);
                }
            }
        }
        set.fuv = fuv;
        set.sig = sigs;
        set.compact();
    }

    pub fn sync_zones(&mut self) {
        if self.face_zone.is_empty() || self.mesh.faces.is_empty() {
            self.face_sig.clear();
            return;
        }
        let n = self.mesh.faces.len();
        let sigs: Vec<u64> = self.mesh.faces.iter().map(|f| face_signature(&self.mesh, f)).collect();
        if self.face_zone.len() != n {
            let old: std::collections::HashMap<u64, u16> = self.face_sig.iter().copied().zip(self.face_zone.iter().copied()).collect();
            let mut fz: Vec<Option<u16>> = sigs.iter().map(|s| old.get(s).copied()).collect();
            // an old face whose fingerprint vanished was moved or reshaped in place (extrude, bevel): its
            // index usually survives, so an unmatched new face at that index inherits its zone
            let now: std::collections::HashSet<u64> = sigs.iter().copied().collect();
            for (i, slot) in fz.iter_mut().enumerate() {
                if slot.is_none() && i < self.face_sig.len() && !now.contains(&self.face_sig[i]) {
                    *slot = self.face_zone.get(i).copied();
                }
            }
            // faces without a match: majority of the neighbours (faces sharing a vertex) that have one
            let mut at_vertex: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
            for (fi, f) in self.mesh.faces.iter().enumerate() {
                for &v in f {
                    at_vertex.entry(v).or_default().push(fi);
                }
            }
            for _ in 0..4 {
                let snapshot = fz.clone();
                for fi in 0..n {
                    if fz[fi].is_some() {
                        continue;
                    }
                    let mut votes: std::collections::HashMap<u16, u32> = std::collections::HashMap::new();
                    for &v in &self.mesh.faces[fi] {
                        for &g in at_vertex.get(&v).into_iter().flatten() {
                            if let Some(z) = snapshot[g] {
                                *votes.entry(z).or_default() += 1;
                            }
                        }
                    }
                    if let Some((z, _)) = votes.into_iter().max_by_key(|(z, c)| (*c, std::cmp::Reverse(*z))) {
                        fz[fi] = Some(z);
                    }
                }
            }
            self.face_zone = fz.into_iter().map(|z| z.unwrap_or(0)).collect();
        }
        // zones nobody uses any more fold away
        if self.face_zone.iter().all(|z| *z == 0) {
            self.face_zone.clear();
            self.face_sig.clear();
            self.zones.clear();
        } else {
            self.face_sig = sigs;
        }
    }

    /// Mark the geometry as changed so derived caches are rebuilt.
    pub fn touch(&mut self) {
        self.sync_zones();
        self.sync_uvs();
        self.cache.version = self.cache.version.wrapping_add(1);
        self.cache.edge_count = None;
        self.cache.split = None;
        self.cache.zoned = None;
    }

    /// Unique edge count (cached until the mesh changes).
    pub fn edge_count(&mut self) -> usize {
        if let Some(n) = self.cache.edge_count {
            return n;
        }
        let n = crate::geo::edit::Poly::from_mesh(&self.mesh).edge_list().len();
        self.cache.edge_count = Some(n);
        n
    }
}

#[derive(Clone, Debug)]
pub struct NucleusData {
    pub solver: Solver,
    pub snapshot: Vec<f32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConstraintKind {
    Point,
    Orient,
    Scale,
    Aim,
    Parent,
    /// Orients the object to the normal of a surface.
    Normal,
}

impl ConstraintKind {
    pub fn name(self) -> &'static str {
        match self {
            ConstraintKind::Point => "point",
            ConstraintKind::Orient => "orient",
            ConstraintKind::Scale => "scale",
            ConstraintKind::Aim => "aim",
            ConstraintKind::Parent => "parent",
            ConstraintKind::Normal => "normal",
        }
    }
    pub fn from_name(s: &str) -> Option<ConstraintKind> {
        [ConstraintKind::Point, ConstraintKind::Orient, ConstraintKind::Scale, ConstraintKind::Aim, ConstraintKind::Parent, ConstraintKind::Normal].into_iter().find(|k| k.name() == s)
    }
}

/// A constraint node lives under the transform it drives. Its attributes `w0`, `w1`, ... weight the targets.
#[derive(Clone, Debug)]
pub struct ConstraintData {
    pub kind: ConstraintKind,
    pub targets: Vec<NodeId>,
    /// Target names while a file is loading; resolved to `targets` afterwards.
    pub target_names: Vec<String>,
    /// Parent constraints: the driven object's matrix in each target's space (maintain offset).
    pub offsets: Vec<Mat4>,
}

/// Motion path: drives the transform it lives under along a NURBS curve (Animate > Motion Paths).
#[derive(Clone, Debug, Default)]
pub struct MotionPathData {
    pub curve: Option<NodeId>,
    /// Curve shape name while a file is loading.
    pub curve_name: String,
}

/// An audio file on the time slider (Sound).
#[derive(Clone, Debug, Default)]
pub struct SoundData {
    pub path: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub seconds: f32,
    /// Peak amplitude (0..1) per time slice for the waveform display.
    pub peaks: Vec<f32>,
}

/// Rigid binding of a lattice's points to joints (Skin > Bind Skin on a lattice).
#[derive(Clone, Debug, Default)]
pub struct LatticeSkin {
    pub joints: Vec<NodeId>,
    pub joint_names: Vec<String>,
    pub bind_inv: Vec<Mat4>,
    /// World matrix of the lattice transform at bind time.
    pub bind_world: Mat4,
    pub weights: Vec<Vec<(u16, f32)>>,
}

/// A lattice deformer shape: a grid of control points over the unit cube (scaled by its transform). The point offsets
/// are channels `pt_I_J_K_x/y/z` so they can be keyed, driven and edited as components.
#[derive(Clone, Debug)]
pub struct LatticeData {
    pub divs: [usize; 3],
    pub skin: Option<LatticeSkin>,
}

impl LatticeData {
    pub fn count(&self) -> usize {
        self.divs[0] * self.divs[1] * self.divs[2]
    }
    /// Flat index of point (i, j, k).
    pub fn index(&self, i: usize, j: usize, k: usize) -> usize {
        (i * self.divs[1] + j) * self.divs[2] + k
    }
    pub fn coords(&self, idx: usize) -> (usize, usize, usize) {
        let k = idx % self.divs[2];
        let j = (idx / self.divs[2]) % self.divs[1];
        (idx / (self.divs[1] * self.divs[2]), j, k)
    }
    /// Rest position of a point in the lattice's local cube (-0.5 .. 0.5).
    pub fn rest(&self, idx: usize) -> crate::math::Vec3 {
        let (i, j, k) = self.coords(idx);
        let f = |a: usize, n: usize| if n > 1 { a as f32 / (n - 1) as f32 - 0.5 } else { 0.0 };
        crate::math::Vec3::new(f(i, self.divs[0]), f(j, self.divs[1]), f(k, self.divs[2]))
    }
}

/// A character set: a named group of channels that are keyed together (Animation > Character > Create Character Set).
#[derive(Clone, Debug, Default)]
pub struct CharacterData {
    pub channels: Vec<(NodeId, String)>,
    /// Channel names (node, attribute) while a file is loading.
    pub channel_names: Vec<(String, String)>,
}

/// Distance Tool: the length between two locators, kept up to date as a read only `distance` channel.
#[derive(Clone, Debug, Default)]
pub struct DistanceData {
    pub start: Option<NodeId>,
    pub end: Option<NodeId>,
    pub start_name: String,
    pub end_name: String,
}

/// How a NURBS curve or surface is generated from other nodes ("history"): `kind` names the generator, its
/// numbers are plugs on the shape, `inputs` are the curve shapes it reads.
#[derive(Clone, Debug, Default)]
pub struct NurbsGen {
    pub kind: String,
    pub inputs: Vec<NodeId>,
    /// Input names while a file is loading; resolved to `inputs` afterwards.
    pub input_names: Vec<String>,
}

/// The NURBS source of a mesh shape: the display mesh is its tessellation. Control points of a generated
/// surface are edited as offsets from `base` (the generator output), like tweaks.
#[derive(Clone, Debug)]
pub struct NurbsSurf {
    pub surf: crate::geo::nurbs::Surface,
    pub gen: Option<NurbsGen>,
    pub base: Vec<crate::math::Vec3>,
    pub key: u64,
    /// Isoparm lines for display.
    pub iso: Vec<Vec<crate::math::Vec3>>,
}

impl NurbsSurf {
    /// The control point net as a polygon mesh (so the component tools can select and move CVs).
    pub fn hull_poly(&self) -> crate::geo::edit::Poly {
        let (nu, nv) = (self.surf.u.n, self.surf.v.n);
        let mut faces = Vec::new();
        let (pu, pv) = (self.surf.u.periodic, self.surf.v.periodic);
        for i in 0..if pu { nu } else { nu - 1 } {
            for j in 0..if pv { nv } else { nv - 1 } {
                let (i2, j2) = ((i + 1) % nu, (j + 1) % nv);
                faces.push(vec![(i * nv + j) as u32, (i * nv + j2) as u32, (i2 * nv + j2) as u32, (i2 * nv + j) as u32]);
            }
        }
        crate::geo::edit::Poly { pos: self.surf.cvs.clone(), faces }
    }
}

/// A curve that lies on a surface (Project Curve on Surface): its path in the surface's parameter space.
#[derive(Clone, Debug)]
pub struct OnSurface {
    pub surface: NodeId,
    /// Surface name while a file is loading.
    pub name: String,
    pub uv: Vec<(f32, f32)>,
}

/// A deformer acting on the control points of a curve.
#[derive(Clone, Debug)]
pub enum CurveDriverKind {
    /// A handle moves, rotates and scales the listed control points.
    Cluster { members: Vec<usize>, bind_inv: Mat4 },
    /// A locator moves the curve point at `param` (Point on Curve); `rest` is where the locator was made.
    PointOnCurve { param: f32, rest: crate::math::Vec3 },
}

#[derive(Clone, Debug)]
pub struct CurveDriver {
    pub kind: CurveDriverKind,
    pub handle: Option<NodeId>,
    pub handle_name: String,
}

/// A NURBS curve shape.
#[derive(Clone, Debug)]
pub struct CurveData {
    /// Cluster and point on curve deformers; `driver_base` holds the control points before they act.
    pub drivers: Vec<CurveDriver>,
    pub driver_base: Vec<crate::math::Vec3>,
    pub on_surface: Option<OnSurface>,
    pub curve: crate::geo::nurbs::Curve,
    pub gen: Option<NurbsGen>,
    pub base: Vec<crate::math::Vec3>,
    pub key: u64,
}

/// Generator parameters (plugs on the shape) per generator kind.
pub fn nurbs_gen_plugs(kind: &str) -> &'static [(&'static str, f32)] {
    match kind {
        "sphere" => &[("radius", 1.0), ("sections", 8.0), ("spans", 4.0)],
        "cylinder" | "cone" => &[("radius", 1.0), ("height", 2.0), ("sections", 8.0), ("spans", 1.0)],
        "torus" => &[("radius", 1.0), ("sectionRadius", 0.5), ("sections", 8.0), ("spans", 8.0)],
        "plane" => &[("width", 1.0), ("length", 1.0), ("patchesU", 1.0), ("patchesV", 1.0)],
        "circle" => &[("radius", 1.0), ("sections", 8.0)],
        "square" => &[("sideLength", 1.0)],
        "revolve" => &[("degrees", 360.0), ("sections", 8.0), ("axisX", 0.0), ("axisY", 1.0), ("axisZ", 0.0)],
        _ => &[],
    }
}

/// Names of the procedural texture kinds that are evaluated as shading networks (others are image patterns).
pub const NET_KINDS: [&str; 5] = ["ramp", "noise", "fractal", "cloth", "brownian"];

/// Extra channels of a texture node by kind (on top of placement, color balance and outputs).
pub fn texture_kind_plugs(kind: &str) -> Vec<(String, f32)> {
    let mut v: Vec<(String, f32)> = Vec::new();
    let mut add = |n: &str, x: f32| v.push((n.to_string(), x));
    match kind {
        "ramp" => {
            // 0 V ramp, 1 U ramp, 2 diagonal, 3 radial, 4 circular, 5 box
            add("rampType", 0.0);
            // 0 none, 1 linear, 2 exponential up, 3 exponential down, 4 smooth
            add("interpolation", 1.0);
            add("entryCount", 3.0);
            let pos = [0.0, 0.5, 1.0, 0.75, 1.0];
            let col = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]];
            for i in 0..5 {
                add(&format!("ePos{}", i), pos[i]);
                for (k, c) in ["R", "G", "B"].iter().enumerate() {
                    add(&format!("eCol{}{}", i, c), col[i][k]);
                }
            }
        }
        "noise" | "brownian" => {
            add("amplitude", 1.0);
            add("ratio", 0.707);
            add("frequencyRatio", 2.0);
            add("octaves", 4.0);
            add("threshold", 0.0);
            add("frequency", 8.0);
        }
        "fractal" => {
            add("amplitude", 1.0);
            add("ratio", 0.707);
            add("frequencyRatio", 2.0);
            add("octaves", 5.0);
            add("threshold", 0.0);
            add("frequency", 4.0);
            add("bias", 0.0);
        }
        "cloth" => {
            for (n, c) in [("uColor", [0.2, 0.2, 0.2]), ("vColor", [0.8, 0.8, 0.8])] {
                for (k, a) in ["R", "G", "B"].iter().enumerate() {
                    add(&format!("{}{}", n, a), c[k]);
                }
            }
            add("uWidth", 0.1);
            add("vWidth", 0.1);
            add("uWave", 0.0);
            add("vWave", 0.0);
            add("brightSpread", 0.5);
            add("randomness", 0.0);
        }
        _ => {}
    }
    v
}

/// A texture node: an image file or a built-in pattern. Its `outColor*` plugs feed material colors
/// (connect `tex.outColor` to `mat.color`), its placement plugs repeat, offset and rotate the image.
#[derive(Clone, Debug, Default)]
pub struct TextureData {
    /// "file" or a pattern name (checker, grid, bricks, stripes, dots, noise, gradient).
    pub kind: String,
    /// Image path for "file" textures (relative to the project when inside it).
    pub path: String,
}

/// Shading models of a material node (`shaderModel` attribute).
pub const SHADER_MODELS: [&str; 4] = ["Lambert", "Blinn", "Phong", "Surface Shader"];

/// A selection set (Maya: objectSet / Quick Select Set): a named list of nodes.
#[derive(Clone, Debug, Default)]
pub struct SetData {
    pub members: Vec<NodeId>,
    /// Member names while a file is loading; resolved to `members` afterwards.
    pub member_names: Vec<String>,
}

/// Maximum number of targets of one constraint.
pub const MAX_CONSTRAINT_TARGETS: usize = 8;

#[derive(Clone, Debug)]
pub enum NodeData {
    Transform,
    Mesh(MeshData),
    Camera,
    StereoCamera,
    Material,
    Light,
    /// Particle emitter shape: its transform sets where and which way particles are emitted.
    Emitter,
    /// Locator shape (a marker that can be moved, parented to and constrained to).
    Locator,
    Constraint(Box<ConstraintData>),
    /// Selection set: a named group of objects (Create > Sets > Quick Select Set).
    ObjectSet(Box<SetData>),
    /// File or procedural texture feeding material attributes.
    Texture(Box<TextureData>),
    /// NURBS curve shape (surfaces are mesh shapes with a NURBS source).
    NurbsCurve(Box<CurveData>),
    /// Lattice deformer shape.
    Lattice(Box<LatticeData>),
    /// Character set.
    Character(Box<CharacterData>),
    /// Distance Tool measurement between two locators.
    Distance(Box<DistanceData>),
    MotionPath(Box<MotionPathData>),
    Brush(Box<BrushData>),
    RenderGlobals(Box<RenderGlobalsData>),
    RigidBody(Box<RigidData>),
    DynSolver(Box<DynSolverData>),
    Field(Box<FieldData>),
    RigidJoint(Box<RigidJointData>),
    PEmitter(Box<PEmitterData>),
    Particle(Box<ParticleData>),
    Sound(Box<SoundData>),
    /// Set Driven Key curve: `output` is the curve evaluated at `input`.
    DrivenCurve(Box<crate::anim::keyframes::Curve>),
    Nucleus(Box<NucleusData>),
    PolyOp(Box<PolyOpData>),
    Joint,
    IkHandle(Box<IkData>),
}

impl NodeData {
    /// Maya style node type name.
    pub fn type_name(&self) -> &'static str {
        match self {
            NodeData::Transform => "transform",
            NodeData::Mesh(_) => "mesh",
            NodeData::Camera => "camera",
            NodeData::StereoCamera => "stereoRigCamera",
            NodeData::Material => "lambert",
            NodeData::Light => "light",
            NodeData::Emitter => "emitter",
            NodeData::Locator => "locator",
            NodeData::Constraint(_) => "constraint",
            NodeData::ObjectSet(_) => "objectSet",
            NodeData::Texture(_) => "texture",
            NodeData::NurbsCurve(_) => "nurbsCurve",
            NodeData::Lattice(_) => "lattice",
            NodeData::Character(_) => "character",
            NodeData::Distance(_) => "distanceDimension",
            NodeData::MotionPath(_) => "motionPath",
            NodeData::Brush(_) => "brush",
            NodeData::RenderGlobals(_) => "renderGlobals",
            NodeData::RigidBody(_) => "rigidBody",
            NodeData::DynSolver(_) => "rigidSolver",
            NodeData::Field(_) => "field",
            NodeData::RigidJoint(_) => "rigidConstraint",
            NodeData::PEmitter(_) => "particleEmitter",
            NodeData::Particle(_) => "particle",
            NodeData::Sound(_) => "sound",
            NodeData::DrivenCurve(_) => "animCurveUU",
            NodeData::Nucleus(_) => "nucleus",
            NodeData::PolyOp(_) => "polyOp",
            NodeData::Joint => "joint",
            NodeData::IkHandle(_) => "ikHandle",
        }
    }

    pub fn from_type_name(s: &str) -> Option<NodeData> {
        match s {
            "transform" => Some(NodeData::Transform),
            "camera" => Some(NodeData::Camera),
            "stereoRigCamera" => Some(NodeData::StereoCamera),
            "lambert" | "material" => Some(NodeData::Material),
            "light" => Some(NodeData::Light),
            "emitter" => Some(NodeData::Emitter),
            "locator" => Some(NodeData::Locator),
            "objectSet" => Some(NodeData::ObjectSet(Box::default())),
            "nurbsCurve" => Some(NodeData::NurbsCurve(Box::new(CurveData { curve: crate::geo::nurbs::Curve::open(vec![crate::math::Vec3::ZERO, crate::math::Vec3::new(1.0, 0.0, 0.0)], 1), gen: None, base: Vec::new(), key: 0, on_surface: None, drivers: Vec::new(), driver_base: Vec::new() }))),
            "character" => Some(NodeData::Character(Box::default())),
            "distanceDimension" => Some(NodeData::Distance(Box::default())),
            "lattice" => Some(NodeData::Lattice(Box::new(LatticeData { divs: [2, 2, 2], skin: None }))),
            "motionPath" => Some(NodeData::MotionPath(Box::default())),
            "brush" => Some(NodeData::Brush(Box::default())),
            "renderGlobals" => Some(NodeData::RenderGlobals(Box::default())),
            "rigidBody" => Some(NodeData::RigidBody(Box::default())),
            "rigidSolver" => Some(NodeData::DynSolver(Box::default())),
            "field" => Some(NodeData::Field(Box::default())),
            "rigidConstraint" => Some(NodeData::RigidJoint(Box::default())),
            "particleEmitter" => Some(NodeData::PEmitter(Box::default())),
            "particle" => Some(NodeData::Particle(Box::default())),
            "sound" => Some(NodeData::Sound(Box::default())),
            "texture" => Some(NodeData::Texture(Box::new(TextureData { kind: "file".to_string(), path: String::new() }))),
            "constraint" => Some(NodeData::Constraint(Box::new(ConstraintData { kind: ConstraintKind::Point, targets: Vec::new(), target_names: Vec::new(), offsets: Vec::new() }))),
            "animCurveUU" => Some(NodeData::DrivenCurve(Box::default())),
            "joint" => Some(NodeData::Joint),
            "ikHandle" => Some(NodeData::IkHandle(Box::new(IkData::default()))),
            "polyOp" => Some(NodeData::PolyOp(Box::new(PolyOpData::new(PolyKind::Source)))),
            "nucleus" => Some(NodeData::Nucleus(Box::new(NucleusData {
                solver: Solver::new(),
                snapshot: Vec::new(),
            }))),
            _ => None,
        }
    }

    pub fn is_transform(&self) -> bool {
        matches!(self, NodeData::Transform | NodeData::Joint | NodeData::IkHandle(_))
    }
}

/// Order matters: indices 0..=27 are mirrored into the Solver, 28 is a read only output.
pub const SOLVER_ATTRS: [(&str, f32); 29] = [
    ("gravityX", 0.0),
    ("gravityY", -9.8),
    ("gravityZ", 0.0),
    ("windX", 0.0),
    ("windY", 0.0),
    ("windZ", 0.0),
    ("windDrag", 0.05),
    ("substeps", 3.0),
    ("groundY", 0.0),
    ("bounce", 0.6),
    ("friction", 0.1),
    ("collide", 1.0),
    ("particleRadius", 0.15),
    ("particleMass", 1.0),
    ("emitRate", 40.0),
    ("emitX", 0.0),
    ("emitY", 5.0),
    ("emitZ", 0.0),
    ("emitDirX", 0.0),
    ("emitDirY", 1.0),
    ("emitDirZ", 0.0),
    ("emitSpeed", 4.0),
    ("spread", 0.35),
    ("lifespan", 0.0),
    ("maxParticles", 600.0),
    ("startFrame", 1.0),
    ("seed", 1.0),
    ("enabled", 1.0),
    ("particleCount", 0.0),
];

/// Copy plug values (in SOLVER_ATTRS order) into the solver. Returns true if the sim must restart.
pub fn apply_solver_values(nd: &mut NucleusData, v: &[f32]) -> bool {
    let changed = nd.snapshot.as_slice() != v;
    if !changed {
        return false;
    }
    let s = &mut nd.solver;
    s.gravity = crate::math::Vec3::new(v[0], v[1], v[2]);
    s.wind = crate::math::Vec3::new(v[3], v[4], v[5]);
    s.wind_drag = v[6];
    s.substeps = v[7].max(1.0) as u32;
    s.plane_y = v[8];
    s.bounce = v[9];
    s.friction = v[10];
    s.particle_collide = v[11] > 0.5;
    s.particle_radius = v[12].max(0.001);
    s.particle_mass = v[13].max(0.0001);
    s.emit_rate = v[14].max(0.0);
    s.emit_pos = crate::math::Vec3::new(v[15], v[16], v[17]);
    s.emit_dir = crate::math::Vec3::new(v[18], v[19], v[20]);
    s.emit_speed = v[21];
    s.spread = v[22];
    s.lifespan = v[23];
    s.max_particles = v[24].max(0.0) as usize;
    s.start_frame = v[25];
    s.seed = v[26].max(0.0) as u64;
    s.enabled = v[27] > 0.5;
    nd.snapshot = v.to_vec();
    s.reset();
    true
}

/// Default plugs for each node type.
pub fn default_plugs(data: &NodeData) -> Vec<Plug> {
    let mut p = Vec::new();
    let mut add = |name: &str, v: f32| p.push(Plug::new(name, v));
    match data {
        NodeData::Joint | NodeData::IkHandle(_) => {
            for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ"] {
                add(a, 0.0);
            }
            if matches!(data, NodeData::Joint) {
                for a in ["rotatePivotX", "rotatePivotY", "rotatePivotZ", "scalePivotX", "scalePivotY", "scalePivotZ"] {
                    add(a, 0.0);
                }
                for a in ["rotatePivotTranslateX", "rotatePivotTranslateY", "rotatePivotTranslateZ"] {
                add(a, 0.0);
            }

            }
            for a in ["scaleX", "scaleY", "scaleZ"] {
                add(a, 1.0);
            }
            add("visibility", 1.0);
            if matches!(data, NodeData::Joint) {
                // the orientation of the joint's axes relative to its parent; rotate stays zero in the bind pose
                for a in ["jointOrientX", "jointOrientY", "jointOrientZ"] {
                    add(a, 0.0);
                }
                // the bend the IK solvers prefer (Skeleton > Set Preferred Angle)
                for a in ["preferredAngleX", "preferredAngleY", "preferredAngleZ"] {
                    add(a, 0.0);
                }
                add("inheritsTransform", 1.0);
                add("radius", 0.5);
            } else {
                add("enable", 1.0);
                // rotate plane: spin of the bend about the root-to-handle axis (degrees)
                add("twist", 0.0);
                // 1: the solver decides, 0: the joint rotations stay as animated
                add("ikBlend", 1.0);
                // spline IK: shifts the chain along the curve (parameter units) and rolls it about the curve
                add("offset", 0.0);
                add("roll", 0.0);
            }
        }
        NodeData::Transform => {
            for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ"] {
                add(a, 0.0);
            }
            // pivots in the object's own space: rotation and scale happen around these points
            for a in ["rotatePivotX", "rotatePivotY", "rotatePivotZ", "scalePivotX", "scalePivotY", "scalePivotZ"] {
                add(a, 0.0);
            }
            // moves the object by a fixed amount when a pivot is placed away from it (Maya: rotatePivotTranslate)
            for a in ["rotatePivotTranslateX", "rotatePivotTranslateY", "rotatePivotTranslateZ"] {
                add(a, 0.0);
            }
            for a in ["scaleX", "scaleY", "scaleZ"] {
                add(a, 1.0);
            }
            add("visibility", 1.0);
            // 0: the object ignores the transformation of its parents
            add("inheritsTransform", 1.0);
        }
        NodeData::Mesh(_) => {
            add("uvMode", 0.0);
            add("uvTilingU", 1.0);
            add("uvTilingV", 1.0);
            add("uvOffsetU", 0.0);
            add("uvOffsetV", 0.0);
            add("uvRotate", 0.0);
            // edges sharper than this many degrees render hard (0 = all hard, 180 = all soft)
            add("smoothAngle", 40.0);
            // Smooth Mesh Preview: 1 cage only, 2 cage and smoothed mesh, 3 smoothed mesh only
            add("displaySmoothness", 1.0);
            add("smoothLevel", 2.0);
            // Render Stats
            add("castsShadows", 1.0);
            add("receiveShadows", 1.0);
            add("motionBlur", 1.0);
            add("primaryVisibility", 1.0);
            add("visibleInReflections", 1.0);
        }
        NodeData::PolyOp(op) => match op.kind {
            PolyKind::Cube => {
                add("width", 1.0);
                add("height", 1.0);
                add("depth", 1.0);
                add("subdivisions", 1.0);
            }
            PolyKind::Sphere => {
                add("radius", 1.0);
                add("subdivisions", 20.0);
            }
            PolyKind::Cylinder => {
                add("radius", 1.0);
                add("height", 2.0);
                add("subdivisions", 20.0);
            }
            PolyKind::Plane => {
                add("width", 1.0);
                add("height", 1.0);
                add("subdivisions", 1.0);
            }
            PolyKind::Cone => {
                add("radius", 1.0);
                add("height", 2.0);
                add("subdivisions", 20.0);
            }
            PolyKind::Torus => {
                add("radius", 1.0);
                add("sectionRadius", 0.5);
                add("subdivisions", 24.0);
                add("sectionSubdivisions", 12.0);
            }
            PolyKind::Bend | PolyKind::Twist | PolyKind::Flare | PolyKind::Squash | PolyKind::Sine => {
                add("axis", 1.0);
                add("lowBound", -1.0);
                add("highBound", 1.0);
                match op.kind {
                    PolyKind::Bend => add("curvature", 0.0),
                    PolyKind::Twist => {
                        add("startAngle", 0.0);
                        add("endAngle", 0.0);
                    }
                    PolyKind::Flare => {
                        add("startFlareX", 1.0);
                        add("startFlareZ", 1.0);
                        add("endFlareX", 1.0);
                        add("endFlareZ", 1.0);
                        add("curve", 0.0);
                    }
                    PolyKind::Squash => {
                        add("factor", 0.0);
                        add("expand", 1.0);
                    }
                    _ => {
                        add("amplitude", 0.0);
                        add("wavelength", 2.0);
                        add("offset", 0.0);
                        add("dropoff", 0.0);
                    }
                }
                add("envelope", 1.0);
            }
            PolyKind::Wave => {
                add("axis", 1.0);
                add("amplitude", 0.3);
                add("wavelength", 1.0);
                add("offset", 0.0);
                add("minRadius", 0.0);
                add("maxRadius", 5.0);
                add("dropoff", 0.0);
                add("envelope", 1.0);
            }
            PolyKind::Wire => {
                add("dropoffDistance", 1.0);
                add("scale", 1.0);
                add("rotation", 1.0);
                add("envelope", 1.0);
            }
            PolyKind::Cluster => add("envelope", 1.0),
            PolyKind::SoftMod => {
                add("falloffRadius", 1.0);
                // 0 smooth, 1 linear, 2 ease out
                add("falloffType", 0.0);
                add("envelope", 1.0);
            }
            PolyKind::Jiggle => {
                add("stiffness", 0.1);
                add("damping", 0.2);
                add("jiggleWeight", 1.0);
                add("motionMultiplier", 1.0);
                add("ignoreTransform", 0.0);
                add("envelope", 1.0);
            }
            PolyKind::Average => {
                add("iterations", 1.0);
                add("strength", 1.0);
            }
            PolyKind::ChipOff => add("duplicate", 0.0),
            PolyKind::Blend => add("weight", 0.0),
            PolyKind::Transform => {
                for i in 0..16 {
                    add(&format!("m{}", i), if i % 5 == 0 { 1.0 } else { 0.0 });
                }
            }
            PolyKind::Extrude | PolyKind::ExtrudeEdge => {
                add("localTranslateZ", 0.0);
                // Edit Mesh > Keep Faces Together: 0 extrudes every face on its own
                add("keepFacesTogether", 1.0);
            }
            PolyKind::OffsetLoop => add("offset", 0.25),
            PolyKind::SlideEdge => add("amount", 0.0),
            PolyKind::MirrorCut => {
                add("axis", 0.0);
                add("planeOffset", 0.0);
                add("keepNegative", 0.0);
            }
            PolyKind::Bevel => add("offset", 0.2),
            PolyKind::Merge => add("distance", 0.001),
            PolyKind::Smooth => add("divisions", 1.0),
            PolyKind::Mirror => add("axis", 0.0),
            PolyKind::InsertLoop => add("weight", 0.5),
            _ => {}
        },
        NodeData::Camera => {
            add("fieldOfView", 45.0);
            add("nearClip", 0.1);
            add("farClip", 1000.0);
            add("orthographic", 0.0);
            add("renderable", 0.0);
            // Fit Resolution Gate: 0 fill, 1 horizontal, 2 vertical (the field of view is vertical), 3 overscan
            add("filmFit", 2.0);
        }
        NodeData::StereoCamera => {
            add("fieldOfView", 45.0);
            add("nearClip", 0.1);
            add("farClip", 1000.0);
            add("interaxialSeparation", 0.4);
            add("zeroParallax", 10.0);
        }
        NodeData::Material => {
            add("colorR", 0.65);
            add("colorG", 0.68);
            add("colorB", 0.75);
            add("ambient", 0.12);
            add("diffuse", 0.85);
            add("specular", 0.3);
            add("shininess", 30.0);
            add("reflectivity", 0.0);
            // 0 Lambert, 1 Blinn, 2 Phong, 3 Surface Shader (older scenes keep the highlight they had)
            add("shaderModel", 1.0);
            add("transparency", 0.0);
            add("incandescenceR", 0.0);
            add("incandescenceG", 0.0);
            add("incandescenceB", 0.0);
            add("specularColorR", 1.0);
            add("specularColorG", 1.0);
            add("specularColorB", 1.0);
            // bump map: connect a texture to bump (its brightness is the height); bumpDepth scales it (negative dents)
            add("bumpR", 0.0);
            add("bumpG", 0.0);
            add("bumpB", 0.0);
            add("bumpDepth", 1.0);
        }
        NodeData::Character(_) => {}
        NodeData::Distance(_) => {
            let mut pl = Plug::new("distance", 0.0);
            pl.readonly = true;
            pl.keyable = false;
            p.push(pl);
        }
        NodeData::Lattice(ld) => {
            for idx in 0..ld.count() {
                let (i, j, k) = ld.coords(idx);
                for a in ["x", "y", "z"] {
                    add(&format!("pt_{}_{}_{}_{}", i, j, k, a), 0.0);
                }
            }
        }
        NodeData::RigidBody(_) => {
            // 1 active (reacts to dynamics), 0 passive (follows its keys)
            add("active", 1.0);
            add("mass", 1.0);
            add("bounciness", 0.6);
            add("damping", 0.0);
            add("staticFriction", 0.2);
            add("dynamicFriction", 0.2);
            add("centerOfMassX", 0.0);
            add("centerOfMassY", 0.0);
            add("centerOfMassZ", 0.0);
            add("initialVelocityX", 0.0);
            add("initialVelocityY", 0.0);
            add("initialVelocityZ", 0.0);
            add("initialSpinX", 0.0);
            add("initialSpinY", 0.0);
            add("initialSpinZ", 0.0);
            add("collisions", 1.0);
            // stand-in shape: 0 box, 1 sphere, 2 mesh (passive bodies only)
            add("shape", 0.0);
        }
        NodeData::DynSolver(_) => {
            add("state", 1.0);
            add("startFrame", 1.0);
            add("stepSize", 0.02);
            add("collisionTolerance", 0.02);
            add("contactIterations", 10.0);
            add("cacheData", 0.0);
            add("cachedFrames", 0.0);
        }
        NodeData::Field(_) => {
            add("magnitude", 10.0);
            add("attenuation", 0.0);
            add("directionX", 0.0);
            add("directionY", -1.0);
            add("directionZ", 0.0);
            add("maxDistance", 5.0);
            add("useMaxDistance", 0.0);
            // 0 none, 1 cube, 2 sphere, 3 cylinder, 4 cone: sized by the field's scale
            add("volumeShape", 0.0);
            add("speed", 0.0);
            add("frequency", 1.0);
            add("phase", 0.0);
        }
        NodeData::RigidJoint(_) => {
            add("stiffness", 10.0);
            add("damping", 0.5);
            add("restLength", 0.0);
        }
        NodeData::PEmitter(_) => {
            add("rate", 100.0);
            // 0 omni, 1 directional, 2 volume, 3 surface, 4 curve
            add("emitterType", 0.0);
            add("speed", 1.0);
            add("speedRandom", 0.0);
            add("spread", 0.0);
            add("directionX", 0.0);
            add("directionY", 1.0);
            add("directionZ", 0.0);
            add("inheritFactor", 0.0);
            add("enabled", 1.0);
        }
        NodeData::Particle(_) => {
            add("startFrame", 1.0);
            // 0 live forever, 1 constant, 2 random range
            add("lifespanMode", 0.0);
            add("lifespan", 1.0);
            add("lifespanRandom", 0.0);
            add("conserve", 1.0);
            add("damping", 0.0);
            add("mass", 1.0);
            add("radius", 0.5);
            // 0 points, 1 spheres, 2 blobby surface, 3 sprites, 4 streaks
            add("renderType", 0.0);
            add("threshold", 1.0);
            add("maxCount", 5000.0);
            add("seed", 1.0);
            add("particleCount", 0.0);
            add("opacity", 1.0);
            add("colorR", 0.8);
            add("colorG", 0.8);
            add("colorB", 0.8);
        }
        NodeData::RenderGlobals(_) => {
            add("width", 640.0);
            add("height", 480.0);
            // 0 software (ray traced), 1 hardware (no ray tracing), 2 vector (flat fills and outlines)
            add("renderer", 0.0);
            // anti-aliasing: 0 preview, 1 intermediate, 2 production, 3 high
            add("quality", 1.0);
            add("startFrame", 1.0);
            add("endFrame", 10.0);
            add("byFrame", 1.0);
            add("framePadding", 4.0);
            // 0 name.ext (single image), 1 name.#.ext, 2 name.ext.#
            add("animation", 1.0);
            add("raytracing", 1.0);
            add("motionBlur", 0.0);
            // 0: 2D (fast, smears the finished image), 1: 3D (renders several moments of the shutter)
            add("motionBlurType", 0.0);
            add("blurLength", 1.0);
            add("shutterAngle", 144.0);
            add("blurSamples", 5.0);
            add("alphaChannel", 1.0);
            add("depthChannel", 0.0);
            // Maya Hardware: geometry hides particles behind it and renders as an empty mask; blur exposures
            add("geometryMask", 0.0);
            add("hardwareExposures", 7.0);
            add("fullFrame", 1.0);
            // Maya Vector options
            // 0 single color, 1 full color, 2 two color
            add("vectorFill", 1.0);
            add("vectorEdges", 1.0);
            add("vectorEdgeWeight", 1.5);
            add("vectorShadows", 1.0);
            add("vectorHighlights", 1.0);
            add("vectorReflections", 1.0);
            add("bgTopR", 0.36);
            add("bgTopG", 0.40);
            add("bgTopB", 0.46);
            add("bgBottomR", 0.14);
            add("bgBottomG", 0.15);
            add("bgBottomB", 0.17);
        }
        NodeData::Brush(_) => {
            for (k, v) in crate::geo::paintfx::BRUSH_DEFAULTS {
                add(k, *v);
            }
        }
        NodeData::MotionPath(_) => {
            add("uValue", 0.0);
            // 0: uValue is a curve parameter; 1: a fraction 0..1 of the curve
            add("fractionMode", 0.0);
            add("follow", 0.0);
            // 0 X, 1 Y, 2 Z
            add("frontAxis", 0.0);
            add("upAxis", 1.0);
            add("inverseFront", 0.0);
            add("inverseUp", 0.0);
            add("bank", 0.0);
            add("bankScale", 1.0);
            add("bankLimit", 90.0);
            add("frontTwist", 0.0);
            add("upTwist", 0.0);
            add("sideTwist", 0.0);
        }
        NodeData::Sound(_) => {
            // frames the sound starts at
            add("offset", 1.0);
        }
        NodeData::NurbsCurve(cd) => {
            if let Some(g) = &cd.gen {
                for (k, v) in nurbs_gen_plugs(&g.kind) {
                    add(k, *v);
                }
            }
        }
        NodeData::Texture(td) => {
            add("repeatU", 1.0);
            add("repeatV", 1.0);
            add("offsetU", 0.0);
            add("offsetV", 0.0);
            add("rotateUV", 0.0);
            // 1: the texture repeats outside 0..1; 0: the default color shows there
            add("wrapU", 1.0);
            add("wrapV", 1.0);
            add("defaultColorR", 0.5);
            add("defaultColorG", 0.5);
            add("defaultColorB", 0.5);
            add("colorGainR", 1.0);
            add("colorGainG", 1.0);
            add("colorGainB", 1.0);
            add("colorOffsetR", 0.0);
            add("colorOffsetG", 0.0);
            add("colorOffsetB", 0.0);
            for (k, v) in texture_kind_plugs(&td.kind) {
                add(&k, v);
            }
            for a in ["outColorR", "outColorG", "outColorB", "outAlpha", "outTransparency"] {
                add(a, 0.5);
            }
        }
        NodeData::Light => {
            add("intensity", 1.0);
            add("colorR", 1.0);
            add("colorG", 1.0);
            add("colorB", 1.0);
            // 0 point, 1 directional, 2 spot, 3 ambient, 4 area
            add("lightType", 0.0);
            add("coneAngle", 40.0);
            add("penumbraAngle", 0.0);
            add("dropoff", 0.0);
            // 0 none, 1 linear, 2 quadratic, 3 cubic, 4 soft (the original gentle falloff)
            add("decayRate", 4.0);
            add("emitDiffuse", 1.0);
            add("emitSpecular", 1.0);
            add("areaWidth", 2.0);
            add("areaHeight", 2.0);
            add("shadows", 1.0);
            // radius of the light in world units (degrees for a directional light): 0 gives hard shadows
            add("shadowSoftness", 0.0);
            add("shadowSamples", 16.0);
            add("shadowColorR", 0.0);
            add("shadowColorG", 0.0);
            add("shadowColorB", 0.0);
            // depth map shadows: map resolution and filter size set the blur of the shadow edge
            add("useDepthMapShadows", 0.0);
            add("dmapResolution", 512.0);
            add("dmapFilterSize", 1.0);
            // Light Effects: glow and halo (opticalFX) and light fog
            add("lightGlow", 0.0);
            // 0 none, 1 linear, 2 exponential
            add("glowType", 1.0);
            add("glowIntensity", 1.0);
            add("glowSpread", 1.0);
            // 0 none, 1 linear, 2 exponential, 3 lens flare
            add("haloType", 0.0);
            add("haloIntensity", 1.0);
            add("lensFlare", 0.0);
            add("flareIntensity", 1.0);
            // 0 none, 1 normal, 2 linear
            add("fogType", 0.0);
            add("fogRadius", 1.0);
            add("fogIntensity", 1.0);
        }
        NodeData::Locator => add("size", 1.0),
        NodeData::ObjectSet(_) => {}
        NodeData::DrivenCurve(_) => {
            add("input", 0.0);
            add("output", 0.0);
        }
        NodeData::Constraint(_) => {
            add("enable", 1.0);
            for i in 0..MAX_CONSTRAINT_TARGETS {
                add(&format!("w{}", i), 1.0);
            }
            // point: world offset; orient: rotation offset (degrees); scale: scale factor
            add("offsetX", 0.0);
            add("offsetY", 0.0);
            add("offsetZ", 0.0);
            add("aimX", 1.0);
            add("aimY", 0.0);
            add("aimZ", 0.0);
            add("upX", 0.0);
            add("upY", 1.0);
            add("upZ", 0.0);
            // world up for aim and normal constraints: 0 scene up, 1 object up (the second target), 2 vector, 3 none
            add("worldUpType", 2.0);
            add("worldUpX", 0.0);
            add("worldUpY", 1.0);
            add("worldUpZ", 0.0);
        }
        NodeData::Emitter => {
            add("rate", 40.0);
            add("speed", 4.0);
            add("spread", 0.35);
            add("lifespan", 0.0);
            add("maxParticles", 600.0);
            add("startFrame", 1.0);
            add("seed", 1.0);
            add("enabled", 1.0);
        }
        NodeData::Nucleus(_) => {
            for (n, v) in SOLVER_ATTRS.iter() {
                add(n, *v);
            }
        }
    }
    if matches!(data, NodeData::Nucleus(_)) {
        if let Some(pl) = p.last_mut() {
            pl.readonly = true;
        }
    }
    if matches!(data, NodeData::Texture(_)) {
        for pl in p.iter_mut().filter(|pl| pl.name.starts_with("out")) {
            pl.readonly = true;
            pl.keyable = false;
        }
    }
    p
}

pub struct Node {
    pub name: String,
    pub data: NodeData,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub plugs: Vec<Plug>,
    pub world_cache: Option<Mat4>,
}

impl Node {
    pub fn plug(&self, name: &str) -> Option<&Plug> {
        self.plugs.iter().find(|p| p.name == name)
    }
    pub fn plug_mut(&mut self, name: &str) -> Option<&mut Plug> {
        self.plugs.iter_mut().find(|p| p.name == name)
    }
}

/// Build a primitive mesh from its generator name and parameter list.
pub fn build_mesh(prim: &str, params: &[(String, f32)]) -> Option<Mesh> {
    let get = |k: &str, d: f32| params.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(d);
    match prim {
        "polyCube" => Some(primitives::poly_cube(
            get("width", 1.0),
            get("height", 1.0),
            get("depth", 1.0),
            get("subdivisions", 1.0) as usize,
        )),
        "polySphere" => Some(primitives::poly_sphere(get("radius", 1.0), get("subdivisions", 20.0) as usize)),
        "polyCylinder" => Some(primitives::poly_cylinder(
            get("radius", 1.0),
            get("height", 2.0),
            get("subdivisions", 20.0) as usize,
        )),
        "polyPlane" => Some(primitives::poly_plane(
            get("width", 1.0),
            get("height", 1.0),
            get("subdivisions", 1.0) as usize,
        )),
        "polyCone" => Some(primitives::poly_cone(get("radius", 1.0), get("height", 2.0), get("subdivisions", 20.0) as usize)),
        "polyTorus" => Some(primitives::poly_torus(
            get("radius", 1.0),
            get("sectionRadius", 0.5),
            get("subdivisions", 24.0) as usize,
            get("sectionSubdivisions", 12.0) as usize,
        )),
        _ => None,
    }
}
