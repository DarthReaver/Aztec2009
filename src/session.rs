use crate::dag::{NodeId, Scene};
use crate::geo::edit::Sel;
use crate::mel::parser::Value;
use crate::ui::viewport::ViewState;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelMode {
    Object,
    Vertex,
    Edge,
    Face,
}

/// Component selection on one mesh shape (Maya style component mode).
#[derive(Clone, Debug)]
pub struct Components {
    pub mode: SelMode,
    pub node: Option<NodeId>,
    pub sel: Sel,
    /// Selected UV points of the shape in the UV Texture Editor.
    pub uv_sel: std::collections::BTreeSet<u32>,
}

impl Components {
    pub fn clear_sel(&mut self) {
        self.sel = Sel::default();
    }
}

/// Settings that persist between sessions (Window > Settings/Preferences) and snapping options.
#[derive(Clone, Debug)]
pub struct Prefs {
    pub snap_grid: bool,
    pub snap_point: bool,
    /// Snap increments: world units, degrees and scale factor.
    pub move_step: f32,
    pub rotate_step: f32,
    pub scale_step: f32,
    /// Undo levels kept (0 = unlimited).
    pub undo_limit: usize,
    /// Minutes between automatic saves of the open scene (0 = off).
    pub autosave_minutes: f32,
    /// Display mode of new sessions: "shaded", "textured" or "wireframe".
    pub default_display: String,
    /// Size of the move / rotate / scale manipulators, 0.5 to 3.
    pub manip_size: f32,
    /// Default key tangent type: "spline", "linear" or "step".
    pub key_tangent: String,
    /// Frame rate unit of new scenes: film, pal, ntsc, show, game, palf, ntscf.
    pub time_unit: String,
    /// Custom hotkeys (Window > Settings/Preferences > Hotkey Editor).
    pub hotkeys: Vec<Hotkey>,
    /// Edit Mesh > Keep Faces Together: extruding several faces moves them as one piece.
    pub keep_faces_together: bool,
    /// Tangent types of new keys: in and out (clamped, spline, linear, flat, plateau; out also step).
    pub tangent_in: String,
    pub tangent_out: String,
    /// Custom shelf buttons and marking menus (saved with the preferences).
    pub shelf: Vec<crate::mel::ui::ShelfButton>,
    pub marking_menus: Vec<crate::mel::ui::MarkingMenu>,
}

/// A key bound to a command; runs when the key is pressed over the workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct Hotkey {
    /// egui key name: "A", "F5", "Insert", "Num1" ...
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub command: String,
}

impl Hotkey {
    /// "ctrl+shift+K" style label.
    pub fn label(&self) -> String {
        let mut s = String::new();
        for (on, n) in [(self.ctrl, "ctrl+"), (self.alt, "alt+"), (self.shift, "shift+")] {
            if on {
                s.push_str(n);
            }
        }
        s + &self.key
    }

    /// Parse "ctrl+alt+K" (case insensitive modifiers).
    pub fn parse_label(label: &str, command: &str) -> Option<Hotkey> {
        let mut h = Hotkey { key: String::new(), ctrl: false, alt: false, shift: false, command: command.to_string() };
        for part in label.split('+') {
            match part.trim().to_lowercase().as_str() {
                "ctrl" | "control" | "cmd" => h.ctrl = true,
                "alt" => h.alt = true,
                "shift" => h.shift = true,
                "" => {}
                _ => h.key = part.trim().to_string(),
            }
        }
        if h.key.is_empty() || command.trim().is_empty() {
            return None;
        }
        Some(h)
    }
}

