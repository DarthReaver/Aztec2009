//! The interactive workspace: layout, focus, keyboard and mouse handling, and the main loop.

use super::channel::{self, ChanRow, ChannelState};
use super::menu::{self, Action, MenuState, Special, MENUS};
use super::outliner::{self, OutlinerState};
use super::screen::*;
use super::shell::ShellState;
use super::term::*;
use super::timeline::{self, TimelineState, Transport};
use super::viewport::DisplayMode;
use super::vp_render;
use crate::anim::Interp;
use crate::dag::NodeId;
use crate::session::Session;
use std::io::Write;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Focus {
    Shell,
    Viewport,
    Outliner,
    Channel,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    None,
    Orbit,
    Pan,
    Zoom,
    ViewClick { moved: bool, shift: bool },
    Scrub,
    Slider(Rect, usize),
}

#[derive(Default, Clone, Copy)]
struct Layout {
    outliner: Option<Rect>,
    viewport: Option<Rect>,
    channel: Option<Rect>,
    editor: Option<Rect>,
    timeline: Option<Rect>,
    shell: Option<Rect>,
}

fn compute_layout(w: usize, h: usize, expanded: bool) -> Layout {
    let shell_h = if expanded { (h * 3 / 5).clamp(6, h.saturating_sub(6)) } else { (h / 6).clamp(4, 7) };
    let tl_h = 3;
    let main_h = h.saturating_sub(1 + tl_h + shell_h);
    let out_w = if w >= 110 {
        24
    } else if w >= 80 {
        20
    } else {
        0
    };
    let right_w = if w >= 120 {
        34
    } else if w >= 90 {
        28
    } else {
        0
    };
    let vp_w = w - out_w - right_w;
    let mut l = Layout::default();
    if out_w > 0 {
        l.outliner = Some(Rect::new(0, 1, out_w, main_h));
    }
    l.viewport = Some(Rect::new(out_w, 1, vp_w, main_h));
    if right_w > 0 {
        let ch = (main_h * 6 / 10).max(3).min(main_h);
        l.channel = Some(Rect::new(out_w + vp_w, 1, right_w, ch));
        l.editor = Some(Rect::new(out_w + vp_w, 1 + ch, right_w, main_h - ch));
    }
    l.timeline = Some(Rect::new(0, 1 + main_h, w, tl_h));
    l.shell = Some(Rect::new(0, 1 + main_h + tl_h, w, shell_h));
    l
}

const HELP_LINES: &[&str] = &[
    "Aztec 2009 key bindings",
    "",
    "Global     Esc shell/viewport   Tab next panel   F1 this help   F2 big shell",
    "           F10 or Alt+F/E/C/D/M/R menus   Ctrl+Q quit   Space play/pause",
    "Viewport   arrows orbit   Shift+arrows pan   + / - zoom   1 2 3 shaded/wire/anaglyph",
    "           m cycle mode   g grid   f frame all   a select all   k set key",
    "           , . prev/next frame   Home rewind   End last frame   Del delete",
    "Mouse      Alt+Left orbit   Alt+Middle pan   Alt+Right zoom   wheel zoom",
    "           plain Left drag orbit, Middle drag pan, Right drag zoom, click select",
    "           Shift+click adds to selection, click timeline to scrub, click layer bar",
    "Outliner   Up/Down select   Del delete",
    "Channels   Up/Down row   Enter edit value   + / - nudge   k key channel",
    "Shell      Enter run   Up/Down history   Tab complete   PgUp/PgDn scroll",
    "",
    "Press any key to close.",
];

pub struct App {
    session: Session,
    screen: Screen,
    menu: MenuState,
    shell: ShellState,
    outliner: OutlinerState,
    chan: ChannelState,
    tl: TimelineState,
    rows: Vec<ChanRow>,
    focus: Focus,
    expanded: bool,
    help: bool,
    drag: Drag,
    last: (usize, usize),
    layout: Layout,
    status: String,
    dirty: bool,
}

