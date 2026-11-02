use super::particles::{ParticleSystem, Rng};
use crate::math::Vec3;
use std::collections::HashMap;

/// Nucleus style dynamics solver with a built in particle emitter.
/// Runs fixed sub-step integration for gravity, wind drag, a collision plane and
/// particle-to-particle collisions. Deterministic so timeline scrubbing is repeatable.
#[derive(Clone, Debug)]
pub struct Solver {
    pub enabled: bool,
    pub gravity: Vec3,
    pub wind: Vec3,
    pub wind_drag: f32,
    pub substeps: u32,
    pub plane_y: f32,
    pub bounce: f32,
    pub friction: f32,
    pub particle_collide: bool,
    pub particle_radius: f32,
    pub particle_mass: f32,
    pub emit_rate: f32,
    pub emit_pos: Vec3,
    pub emit_dir: Vec3,
    pub emit_speed: f32,
    pub spread: f32,
    pub lifespan: f32,
    pub max_particles: usize,
    pub start_frame: f32,
    pub seed: u64,
    pub system: ParticleSystem,
    rng: Rng,
    last_frame: i32,
    emit_accum: f32,
}

impl Default for Solver {
    fn default() -> Self {
        Solver::new()
    }
}

impl Solver {
    pub fn new() -> Solver {
        let mut s = Solver {
            enabled: true,
            gravity: Vec3::new(0.0, -9.8, 0.0),
            wind: Vec3::ZERO,
            wind_drag: 0.05,
            substeps: 3,
            plane_y: 0.0,
            bounce: 0.6,
            friction: 0.1,
            particle_collide: true,
            particle_radius: 0.15,
            particle_mass: 1.0,
            emit_rate: 40.0,
            emit_pos: Vec3::new(0.0, 5.0, 0.0),
            emit_dir: Vec3::new(0.0, 1.0, 0.0),
            emit_speed: 4.0,
            spread: 0.35,
            lifespan: 0.0,
            max_particles: 600,
            start_frame: 1.0,
            seed: 1,
            system: ParticleSystem::default(),
            rng: Rng::new(1),
            last_frame: 0,
            emit_accum: 0.0,
        };
        s.reset();
        s
    }

    /// Clear all particles and rewind the solver to its start frame.
    pub fn reset(&mut self) {
        self.system.clear();
        self.rng = Rng::new(self.seed);
        self.emit_accum = 0.0;
        self.last_frame = self.start_frame as i32 - 1;
    }

    pub fn particle_count(&self) -> usize {
        self.system.len()
    }

    /// Bring the simulation to `frame`. Going backwards or before start frame re-simulates from the start.
    pub fn advance_to(&mut self, frame: i32, fps: f32) {
        if !self.enabled {
            return;
        }
        if frame < self.last_frame {
            self.reset();
        }
        let dt = 1.0 / fps.max(1.0);
        while self.last_frame < frame {
            self.last_frame += 1;
            if self.last_frame as f32 >= self.start_frame {
                self.step(dt);
            }
        }
    }

    /// Advance one frame worth of time using sub-steps.
    pub fn step(&mut self, dt: f32) {
        let n = self.substeps.max(1);
        let h = dt / n as f32;
        for _ in 0..n {
            self.emit(h);
            self.integrate(h);
            if self.particle_collide {
                self.collide_particles();
            }
            self.collide_plane();
            self.age_and_cull(h);
        }
    }

    fn emit(&mut self, h: f32) {
        self.emit_accum += self.emit_rate * h;
        while self.emit_accum >= 1.0 {
            self.emit_accum -= 1.0;
            if self.system.len() >= self.max_particles {
                self.emit_accum = 0.0;
                break;
            }
            let jitter = Vec3::new(self.rng.signed(), self.rng.signed(), self.rng.signed());
            let dir = (self.emit_dir.normalized() + jitter * self.spread).normalized();
            let speed = self.emit_speed * (0.8 + 0.4 * self.rng.next_f32());
            let pos = self.emit_pos + jitter * (self.particle_radius * 0.5);
            self.system.spawn(pos, dir * speed, self.particle_radius, self.particle_mass);
        }
    }

