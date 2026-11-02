//! NURBS curves and surfaces (non-rational B-splines): evaluation, knot insertion, splitting, joining and the
//! surface generators behind Create > NURBS Primitives, Revolve, Loft and Extrude.

use super::mesh::Mesh;
use crate::math::{Vec2, Vec3};
use std::collections::HashMap;

/// One parameter direction: degree, number of control points, knots and whether it wraps around.
/// Periodic directions keep their `n` unique control points; the first `degree` are repeated to close them.
#[derive(Clone, Debug, PartialEq)]
pub struct Dir {
    pub degree: usize,
    pub n: usize,
    pub periodic: bool,
    pub knots: Vec<f32>,
}

impl Dir {
    /// Clamped uniform knots: the curve starts and ends on its end control points.
    pub fn open(n: usize, degree: usize) -> Dir {
        let degree = degree.min(n.saturating_sub(1)).max(1);
        let m = n - degree;
        let mut knots = vec![0.0; degree + 1];
        for i in 1..m {
            knots.push(i as f32);
        }
        knots.extend(std::iter::repeat(m as f32).take(degree + 1));
        Dir { degree, n, periodic: false, knots }
    }

    /// Uniform knots closed into a loop (needs more control points than the degree).
    pub fn periodic(n: usize, degree: usize) -> Dir {
        let degree = degree.min(n.saturating_sub(1)).max(1);
        let ext = n + degree;
        Dir { degree, n, periodic: true, knots: (0..ext + degree + 1).map(|i| i as f32).collect() }
    }

    pub fn ext(&self) -> usize {
        if self.periodic {
            self.n + self.degree
        } else {
            self.n
        }
    }

    pub fn domain(&self) -> (f32, f32) {
        (self.knots[self.degree], self.knots[self.ext()])
    }

    /// Number of polynomial pieces.
    pub fn spans(&self) -> usize {
        if self.periodic {
            self.n
        } else {
            self.n.saturating_sub(self.degree).max(1)
        }
    }

    /// Unique control point indices influencing parameter `t`, with their basis weights.
    pub fn basis(&self, t: f32) -> (Vec<usize>, Vec<f32>) {
        let d = self.degree;
        let (a, b) = self.domain();
        let t = if self.periodic {
            let len = b - a;
            a + (t - a).rem_euclid(len)
        } else {
            t.clamp(a, b)
        };
        let ext = self.ext();
        let mut k = d;
        while k + 1 < ext && t >= self.knots[k + 1] {
            k += 1;
        }
        let mut n = vec![0.0f32; d + 1];
        let mut left = vec![0.0f32; d + 1];
        let mut right = vec![0.0f32; d + 1];
        n[0] = 1.0;
        for j in 1..=d {
            left[j] = t - self.knots[k + 1 - j];
            right[j] = self.knots[k + j] - t;
            let mut saved = 0.0;
            for r in 0..j {
                let den = right[r + 1] + left[j - r];
                let tmp = if den.abs() < 1e-12 { 0.0 } else { n[r] / den };
                n[r] = saved + right[r + 1] * tmp;
                saved = left[j - r] * tmp;
            }
            n[j] = saved;
        }
        let idx = (0..=d).map(|i| (k - d + i) % self.n).collect();
        (idx, n)
    }

    /// Greville abscissae: the parameter at which each control point has most influence.
    pub fn greville(&self) -> Vec<f32> {
        (0..self.n).map(|i| (1..=self.degree).map(|j| self.knots[i + j]).sum::<f32>() / self.degree as f32).collect()
    }

    /// Parameters of the span boundaries.
    pub fn span_params(&self) -> Vec<f32> {
        let (a, b) = self.domain();
        let mut v = vec![a];
        for &k in &self.knots {
            if k > a + 1e-6 && k < b - 1e-6 && (v.last().map(|l| k > *l + 1e-6).unwrap_or(true)) {
                v.push(k);
            }
        }
        v.push(b);
        v
    }

    /// Insert knot `t` once (open directions). Returns the weights mixing old control points: new[i] = a_i old[i] + (1-a_i) old[i-1].
    fn insert(&self, t: f32) -> Option<(Dir, Vec<(usize, usize, f32)>)> {
        if self.periodic {
            return None;
        }
        let d = self.degree;
        let (lo, hi) = self.domain();
        if t <= lo + 1e-6 || t >= hi - 1e-6 {
            return None;
        }
        let mut k = d;
        while k + 1 < self.n && t >= self.knots[k + 1] {
            k += 1;
        }
        let mut mix = Vec::with_capacity(self.n + 1);
        for i in 0..=self.n {
            if i <= k - d {
                mix.push((i, i, 1.0));
            } else if i > k {
                mix.push((i - 1, i - 1, 0.0));
            } else {
                let a = (t - self.knots[i]) / (self.knots[i + d] - self.knots[i]);
                mix.push((i, i - 1, a));
            }
        }
        let mut knots = self.knots.clone();
        knots.insert(k + 1, t);
        Some((Dir { degree: d, n: self.n + 1, periodic: false, knots }, mix))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub dir: Dir,
    pub cvs: Vec<Vec3>,
}

impl Curve {
    pub fn open(cvs: Vec<Vec3>, degree: usize) -> Curve {
        let dir = Dir::open(cvs.len().max(2), degree);
        Curve { dir, cvs }
    }

    pub fn periodic(cvs: Vec<Vec3>, degree: usize) -> Curve {
        let dir = Dir::periodic(cvs.len(), degree);
        Curve { dir, cvs }
    }

    pub fn point(&self, t: f32) -> Vec3 {
        let (idx, w) = self.dir.basis(t);
        idx.iter().zip(&w).fold(Vec3::ZERO, |a, (&i, &w)| a + self.cvs[i] * w)
    }

    pub fn tangent(&self, t: f32) -> Vec3 {
        let (a, b) = self.dir.domain();
        let h = (b - a) * 1e-3;
        let (t0, t1) = if self.dir.periodic { (t - h, t + h) } else { ((t - h).max(a), (t + h).min(b)) };
        (self.point(t1) - self.point(t0)).normalized()
    }

    pub fn start(&self) -> Vec3 {
        self.point(self.dir.domain().0)
    }

    pub fn end(&self) -> Vec3 {
        self.point(self.dir.domain().1)
    }

    /// Points along the curve, `per_span` per span (closing back on the start for periodic curves).
    pub fn sample(&self, per_span: usize) -> Vec<Vec3> {
        let ps = self.dir.span_params();
        let mut out = Vec::new();
        for w in ps.windows(2) {
            for k in 0..per_span {
                out.push(self.point(w[0] + (w[1] - w[0]) * k as f32 / per_span as f32));
            }
        }
        out.push(self.point(*ps.last().unwrap()));
        out
    }