impl Prefs {
    /// ~/.config/aztec2009/prefs.txt (or $XDG_CONFIG_HOME); %APPDATA%\aztec2009\prefs.txt on Windows
    pub fn path() -> Option<std::path::PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
            .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
            .or_else(|| std::env::var_os("USERPROFILE").map(|h| std::path::PathBuf::from(h).join(".config")))?;
        Some(base.join("aztec2009").join("prefs.txt"))
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# Aztec 2009 preferences\n");
        s.push_str(&format!("snap_grid={}\nsnap_point={}\nmove_step={}\nrotate_step={}\nscale_step={}\n", self.snap_grid as u8, self.snap_point as u8, self.move_step, self.rotate_step, self.scale_step));
        s.push_str(&format!("undo_limit={}\nautosave_minutes={}\ndefault_display={}\nmanip_size={}\nkey_tangent={}\ntime_unit={}\nkeep_faces_together={}\ntangent_in={}\ntangent_out={}\n", self.undo_limit, self.autosave_minutes, self.default_display, self.manip_size, self.key_tangent, self.time_unit, self.keep_faces_together as u8, self.tangent_in, self.tangent_out));
        for h in &self.hotkeys {
            s.push_str(&format!("hotkey={}|{}\n", h.label(), h.command.replace('\n', " ")));
        }
        for b in &self.shelf {
            s.push_str(&format!("shelf={}\t{}\t{}\n", b.label.replace(['\t', '\n'], " "), b.command.replace(['\t', '\n'], " "), b.annotation.replace(['\t', '\n'], " ")));
        }
        for m in &self.marking_menus {
            let items: Vec<String> = m.items.iter().map(|i| match i { Some((l, c)) => format!("{}\u{1}{}", l.replace(['\t', '\n', '\u{1}'], " "), c.replace(['\t', '\n', '\u{1}'], " ")), None => String::new() }).collect();
            s.push_str(&format!("markingMenu={}\t{}\n", m.name, items.join("\t")));
        }
        s
    }

    /// Read settings from text; unknown or bad lines are ignored so old files keep working.
    pub fn from_text(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let line = line.trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            let f = || v.trim().parse::<f32>().ok();
            match k.trim() {
                "snap_grid" => p.snap_grid = v.trim() == "1",
                "snap_point" => p.snap_point = v.trim() == "1",
                "move_step" => p.move_step = f().filter(|x| *x > 0.0).unwrap_or(p.move_step),
                "rotate_step" => p.rotate_step = f().filter(|x| *x > 0.0).unwrap_or(p.rotate_step),
                "scale_step" => p.scale_step = f().filter(|x| *x > 0.0).unwrap_or(p.scale_step),
                "undo_limit" => p.undo_limit = v.trim().parse().unwrap_or(p.undo_limit),
                "autosave_minutes" => p.autosave_minutes = f().filter(|x| *x >= 0.0).unwrap_or(0.0),
                "default_display" => p.default_display = v.trim().to_string(),
                "manip_size" => p.manip_size = f().map(|x| x.clamp(0.5, 3.0)).unwrap_or(1.0),
                "keep_faces_together" => p.keep_faces_together = v.trim() != "0",
                "key_tangent" => p.key_tangent = v.trim().to_string(),
                "tangent_in" => p.tangent_in = v.trim().to_string(),
                "tangent_out" => p.tangent_out = v.trim().to_string(),
                "time_unit" => p.time_unit = v.trim().to_string(),
                "shelf" => {
                    let f: Vec<&str> = v.split('\t').collect();
                    if f.len() >= 2 {
                        p.shelf.push(crate::mel::ui::ShelfButton { label: f[0].to_string(), command: f[1].to_string(), annotation: f.get(2).unwrap_or(&"").to_string() });
                    }
                }
                "markingMenu" => {
                    let f: Vec<&str> = v.split('\t').collect();
                    if !f.is_empty() && !f[0].is_empty() {
                        let mut m = crate::mel::ui::MarkingMenu { name: f[0].to_string(), items: Default::default() };
                        for (i, it) in f[1..].iter().take(8).enumerate() {
                            if let Some((l, c)) = it.split_once('\u{1}') {
                                m.items[i] = Some((l.to_string(), c.to_string()));
                            }
                        }
                        p.marking_menus.push(m);
                    }
                }
                "hotkey" => {
                    if let Some((label, cmd)) = v.split_once('|') {
                        if let Some(h) = Hotkey::parse_label(label, cmd) {
                            p.hotkeys.push(h);
                        }
                    }
                }
                _ => {}
            }
        }
        p
    }

    pub fn load() -> Prefs {
        Prefs::path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Prefs::from_text(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> Result<std::path::PathBuf, String> {
        let p = Prefs::path().ok_or("No home directory for the preferences file")?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("Cannot create '{}': {}", d.display(), e))?;
        }
        std::fs::write(&p, self.to_text()).map_err(|e| format!("Cannot write '{}': {}", p.display(), e))?;
        Ok(p)
    }
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { snap_grid: false, snap_point: false, move_step: 1.0, rotate_step: 15.0, scale_step: 0.1, undo_limit: 100, autosave_minutes: 0.0, default_display: "shaded".to_string(), manip_size: 1.0, key_tangent: "spline".to_string(), time_unit: "film".to_string(), hotkeys: Vec::new(), keep_faces_together: true, tangent_in: "clamped".to_string(), tangent_out: "clamped".to_string(), shelf: Vec::new(), marking_menus: Vec::new() }
    }
}

