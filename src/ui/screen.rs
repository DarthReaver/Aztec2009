//! Cell buffer with diffed output: only changed cells are written each frame.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn scale(self, f: f32) -> Rgb {
        let s = |v: u8| (v as f32 * f).clamp(0.0, 255.0) as u8;
        Rgb(s(self.0), s(self.1), s(self.2))
    }
    pub fn mix(self, o: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).clamp(0.0, 255.0) as u8;
        Rgb(m(self.0, o.0), m(self.1, o.1), m(self.2, o.2))
    }
}

// Maya 2009 style grey UI palette.
pub const PANEL_BG: Rgb = Rgb(64, 64, 66);
pub const PANEL_DARK: Rgb = Rgb(44, 44, 46);
pub const TITLE_BG: Rgb = Rgb(92, 92, 96);
pub const TEXT: Rgb = Rgb(218, 218, 218);
pub const TEXT_DIM: Rgb = Rgb(150, 150, 156);
pub const SEL_BG: Rgb = Rgb(86, 128, 176);
pub const KEY_BG: Rgb = Rgb(190, 120, 40);
pub const CONN_BG: Rgb = Rgb(120, 84, 160);
pub const RED: Rgb = Rgb(235, 90, 80);
pub const GREEN: Rgb = Rgb(120, 220, 130);
pub const CYAN: Rgb = Rgb(110, 205, 230);
pub const YELLOW: Rgb = Rgb(240, 210, 100);
pub const BLACK: Rgb = Rgb(0, 0, 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, w: usize, h: usize) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: usize, py: usize) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
}

impl Cell {
    const BLANK: Cell = Cell { ch: ' ', fg: TEXT, bg: BLACK, bold: false };
}

pub struct Screen {
    pub w: usize,
    pub h: usize,
    cells: Vec<Cell>,
    prev: Vec<Cell>,
    force: bool,
    truecolor: bool,
    pub cursor: Option<(usize, usize)>,
}

impl Screen {
    pub fn new(w: usize, h: usize, truecolor: bool) -> Screen {
        Screen {
            w,
            h,
            cells: vec![Cell::BLANK; w * h],
            prev: vec![Cell::BLANK; w * h],
            force: true,
            truecolor,
            cursor: None,
        }
    }

    pub fn resize(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.cells = vec![Cell::BLANK; w * h];
        self.prev = vec![Cell::BLANK; w * h];
        self.force = true;
    }

    pub fn clear(&mut self, bg: Rgb) {
        for c in self.cells.iter_mut() {
            *c = Cell { ch: ' ', fg: TEXT, bg, bold: false };
        }
        self.cursor = None;
    }

    pub fn get(&self, x: usize, y: usize) -> Option<Cell> {
        if x < self.w && y < self.h {
            Some(self.cells[y * self.w + x])
        } else {
            None
        }
    }

    pub fn set(&mut self, x: usize, y: usize, cell: Cell) {
        if x < self.w && y < self.h {
            self.cells[y * self.w + x] = cell;
        }
    }

    pub fn fill(&mut self, r: Rect, ch: char, fg: Rgb, bg: Rgb) {
        for y in r.y..(r.y + r.h).min(self.h) {
            for x in r.x..(r.x + r.w).min(self.w) {
                self.cells[y * self.w + x] = Cell { ch, fg, bg, bold: false };
            }
        }
    }

    /// Write text starting at (x, y), clipped to `max_x` (exclusive). Returns the x after the last glyph.
    pub fn text(&mut self, x: usize, y: usize, s: &str, fg: Rgb, bg: Rgb, max_x: usize) -> usize {
        let mut cx = x;
        for ch in s.chars() {
            if cx >= max_x || cx >= self.w {
                break;
            }
            let ch = if ch.is_control() { ' ' } else { ch };
            self.set(cx, y, Cell { ch, fg, bg, bold: false });
            cx += 1;
        }
        cx
    }

