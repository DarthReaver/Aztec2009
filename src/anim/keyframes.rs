#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Linear,
    Bezier,
    Step,
}

impl Interp {
    pub fn name(self) -> &'static str {
        match self {
            Interp::Linear => "linear",
            Interp::Bezier => "bezier",
            Interp::Step => "step",
        }
    }
    pub fn parse(s: &str) -> Option<Interp> {
        match s.to_ascii_lowercase().as_str() {
            "linear" => Some(Interp::Linear),
            "bezier" | "spline" | "auto" | "smooth" => Some(Interp::Bezier),
            "step" | "stepped" => Some(Interp::Step),
            _ => None,
        }
    }
}

/// Tangent type of one side of a key (Graph Editor > Tangents).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tan {
    /// Maya's default: smooth, but flat at extremes so the curve never overshoots.
    Clamped,
    /// Smooth through the neighbours, overshoots allowed.
    Spline,
    Linear,
    Flat,
    /// Out tangent only: the value holds until the next key.
    Step,
    /// Like clamped, and also limits the slope so a key between close values stays gentle.
    Plateau,
    /// Keeps the angle the handle was given.
    Fixed,
}

impl Tan {
    pub fn name(self) -> &'static str {
        match self {
            Tan::Clamped => "clamped",
            Tan::Spline => "spline",
            Tan::Linear => "linear",
            Tan::Flat => "flat",
            Tan::Step => "step",
            Tan::Plateau => "plateau",
            Tan::Fixed => "fixed",
        }
    }
    pub fn parse(s: &str) -> Option<Tan> {
        match s.to_ascii_lowercase().as_str() {
            "clamped" | "auto" | "bezier" => Some(Tan::Clamped),
            "spline" | "smooth" => Some(Tan::Spline),
            "linear" => Some(Tan::Linear),
            "flat" => Some(Tan::Flat),
            "step" | "stepped" => Some(Tan::Step),
            "plateau" => Some(Tan::Plateau),
            "fixed" => Some(Tan::Fixed),
            _ => None,
        }
    }
    /// Segment interpolation the out tangent type implies.
    pub fn interp(self) -> Interp {
        match self {
            Tan::Linear => Interp::Linear,
            Tan::Step => Interp::Step,
            _ => Interp::Bezier,
        }
    }
}

impl Interp {
    pub fn tan(self) -> Tan {
        match self {
            Interp::Linear => Tan::Linear,
            Interp::Step => Tan::Step,
            Interp::Bezier => Tan::Clamped,
        }
    }
}

/// What a curve does before its first and after its last key (Graph Editor > Curves > Pre / Post Infinity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Infinity {
    #[default]
    Constant,
    Linear,
    Cycle,
    CycleOffset,
    Oscillate,
}

impl Infinity {
    pub fn name(self) -> &'static str {
        match self {
            Infinity::Constant => "constant",
            Infinity::Linear => "linear",
            Infinity::Cycle => "cycle",
            Infinity::CycleOffset => "cycleWithOffset",
            Infinity::Oscillate => "oscillate",
        }
    }
    pub fn parse(s: &str) -> Option<Infinity> {
        match s.to_ascii_lowercase().as_str() {
            "constant" => Some(Infinity::Constant),
            "linear" => Some(Infinity::Linear),
            "cycle" => Some(Infinity::Cycle),
            "cyclewithoffset" | "cycleoffset" | "offset" => Some(Infinity::CycleOffset),
            "oscillate" => Some(Infinity::Oscillate),
            _ => None,
        }
    }
}

/// A keyframe with auto computed bezier handles (relative offsets from the key).
#[derive(Clone, Debug)]
pub struct Key {
    pub t: f32,
    pub v: f32,
    /// Interpolation used on the segment leaving this key.
    pub interp: Interp,
    pub in_dt: f32,
    pub in_dv: f32,
    pub out_dt: f32,
    pub out_dv: f32,
    pub in_tan: Tan,
    pub out_tan: Tan,
}

#[derive(Clone, Debug, Default)]
pub struct Curve {
    pub keys: Vec<Key>,
    pub pre: Infinity,
    pub post: Infinity,
}