/// Everything the shell, scripts and UI share: the scene, script variables and view settings.
pub struct Session {
    pub scene: Scene,
    pub vars: HashMap<String, Value>,
    pub view: ViewState,
    pub comp: Components,
    /// Record construction history for modeling operations (Maya's history toggle).
    pub history_on: bool,
    pub quit: bool,
    /// Nesting depth of `source` commands, to stop runaway recursion.
    pub source_depth: u32,
    /// Objects hidden by Isolate Select, restored when it is switched off.
    pub isolated: Vec<NodeId>,
    /// What the last `hide` hid (Display > Show > Last Hidden).
    pub last_hidden: Vec<NodeId>,
    /// User procedures (`proc name(...) { ... }`).
    pub procs: HashMap<String, std::sync::Arc<crate::mel::script::Proc>>,
    /// Local variable frames of the running procedures.
    pub frames: Vec<crate::mel::script::Frame>,
    /// Statements run by the current top level script (loop guard).
    pub steps: u64,
    pub exec_depth: u32,
    /// Prefix errors with the script line number (multi line scripts).
    pub err_lines: bool,
    pub rng: u64,
    /// The character set that Set Key keys (Current Character).
    pub current_character: Option<crate::dag::NodeId>,
    /// Output printed by procedures called from inside expressions.
    pub pending: Vec<String>,
    /// Copied keyframes (time slider Copy / Cut / Paste).
    pub key_clip: Vec<crate::mel::cmd_keys::ClipKey>,
    pub prefs: Prefs,
    /// 3D Paint tool: the texture being painted and the brush.
    pub paint: crate::mel::cmd_paint::PaintState,
    /// Windows and controls made by scripts.
    pub ui: crate::mel::ui::UiState,
    /// A marking menu the workspace should show at the pointer (markingMenu -popup).
    pub popup_menu: Option<String>,
    /// Echo All Commands: the Script Editor also shows the commands scripts and procedures run.
    pub echo_all: bool,
    /// Camera bookmarks: name and the view they restore.
    pub bookmarks: Vec<(String, crate::ui::viewport::ViewState)>,
    /// Sculpt Geometry Tool settings and its reference surfaces.
    pub sculpt: crate::mel::cmd_sculpt::SculptState,
    /// The message of the last error caught by catch().
    pub last_error: String,
    /// undo / redo asked for by a script: -1 undo, 1 redo (the workspace carries it out).
    pub undo_request: i32,
    /// Extra folders searched for scripts (scriptPath -add).
    pub script_dirs: Vec<std::path::PathBuf>,
    /// The Paint Effects 2D canvas.
    pub canvas: crate::mel::cmd_paintfx::Canvas,
}

impl Session {
    pub fn new() -> Session {
        let mut s = Session {
            scene: Scene::new(),
            vars: HashMap::new(),
            view: ViewState::new(),
            comp: Components { mode: SelMode::Object, node: None, sel: Sel::default(), uv_sel: Default::default() },
            history_on: true,
            quit: false,
            source_depth: 0,
            isolated: Vec::new(),
            last_hidden: Vec::new(),
            procs: HashMap::new(),
            frames: Vec::new(),
            steps: 0,
            exec_depth: 0,
            err_lines: false,
            rng: 0,
            current_character: None,
            pending: Vec::new(),
            key_clip: Vec::new(),
            prefs: Prefs::default(),
            paint: Default::default(),
            ui: Default::default(),
            popup_menu: None,
            echo_all: false,
            bookmarks: Vec::new(),
            sculpt: Default::default(),
            last_error: String::new(),
            undo_request: 0,
            script_dirs: Vec::new(),
            canvas: Default::default(),
        };
        s.view.apply(&mut s.scene);
        s
    }

    /// Re-read the viewport camera from the scene (after opening a file, for example).
    pub fn resync_view(&mut self) {
        if self.scene.find("persp").is_none() {
            self.scene.add_default_camera();
        }
        self.view.sync_from_scene(&mut self.scene);
    }
}

impl Session {
    /// Push the tangent preferences into the animation system (new keys use them).
    pub fn sync_anim_prefs(&mut self) {
        use crate::anim::keyframes::Tan;
        let i = Tan::parse(&self.prefs.tangent_in).unwrap_or(Tan::Clamped);
        let o = Tan::parse(&self.prefs.tangent_out).unwrap_or(Tan::Clamped);
        self.scene.anim.default_tans = (i, o);
    }

    /// Apply the saved preferences when the workspace opens (not for scripts, so batch runs stay predictable).
    pub fn apply_startup_prefs(&mut self) {
        self.prefs = Prefs::load();
        self.sync_anim_prefs();
        if let Some(m) = crate::ui::viewport::DisplayMode::parse(&self.prefs.default_display) {
            self.view.mode = m;
        }
        let fps = match self.prefs.time_unit.as_str() {
            "game" => 15.0,
            "pal" => 25.0,
            "ntsc" => 30.0,
            "show" => 48.0,
            "palf" => 50.0,
            "ntscf" => 60.0,
            _ => 24.0,
        };
        self.scene.timeline.fps = fps;
    }
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}