    fn integrate(&mut self, h: f32) {
        let g = self.gravity;
        let wind = self.wind;
        let drag = self.wind_drag;
        for p in self.system.items.iter_mut() {
            let acc = g + (wind - p.vel) * (drag / p.mass.max(1e-4));
            p.vel += acc * h;
            p.pos += p.vel * h;
        }
    }

    fn collide_plane(&mut self) {
        let y0 = self.plane_y;
        let bounce = self.bounce;
        let fric = self.friction.clamp(0.0, 1.0);
        for p in self.system.items.iter_mut() {
            if p.pos.y - p.radius < y0 {
                p.pos.y = y0 + p.radius;
                if p.vel.y < 0.0 {
                    p.vel.y = -p.vel.y * bounce;
                    if p.vel.y.abs() < 0.3 {
                        p.vel.y = 0.0;
                    }
                    p.vel.x *= 1.0 - fric;
                    p.vel.z *= 1.0 - fric;
                }
            }
        }
    }

    fn collide_particles(&mut self) {
        let n = self.system.items.len();
        if n < 2 {
            return;
        }
        let max_r = self
            .system
            .items
            .iter()
            .fold(0.0f32, |a, p| a.max(p.radius))
            .max(1e-3);
        let cell = max_r * 2.0;
        let key = |p: Vec3| -> (i32, i32, i32) {
            (
                (p.x / cell).floor() as i32,
                (p.y / cell).floor() as i32,
                (p.z / cell).floor() as i32,
            )
        };
        let mut grid: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
        for (i, p) in self.system.items.iter().enumerate() {
            grid.entry(key(p.pos)).or_default().push(i);
        }
        let e = self.bounce;
        for i in 0..n {
            let k = key(self.system.items[i].pos);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let cell_key = (k.0 + dx, k.1 + dy, k.2 + dz);
                        let list = match grid.get(&cell_key) {
                            Some(l) => l,
                            None => continue,
                        };
                        for &j in list {
                            if j <= i {
                                continue;
                            }
                            let (pi, vi, ri, mi) = {
                                let a = &self.system.items[i];
                                (a.pos, a.vel, a.radius, a.mass.max(1e-4))
                            };
                            let (pj, vj, rj, mj) = {
                                let b = &self.system.items[j];
                                (b.pos, b.vel, b.radius, b.mass.max(1e-4))
                            };
                            let d = pj - pi;
                            let rsum = ri + rj;
                            let d2 = d.length_sq();
                            if d2 >= rsum * rsum {
                                continue;
                            }
                            let (nrm, dist) = if d2 > 1e-12 {
                                let l = d2.sqrt();
                                (d / l, l)
                            } else {
                                (Vec3::new(1.0, 0.0, 0.0), 0.0)
                            };
                            let overlap = rsum - dist;
                            let inv_sum = 1.0 / mi + 1.0 / mj;
                            let ci = overlap * (1.0 / mi) / inv_sum;
                            let cj = overlap * (1.0 / mj) / inv_sum;
                            let vrel = (vj - vi).dot(nrm);
                            let mut dvi = Vec3::ZERO;
                            let mut dvj = Vec3::ZERO;
                            if vrel < 0.0 {
                                let imp = -(1.0 + e) * vrel / inv_sum;
                                dvi = nrm * (-imp / mi);
                                dvj = nrm * (imp / mj);
                            }
                            {
                                let a = &mut self.system.items[i];
                                a.pos -= nrm * ci;
                                a.vel += dvi;
                            }
                            {
                                let b = &mut self.system.items[j];
                                b.pos += nrm * cj;
                                b.vel += dvj;
                            }
                        }
                    }
                }
            }
        }
    }

    fn age_and_cull(&mut self, h: f32) {
        let life = self.lifespan;
        for p in self.system.items.iter_mut() {
            p.age += h;
        }
        if life > 0.0 {
            self.system.items.retain(|p| p.age <= life);
        }
    }
}
