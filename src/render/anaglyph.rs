use super::rasterizer::{rasterize, shade};
use super::{CameraInfo, RenderScene, StereoParams};
use crate::math::{Mat4, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct EyeMatrices {
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
}

/// Stereo parameters for a plain (non rig) camera: converge on the world origin.
pub fn default_stereo(cam: &CameraInfo) -> StereoParams {
    let zp = cam.eye().length().max(cam.near * 2.0);
    StereoParams { interaxial: zp * 0.035, zero_parallax: zp }
}

/// Left and right eye matrices with an off-axis (asymmetric) frustum converging at the zero parallax plane.
pub fn eye_matrices(cam: &CameraInfo, st: &StereoParams, aspect: f32) -> (EyeMatrices, EyeMatrices) {
    let near = cam.near;
    let far = cam.far;
    let hh = near * (cam.fov_deg.to_radians() * 0.5).tan();
    let hw = hh * aspect;
    let zp = st.zero_parallax.max(near * 1.01);
    let make = |sign: f32| -> EyeMatrices {
        let e = sign * st.interaxial * 0.5;
        let eye_world = cam.world * Mat4::translation(Vec3::new(e, 0.0, 0.0));
        let shift = -e * near / zp;
        EyeMatrices {
            view: eye_world.inverse(),
            proj: Mat4::frustum(-hw + shift, hw + shift, -hh, hh, near, far),
            eye: eye_world.translation_part(),
        }
    };
    (make(-1.0), make(1.0))
}

/// Optimized red-cyan anaglyph: red from the left eye, green and blue from the right eye.
pub fn combine(left: &[Vec3], right: &[Vec3]) -> Vec<Vec3> {
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| Vec3::new((0.7 * l.y + 0.3 * l.z).clamp(0.0, 1.0), r.y, r.z))
        .collect()
}

pub struct StereoImages {
    pub anaglyph: Vec<Vec3>,
    pub left: Vec<Vec3>,
    pub right: Vec<Vec3>,
}

pub fn render_stereo(
    rs: &RenderScene,
    cam: &CameraInfo,
    st: &StereoParams,
    w: usize,
    h: usize,
    pixel_aspect: f32,
    bg: Vec3,
) -> StereoImages {
    let aspect = (w as f32 / h as f32) * pixel_aspect;
    let (l, r) = eye_matrices(cam, st, aspect);
    let gl = rasterize(rs, &l.view, &l.proj, w, h);
    let gr = rasterize(rs, &r.view, &r.proj, w, h);
    let left = shade(&gl, rs, l.eye, bg);
    let right = shade(&gr, rs, r.eye, bg);
    StereoImages { anaglyph: combine(&left, &right), left, right }
}