/// Evaluate a cubic bezier segment (control points as (time, value)) at time `t`.
/// Solves x(s) = t by bisection since the x control points are kept monotonic.
pub fn bezier_eval(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), t: f32) -> f32 {
    let cubic = |a: f32, b: f32, c: f32, d: f32, s: f32| -> f32 {
        let u = 1.0 - s;
        u * u * u * a + 3.0 * u * u * s * b + 3.0 * u * s * s * c + s * s * s * d
    };
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        let x = cubic(p0.0, p1.0, p2.0, p3.0, mid);
        if x < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let s = 0.5 * (lo + hi);
    cubic(p0.1, p1.1, p2.1, p3.1, s)
}

impl Curve {
    pub fn new() -> Curve {
        Curve { keys: Vec::new(), pre: Infinity::Constant, post: Infinity::Constant }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Insert a key, or replace the value at an existing key time.
    pub fn set_key(&mut self, t: f32, v: f32, interp: Interp) {
        self.set_key_with(t, v, interp, None);
    }

    /// Like `set_key`; a new key of bezier interpolation takes the `defaults` (in, out) tangent types.
    pub fn set_key_with(&mut self, t: f32, v: f32, interp: Interp, defaults: Option<(Tan, Tan)>) {
        if let Some(k) = self.keys.iter_mut().find(|k| (k.t - t).abs() < 1e-4) {
            k.v = v;
            if k.interp != interp {
                k.interp = interp;
                k.out_tan = interp.tan();
                if interp == Interp::Linear {
                    k.in_tan = Tan::Linear;
                }
            }
        } else {
            let (in_tan, out_tan) = match (interp, defaults) {
                (Interp::Bezier, Some(d)) => d,
                _ => (if interp == Interp::Linear { Tan::Linear } else { Tan::Clamped }, interp.tan()),
            };
            self.keys.push(Key { t, v, interp: out_tan.interp(), in_dt: 0.0, in_dv: 0.0, out_dt: 0.0, out_dv: 0.0, in_tan, out_tan });
            self.keys.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        }
        self.recompute_tangents();
    }

    /// Set the tangent types of the key at time `t` (either side may be left alone).
    pub fn set_tangents_at(&mut self, t: f32, in_tan: Option<Tan>, out_tan: Option<Tan>) -> bool {
        let mut hit = false;
        for k in self.keys.iter_mut().filter(|k| (k.t - t).abs() < 1e-3) {
            if let Some(i) = in_tan {
                k.in_tan = i;
            }
            if let Some(o) = out_tan {
                k.out_tan = o;
                k.interp = o.interp();
            }
            hit = true;
        }
        if hit {
            self.recompute_tangents();
        }
        hit
    }

    pub fn remove_key(&mut self, t: f32) -> bool {
        let before = self.keys.len();
        self.keys.retain(|k| (k.t - t).abs() >= 1e-4);
        self.recompute_tangents();
        before != self.keys.len()
    }

    pub fn has_key_at(&self, t: f32) -> bool {
        self.keys.iter().any(|k| (k.t - t).abs() < 1e-4)
    }

    /// Compute the handles from the tangent types of every key.
    pub fn recompute_tangents(&mut self) {
        let n = self.keys.len();
        for i in 0..n {
            let prev = if i > 0 { Some((self.keys[i - 1].t, self.keys[i - 1].v)) } else { None };
            let next = if i + 1 < n { Some((self.keys[i + 1].t, self.keys[i + 1].v)) } else { None };
            let (ct, cv) = (self.keys[i].t, self.keys[i].v);
            let slope_to = |p: (f32, f32), q: (f32, f32)| if (q.0 - p.0).abs() > 1e-9 { (q.1 - p.1) / (q.0 - p.0) } else { 0.0 };
            // slope a tangent of type `tan` takes; `outgoing` picks the side it belongs to
            let slope_of = |tan: Tan, outgoing: bool, fixed: f32| -> f32 {
                match tan {
                    Tan::Flat | Tan::Step => 0.0,
                    Tan::Fixed => fixed,
                    Tan::Linear => match (outgoing, prev, next) {
                        (true, _, Some(nx)) => slope_to((ct, cv), nx),
                        (false, Some(p), _) => slope_to(p, (ct, cv)),
                        _ => 0.0,
                    },
                    Tan::Spline => match (prev, next) {
                        (Some(p), Some(nx)) => slope_to(p, nx),
                        (None, Some(nx)) => slope_to((ct, cv), nx),
                        (Some(p), None) => slope_to(p, (ct, cv)),
                        _ => 0.0,
                    },
                    Tan::Clamped | Tan::Plateau => match (prev, next) {
                        (Some(p), Some(nx)) => {
                            if (cv - p.1) * (nx.1 - cv) <= 0.0 {
                                0.0
                            } else {
                                let s = slope_to(p, nx);
                                if tan == Tan::Plateau {
                                    // never steeper than twice the gentler neighbouring slope
                                    let lim = 2.0 * slope_to(p, (ct, cv)).abs().min(slope_to((ct, cv), nx).abs());
                                    s.signum() * s.abs().min(lim)
                                } else {
                                    s
                                }
                            }
                        }
                        _ => 0.0,
                    },
                }
            };
            let k = &self.keys[i];
            let fixed_in = if k.in_dt > 1e-9 { k.in_dv / k.in_dt } else { 0.0 };
            let fixed_out = if k.out_dt > 1e-9 { k.out_dv / k.out_dt } else { 0.0 };
            let (s_in, s_out) = (slope_of(k.in_tan, false, fixed_in), slope_of(k.out_tan, true, fixed_out));
            let in_dt = prev.map(|p| (ct - p.0) / 3.0).unwrap_or(0.0);
            let out_dt = next.map(|nx| (nx.0 - ct) / 3.0).unwrap_or(0.0);
            let k = &mut self.keys[i];
            k.in_dt = in_dt;
            k.in_dv = s_in * in_dt;
            k.out_dt = out_dt;
            k.out_dv = s_out * out_dt;
        }
    }

    /// Value outside the key range according to the infinity setting.
    fn extrapolate(&self, t: f32) -> f32 {
        let n = self.keys.len();
        let (first, last) = (&self.keys[0], &self.keys[n - 1]);
        let before = t < first.t;
        let mode = if before { self.pre } else { self.post };
        let period = last.t - first.t;
        let (edge, other) = if before { (first, last) } else { (last, first) };
        if n < 2 || period < 1e-6 {
            return edge.v;
        }
        match mode {
            Infinity::Constant => edge.v,
            Infinity::Linear => {
                let slope = if before { if first.out_dt > 1e-9 { first.out_dv / first.out_dt } else { 0.0 } } else if last.in_dt > 1e-9 { last.in_dv / last.in_dt } else { 0.0 };
                edge.v + slope * (t - edge.t)
            }
            Infinity::Cycle => {
                let tt = first.t + (t - first.t).rem_euclid(period);
                self.eval_inside(tt)
            }
            Infinity::CycleOffset => {
                let cycles = ((t - first.t) / period).floor();
                let tt = t - cycles * period;
                self.eval_inside(tt) + cycles * (last.v - first.v)
            }
            Infinity::Oscillate => {
                let phase = (t - first.t).rem_euclid(2.0 * period);
                let tt = if phase <= period { first.t + phase } else { first.t + 2.0 * period - phase };
                let _ = other;
                self.eval_inside(tt)
            }
        }
    }

    pub fn eval(&self, t: f32) -> Option<f32> {
        let n = self.keys.len();
        if n == 0 {
            return None;
        }
        if t < self.keys[0].t || t > self.keys[n - 1].t {
            return Some(self.extrapolate(t));
        }
        Some(self.eval_inside(t))
    }

    /// Value for `t` within the first and last key.
    fn eval_inside(&self, t: f32) -> f32 {
        let n = self.keys.len();
        if t <= self.keys[0].t {
            return self.keys[0].v;
        }
        if t >= self.keys[n - 1].t {
            return self.keys[n - 1].v;
        }
        let mut i = 0;
        while i + 1 < n && self.keys[i + 1].t <= t {
            i += 1;
        }
        let a = &self.keys[i];
        let b = &self.keys[i + 1];
        let span = b.t - a.t;
        if span.abs() < 1e-9 {
            return b.v;
        }
        match a.interp {
            Interp::Step => a.v,
            Interp::Linear => a.v + (b.v - a.v) * ((t - a.t) / span),
            Interp::Bezier => {
                let p0 = (a.t, a.v);
                let p1 = (a.t + a.out_dt.clamp(0.0, span), a.v + a.out_dv);
                let p2 = (b.t - b.in_dt.clamp(0.0, span), b.v - b.in_dv);
                let p3 = (b.t, b.v);
                bezier_eval(p0, p1, p2, p3, t)
            }
        }
    }
}