    pub fn text_bold(&mut self, x: usize, y: usize, s: &str, fg: Rgb, bg: Rgb, max_x: usize) -> usize {
        let mut cx = x;
        for ch in s.chars() {
            if cx >= max_x || cx >= self.w {
                break;
            }
            self.set(cx, y, Cell { ch, fg, bg, bold: true });
            cx += 1;
        }
        cx
    }

    /// Title bar occupying the first row of `r`.
    pub fn title_bar(&mut self, r: Rect, title: &str, focused: bool) {
        let bg = if focused { SEL_BG.scale(0.9) } else { TITLE_BG };
        self.fill(Rect::new(r.x, r.y, r.w, 1), ' ', TEXT, bg);
        self.text_bold(r.x + 1, r.y, title, TEXT, bg, r.x + r.w);
    }

    /// Single line box with an optional title, drawn inside `r`.
    pub fn draw_box(&mut self, r: Rect, fg: Rgb, bg: Rgb, title: Option<&str>) {
        if r.w < 2 || r.h < 2 {
            return;
        }
        self.fill(r, ' ', fg, bg);
        for x in r.x + 1..r.x + r.w - 1 {
            self.set(x, r.y, Cell { ch: '\u{2500}', fg, bg, bold: false });
            self.set(x, r.y + r.h - 1, Cell { ch: '\u{2500}', fg, bg, bold: false });
        }
        for y in r.y + 1..r.y + r.h - 1 {
            self.set(r.x, y, Cell { ch: '\u{2502}', fg, bg, bold: false });
            self.set(r.x + r.w - 1, y, Cell { ch: '\u{2502}', fg, bg, bold: false });
        }
        self.set(r.x, r.y, Cell { ch: '\u{250c}', fg, bg, bold: false });
        self.set(r.x + r.w - 1, r.y, Cell { ch: '\u{2510}', fg, bg, bold: false });
        self.set(r.x, r.y + r.h - 1, Cell { ch: '\u{2514}', fg, bg, bold: false });
        self.set(r.x + r.w - 1, r.y + r.h - 1, Cell { ch: '\u{2518}', fg, bg, bold: false });
        if let Some(t) = title {
            self.text(r.x + 2, r.y, &format!(" {} ", t), TEXT, bg, r.x + r.w - 1);
        }
    }

    fn color_seq(&self, fg: Rgb, bg: Rgb, bold: bool, out: &mut String) {
        use std::fmt::Write;
        out.push_str(if bold { "\x1b[0;1;" } else { "\x1b[0;" });
        if self.truecolor {
            let _ = write!(out, "38;2;{};{};{};48;2;{};{};{}m", fg.0, fg.1, fg.2, bg.0, bg.1, bg.2);
        } else {
            let q = |c: Rgb| -> u32 {
                let t = |v: u8| ((v as f32 / 255.0 * 5.0).round() as u32).min(5);
                16 + 36 * t(c.0) + 6 * t(c.1) + t(c.2)
            };
            let _ = write!(out, "38;5;{};48;5;{}m", q(fg), q(bg));
        }
    }

    /// Produce the escape sequence that brings the real terminal up to date, then remember this frame.
    pub fn flush(&mut self) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(8192);
        let mut cur_pos: Option<(usize, usize)> = None;
        let mut cur_style: Option<(Rgb, Rgb, bool)> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                let c = self.cells[i];
                if !self.force && c == self.prev[i] {
                    continue;
                }
                if cur_pos != Some((x, y)) {
                    let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
                }
                if cur_style != Some((c.fg, c.bg, c.bold)) {
                    self.color_seq(c.fg, c.bg, c.bold, &mut out);
                    cur_style = Some((c.fg, c.bg, c.bold));
                }
                out.push(c.ch);
                cur_pos = Some((x + 1, y));
            }
        }
        out.push_str("\x1b[0m");
        match self.cursor {
            Some((cx, cy)) => {
                let _ = write!(out, "\x1b[{};{}H\x1b[?25h", cy + 1, cx + 1);
            }
            None => out.push_str("\x1b[?25l"),
        }
        self.prev.clone_from(&self.cells);
        self.force = false;
        out
    }
}
