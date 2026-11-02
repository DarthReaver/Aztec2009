use super::mesh::Mesh;
use crate::math::{Vec2, Vec3};
use std::f32::consts::PI;

/// Append a (sx x sy) quad grid spanning `origin + u*s + v*t` for s,t in 0..1.
fn add_grid(m: &mut Mesh, origin: Vec3, u: Vec3, v: Vec3, normal: Vec3, sx: usize, sy: usize) {
    let base = m.vertex_count() as u32;
    for j in 0..=sy {
        for i in 0..=sx {
            let s = i as f32 / sx as f32;
            let t = j as f32 / sy as f32;
            m.add_vertex(origin + u * s + v * t, normal, Vec2::new(s, t));
        }
    }
    let row = (sx + 1) as u32;
    for j in 0..sy as u32 {
        for i in 0..sx as u32 {
            let a = base + j * row + i;
            let b = a + 1;
            let c = a + row + 1;
            let d = a + row;
            m.add_face(&[a, b, c, d]);
        }
    }
}

/// polyCube: width (x), height (y), depth (z), subdivisions per axis (flat shaded faces).
pub fn poly_cube(w: f32, h: f32, d: f32, sub: usize) -> Mesh {
    let sub = sub.max(1);
    let (hw, hh, hd) = (w * 0.5, h * 0.5, d * 0.5);
    let mut m = Mesh::new();
    // +Z front
    add_grid(&mut m, Vec3::new(-hw, -hh, hd), Vec3::new(w, 0.0, 0.0), Vec3::new(0.0, h, 0.0), Vec3::new(0.0, 0.0, 1.0), sub, sub);
    // -Z back
    add_grid(&mut m, Vec3::new(hw, -hh, -hd), Vec3::new(-w, 0.0, 0.0), Vec3::new(0.0, h, 0.0), Vec3::new(0.0, 0.0, -1.0), sub, sub);
    // +X right
    add_grid(&mut m, Vec3::new(hw, -hh, hd), Vec3::new(0.0, 0.0, -d), Vec3::new(0.0, h, 0.0), Vec3::new(1.0, 0.0, 0.0), sub, sub);
    // -X left
    add_grid(&mut m, Vec3::new(-hw, -hh, -hd), Vec3::new(0.0, 0.0, d), Vec3::new(0.0, h, 0.0), Vec3::new(-1.0, 0.0, 0.0), sub, sub);
    // +Y top
    add_grid(&mut m, Vec3::new(-hw, hh, hd), Vec3::new(w, 0.0, 0.0), Vec3::new(0.0, 0.0, -d), Vec3::new(0.0, 1.0, 0.0), sub, sub);
    // -Y bottom
    add_grid(&mut m, Vec3::new(-hw, -hh, -hd), Vec3::new(w, 0.0, 0.0), Vec3::new(0.0, 0.0, d), Vec3::new(0.0, -1.0, 0.0), sub, sub);
    m
}

/// polySphere: radius, subdivisions (used for both axis and height divisions).
pub fn poly_sphere(r: f32, sub: usize) -> Mesh {
    let sx = sub.max(3);
    let sy = (sub / 2).max(2);
    let mut m = Mesh::new();
    for j in 0..=sy {
        let v = j as f32 / sy as f32;
        let phi = v * PI;
        for i in 0..=sx {
            let u = i as f32 / sx as f32;
            let th = u * 2.0 * PI;
            let n = Vec3::new(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin());
            m.add_vertex(n * r, n, Vec2::new(u, 1.0 - v));
        }
    }
    let row = (sx + 1) as u32;
    for j in 0..sy as u32 {
        for i in 0..sx as u32 {
            let a = j * row + i;
            let b = a + 1;
            let c = a + row + 1;
            let d = a + row;
            if j == 0 {
                m.add_face(&[a, c, d]);
            } else if j == sy as u32 - 1 {
                m.add_face(&[a, b, d]);
            } else {
                m.add_face(&[a, b, c, d]);
            }
        }
    }
    m
}