    /// Arc length by sampling.
    pub fn length(&self) -> f32 {
        let s = self.sample(16);
        s.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    pub fn reverse(&mut self) {
        self.cvs.reverse();
        let (a, b) = self.dir.domain();
        let _ = (a, b);
        let top = *self.dir.knots.last().unwrap() + self.dir.knots[0];
        self.dir.knots = self.dir.knots.iter().rev().map(|k| top - k).collect();
    }

    pub fn transform(&mut self, f: impl Fn(Vec3) -> Vec3) {
        for c in self.cvs.iter_mut() {
            *c = f(*c);
        }
    }

    /// Insert a knot at `t`, keeping the shape.
    pub fn insert_knot(&self, t: f32) -> Result<Curve, String> {
        let (dir, mix) = self.dir.insert(t).ok_or("Cannot insert a knot there (open curves only, inside the parameter range)")?;
        let cvs = mix.iter().map(|&(i, j, a)| self.cvs[i] * a + self.cvs[j] * (1.0 - a)).collect();
        Ok(Curve { dir, cvs })
    }

    /// Cut the curve at `t` into two curves.
    pub fn split(&self, t: f32) -> Result<(Curve, Curve), String> {
        if self.dir.periodic {
            return Err("Open the curve first: periodic curves cannot be detached".to_string());
        }
        let d = self.dir.degree;
        let mut c = self.clone();
        let mult = |c: &Curve| c.dir.knots.iter().filter(|k| (**k - t).abs() < 1e-5).count();
        let mut guard = 0;
        while mult(&c) < d && guard < 8 {
            c = c.insert_knot(t)?;
            guard += 1;
        }
        let a = c.dir.knots.iter().position(|k| (*k - t).abs() < 1e-5).ok_or("Cannot split there")?;
        let left_n = a;
        let mut lk: Vec<f32> = c.dir.knots[..a + d].to_vec();
        lk.push(t);
        let mut rk: Vec<f32> = vec![t];
        rk.extend_from_slice(&c.dir.knots[a..]);
        let left = Curve { dir: Dir { degree: d, n: left_n, periodic: false, knots: lk }, cvs: c.cvs[..left_n].to_vec() };
        let right = Curve { dir: Dir { degree: d, n: c.cvs.len() - a + 1, periodic: false, knots: rk }, cvs: c.cvs[a - 1..].to_vec() };
        Ok((left, right))
    }

    /// Join the end of `self` to the start of `other` into one curve of the lower degree, with uniform knots.
    /// The joint control point sits halfway between the two ends.
    pub fn attach(&self, other: &Curve) -> Curve {
        let degree = self.dir.degree.min(other.dir.degree);
        let mut cvs = self.cvs.clone();
        let join = (self.end() + other.start()) * 0.5;
        if let Some(l) = cvs.last_mut() {
            *l = join;
        }
        cvs.extend(other.cvs.iter().skip(1).copied());
        Curve::open(cvs, degree)
    }
}

/// Solve A x = b (n x n) for three right-hand sides at once; None when singular.
fn solve(mut a: Vec<Vec<f32>>, mut b: Vec<Vec3>) -> Option<Vec<Vec3>> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().partial_cmp(&a[j][c].abs()).unwrap())?;
        if a[p][c].abs() < 1e-9 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            if f != 0.0 {
                for k in c..n {
                    a[r][k] -= f * a[c][k];
                }
                let bc = b[c];
                b[r] = b[r] - bc * f;
            }
        }
    }
    let mut x = vec![Vec3::ZERO; n];
    for r in (0..n).rev() {
        let mut s = b[r];
        for k in r + 1..n {
            s = s - x[k] * a[r][k];
        }
        x[r] = s * (1.0 / a[r][r]);
    }
    Some(x)
}

/// An open uniform curve of `degree` that passes through `pts` (one control point per point; the point k is
/// reached at the k-th Greville parameter).
pub fn interpolate(pts: &[Vec3], degree: usize) -> Curve {
    let n = pts.len();
    if n < 3 {
        return Curve::open(pts.to_vec(), 1);
    }
    let dir = Dir::open(n, degree);
    let g = dir.greville();
    let a: Vec<Vec<f32>> = g
        .iter()
        .map(|&t| {
            let mut row = vec![0.0; n];
            let (idx, w) = dir.basis(t);
            for (i, w) in idx.iter().zip(&w) {
                row[*i] += w;
            }
            row
        })
        .collect();
    match solve(a, pts.to_vec()) {
        Some(cvs) => Curve { dir, cvs },
        None => Curve::open(pts.to_vec(), degree),
    }
}

/// Resample a curve to `n` control points that follow the same shape (Rebuild Curve with uniform spans).
pub fn rebuild_curve(c: &Curve, n: usize, degree: usize) -> Curve {
    let (a, b) = c.dir.domain();
    if c.dir.periodic {
        // a closed loop: sample around and fit a periodic curve through the points
        let pts: Vec<Vec3> = (0..n).map(|i| c.point(a + (b - a) * i as f32 / n as f32)).collect();
        return periodic_through(&pts, degree);
    }
    let dir = Dir::open(n.max(degree + 1), degree);
    let g = dir.greville();
    let (ga, gb) = (g[0], g[g.len() - 1]);
    let pts: Vec<Vec3> = g.iter().map(|&t| c.point(a + (b - a) * (t - ga) / (gb - ga).max(1e-9))).collect();
    interpolate(&pts, degree)
}

/// A closed periodic curve that passes through `pts` at its knots.
pub fn periodic_through(pts: &[Vec3], degree: usize) -> Curve {
    let n = pts.len();
    let dir = Dir::periodic(n, degree);
    // at integer parameters d + i the curve is a mix of cvs i-1.. with fixed uniform weights
    let a: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            let mut row = vec![0.0; n];
            let (idx, w) = dir.basis(degree as f32 + i as f32);
            for (j, w) in idx.iter().zip(&w) {
                row[*j] += w;
            }
            row
        })
        .collect();
    match solve(a, pts.to_vec()) {
        Some(cvs) => Curve { dir, cvs },
        None => Curve { dir, cvs: pts.to_vec() },
    }
}

/// A circle (or arc) as a closed periodic curve: `sections` control points on a ring that the curve follows
/// at its knots.
pub fn circle(radius: f32, sections: usize, degree: usize) -> Curve {
    let n = sections.max(degree + 1);
    let pts: Vec<Vec3> = (0..n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            Vec3::new(a.cos() * radius, 0.0, -a.sin() * radius)
        })
        .collect();
    periodic_through(&pts, degree)
}

/// A trim boundary in parameter space. The surface keeps the points inside (`keep`) or outside the loop.
#[derive(Clone, Debug, PartialEq)]
pub struct Trim {
    pub pts: Vec<(f32, f32)>,
    pub keep: bool,
}

