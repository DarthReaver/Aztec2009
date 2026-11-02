use super::quat::Quat;
use super::vec::{Vec3, Vec4};
use std::ops::Mul;

/// Column-major 4x4 matrix. `m[col][row]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub m: [[f32; 4]; 4],
}

impl Default for Mat4 {
    fn default() -> Self {
        Mat4::identity()
    }
}

impl Mat4 {
    pub fn identity() -> Mat4 {
        let mut m = [[0.0; 4]; 4];
        for i in 0..4 {
            m[i][i] = 1.0;
        }
        Mat4 { m }
    }

    pub fn translation(t: Vec3) -> Mat4 {
        let mut r = Mat4::identity();
        r.m[3][0] = t.x;
        r.m[3][1] = t.y;
        r.m[3][2] = t.z;
        r
    }

    pub fn scaling(s: Vec3) -> Mat4 {
        let mut r = Mat4::identity();
        r.m[0][0] = s.x;
        r.m[1][1] = s.y;
        r.m[2][2] = s.z;
        r
    }

    pub fn from_quat(q: Quat) -> Mat4 {
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let mut r = Mat4::identity();
        r.m[0][0] = 1.0 - 2.0 * (y * y + z * z);
        r.m[0][1] = 2.0 * (x * y + z * w);
        r.m[0][2] = 2.0 * (x * z - y * w);
        r.m[1][0] = 2.0 * (x * y - z * w);
        r.m[1][1] = 1.0 - 2.0 * (x * x + z * z);
        r.m[1][2] = 2.0 * (y * z + x * w);
        r.m[2][0] = 2.0 * (x * z + y * w);
        r.m[2][1] = 2.0 * (y * z - x * w);
        r.m[2][2] = 1.0 - 2.0 * (x * x + y * y);
        r
    }

    /// Compose translate * rotate * scale.
    pub fn trs(t: Vec3, q: Quat, s: Vec3) -> Mat4 {
        Mat4::translation(t) * Mat4::from_quat(q) * Mat4::scaling(s)
    }

    pub fn perspective(fov_y_rad: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let f = 1.0 / (fov_y_rad * 0.5).tan();
        let mut r = Mat4 { m: [[0.0; 4]; 4] };
        r.m[0][0] = f / aspect;
        r.m[1][1] = f;
        r.m[2][2] = (far + near) / (near - far);
        r.m[2][3] = -1.0;
        r.m[3][2] = (2.0 * far * near) / (near - far);
        r
    }

    /// Off-axis (asymmetric) perspective frustum, used for stereo zero-parallax projections.
    pub fn frustum(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> Mat4 {
        let mut m = Mat4 { m: [[0.0; 4]; 4] };
        m.m[0][0] = 2.0 * n / (r - l);
        m.m[1][1] = 2.0 * n / (t - b);
        m.m[2][0] = (r + l) / (r - l);
        m.m[2][1] = (t + b) / (t - b);
        m.m[2][2] = -(f + n) / (f - n);
        m.m[2][3] = -1.0;
        m.m[3][2] = -2.0 * f * n / (f - n);
        m
    }

    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
        let f = (target - eye).normalized();
        let mut s = f.cross(up).normalized();
        if s.length_sq() < 1e-12 {
            s = f.cross(Vec3::new(0.0, 0.0, 1.0)).normalized();
        }
        let u = s.cross(f);
        let mut r = Mat4::identity();
        r.m[0][0] = s.x;
        r.m[1][0] = s.y;
        r.m[2][0] = s.z;
        r.m[0][1] = u.x;
        r.m[1][1] = u.y;
        r.m[2][1] = u.z;
        r.m[0][2] = -f.x;
        r.m[1][2] = -f.y;
        r.m[2][2] = -f.z;
        r.m[3][0] = -s.dot(eye);
        r.m[3][1] = -u.dot(eye);
        r.m[3][2] = f.dot(eye);
        r
    }

    /// Split into translation, Maya style XYZ euler rotation (degrees, R = Rz * Ry * Rx) and scale.
    pub fn decompose(&self) -> (Vec3, Vec3, Vec3) {
        let t = self.translation_part();
        let col = |c: usize| Vec3::new(self.m[c][0], self.m[c][1], self.m[c][2]);
        let (c0, c1, c2) = (col(0), col(1), col(2));
        let mut s = Vec3::new(c0.length(), c1.length(), c2.length());
        let det = c0.dot(c1.cross(c2));
        if det < 0.0 {
            s.x = -s.x;
        }
        let n0 = if s.x.abs() > 1e-8 { c0 * (1.0 / s.x) } else { Vec3::new(1.0, 0.0, 0.0) };
        let n1 = if s.y.abs() > 1e-8 { c1 * (1.0 / s.y) } else { Vec3::new(0.0, 1.0, 0.0) };
        let n2 = if s.z.abs() > 1e-8 { c2 * (1.0 / s.z) } else { Vec3::new(0.0, 0.0, 1.0) };
        // R(row, col): columns are n0, n1, n2
        let r = |row: usize, c: usize| -> f32 {
            let v = match c {
                0 => n0,
                1 => n1,
                _ => n2,
            };
            match row {
                0 => v.x,
                1 => v.y,
                _ => v.z,
            }
        };
        let sb = (-r(2, 0)).clamp(-1.0, 1.0);
        let beta = sb.asin();
        let (alpha, gamma) = if sb.abs() < 0.99999 {
            (r(2, 1).atan2(r(2, 2)), r(1, 0).atan2(r(0, 0)))
        } else {
            ((-r(1, 2)).atan2(r(1, 1)), 0.0)
        };
        (t, Vec3::new(alpha.to_degrees(), beta.to_degrees(), gamma.to_degrees()), s)
    }

