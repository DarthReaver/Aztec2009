use crate::dag::Scene;
use crate::math::{Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DisplayMode {
    Shaded,
    /// Shaded with scene lights and color maps (not render quality).
    Textured,
    Wireframe,
    Anaglyph,
}

impl DisplayMode {
    pub fn name(self) -> &'static str {
        match self {
            DisplayMode::Shaded => "shaded",
            DisplayMode::Textured => "textured",
            DisplayMode::Wireframe => "wireframe",
            DisplayMode::Anaglyph => "anaglyph",
        }
    }
    pub fn parse(s: &str) -> Option<DisplayMode> {
        match s.to_ascii_lowercase().as_str() {
            "shaded" | "smooth" | "beauty" => Some(DisplayMode::Shaded),
            "textured" | "texture" | "textures" => Some(DisplayMode::Textured),
            "wire" | "wireframe" => Some(DisplayMode::Wireframe),
            "anaglyph" | "stereo" => Some(DisplayMode::Anaglyph),
            _ => None,
        }
    }
    pub fn next(self) -> DisplayMode {
        match self {
            DisplayMode::Wireframe => DisplayMode::Shaded,
            DisplayMode::Shaded => DisplayMode::Textured,
            DisplayMode::Textured => DisplayMode::Anaglyph,
            DisplayMode::Anaglyph => DisplayMode::Wireframe,
        }
    }
}

/// Orbit camera state that drives the scene's `persp` camera.
#[derive(Clone, Debug)]
pub struct ViewState {
    pub mode: DisplayMode,
    /// Look through this scene camera instead of the orbit camera (None means `persp`).
    pub camera: Option<String>,
    pub target: Vec3,
    pub dist: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub grid: bool,
    /// Draw every object's edges over the shaded view (Maya: Shading > Wireframe on Shaded).
    pub wire_on_shaded: bool,
    /// Multiplier on the automatic interaxial distance when anaglyph viewing a non stereo camera.
    pub stereo_scale: f32,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState::new()
    }
}

impl ViewState {
    pub fn new() -> ViewState {
        ViewState {
            mode: DisplayMode::Shaded,
            camera: None,
            target: Vec3::ZERO,
            dist: 22.0,
            yaw: 45.0,
            pitch: 26.8,
            grid: true,
            wire_on_shaded: false,
            stereo_scale: 1.0,
        }
    }

    /// Color maps are drawn in Textured mode only.
    pub fn textured(&self) -> bool {
        self.mode == DisplayMode::Textured
    }

    /// Switch to Textured display when a texture was just assigned (Shaded would hide it).
    pub fn show_textures(&mut self) {
        if self.mode == DisplayMode::Shaded {
            self.mode = DisplayMode::Textured;
        }
    }

    fn rotation(&self) -> Quat {
        Quat::from_euler_deg(Vec3::new(-self.pitch, self.yaw, 0.0))
    }

    pub fn eye(&self) -> Vec3 {
        let fwd = self.rotation().rotate(Vec3::new(0.0, 0.0, -1.0));
        self.target - fwd * self.dist
    }

    pub fn orbit(&mut self, dyaw: f32, dpitch: f32) {
        self.camera = None;
        self.yaw = (self.yaw + dyaw).rem_euclid(360.0);
        self.pitch = (self.pitch + dpitch).clamp(-89.0, 89.0);
    }

    /// Pan in screen units (fractions of the view distance).
    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.camera = None;
        let q = self.rotation();
        let right = q.rotate(Vec3::new(1.0, 0.0, 0.0));
        let up = q.rotate(Vec3::new(0.0, 1.0, 0.0));
        self.target += right * (dx * self.dist) + up * (dy * self.dist);
    }

    pub fn zoom(&mut self, factor: f32) {
        self.camera = None;
        self.dist = (self.dist * factor).clamp(0.5, 500.0);
    }

    /// Write the orbit state into the `persp` transform attributes.
    pub fn apply(&self, scene: &mut Scene) {
        if let Some(t) = scene.find("persp") {
            let e = self.eye();
            let _ = scene.set_attr(t, "translateX", e.x);
            let _ = scene.set_attr(t, "translateY", e.y);
            let _ = scene.set_attr(t, "translateZ", e.z);
            let _ = scene.set_attr(t, "rotateX", -self.pitch);
            let _ = scene.set_attr(t, "rotateY", self.yaw);
            let _ = scene.set_attr(t, "rotateZ", 0.0);
        }
    }

    /// Recover orbit state from the `persp` transform.
    pub fn sync_from_scene(&mut self, scene: &mut Scene) {
        if let Some(t) = scene.find("persp") {
            let g = |s: &mut Scene, a: &str| s.get_attr(t, a).unwrap_or(0.0);
            let pos = Vec3::new(g(scene, "translateX"), g(scene, "translateY"), g(scene, "translateZ"));
            self.yaw = g(scene, "rotateY");
            self.pitch = (-g(scene, "rotateX")).clamp(-89.0, 89.0);
            self.dist = pos.length().max(1.0);
            let fwd = self.rotation().rotate(Vec3::new(0.0, 0.0, -1.0));
            self.target = pos + fwd * self.dist;
        }
    }

    /// Frame the camera on the bounding sphere of everything visible.
    pub fn frame_scene(&mut self, scene: &mut Scene) {
        let rs = crate::render::build_scene(scene);
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for it in &rs.items {
            for p in &it.mesh.positions {
                let w = it.world.transform_point(*p);
                lo = Vec3::new(lo.x.min(w.x), lo.y.min(w.y), lo.z.min(w.z));
                hi = Vec3::new(hi.x.max(w.x), hi.y.max(w.y), hi.z.max(w.z));
                any = true;
            }
        }
        if !any {
            self.target = Vec3::ZERO;
            self.dist = 22.0;
        } else {
            self.target = (lo + hi) * 0.5;
            let radius = ((hi - lo).length() * 0.5).max(0.5);
            self.dist = (radius * 2.8).clamp(2.0, 500.0);
        }
        self.camera = None;
        self.apply(scene);
    }
}