fn rgb(v: crate::math::Vec3) -> Rgb {
    Rgb((v.x.clamp(0.0, 1.0) * 255.0) as u8, (v.y.clamp(0.0, 1.0) * 255.0) as u8, (v.z.clamp(0.0, 1.0) * 255.0) as u8)
}

impl App {
    fn new(session: Session, w: usize, h: usize, truecolor: bool) -> App {
        App {
            session,
            screen: Screen::new(w, h, truecolor),
            menu: MenuState::default(),
            shell: ShellState::new(),
            outliner: OutlinerState::default(),
            chan: ChannelState::default(),
            tl: TimelineState::default(),
            rows: Vec::new(),
            focus: Focus::Viewport,
            expanded: false,
            help: false,
            drag: Drag::None,
            last: (0, 0),
            layout: compute_layout(w, h, false),
            status: String::new(),
            dirty: true,
        }
    }

    // ---------------------------------------------------------------- drawing

    fn too_small(&self) -> bool {
        self.screen.w < 60 || self.screen.h < 16
    }

    fn draw(&mut self) {
        let (w, h) = (self.screen.w, self.screen.h);
        self.screen.clear(PANEL_DARK);
        if self.too_small() {
            let msg = format!("Terminal too small ({}x{}). Need at least 60x16.", w, h);
            self.screen.text(1, h / 2, &msg, YELLOW, PANEL_DARK, w);
            return;
        }
        self.layout = compute_layout(w, h, self.expanded);
        let l = self.layout;
        self.rows = channel::rows(&mut self.session.scene);
        self.chan.clamp(&self.rows);

        let status = self.status_line();
        self.menu.draw_bar(&mut self.screen, w, &status);

        if let Some(r) = l.outliner {
            outliner::draw(&mut self.screen, r, &self.session.scene, &mut self.outliner, self.focus == Focus::Outliner);
        }
        if let Some(r) = l.viewport {
            self.draw_viewport(r);
        }
        if let Some(r) = l.channel {
            channel::draw_channel(&mut self.screen, r, &self.session.scene, &self.rows, &mut self.chan, self.focus == Focus::Channel);
        }
        if let Some(r) = l.editor {
            channel::draw_editor(&mut self.screen, r, &self.session.scene, &mut self.chan);
        }
        if let Some(r) = l.timeline {
            timeline::draw(&mut self.screen, r, &self.session.scene, &mut self.tl);
        }
        if let Some(r) = l.shell {
            self.shell.draw(&mut self.screen, r, self.focus == Focus::Shell);
        }
        self.menu.draw_dropdown(&mut self.screen);
        if self.help {
            self.draw_help();
        }
        if self.focus != Focus::Shell || self.menu.is_open() || self.help {
            self.screen.cursor = None;
        }
    }

    fn status_line(&self) -> String {
        let s = &self.session.scene;
        let name = s.file_path.clone().unwrap_or_else(|| "untitled".to_string());
        let sel = match s.selection.first() {
            Some(id) => s.name_of(*id),
            None => "none".to_string(),
        };
        let extra = if self.status.is_empty() { String::new() } else { format!("  |  {}", self.status) };
        format!("{}  |  selected: {}{}", name, sel, extra)
    }