/// polyCylinder: radius, height, subdivisions around the axis, with end caps.
pub fn poly_cylinder(r: f32, h: f32, sub: usize) -> Mesh {
    let n = sub.max(3);
    let hh = h * 0.5;
    let mut m = Mesh::new();
    for j in 0..2 {
        let y = if j == 0 { -hh } else { hh };
        for i in 0..=n {
            let u = i as f32 / n as f32;
            let th = u * 2.0 * PI;
            let nn = Vec3::new(th.cos(), 0.0, th.sin());
            m.add_vertex(Vec3::new(r * th.cos(), y, r * th.sin()), nn, Vec2::new(u, j as f32));
        }
    }
    let row = (n + 1) as u32;
    for i in 0..n as u32 {
        let a = i;
        let b = i + 1;
        m.add_face(&[a, row + a, row + b, b]);
    }
    for (cap, y, ny) in [(0usize, -hh, -1.0f32), (1usize, hh, 1.0f32)] {
        let ring_start = m.vertex_count() as u32;
        for i in 0..n {
            let th = i as f32 / n as f32 * 2.0 * PI;
            m.add_vertex(
                Vec3::new(r * th.cos(), y, r * th.sin()),
                Vec3::new(0.0, ny, 0.0),
                Vec2::new(0.5 + 0.5 * th.cos(), 0.5 + 0.5 * th.sin()),
            );
        }
        let mut face: Vec<u32> = (0..n as u32).map(|i| ring_start + i).collect();
        if cap == 0 {
            face.reverse();
        }
        m.add_face(&face);
    }
    m
}

/// polyPlane: width (x), height (z) in the XZ plane facing +Y.
pub fn poly_plane(w: f32, h: f32, sub: usize) -> Mesh {
    let sub = sub.max(1);
    let mut m = Mesh::new();
    add_grid(
        &mut m,
        Vec3::new(-w * 0.5, 0.0, h * 0.5),
        Vec3::new(w, 0.0, 0.0),
        Vec3::new(0.0, 0.0, -h),
        Vec3::UP,
        sub,
        sub,
    );
    m
}

/// polyCone: base radius, height, subdivisions around the axis; apex up, with a base cap.
pub fn poly_cone(r: f32, h: f32, sub: usize) -> Mesh {
    let n = sub.max(3);
    let hh = h * 0.5;
    let slope = r / h.max(1e-6);
    let mut m = Mesh::new();
    // side: one apex vertex per triangle keeps the shading smooth around the cone
    for i in 0..n {
        let (t0, t1) = (i as f32 / n as f32 * 2.0 * PI, (i + 1) as f32 / n as f32 * 2.0 * PI);
        let tm = 0.5 * (t0 + t1);
        let nn = |t: f32| Vec3::new(t.cos(), slope, t.sin()).normalized();
        let a = m.add_vertex(Vec3::new(0.0, hh, 0.0), nn(tm), Vec2::new((i as f32 + 0.5) / n as f32, 1.0));
        let b = m.add_vertex(Vec3::new(r * t0.cos(), -hh, r * t0.sin()), nn(t0), Vec2::new(i as f32 / n as f32, 0.0));
        let c = m.add_vertex(Vec3::new(r * t1.cos(), -hh, r * t1.sin()), nn(t1), Vec2::new((i + 1) as f32 / n as f32, 0.0));
        m.add_face(&[a, c, b]);
    }
    let start = m.vertex_count() as u32;
    for i in 0..n {
        let th = i as f32 / n as f32 * 2.0 * PI;
        m.add_vertex(Vec3::new(r * th.cos(), -hh, r * th.sin()), Vec3::new(0.0, -1.0, 0.0), Vec2::new(0.5 + 0.5 * th.cos(), 0.5 + 0.5 * th.sin()));
    }
    let mut cap: Vec<u32> = (0..n as u32).map(|i| start + i).collect();
    cap.reverse();
    m.add_face(&cap);
    m
}

/// polyTorus: ring radius, section (tube) radius, subdivisions around the ring and around the tube.
pub fn poly_torus(r: f32, sr: f32, sub_axis: usize, sub_section: usize) -> Mesh {
    let na = sub_axis.max(3);
    let ns = sub_section.max(3);
    let mut m = Mesh::new();
    for i in 0..=na {
        let u = i as f32 / na as f32;
        let th = u * 2.0 * PI;
        for j in 0..=ns {
            let v = j as f32 / ns as f32;
            let ph = v * 2.0 * PI;
            let nrm = Vec3::new(ph.cos() * th.cos(), ph.sin(), ph.cos() * th.sin());
            let p = Vec3::new((r + sr * ph.cos()) * th.cos(), sr * ph.sin(), (r + sr * ph.cos()) * th.sin());
            m.add_vertex(p, nrm, Vec2::new(u, v));
        }
    }
    let row = (ns + 1) as u32;
    for i in 0..na as u32 {
        for j in 0..ns as u32 {
            let a = i * row + j;
            m.add_face(&[a, a + 1, a + row + 1, a + row]);
        }
    }
    m
}
