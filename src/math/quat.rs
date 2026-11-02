use super::vec::Vec3;
use std::ops::{Mul, Neg};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Quat::identity()
    }
}

impl Quat {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Quat {
        Quat { x, y, z, w }
    }
    pub const fn identity() -> Quat {
        Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }
    }
    pub fn from_axis_angle(axis: Vec3, angle_rad: f32) -> Quat {
        let a = axis.normalized();
        let h = angle_rad * 0.5;
        let s = h.sin();
        Quat::new(a.x * s, a.y * s, a.z * s, h.cos())
    }
    /// Maya style XYZ euler angles in degrees. Rotation order applies X, then Y, then Z.
    pub fn from_euler_deg(r: Vec3) -> Quat {
        let qx = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), r.x.to_radians());
        let qy = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), r.y.to_radians());
        let qz = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), r.z.to_radians());
        qz * qy * qx
    }
    pub fn dot(self, o: Quat) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn normalized(self) -> Quat {
        let l = self.length();
        if l < 1e-12 {
            Quat::identity()
        } else {
            Quat::new(self.x / l, self.y / l, self.z / l, self.w / l)
        }
    }
    pub fn conjugate(self) -> Quat {
        Quat::new(-self.x, -self.y, -self.z, self.w)
    }
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let qv = Vec3::new(self.x, self.y, self.z);
        let t = qv.cross(v) * 2.0;
        v + t * self.w + qv.cross(t)
    }
    pub fn slerp(self, other: Quat, t: f32) -> Quat {
        let mut b = other;
        let mut d = self.dot(b);
        if d < 0.0 {
            b = -b;
            d = -d;
        }
        if d > 0.9995 {
            let r = Quat::new(
                self.x + (b.x - self.x) * t,
                self.y + (b.y - self.y) * t,
                self.z + (b.z - self.z) * t,
                self.w + (b.w - self.w) * t,
            );
            return r.normalized();
        }
        let th0 = d.clamp(-1.0, 1.0).acos();
        let th = th0 * t;
        let s0 = (th0 - th).sin() / th0.sin();
        let s1 = th.sin() / th0.sin();
        Quat::new(
            self.x * s0 + b.x * s1,
            self.y * s0 + b.y * s1,
            self.z * s0 + b.z * s1,
            self.w * s0 + b.w * s1,
        )
    }
}

impl Mul for Quat {
    type Output = Quat;
    fn mul(self, o: Quat) -> Quat {
        Quat::new(
            self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
            self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        )
    }
}

impl Mul<Vec3> for Quat {
    type Output = Vec3;
    fn mul(self, v: Vec3) -> Vec3 {
        self.rotate(v)
    }
}

impl Neg for Quat {
    type Output = Quat;
    fn neg(self) -> Quat {
        Quat::new(-self.x, -self.y, -self.z, -self.w)
    }
}