    fn draw_viewport(&mut self, r: Rect) {
        let focus = self.focus == Focus::Viewport;
        let cam = self.session.view.camera.clone().unwrap_or_else(|| "persp".to_string());
        let particles: usize = {
            use crate::dag::nodes::NodeData;
            let s = &self.session.scene;
            s.ids_where(|d| matches!(d, NodeData::Nucleus(_)))
                .iter()
                .filter_map(|id| match s.node(*id).map(|n| &n.data) {
                    Some(NodeData::Nucleus(nd)) => Some(nd.solver.particle_count()),
                    _ => None,
                })
                .sum()
        };
        let mut title = format!("Viewport: {} | {} | frame {}", cam, self.session.view.mode.name(), self.session.scene.timeline.frame());
        if particles > 0 {
            title.push_str(&format!(" | {} particles", particles));
        }
        self.screen.title_bar(Rect::new(r.x, r.y, r.w, 1), &title, focus);
        let rows = r.h.saturating_sub(1);
        if rows == 0 || r.w == 0 {
            return;
        }
        let pw = r.w;
        let ph = rows * 2;
        let pix = vp_render::render_view(&mut self.session.scene, &self.session.view, pw, ph);
        if pix.len() < pw * ph {
            return;
        }
        for cy in 0..rows {
            for cx in 0..pw {
                let top = rgb(pix[(cy * 2) * pw + cx]);
                let bot = rgb(pix[(cy * 2 + 1) * pw + cx]);
                self.screen.set(r.x + cx, r.y + 1 + cy, Cell { ch: '\u{2580}', fg: top, bg: bot, bold: false });
            }
        }
    }

    fn draw_help(&mut self) {
        let w = HELP_LINES.iter().map(|l| l.chars().count()).max().unwrap_or(40) + 4;
        let h = HELP_LINES.len() + 2;
        let w = w.min(self.screen.w);
        let h = h.min(self.screen.h);
        let r = Rect::new((self.screen.w - w) / 2, (self.screen.h - h) / 2, w, h);
        self.screen.draw_box(r, CYAN, PANEL_DARK, Some("Help"));
        for (i, line) in HELP_LINES.iter().enumerate() {
            if i + 2 > h - 1 {
                break;
            }
            let fg = if i == 0 { YELLOW } else { TEXT };
            self.screen.text(r.x + 2, r.y + 1 + i, line, fg, PANEL_DARK, r.x + r.w - 1);
        }
    }

    // ---------------------------------------------------------------- actions

    fn run_cmd(&mut self, cmd: &str) {
        self.shell.run(&mut self.session, cmd);
        self.dirty = true;
    }

    fn selection_names(&self) -> Vec<String> {
        self.session.scene.selection.iter().map(|id| self.session.scene.name_of(*id)).collect()
    }

    fn say(&mut self, msg: &str) {
        self.shell.push(&format!("// {}", msg), false);
    }

    fn key_channels(&mut self, targets: Vec<(NodeId, String)>) {
        let layer = self.session.scene.anim.active_layer;
        let mut n = 0;
        let mut err: Option<String> = None;
        for (id, attr) in targets {
            match self.session.scene.set_keyframe(id, &attr, None, None, Interp::Bezier, layer) {
                Ok(_) => n += 1,
                Err(e) => err = Some(e),
            }
        }
        let f = self.session.scene.timeline.frame();
        if n > 0 {
            self.say(&format!("Keyed {} channel(s) at frame {}", n, f));
        } else if let Some(e) = err {
            self.shell.push(&format!("// Error: {}", e), true);
        }
    }

