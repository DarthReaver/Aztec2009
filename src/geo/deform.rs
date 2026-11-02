//! Non-linear deformers (bend, twist, flare, squash, sine) and vertex averaging, applied in object space.
//!
//! Every deformer works along one axis (0 = X, 1 = Y, 2 = Z) between a low and a high bound; the
//! other two coordinates are the cross section.

use super::edit::Poly;
use crate::math::Vec3;
use std::f32::consts::PI;

fn get(v: Vec3, k: usize) -> f32 {
    [v.x, v.y, v.z][k]
}

/// Split into (along axis, first cross coordinate, second cross coordinate).
fn split(v: Vec3, axis: usize) -> (f32, f32, f32) {
    (get(v, axis), get(v, (axis + 1) % 3), get(v, (axis + 2) % 3))
}

fn join(a: f32, b: f32, c: f32, axis: usize) -> Vec3 {
    let mut o = [0.0f32; 3];
    o[axis] = a;
    o[(axis + 1) % 3] = b;
    o[(axis + 2) % 3] = c;
    Vec3::new(o[0], o[1], o[2])
}

fn param(a: f32, lo: f32, hi: f32) -> f32 {
    if (hi - lo).abs() < 1e-9 {
        0.0
    } else {
        ((a - lo) / (hi - lo)).clamp(0.0, 1.0)
    }
}

/// Bend the axis into an arc with the given curvature (1 / radius), starting at `lo`.
pub fn bend(pos: &mut [Vec3], axis: usize, lo: f32, hi: f32, curvature: f32) {
    if curvature.abs() < 1e-6 {
        return;
    }
    let r = 1.0 / curvature;
    for p in pos.iter_mut() {
        let (a, b, c) = split(*p, axis);
        let ac = a.clamp(lo.min(hi), lo.max(hi));
        let th = curvature * (ac - lo);
        let (s, co) = th.sin_cos();
        let extra = a - ac;
        let na = lo + (r - b) * s + extra * co;
        let nb = r - (r - b) * co + extra * s;
        *p = join(na, nb, c, axis);
    }
}

/// Rotate the cross section around the axis, from `start` degrees at the low bound to `end` at the high bound.
pub fn twist(pos: &mut [Vec3], axis: usize, lo: f32, hi: f32, start: f32, end: f32) {
    for p in pos.iter_mut() {
        let (a, b, c) = split(*p, axis);
        let t = param(a, lo, hi);
        let (s, co) = (start + (end - start) * t).to_radians().sin_cos();
        *p = join(a, b * co - c * s, b * s + c * co, axis);
    }
}

/// Scale the cross section from `start` (b, c) at the low bound to `end` at the high bound; `curve` bulges the middle.
pub fn flare(pos: &mut [Vec3], axis: usize, lo: f32, hi: f32, start: (f32, f32), end: (f32, f32), curve: f32) {
    for p in pos.iter_mut() {
        let (a, b, c) = split(*p, axis);
        let t = param(a, lo, hi);
        let bulge = curve * 4.0 * t * (1.0 - t);
        let sb = start.0 + (end.0 - start.0) * t + bulge;
        let sc = start.1 + (end.1 - start.1) * t + bulge;
        *p = join(a, b * sb, c * sc, axis);
    }
}

/// Shorten the object along the axis by `factor` while the middle of the cross section bulges by `expand`.
pub fn squash(pos: &mut [Vec3], axis: usize, lo: f32, hi: f32, factor: f32, expand: f32) {
    let stretch = (1.0 - factor).max(0.05);
    let mid = 0.5 * (lo + hi);
    for p in pos.iter_mut() {
        let (a, b, c) = split(*p, axis);
        let t = param(a, lo, hi);
        let radial = 1.0 + factor * expand * (PI * t).sin();
        *p = join(mid + (a - mid) * stretch, b * radial, c, axis);
    }
}

/// Push the first cross coordinate along a sine wave of the axis position.
pub fn sine(pos: &mut [Vec3], axis: usize, lo: f32, hi: f32, amplitude: f32, wavelength: f32, offset: f32, dropoff: f32) {
    let wl = if wavelength.abs() < 1e-4 { 1e-4 } else { wavelength };
    for p in pos.iter_mut() {
        let (a, b, c) = split(*p, axis);
        let t = param(a, lo, hi);
        let d = dropoff.clamp(-1.0, 1.0);
        let env = if d >= 0.0 { 1.0 - d * t } else { 1.0 + d * (1.0 - t) };
        *p = join(a, b + amplitude * env * (2.0 * PI * a / wl + offset).sin(), c, axis);
    }
}

/// Concentric ripples in the plane around `axis`: the position along the axis moves by a sine of the distance from the
/// axis. `min_r` and `max_r` bound the rippling region, `dropoff` fades the amplitude toward `max_r`.
pub fn wave(pos: &mut [Vec3], axis: usize, amplitude: f32, wavelength: f32, offset: f32, min_r: f32, max_r: f32, dropoff: f32) {
    let wl = if wavelength.abs() < 1e-4 { 1e-4 } else { wavelength };
    for p in pos.iter_mut() {
        let (a, b, c) = split(*p, axis);
        let r = (b * b + c * c).sqrt();
        if r < min_r || r > max_r {
            continue;
        }
        let t = if (max_r - min_r).abs() > 1e-6 { ((r - min_r) / (max_r - min_r)).clamp(0.0, 1.0) } else { 0.0 };
        let env = 1.0 - dropoff.clamp(0.0, 1.0) * t;
        *p = join(a + amplitude * env * (2.0 * PI * r / wl + offset).sin(), b, c, axis);
    }
}

/// Laplacian smoothing: every vertex moves toward the average of its edge neighbours.
pub fn average_vertices(poly: &mut Poly, iterations: u32, strength: f32) {
    let n = poly.pos.len();
    let mut nb: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (a, b) in poly.edge_list() {
        nb[a as usize].push(b);
        nb[b as usize].push(a);
    }
    let k = strength.clamp(0.0, 1.0);
    for _ in 0..iterations.min(50) {
        let old = poly.pos.clone();
        for i in 0..n {
            if nb[i].is_empty() {
                continue;
            }
            let mut sum = Vec3::ZERO;
            for &j in &nb[i] {
                sum += old[j as usize];
            }
            let avg = sum * (1.0 / nb[i].len() as f32);
            poly.pos[i] = old[i].lerp(avg, k);
        }
    }
}