    pub fn transpose(&self) -> Mat4 {
        let mut r = Mat4::identity();
        for c in 0..4 {
            for row in 0..4 {
                r.m[c][row] = self.m[row][c];
            }
        }
        r
    }

    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        let v = *self * p.extend(1.0);
        if v.w.abs() > 1e-12 && (v.w - 1.0).abs() > 1e-9 {
            v.xyz() / v.w
        } else {
            v.xyz()
        }
    }

    pub fn transform_dir(&self, d: Vec3) -> Vec3 {
        (*self * d.extend(0.0)).xyz()
    }

    pub fn translation_part(&self) -> Vec3 {
        Vec3::new(self.m[3][0], self.m[3][1], self.m[3][2])
    }

    /// General 4x4 inverse via cofactor expansion. Returns identity if singular.
    pub fn inverse(&self) -> Mat4 {
        let mut a = [0.0f32; 16];
        for c in 0..4 {
            for r in 0..4 {
                a[c * 4 + r] = self.m[c][r];
            }
        }
        let mut inv = [0.0f32; 16];
        inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15]
            + a[9] * a[7] * a[14] + a[13] * a[6] * a[11] - a[13] * a[7] * a[10];
        inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15]
            - a[8] * a[7] * a[14] - a[12] * a[6] * a[11] + a[12] * a[7] * a[10];
        inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15]
            + a[8] * a[7] * a[13] + a[12] * a[5] * a[11] - a[12] * a[7] * a[9];
        inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14]
            - a[8] * a[6] * a[13] - a[12] * a[5] * a[10] + a[12] * a[6] * a[9];
        inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15]
            - a[9] * a[3] * a[14] - a[13] * a[2] * a[11] + a[13] * a[3] * a[10];
        inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15]
            + a[8] * a[3] * a[14] + a[12] * a[2] * a[11] - a[12] * a[3] * a[10];
        inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15]
            - a[8] * a[3] * a[13] - a[12] * a[1] * a[11] + a[12] * a[3] * a[9];
        inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14]
            + a[8] * a[2] * a[13] + a[12] * a[1] * a[10] - a[12] * a[2] * a[9];
        inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15]
            + a[5] * a[3] * a[14] + a[13] * a[2] * a[7] - a[13] * a[3] * a[6];
        inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15]
            - a[4] * a[3] * a[14] - a[12] * a[2] * a[7] + a[12] * a[3] * a[6];
        inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15]
            + a[4] * a[3] * a[13] + a[12] * a[1] * a[7] - a[12] * a[3] * a[5];
        inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14]
            - a[4] * a[2] * a[13] - a[12] * a[1] * a[6] + a[12] * a[2] * a[5];
        inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11]
            - a[5] * a[3] * a[10] - a[9] * a[2] * a[7] + a[9] * a[3] * a[6];
        inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11]
            + a[4] * a[3] * a[10] + a[8] * a[2] * a[7] - a[8] * a[3] * a[6];
        inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11]
            - a[4] * a[3] * a[9] - a[8] * a[1] * a[7] + a[8] * a[3] * a[5];
        inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10]
            + a[4] * a[2] * a[9] + a[8] * a[1] * a[6] - a[8] * a[2] * a[5];
        let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
        if det.abs() < 1e-20 {
            return Mat4::identity();
        }
        let id = 1.0 / det;
        let mut r = Mat4::identity();
        for c in 0..4 {
            for row in 0..4 {
                r.m[c][row] = inv[c * 4 + row] * id;
            }
        }
        r
    }
}

impl Mul for Mat4 {
    type Output = Mat4;
    fn mul(self, o: Mat4) -> Mat4 {
        let mut r = Mat4 { m: [[0.0; 4]; 4] };
        for c in 0..4 {
            for row in 0..4 {
                let mut s = 0.0;
                for k in 0..4 {
                    s += self.m[k][row] * o.m[c][k];
                }
                r.m[c][row] = s;
            }
        }
        r
    }
}

impl Mul<Vec4> for Mat4 {
    type Output = Vec4;
    fn mul(self, v: Vec4) -> Vec4 {
        Vec4::new(
            self.m[0][0] * v.x + self.m[1][0] * v.y + self.m[2][0] * v.z + self.m[3][0] * v.w,
            self.m[0][1] * v.x + self.m[1][1] * v.y + self.m[2][1] * v.z + self.m[3][1] * v.w,
            self.m[0][2] * v.x + self.m[1][2] * v.y + self.m[2][2] * v.z + self.m[3][2] * v.w,
            self.m[0][3] * v.x + self.m[1][3] * v.y + self.m[2][3] * v.z + self.m[3][3] * v.w,
        )
    }
}
