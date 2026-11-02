/// Scrubbable timeline: default range 1 to 120 at 24 fps.
#[derive(Clone, Debug)]
pub struct Timeline {
    pub start: f32,
    pub end: f32,
    pub current: f32,
    pub fps: f32,
    pub playing: bool,
    pub looping: bool,
    accum: f32,
}

impl Default for Timeline {
    fn default() -> Self {
        Timeline::new()
    }
}

impl Timeline {
    pub fn new() -> Timeline {
        Timeline { start: 1.0, end: 120.0, current: 1.0, fps: 24.0, playing: false, looping: true, accum: 0.0 }
    }

    pub fn set_range(&mut self, start: f32, end: f32) {
        let (s, e) = if start <= end { (start, end) } else { (end, start) };
        self.start = s;
        self.end = e;
        self.current = self.current.clamp(s, e);
    }

    pub fn set_time(&mut self, t: f32) {
        self.current = t.clamp(self.start, self.end);
    }

    pub fn play(&mut self) {
        self.playing = true;
        self.accum = 0.0;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn rewind(&mut self) {
        self.current = self.start;
        self.accum = 0.0;
    }

    pub fn step_forward(&mut self) {
        self.current = if self.current + 1.0 > self.end { self.start } else { self.current + 1.0 };
    }

    pub fn step_back(&mut self) {
        self.current = if self.current - 1.0 < self.start { self.end } else { self.current - 1.0 };
    }

    /// Advance playback by real elapsed seconds. Returns true if the frame changed.
    pub fn tick(&mut self, dt: f32) -> bool {
        if !self.playing {
            return false;
        }
        self.accum += dt;
        let frame_time = 1.0 / self.fps.max(1.0);
        let mut changed = false;
        while self.accum >= frame_time {
            self.accum -= frame_time;
            if self.current + 1.0 > self.end {
                if self.looping {
                    self.current = self.start;
                } else {
                    self.playing = false;
                    self.accum = 0.0;
                    break;
                }
            } else {
                self.current += 1.0;
            }
            changed = true;
        }
        changed
    }

    pub fn frame(&self) -> i32 {
        self.current.round() as i32
    }
}