    fn transform_channels(&self) -> Vec<(NodeId, String)> {
        let mut v = Vec::new();
        for id in &self.session.scene.selection {
            if self.session.scene.node(*id).map(|n| n.data.is_transform()).unwrap_or(false) {
                for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"] {
                    v.push((*id, a.to_string()));
                }
            }
        }
        v
    }

    fn focused_channel(&self) -> Option<(NodeId, String)> {
        let row = self.rows.get(self.chan.cursor)?;
        Some((row.node?, row.attr.clone()))
    }

    fn perform(&mut self, a: Action) {
        self.dirty = true;
        match a {
            Action::Cmd(c) => self.run_cmd(c),
            Action::Special(s) => self.special(s),
        }
    }

    fn special(&mut self, s: Special) {
        let frame = self.session.scene.timeline.frame();
        match s {
            Special::Exit => self.session.quit = true,
            Special::ToggleGrid => self.session.view.grid = !self.session.view.grid,
            Special::FrameAll => {
                let (v, sc) = (&mut self.session.view, &mut self.session.scene);
                v.frame_scene(sc);
            }
            Special::NextMode => self.session.view.mode = self.session.view.mode.next(),
            Special::SetMode(m) => {
                if let Some(mode) = DisplayMode::parse(m) {
                    self.session.view.mode = mode;
                }
            }
            Special::KeySelected => {
                let targets = if self.focus == Focus::Channel {
                    match self.focused_channel() {
                        Some(t) => vec![t],
                        None => self.transform_channels(),
                    }
                } else {
                    self.transform_channels()
                };
                if targets.is_empty() {
                    self.say("Select an object first");
                } else {
                    self.key_channels(targets);
                }
            }
            Special::CutKeySelected => {
                let layers = self.session.scene.anim.layers.len();
                let t = self.session.scene.timeline.current;
                let mut n = 0;
                for (id, a) in self.transform_channels() {
                    for l in 0..layers {
                        if self.session.scene.anim.remove_key(id, &a, l, t) {
                            n += 1;
                        }
                    }
                }
                self.session.scene.evaluate_frame();
                self.say(&format!("Removed {} key(s) at frame {}", n, frame));
            }
            Special::RenderBeauty => self.run_cmd(&format!("render -mode beauty -out \"frame_{}_beauty.txt\"", frame)),
            Special::RenderAll => self.run_cmd(&format!("render -mode all -out \"frame_{}_all.txt\"", frame)),
            Special::RenderAnaglyph => self.run_cmd(&format!("render -mode anaglyph -out \"frame_{}_anaglyph.txt\"", frame)),
            Special::Help => self.help = true,
            Special::ToggleShell => self.expanded = !self.expanded,
            Special::ResetTransform => {
                for n in self.selection_names() {
                    self.run_cmd(&format!("setAttr \"{}.translate\" 0 0 0", n));
                    self.run_cmd(&format!("setAttr \"{}.rotate\" 0 0 0", n));
                    self.run_cmd(&format!("setAttr \"{}.scale\" 1 1 1", n));
                }
            }
            Special::PlayPause => {
                self.session.scene.timeline.toggle();
            }
            Special::SelectAll => self.run_cmd("select -all"),
            Special::Deselect => self.run_cmd("select -cl"),
        }
    }

    fn jump(&mut self, frame: f32) {
        self.session.scene.set_time(frame);
        self.dirty = true;
    }

    fn step_frame(&mut self, d: i32) {
        let tl = &self.session.scene.timeline;
        let t = (tl.current + d as f32).clamp(tl.start, tl.end);
        self.jump(t);
    }

    // ---------------------------------------------------------------- keyboard

    fn on_key(&mut self, key: Key, mods: Mods) {
        self.dirty = true;
        if self.help {
            self.help = false;
            return;
        }
        match key {
            Key::Ctrl('q') => {
                self.session.quit = true;
                return;
            }
            Key::Ctrl('c') => {
                self.session.scene.timeline.pause();
                self.shell.input.clear();
                self.shell.cursor = 0;
                return;
            }
            Key::F(1) => {
                self.help = true;
                return;
            }
            Key::F(2) => {
                self.expanded = !self.expanded;
                return;
            }
            Key::F(10) => {
                self.menu.open_menu(0);
                return;
            }
            _ => {}
        }
        if mods.alt {
            if let Key::Char(c) = key {
                if let Some(i) = menu::menu_for_alt(c) {
                    if self.menu.open == Some(i) {
                        self.menu.close();
                    } else {
                        self.menu.open_menu(i);
                    }
                    return;
                }
            }
        }
        if self.menu.is_open() {
            match key {
                Key::Esc => self.menu.close(),
                Key::Left => self.menu.move_title(-1),
                Key::Right => self.menu.move_title(1),
                Key::Up => self.menu.move_item(-1),
                Key::Down => self.menu.move_item(1),
                Key::Enter => {
                    if let Some(a) = self.menu.current() {
                        self.menu.close();
                        self.perform(a);
                    }
                }
                _ => {}
            }
            return;
        }
        if key == Key::Esc {
            if self.chan.editing.is_some() {
                self.chan.editing = None;
            } else {
                self.focus = if self.focus == Focus::Shell { Focus::Viewport } else { Focus::Shell };
            }
            return;
        }
        match self.focus {
            Focus::Shell => {
                if let Some(line) = self.shell.handle_key(key, mods) {
                    self.shell.run(&mut self.session, &line);
                }
            }
            _ => {
                if self.chan.editing.is_some() {
                    self.edit_key(key);
                    return;
                }
                match key {
                    Key::Tab => {
                        self.cycle_focus(1);
                        return;
                    }
                    Key::BackTab => {
                        self.cycle_focus(-1);
                        return;
                    }
                    Key::Char(' ') => {
                        self.special(Special::PlayPause);
                        return;
                    }
                    Key::Char(',') => {
                        self.step_frame(-1);
                        return;
                    }
                    Key::Char('.') => {
                        self.step_frame(1);
                        return;
                    }
                    Key::Char('k') => {
                        self.special(Special::KeySelected);
                        return;
                    }
                    Key::Delete => {
                        self.run_cmd("delete");
                        return;
                    }
                    _ => {}
                }
                match self.focus {
                    Focus::Viewport => self.viewport_key(key, mods),
                    Focus::Outliner => self.outliner_key(key),
                    Focus::Channel => self.channel_key(key),
                    Focus::Shell => {}
                }
            }
        }
    }

    fn cycle_focus(&mut self, d: i32) {
        let mut order = vec![Focus::Viewport];
        if self.layout.outliner.is_some() {
            order.insert(0, Focus::Outliner);
        }
        if self.layout.channel.is_some() {
            order.push(Focus::Channel);
        }
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0) as i32;
        self.focus = order[(i + d).rem_euclid(order.len() as i32) as usize];
    }

    fn viewport_key(&mut self, key: Key, mods: Mods) {
        let v = &mut self.session.view;
        match key {
            Key::Left if mods.shift => v.pan(-0.04, 0.0),
            Key::Right if mods.shift => v.pan(0.04, 0.0),
            Key::Up if mods.shift => v.pan(0.0, 0.04),
            Key::Down if mods.shift => v.pan(0.0, -0.04),
            Key::Left => v.orbit(-5.0, 0.0),
            Key::Right => v.orbit(5.0, 0.0),
            Key::Up => v.orbit(0.0, 4.0),
            Key::Down => v.orbit(0.0, -4.0),
            Key::Char('+') | Key::Char('=') => v.zoom(0.88),
            Key::Char('-') | Key::Char('_') => v.zoom(1.0 / 0.88),
            Key::Char('1') => v.mode = DisplayMode::Shaded,
            Key::Char('2') => v.mode = DisplayMode::Wireframe,
            Key::Char('3') => v.mode = DisplayMode::Anaglyph,
            Key::Char('m') => v.mode = v.mode.next(),
            Key::Char('g') => v.grid = !v.grid,
            Key::Char('f') => {
                let (v, sc) = (&mut self.session.view, &mut self.session.scene);
                v.frame_scene(sc);
            }
            Key::Char('a') => self.run_cmd("select -all"),
            Key::Home => {
                let s = self.session.scene.timeline.start;
                self.jump(s);
            }
            Key::End => {
                let e = self.session.scene.timeline.end;
                self.jump(e);
            }
            _ => {}
        }
        let (v, sc) = (&self.session.view, &mut self.session.scene);
        v.apply(sc);
    }

    fn outliner_key(&mut self, key: Key) {
        match key {
            Key::Up => outliner::step_selection(&mut self.session.scene, -1),
            Key::Down => outliner::step_selection(&mut self.session.scene, 1),
            Key::Home => outliner::step_selection(&mut self.session.scene, -10_000),
            Key::End => outliner::step_selection(&mut self.session.scene, 10_000),
            _ => {}
        }
    }

    fn channel_key(&mut self, key: Key) {
        let rows = std::mem::take(&mut self.rows);
        match key {
            Key::Up => self.chan.move_cursor(&rows, -1),
            Key::Down => self.chan.move_cursor(&rows, 1),
            Key::Enter => {
                if let Some(r) = rows.get(self.chan.cursor) {
                    if r.editable {
                        self.chan.editing = Some(r.text.clone());
                    }
                }
            }
            Key::Char('+') | Key::Char('=') | Key::Char('-') | Key::Char('_') => {
                if let Some(r) = rows.get(self.chan.cursor) {
                    if let (true, Some(id)) = (r.editable, r.node) {
                        let step = if r.attr.starts_with("rotate") { 5.0 } else { 0.25 };
                        let sign = if matches!(key, Key::Char('+') | Key::Char('=')) { 1.0 } else { -1.0 };
                        let _ = self.session.scene.set_attr(id, &r.attr, r.value + sign * step);
                        self.session.scene.evaluate_frame();
                    }
                }
            }
            _ => {}
        }
        self.rows = rows;
    }

    fn edit_key(&mut self, key: Key) {
        match key {
            Key::Enter => {
                let text = self.chan.editing.take().unwrap_or_default();
                if let Some((id, attr)) = self.focused_channel() {
                    let name = self.session.scene.name_of(id);
                    self.run_cmd(&format!("setAttr \"{}.{}\" {}", name, attr, text.trim()));
                }
            }
            Key::Backspace => {
                if let Some(b) = self.chan.editing.as_mut() {
                    b.pop();
                }
            }
            Key::Char(c) => {
                if let Some(b) = self.chan.editing.as_mut() {
                    b.push(c);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- mouse

    fn viewport_pixels(&self) -> Option<Rect> {
        let r = self.layout.viewport?;
        Some(Rect::new(r.x, r.y + 1, r.w, r.h.saturating_sub(1)))
    }

    fn pick_at(&mut self, x: usize, y: usize, add: bool) {
        let p = match self.viewport_pixels() {
            Some(p) if !p.is_empty() => p,
            _ => return,
        };
        let nx = (x.saturating_sub(p.x) as f32 + 0.5) / p.w as f32;
        let ny = (y.saturating_sub(p.y) as f32 + 0.5) / p.h as f32;
        let aspect = p.w as f32 / (p.h * 2) as f32;
        let hit = vp_render::pick(&mut self.session.scene, &self.session.view, nx, ny, aspect);
        let sc = &mut self.session.scene;
        match hit {
            Some(id) => {
                let t = sc.node(id).and_then(|n| n.parent).unwrap_or(id);
                if add {
                    if let Some(i) = sc.selection.iter().position(|s| *s == t) {
                        sc.selection.remove(i);
                    } else {
                        sc.selection.push(t);
                    }
                } else {
                    sc.selection = vec![t];
                }
            }
            None => {
                if !add {
                    sc.selection.clear();
                }
            }
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        self.dirty = true;
        if self.help {
            if matches!(m.kind, MouseKind::Down(_)) {
                self.help = false;
            }
            return;
        }
        let (x, y) = (m.x, m.y);
        if self.menu.is_open() {
            match m.kind {
                MouseKind::Down(Button::Left) => {
                    if let Some(k) = self.menu.item_at(x, y) {
                        let i = self.menu.open.unwrap_or(0);
                        let a = MENUS[i].items[k].action;
                        self.menu.close();
                        self.perform(a);
                    } else if let Some(t) = self.menu.title_at(x, y) {
                        if self.menu.open == Some(t) {
                            self.menu.close();
                        } else {
                            self.menu.open_menu(t);
                        }
                    } else {
                        self.menu.close();
                    }
                }
                MouseKind::Drag(Button::Left) => {
                    if let Some(k) = self.menu.item_at(x, y) {
                        self.menu.hover = k;
                    }
                }
                MouseKind::Up(Button::Left) => {
                    if let (Some(k), Some(i)) = (self.menu.item_at(x, y), self.menu.open) {
                        if self.menu.in_dropdown(x, y) {
                            let a = MENUS[i].items[k].action;
                            self.menu.close();
                            self.perform(a);
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        let l = self.layout;
        match m.kind {
            MouseKind::Down(b) => {
                self.last = (x, y);
                if y == 0 {
                    if let Some(t) = self.menu.title_at(x, y) {
                        self.menu.open_menu(t);
                    }
                    return;
                }
                if l.viewport.map(|r| r.contains(x, y)).unwrap_or(false) {
                    self.focus = Focus::Viewport;
                    if y == l.viewport.map(|r| r.y).unwrap_or(0) {
                        return;
                    }
                    self.drag = match (b, m.mods.alt) {
                        (Button::Left, true) => Drag::Orbit,
                        (Button::Left, false) => Drag::ViewClick { moved: false, shift: m.mods.shift },
                        (Button::Middle, _) => Drag::Pan,
                        (Button::Right, _) => Drag::Zoom,
                    };
                } else if let Some(r) = l.outliner.filter(|r| r.contains(x, y)) {
                    self.focus = Focus::Outliner;
                    if let Some(id) = outliner::row_at(&self.session.scene, &self.outliner, r, y) {
                        let sc = &mut self.session.scene;
                        if m.mods.shift && !sc.selection.contains(&id) {
                            sc.selection.push(id);
                        } else {
                            sc.selection = vec![id];
                        }
                    }
                } else if let Some(r) = l.channel.filter(|r| r.contains(x, y)) {
                    self.focus = Focus::Channel;
                    if y > r.y {
                        let idx = self.chan.scroll + y - r.y - 1;
                        if let Some(row) = self.rows.get(idx) {
                            if row.node.is_some() {
                                self.chan.cursor = idx;
                                self.chan.editing = None;
                            }
                        }
                    }
                } else if let Some(r) = l.editor.filter(|r| r.contains(x, y)) {
                    let hit = self.chan.sliders.iter().find(|(sr, _)| sr.contains(x, y) || (y == sr.y && x >= r.x && x < r.x + r.w && x + 8 >= sr.x && x <= sr.x + sr.w)).copied();
                    if let Some((sr, idx)) = hit {
                        self.set_weight(sr, idx, x);
                        self.drag = Drag::Slider(sr, idx);
                    }
                } else if l.timeline.map(|r| r.contains(x, y)).unwrap_or(false) {
                    let btn = self.tl.buttons.iter().find(|(r, _)| r.contains(x, y)).map(|(_, t)| *t);
                    if let Some(t) = btn {
                        self.transport(t);
                    } else if let Some(tr) = self.tl.track {
                        if tr.contains(x, y) {
                            if let Some(f) = timeline::frame_at(&self.tl, &self.session.scene, x) {
                                self.jump(f);
                            }
                            self.drag = Drag::Scrub;
                        }
                    }
                } else if l.shell.map(|r| r.contains(x, y)).unwrap_or(false) {
                    self.focus = Focus::Shell;
                }
            }
            MouseKind::Drag(_) => {
                let dx = x as f32 - self.last.0 as f32;
                let dy = y as f32 - self.last.1 as f32;
                match self.drag {
                    Drag::Orbit => self.session.view.orbit(-dx * 2.0, dy * 3.5),
                    Drag::Pan => self.session.view.pan(-dx * 0.02, dy * 0.04),
                    Drag::Zoom => self.session.view.zoom((1.0 - dx * 0.03 + dy * 0.05).clamp(0.5, 2.0)),
                    Drag::ViewClick { moved, shift } => {
                        if dx != 0.0 || dy != 0.0 {
                            self.session.view.orbit(-dx * 2.0, dy * 3.5);
                            self.drag = Drag::ViewClick { moved: true, shift };
                        }
                        let _ = moved;
                    }
                    Drag::Scrub => {
                        if let Some(f) = timeline::frame_at(&self.tl, &self.session.scene, x) {
                            self.jump(f);
                        }
                    }
                    Drag::Slider(sr, idx) => self.set_weight(sr, idx, x),
                    Drag::None => {}
                }
                self.last = (x, y);
                let (v, sc) = (&self.session.view, &mut self.session.scene);
                v.apply(sc);
            }
            MouseKind::Up(_) => {
                if let Drag::ViewClick { moved: false, shift } = self.drag {
                    self.pick_at(x, y, shift);
                }
                self.drag = Drag::None;
            }
            MouseKind::WheelUp | MouseKind::WheelDown => {
                let up = m.kind == MouseKind::WheelUp;
                if l.viewport.map(|r| r.contains(x, y)).unwrap_or(false) {
                    self.session.view.zoom(if up { 0.88 } else { 1.0 / 0.88 });
                    let (v, sc) = (&self.session.view, &mut self.session.scene);
                    v.apply(sc);
                } else if l.shell.map(|r| r.contains(x, y)).unwrap_or(false) {
                    if up {
                        self.shell.scroll = (self.shell.scroll + 2).min(self.shell.log.len().saturating_sub(1));
                    } else {
                        self.shell.scroll = self.shell.scroll.saturating_sub(2);
                    }
                } else if l.outliner.map(|r| r.contains(x, y)).unwrap_or(false) {
                    if up {
                        self.outliner.scroll = self.outliner.scroll.saturating_sub(2);
                    } else {
                        self.outliner.scroll += 2;
                    }
                } else if l.channel.map(|r| r.contains(x, y)).unwrap_or(false) {
                    if up {
                        self.chan.scroll = self.chan.scroll.saturating_sub(2);
                    } else {
                        self.chan.scroll += 2;
                    }
                }
            }
        }
    }

    fn set_weight(&mut self, sr: Rect, idx: usize, x: usize) {
        let w = channel::slider_weight(sr, x);
        if idx < self.session.scene.anim.layers.len() {
            self.session.scene.anim.set_weight(idx, w);
            self.session.scene.evaluate_frame();
        }
    }

    fn transport(&mut self, t: Transport) {
        let s = self.session.scene.timeline.start;
        let e = self.session.scene.timeline.end;
        match t {
            Transport::Rewind => self.jump(s),
            Transport::StepBack => self.step_frame(-1),
            Transport::PlayPause => self.session.scene.timeline.toggle(),
            Transport::StepFwd => self.step_frame(1),
            Transport::End => self.jump(e),
        }
    }

    // ---------------------------------------------------------------- loop

    fn present(&mut self) {
        let out = self.screen.flush();
        let mut so = std::io::stdout().lock();
        let _ = so.write_all(out.as_bytes());
        let _ = so.flush();
    }
}

/// Run the full screen workspace until the user quits.
pub fn run(session: Session) -> std::io::Result<Session> {
    let _term = RawTerm::enter()?;
    let (w, h) = term_size();
    let mut app = App::new(session, w, h, supports_truecolor());
    let rx = spawn_input();
    let mut last_tick = Instant::now();
    loop {
        let (nw, nh) = term_size();
        if nw != app.screen.w || nh != app.screen.h {
            app.screen.resize(nw, nh);
            app.dirty = true;
        }
        if app.dirty {
            app.dirty = false;
            app.draw();
            app.present();
        }
        if app.session.quit {
            break;
        }
        let playing = app.session.scene.timeline.playing;
        let wait = if playing { Duration::from_millis(15) } else { Duration::from_millis(80) };
        match rx.recv_timeout(wait) {
            Ok(first) => {
                let mut evs = vec![first];
                while let Ok(e) = rx.try_recv() {
                    evs.push(e);
                    if evs.len() > 256 {
                        break;
                    }
                }
                for e in evs {
                    match e {
                        Event::Key(k, m) => app.on_key(k, m),
                        Event::Mouse(m) => app.on_mouse(m),
                    }
                    if app.session.quit {
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let now = Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f32();
        last_tick = now;
        if app.session.scene.timeline.playing {
            let was_playing = true;
            if app.session.scene.advance_playback(dt) || (was_playing && !app.session.scene.timeline.playing) {
                app.dirty = true;
            }
        }
    }
    Ok(app.session)
}