/// Even-odd point in polygon test.
pub fn inside_loop(pts: &[(f32, f32)], u: f32, v: f32) -> bool {
    let mut inside = false;
    let n = pts.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (pts[i], pts[j]);
        if (a.1 > v) != (b.1 > v) && u < (b.0 - a.0) * (v - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub u: Dir,
    pub v: Dir,
    /// Row-major: `cvs[i * v.n + j]`.
    pub cvs: Vec<Vec3>,
    /// Trim loops (Edit NURBS > Trim Tool); a point is on the surface when every trim accepts it.
    pub trims: Vec<Trim>,
}

impl Surface {
    pub fn cv(&self, i: usize, j: usize) -> Vec3 {
        self.cvs[i * self.v.n + j]
    }

    pub fn point(&self, u: f32, v: f32) -> Vec3 {
        let (iu, wu) = self.u.basis(u);
        let (iv, wv) = self.v.basis(v);
        let mut p = Vec3::ZERO;
        for (a, &i) in iu.iter().enumerate() {
            for (b, &j) in iv.iter().enumerate() {
                p += self.cv(i, j) * (wu[a] * wv[b]);
            }
        }
        p
    }

    pub fn normal(&self, u: f32, v: f32) -> Vec3 {
        let (ua, ub) = self.u.domain();
        let (va, vb) = self.v.domain();
        let (hu, hv) = ((ub - ua) * 1e-3, (vb - va) * 1e-3);
        let du = self.point(u + hu, v) - self.point(u - hu, v);
        let dv = self.point(u, v + hv) - self.point(u, v - hv);
        du.cross(dv).normalized()
    }

    /// Control point rows of the isoparametric curve at `u` (a curve running in v) or at `v` (running in u).
    pub fn isoparm(&self, along_u: bool, t: f32) -> Curve {
        if along_u {
            // constant v: a curve in the u direction
            let (iv, wv) = self.v.basis(t);
            let cvs = (0..self.u.n).map(|i| iv.iter().zip(&wv).fold(Vec3::ZERO, |a, (&j, &w)| a + self.cv(i, j) * w)).collect();
            Curve { dir: self.u.clone(), cvs }
        } else {
            let (iu, wu) = self.u.basis(t);
            let cvs = (0..self.v.n).map(|j| iu.iter().zip(&wu).fold(Vec3::ZERO, |a, (&i, &w)| a + self.cv(i, j) * w)).collect();
            Curve { dir: self.v.clone(), cvs }
        }
    }

    /// Insert an isoparm: a knot at `t` in the u direction (`in_u`) or v direction.
    pub fn insert_knot(&self, in_u: bool, t: f32) -> Result<Surface, String> {
        let dir = if in_u { &self.u } else { &self.v };
        let (nd, mix) = dir.insert(t).ok_or("Cannot insert an isoparm there (open directions, inside the range)")?;
        let mut s = self.clone();
        if in_u {
            let mut cvs = Vec::with_capacity(nd.n * self.v.n);
            for &(i, j, a) in &mix {
                for c in 0..self.v.n {
                    cvs.push(self.cv(i, c) * a + self.cv(j, c) * (1.0 - a));
                }
            }
            s.cvs = cvs;
            s.u = nd;
        } else {
            let mut cvs = Vec::with_capacity(self.u.n * nd.n);
            for r in 0..self.u.n {
                for &(i, j, a) in &mix {
                    cvs.push(self.cv(r, i) * a + self.cv(r, j) * (1.0 - a));
                }
            }
            s.cvs = cvs;
            s.v = nd;
        }
        Ok(s)
    }

    /// Cut the surface along the isoparm at `t` into two surfaces.
    pub fn split(&self, in_u: bool, t: f32) -> Result<(Surface, Surface), String> {
        let dir = if in_u { &self.u } else { &self.v };
        if dir.periodic {
            return Err("Detaching needs an open direction (a periodic one has no ends; use a closed-to-open conversion first)".to_string());
        }
        let d = dir.degree;
        let mut s = self.clone();
        let mult = |s: &Surface| (if in_u { &s.u } else { &s.v }).knots.iter().filter(|k| (**k - t).abs() < 1e-5).count();
        let mut guard = 0;
        while mult(&s) < d && guard < 8 {
            s = s.insert_knot(in_u, t)?;
            guard += 1;
        }
        let dir = if in_u { &s.u } else { &s.v };
        let a = dir.knots.iter().position(|k| (*k - t).abs() < 1e-5).ok_or("Cannot detach there")?;
        let mut lk: Vec<f32> = dir.knots[..a + d].to_vec();
        lk.push(t);
        let mut rk: Vec<f32> = vec![t];
        rk.extend_from_slice(&dir.knots[a..]);
        let total = dir.n;
        let take = |s: &Surface, range: std::ops::Range<usize>, knots: Vec<f32>| -> Surface {
            let mut out = s.clone();
            if in_u {
                let mut cvs = Vec::new();
                for i in range.clone() {
                    for j in 0..s.v.n {
                        cvs.push(s.cv(i, j));
                    }
                }
                out.cvs = cvs;
                out.u = Dir { degree: d, n: range.len(), periodic: false, knots };
            } else {
                let mut cvs = Vec::new();
                for i in 0..s.u.n {
                    for j in range.clone() {
                        cvs.push(s.cv(i, j));
                    }
                }
                out.cvs = cvs;
                out.v = Dir { degree: d, n: range.len(), periodic: false, knots };
            }
            out.trims.clear();
            out
        };
        Ok((take(&s, 0..a, lk), take(&s, a - 1..total, rk)))
    }

    /// Join `other` onto the end of `self` along u (or v). The shared edge is averaged. Both need the same
    /// structure in the other direction.
    pub fn attach(&self, other: &Surface, in_u: bool) -> Result<Surface, String> {
        let (a, b) = if in_u { (&self.v, &other.v) } else { (&self.u, &other.u) };
        if a.n != b.n || a.degree != b.degree || a.periodic != b.periodic {
            return Err("The surfaces need the same number of CVs across the join (rebuild them to match)".to_string());
        }
        let (da, db) = if in_u { (&self.u, &other.u) } else { (&self.v, &other.v) };
        if da.periodic || db.periodic {
            return Err("Periodic surfaces cannot be attached in the periodic direction".to_string());
        }
        let degree = da.degree.min(db.degree);
        let mut out = self.clone();
        if in_u {
            let mut rows: Vec<Vec<Vec3>> = (0..self.u.n).map(|i| (0..self.v.n).map(|j| self.cv(i, j)).collect()).collect();
            let last = rows.len() - 1;
            for j in 0..self.v.n {
                rows[last][j] = (self.cv(self.u.n - 1, j) + other.cv(0, j)) * 0.5;
            }
            for i in 1..other.u.n {
                rows.push((0..other.v.n).map(|j| other.cv(i, j)).collect());
            }
            out.u = Dir::open(rows.len(), degree);
            out.cvs = rows.into_iter().flatten().collect();
        } else {
            let mut rows: Vec<Vec<Vec3>> = Vec::new();
            for i in 0..self.u.n {
                let mut r: Vec<Vec3> = (0..self.v.n).map(|j| self.cv(i, j)).collect();
                let l = r.len() - 1;
                r[l] = (self.cv(i, self.v.n - 1) + other.cv(i, 0)) * 0.5;
                r.extend((1..other.v.n).map(|j| other.cv(i, j)));
                rows.push(r);
            }
            out.v = Dir::open(rows[0].len(), degree);
            out.cvs = rows.into_iter().flatten().collect();
        }
        out.trims.clear();
        Ok(out)
    }

    /// Edit Surfaces > Move Seam: rotate the periodic direction so its seam sits `shift` isoparms further on.
    pub fn move_seam(&self, in_u: bool, shift: usize) -> Result<Surface, String> {
        let dir = if in_u { &self.u } else { &self.v };
        if !dir.periodic {
            return Err("The surface has no seam in that direction".to_string());
        }
        let n = dir.n;
        let mut out = self.clone();
        for i in 0..self.u.n {
            for j in 0..self.v.n {
                let (si, sj) = if in_u { ((i + shift) % n, j) } else { (i, (j + shift) % n) };
                out.cvs[i * self.v.n + j] = self.cv(si, sj);
            }
        }
        out.trims.clear();
        Ok(out)
    }

    pub fn transform(&mut self, f: impl Fn(Vec3) -> Vec3) {
        for c in self.cvs.iter_mut() {
            *c = f(*c);
        }
    }

    /// Triangulated display surface. UVs run 0..1 across the parameter ranges (so file textures map as in Maya).
    pub fn tessellate(&self, per_span: usize) -> Mesh {
        if !self.trims.is_empty() {
            return self.tessellate_trimmed(per_span.max(10));
        }
        let pu = self.u.span_params();
        let pv = self.v.span_params();
        let samples = |ps: &[f32]| -> Vec<f32> {
            let mut out = Vec::new();
            for w in ps.windows(2) {
                for k in 0..per_span {
                    out.push(w[0] + (w[1] - w[0]) * k as f32 / per_span as f32);
                }
            }
            out.push(*ps.last().unwrap());
            out
        };
        let (us, vs) = (samples(&pu), samples(&pv));
        let (ua, ub) = self.u.domain();
        let (va, vb) = self.v.domain();
        let mut m = Mesh::new();
        for &u in &us {
            for &v in &vs {
                // nudge off the closing edge so periodic seams evaluate from the right side
                let (eu, ev) = (if self.u.periodic && u >= ub - 1e-6 { ua } else { u }, if self.v.periodic && v >= vb - 1e-6 { va } else { v });
                m.add_vertex(self.point(eu, ev), Vec3::UP, Vec2::new((u - ua) / (ub - ua), (v - va) / (vb - va)));
            }
        }
        let w = vs.len() as u32;
        for i in 0..us.len() as u32 - 1 {
            for j in 0..w - 1 {
                let (a, b, c, d) = (i * w + j, i * w + j + 1, (i + 1) * w + j + 1, (i + 1) * w + j);
                // poles collapse a quad to a triangle
                let quad = [a, b, c, d];
                let mut f: Vec<u32> = Vec::new();
                for &q in &quad {
                    if f.last().map(|l| m.positions[*l as usize].distance(m.positions[q as usize]) > 1e-6).unwrap_or(true) {
                        f.push(q);
                    }
                }
                if f.len() > 1 && m.positions[f[0] as usize].distance(m.positions[*f.last().unwrap() as usize]) <= 1e-6 {
                    f.pop();
                }
                if f.len() >= 3 {
                    m.add_face(&f);
                }
            }
        }
        m.recompute_normals();
        // one normal per position so the seam and the poles shade smoothly
        let mut sum: HashMap<(i64, i64, i64), Vec3> = HashMap::new();
        let key = |p: Vec3| ((p.x * 2000.0).round() as i64, (p.y * 2000.0).round() as i64, (p.z * 2000.0).round() as i64);
        for (p, n) in m.positions.iter().zip(&m.normals) {
            *sum.entry(key(*p)).or_insert(Vec3::ZERO) += *n;
        }
        for (p, n) in m.positions.iter().zip(m.normals.iter_mut()) {
            let s = sum[&key(*p)].normalized();
            if s.length_sq() > 0.0 {
                *n = s;
            }
        }
        m
    }

    /// Does the trimmed surface contain the parameter point?
    pub fn accepts(&self, u: f32, v: f32) -> bool {
        self.trims.iter().all(|t| inside_loop(&t.pts, u, v) == t.keep)
    }

    fn eval_wrapped(&self, u: f32, v: f32) -> Vec3 {
        let (ua, ub) = self.u.domain();
        let (va, vb) = self.v.domain();
        self.point(if self.u.periodic && u >= ub - 1e-6 { ua } else { u }, if self.v.periodic && v >= vb - 1e-6 { va } else { v })
    }

    /// Tessellation with trim loops: cells are clipped at the boundary (marching squares on the trim test).
    fn tessellate_trimmed(&self, per_span: usize) -> Mesh {
        let samples = |ps: &[f32]| -> Vec<f32> {
            let mut out = Vec::new();
            for w in ps.windows(2) {
                for k in 0..per_span {
                    out.push(w[0] + (w[1] - w[0]) * k as f32 / per_span as f32);
                }
            }
            out.push(*ps.last().unwrap());
            out
        };
        let (us, vs) = (samples(&self.u.span_params()), samples(&self.v.span_params()));
        let (ua, ub) = self.u.domain();
        let (va, vb) = self.v.domain();
        let inside: Vec<Vec<bool>> = us.iter().map(|&u| vs.iter().map(|&v| self.accepts(u, v)).collect()).collect();
        let mut m = Mesh::new();
        let mut cache: HashMap<(i64, i64), u32> = HashMap::new();
        let mut vert = |m: &mut Mesh, u: f32, v: f32| -> u32 {
            let key = ((u * 100000.0).round() as i64, (v * 100000.0).round() as i64);
            *cache.entry(key).or_insert_with(|| m.add_vertex(self.eval_wrapped(u, v), Vec3::UP, Vec2::new((u - ua) / (ub - ua), (v - va) / (vb - va))))
        };
        // the point on the edge a-b where the trim test flips
        let cross = |a: (f32, f32), b: (f32, f32), a_in: bool| -> (f32, f32) {
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..14 {
                let mid = (lo + hi) * 0.5;
                let p = (a.0 + (b.0 - a.0) * mid, a.1 + (b.1 - a.1) * mid);
                if self.accepts(p.0, p.1) == a_in {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let t = (lo + hi) * 0.5;
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        };
        for i in 0..us.len() - 1 {
            for j in 0..vs.len() - 1 {
                let c = [(us[i], vs[j]), (us[i], vs[j + 1]), (us[i + 1], vs[j + 1]), (us[i + 1], vs[j])];
                let flags = [inside[i][j], inside[i][j + 1], inside[i + 1][j + 1], inside[i + 1][j]];
                let count = flags.iter().filter(|f| **f).count();
                if count == 0 {
                    continue;
                }
                let mut poly: Vec<(f32, f32)> = Vec::new();
                let mut crossings = 0;
                let mut cross_pts: Vec<(usize, (f32, f32))> = Vec::new();
                for k in 0..4 {
                    let k2 = (k + 1) % 4;
                    if flags[k] {
                        poly.push(c[k]);
                    }
                    if flags[k] != flags[k2] {
                        let p = if flags[k] { cross(c[k], c[k2], true) } else { cross(c[k2], c[k], true) };
                        poly.push(p);
                        cross_pts.push((k, p));
                        crossings += 1;
                    }
                }
                if crossings == 4 {
                    // saddle: a small triangle at every inside corner
                    for k in 0..4 {
                        if flags[k] {
                            let prev = cross_pts.iter().find(|(e, _)| (*e + 1) % 4 == k).map(|x| x.1);
                            let next = cross_pts.iter().find(|(e, _)| *e == k).map(|x| x.1);
                            if let (Some(a), Some(b)) = (prev, next) {
                                let ids = [vert(&mut m, a.0, a.1), vert(&mut m, c[k].0, c[k].1), vert(&mut m, b.0, b.1)];
                                m.add_face(&ids);
                            }
                        }
                    }
                    continue;
                }
                let mut ids: Vec<u32> = Vec::new();
                for p in poly {
                    let id = vert(&mut m, p.0, p.1);
                    if ids.last().map(|l| m.positions[*l as usize].distance(m.positions[id as usize]) > 1e-6).unwrap_or(true) {
                        ids.push(id);
                    }
                }
                if ids.len() > 1 && m.positions[ids[0] as usize].distance(m.positions[*ids.last().unwrap() as usize]) <= 1e-6 {
                    ids.pop();
                }
                if ids.len() >= 3 {
                    m.add_face(&ids);
                }
            }
        }
        m.recompute_normals();
        let mut sum: HashMap<(i64, i64, i64), Vec3> = HashMap::new();
        let key = |p: Vec3| ((p.x * 2000.0).round() as i64, (p.y * 2000.0).round() as i64, (p.z * 2000.0).round() as i64);
        for (p, n) in m.positions.iter().zip(&m.normals) {
            *sum.entry(key(*p)).or_insert(Vec3::ZERO) += *n;
        }
        for (p, n) in m.positions.iter().zip(m.normals.iter_mut()) {
            let s = sum[&key(*p)].normalized();
            if s.length_sq() > 0.0 {
                *n = s;
            }
        }
        m
    }

    /// Where a point `p` moved along `dir` meets the surface: the parameter closest to the line, and its distance.
    pub fn project_point(&self, p: Vec3, dir: Vec3) -> Option<(f32, f32, f32)> {
        let d = dir.normalized();
        let (ua, ub) = self.u.domain();
        let (va, vb) = self.v.domain();
        let line_dist = |s: Vec3| -> (f32, f32) {
            let r = s - p;
            let t = r.dot(d);
            ((r - d * t).length(), t)
        };
        let (nu, nv) = (self.u.spans() * 12, self.v.spans() * 12);
        let mut best: Option<(f32, f32, f32, f32)> = None;
        let mut cell = f32::MAX;
        for i in 0..=nu {
            for j in 0..=nv {
                let (u, v) = (ua + (ub - ua) * i as f32 / nu as f32, va + (vb - va) * j as f32 / nv as f32);
                let s = self.eval_wrapped(u, v);
                let (dist, t) = line_dist(s);
                if i + 1 <= nu && j + 1 <= nv {
                    cell = cell.min(s.distance(self.eval_wrapped(ua + (ub - ua) * (i + 1) as f32 / nu as f32, v)).max(1e-4));
                }
                // prefer hits near the point (front side) among the lines that pass close by
                let score = dist * 4.0 + t.abs() * 0.02;
                if best.map(|b| score < b.3).unwrap_or(true) {
                    best = Some((u, v, dist, score));
                }
            }
        }
        let (mut u, mut v, _, _) = best?;
        let mut step = ((ub - ua) / nu as f32).max((vb - va) / nv as f32);
        let f = |s: &Surface, u: f32, v: f32| line_dist(s.eval_wrapped(u, v)).0;
        let mut cur = f(self, u, v);
        for _ in 0..80 {
            let mut improved = false;
            for (du, dv) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0)] {
                let (nu_, nv_) = (u + du * step, v + dv * step);
                let (nu_, nv_) = (if self.u.periodic { ua + (nu_ - ua).rem_euclid(ub - ua) } else { nu_.clamp(ua, ub) }, if self.v.periodic { va + (nv_ - va).rem_euclid(vb - va) } else { nv_.clamp(va, vb) });
                let c = f(self, nu_, nv_);
                if c < cur - 1e-9 {
                    cur = c;
                    u = nu_;
                    v = nv_;
                    improved = true;
                }
            }
            if !improved {
                step *= 0.5;
                if step < 1e-6 {
                    break;
                }
            }
        }
        // a miss: the line passes more than a cell away from the surface
        if cur > cell.max(1e-3) * 1.5 {
            return None;
        }
        Some((u, v, cur))
    }

    /// Rebuild with a new structure (spans and degree per direction) that follows the same shape.
    pub fn rebuild(&self, spans_u: usize, spans_v: usize, degree_u: usize, degree_v: usize) -> Surface {
        let mk = |old: &Dir, spans: usize, degree: usize| -> Dir {
            if old.periodic {
                Dir::periodic(spans.max(3), degree)
            } else {
                Dir::open(spans.max(1) + degree.max(1), degree)
            }
        };
        let (nu, nv) = (mk(&self.u, spans_u, degree_u), mk(&self.v, spans_v, degree_v));
        // old parameter for the k-th sample of a new direction
        let params = |old: &Dir, new: &Dir| -> Vec<f32> {
            let (a, b) = old.domain();
            if new.periodic {
                (0..new.n).map(|i| a + (b - a) * i as f32 / new.n as f32).collect()
            } else {
                let g = new.greville();
                let (ga, gb) = (g[0], g[g.len() - 1]);
                g.iter().map(|x| a + (b - a) * (x - ga) / (gb - ga).max(1e-9)).collect()
            }
        };
        let (pu, pv) = (params(&self.u, &nu), params(&self.v, &nv));
        // each direction solves B * cvs = samples, B from the new basis at its own sample parameters
        let matrix = |d: &Dir| -> Vec<Vec<f32>> {
            let (a, _) = d.domain();
            let g: Vec<f32> = if d.periodic { (0..d.n).map(|i| a + i as f32).collect() } else { d.greville() };
            g.iter()
                .map(|&t| {
                    let mut row = vec![0.0; d.n];
                    let (idx, w) = d.basis(t);
                    for (i, w) in idx.iter().zip(&w) {
                        row[*i] += w;
                    }
                    row
                })
                .collect()
        };
        let (mu, mv) = (matrix(&nu), matrix(&nv));
        // samples on the new grid, then interpolate along v and along u
        let mut mid = vec![Vec3::ZERO; nu.n * nv.n];
        for (i, &u) in pu.iter().enumerate() {
            let pts: Vec<Vec3> = pv.iter().map(|&v| self.eval_wrapped(u, v)).collect();
            let x = solve(mv.clone(), pts).unwrap_or_else(|| vec![Vec3::ZERO; nv.n]);
            for j in 0..nv.n {
                mid[i * nv.n + j] = x[j];
            }
        }
        let mut cvs = vec![Vec3::ZERO; nu.n * nv.n];
        for j in 0..nv.n {
            let col: Vec<Vec3> = (0..nu.n).map(|i| mid[i * nv.n + j]).collect();
            let x = solve(mu.clone(), col).unwrap_or_else(|| vec![Vec3::ZERO; nu.n]);
            for i in 0..nu.n {
                cvs[i * nv.n + j] = x[i];
            }
        }
        Surface { u: nu, v: nv, cvs, trims: Vec::new() }
    }

    /// A flat patch through a closed set of points, trimmed to them: Surfaces > Planar.
    pub fn planar_from(pts: &[Vec3]) -> Option<Surface> {
        if pts.len() < 3 {
            return None;
        }
        // Newell normal and a frame in the plane
        let mut n = Vec3::ZERO;
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
        if n.length_sq() < 1e-12 {
            return None;
        }
        let n = n.normalized();
        let helper = if n.y.abs() < 0.9 { Vec3::UP } else { Vec3::new(1.0, 0.0, 0.0) };
        let ex = helper.cross(n).normalized();
        let ey = n.cross(ex);
        let c = pts.iter().fold(Vec3::ZERO, |a, b| a + *b) * (1.0 / pts.len() as f32);
        let local: Vec<(f32, f32)> = pts.iter().map(|p| ((*p - c).dot(ex), (*p - c).dot(ey))).collect();
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for p in &local {
            lo = (lo.0.min(p.0), lo.1.min(p.1));
            hi = (hi.0.max(p.0), hi.1.max(p.1));
        }
        let (w, h) = ((hi.0 - lo.0).max(1e-4) * 1.02, (hi.1 - lo.1).max(1e-4) * 1.02);
        let mid = ((lo.0 + hi.0) * 0.5, (lo.1 + hi.1) * 0.5);
        let mut cvs = Vec::with_capacity(16);
        for i in 0..4 {
            for j in 0..4 {
                let (a, b) = (mid.0 + (i as f32 / 3.0 - 0.5) * w, mid.1 + (j as f32 / 3.0 - 0.5) * h);
                cvs.push(c + ex * a + ey * b);
            }
        }
        let mut s = Surface { u: Dir::open(4, 3), v: Dir::open(4, 3), cvs, trims: Vec::new() };
        // u runs along ex, v along ey, both over 0..1
        let loop_uv: Vec<(f32, f32)> = local.iter().map(|p| ((p.0 - (mid.0 - w * 0.5)) / w, (p.1 - (mid.1 - h * 0.5)) / h)).collect();
        s.trims.push(Trim { pts: loop_uv, keep: true });
        Some(s)
    }

    /// Isoparm lines for display: every span boundary in both directions, as polylines.
    pub fn isoparm_lines(&self, per_span: usize) -> Vec<Vec<Vec3>> {
        let mut lines = Vec::new();
        for &u in &self.u.span_params() {
            lines.push(self.isoparm(false, u).sample(per_span));
        }
        // a periodic direction repeats its first line at the end
        for &v in &self.v.span_params() {
            lines.push(self.isoparm(true, v).sample(per_span));
        }
        lines
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut lo = self.cvs[0];
        let mut hi = self.cvs[0];
        for p in &self.cvs {
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        (lo, hi)
    }
}

/// Revolve a profile curve about the axis through `origin` along `axis`. A full turn gives a periodic surface
/// with `sections` rings; a partial turn an open one.
pub fn revolve(profile: &Curve, origin: Vec3, axis: Vec3, degrees: f32, sections: usize, degree: usize) -> Surface {
    let axis = axis.normalized();
    let full = degrees >= 359.99;
    let sections = sections.max(3);
    let nv = if full { sections } else { sections + 1 };
    // a periodic spline pulls its ring inside the control polygon; push the control ring out to compensate
    let scale = if full { 3.0 / (2.0 + (std::f32::consts::TAU / nv as f32).cos()) } else { 1.0 };
    let mut cvs = Vec::with_capacity(profile.cvs.len() * nv);
    for p in &profile.cvs {
        let rel = *p - origin;
        let along = axis * rel.dot(axis);
        let radial = rel - along;
        let perp = axis.cross(radial);
        for k in 0..nv {
            let a = if full { std::f32::consts::TAU * k as f32 / nv as f32 } else { degrees.to_radians() * k as f32 / sections as f32 };
            cvs.push(origin + along + (radial * a.cos() + perp * a.sin()) * scale);
        }
    }
    let v = if full { Dir::periodic(nv, degree) } else { Dir::open(nv, degree) };
    Surface { u: profile.dir.clone(), v, cvs, trims: Vec::new() }
}

/// Loft through curves (in order): the surface passes through each of them.
pub fn loft(curves: &[Curve]) -> Result<Surface, String> {
    if curves.len() < 2 {
        return Err("Loft needs at least two curves".to_string());
    }
    // bring every curve to the structure of the richest one
    let n = curves.iter().map(|c| c.cvs.len()).max().unwrap();
    let degree = curves.iter().map(|c| c.dir.degree).max().unwrap();
    let periodic = curves[0].dir.periodic;
    let curves: Vec<Curve> = curves
        .iter()
        .map(|c| if c.cvs.len() == n && c.dir.degree == degree && c.dir.periodic == periodic { c.clone() } else { rebuild_curve(c, n, degree) })
        .collect();
    let dv = curves[0].dir.clone();
    let du_deg = 3.min(curves.len() - 1);
    let m = curves.len();
    let du = Dir::open(m, du_deg);
    let g = du.greville();
    let a: Vec<Vec<f32>> = g
        .iter()
        .map(|&t| {
            let mut row = vec![0.0; m];
            let (idx, w) = du.basis(t);
            for (i, w) in idx.iter().zip(&w) {
                row[*i] += w;
            }
            row
        })
        .collect();
    let mut cvs = vec![Vec3::ZERO; m * n];
    for j in 0..n {
        let col: Vec<Vec3> = curves.iter().map(|c| c.cvs[j]).collect();
        let x = solve(a.clone(), col).ok_or("The curves cannot be lofted (they coincide)")?;
        for i in 0..m {
            cvs[i * n + j] = x[i];
        }
    }
    Ok(Surface { u: du, v: dv, cvs, trims: Vec::new() })
}

/// Sweep a profile along a path, keeping its orientation relative to the path (parallel transport).
/// The sweep starts at the path end closest to the profile.
pub fn extrude_along(profile: &Curve, path: &Curve) -> Surface {
    let mut path = path.clone();
    let center = profile.cvs.iter().fold(Vec3::ZERO, |a, b| a + *b) * (1.0 / profile.cvs.len() as f32);
    if path.end().distance(center) < path.start().distance(center) {
        path.reverse();
    }
    let rows = (path.dir.spans() * 3 + 1).clamp(5, 48);
    let (a, b) = path.dir.domain();
    let ts: Vec<f32> = (0..rows).map(|i| a + (b - a) * i as f32 / (rows - 1) as f32).collect();
    let start = path.point(ts[0]);
    let mut prev_t = path.tangent(ts[0]);
    // the frame carried along the path: rotation about the axis prev_t x t by the angle between them
    let mut frames: Vec<[Vec3; 3]> = vec![[Vec3::new(1.0, 0.0, 0.0), Vec3::UP, Vec3::new(0.0, 0.0, 1.0)]];
    for &t in ts.iter().skip(1) {
        let cur = path.tangent(t);
        let last = *frames.last().unwrap();
        let axis = prev_t.cross(cur);
        let s = axis.length();
        let c = prev_t.dot(cur).clamp(-1.0, 1.0);
        let rot = |v: Vec3| -> Vec3 {
            if s < 1e-6 {
                return v;
            }
            let k = axis * (1.0 / s);
            // Rodrigues' formula with sin = s, cos = c
            v * c + k.cross(v) * s + k * (k.dot(v) * (1.0 - c))
        };
        frames.push([rot(last[0]), rot(last[1]), rot(last[2])]);
        prev_t = cur;
    }
    let n = profile.cvs.len();
    let mut rows_pts: Vec<Vec<Vec3>> = Vec::new();
    for (i, &t) in ts.iter().enumerate() {
        let f = frames[i];
        let at = path.point(t);
        rows_pts.push(profile.cvs.iter().map(|p| {
            let r = *p - center;
            at + f[0] * r.x + f[1] * r.y + f[2] * r.z + (center - start)
        }).collect());
    }
    // interpolate each profile CV along the sweep so the surface passes through the placed profiles
    let du = Dir::open(rows, 3);
    let g = du.greville();
    let _ = g;
    let mut cvs = vec![Vec3::ZERO; rows * n];
    for j in 0..n {
        let col: Vec<Vec3> = rows_pts.iter().map(|r| r[j]).collect();
        let c = interpolate(&col, 3);
        for i in 0..rows {
            cvs[i * n + j] = c.cvs[i];
        }
    }
    Surface { u: du, v: profile.dir.clone(), cvs, trims: Vec::new() }
}

// ----------------------------------------------------------------------------------------------- primitives

/// Straight profile with `spans` spans from `a` to `b`.
fn line_profile(a: Vec3, b: Vec3, spans: usize) -> Curve {
    let n = spans.max(1) + 3;
    // evenly spaced control points on a line give an evenly parameterized straight curve
    let pts: Vec<Vec3> = (0..n).map(|i| a.lerp(b, i as f32 / (n - 1) as f32)).collect();
    Curve::open(pts, 3)
}

/// Create > NURBS Primitives > Sphere (poles on the Y axis).
pub fn sphere(radius: f32, sections: usize, spans: usize) -> Surface {
    let n = spans.max(1) + 3;
    let pts: Vec<Vec3> = (0..n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / (n - 1) as f32;
            Vec3::new(a.cos() * radius, a.sin() * radius, 0.0)
        })
        .collect();
    let prof = interpolate(&pts, 3);
    revolve(&prof, Vec3::ZERO, Vec3::UP, 360.0, sections, 3)
}

pub fn cylinder(radius: f32, height: f32, sections: usize, spans: usize) -> Surface {
    revolve(&line_profile(Vec3::new(radius, 0.0, 0.0), Vec3::new(radius, height, 0.0), spans), Vec3::ZERO, Vec3::UP, 360.0, sections, 3)
}

pub fn cone(radius: f32, height: f32, sections: usize, spans: usize) -> Surface {
    revolve(&line_profile(Vec3::new(radius, 0.0, 0.0), Vec3::new(0.0, height, 0.0), spans), Vec3::ZERO, Vec3::UP, 360.0, sections, 3)
}

pub fn torus(radius: f32, section_radius: f32, sections: usize, spans: usize) -> Surface {
    let mut ring = circle(section_radius, spans.max(3), 3);
    ring.transform(|p| Vec3::new(radius + p.x, -p.z, 0.0));
    revolve(&ring, Vec3::ZERO, Vec3::UP, 360.0, sections, 3)
}

/// A flat patch in the XZ plane.
pub fn plane(width: f32, length: f32, patches_u: usize, patches_v: usize) -> Surface {
    let (nu, nv) = (patches_u.max(1) + 3, patches_v.max(1) + 3);
    let mut cvs = Vec::with_capacity(nu * nv);
    for i in 0..nu {
        for j in 0..nv {
            cvs.push(Vec3::new((j as f32 / (nv - 1) as f32 - 0.5) * width, 0.0, (i as f32 / (nu - 1) as f32 - 0.5) * length));
        }
    }
    Surface { u: Dir::open(nu, 3), v: Dir::open(nv, 3), cvs, trims: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3, e: f32) -> bool {
        a.distance(b) < e
    }

    #[test]
    fn open_curve_starts_and_ends_on_its_end_cvs() {
        let c = Curve::open(vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 0.0), Vec3::new(3.0, 2.0, 0.0), Vec3::new(4.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 0.0)], 3);
        assert!(close(c.start(), c.cvs[0], 1e-5));
        assert!(close(c.end(), c.cvs[4], 1e-5));
        assert_eq!(c.dir.spans(), 2);
    }

    #[test]
    fn circle_has_the_requested_radius_at_its_knots_and_nearby() {
        let c = circle(2.0, 8, 3);
        for i in 0..8 {
            assert!((c.point(3.0 + i as f32).length() - 2.0).abs() < 1e-3);
        }
        // between the knots a cubic circle is within 1% of the radius
        for k in 0..64 {
            let r = c.point(3.0 + k as f32 / 8.0).length();
            assert!((r - 2.0).abs() < 0.03, "{}", r);
        }
    }

    #[test]
    fn knot_insertion_keeps_the_shape_and_split_pieces_meet() {
        let c = Curve::open((0..7).map(|i| Vec3::new(i as f32, ((i * 5) % 3) as f32, 0.0)).collect(), 3);
        let d = c.insert_knot(1.37).unwrap();
        assert_eq!(d.cvs.len(), 8);
        for k in 0..20 {
            let t = k as f32 / 19.0 * 4.0;
            assert!(close(c.point(t), d.point(t), 1e-4));
        }
        let (a, b) = c.split(1.5).unwrap();
        assert!(close(a.end(), b.start(), 1e-4));
        assert!(close(a.end(), c.point(1.5), 1e-4));
        assert!(close(a.point(0.7 * 1.5), c.point(0.7 * 1.5), 1e-4));
    }

    #[test]
    fn interpolation_passes_through_the_points() {
        let pts: Vec<Vec3> = (0..6).map(|i| Vec3::new(i as f32, (i as f32).sin(), 0.0)).collect();
        let c = interpolate(&pts, 3);
        let g = c.dir.greville();
        for (t, p) in g.iter().zip(&pts) {
            assert!(close(c.point(*t), *p, 1e-4));
        }
    }

    #[test]
    fn sphere_surface_lies_on_the_sphere_and_has_eight_sections() {
        let s = sphere(2.0, 8, 4);
        assert!(s.v.periodic && s.v.n == 8 && s.u.spans() == 4);
        let (ua, ub) = s.u.domain();
        let (va, vb) = s.v.domain();
        for i in 0..=8 {
            for j in 0..16 {
                let p = s.point(ua + (ub - ua) * i as f32 / 8.0, va + (vb - va) * j as f32 / 16.0);
                assert!((p.length() - 2.0).abs() < 0.12, "radius {}", p.length());
            }
        }
        let m = s.tessellate(4);
        assert!(m.faces.len() > 100);
    }

    #[test]
    fn surface_split_and_attach_round_trip() {
        let s = plane(4.0, 6.0, 4, 2);
        let (a, b) = s.split(true, 2.0).unwrap();
        assert!(close(a.point(a.u.domain().1, 0.5), b.point(b.u.domain().0, 0.5), 1e-4));
        let joined = a.attach(&b, true).unwrap();
        // both are planar: the join stays planar and covers the same extent
        let (lo, hi) = joined.bounds();
        assert!((hi.z - lo.z - 6.0).abs() < 1e-3 && lo.y.abs() < 1e-5 && hi.y.abs() < 1e-5);
        assert!(a.attach(&plane(4.0, 6.0, 4, 5), true).is_err());
    }

    #[test]
    fn loft_passes_through_its_curves_and_revolve_makes_a_ring() {
        let mut c1 = circle(1.0, 8, 3);
        let mut c2 = circle(0.5, 8, 3);
        let mut c3 = circle(1.0, 8, 3);
        c1.transform(|p| p + Vec3::new(0.0, 0.0, 0.0));
        c2.transform(|p| p + Vec3::new(0.0, 2.0, 0.0));
        c3.transform(|p| p + Vec3::new(0.0, 4.0, 0.0));
        let l = loft(&[c1.clone(), c2.clone(), c3.clone()]).unwrap();
        let g = l.u.greville();
        for (k, c) in [&c1, &c2, &c3].iter().enumerate() {
            let iso = l.isoparm(false, g[k]);
            for j in 0..8 {
                assert!(close(iso.cvs[j], c.cvs[j], 1e-3), "curve {} cv {}", k, j);
            }
        }
        assert!(loft(&[c1]).is_err());
    }

    #[test]
    fn extrude_follows_the_path() {
        let path = Curve::open(vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.0, 3.0, 0.0), Vec3::new(3.0, 3.0, 0.0), Vec3::new(4.0, 3.0, 0.0)], 3);
        let mut prof = circle(0.3, 8, 3);
        prof.transform(|p| p + path.start());
        let s = extrude_along(&prof, &path);
        let (ua, ub) = s.u.domain();
        // the sweep ends near the far end of the path
        let end: Vec3 = (0..s.v.n).map(|j| s.isoparm(false, ub).cvs[j]).fold(Vec3::ZERO, |a, b| a + b) * (1.0 / s.v.n as f32);
        assert!(close(end, path.end(), 0.05), "{:?}", (end.x, end.y, end.z));
        let _ = ua;
    }

    #[test]
    fn move_seam_shifts_the_ring_and_non_periodic_is_refused() {
        let s = cylinder(1.0, 2.0, 8, 2);
        let m = s.move_seam(false, 2).unwrap();
        assert!(close(m.cv(0, 0), s.cv(0, 2), 1e-6));
        assert!(plane(1.0, 1.0, 1, 1).move_seam(true, 1).is_err());
    }

    fn mesh_area(m: &Mesh) -> f32 {
        m.triangulate().iter().map(|t| (m.positions[t[1] as usize] - m.positions[t[0] as usize]).cross(m.positions[t[2] as usize] - m.positions[t[0] as usize]).length() * 0.5).sum()
    }

    #[test]
    fn trim_removes_the_region_and_keeps_a_smooth_boundary() {
        let mut s = plane(4.0, 4.0, 4, 4);
        let full = mesh_area(&s.tessellate(4));
        assert!((full - 16.0).abs() < 0.05, "{}", full);
        // a square hole on the isoparms u,v = 1..3: the plane maps parameter 1 and 3 to x = -2/3 and 2/3
        let sq = vec![(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)];
        s.trims.push(Trim { pts: sq.clone(), keep: false });
        let holed = mesh_area(&s.tessellate(4));
        let w = s.point(2.0, 3.0).x - s.point(2.0, 1.0).x;
        assert!((holed - (16.0 - w * w)).abs() < 0.02, "area {} width {}", holed, w);
        s.trims[0].keep = true;
        let kept = mesh_area(&s.tessellate(4));
        assert!((kept - w * w).abs() < 0.02, "{}", kept);
        // a round hole: kept and discarded halves add up to the whole and the boundary is smooth (many edges)
        let circle: Vec<(f32, f32)> = (0..48).map(|k| { let a = k as f32 / 48.0 * std::f32::consts::TAU; (2.0 + a.cos(), 2.0 + a.sin()) }).collect();
        s.trims[0] = Trim { pts: circle.clone(), keep: true };
        let inside = mesh_area(&s.tessellate(4));
        s.trims[0].keep = false;
        let outside = mesh_area(&s.tessellate(4));
        assert!((inside + outside - 16.0).abs() < 0.05, "{} + {}", inside, outside);
        assert!(inside > 1.0 && inside < 3.0);
        assert!(!s.accepts(2.0, 2.0) && s.accepts(0.2, 0.2));
    }

    #[test]
    fn projection_finds_the_surface_point_below() {
        let s = plane(4.0, 4.0, 4, 4);
        let (u, v, d) = s.project_point(Vec3::new(0.5, 3.0, -1.0), Vec3::new(0.0, -1.0, 0.0)).unwrap();
        let p = s.point(u, v);
        assert!((p.x - 0.5).abs() < 0.01 && (p.z + 1.0).abs() < 0.01 && d < 0.01, "{:?}", (p.x, p.y, p.z));
        assert!(s.project_point(Vec3::new(30.0, 3.0, 0.0), Vec3::new(0.0, -1.0, 0.0)).is_none(), "a miss");
    }

    #[test]
    fn rebuild_keeps_the_shape_with_fewer_spans_and_planar_trims_to_the_outline() {
        let s = sphere(2.0, 8, 8);
        let r = s.rebuild(4, 8, 3, 3);
        assert_eq!((r.u.spans(), r.v.n), (4, 8));
        let (ua, ub) = r.u.domain();
        let (va, vb) = r.v.domain();
        for i in 1..8 {
            for j in 0..8 {
                let p = r.point(ua + (ub - ua) * i as f32 / 8.0, va + (vb - va) * j as f32 / 8.0);
                assert!((p.length() - 2.0).abs() < 0.2, "{}", p.length());
            }
        }
        let tri = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 3.0)];
        let pl = Surface::planar_from(&tri).unwrap();
        let area = mesh_area(&pl.tessellate(4));
        assert!((area - 6.0).abs() < 0.3, "triangle area {}", area);
    }
}
