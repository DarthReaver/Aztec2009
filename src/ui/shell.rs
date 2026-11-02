//! The bottom aztecsh command line: input editing, history, scrollback and tab completion.

use super::screen::*;
use super::term::{Key, Mods};
use crate::mel::commands::HELP;
use crate::mel::{execute_line, Outcome};
use crate::session::Session;

pub struct ShellState {
    pub input: Vec<char>,
    pub cursor: usize,
    pub history: Vec<String>,
    hist_pos: Option<usize>,
    pub log: Vec<(String, bool)>,
    /// Lines scrolled up from the bottom of the log.
    pub scroll: usize,
}

impl ShellState {
    pub fn new() -> ShellState {
        let mut s = ShellState {
            input: Vec::new(),
            cursor: 0,
            history: Vec::new(),
            hist_pos: None,
            log: Vec::new(),
            scroll: 0,
        };
        s.push("Aztec 2009 ready. Type help for commands, F1 for key bindings.", false);
        s
    }

    pub fn push(&mut self, line: &str, err: bool) {
        for l in line.split('\n') {
            self.log.push((l.to_string(), err));
        }
        if self.log.len() > 2000 {
            let drop = self.log.len() - 2000;
            self.log.drain(..drop);
        }
        self.scroll = 0;
    }

    pub fn push_outcome(&mut self, o: Outcome) {
        for l in o.lines {
            self.push(&l, false);
        }
        if let Some(e) = o.error {
            self.push(&format!("// Error: {}", e), true);
        }
    }

    /// Run a command string, echoing it into the log.
    pub fn run(&mut self, session: &mut Session, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        self.push(&format!("aztecsh# {}", line), false);
        let o = execute_line(session, line);
        self.push_outcome(o);
    }

    fn complete(&mut self) {
        let start = self.input.iter().rposition(|c| c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
        if self.input[..start].iter().any(|c| !c.is_whitespace()) {
            return; // only complete the command word
        }
        let word: String = self.input[start..self.cursor.min(self.input.len())].iter().collect();
        if word.is_empty() {
            return;
        }
        let lw = word.to_ascii_lowercase();
        let matches: Vec<&str> = HELP.iter().map(|(n, _)| *n).filter(|n| n.to_ascii_lowercase().starts_with(&lw)).collect();
        match matches.len() {
            0 => {}
            1 => {
                let m: Vec<char> = format!("{} ", matches[0]).chars().collect();
                self.input.splice(start..self.cursor, m.iter().copied());
                self.cursor = start + m.len();
            }
            _ => {
                // extend to the longest common prefix, then list candidates
                let first: Vec<char> = matches[0].chars().collect();
                let mut common = first.len();
                for m in &matches[1..] {
                    let mc: Vec<char> = m.chars().collect();
                    let mut k = 0;
                    while k < common && k < mc.len() && first[k].eq_ignore_ascii_case(&mc[k]) {
                        k += 1;
                    }
                    common = k;
                }
                if common > word.chars().count() {
                    let p = &first[..common];
                    self.input.splice(start..self.cursor, p.iter().copied());
                    self.cursor = start + common;
                }
                let list = matches.join("  ");
                self.push(&list, false);
            }
        }
    }

    /// Returns a submitted line when Enter is pressed. Returns None for all other keys.
    pub fn handle_key(&mut self, key: Key, mods: Mods) -> Option<String> {
        match key {
            Key::Enter => {
                let line: String = self.input.iter().collect();
                self.input.clear();
                self.cursor = 0;
                self.hist_pos = None;
                if !line.trim().is_empty() && self.history.last().map(|h| h != &line).unwrap_or(true) {
                    self.history.push(line.clone());
                }
                return Some(line);
            }
            Key::Char(c) if !mods.ctrl && !mods.alt => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.input.remove(self.cursor);
                }
            }
            Key::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                }
            }
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(self.input.len()),
            Key::Home | Key::Ctrl('a') => self.cursor = 0,
            Key::End | Key::Ctrl('e') => self.cursor = self.input.len(),
            Key::Ctrl('u') => {
                self.input.drain(..self.cursor);
                self.cursor = 0;
            }
            Key::Ctrl('k') => {
                self.input.truncate(self.cursor);
            }
            Key::Ctrl('w') => {
                let mut i = self.cursor;
                while i > 0 && self.input[i - 1].is_whitespace() {
                    i -= 1;
                }
                while i > 0 && !self.input[i - 1].is_whitespace() {
                    i -= 1;
                }
                self.input.drain(i..self.cursor);
                self.cursor = i;
            }
            Key::Up => {
                if self.history.is_empty() {
                    return None;
                }
                let p = match self.hist_pos {
                    None => self.history.len() - 1,
                    Some(p) => p.saturating_sub(1),
                };
                self.hist_pos = Some(p);
                self.input = self.history[p].chars().collect();
                self.cursor = self.input.len();
            }
            Key::Down => {
                if let Some(p) = self.hist_pos {
                    if p + 1 < self.history.len() {
                        self.hist_pos = Some(p + 1);
                        self.input = self.history[p + 1].chars().collect();
                    } else {
                        self.hist_pos = None;
                        self.input.clear();
                    }
                    self.cursor = self.input.len();
                }
            }
            Key::PageUp => self.scroll = (self.scroll + 3).min(self.log.len().saturating_sub(1)),
            Key::PageDown => self.scroll = self.scroll.saturating_sub(3),
            Key::Tab => self.complete(),
            _ => {}
        }
        None
    }

    pub fn draw(&self, screen: &mut Screen, r: Rect, focus: bool) {
        if r.is_empty() {
            return;
        }
        screen.fill(r, ' ', TEXT, BLACK.mix(PANEL_DARK, 0.6));
        let bg = BLACK.mix(PANEL_DARK, 0.6);
        let title = if self.scroll > 0 { format!("aztecsh (scrolled {} lines, PgDn)", self.scroll) } else { "aztecsh".to_string() };
        screen.title_bar(Rect::new(r.x, r.y, r.w, 1), &title, focus);
        let rows = r.h.saturating_sub(2);
        let total = self.log.len();
        let end = total.saturating_sub(self.scroll);
        let start = end.saturating_sub(rows);
        for (k, (line, err)) in self.log[start..end].iter().enumerate() {
            let y = r.y + 1 + k;
            let fg = if *err {
                RED
            } else if line.starts_with("aztecsh#") {
                CYAN
            } else if line.starts_with("//") {
                YELLOW
            } else {
                TEXT
            };
            screen.text(r.x + 1, y, line, fg, bg, r.x + r.w);
        }
        let y = r.y + r.h - 1;
        let prompt = "aztecsh# ";
        screen.text_bold(r.x + 1, y, prompt, GREEN, bg, r.x + r.w);
        let avail = r.w.saturating_sub(prompt.len() + 2).max(1);
        let off = if self.cursor >= avail { self.cursor + 1 - avail } else { 0 };
        let shown: String = self.input.iter().skip(off).take(avail).collect();
        let x0 = r.x + 1 + prompt.len();
        screen.text(x0, y, &shown, TEXT, bg, r.x + r.w);
        if focus {
            screen_cursor(screen, x0 + self.cursor - off, y);
        }
    }
}

fn screen_cursor(screen: &mut Screen, x: usize, y: usize) {
    screen.cursor = Some((x, y));
}
