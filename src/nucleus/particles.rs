use crate::math::Vec3;

#[derive(Clone, Debug)]
pub struct Particle {
    pub id: u32,
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub radius: f32,
    pub mass: f32,
}

#[derive(Clone, Debug, Default)]
pub struct ParticleSystem {
    pub items: Vec<Particle>,
    pub next_id: u32,
}

impl ParticleSystem {
    pub fn clear(&mut self) {
        self.items.clear();
        self.next_id = 0;
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn spawn(&mut self, pos: Vec3, vel: Vec3, radius: f32, mass: f32) {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Particle { id, pos, vel, age: 0.0, radius, mass });
    }
}

/// Small deterministic xorshift64* generator so simulations replay identically when scrubbing.
#[derive(Clone, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 { 0x9E3779B97F4A7C15 } else { seed })
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / (1u64 << 24) as f32
    }
    /// Uniform in [-1, 1).
    pub fn signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}
