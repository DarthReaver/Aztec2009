//! The windowed workspace: menus, shelf, tool box, viewports with manipulators, channel box,
//! outliner, layer editor, time slider and command line, laid out like Maya 2009.

use super::icons::{self, Icon};
use super::png;
use super::theme::*;
use super::viewport::{self, bounds, make_cam, project, Cam, Ortho, ViewKind};
use crate::dag::nodes::{NodeData, PolyKind};
use crate::dag::{serialize, NodeId, Scene};
use crate::math::{Mat4, Vec3};
use crate::geo::edit::{ekey, Edge, Poly, Sel};
use crate::mel::cmd_keys;
use crate::mel::cmd_model::{self, ModelOp, SplitPt};
use crate::mel::cmd_rig;
use crate::mel::execute_line;
use crate::session::{SelMode, Session};
use crate::ui::channel::{self, ChanRow};
use crate::ui::viewport::{DisplayMode, ViewState};
use crate::ui::vp_render;
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, FontId, Key, Layout, PointerButton, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

const KINDS: [ViewKind; 4] = [ViewKind::Persp, ViewKind::Top, ViewKind::Front, ViewKind::Side];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tool {
    Select,
    Move,
    Rotate,
    Scale,
    SplitPoly,
    EdgeLoop,
    Joint,
    IkHandle,
    /// Interactive creation of a polygon primitive: click two points on the grid.
    Prim,
    /// 3D Paint Tool: drag over a textured object to paint it.
    Paint,
    /// CV / EP Curve Tool: click points, Enter creates the curve.
    Curve,
    /// Paint Skin Weights tool.
    PaintSkin,
    PaintFx,
    Sculpt,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuSet {
    Animation,
    Polygons,
    Surfaces,
    Dynamics,
    Rendering,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum SelOp {
    Replace,
    Toggle,
    Remove,
}

enum CompHit {
    Vert(u32),
    Edge(Edge),
    Face(u32),
}

/// Screen space picture of the edited mesh for one viewport (projection, visibility, hit data).
struct CompView {
    shape: NodeId,
    poly: Poly,
    sp: Vec<Option<Pos2>>,
    vvis: Vec<bool>,
    edges: Vec<Edge>,
    evis: Vec<bool>,
    fvis: Vec<bool>,
    fctr: Vec<Option<Pos2>>,
    fdepth: Vec<f32>,
}

struct CompDrag {
    shape: NodeId,
    verts: Vec<(u32, Vec3)>,
    world: Mat4,
    inv: Mat4,
}

enum CbItem {
    Header { text: String, tag: String, level: u8, node: Option<NodeId>, shape: Option<NodeId> },
    Section(String),
    Info(String),
    Attr { node: NodeId, attr: String, label: String, value: f32, keyed: bool, connected: bool, editable: bool },
    Curve { name: String, ty: &'static str, node: NodeId, attr: String },
    Texture { mat: NodeId, path: Option<String> },
}

fn hist_label(k: PolyKind) -> &'static str {
    match k {
        PolyKind::Cube | PolyKind::Sphere | PolyKind::Cylinder | PolyKind::Plane | PolyKind::Cone | PolyKind::Torus => "polyPrimitive",
        PolyKind::Bend | PolyKind::Twist | PolyKind::Flare | PolyKind::Squash | PolyKind::Sine => "nonLinear",
        PolyKind::Source => "source geometry",
        PolyKind::Tweak => "tweak",
        _ => "polyModifier",
    }
}

#[derive(Clone, Copy, Default)]
struct PolyStats {
    total: [usize; 4],
    object: [usize; 4],
    selected: [usize; 3],
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum LayoutMode {
    Single,
    Four,
    PerspOutliner,
}

#[derive(Default)]
struct PanelState {
    depth: Vec<f32>,
    dsize: (usize, usize),
    tex: Option<TextureHandle>,
    size: (usize, usize),
    hq: bool,
    gen: u64,
}

struct ManipFrame {
    pivot: Vec3,
    center: Pos2,
    axes: [Vec3; 3],
    ends: [Pos2; 3],
    len_world: f32,
    upp: f32,
    right: Vec3,
    up: Vec3,
    eye_dir: Vec3,
}

struct ManipDrag {
    tool: Tool,
    /// 0..2 axis, 3 free / uniform.
    axis: usize,
    origin: Pos2,
    frame_center: Pos2,
    axis_world: Vec3,
    axis_dir: egui::Vec2,
    units_per_px: f32,
    right: Vec3,
    up: Vec3,
    upp: f32,
    start_angle: f32,
    angle_sign: f32,
    starts: Vec<(NodeId, Vec3, Vec3, Vec3, Mat4)>,
    comp: Option<CompDrag>,
    pivot: Vec3,
    /// World pivot of every dragged object when the drag started (pivot edit mode).
    pivot_starts: Vec<(NodeId, Vec3)>,
    /// Points the move can snap to (vertices, joints, locators), when point snapping is on.
    snap_pts: Vec<Vec3>,
    changed: bool,
}

enum Drag {
    None,
    Manip(ManipDrag),
    Marquee { start: Pos2, kind: ViewKind },
}

/// Create > Polygon Primitives > Options.
#[derive(Clone)]
struct PrimOpts {
    kind: usize,
    size: [f32; 3],
    radius: f32,
    section: f32,
    subdivisions: f32,
    name: String,
}

impl Default for PrimOpts {
    fn default() -> PrimOpts {
        PrimOpts { kind: 0, size: [1.0, 1.0, 1.0], radius: 1.0, section: 0.5, subdivisions: 1.0, name: String::new() }
    }
}

const PRIM_KINDS: [(&str, &str); 6] = [("Cube", "polyCube"), ("Sphere", "polySphere"), ("Cylinder", "polyCylinder"), ("Cone", "polyCone"), ("Plane", "polyPlane"), ("Torus", "polyTorus")];

/// Window > Animation Editors > Graph Editor state.
struct GraphEd {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    /// (binding index, key index)
    sel: Vec<(usize, usize)>,
    hidden: std::collections::HashSet<(NodeId, String)>,
    all_nodes: bool,
    fit_next: bool,
    drag: Option<(f32, f32)>,
}

/// Animate > Set Driven Key window: pick a driver and a driven attribute, pose, press Key.
#[derive(Default)]
struct SdkWindow {
    driver: Option<NodeId>,
    driver_attr: String,
    driven: Option<NodeId>,
    driven_attr: String,
}

/// Edit > Duplicate Special options.
struct DupSpecial {
    copies: i32,
    t: [f32; 3],
    r: [f32; 3],
    s: [f32; 3],
    group: bool,
    instance: bool,
}

struct FileDialog {
    /// 0 scene files, 1 import, 2 export all, 3 export selection
    io: u8,
    tex_for: Option<NodeId>,
    /// Component string (`shape.f[0:5]`) when the color map goes on faces only.
    tex_faces: Option<String>,
    image_plane: bool,
    save: bool,
    dir: PathBuf,
    name: String,
}

struct RenderView {
    tex: Option<TextureHandle>,
    w: usize,
    h: usize,
    rgba: Vec<u8>,
    res: usize,
    anaglyph: bool,
    ray: bool,
    quality: usize,
    save_name: String,
    note: String,
    /// Render with the Render Settings (layers, passes, effects) in a background process.
    pipe: bool,
    /// Show the alpha channel instead of the colors.
    show_alpha: bool,
    /// Pictures kept for comparison (Keep Image): note, width, height, RGBA.
    kept: Vec<(String, usize, usize, Vec<u8>)>,
    kept_view: Option<usize>,
    /// Region to render (pixels) and IPR state.
    region: Option<(usize, usize, usize, usize)>,
    auto_region: bool,
    ipr: bool,
    open_path: String,
}

/// A render running in its own process (so the window stays usable and the render can be cancelled).
struct PipeJob {
    child: std::process::Child,
    out: std::path::PathBuf,
    started: std::time::Instant,
    region: Option<(usize, usize, usize, usize)>,
    frame: i32,
}

#[derive(Default)]
struct RsetUi {
    show: bool,
    show_layers: bool,
    /// Layer whose Render Settings are shown ("" is the master layer).
    layer: String,
    tab: usize,
    new_layer: String,
}

#[derive(Default)]
struct FcheckUi {
    show: bool,
    path: String,
    frames: Vec<(i32, String)>,
    idx: usize,
    playing: bool,
    last: Option<std::time::Instant>,
    tex: Option<TextureHandle>,
    shown: Option<usize>,
    note: String,
}

struct RenderJob {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    progress: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    w: usize,
    h: usize,
    frame: i32,
    started: std::time::Instant,
}

pub struct App {
    session: Session,
    tool: Tool,
    layout: LayoutMode,
    single_kind: ViewKind,
    active: ViewKind,
    panels: [PanelState; 4],
    ortho: [Ortho; 3],
    gen: u64,
    frame_gen: u64,
    interacting: bool,
    drag: Drag,
    shelf_tab: usize,
    mel_input: String,
    mel_result: String,
    mel_error: bool,
    log: Vec<(String, bool)>,
    undo: Vec<String>,
    redo: Vec<String>,
    last_state: String,
    scene_edits: bool,
    auto_key: bool,
    show_outliner: bool,
    show_channels: bool,
    show_script: bool,
    show_help: bool,
    show_about: bool,
    show_layers: bool,
    help_line: String,
    rename: Option<(NodeId, String)>,
    file_dialog: Option<FileDialog>,
    dup_special: Option<DupSpecial>,
    sdk: Option<SdkWindow>,
    constrain_offset: bool,
    graph: Option<GraphEd>,
    node_ed: NodeEd,
    render_view: Option<RenderView>,
    stereo_scale: f32,
    rs_cache: Option<(u64, crate::render::RenderScene)>,
    menu_set: MenuSet,
    last_comp_mode: SelMode,
    hud_poly: bool,
    axis_normal: bool,
    split_pts: Vec<SplitPt>,
    loop_drag: Option<(Edge, f32)>,
    last_op: Option<ModelOp>,
    poly_stats: Option<(u64, PolyStats)>,
    curve_win: Option<(NodeId, String)>,
    show_ae: bool,
    ae_tab: Option<NodeId>,
    show_uv: bool,
    /// Pivot edit mode (Insert): the Move tool moves pivots instead of objects.
    pivot_edit: bool,
    /// Create > Polygon Primitives > Interactive Creation
    interactive_create: bool,
    prim_cmd: String,
    prim_first: Option<Vec3>,
    show_prim_opts: bool,
    prim_opts: PrimOpts,
    show_prefs: bool,
    show_hotkeys: bool,
    hotkey_new: (String, String),
    show_project: bool,
    show_refs: bool,
    show_member: bool,
    show_flexor: bool,
    flexor_pos: bool,
    flexor_divs: [u32; 3],
    flexor_msg: String,
    member_node: String,
    member_value: f32,
    member_msg: String,
    saved_layouts: Vec<(String, LayoutMode, ViewKind)>,
    ref_path: String,
    ref_ns: String,
    ref_msg: String,
    project_path: String,
    project_msg: String,
    hotbox: Option<Pos2>,
    space_prev: bool,
    space_at: f64,
    last_autosave: std::time::Instant,
    ui_status: bool,
    ui_shelf: bool,
    ui_toolbox: bool,
    ui_timeslider: bool,
    ui_rangeslider: bool,
    ui_cmdline: bool,
    ui_helpline: bool,
    uv_zone: u16,
    uv_tex: Option<(String, TextureHandle)>,
    uv_ui: UvUi,
    nurbs_ui: NurbsUi,
    dope: Option<DopeEd>,
    skin_ui: SkinUi,
    pfx_ui: PfxUi,
    sculpt_ui: SculptUi,
    script_ui: ScriptUi,
    mm_ui: MmUi,
    expr_ui: ExprUi,
    popup_at: Option<Pos2>,
    trax_ui: TraxUi,
    event_ui: EventUi,
    joint_last: Option<NodeId>,
    ik_start: Option<NodeId>,
    render_job: Option<RenderJob>,
    pipe_job: Option<PipeJob>,
    batch_job: Option<(std::process::Child, std::time::Instant)>,
    rset_ui: RsetUi,
    fcheck_ui: FcheckUi,
    res_gate: bool,
    last_title: String,
    show_shapes: bool,
    collapsed: BTreeSet<NodeId>,
    outliner_drag: Option<NodeId>,
    name_buf: (NodeId, String),
    name_editing: bool,
    key_sel: Vec<f32>,
    ts_menu_frame: f32,
    key_drag: Option<(f32, f32)>,
}

/// Locate examples/demo.mel: current folder, next to the executable, or the per-user install folder.
fn demo_script_path() -> Option<String> {
    let mut candidates: Vec<PathBuf> = vec![PathBuf::from("examples/demo.mel")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("examples/demo.mel"));
            candidates.push(dir.join("../examples/demo.mel"));
            candidates.push(dir.join("../../examples/demo.mel"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/share/aztec2009/examples/demo.mel"));
    }
    candidates.into_iter().find(|p| p.is_file()).map(|p| p.to_string_lossy().to_string())
}

fn nice_name(attr: &str) -> String {
    let mut out = String::new();
    for (i, ch) in attr.chars().enumerate() {
        if i == 0 {
            out.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            out.push(' ');
            out.push(ch);
        } else {
            out.push(ch);
        }
    }
    out
}

fn v3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

impl App {
    pub fn new(mut session: Session) -> App {
        if session.scene.find("persp").is_none() {
            session.scene.add_default_camera();
        }
        session.view.apply(&mut session.scene);
        let last_state = serialize::to_ascii(&session.scene);
        let mut app = App {
            session,
            tool: Tool::Select,
            layout: LayoutMode::Single,
            single_kind: ViewKind::Persp,
            active: ViewKind::Persp,
            panels: Default::default(),
            ortho: [Ortho::default(); 3],
            gen: 1,
            frame_gen: 0,
            interacting: false,
            drag: Drag::None,
            shelf_tab: 1,
            mel_input: String::new(),
            mel_result: String::new(),
            mel_error: false,
            log: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_state,
            scene_edits: false,
            auto_key: false,
            show_outliner: false,
            show_channels: true,
            show_script: false,
            show_help: false,
            show_about: false,
            show_layers: true,
            help_line: String::new(),
            rename: None,
            file_dialog: None,
            dup_special: None,
            sdk: None,
            constrain_offset: false,
            graph: None,
            node_ed: NodeEd { attr_keyable: true, attr_default: "0".to_string(), ..Default::default() },
            render_view: None,
            stereo_scale: 1.0,
            rs_cache: None,
            menu_set: MenuSet::Polygons,
            last_comp_mode: SelMode::Face,
            hud_poly: true,
            axis_normal: false,
            split_pts: Vec::new(),
            loop_drag: None,
            last_op: None,
            poly_stats: None,
            curve_win: None,
            show_ae: false,
            ae_tab: None,
            show_uv: false,
            pivot_edit: false,
            interactive_create: false,
            prim_cmd: String::new(),
            prim_first: None,
            show_prim_opts: false,
            prim_opts: PrimOpts::default(),
            show_prefs: false,
            show_hotkeys: false,
            hotkey_new: (String::new(), String::new()),
            show_project: false,
            show_refs: false,
            show_member: false,
            show_flexor: false,
            flexor_pos: true,
            flexor_divs: [2, 5, 2],
            flexor_msg: String::new(),
            member_node: String::new(),
            member_value: 1.0,
            member_msg: String::new(),
            saved_layouts: Vec::new(),
            ref_path: String::new(),
            ref_ns: String::new(),
            ref_msg: String::new(),
            project_path: String::new(),
            project_msg: String::new(),
            hotbox: None,
            space_prev: false,
            space_at: 0.0,
            last_autosave: std::time::Instant::now(),
            ui_status: true,
            ui_shelf: true,
            ui_toolbox: true,
            ui_timeslider: true,
            ui_rangeslider: true,
            ui_cmdline: true,
            ui_helpline: true,
            uv_zone: 0,
            uv_tex: None,
            uv_ui: UvUi { show_image: true, ..Default::default() },
            nurbs_ui: NurbsUi { degree: 3, ..Default::default() },
            dope: None,
            skin_ui: SkinUi { value: 1.0, radius: 0.6, strength: 0.6, ..Default::default() },
            pfx_ui: PfxUi::default(),
            sculpt_ui: SculptUi::default(),
            script_ui: ScriptUi { file: "myScript.mel".to_string(), ..Default::default() },
            mm_ui: MmUi::default(),
            expr_ui: ExprUi::default(),
            popup_at: None,
            trax_ui: TraxUi::default(),
            event_ui: EventUi { num: 3, spread: 1.0, inherit: 1.0, dies: true, ..Default::default() },
            joint_last: None,
            ik_start: None,
            render_job: None,
            pipe_job: None,
            batch_job: None,
            rset_ui: RsetUi::default(),
            fcheck_ui: FcheckUi::default(),
            res_gate: false,
            last_title: String::new(),
            show_shapes: false,
            collapsed: BTreeSet::new(),
            outliner_drag: None,
            name_buf: (NodeId { index: u32::MAX, gen: 0 }, String::new()),
            name_editing: false,
            key_sel: Vec::new(),
            ts_menu_frame: 1.0,
            key_drag: None,
        };
        let has_geo = {
            let rs = viewport::scene_snapshot(&mut app.session.scene);
            !rs.items.is_empty()
        };
        if has_geo {
            app.frame(false);
        }
        app.log.push(("Aztec 2009 ready. Type commands in the command line, or use the menus and shelf.".to_string(), false));
        app
    }

    // ------------------------------------------------------------------ state helpers

    fn mark_dirty(&mut self) {
        self.gen += 1;
    }

    fn default_interp(&self) -> crate::anim::Interp {
        crate::anim::Interp::parse(&self.session.prefs.key_tangent).unwrap_or(crate::anim::Interp::Bezier)
    }

    fn scene_edited(&mut self) {
        self.scene_edits = true;
        self.mark_dirty();
    }

    fn say(&mut self, msg: &str) {
        self.mel_result = msg.to_string();
        self.mel_error = false;
        self.log.push((format!("// {}", msg), false));
    }

    fn exec(&mut self, cmd: &str) {
        let o = execute_line(&mut self.session, cmd);
        self.ipr_after_command();
        // undo / redo typed as commands
        match std::mem::take(&mut self.session.undo_request) {
            -1 => self.do_undo(),
            1 => self.do_redo(),
            _ => {}
        }
        self.log.push((format!("{}", cmd), false));
        // one line of output is a result, like Maya's "// Result: pSphere1 //"
        let first = cmd.trim_start().split(|c: char| c.is_whitespace() || c == ';').next().unwrap_or("");
        if o.lines.len() == 1 && !matches!(first, "print" | "echo" | "help" | "about") && !o.lines[0].starts_with("//") {
            self.log.push((format!("// Result: {} //", o.lines[0]), false));
        } else {
            for l in &o.lines {
                self.log.push((l.clone(), false));
            }
        }
        if let Some(l) = o.lines.last() {
            self.mel_result = l.clone();
            self.mel_error = false;
        }
        if let Some(e) = o.error {
            self.mel_result = format!("// Error: {}", e);
            self.mel_error = true;
            self.log.push((self.mel_result.clone(), true));
        }
        if self.log.len() > 3000 {
            let n = self.log.len() - 3000;
            self.log.drain(..n);
        }
        if self.session.quit {
            self.session.quit = false;
        }
        self.after_command(cmd);
    }

    fn after_command(&mut self, cmd: &str) {
        let c = cmd.trim_start();
        if c.starts_with("file") || c.starts_with("source") || c.starts_with("select") {
            if c.starts_with("file") || c.starts_with("source") {
                self.session.comp.mode = SelMode::Object;
                self.session.comp.node = None;
                self.session.comp.sel = Sel::default();
            }
        }
        if c.starts_with("file") || c.starts_with("source") {
            if self.session.scene.find("persp").is_none() {
                self.session.scene.add_default_camera();
            }
            self.session.resync_view();
        }
        self.scene_edited();
    }

    /// Record an undo step when the scene differs from the last committed state.
    fn commit(&mut self) {
        if !self.scene_edits {
            return;
        }
        self.scene_edits = false;
        let now = serialize::to_ascii(&self.session.scene);
        if now != self.last_state {
            let prev = std::mem::replace(&mut self.last_state, now);
            self.undo.push(prev);
            let limit = if self.session.prefs.undo_limit == 0 { usize::MAX } else { self.session.prefs.undo_limit };
            while self.undo.len() > limit {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn restore(&mut self, text: &str) {
        match serialize::from_ascii(text) {
            Ok(mut scene) => {
                let names: Vec<String> = self.session.scene.selection.iter().map(|i| self.session.scene.name_of(*i)).collect();
                let cur = self.session.scene.timeline.current;
                scene.file_path = self.session.scene.file_path.clone();
                if scene.find("persp").is_none() {
                    scene.add_default_camera();
                }
                self.session.scene = scene;
                self.session.comp.mode = SelMode::Object;
                self.session.comp.node = None;
                self.session.comp.sel = Sel::default();
                self.session.view.apply(&mut self.session.scene);
                self.session.scene.set_time(cur);
                self.session.scene.selection = names.iter().filter_map(|n| self.session.scene.find(n)).collect();
                self.last_state = serialize::to_ascii(&self.session.scene);
                self.mark_dirty();
            }
            Err(e) => self.say(&format!("Cannot restore state: {}", e)),
        }
    }

    fn do_undo(&mut self) {
        self.commit();
        if let Some(prev) = self.undo.pop() {
            let cur = std::mem::replace(&mut self.last_state, String::new());
            self.redo.push(cur);
            self.restore(&prev);
            self.say("Undo");
        }
    }

    fn do_redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let cur = self.last_state.clone();
            self.undo.push(cur);
            self.restore(&next);
            self.say("Redo");
        }
    }

    fn sel_names(&self) -> Vec<String> {
        self.session.scene.selection.iter().map(|i| self.session.scene.name_of(*i)).collect()
    }

    /// Selected transforms (shapes resolve to their transform).
    fn selected_transforms(&self) -> Vec<NodeId> {
        let sc = &self.session.scene;
        let mut v = Vec::new();
        for id in &sc.selection {
            let node = match sc.node(*id) {
                Some(n) => n,
                None => continue,
            };
            let t = if node.data.is_transform() { *id } else { node.parent.unwrap_or(*id) };
            if sc.node(t).map(|n| n.data.is_transform()).unwrap_or(false) && !v.contains(&t) {
                v.push(t);
            }
        }
        v
    }

    fn key_selected(&mut self) {
        if self.session.comp.mode != SelMode::Object {
            let c = &self.session.comp;
            if let Some(shape) = c.node {
                if let Some((poly, _)) = self.mesh_poly(shape) {
                    let verts: Vec<u32> = cmd_model::verts_of(&poly, &c.sel, c.mode).into_iter().collect();
                    if verts.is_empty() {
                        self.say("Select vertices to key");
                        return;
                    }
                    let layer = self.session.scene.anim.active_layer;
                    match self.session.scene.key_vertices(shape, &verts, layer) {
                        Ok(n) => {
                            let f = self.session.scene.timeline.frame();
                            self.say(&format!("Keyed {} vertices ({} channels) at frame {}", verts.len(), n, f));
                            self.scene_edited();
                        }
                        Err(e) => self.say(&e),
                    }
                    return;
                }
            }
        }
        // a current character set takes the key instead of the selection
        let interp = self.default_interp();
        if let Some(n) = crate::mel::cmd_char::key_current(&mut self.session, interp) {
            let f = self.session.scene.timeline.frame();
            self.say(&format!("Keyed the character set: {} channels at frame {}", n, f));
            self.scene_edited();
            return;
        }
        let ts = self.selected_transforms();
        if ts.is_empty() {
            self.say("Select an object first");
            return;
        }
        let layer = self.session.scene.anim.active_layer;
        let mut n = 0;
        for id in ts {
            for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"] {
                if self.session.scene.set_keyframe_noeval(id, a, None, None, self.default_interp(), layer).is_ok() {
                    n += 1;
                }
            }
        }
        self.session.scene.evaluate_anim();
        let f = self.session.scene.timeline.frame();
        self.say(&format!("Keyed {} channels at frame {}", n, f));
        self.scene_edited();
    }

    fn cut_key(&mut self) {
        let t = self.session.scene.timeline.current;
        let layers = self.session.scene.anim.layers.len();
        let mut n = 0;
        for id in self.selected_transforms() {
            for a in ["translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ", "scaleX", "scaleY", "scaleZ"] {
                for l in 0..layers {
                    if self.session.scene.anim.remove_key(id, a, l, t) {
                        n += 1;
                    }
                }
            }
        }
        self.session.scene.evaluate_frame();
        self.say(&format!("Removed {} keys", n));
        self.scene_edited();
    }

    fn set_time(&mut self, t: f32) {
        self.session.scene.set_time(t);
        self.mark_dirty();
    }

    fn step_frame(&mut self, d: f32) {
        let tl = &self.session.scene.timeline;
        let t = (tl.current + d).clamp(tl.start, tl.end);
        self.set_time(t);
    }

    fn set_mode(&mut self, m: DisplayMode) {
        self.session.view.mode = m;
        self.mark_dirty();
    }

    fn frame(&mut self, selected: bool) {
        let rs = viewport::scene_snapshot(&mut self.session.scene);
        let b = bounds(&rs, selected).or_else(|| bounds(&rs, false));
        let (lo, hi) = match b {
            Some(b) => b,
            None => (Vec3::splat(-5.0), Vec3::splat(5.0)),
        };
        let center = (lo + hi) * 0.5;
        let radius = ((hi - lo).length() * 0.5).max(0.5);
        match self.active_kind() {
            ViewKind::Persp => {
                let v = &mut self.session.view;
                v.camera = None;
                v.target = center;
                v.dist = (radius * 2.8).clamp(2.0, 500.0);
                v.apply(&mut self.session.scene);
            }
            k => {
                let o = &mut self.ortho[k as usize - 1];
                o.target = center;
                o.size = (radius * 1.3).clamp(0.5, 3000.0);
            }
        }
        self.mark_dirty();
    }

    fn active_kind(&self) -> ViewKind {
        if self.layout == LayoutMode::Four {
            self.active
        } else {
            self.single_kind
        }
    }

    fn open_file_dialog(&mut self, save: bool) {
        let dir = self
            .session
            .scene
            .file_path
            .as_ref()
            .and_then(|p| PathBuf::from(p).parent().map(|d| d.to_path_buf()))
            .filter(|d| d.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let name = self
            .session
            .scene
            .file_path
            .as_ref()
            .and_then(|p| PathBuf::from(p).file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "scene.aztec".to_string());
        // inside a project the dialogs start in its scenes folder
        let dir = match (&self.session.scene.project_dir, self.session.scene.file_path.is_some()) {
            (Some(d), false) if d.join("scenes").is_dir() => d.join("scenes"),
            _ => dir,
        };
        self.file_dialog = Some(FileDialog { io: 0, tex_for: None, tex_faces: None, image_plane: false, save, dir, name });
    }

    /// Import (mode 1), export all (2) or export selection (3) of OBJ / FBX files.
    fn open_io_dialog(&mut self, mode: u8) {
        self.open_file_dialog(mode != 1);
        if let Some(fd) = &mut self.file_dialog {
            fd.io = mode;
            if mode != 1 {
                let stem = self.session.scene.file_path.as_ref().and_then(|p| PathBuf::from(p).file_stem().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "export".to_string());
                fd.name = format!("{}.fbx", stem);
            }
        }
    }

    fn quote(path: &str) -> String {
        format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
    }

    // ------------------------------------------------------------------ rendering a still

    fn render_still(&mut self) {
        let (res, anaglyph, ray, quality) = match &self.render_view {
            Some(r) => (r.res, r.anaglyph, r.ray, r.quality),
            None => (1, false, false, 1),
        };
        let (w, h) = [(640, 360), (960, 540), (1280, 720), (1920, 1080)][res.min(3)];
        let mut rs = viewport::scene_snapshot(&mut self.session.scene);
        rs.use_textures = true;
        let view = self.session.view.clone();
        let cam = match make_cam(&mut self.session.scene, &view, ViewKind::Persp, &Ortho::default(), w as f32 / h as f32) {
            Some(c) => c,
            None => {
                self.say("The scene has no camera");
                return;
            }
        };
        let frame = self.session.scene.timeline.frame();
        let rv = self.render_view.get_or_insert(RenderView {
            tex: None,
            w,
            h,
            rgba: Vec::new(),
            res,
            anaglyph,
            ray,
            quality,
            save_name: format!("render_{:04}.png", frame),
            note: String::new(),
            pipe: true,
            show_alpha: false,
            kept: Vec::new(),
            kept_view: None,
            region: None,
            auto_region: false,
            ipr: false,
            open_path: String::new(),
        });
        rv.w = w;
        rv.h = h;
        rv.save_name = format!("render_{:04}.png", frame);
        if rv.pipe && !anaglyph {
            let region = rv.region;
            self.start_pipe_render(region);
            return;
        }
        let rv = self.render_view.as_mut().unwrap();
        if ray && !anaglyph {
            if self.render_job.is_some() {
                return;
            }
            rv.note = "Raytracing...".to_string();
            let progress = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let p2 = progress.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let (v, pj) = (cam.view, cam.proj);
            let grid = quality.clamp(1, 4);
            std::thread::spawn(move || {
                let opt = crate::render::raytrace::Options { grid, ..Default::default() };
                let pix = crate::render::raytrace::raytrace(&rs, &v, &pj, w, h, &opt, Some(&p2));
                let mut rgba = vec![255u8; w * h * 4];
                for (i, c) in pix.iter().enumerate() {
                    rgba[i * 4] = (c.x * 255.0) as u8;
                    rgba[i * 4 + 1] = (c.y * 255.0) as u8;
                    rgba[i * 4 + 2] = (c.z * 255.0) as u8;
                }
                let _ = tx.send(rgba);
            });
            self.render_job = Some(RenderJob { rx, progress, w, h, frame, started: std::time::Instant::now() });
            return;
        }
        let mode = if anaglyph { DisplayMode::Anaglyph } else { DisplayMode::Shaded };
        let rgba = viewport::render(&rs, &cam, mode, false, false, false, self.stereo_scale, w, h, 2).rgba;
        rv.rgba = rgba;
        rv.tex = None;
        rv.note = format!("Rendered frame {} at {}x{}", frame, w, h);
    }

    fn poll_render_job(&mut self, ctx: &egui::Context) {
        let done = match &self.render_job {
            Some(j) => match j.rx.try_recv() {
                Ok(rgba) => Some(Some(rgba)),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    None
                }
                Err(_) => Some(None),
            },
            None => return,
        };
        if let Some(result) = done {
            let job = self.render_job.take().unwrap();
            if let (Some(rgba), Some(rv)) = (result, &mut self.render_view) {
                rv.rgba = rgba;
                rv.w = job.w;
                rv.h = job.h;
                rv.tex = None;
                rv.note = format!("Raytraced frame {} at {}x{} in {:.1}s", job.frame, job.w, job.h, job.started.elapsed().as_secs_f32());
            }
        }
    }
}

/// Render the perspective view of a session to a PNG without opening a window (used by `--snapshot`).
pub fn snapshot_png(session: &mut Session, path: &str, w: usize, h: usize) -> Result<(), String> {
    session.view.apply(&mut session.scene);
    let rs = viewport::scene_snapshot(&mut session.scene);
    let view = session.view.clone();
    let cam = make_cam(&mut session.scene, &view, ViewKind::Persp, &Ortho::default(), w as f32 / h as f32).ok_or("no camera")?;
    let rgba = viewport::render(&rs, &cam, view.mode, view.grid, true, view.wire_on_shaded, 1.0, w, h, 2).rgba;
    std::fs::write(path, png::encode_rgba(w, h, &rgba)).map_err(|e| e.to_string())
}

// ====================================================================== widgets

fn icon_button(ui: &mut Ui, icon: Icon, selected: bool, tip: &str, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hovered = resp.hovered();
    let bg = if selected {
        Color32::from_rgb(0xe6, 0xd2, 0x8c)
    } else if hovered {
        Color32::from_rgb(0xe2, 0xe2, 0xe2)
    } else {
        Color32::from_rgb(0xc4, 0xc4, 0xc4)
    };
    let stroke = if selected { Stroke::new(1.5_f32, Color32::from_rgb(0xa0, 0x70, 0x10)) } else { Stroke::new(1.0_f32, Color32::from_rgb(0x6c, 0x6c, 0x6c)) };
    ui.painter().rect(rect, 2.0, bg, stroke, eframe::egui::StrokeKind::Inside);
    icons::draw(ui.painter(), rect.shrink(size * 0.17), icon);
    resp.on_hover_text(tip)
}

fn text_button(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let b = egui::Button::new(label).selected(selected);
    ui.add(b)
}

fn separator_v(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(6.0, 20.0), Sense::hover());
    let x = rect.center().x;
    ui.painter().line_segment([pos2(x, rect.top() + 1.0), pos2(x, rect.bottom() - 1.0)], Stroke::new(1.0_f32, CHROME_DARK));
    ui.painter().line_segment([pos2(x + 1.0, rect.top() + 1.0), pos2(x + 1.0, rect.bottom() - 1.0)], Stroke::new(1.0_f32, CHROME_LIGHT));
}

fn node_icon(data: &NodeData) -> (&'static str, Color32) {
    match data {
        NodeData::Transform => ("\u{25A1}", Color32::from_rgb(0x30, 0x30, 0x30)),
        NodeData::Mesh(_) => ("\u{25B3}", Color32::from_rgb(0x30, 0x80, 0x30)),
        NodeData::Camera => ("\u{25A3}", Color32::from_rgb(0x30, 0x70, 0xa0)),
        NodeData::StereoCamera => ("\u{25A3}", Color32::from_rgb(0xa0, 0x40, 0xa0)),
        NodeData::Light => ("\u{2600}", Color32::from_rgb(0xb0, 0x90, 0x10)),
        NodeData::Material => ("\u{25CF}", Color32::from_rgb(0xc0, 0x60, 0x30)),
        NodeData::Emitter => ("\u{2735}", Color32::from_rgb(0xd0, 0x80, 0x20)),
        NodeData::Locator => ("\u{271B}", Color32::from_rgb(0xb0, 0x90, 0x10)),
        NodeData::Constraint(_) => ("\u{26D3}", Color32::from_rgb(0x90, 0x50, 0xd0)),
        NodeData::ObjectSet(_) => ("\u{25CB}", Color32::from_rgb(0x30, 0x90, 0x80)),
        NodeData::Texture(_) => ("\u{25A6}", Color32::from_rgb(0xd0, 0x80, 0x30)),
        NodeData::NurbsCurve(_) => ("\u{223F}", Color32::from_rgb(0x50, 0xc0, 0xf0)),
        NodeData::MotionPath(_) => ("\u{2933}", Color32::from_rgb(0x60, 0xd0, 0x60)),
        NodeData::Brush(_) => ("\u{270e}", Color32::from_rgb(0x60, 0xd0, 0x80)),
        NodeData::RenderGlobals(_) => ("\u{1f3ac}", Color32::from_rgb(0xd0, 0xd0, 0x70)),
        NodeData::RigidBody(_) => ("\u{25a3}", Color32::from_rgb(0xe0, 0xa0, 0x60)),
        NodeData::DynSolver(_) => ("\u{2699}", Color32::from_rgb(0xe0, 0xa0, 0x60)),
        NodeData::Field(_) => ("\u{21e3}", Color32::from_rgb(0x70, 0xb0, 0xf0)),
        NodeData::RigidJoint(_) => ("\u{2693}", Color32::from_rgb(0xe0, 0xa0, 0x60)),
        NodeData::PEmitter(_) => ("\u{2735}", Color32::from_rgb(0xf0, 0xa0, 0x40)),
        NodeData::Particle(_) => ("\u{2234}", Color32::from_rgb(0x70, 0xc8, 0xf0)),
        NodeData::Lattice(_) => ("\u{25A4}", Color32::from_rgb(0xf0, 0x90, 0xd0)),
        NodeData::Character(_) => ("\u{263A}", Color32::from_rgb(0xf0, 0xc0, 0x70)),
        NodeData::Distance(_) => ("\u{2194}", Color32::from_rgb(0xc8, 0xc8, 0x70)),
        NodeData::Sound(_) => ("\u{266B}", Color32::from_rgb(0xe0, 0xc0, 0x60)),
        NodeData::DrivenCurve(_) => ("\u{223F}", Color32::from_rgb(0x90, 0x50, 0xd0)),
        NodeData::Nucleus(_) => ("\u{2736}", Color32::from_rgb(0x40, 0x60, 0xc0)),
        NodeData::PolyOp(_) => ("\u{25AA}", Color32::from_rgb(0x70, 0x70, 0x70)),
        NodeData::Joint => ("\u{25C6}", Color32::from_rgb(0x30, 0xa0, 0x20)),
        NodeData::IkHandle(_) => ("\u{2726}", Color32::from_rgb(0x20, 0x90, 0xc0)),
    }
}

// ====================================================================== UI

impl App {
    fn menu_bar(&mut self, ui: &mut Ui) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("New Scene").shortcut_text("Ctrl+N")).clicked() {
                    self.exec("file -new");
                    self.session.view = ViewState::new();
                    self.session.view.apply(&mut self.session.scene);
                    self.undo.clear();
                    self.redo.clear();
                    self.last_state = serialize::to_ascii(&self.session.scene);
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Open Scene...").shortcut_text("Ctrl+O")).clicked() {
                    self.open_file_dialog(false);
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Save Scene").shortcut_text("Ctrl+S")).clicked() {
                    if self.session.scene.file_path.is_some() {
                        self.exec("file -save");
                    } else {
                        self.open_file_dialog(true);
                    }
                    ui.close_menu();
                }
                if ui.button("Save Scene As...").clicked() {
                    self.open_file_dialog(true);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Import (OBJ, FBX)...").clicked() {
                    self.open_io_dialog(1);
                    ui.close_menu();
                }
                if ui.button("Export All (OBJ, FBX)...").clicked() {
                    self.open_io_dialog(2);
                    ui.close_menu();
                }
                if ui.add_enabled(!self.session.scene.selection.is_empty(), egui::Button::new("Export Selection (OBJ, FBX)...")).clicked() {
                    self.open_io_dialog(3);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Create Reference...").on_hover_text("Bring another scene in under a namespace").clicked() {
                    self.show_refs = true;
                    ui.close_menu();
                }
                if ui.button("Reference Editor").clicked() {
                    self.show_refs = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Project...").on_hover_text("Set or create a project folder (scenes, images, sourceimages ...)").clicked() {
                    self.show_project = true;
                    if self.project_path.is_empty() {
                        self.project_path = self.session.scene.project_dir.as_ref().map(|d| d.to_string_lossy().to_string()).unwrap_or_else(|| std::env::current_dir().map(|d| d.to_string_lossy().to_string()).unwrap_or_default());
                    }
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Run Demo Script").clicked() {
                    match demo_script_path() {
                        Some(p) => self.exec(&format!("source {}", Self::quote(&p))),
                        None => self.say("examples/demo.mel was not found next to the program"),
                    }
                    ui.close_menu();
                }
                ui.separator();
                if ui.add(egui::Button::new("Exit").shortcut_text("Ctrl+Q")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(!self.undo.is_empty() || self.scene_edits, egui::Button::new("Undo").shortcut_text("Ctrl+Z")).clicked() {
                    self.do_undo();
                    ui.close_menu();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo").shortcut_text("Ctrl+Y")).clicked() {
                    self.do_redo();
                    ui.close_menu();
                }
                ui.separator();
                if ui.add(egui::Button::new("Duplicate").shortcut_text("Ctrl+D")).clicked() {
                    self.exec("duplicate");
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Group").shortcut_text("Ctrl+G")).clicked() {
                    self.exec("group");
                    ui.close_menu();
                }
                if ui.button("Ungroup").clicked() {
                    self.exec("ungroup");
                    ui.close_menu();
                }
                if ui.button("Duplicate Special...").clicked() {
                    self.dup_special = Some(DupSpecial { copies: 3, t: [2.0, 0.0, 0.0], r: [0.0; 3], s: [1.0; 3], group: false, instance: false });
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Delete").shortcut_text("Del")).clicked() {
                    self.exec("delete");
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Parent").shortcut_text("P")).clicked() {
                    self.parent_selection();
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Unparent").shortcut_text("Shift+P")).clicked() {
                    self.unparent_selection();
                    ui.close_menu();
                }
                if ui.button("Select Hierarchy").clicked() {
                    self.select_hierarchy();
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Rename...")).clicked() {
                    if let Some(id) = self.session.scene.selection.first().copied() {
                        let n = self.session.scene.name_of(id);
                        self.rename = Some((id, n));
                        self.show_outliner = true;
                    }
                    ui.close_menu();
                }
                ui.menu_button("Delete All by Type", |ui| {
                    for (l, c) in [("History", "deleteHistory"), ("Non-Deformer History", "delete -nd"), ("Static Channels", "delete -sc"), ("Animation Channels", "delete -c"), ("Constraints", "delete -cn")] {
                        if ui.button(l).clicked() {
                            self.exec(c);
                            ui.close_menu();
                        }
                    }
                });
                if ui.button("Delete Unused Nodes").on_hover_text("Materials and curve nodes nothing uses").clicked() {
                    self.exec("deleteUnusedNodes");
                    ui.close_menu();
                }
                ui.menu_button("Select All by Type", |ui| {
                    for (l, c) in [("Polygon Geometry", "polygon"), ("Lights", "light"), ("Cameras", "camera"), ("Joints", "joint"), ("IK Handles", "ikHandle"), ("Locators", "locator"), ("Emitters", "emitter"), ("Constraints", "constraint")] {
                        if ui.button(l).clicked() {
                            self.exec(&format!("selectAllByType {}", c));
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                if ui.button("Select All").clicked() {
                    self.exec("select -all");
                    ui.close_menu();
                }
                if ui.button("Deselect").clicked() {
                    self.exec("select -cl");
                    ui.close_menu();
                }
            });
            ui.menu_button("Modify", |ui| {
                for (t, l, k) in [(Tool::Move, "Move Tool", "W"), (Tool::Rotate, "Rotate Tool", "E"), (Tool::Scale, "Scale Tool", "R")] {
                    if ui.add(egui::Button::new(l).shortcut_text(k).selected(self.tool == t)).clicked() {
                        self.tool = t;
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Make Live").on_hover_text("Curves drawn with the curve tools snap to this surface").clicked() {
                    self.exec("makeLive");
                    ui.close_menu();
                }
                if ui.button("Make Not Live").clicked() {
                    self.exec("makeLive -off");
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Freeze Transformations").on_hover_text("Bake translate / rotate / scale into the geometry").clicked() {
                    self.exec("makeIdentity -apply true");
                    ui.close_menu();
                }
                if ui.button("Center Pivot").on_hover_text("Put the pivot in the middle of the selected objects").clicked() {
                    self.exec("xform -cp");
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Edit Pivot").shortcut_text("Insert").selected(self.pivot_edit)).clicked() {
                    self.pivot_edit = !self.pivot_edit;
                    ui.close_menu();
                }
                if ui.button("Reset Pivot").on_hover_text("Pivot back to the object's origin").clicked() {
                    self.exec("xform -piv 0 0 0");
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Reset Transformations").clicked() {
                    for n in self.sel_names() {
                        self.exec(&format!("setAttr \"{}.translate\" 0 0 0", n));
                        self.exec(&format!("setAttr \"{}.rotate\" 0 0 0", n));
                        self.exec(&format!("setAttr \"{}.scale\" 1 1 1", n));
                    }
                    ui.close_menu();
                }
                let mats: Vec<String> = {
                    let sc = &self.session.scene;
                    sc.ids_where(|d| matches!(d, NodeData::Material)).iter().map(|i| sc.name_of(*i)).collect()
                };
                ui.add_enabled_ui(!mats.is_empty() && !self.session.scene.selection.is_empty(), |ui| {
                    ui.menu_button("Assign Material", |ui| {
                        for m in &mats {
                            if ui.button(m).clicked() {
                                if let Some(c) = self.face_component() {
                                    self.exec(&format!("assignMaterial \"{}\" \"{}\"", m, c));
                                } else {
                                    for n in self.sel_names() {
                                        self.exec(&format!("assignMaterial \"{}\" \"{}\"", m, n));
                                    }
                                }
                                ui.close_menu();
                            }
                        }
                    });
                });
            });
            ui.menu_button("Create", |ui| {
                ui.menu_button("Polygon Primitives", |ui| {
                    ui.checkbox(&mut self.interactive_create, "Interactive Creation").on_hover_text("Click two points on the grid to size the new primitive");
                    if ui.button("Options...").on_hover_text("Create a primitive with exact sizes and subdivisions").clicked() {
                        self.show_prim_opts = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("Platonic Solids", |ui| {
                        for k in ["tetrahedron", "cube", "octahedron", "dodecahedron", "icosahedron"] {
                            if ui.button(k).clicked() {
                                self.exec(&format!("polyPlatonicSolid -type {}", k));
                                ui.close_menu();
                            }
                        }
                    });
                    for (l, c) in PRIM_KINDS {
                        if ui.button(l).clicked() {
                            if self.interactive_create {
                                self.prim_cmd = c.to_string();
                                self.prim_first = None;
                                self.tool = Tool::Prim;
                            } else {
                                self.exec(c);
                            }
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("NURBS Primitives", |ui| {
                    for (l, c) in [("Sphere", "sphere"), ("Cylinder", "cylinder"), ("Cone", "cone"), ("Torus", "torus"), ("Plane", "nurbsPlane"), ("Circle", "circle"), ("Square", "nurbsSquare")] {
                        if ui.button(l).clicked() {
                            self.exec(c);
                            ui.close_menu();
                        }
                    }
                });
                if ui.button("CV Curve Tool").on_hover_text("Click control vertices, Enter creates the curve").clicked() {
                    self.nurbs_ui.ep = false;
                    self.nurbs_ui.pts.clear();
                    self.start_tool(Tool::Curve);
                    ui.close_menu();
                }
                if ui.button("EP Curve Tool").on_hover_text("Click edit points the curve passes through, Enter creates the curve").clicked() {
                    self.nurbs_ui.ep = true;
                    self.nurbs_ui.pts.clear();
                    self.start_tool(Tool::Curve);
                    ui.close_menu();
                }
                ui.menu_button("Lights", |ui| {
                    if ui.button("Point Light").clicked() {
                        self.exec("pointLight");
                        ui.close_menu();
                    }
                    if ui.button("Directional Light").clicked() {
                        self.exec("directionalLight");
                        ui.close_menu();
                    }
                    if ui.button("Spot Light").clicked() {
                        self.exec("spotLight");
                        ui.close_menu();
                    }
                    if ui.button("Ambient Light").clicked() {
                        self.exec("ambientLight");
                        ui.close_menu();
                    }
                    if ui.button("Area Light").clicked() {
                        self.exec("areaLight");
                        ui.close_menu();
                    }
                });
                ui.menu_button("Cameras", |ui| {
                    if ui.button("Camera").clicked() {
                        self.exec("camera");
                        ui.close_menu();
                    }
                    if ui.button("Stereo Camera").clicked() {
                        self.exec("stereoCamera");
                        ui.close_menu();
                    }
                    if ui.button("Camera and Aim").on_hover_text("A camera that looks at an aim node: key both").clicked() {
                        self.exec("cameraAim");
                        ui.close_menu();
                    }
                    if ui.button("Camera, Aim and Up").clicked() {
                        self.exec("cameraAim -up");
                        ui.close_menu();
                    }
                });
                if ui.button("Joint Tool").clicked() {
                    self.start_tool(Tool::Joint);
                    ui.close_menu();
                }
                if ui.button("Locator").clicked() {
                    self.exec("spaceLocator");
                    ui.close_menu();
                }
                ui.menu_button("Sets", |ui| {
                    if ui.button("Quick Select Set...").on_hover_text("A set of the current selection").clicked() {
                        self.exec("sets -name quickSelectSet1");
                        ui.close_menu();
                    }
                });
                ui.menu_button("Measure Tools", |ui| {
                    if ui.button("Distance Tool").on_hover_text("Two locators and a Distance node; read it in the Channel Box").clicked() {
                        self.exec("distanceDimension");
                        ui.close_menu();
                    }
                });
                if ui.button("Image Plane...").clicked() {
                    self.open_file_dialog(false);
                    if let Some(fd) = &mut self.file_dialog {
                        fd.image_plane = true;
                        fd.name = String::new();
                    }
                    ui.close_menu();
                }
                if ui.button("nParticles (Nucleus)").clicked() {
                    self.exec("nParticle");
                    ui.close_menu();
                }
                if ui.button("Lambert Material").clicked() {
                    self.exec("lambert");
                    ui.close_menu();
                }
                if ui.button("Animation Layer").clicked() {
                    self.exec("animLayer layer");
                    ui.close_menu();
                }
            });
            ui.menu_button("Display", |ui| {
                let mut grid = self.session.view.grid;
                if ui.checkbox(&mut grid, "Grid").changed() {
                    self.session.view.grid = grid;
                    self.mark_dirty();
                }
                let mut wos = self.session.view.wire_on_shaded;
                if ui.checkbox(&mut wos, "Wireframe on Shaded").changed() {
                    self.session.view.wire_on_shaded = wos;
                    self.mark_dirty();
                }
                ui.checkbox(&mut self.hud_poly, "Heads Up Display: Poly Count");
                let mut fnm = self.session.scene.show_normals;
                if ui.checkbox(&mut fnm, "Face Normals").on_hover_text("Draw a line along the normal of every face").changed() {
                    self.session.scene.show_normals = fnm;
                    self.mark_dirty();
                }
                let mut bo = self.session.scene.show_borders;
                if ui.checkbox(&mut bo, "Border Edges (show gaps)").on_hover_text("Edges with a face on one side only are drawn in red").changed() {
                    self.session.scene.show_borders = bo;
                    self.mark_dirty();
                }
                ui.separator();
                for (m, l) in [(DisplayMode::Wireframe, "Wireframe"), (DisplayMode::Shaded, "Smooth Shade All"), (DisplayMode::Textured, "Textured (lights + color maps)"), (DisplayMode::Anaglyph, "Stereo Anaglyph (red/cyan)")] {
                    if ui.radio(self.session.view.mode == m, l).clicked() {
                        self.set_mode(m);
                        ui.close_menu();
                    }
                }
                ui.separator();
                ui.menu_button("Show", |ui| self.show_type_items(ui));
                ui.menu_button("Transform Display", |ui| {
                    let mut p = self.session.scene.show.pivots;
                    if ui.checkbox(&mut p, "Rotate / Scale Pivots").on_hover_text("Draw the pivot of the selected objects").changed() {
                        self.session.scene.show.pivots = p;
                        self.mark_dirty();
                    }
                    let mut h = self.session.scene.show.handles;
                    if ui.checkbox(&mut h, "Selection Handles").on_hover_text("Draw a handle at the pivot of every object").changed() {
                        self.session.scene.show.handles = h;
                        self.mark_dirty();
                    }
                });
                ui.menu_button("UI Elements", |ui| {
                    ui.checkbox(&mut self.ui_status, "Status Line");
                    ui.checkbox(&mut self.ui_shelf, "Shelf");
                    ui.checkbox(&mut self.ui_toolbox, "Tool Box");
                    ui.checkbox(&mut self.show_channels, "Channel Box");
                    ui.checkbox(&mut self.show_layers, "Layer Editor");
                    ui.checkbox(&mut self.show_outliner, "Outliner");
                    ui.checkbox(&mut self.ui_timeslider, "Time Slider");
                    ui.checkbox(&mut self.ui_rangeslider, "Range Slider");
                    ui.checkbox(&mut self.ui_cmdline, "Command Line");
                    ui.checkbox(&mut self.ui_helpline, "Help Line");
                    ui.separator();
                    if ui.button("Show All UI Elements").clicked() {
                        self.set_ui_elements(true);
                        ui.close_menu();
                    }
                    if ui.button("Hide All UI Elements").clicked() {
                        self.set_ui_elements(false);
                        ui.close_menu();
                    }
                });
                ui.separator();
                if ui.add(egui::Button::new("Hide Selection").shortcut_text("Ctrl+H")).clicked() {
                    self.exec("hide");
                    ui.close_menu();
                }
                if ui.button("Show Last Hidden").clicked() {
                    self.exec("showHidden -last");
                    ui.close_menu();
                }
                if ui.button("Show All").clicked() {
                    self.exec("showHidden -all");
                    ui.close_menu();
                }
                let iso = !self.session.isolated.is_empty();
                if ui.add(egui::Button::new("Isolate Select").selected(iso)).clicked() {
                    self.exec(&format!("isolateSelect -state {}", if iso { 0 } else { 1 }));
                    ui.close_menu();
                }
                ui.separator();
                if ui.add(egui::Button::new("Frame Selection").shortcut_text("F")).clicked() {
                    self.frame(true);
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Frame All").shortcut_text("A")).clicked() {
                    self.frame(false);
                    ui.close_menu();
                }
            });
            ui.menu_button("Window", |ui| {
                ui.checkbox(&mut self.show_outliner, "Outliner");
                ui.checkbox(&mut self.show_channels, "Channel Box");
                ui.checkbox(&mut self.show_layers, "Layer Editor");
                ui.checkbox(&mut self.trax_ui.show, "Trax Editor");
                ui.checkbox(&mut self.expr_ui.show, "Expression Editor");
                ui.checkbox(&mut self.show_script, "Script Editor");
                ui.checkbox(&mut self.show_ae, "Attribute Editor");
                ui.checkbox(&mut self.show_uv, "UV Texture Editor");
                ui.checkbox(&mut self.uv_ui.show_paint, "3D Paint Tool");
                ui.separator();
                ui.menu_button("Settings/Preferences", |ui| {
                    if ui.button("Reset Settings").on_hover_text("Preferences back to their defaults (not saved until you choose Save Preferences)").clicked() {
                        self.session.prefs = crate::session::Prefs::default();
                        self.layout = LayoutMode::Single;
                        self.mark_dirty();
                        ui.close_menu();
                    }
                    if ui.button("Marking Menu Editor...").clicked() {
                        self.mm_ui.show = true;
                        ui.close_menu();
                    }
                    if ui.button("Preferences...").clicked() {
                        self.show_prefs = true;
                        ui.close_menu();
                    }
                    if ui.button("Hotkey Editor...").clicked() {
                        self.show_hotkeys = true;
                        ui.close_menu();
                    }
                });
                ui.separator();
                if ui.button("Render View").clicked() {
                    if self.render_view.is_none() {
                        self.render_still();
                    }
                    ui.close_menu();
                }
                if ui.button("Graph Editor").on_hover_text("Edit animation curves of the selection").clicked() {
                    self.graph = Some(GraphEd { x0: 0.0, x1: 50.0, y0: -1.0, y1: 10.0, sel: Vec::new(), hidden: Default::default(), all_nodes: false, fit_next: true, drag: None });
                    ui.close_menu();
                }
                ui.separator();
                self.node_ed_menu(ui);
                ui.separator();
                if ui.button("Dope Sheet").on_hover_text("Keys of the selection in rows over time: move, copy, scale").clicked() {
                    self.dope = Some(DopeEd { x0: -2.0, x1: 50.0, sel: Vec::new(), all_nodes: false, drag: None });
                    ui.close_menu();
                }
                if ui.button("Playblast").on_hover_text("Render every frame of the timeline to playblast/frame.NNNN.png").clicked() {
                    self.exec("playblast");
                    ui.close_menu();
                }
                ui.separator();
                ui.menu_button("Settings: Time Unit", |ui| {
                    let fps = self.session.scene.timeline.fps;
                    for (l, u, f) in [("Film (24 fps)", "film", 24.0f32), ("PAL (25 fps)", "pal", 25.0), ("NTSC (30 fps)", "ntsc", 30.0), ("Show (48 fps)", "show", 48.0), ("NTSC Field (60 fps)", "ntscf", 60.0)] {
                        if ui.radio((fps - f).abs() < 0.01, l).clicked() {
                            self.exec(&format!("currentUnit -time {}", u));
                            ui.close_menu();
                        }
                    }
                });
            });
            if self.menu_set == MenuSet::Animation {
            ui.menu_button("Animate", |ui| {
                ui.menu_button("Motion Paths", |ui| {
                    if ui.button("Attach to Motion Path").on_hover_text("Select the object, then the curve").clicked() {
                        self.exec("pathAnimation");
                        ui.close_menu();
                    }
                    if ui.button("Attach and Follow").on_hover_text("The object also turns to face along the path (front axis X, up Y)").clicked() {
                        self.exec("pathAnimation -follow 1 -followAxis x -upAxis y");
                        ui.close_menu();
                    }
                    if ui.button("Attach, Follow and Bank").clicked() {
                        self.exec("pathAnimation -follow 1 -followAxis x -upAxis y -bank 1");
                        ui.close_menu();
                    }
                });
                if ui.add(egui::Button::new("Set Key").shortcut_text("S")).clicked() {
                    self.key_selected();
                    ui.close_menu();
                }
                if ui.button("Delete Key at Current Time").clicked() {
                    self.cut_key();
                    ui.close_menu();
                }
                ui.checkbox(&mut self.auto_key, "Auto Key");
                if ui.button("Set Driven Key...").on_hover_text("Drive one attribute from another with a curve").clicked() {
                    self.sdk = Some(SdkWindow { driver_attr: "translateX".into(), driven_attr: "translateY".into(), ..Default::default() });
                    ui.close_menu();
                }
                if ui.button("Bake Simulation").on_hover_text("Key every frame of the selection's animated channels (folds layers and connections)").clicked() {
                    self.exec("bakeResults");
                    ui.close_menu();
                }
                ui.separator();
                let playing = self.session.scene.timeline.playing;
                if ui.add(egui::Button::new(if playing { "Pause" } else { "Play" }).shortcut_text("Alt+V")).clicked() {
                    self.session.scene.timeline.toggle();
                    self.mark_dirty();
                    ui.close_menu();
                }
                if ui.button("Go to Start").clicked() {
                    let s = self.session.scene.timeline.start;
                    self.set_time(s);
                    ui.close_menu();
                }
                if ui.button("Go to End").clicked() {
                    let e = self.session.scene.timeline.end;
                    self.set_time(e);
                    ui.close_menu();
                }
            });
            }
            if self.menu_set == MenuSet::Animation {
                self.rig_menus(ui);
            }
            if self.menu_set == MenuSet::Dynamics {
                self.dynamics_menus(ui);
            }
            if self.menu_set == MenuSet::Rendering {
            ui.menu_button("Render", |ui| {
                if ui.button("Render Current Frame").clicked() {
                    if let Some(r) = &mut self.render_view {
                        r.anaglyph = false;
                    }
                    self.render_still();
                    ui.close_menu();
                }
                if ui.button("Render Stereo Anaglyph").clicked() {
                    if self.render_view.is_none() {
                        self.render_still();
                    }
                    if let Some(r) = &mut self.render_view {
                        r.anaglyph = true;
                    }
                    self.render_still();
                    ui.close_menu();
                }
                ui.separator();
                let f = self.session.scene.timeline.frame();
                if ui.button("Write ASCII Passes (all)").clicked() {
                    self.exec(&format!("render -mode all -out \"frame_{}_all.txt\"", f));
                    ui.close_menu();
                }
                self.render_menu_extras(ui);
            });
            }
            if self.menu_set == MenuSet::Polygons {
                self.polygon_menus(ui);
            }
            if self.menu_set == MenuSet::Surfaces {
                self.surface_menus(ui);
            }
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard Shortcuts").clicked() {
                    self.show_help = true;
                    ui.close_menu();
                }
                if ui.button("About Aztec 2009").clicked() {
                    self.show_about = true;
                    ui.close_menu();
                }
            });
        });
    }

    /// Check boxes for the object types the viewports draw, plus All / None.
    fn show_type_items(&mut self, ui: &mut Ui) {
        let mut changed = false;
        {
            let s = &mut self.session.scene.show;
            for (flag, label) in [
                (&mut s.polygons, "Polygons"),
                (&mut s.lights, "Lights"),
                (&mut s.cameras, "Cameras"),
                (&mut s.joints, "Joints"),
                (&mut s.ik_handles, "IK Handles"),
                (&mut s.locators, "Locators"),
                (&mut s.emitters, "Emitters"),
                (&mut s.particles, "Particles"),
                (&mut s.manipulators, "Manipulators"),
                (&mut s.deformers, "Deformers"),
            ] {
                changed |= ui.checkbox(flag, label).changed();
            }
        }
        ui.separator();
        let s = &mut self.session.scene.show;
        if ui.button("Show All Types").clicked() {
            (s.polygons, s.lights, s.cameras, s.joints, s.ik_handles, s.locators, s.emitters, s.particles) = (true, true, true, true, true, true, true, true);
            changed = true;
        }
        if ui.button("Hide All Types").clicked() {
            (s.polygons, s.lights, s.cameras, s.joints, s.ik_handles, s.locators, s.emitters, s.particles) = (false, false, false, false, false, false, false, false);
            changed = true;
        }
        if changed {
            self.mark_dirty();
        }
    }

    fn set_ui_elements(&mut self, on: bool) {
        for f in [&mut self.ui_status, &mut self.ui_shelf, &mut self.ui_toolbox, &mut self.show_channels, &mut self.show_layers, &mut self.show_outliner, &mut self.ui_timeslider, &mut self.ui_rangeslider, &mut self.ui_cmdline, &mut self.ui_helpline] {
            *f = on;
        }
    }

    fn status_line(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            egui::ComboBox::from_id_salt("menuset")
                .width(92.0)
                .selected_text(match self.menu_set {
                    MenuSet::Animation => "Animation",
                    MenuSet::Polygons => "Polygons",
                    MenuSet::Surfaces => "Surfaces",
                    MenuSet::Dynamics => "Dynamics",
                    MenuSet::Rendering => "Rendering",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.menu_set, MenuSet::Animation, "Animation  F2");
                    ui.selectable_value(&mut self.menu_set, MenuSet::Polygons, "Polygons  F3");
                    ui.selectable_value(&mut self.menu_set, MenuSet::Surfaces, "Surfaces  F4");
                    ui.selectable_value(&mut self.menu_set, MenuSet::Dynamics, "Dynamics  F5");
                    ui.selectable_value(&mut self.menu_set, MenuSet::Rendering, "Rendering  F6");
                });
            separator_v(ui);
            if text_button(ui, "New", false).clicked() {
                self.exec("file -new");
                self.session.view = ViewState::new();
                self.session.view.apply(&mut self.session.scene);
                self.undo.clear();
                self.redo.clear();
                self.last_state = serialize::to_ascii(&self.session.scene);
            }
            if text_button(ui, "Open", false).clicked() {
                self.open_file_dialog(false);
            }
            if text_button(ui, "Save", false).clicked() {
                if self.session.scene.file_path.is_some() {
                    self.exec("file -save");
                } else {
                    self.open_file_dialog(true);
                }
            }
            separator_v(ui);
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo")).clicked() {
                self.do_undo();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo")).clicked() {
                self.do_redo();
            }
            separator_v(ui);
            let cm = self.session.comp.mode;
            for (m, l, tip) in [
                (SelMode::Object, "Obj", "Select by object type (F8)"),
                (SelMode::Vertex, "Vtx", "Select by vertex (F9)"),
                (SelMode::Edge, "Edge", "Select by edge (F10)"),
                (SelMode::Face, "Face", "Select by face (F11)"),
            ] {
                if text_button(ui, l, cm == m).on_hover_text(tip).clicked() {
                    self.set_sel_mode(m);
                }
            }
            separator_v(ui);
            let mode = self.session.view.mode;
            if icon_button(ui, Icon::Wire, mode == DisplayMode::Wireframe, "Wireframe (4)", 22.0).clicked() {
                self.set_mode(DisplayMode::Wireframe);
            }
            if icon_button(ui, Icon::Smooth, mode == DisplayMode::Shaded, "Smooth Shade All (5)", 22.0).clicked() {
                self.set_mode(DisplayMode::Shaded);
            }
            if icon_button(ui, Icon::Textured, mode == DisplayMode::Textured, "Textured: lights and color maps (6)", 22.0).clicked() {
                self.set_mode(DisplayMode::Textured);
            }
            if icon_button(ui, Icon::Anaglyph, mode == DisplayMode::Anaglyph, "Stereo anaglyph (7)", 22.0).clicked() {
                self.set_mode(DisplayMode::Anaglyph);
            }
            if icon_button(ui, Icon::Grid, self.session.view.grid, "Toggle grid", 22.0).clicked() {
                self.session.view.grid = !self.session.view.grid;
                self.mark_dirty();
            }
            separator_v(ui);
            let (sg, sp) = (self.session.prefs.snap_grid, self.session.prefs.snap_point);
            if text_button(ui, "Grid Snap", sg).on_hover_text("Snap moves to the grid step, rotation and scale to their steps (Edit > Preferences for the values)").clicked() {
                self.session.prefs.snap_grid = !sg;
            }
            if text_button(ui, "Point Snap", sp).on_hover_text("Snap moves to the vertex, joint or locator under the cursor").clicked() {
                self.session.prefs.snap_point = !sp;
            }
            if text_button(ui, "Pivot", self.pivot_edit).on_hover_text("Edit Pivot mode (Insert): the Move tool moves the pivot instead of the object").clicked() {
                self.pivot_edit = !self.pivot_edit;
            }
            separator_v(ui);
            ui.checkbox(&mut self.auto_key, "Auto Key");
            let mut hist = self.session.history_on;
            if ui.checkbox(&mut hist, "History").on_hover_text("Record construction history").changed() {
                self.session.history_on = hist;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let sc = &self.session.scene;
                let file = sc.file_path.clone().unwrap_or_else(|| "untitled".to_string());
                let sel = match sc.selection.first() {
                    Some(id) => sc.name_of(*id),
                    None => "nothing selected".to_string(),
                };
                ui.label(egui::RichText::new(format!("{}   |   {}", file, sel)).color(TEXT_DIM));
            });
        });
    }

    fn shelf(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, t) in ["General", "Polygons", "Animation", "Rendering", "Custom"].iter().enumerate() {
                if text_button(ui, t, self.shelf_tab == i).clicked() {
                    self.shelf_tab = i;
                }
            }
            separator_v(ui);
            let sz = 26.0;
            let mut cmd: Option<&'static str> = None;
            match self.shelf_tab {
                0 => {
                    for (i, t, c) in [
                        (Icon::Cube, "Polygon Cube", "polyCube"),
                        (Icon::Sphere, "Polygon Sphere", "polySphere"),
                        (Icon::Cylinder, "Polygon Cylinder", "polyCylinder"),
                        (Icon::Plane, "Polygon Plane", "polyPlane"),
                        (Icon::Camera, "Camera", "camera"),
                        (Icon::Light, "Point Light", "pointLight"),
                        (Icon::Particles, "nParticles", "nParticle"),
                        (Icon::Group, "Group (Ctrl+G)", "group"),
                        (Icon::Duplicate, "Duplicate (Ctrl+D)", "duplicate"),
                        (Icon::Delete, "Delete", "delete"),
                    ] {
                        if icon_button(ui, i, false, t, sz).clicked() {
                            cmd = Some(c);
                        }
                    }
                }
                1 => {
                    for (i, t, c) in [
                        (Icon::Cube, "Polygon Cube", "polyCube"),
                        (Icon::Sphere, "Polygon Sphere", "polySphere"),
                        (Icon::Cylinder, "Polygon Cylinder", "polyCylinder"),
                        (Icon::Plane, "Polygon Plane", "polyPlane"),
                    ] {
                        if icon_button(ui, i, false, t, sz).clicked() {
                            cmd = Some(c);
                        }
                    }
                    separator_v(ui);
                    let ops: [(Icon, &str, Option<ModelOp>); 10] = [
                        (Icon::Extrude, "Extrude (Ctrl+E)", Some(ModelOp::Extrude { ltz: 0.0 })),
                        (Icon::Bevel, "Bevel (Ctrl+B)", Some(ModelOp::Bevel { offset: 0.2 })),
                        (Icon::Merge, "Merge Vertices", Some(ModelOp::Merge { dist: 0.001 })),
                        (Icon::Merge, "Merge to Center (all selected vertices become one)", Some(ModelOp::MergeCenter)),
                        (Icon::DelEdge, "Delete Edge / Vertex (Ctrl+Del)", Some(ModelOp::Delete { clean: true })),
                        (Icon::FillHole, "Fill Hole", Some(ModelOp::FillHole)),
                        (Icon::Smooth2, "Smooth", Some(ModelOp::Smooth { levels: 1 })),
                        (Icon::Combine, "Combine", Some(ModelOp::Combine)),
                        (Icon::Separate, "Separate", Some(ModelOp::Separate)),
                        (Icon::Mirror, "Mirror Geometry (X)", Some(ModelOp::Mirror { axis: 0 })),
                    ];
                    for (i, t, op) in ops {
                        if icon_button(ui, i, false, t, sz).clicked() {
                            if let Some(op) = op {
                                self.model(op);
                            }
                        }
                    }
                    if icon_button(ui, Icon::Triangulate, false, "Triangulate", sz).clicked() {
                        self.model(ModelOp::Triangulate);
                    }
                    separator_v(ui);
                    if icon_button(ui, Icon::SplitTool, self.tool == Tool::SplitPoly, "Split Polygon Tool", sz).clicked() {
                        self.start_tool(Tool::SplitPoly);
                    }
                    if icon_button(ui, Icon::LoopTool, self.tool == Tool::EdgeLoop, "Insert Edge Loop Tool", sz).clicked() {
                        self.start_tool(Tool::EdgeLoop);
                    }
                }
                2 => {
                    if icon_button(ui, Icon::Key, false, "Set Key (S)", sz).clicked() {
                        self.key_selected();
                    }
                    if icon_button(ui, Icon::Layer, false, "New animation layer", sz).clicked() {
                        cmd = Some("animLayer layer");
                    }
                    if icon_button(ui, Icon::Joint, self.tool == Tool::Joint, "Joint Tool", sz).clicked() {
                        self.start_tool(Tool::Joint);
                    }
                    if icon_button(ui, Icon::IkHandle, self.tool == Tool::IkHandle, "IK Handle Tool", sz).clicked() {
                        self.start_tool(Tool::IkHandle);
                    }
                    if icon_button(ui, Icon::Bind, false, "Bind Skin", sz).clicked() {
                        cmd = Some("bindSkin");
                    }
                    if icon_button(ui, Icon::Rewind, false, "Go to start", sz).clicked() {
                        let s = self.session.scene.timeline.start;
                        self.set_time(s);
                    }
                    if icon_button(ui, Icon::Play, self.session.scene.timeline.playing, "Play / pause (Alt+V)", sz).clicked() {
                        self.session.scene.timeline.toggle();
                        self.mark_dirty();
                    }
                }
                4 => {
                    // buttons made with shelfButton or Add to Shelf: click to run, right click to remove
                    let buttons = self.session.prefs.shelf.clone();
                    let mut remove: Option<String> = None;
                    let mut run: Option<String> = None;
                    if buttons.is_empty() {
                        ui.label(egui::RichText::new("Empty: the Script Editor's Add to Shelf button or the shelfButton command fills it").color(TEXT_DIM));
                    }
                    for b in &buttons {
                        let r = text_button(ui, &b.label, false).on_hover_text(if b.annotation.is_empty() { b.command.clone() } else { b.annotation.clone() });
                        if r.clicked() {
                            run = Some(b.command.clone());
                        }
                        r.context_menu(|ui| {
                            if ui.button("Delete").clicked() {
                                remove = Some(b.label.clone());
                                ui.close_menu();
                            }
                        });
                    }
                    if let Some(l) = remove {
                        self.session.prefs.shelf.retain(|b| b.label != l);
                        let _ = self.session.prefs.save();
                    }
                    if let Some(c) = run {
                        self.exec(&c);
                    }
                }
                _ => {
                    for (i, t, c) in [
                        (Icon::Light, "Point Light", "pointLight"),
                        (Icon::Directional, "Directional Light", "directionalLight"),
                        (Icon::Camera, "Camera", "camera"),
                        (Icon::StereoCamera, "Stereo Camera", "stereoCamera"),
                    ] {
                        if icon_button(ui, i, false, t, sz).clicked() {
                            cmd = Some(c);
                        }
                    }
                    if icon_button(ui, Icon::Render, false, "Render current frame", sz).clicked() {
                        self.render_still();
                    }
                }
            }
            if let Some(c) = cmd {
                self.exec(c);
            }
        });
    }

    fn tool_box(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(4.0);
            for (t, i, tip) in [
                (Tool::Select, Icon::Select, "Select Tool (Q)"),
                (Tool::Move, Icon::Move, "Move Tool (W)"),
                (Tool::Rotate, Icon::Rotate, "Rotate Tool (E)"),
                (Tool::Scale, Icon::Scale, "Scale Tool (R)"),
            ] {
                if icon_button(ui, i, self.tool == t, tip, 32.0).clicked() {
                    self.tool = t;
                }
            }
            if icon_button(ui, Icon::SplitTool, self.tool == Tool::SplitPoly, "Split Polygon Tool", 32.0).clicked() {
                self.start_tool(Tool::SplitPoly);
            }
            if icon_button(ui, Icon::LoopTool, self.tool == Tool::EdgeLoop, "Insert Edge Loop Tool", 32.0).clicked() {
                self.start_tool(Tool::EdgeLoop);
            }
            ui.add_space(8.0);
            let sep = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(sep * 0.8, 1.0), Sense::hover());
            ui.painter().rect_filled(r, 0.0, CHROME_DARK);
            ui.add_space(8.0);
            if icon_button(ui, Icon::Single, self.layout == LayoutMode::Single, "Single perspective view", 32.0).clicked() {
                self.layout = LayoutMode::Single;
                self.single_kind = ViewKind::Persp;
                self.show_outliner = false;
                self.mark_dirty();
            }
            if icon_button(ui, Icon::Four, self.layout == LayoutMode::Four, "Four view (Space)", 32.0).clicked() {
                self.layout = LayoutMode::Four;
                self.show_outliner = false;
                self.mark_dirty();
            }
            if icon_button(ui, Icon::PerspOutliner, self.layout == LayoutMode::PerspOutliner, "Persp / Outliner", 32.0).clicked() {
                self.layout = LayoutMode::PerspOutliner;
                self.single_kind = ViewKind::Persp;
                self.show_outliner = true;
                self.mark_dirty();
            }
            ui.add_space(8.0);
            if icon_button(ui, Icon::Frame, false, "Frame selection (F)", 32.0).clicked() {
                self.frame(true);
            }
        });
    }

    fn outliner(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Outliner").strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.checkbox(&mut self.show_shapes, "Shapes");
            });
        });
        ui.separator();
        // flatten the visible tree
        let mut rows: Vec<(NodeId, usize, bool)> = Vec::new(); // id, depth, has visible children
        {
            let sc = &self.session.scene;
            let show_shapes = self.show_shapes;
            let collapsed = &self.collapsed;
            fn walk(sc: &Scene, id: NodeId, depth: usize, show_shapes: bool, collapsed: &BTreeSet<NodeId>, rows: &mut Vec<(NodeId, usize, bool)>) {
                let node = match sc.node(id) {
                    Some(n) => n,
                    None => return,
                };
                let kids: Vec<NodeId> = node
                    .children
                    .iter()
                    .copied()
                    .filter(|c| sc.node(*c).map(|n| show_shapes || n.data.is_transform()).unwrap_or(false))
                    .collect();
                rows.push((id, depth, !kids.is_empty()));
                if !collapsed.contains(&id) {
                    for c in kids {
                        walk(sc, c, depth + 1, show_shapes, collapsed, rows);
                    }
                }
            }
            for r in &sc.roots {
                let n = match sc.node(*r) {
                    Some(n) => n,
                    None => continue,
                };
                // the default camera and other non-transform roots stay visible like Maya's outliner
                let _ = n;
                walk(sc, *r, 0, show_shapes, collapsed, &mut rows);
            }
        }
        let mut clicked: Option<(NodeId, bool)> = None;
        let mut start_rename: Option<(NodeId, String)> = None;
        let mut toggle: Option<NodeId> = None;
        let mut drag_start: Option<NodeId> = None;
        let mut ctx_action: Option<(NodeId, u8)> = None;
        let mut row_rects: Vec<(NodeId, Rect)> = Vec::new();
        egui::Frame::NONE.fill(LIST_BG).inner_margin(2.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for (id, depth, has_kids) in &rows {
                    let node = match self.session.scene.node(*id) {
                        Some(n) => n,
                        None => continue,
                    };
                    let selected = self.session.scene.selection.contains(id);
                    let (glyph, gc) = node_icon(&node.data);
                    let name = node.name.clone();
                    let collapsed = self.collapsed.contains(id);
                    let resp = ui
                        .horizontal(|ui| {
                            ui.add_space(*depth as f32 * 14.0);
                            if *has_kids {
                                let arrow = if collapsed { "\u{25B8}" } else { "\u{25BE}" };
                                if ui.add(egui::Label::new(egui::RichText::new(arrow).color(TEXT_DIM)).sense(Sense::click())).clicked() {
                                    toggle = Some(*id);
                                }
                            } else {
                                ui.add_space(10.0);
                            }
                            ui.label(egui::RichText::new(glyph).color(gc));
                            if let Some((rid, buf)) = &mut self.rename {
                                if rid == id {
                                    let r = ui.add(egui::TextEdit::singleline(buf).desired_width(120.0));
                                    r.request_focus();
                                    if r.lost_focus() {
                                        let nn = buf.clone();
                                        let rid = *rid;
                                        self.rename = None;
                                        if ui.input(|i| i.key_pressed(Key::Enter)) && !nn.is_empty() {
                                            if self.session.scene.rename(rid, &nn).is_err() {
                                                self.mel_result = "// Error: could not rename".into();
                                            }
                                            self.scene_edits = true;
                                            self.gen += 1;
                                        }
                                    }
                                    return None;
                                }
                            }
                            let fill = if selected { SELECT } else { Color32::TRANSPARENT };
                            let txt = if selected { egui::RichText::new(name.clone()).color(Color32::WHITE) } else { egui::RichText::new(name.clone()) };
                            let r = egui::Frame::NONE
                                .fill(fill)
                                .inner_margin(egui::Margin::symmetric(3, 0))
                                .show(ui, |ui| ui.add(egui::Label::new(txt).sense(Sense::click_and_drag())))
                                .inner;
                            Some(r)
                        })
                        .inner;
                    if let Some(r) = resp {
                        row_rects.push((*id, r.rect));
                        if r.clicked() {
                            clicked = Some((*id, ui.input(|i| i.modifiers.shift || i.modifiers.ctrl)));
                        }
                        if r.double_clicked() {
                            start_rename = Some((*id, name.clone()));
                        }
                        if r.drag_started() {
                            drag_start = Some(*id);
                        }
                        r.context_menu(|ui| {
                            for (k, l) in [
                                (1u8, "Rename"),
                                (2, "Select Hierarchy"),
                                (3, "Duplicate"),
                                (4, "Group"),
                                (5, "Unparent (to world)"),
                                (6, "Delete"),
                                (7, "Delete History"),
                            ] {
                                if ui.button(l).clicked() {
                                    ctx_action = Some((*id, k));
                                    ui.close_menu();
                                }
                            }
                        });
                    }
                }
                // dropping on empty space unparents
                let rest = ui.available_rect_before_wrap();
                let _ = rest;
            });
        });
        if let Some(t) = toggle {
            if !self.collapsed.remove(&t) {
                self.collapsed.insert(t);
            }
        }
        if let Some(d) = drag_start {
            self.outliner_drag = Some(d);
        }
        if let Some(d) = self.outliner_drag {
            let (pos, released) = ui.ctx().input(|i| (i.pointer.hover_pos(), i.pointer.any_released()));
            if let Some(p) = pos {
                let target = row_rects.iter().find(|(id, r)| r.contains(p) && *id != d).map(|(id, _)| *id);
                if let Some((_, r)) = row_rects.iter().find(|(id, r)| r.contains(p) && *id != d) {
                    ui.painter().rect_stroke(r.expand(1.0), 2.0, Stroke::new(1.5_f32, Color32::from_rgb(0xff, 0xa0, 0x20)), eframe::egui::StrokeKind::Inside);
                }
                if released {
                    self.outliner_drag = None;
                    match target {
                        Some(t) => {
                            match self.session.scene.set_parent_keep_world(d, Some(t)) {
                                Ok(_) => {
                                    self.collapsed.remove(&t);
                                    self.scene_edited();
                                }
                                Err(e) => self.say(&e),
                            }
                        }
                        None => {
                            let over_list = ui.min_rect().contains(p);
                            if over_list {
                                let _ = self.session.scene.set_parent_keep_world(d, None);
                                self.scene_edited();
                            }
                        }
                    }
                }
            } else if released {
                self.outliner_drag = None;
            }
        }
        if let Some((id, add)) = clicked {
            let sel = &mut self.session.scene.selection;
            if add {
                if let Some(p) = sel.iter().position(|s| *s == id) {
                    sel.remove(p);
                } else {
                    sel.push(id);
                }
            } else {
                *sel = vec![id];
            }
            self.mark_dirty();
        }
        if let Some(r) = start_rename {
            self.rename = Some(r);
        }
        if let Some((id, k)) = ctx_action {
            if !self.session.scene.selection.contains(&id) {
                self.session.scene.selection = vec![id];
            }
            match k {
                1 => {
                    let n = self.session.scene.name_of(id);
                    self.rename = Some((id, n));
                }
                2 => self.select_hierarchy(),
                3 => self.exec("duplicate"),
                4 => self.exec("group"),
                5 => self.unparent_selection(),
                6 => self.exec("delete"),
                _ => self.exec("deleteHistory"),
            }
            self.mark_dirty();
        }
    }

    fn parent_selection(&mut self) {
        let sel = self.session.scene.selection.clone();
        if sel.len() < 2 {
            self.say("Select the children, then the parent last");
            return;
        }
        let parent = *sel.last().unwrap();
        let mut n = 0;
        for c in &sel[..sel.len() - 1] {
            match self.session.scene.set_parent_keep_world(*c, Some(parent)) {
                Ok(_) => n += 1,
                Err(e) => {
                    self.say(&e);
                    return;
                }
            }
        }
        self.collapsed.remove(&parent);
        self.say(&format!("Parented {} object(s) to {}", n, self.session.scene.name_of(parent)));
        self.scene_edited();
    }

    fn unparent_selection(&mut self) {
        let sel = self.session.scene.selection.clone();
        for c in sel {
            let _ = self.session.scene.set_parent_keep_world(c, None);
        }
        self.scene_edited();
    }

    fn select_hierarchy(&mut self) {
        let mut out: Vec<NodeId> = Vec::new();
        for id in self.session.scene.selection.clone() {
            for d in self.session.scene.descendants(id) {
                let is_t = self.session.scene.node(d).map(|n| n.data.is_transform()).unwrap_or(false);
                if is_t && !out.contains(&d) {
                    out.push(d);
                }
            }
        }
        self.session.scene.selection = out;
        self.mark_dirty();
    }

    fn cb_plug_items(sc: &mut Scene, id: NodeId, out: &mut Vec<CbItem>, keep: &dyn Fn(&str) -> bool) {
        let list: Vec<(String, bool, bool, bool, bool)> = match sc.node(id) {
            Some(n) => n.plugs.iter().map(|p| (p.name.clone(), p.source.is_some(), p.readonly || p.locked, p.keyable, p.locked)).collect(),
            None => return,
        };
        for (name, connected, readonly, keyable, _locked) in list {
            if !keep(&name) {
                continue;
            }
            // the Channel Box lists keyable attributes, plus anything animated or connected
            if !keyable && !connected && !sc.anim.is_animated(id, &name) {
                continue;
            }
            let value = sc.get_attr(id, &name).unwrap_or(0.0);
            let label = match name.strip_prefix("pnts_") {
                Some(r) => {
                    let mut it = r.split('_');
                    format!("pnts[{}].{}", it.next().unwrap_or("?"), it.next().unwrap_or("?"))
                }
                None => nice_name(&name),
            };
            out.push(CbItem::Attr {
                node: id,
                attr: name.clone(),
                label,
                value,
                keyed: sc.anim.is_animated(id, &name),
                connected,
                editable: !connected && !readonly,
            });
        }
    }

    fn cb_items(&mut self) -> Vec<CbItem> {
        self.session.scene.flush_meshes();
        let mut out: Vec<CbItem> = Vec::new();
        let id = match self.session.scene.selection.first() {
            Some(i) if self.session.scene.node(*i).is_some() => *i,
            _ => return out,
        };
        let sel_verts: BTreeSet<u32> = {
            let c = &self.session.comp;
            match (c.mode, c.node) {
                (SelMode::Object, _) | (_, None) => BTreeSet::new(),
                (_, Some(shape)) => match self.mesh_poly(shape) {
                    Some((poly, _)) => cmd_model::verts_of(&poly, &c.sel, c.mode),
                    None => BTreeSet::new(),
                },
            }
        };
        let sc = &mut self.session.scene;
        let name = sc.name_of(id);
        let tag = sc.node(id).map(|n| n.data.type_name()).unwrap_or("");
        out.push(CbItem::Header { text: name, tag: tag.to_string(), level: 0, node: None, shape: None });
        Self::cb_plug_items(sc, id, &mut out, &|_| true);
        let children: Vec<NodeId> = sc.node(id).map(|n| n.children.clone()).unwrap_or_default();
        let mut owners: Vec<NodeId> = vec![id];
        let mut material: Option<NodeId> = None;
        for c in children {
            let (cname, is_mesh, ctag) = match sc.node(c) {
                Some(n) => (n.name.clone(), matches!(n.data, NodeData::Mesh(_)), n.data.type_name()),
                None => continue,
            };
            owners.push(c);
            out.push(CbItem::Header { text: cname, tag: ctag.to_string(), level: 1, node: None, shape: None });
            Self::cb_plug_items(sc, c, &mut out, &|_| true);
            if !is_mesh {
                continue;
            }
            let (hist, mat, counts) = match sc.node(c).map(|n| &n.data) {
                Some(NodeData::Mesh(md)) => (md.history.clone(), md.material, (md.mesh.positions.len(), md.mesh.faces.len())),
                _ => continue,
            };
            out.push(CbItem::Info(format!("{} vertices, {} faces", counts.0, counts.1)));
            if material.is_none() {
                material = mat;
            }
            if !hist.is_empty() {
                out.push(CbItem::Section("INPUTS".to_string()));
            }
            for h in hist.iter().rev() {
                owners.push(*h);
                let (hname, kind, ntw) = match sc.node(*h) {
                    Some(n) => match &n.data {
                        NodeData::PolyOp(op) => (
                            n.name.clone(),
                            op.kind,
                            n.plugs.iter().filter(|p| p.name.starts_with("pnts_") && p.name.ends_with("_x")).count(),
                        ),
                        _ => continue,
                    },
                    None => continue,
                };
                out.push(CbItem::Header { text: hname, tag: hist_label(kind).to_string(), level: 2, node: Some(*h), shape: Some(c) });
                let sv = sel_verts.clone();
                Self::cb_plug_items(sc, *h, &mut out, &move |n: &str| match n.strip_prefix("pnts_") {
                    Some(r) => r.split('_').next().and_then(|v| v.parse::<u32>().ok()).map(|v| sv.contains(&v)).unwrap_or(false),
                    None => true,
                });
                if kind == PolyKind::Tweak {
                    out.push(CbItem::Info(format!("{} tweaked vertices", ntw)));
                }
            }
        }
        // animation curve nodes
        let curves: Vec<(NodeId, String)> = sc
            .anim
            .bindings
            .iter()
            .filter(|b| owners.contains(&b.node))
            .map(|b| (b.node, b.attr.clone()))
            .collect();
        let mut seen: Vec<(NodeId, String)> = Vec::new();
        for (n, a) in curves {
            if seen.contains(&(n, a.clone())) {
                continue;
            }
            seen.push((n, a.clone()));
            if !out.iter().any(|i| matches!(i, CbItem::Section(s) if s == "ANIMATION")) {
                out.push(CbItem::Section("ANIMATION".to_string()));
            }
            let ty = if a.starts_with("translate") || a.starts_with("pnts_") {
                "animCurveTL"
            } else if a.starts_with("rotate") {
                "animCurveTA"
            } else {
                "animCurveTU"
            };
            out.push(CbItem::Curve { name: format!("{}_{}", sc.name_of(n), a), ty, node: n, attr: a });
        }
        if let Some(m) = material {
            out.push(CbItem::Section("OUTPUTS".to_string()));
            let mname = sc.name_of(m);
            out.push(CbItem::Header { text: mname, tag: "lambert".to_string(), level: 2, node: None, shape: None });
            Self::cb_plug_items(sc, m, &mut out, &|_| true);
            out.push(CbItem::Texture { mat: m, path: sc.color_map(m) });
        }
        out
    }

    fn channel_box(&mut self, ui: &mut Ui) {
        let items = self.cb_items();
        let title = match self.session.scene.selection.first().and_then(|i| self.session.scene.node(*i)) {
            Some(n) => n.name.clone(),
            None => "Channel Box".to_string(),
        };
        match self.session.scene.selection.first().copied() {
            Some(id) => {
                if self.name_buf.0 != id || (!self.name_editing && self.name_buf.1 != title) {
                    self.name_buf = (id, title.clone());
                }
                let r = ui.add(egui::TextEdit::singleline(&mut self.name_buf.1).desired_width(f32::INFINITY));
                self.name_editing = r.has_focus();
                if r.lost_focus() {
                    let nn = self.name_buf.1.trim().to_string();
                    if !nn.is_empty() && nn != title && ui.input(|i| i.key_pressed(Key::Enter)) {
                        match self.session.scene.rename(id, &nn) {
                            Ok(final_name) => {
                                self.name_buf.1 = final_name;
                                self.scene_edited();
                            }
                            Err(e) => self.say(&e),
                        }
                    } else {
                        self.name_buf.1 = title.clone();
                    }
                }
            }
            None => {
                ui.label(egui::RichText::new(title).strong());
            }
        }
        ui.separator();
        let mut sets: Vec<(NodeId, String, f32)> = Vec::new();
        let mut key_one: Option<(NodeId, String)> = None;
        let mut cut_one: Option<(NodeId, String)> = None;
        let mut open_curve: Option<(NodeId, String)> = None;
        let mut pick_texture: Option<NodeId> = None;
        let mut clear_texture: Option<NodeId> = None;
        let mut del_node: Option<(NodeId, NodeId)> = None;
        let mut del_all: Option<NodeId> = None;
        egui::ScrollArea::vertical().id_salt("channels").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            for item in &items {
                match item {
                    CbItem::Header { text, tag, level, node, shape } => {
                        ui.add_space(4.0);
                        let col = match level {
                            0 => Color32::from_rgb(0x10, 0x10, 0x10),
                            1 => Color32::from_rgb(0x20, 0x40, 0x80),
                            _ => Color32::from_rgb(0x60, 0x30, 0x80),
                        };
                        ui.horizontal(|ui| {
                            let r = ui.add(egui::Label::new(egui::RichText::new(text).strong().color(col)).sense(Sense::click()));
                            ui.label(egui::RichText::new(tag).small().color(TEXT_DIM));
                            if let (Some(n), Some(sh)) = (node, shape) {
                                r.on_hover_text("Right click: delete this history node").context_menu(|ui| {
                                    if ui.button("Delete This Node").clicked() {
                                        del_node = Some((*sh, *n));
                                        ui.close_menu();
                                    }
                                    if ui.button("Delete All History").clicked() {
                                        del_all = Some(*sh);
                                        ui.close_menu();
                                    }
                                });
                            }
                        });
                    }
                    CbItem::Section(t) => {
                        ui.add_space(6.0);
                        egui::Frame::NONE.fill(CHROME_DARK).inner_margin(egui::Margin::symmetric(4, 1)).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(egui::RichText::new(t).strong().size(11.0));
                        });
                    }
                    CbItem::Info(t) => {
                        ui.label(egui::RichText::new(t).small().color(TEXT_DIM));
                    }
                    CbItem::Attr { node, attr, label, value, keyed, connected, editable } => {
                        let bg = if *keyed {
                            KEYED
                        } else if *connected {
                            CONNECTED
                        } else {
                            FIELD
                        };
                        egui::Frame::NONE.fill(bg).inner_margin(egui::Margin::symmetric(3, 1)).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.add_sized(vec2(100.0, 16.0), egui::Label::new(label).truncate());
                                if *editable {
                                    let mut v = *value;
                                    let speed = if attr.starts_with("rotate") { 0.5 } else { 0.02 };
                                    let r = ui.add(egui::DragValue::new(&mut v).speed(speed).max_decimals(4));
                                    if r.changed() {
                                        sets.push((*node, attr.clone(), v));
                                    }
                                    r.context_menu(|ui| {
                                        if ui.button("Key Selected").clicked() {
                                            key_one = Some((*node, attr.clone()));
                                            ui.close_menu();
                                        }
                                        if ui.button("Delete Key at Current Time").clicked() {
                                            cut_one = Some((*node, attr.clone()));
                                            ui.close_menu();
                                        }
                                    });
                                } else {
                                    ui.label(egui::RichText::new(format!("{:.4}", value)).color(TEXT_DIM));
                                }
                            });
                        });
                    }
                    CbItem::Curve { name, ty, node, attr } => {
                        ui.horizontal(|ui| {
                            if ui.link(name).on_hover_text("Open the animation curve").clicked() {
                                open_curve = Some((*node, attr.clone()));
                            }
                            ui.label(egui::RichText::new(*ty).small().color(TEXT_DIM));
                        });
                    }
                    CbItem::Texture { mat, path } => {
                        ui.horizontal(|ui| {
                            ui.label("Color Map");
                            let shown = path.clone().unwrap_or_else(|| "none".to_string());
                            let short = shown.rsplit('/').next().unwrap_or(&shown).to_string();
                            ui.label(egui::RichText::new(short).color(TEXT_DIM));
                        });
                        ui.horizontal(|ui| {
                            if ui.small_button("Browse...").clicked() {
                                pick_texture = Some(*mat);
                            }
                            if ui.small_button("Checker").clicked() {
                                self.exec(&format!("assignTexture \"{}\" \"builtin:checker\"", self.session.scene.name_of(*mat)));
                            }
                            if path.is_some() && ui.small_button("Clear").clicked() {
                                clear_texture = Some(*mat);
                            }
                        });
                    }
                }
            }
            if items.is_empty() {
                ui.label(egui::RichText::new("Nothing selected").color(TEXT_DIM));
            }
        });
        let layer = self.session.scene.anim.active_layer;
        let auto = self.auto_key;
        for (id, attr, v) in sets {
            let _ = self.session.scene.set_attr(id, &attr, v);
            if auto {
                let _ = self.session.scene.set_keyframe(id, &attr, None, Some(v), self.default_interp(), layer);
            }
            self.scene_edited();
        }
        if let Some((id, a)) = key_one {
            let _ = self.session.scene.set_keyframe(id, &a, None, None, self.default_interp(), layer);
            self.scene_edited();
        }
        if let Some((id, a)) = cut_one {
            let t = self.session.scene.timeline.current;
            for l in 0..self.session.scene.anim.layers.len() {
                self.session.scene.anim.remove_key(id, &a, l, t);
            }
            self.session.scene.evaluate_frame();
            self.scene_edited();
        }
        if let Some((sh, n)) = del_node {
            let name = self.session.scene.name_of(n);
            match self.session.scene.delete_history_node(sh, n) {
                Ok(_) => {
                    self.say(&format!("Deleted {}", name));
                    self.scene_edited();
                }
                Err(e) => self.say(&e),
            }
        }
        if let Some(sh) = del_all {
            let n = self.session.scene.delete_history(sh);
            self.say(&format!("Deleted {} history node(s)", n));
            self.session.comp.sel = Sel::default();
            self.scene_edited();
        }
        if let Some(c) = open_curve {
            self.curve_win = Some(c);
        }
        if let Some(m) = pick_texture {
            self.open_texture_dialog(m);
        }
        if let Some(m) = clear_texture {
            let n = self.session.scene.name_of(m);
            self.exec(&format!("assignTexture -clear \"{}\"", n));
        }
    }

    fn curve_window(&mut self, ctx: &egui::Context) {
        let (node, attr) = match self.curve_win.clone() {
            Some(c) => c,
            None => return,
        };
        let title = format!("{}_{}", self.session.scene.name_of(node), attr);
        let mut open = true;
        let mut changed = false;
        let cur_t = self.session.scene.timeline.current;
        let mut add_key = false;
        egui::Window::new(format!("Animation Curve: {}", title)).open(&mut open).default_size([420.0, 360.0]).show(ctx, |ui| {
            let layers: Vec<usize> = self
                .session
                .scene
                .anim
                .bindings
                .iter()
                .enumerate()
                .filter(|(_, b)| b.node == node && b.attr == attr)
                .map(|(i, _)| i)
                .collect();
            // plot
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 130.0), Sense::hover());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 0.0, Color32::from_rgb(0x58, 0x58, 0x58));
            let mut tmin = f32::INFINITY;
            let mut tmax = f32::NEG_INFINITY;
            let mut vmin = f32::INFINITY;
            let mut vmax = f32::NEG_INFINITY;
            for &bi in &layers {
                for k in &self.session.scene.anim.bindings[bi].curve.keys {
                    tmin = tmin.min(k.t);
                    tmax = tmax.max(k.t);
                    vmin = vmin.min(k.v);
                    vmax = vmax.max(k.v);
                }
            }
            if tmin.is_finite() {
                if (tmax - tmin).abs() < 1e-3 {
                    tmax = tmin + 1.0;
                }
                if (vmax - vmin).abs() < 1e-3 {
                    vmax = vmin + 1.0;
                }
                let pad = (vmax - vmin) * 0.15;
                let (vmin, vmax) = (vmin - pad, vmax + pad);
                let to = |t: f32, v: f32| {
                    pos2(
                        rect.left() + 8.0 + (t - tmin) / (tmax - tmin) * (rect.width() - 16.0),
                        rect.bottom() - 8.0 - (v - vmin) / (vmax - vmin) * (rect.height() - 16.0),
                    )
                };
                for &bi in &layers {
                    let curve = &self.session.scene.anim.bindings[bi].curve;
                    let pts: Vec<Pos2> = (0..=80)
                        .filter_map(|i| {
                            let t = tmin + (tmax - tmin) * i as f32 / 80.0;
                            curve.eval(t).map(|v| to(t, v))
                        })
                        .collect();
                    p.add(egui::Shape::line(pts, Stroke::new(1.5_f32, Color32::from_rgb(0xff, 0xa0, 0x40))));
                    for k in &curve.keys {
                        p.circle_filled(to(k.t, k.v), 3.5, Color32::from_rgb(0xff, 0xe0, 0x40));
                    }
                }
                let x = to(cur_t.clamp(tmin, tmax), vmin).x;
                p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0_f32, Color32::from_rgb(0x60, 0x90, 0xff)));
            }
            for &bi in &layers {
                let lname = {
                    let l = self.session.scene.anim.bindings[bi].layer;
                    self.session.scene.anim.layers.get(l).map(|l| l.name.clone()).unwrap_or_default()
                };
                ui.label(egui::RichText::new(format!("Layer: {}", lname)).strong());
                let mut remove: Option<usize> = None;
                let n = self.session.scene.anim.bindings[bi].curve.keys.len();
                for ki in 0..n {
                    let (mut t, mut v, ip) = {
                        let k = &self.session.scene.anim.bindings[bi].curve.keys[ki];
                        (k.t, k.v, k.interp)
                    };
                    ui.horizontal(|ui| {
                        ui.label(format!("{:>2}", ki + 1));
                        let c1 = ui.add(egui::DragValue::new(&mut t).speed(0.2).prefix("t "));
                        let c2 = ui.add(egui::DragValue::new(&mut v).speed(0.02).prefix("v "));
                        let mut ipn = ip;
                        egui::ComboBox::from_id_salt(("ip", bi, ki)).width(70.0).selected_text(ip.name()).show_ui(ui, |ui| {
                            for opt in [crate::anim::Interp::Bezier, crate::anim::Interp::Linear, crate::anim::Interp::Step] {
                                ui.selectable_value(&mut ipn, opt, opt.name());
                            }
                        });
                        if c1.changed() || c2.changed() || ipn != ip {
                            let b = &mut self.session.scene.anim.bindings[bi];
                            b.curve.keys[ki].t = t.max(0.0);
                            b.curve.keys[ki].v = v;
                            b.curve.keys[ki].interp = ipn;
                            b.curve.keys.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
                            b.curve.recompute_tangents();
                            changed = true;
                        }
                        if ui.small_button("x").on_hover_text("Delete key").clicked() {
                            remove = Some(ki);
                        }
                    });
                }
                if let Some(ki) = remove {
                    let b = &mut self.session.scene.anim.bindings[bi];
                    b.curve.keys.remove(ki);
                    b.curve.recompute_tangents();
                    changed = true;
                }
            }
            if ui.button("Add key at current time").clicked() {
                add_key = true;
            }
        });
        if add_key {
            let layer = self.session.scene.anim.active_layer;
            let _ = self.session.scene.set_keyframe(node, &attr, None, None, self.default_interp(), layer);
            changed = true;
        }
        if changed {
            self.session.scene.anim.bindings.retain(|b| !b.curve.keys.is_empty());
            self.session.scene.evaluate_frame();
            self.scene_edited();
        }
        if !open {
            self.curve_win = None;
        }
    }

    fn open_texture_dialog(&mut self, mat: NodeId) {
        self.open_file_dialog(false);
        let images = self.session.scene.project_dir.as_ref().map(|d| d.join("sourceimages")).filter(|d| d.is_dir());
        if let Some(fd) = &mut self.file_dialog {
            fd.tex_for = Some(mat);
            fd.name = String::new();
            if let Some(d) = images {
                fd.dir = d;
            }
        }
    }

    fn layer_editor(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Anim Layers").strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("New").clicked() {
                    self.exec("animLayer layer");
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let active = self.session.scene.anim.active_layer;
            let lname = self.session.scene.anim.layers.get(active).map(|l| l.name.clone()).unwrap_or_default();
            if active > 0 {
                if ui.small_button("Add Selected").on_hover_text("Add the keyable channels of the selected objects to the active layer").clicked() {
                    self.exec(&format!("animLayer -addSelected {}", lname));
                }
                if ui.small_button("Zero Key").on_hover_text("Key the layer's channels at zero at the current frame").clicked() {
                    self.exec(&format!("animLayer -zeroKey {}", lname));
                }
                if ui.small_button("K").on_hover_text("Key the layer weight at the current frame").clicked() {
                    self.exec(&format!("animLayer -keyWeight {}", lname));
                }
                if ui.small_button("Merge").on_hover_text("Bake the active layer into the base animation").clicked() {
                    self.exec(&format!("animLayer -merge {}", lname));
                }
            }
        });
        ui.separator();
        let n = self.session.scene.anim.layers.len();
        let mut set_active: Option<usize> = None;
        let mut weights: Vec<(usize, f32)> = Vec::new();
        let mut mutes: Vec<(usize, bool)> = Vec::new();
        egui::Frame::NONE.fill(LIST_BG).inner_margin(2.0).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            for i in 0..n {
                let (name, weight, mute) = {
                    let l = &self.session.scene.anim.layers[i];
                    (l.name.clone(), l.weight, l.mute)
                };
                let active = self.session.scene.anim.active_layer == i;
                ui.horizontal(|ui| {
                    if ui.selectable_label(active, name).clicked() {
                        set_active = Some(i);
                    }
                    if i > 0 {
                        let mut w = weight;
                        if ui.add(egui::DragValue::new(&mut w).speed(0.01).range(0.0..=1.0).fixed_decimals(2)).changed() {
                            weights.push((i, w));
                        }
                        let mut m = mute;
                        if ui.checkbox(&mut m, "M").on_hover_text("Mute").changed() {
                            mutes.push((i, m));
                        }
                    }
                });
            }
        });
        if let Some(i) = set_active {
            self.session.scene.anim.active_layer = i;
        }
        for (i, w) in weights {
            self.session.scene.anim.set_weight(i, w);
            self.session.scene.evaluate_frame();
            self.scene_edited();
        }
        for (i, m) in mutes {
            self.session.scene.anim.set_mute(i, m);
            self.session.scene.evaluate_frame();
            self.scene_edited();
        }
    }

    fn time_slider(&mut self, ui: &mut Ui) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        let tl = self.session.scene.timeline.clone();
        p.rect_filled(rect, 0.0, FIELD);
        let left = rect.left() + 10.0;
        let right = rect.right() - 10.0;
        let span = (tl.end - tl.start).max(1.0);
        let x_of = |f: f32| left + (f - tl.start) / span * (right - left);
        let px_per_frame = (right - left) / span;
        let steps = [1.0f32, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0];
        let step = steps.iter().copied().find(|s| s * px_per_frame >= 46.0).unwrap_or(100.0);
        let minor = steps.iter().copied().find(|s| s * px_per_frame >= 5.0).unwrap_or(1.0);
        let mut f = (tl.start / minor).ceil() * minor;
        while f <= tl.end + 0.001 {
            let x = x_of(f);
            let major = (f / step).fract().abs() < 1e-4 || (f - tl.start).abs() < 1e-3;
            p.line_segment(
                [pos2(x, rect.bottom() - if major { 14.0 } else { 7.0 }), pos2(x, rect.bottom())],
                Stroke::new(1.0_f32, Color32::from_rgb(0x50, 0x50, 0x50)),
            );
            if major {
                p.text(pos2(x + 2.0, rect.top() + 3.0), Align2::LEFT_TOP, format!("{}", f as i32), FontId::proportional(10.5), TEXT);
            }
            f += minor;
        }
        // sound waveforms
        for id in self.session.scene.ids_where(|d| matches!(d, NodeData::Sound(_))) {
            let off = self.session.scene.get_attr(id, "offset").unwrap_or(1.0);
            if let Some(NodeData::Sound(sd)) = self.session.scene.node(id).map(|n| &n.data) {
                let per = tl.fps / crate::anim::sound::PEAKS_PER_SECOND as f32;
                let mid = rect.top() + 28.0;
                for (i, pk) in sd.peaks.iter().enumerate() {
                    let fr = off + i as f32 * per;
                    if fr < tl.start || fr > tl.end {
                        continue;
                    }
                    let x = x_of(fr);
                    let h = (pk * 11.0).max(0.5);
                    p.line_segment([pos2(x, mid - h), pos2(x, mid + h)], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0x60, 0xc0, 0x70, 150)));
                }
            }
        }
        // keys of the selection
        let sel = self.session.scene.selection.clone();
        let owners = self.session.scene.key_owner_ids(&sel);
        let times: Vec<f32> = if owners.is_empty() { Vec::new() } else { self.session.scene.anim.key_times(Some(&owners)) };
        let is_sel = |sel_times: &[f32], t: f32| sel_times.iter().any(|s| (s - t).abs() < 1e-3);
        let drag_delta = self.key_drag.map(|d| d.1).unwrap_or(0.0);
        for t in &times {
            if *t >= tl.start && *t <= tl.end {
                let x = x_of(*t);
                let selected = is_sel(&self.key_sel, *t);
                let col = if selected { Color32::from_rgb(0xff, 0xf0, 0x40) } else { Color32::from_rgb(0xd0, 0x20, 0x20) };
                let w = if selected { 2.5 } else { 1.0 };
                p.rect_filled(Rect::from_min_max(pos2(x - w, rect.top() + 14.0), pos2(x + w, rect.bottom() - 2.0)), 0.0, col);
                if selected && drag_delta != 0.0 {
                    let gx = x_of(*t + drag_delta);
                    p.rect_filled(Rect::from_min_max(pos2(gx - 1.5, rect.top() + 14.0), pos2(gx + 1.5, rect.bottom() - 2.0)), 0.0, Color32::from_rgba_unmultiplied(0xff, 0xf0, 0x40, 150));
                    p.line_segment([pos2(x, rect.top() + 24.0), pos2(gx, rect.top() + 24.0)], Stroke::new(1.0_f32, Color32::from_rgb(0x80, 0x60, 0x00)));
                }
            }
        }
        // current time
        let cx = x_of(tl.current);
        let label = format!("{}", tl.frame());
        let w = 8.0 + label.len() as f32 * 6.5;
        let r = Rect::from_center_size(pos2(cx, rect.top() + 22.0), vec2(w.max(18.0), rect.height() - 6.0));
        p.rect(r, 2.0, Color32::from_rgba_unmultiplied(0x60, 0x80, 0xc0, 150), Stroke::new(1.0_f32, Color32::from_rgb(0x20, 0x30, 0x60)), eframe::egui::StrokeKind::Inside);
        p.text(pos2(cx, rect.top() + 7.0), Align2::CENTER_TOP, label, FontId::proportional(11.0), Color32::BLACK);

        let near_key = |x: f32, y: f32| -> Option<f32> {
            if y < rect.top() + 12.0 {
                return None;
            }
            times.iter().copied().filter(|t| *t >= tl.start && *t <= tl.end).find(|t| (x_of(*t) - x).abs() <= 5.0)
        };
        let frame_at = |x: f32| (tl.start + (x - left) / (right - left) * span).round().clamp(tl.start, tl.end);
        let shift = ui.ctx().input(|i| i.modifiers.shift);
        if resp.drag_started() {
            let origin = ui.ctx().input(|i| i.pointer.press_origin());
            if let Some(o) = origin {
                if let Some(t) = near_key(o.x, o.y) {
                    if !is_sel(&self.key_sel, t) {
                        if !shift {
                            self.key_sel.clear();
                        }
                        self.key_sel.push(t);
                    }
                    self.key_drag = Some((t, 0.0));
                } else {
                    self.key_drag = None;
                    self.key_sel.clear();
                }
            }
        }
        if resp.dragged() {
            if let (Some(pos), Some((t0, _))) = (resp.interact_pointer_pos(), self.key_drag) {
                self.key_drag = Some((t0, frame_at(pos.x) - t0));
            } else if let Some(pos) = resp.interact_pointer_pos() {
                self.set_time(frame_at(pos.x));
            }
        }
        if resp.drag_stopped() {
            if let Some((_, delta)) = self.key_drag.take() {
                if delta != 0.0 {
                    let mut ts = self.key_sel.clone();
                    ts.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    if delta > 0.0 {
                        ts.reverse();
                    }
                    let mut moved = 0;
                    let mut new_sel = Vec::new();
                    for t in &ts {
                        moved += self.session.scene.anim.shift_keys(&owners, &[*t], delta);
                        new_sel.push(t + delta);
                    }
                    self.key_sel = new_sel;
                    self.session.scene.evaluate_frame();
                    self.say(&format!("Moved {} keys by {} frames", moved, delta));
                    self.scene_edited();
                }
            }
        }
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                match near_key(pos.x, pos.y) {
                    Some(t) => {
                        if shift {
                            if let Some(i) = self.key_sel.iter().position(|s| (s - t).abs() < 1e-3) {
                                self.key_sel.remove(i);
                            } else {
                                self.key_sel.push(t);
                            }
                        } else {
                            self.key_sel = vec![t];
                        }
                    }
                    None => {
                        self.key_sel.clear();
                        self.set_time(frame_at(pos.x));
                    }
                }
            }
        }
        // right-click menu: Copy / Cut / Delete the selected keys, Paste at the current time or where you clicked
        if resp.secondary_clicked() {
            if let Some(pos) = resp.interact_pointer_pos().or(resp.hover_pos()) {
                self.ts_menu_frame = frame_at(pos.x);
                if let Some(t) = near_key(pos.x, pos.y) {
                    if !is_sel(&self.key_sel, t) {
                        if !shift {
                            self.key_sel.clear();
                        }
                        self.key_sel.push(t);
                    }
                }
            }
        }
        let mut action: u8 = 0;
        let have_sel = !self.key_sel.is_empty();
        let have_clip = !self.session.key_clip.is_empty();
        let (cur_frame, here) = (tl.frame(), self.ts_menu_frame);
        let (n_sel, n_clip) = (self.key_sel.len(), self.session.key_clip.len());
        resp.context_menu(|ui| {
            if ui.add_enabled(have_sel, egui::Button::new(format!("Copy ({} key time{})", n_sel, if n_sel == 1 { "" } else { "s" }))).clicked() {
                action = 1;
            }
            if ui.add_enabled(have_sel, egui::Button::new("Cut")).clicked() {
                action = 2;
            }
            if ui.add_enabled(have_sel, egui::Button::new("Delete")).clicked() {
                action = 3;
            }
            ui.separator();
            if ui.add_enabled(have_clip, egui::Button::new(format!("Paste at Current Time (frame {})", cur_frame))).clicked() {
                action = 4;
            }
            if ui.add_enabled(have_clip && (here - tl.current).abs() > 0.5, egui::Button::new(format!("Paste Here (frame {})", here as i32))).clicked() {
                action = 5;
            }
            if !have_clip {
                ui.label(egui::RichText::new("Clipboard is empty").color(TEXT_DIM));
            } else {
                ui.label(egui::RichText::new(format!("Clipboard: {} key(s)", n_clip)).color(TEXT_DIM));
            }
            ui.separator();
            if ui.button("Select All Keys").clicked() {
                action = 6;
            }
            if action != 0 {
                ui.close_menu();
            }
        });
        if resp.hovered() {
            let (c, v) = ui.ctx().input(|i| (i.events.iter().any(|e| matches!(e, egui::Event::Copy)), i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))));
            if c && have_sel {
                action = 1;
            } else if v && have_clip {
                action = 4;
            }
        }
        if action != 0 {
            let sel_times = self.key_sel.clone();
            match action {
                1 => match cmd_keys::copy_keys(&mut self.session, &owners, Some(&sel_times), None) {
                    Ok(n) => self.say(&format!("Copied {} key(s)", n)),
                    Err(e) => self.say(&e),
                },
                2 => match cmd_keys::copy_keys(&mut self.session, &owners, Some(&sel_times), None) {
                    Ok(n) => {
                        cmd_keys::delete_keys(&mut self.session, &owners, &sel_times);
                        self.key_sel.clear();
                        self.session.scene.evaluate_frame();
                        self.say(&format!("Cut {} key(s)", n));
                        self.scene_edited();
                    }
                    Err(e) => self.say(&e),
                },
                3 => {
                    let n = cmd_keys::delete_keys(&mut self.session, &owners, &sel_times);
                    self.key_sel.clear();
                    self.session.scene.evaluate_frame();
                    self.say(&format!("Deleted {} key(s)", n));
                    self.scene_edited();
                }
                4 | 5 => {
                    let at = if action == 4 { self.session.scene.timeline.current } else { here };
                    match cmd_keys::paste_keys(&mut self.session, at, &owners) {
                        Ok(t) => {
                            self.say(&format!("Pasted keys at frame {}", at as i32));
                            self.key_sel = t;
                            self.scene_edited();
                        }
                        Err(e) => self.say(&e),
                    }
                }
                _ => self.key_sel = times.clone(),
            }
        }
        if let Some(pos) = resp.hover_pos() {
            if near_key(pos.x, pos.y).is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                self.help_line = "Click a key to select it, Shift adds. Drag selected keys to a new frame to retime. Right-click for Copy / Paste.".into();
            } else {
                self.help_line = "Time Slider: click or drag to scrub. Alt+V plays.".into();
            }
        }
    }

    fn range_slider(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let (mut start, mut end, cur) = {
                let t = &self.session.scene.timeline;
                (t.start, t.end, t.current)
            };
            ui.label("Start");
            if ui.add(egui::DragValue::new(&mut start).speed(1.0).max_decimals(0)).changed() {
                let e = self.session.scene.timeline.end;
                self.session.scene.timeline.set_range(start.min(e - 1.0), e);
                self.mark_dirty();
            }
            ui.label("End");
            if ui.add(egui::DragValue::new(&mut end).speed(1.0).max_decimals(0)).changed() {
                let s = self.session.scene.timeline.start;
                self.session.scene.timeline.set_range(s, end.max(s + 1.0));
                self.mark_dirty();
            }
            separator_v(ui);
            let mut c = cur;
            ui.label("Current");
            if ui.add(egui::DragValue::new(&mut c).speed(0.2).max_decimals(0)).changed() {
                self.set_time(c);
            }
            separator_v(ui);
            let playing = self.session.scene.timeline.playing;
            if icon_button(ui, Icon::Rewind, false, "Go to start", 22.0).clicked() {
                let s = self.session.scene.timeline.start;
                self.set_time(s);
            }
            if icon_button(ui, Icon::StepBack, false, "Previous frame (,)", 22.0).clicked() {
                self.step_frame(-1.0);
            }
            if icon_button(ui, Icon::Play, playing, "Play (Alt+V)", 22.0).clicked() {
                self.session.scene.timeline.toggle();
                self.mark_dirty();
            }
            if icon_button(ui, Icon::Pause, false, "Pause", 22.0).clicked() {
                self.session.scene.timeline.pause();
                self.mark_dirty();
            }
            if icon_button(ui, Icon::StepFwd, false, "Next frame (.)", 22.0).clicked() {
                self.step_frame(1.0);
            }
            if icon_button(ui, Icon::End, false, "Go to end", 22.0).clicked() {
                let e = self.session.scene.timeline.end;
                self.set_time(e);
            }
            if icon_button(ui, Icon::Key, false, "Set key (S)", 22.0).clicked() {
                self.key_selected();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let fps = self.session.scene.timeline.fps;
                let looping = self.session.scene.timeline.looping;
                let mut lp = looping;
                ui.label(format!("{} fps", fps));
                if ui.checkbox(&mut lp, "Loop").changed() {
                    self.session.scene.timeline.looping = lp;
                }
            });
        });
    }

    fn command_line(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("MEL").strong());
            let w = (ui.available_width() * 0.45).max(200.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.mel_input)
                    .desired_width(w)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("type a command and press Enter"),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let cmd = std::mem::take(&mut self.mel_input);
                if !cmd.trim().is_empty() {
                    self.exec(&cmd);
                }
                r.request_focus();
            }
            let color = if self.mel_error { Color32::from_rgb(0xa0, 0x10, 0x10) } else { Color32::from_rgb(0x20, 0x50, 0x20) };
            egui::Frame::NONE.fill(FIELD).inner_margin(egui::Margin::symmetric(4, 1)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add(egui::Label::new(egui::RichText::new(&self.mel_result).color(color).monospace()).truncate());
            });
        });
    }
}

// ====================================================================== viewports

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    if l2 < 1e-6 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

const AXIS_COLORS: [Color32; 3] = [Color32::from_rgb(0xe0, 0x2c, 0x2c), Color32::from_rgb(0x2c, 0xc0, 0x2c), Color32::from_rgb(0x30, 0x48, 0xe8)];
const HOT: Color32 = Color32::from_rgb(0xff, 0xea, 0x20);

fn ring_basis(a: Vec3) -> (Vec3, Vec3) {
    let helper = if a.y.abs() < 0.9 { Vec3::new(0.0, 1.0, 0.0) } else { Vec3::new(1.0, 0.0, 0.0) };
    let u = a.cross(helper).normalized();
    let v = a.cross(u).normalized();
    (u, v)
}

impl App {
    fn rs(&mut self) -> crate::render::RenderScene {
        match self.rs_cache.take() {
            Some((g, rs)) if g == self.gen => rs,
            _ => viewport::scene_snapshot(&mut self.session.scene),
        }
    }

    fn put_rs(&mut self, rs: crate::render::RenderScene) {
        self.rs_cache = Some((self.gen, rs));
    }

    fn manip_frame(&mut self, cam: &Cam, rect: Rect) -> Option<ManipFrame> {
        if !matches!(self.tool, Tool::Move | Tool::Rotate | Tool::Scale) || !self.session.scene.show.manipulators {
            return None;
        }
        let mut comp_normal: Option<Vec3> = None;
        let (world, pivot) = if self.session.comp.mode != SelMode::Object {
            let (p, n, w) = self.comp_pivot()?;
            if self.axis_normal {
                comp_normal = Some(n);
            }
            (w, p)
        } else {
            let ts = self.selected_transforms();
            let id = *ts.first()?;
            let world = self.session.scene.world_matrix(id);
            (world, self.session.scene.pivot_world(id))
        };
        let vp = cam.view_proj();
        let (w, h) = (rect.width(), rect.height());
        let (sx, sy, depth) = project(&vp, pivot, w, h)?;
        let upp = if cam.kind == ViewKind::Persp {
            let fov = cam.info.as_ref().map(|i| i.fov_deg).unwrap_or(45.0);
            2.0 * depth * (fov.to_radians() * 0.5).tan() / h
        } else {
            2.0 * cam.size / h
        };
        let len_world = 85.0 * upp * self.session.prefs.manip_size.clamp(0.5, 3.0);
        let axes = if let Some(n) = comp_normal {
            let helper = if n.y.abs() < 0.9 { Vec3::new(0.0, 1.0, 0.0) } else { Vec3::new(1.0, 0.0, 0.0) };
            let x = helper.cross(n).normalized();
            let y = n.cross(x).normalized();
            [x, y, n]
        } else if self.tool == Tool::Scale {
            let col = |c: usize| Vec3::new(world.m[c][0], world.m[c][1], world.m[c][2]).normalized();
            [col(0), col(1), col(2)]
        } else {
            [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)]
        };
        let mut ends = [Pos2::ZERO; 3];
        for i in 0..3 {
            let (ex, ey, _) = project(&vp, pivot + axes[i] * len_world, w, h)?;
            ends[i] = rect.min + vec2(ex, ey);
        }
        let v = &cam.view;
        let right = Vec3::new(v.m[0][0], v.m[1][0], v.m[2][0]);
        let up = Vec3::new(v.m[0][1], v.m[1][1], v.m[2][1]);
        Some(ManipFrame {
            pivot,
            center: rect.min + vec2(sx, sy),
            axes,
            ends,
            len_world,
            upp,
            right,
            up,
            eye_dir: (cam.eye - pivot).normalized(),
        })
    }

    fn ring_points(&self, mf: &ManipFrame, i: usize, cam: &Cam, rect: Rect) -> Vec<Pos2> {
        let (u, v) = ring_basis(mf.axes[i]);
        let r = mf.len_world * 0.85;
        let vp = cam.view_proj();
        let mut out = Vec::with_capacity(49);
        for k in 0..=48 {
            let t = k as f32 / 48.0 * std::f32::consts::TAU;
            let p = mf.pivot + (u * t.cos() + v * t.sin()) * r;
            if let Some((x, y, _)) = project(&vp, p, rect.width(), rect.height()) {
                out.push(rect.min + vec2(x, y));
            }
        }
        out
    }

    fn manip_hit(&self, mf: &ManipFrame, cam: &Cam, rect: Rect, pos: Pos2) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        let mut consider = |d: f32, i: usize, tol: f32| {
            if d <= tol && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, i));
            }
        };
        match self.tool {
            Tool::Rotate => {
                for i in 0..3 {
                    let pts = self.ring_points(mf, i, cam, rect);
                    for w in pts.windows(2) {
                        consider(seg_dist(pos, w[0], w[1]), i, 8.0);
                    }
                }
            }
            _ => {
                for i in 0..3 {
                    consider(seg_dist(pos, mf.center, mf.ends[i]), i, 9.0);
                    if self.tool == Tool::Scale {
                        consider((pos - mf.ends[i]).length(), i, 10.0);
                    }
                }
                consider((pos - mf.center).length() * 0.8, 3, 9.0);
            }
        }
        best.map(|(_, i)| i)
    }

    fn draw_manip(&self, painter: &egui::Painter, mf: &ManipFrame, cam: &Cam, rect: Rect, hot: Option<usize>) {
        let col = |i: usize| if hot == Some(i) { HOT } else { AXIS_COLORS[i] };
        match self.tool {
            Tool::Rotate => {
                for i in 0..3 {
                    let pts = self.ring_points(mf, i, cam, rect);
                    painter.add(egui::Shape::line(pts, Stroke::new(if hot == Some(i) { 3.0 } else { 2.0 }, col(i))));
                }
                painter.circle_stroke(mf.center, 3.0, Stroke::new(1.0_f32, Color32::BLACK));
            }
            _ => {
                for i in 0..3 {
                    let w = if hot == Some(i) { 3.0 } else { 2.0 };
                    painter.line_segment([mf.center, mf.ends[i]], Stroke::new(w, col(i)));
                    let dir = (mf.ends[i] - mf.center).normalized();
                    let n = vec2(-dir.y, dir.x);
                    if self.tool == Tool::Move {
                        let tip = mf.ends[i] + dir * 14.0;
                        painter.add(egui::Shape::convex_polygon(
                            vec![tip, mf.ends[i] + n * 5.0, mf.ends[i] - n * 5.0],
                            col(i),
                            Stroke::new(1.0_f32, Color32::BLACK),
                        ));
                    } else {
                        painter.rect(Rect::from_center_size(mf.ends[i], vec2(10.0, 10.0)), 0.0, col(i), Stroke::new(1.0_f32, Color32::BLACK), eframe::egui::StrokeKind::Inside);
                    }
                }
                let c = if hot == Some(3) { HOT } else { Color32::from_rgb(0xe8, 0xe8, 0x80) };
                painter.rect(Rect::from_center_size(mf.center, vec2(11.0, 11.0)), 0.0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 170), Stroke::new(1.0_f32, Color32::BLACK), eframe::egui::StrokeKind::Inside);
            }
        }
    }

    fn begin_manip(&mut self, mf: &ManipFrame, axis: usize, origin: Pos2) -> Option<ManipDrag> {
        let mut comp: Option<CompDrag> = None;
        let mut ts = self.selected_transforms();
        if self.session.comp.mode != SelMode::Object {
            let shape = self.session.comp.node?;
            let (poly, parent) = match self.session.scene.node(shape) {
                Some(n) => match &n.data {
                    NodeData::Mesh(md) => (md.poly(), n.parent),
                    _ => return None,
                },
                None => return None,
            };
            let world = match parent {
                Some(p) => self.session.scene.world_matrix(p),
                None => Mat4::identity(),
            };
            let c = &self.session.comp;
            let verts = cmd_model::verts_of(&poly, &c.sel, c.mode);
            if verts.is_empty() {
                return None;
            }
            let list: Vec<(u32, Vec3)> = verts.iter().map(|&v| (v, world.transform_point(poly.pos[v as usize]))).collect();
            comp = Some(CompDrag { shape, verts: list, world, inv: world.inverse() });
            ts.clear();
        } else if ts.is_empty() {
            return None;
        }
        let mut starts = Vec::new();
        let mut pivot_starts = Vec::new();
        for id in ts {
            pivot_starts.push((id, self.session.scene.pivot_world(id)));
            let g = |s: &mut Scene, a: &str| s.get_attr(id, a).unwrap_or(0.0);
            let sc = &mut self.session.scene;
            let t = Vec3::new(g(sc, "translateX"), g(sc, "translateY"), g(sc, "translateZ"));
            let r = Vec3::new(g(sc, "rotateX"), g(sc, "rotateY"), g(sc, "rotateZ"));
            let s = Vec3::new(g(sc, "scaleX"), g(sc, "scaleY"), g(sc, "scaleZ"));
            let parent = sc.node(id).and_then(|n| n.parent);
            let pw = match parent {
                Some(p) => sc.world_matrix(p),
                None => Mat4::identity(),
            };
            starts.push((id, t, r, s, pw));
        }
        let (axis_world, axis_dir, units_per_px) = if axis < 3 {
            let d = mf.ends[axis] - mf.center;
            let len = d.length().max(1.0);
            (mf.axes[axis], d / len, mf.len_world / len)
        } else {
            (Vec3::ZERO, vec2(1.0, 0.0), mf.upp)
        };
        let sign = if axis < 3 && mf.axes[axis].dot(mf.eye_dir) < 0.0 { -1.0 } else { 1.0 };
        let start_angle = (-(origin.y - mf.center.y)).atan2(origin.x - mf.center.x);
        Some(ManipDrag {
            tool: self.tool,
            axis,
            origin,
            frame_center: mf.center,
            axis_world,
            axis_dir,
            units_per_px,
            right: mf.right,
            up: mf.up,
            upp: mf.upp,
            start_angle,
            angle_sign: sign,
            starts,
            comp,
            pivot: mf.pivot,
            pivot_starts,
            snap_pts: if self.session.prefs.snap_point && self.tool == Tool::Move { self.snap_points() } else { Vec::new() },
            changed: false,
        })
    }

    /// World positions a move can snap to: mesh vertices, joints and locators.
    fn snap_points(&mut self) -> Vec<Vec3> {
        let mut pts: Vec<Vec3> = Vec::new();
        let rs = self.rs();
        for it in &rs.items {
            for p in &it.mesh.positions {
                pts.push(it.world.transform_point(*p));
            }
        }
        self.put_rs(rs);
        pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z)));
        pts.dedup_by(|a, b| (*a - *b).length_sq() < 1e-10);
        pts.truncate(300_000);
        let sc = &mut self.session.scene;
        for id in sc.ids_where(|d| matches!(d, NodeData::Joint | NodeData::Locator)) {
            let t = match sc.node(id) {
                Some(n) if n.data.is_transform() => id,
                Some(n) => n.parent.unwrap_or(id),
                None => continue,
            };
            pts.push(sc.world_matrix(t).translation_part());
        }
        pts
    }

    /// Apply grid and point snapping to a move of the manipulator, returning the corrected world delta.
    fn snap_move(&self, d: &ManipDrag, pos: Pos2, wd: Vec3) -> Vec3 {
        let prefs = &self.session.prefs;
        if prefs.snap_point && !d.snap_pts.is_empty() {
            // the point nearest to the cursor on screen wins, if it is close enough
            let mut best: Option<(f32, Vec3)> = None;
            for p in &d.snap_pts {
                let off = *p - d.pivot;
                let sx = d.frame_center.x + off.dot(d.right) / d.upp.max(1e-6);
                let sy = d.frame_center.y - off.dot(d.up) / d.upp.max(1e-6);
                let dist = ((sx - pos.x).powi(2) + (sy - pos.y).powi(2)).sqrt();
                if dist < 14.0 && best.map(|b| dist < b.0).unwrap_or(true) {
                    best = Some((dist, *p));
                }
            }
            if let Some((_, p)) = best {
                let off = p - d.pivot;
                return if d.axis < 3 { d.axis_world * off.dot(d.axis_world) } else { d.right * off.dot(d.right) + d.up * off.dot(d.up) + (wd - d.right * wd.dot(d.right) - d.up * wd.dot(d.up)) };
            }
        }
        if prefs.snap_grid && prefs.move_step > 1e-6 {
            let st = prefs.move_step;
            let snap = |v: f32| (v / st).round() * st;
            let target = d.pivot + wd;
            if d.axis < 3 {
                let ax = d.axis_world;
                // snap the coordinate along the world axis the handle follows
                let i = if ax.x.abs() > 0.9 { 0 } else if ax.y.abs() > 0.9 { 1 } else { 2 };
                let mut t = [target.x, target.y, target.z];
                t[i] = snap(t[i]);
                return Vec3::new(t[0], t[1], t[2]) - d.pivot;
            }
            return Vec3::new(snap(target.x), snap(target.y), snap(target.z)) - d.pivot;
        }
        wd
    }

    fn apply_manip(&mut self, d: &mut ManipDrag, pos: Pos2) {
        let delta = pos - d.origin;
        if let Some(cd) = &d.comp {
            let new_pts: Vec<(u32, Vec3)> = match d.tool {
                Tool::Move => {
                    let wd = if d.axis < 3 {
                        d.axis_world * (delta.dot(d.axis_dir) * d.units_per_px)
                    } else {
                        d.right * (delta.x * d.upp) + d.up * (-delta.y * d.upp)
                    };
                    cd.verts.iter().map(|(i, w)| (*i, *w + wd)).collect()
                }
                Tool::Rotate if d.axis < 3 => {
                    let ang = (-(pos.y - d.frame_center.y)).atan2(pos.x - d.frame_center.x);
                    let mut da = ang - d.start_angle;
                    while da > std::f32::consts::PI {
                        da -= std::f32::consts::TAU;
                    }
                    while da < -std::f32::consts::PI {
                        da += std::f32::consts::TAU;
                    }
                    let a = da * d.angle_sign;
                    let k = d.axis_world.normalized();
                    let (c, s) = (a.cos(), a.sin());
                    cd.verts
                        .iter()
                        .map(|(i, w)| {
                            let r = *w - d.pivot;
                            let rr = r * c + k.cross(r) * s + k * (k.dot(r) * (1.0 - c));
                            (*i, d.pivot + rr)
                        })
                        .collect()
                }
                Tool::Scale => {
                    let (f_axis, f_all) = if d.axis < 3 {
                        ((1.0 + delta.dot(d.axis_dir) / 85.0).max(0.001), 1.0)
                    } else {
                        (1.0, (1.0 + (delta.x - delta.y) / 150.0).max(0.001))
                    };
                    let k = d.axis_world.normalized();
                    cd.verts
                        .iter()
                        .map(|(i, w)| {
                            let r = *w - d.pivot;
                            let rr = if d.axis < 3 { r + k * (k.dot(r) * (f_axis - 1.0)) } else { r * f_all };
                            (*i, d.pivot + rr)
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            let (shape, inv) = (cd.shape, cd.inv);
            let edits: Vec<(u32, Vec3)> = new_pts.into_iter().map(|(i, w)| (i, inv.transform_point(w))).collect();
            let _ = self.session.scene.tweak_vertices(shape, &edits);
            d.changed = true;
            self.scene_edited();
            return;
        }
        for (id, t0, r0, s0, pw) in &d.starts {
            match d.tool {
                Tool::Move => {
                    let world_delta = if d.axis < 3 {
                        d.axis_world * (delta.dot(d.axis_dir) * d.units_per_px)
                    } else {
                        d.right * (delta.x * d.upp) + d.up * (-delta.y * d.upp)
                    };
                    let world_delta = self.snap_move(d, pos, world_delta);
                    if self.pivot_edit {
                        // pivot edit mode: the pivot moves, the object stays
                        if let Some((_, p0)) = d.pivot_starts.iter().find(|(i, _)| i == id) {
                            let _ = crate::mel::cmd_extra::set_pivot_world(&mut self.session, *id, *p0 + world_delta);
                        }
                        continue;
                    }
                    let local = pw.inverse().transform_dir(world_delta);
                    let t = *t0 + local;
                    for (a, v) in [("translateX", t.x), ("translateY", t.y), ("translateZ", t.z)] {
                        let _ = self.session.scene.set_attr(*id, a, v);
                    }
                }
                Tool::Rotate => {
                    if d.axis < 3 {
                        let ang = (-(pos.y - d.frame_center.y)).atan2(pos.x - d.frame_center.x);
                        let mut da = ang - d.start_angle;
                        while da > std::f32::consts::PI {
                            da -= std::f32::consts::TAU;
                        }
                        while da < -std::f32::consts::PI {
                            da += std::f32::consts::TAU;
                        }
                        let mut deg = da.to_degrees() * d.angle_sign;
                        if self.session.prefs.snap_grid && self.session.prefs.rotate_step > 1e-6 {
                            deg = (deg / self.session.prefs.rotate_step).round() * self.session.prefs.rotate_step;
                        }
                        let mut r = *r0;
                        match d.axis {
                            0 => r.x += deg,
                            1 => r.y += deg,
                            _ => r.z += deg,
                        }
                        for (a, v) in [("rotateX", r.x), ("rotateY", r.y), ("rotateZ", r.z)] {
                            let _ = self.session.scene.set_attr(*id, a, v);
                        }
                    }
                }
                Tool::Scale => {
                    let mut s = *s0;
                    let step = if self.session.prefs.snap_grid { self.session.prefs.scale_step } else { 0.0 };
                    let q = |f: f32| if step > 1e-6 { ((f / step).round() * step).max(step) } else { f };
                    if d.axis < 3 {
                        let f = q((1.0 + delta.dot(d.axis_dir) / (85.0)).max(0.001));
                        match d.axis {
                            0 => s.x *= f,
                            1 => s.y *= f,
                            _ => s.z *= f,
                        }
                    } else {
                        let f = q((1.0 + (delta.x - delta.y) / 150.0).max(0.001));
                        s = *s0 * f;
                    }
                    for (a, v) in [("scaleX", s.x), ("scaleY", s.y), ("scaleZ", s.z)] {
                        let _ = self.session.scene.set_attr(*id, a, v);
                    }
                }
                _ => {}
            }
        }
        d.changed = true;
        self.scene_edited();
    }

    fn auto_key_manip(&mut self, d: &ManipDrag) {
        if !self.auto_key || !d.changed {
            return;
        }
        let names: [&str; 3] = match d.tool {
            Tool::Move => ["translateX", "translateY", "translateZ"],
            Tool::Rotate => ["rotateX", "rotateY", "rotateZ"],
            _ => ["scaleX", "scaleY", "scaleZ"],
        };
        let layer = self.session.scene.anim.active_layer;
        for (id, ..) in &d.starts {
            for a in names {
                let _ = self.session.scene.set_keyframe_noeval(*id, a, None, None, self.default_interp(), layer);
            }
        }
        self.session.scene.evaluate_anim();
        self.scene_edited();
    }

    fn select_ids(&mut self, ids: Vec<NodeId>, add: bool) {
        let sel = &mut self.session.scene.selection;
        if add {
            for id in ids {
                if let Some(p) = sel.iter().position(|s| *s == id) {
                    sel.remove(p);
                } else {
                    sel.push(id);
                }
            }
        } else {
            *sel = ids;
        }
        self.mark_dirty();
    }

    fn parent_transform(&self, id: NodeId) -> NodeId {
        let sc = &self.session.scene;
        match sc.node(id) {
            Some(n) if !n.data.is_transform() => n.parent.unwrap_or(id),
            _ => id,
        }
    }

    /// Helper objects (lights, joints, locators...) that are shown and can be picked.
    fn pickable_helpers(&self) -> Vec<NodeId> {
        let s = &self.session.scene.show;
        let (lights, cams, joints, locs, ems, iks) = (s.lights, s.cameras, s.joints, s.locators, s.emitters, s.ik_handles);
        self.session.scene.ids_where(|d| match d {
            NodeData::Light => lights,
            NodeData::Camera | NodeData::StereoCamera => cams,
            NodeData::Joint => joints,
            NodeData::Locator => locs,
            NodeData::Emitter => ems,
            NodeData::IkHandle(_) => iks,
            _ => false,
        })
    }

    fn pick_at(&mut self, cam: &Cam, rect: Rect, pos: Pos2, add: bool) {
        let local = pos - rect.min;
        let (nx, ny) = (local.x / rect.width(), local.y / rect.height());
        let vp = cam.view_proj();
        let rs = self.rs();
        let mesh_hit = vp_render::pick_matrix(&rs, &vp, nx, ny);
        self.put_rs(rs);
        // lights and cameras are tiny: pick them by distance to their origin
        let mut near: Option<(f32, NodeId)> = None;
        let helpers = self.pickable_helpers();
        for id in helpers {
            let t = self.parent_transform(id);
            if self.session.scene.name_of(t) == "persp" {
                continue;
            }
            let p = self.session.scene.world_matrix(t).translation_part();
            if let Some((x, y, _)) = project(&vp, p, rect.width(), rect.height()) {
                let d = (rect.min + vec2(x, y) - pos).length();
                if d < 16.0 && near.map(|(bd, _)| d < bd).unwrap_or(true) {
                    near = Some((d, t));
                }
            }
        }
        // NURBS curves: pick by distance to the drawn line
        for id in self.session.scene.ids_where(|d| matches!(d, NodeData::NurbsCurve(_))) {
            if !self.session.scene.is_visible(id) {
                continue;
            }
            let t = self.parent_transform(id);
            let world = self.session.scene.world_matrix(t);
            let pts: Vec<Pos2> = match self.session.scene.node(id).map(|n| &n.data) {
                Some(NodeData::NurbsCurve(cd)) => cd.curve.sample(6).into_iter().filter_map(|p| project(&vp, world.transform_point(p), rect.width(), rect.height()).map(|(x, y, _)| rect.min + vec2(x, y))).collect(),
                _ => continue,
            };
            for w in pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let ab = b - a;
                let tt = if ab.length_sq() > 1e-6 { ((pos - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
                let d = (pos - (a + ab * tt)).length();
                if d < 9.0 && near.map(|(bd, _)| d < bd).unwrap_or(true) {
                    near = Some((d, t));
                }
            }
        }
        let hit = near.map(|(_, t)| t).or_else(|| mesh_hit.map(|m| self.parent_transform(m)));
        match hit {
            Some(t) => self.select_ids(vec![t], add),
            None if !add => self.select_ids(vec![], false),
            None => {}
        }
    }

    fn marquee_select(&mut self, cam: &Cam, rect: Rect, a: Pos2, b: Pos2, add: bool) {
        let sel_rect = Rect::from_two_pos(a, b);
        let vp = cam.view_proj();
        let (w, h) = (rect.width(), rect.height());
        let inside = |p: Vec3| -> bool {
            project(&vp, p, w, h).map(|(x, y, _)| sel_rect.contains(rect.min + vec2(x, y))).unwrap_or(false)
        };
        let rs = self.rs();
        let mut ids: Vec<NodeId> = Vec::new();
        for it in &rs.items {
            if it.mesh.positions.iter().any(|p| inside(it.world.transform_point(*p))) {
                ids.push(it.node);
            }
        }
        self.put_rs(rs);
        let mut out: Vec<NodeId> = ids.into_iter().map(|i| self.parent_transform(i)).collect();
        for id in self.session.scene.ids_where(|d| matches!(d, NodeData::NurbsCurve(_))) {
            let t = self.parent_transform(id);
            let world = self.session.scene.world_matrix(t);
            let hit = match self.session.scene.node(id).map(|n| &n.data) {
                Some(NodeData::NurbsCurve(cd)) => cd.curve.sample(4).into_iter().any(|p| inside(world.transform_point(p))),
                _ => false,
            };
            if hit {
                out.push(t);
            }
        }
        let helpers = self.pickable_helpers();
        for id in helpers {
            let t = self.parent_transform(id);
            if self.session.scene.name_of(t) == "persp" {
                continue;
            }
            let p = self.session.scene.world_matrix(t).translation_part();
            if inside(p) {
                out.push(t);
            }
        }
        out.dedup();
        self.select_ids(out, add);
    }

    fn viewport_bar(&mut self, ui: &mut Ui, kind: ViewKind) {
        egui::menu::bar(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.menu_button("View", |ui| {
                if ui.add(egui::Button::new("Frame All").shortcut_text("A")).clicked() {
                    self.active = kind;
                    self.frame(false);
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Frame Selection").shortcut_text("F")).clicked() {
                    self.active = kind;
                    self.frame(true);
                    ui.close_menu();
                }
                if kind == ViewKind::Persp {
                    ui.separator();
                    let cams: Vec<String> = {
                        let sc = &self.session.scene;
                        let mut v = Vec::new();
                        for id in sc.ids_where(|d| matches!(d, NodeData::Camera | NodeData::StereoCamera)) {
                            if let Some(p) = sc.node(id).and_then(|n| n.parent) {
                                v.push(sc.name_of(p));
                            }
                        }
                        v
                    };
                    for c in cams {
                        let label = if c == "persp" { "Perspective (orbit camera)".to_string() } else { format!("Look through {}", c) };
                        let cur = self.session.view.camera.clone().unwrap_or_else(|| "persp".into()) == c;
                        if ui.radio(cur, label).clicked() {
                            self.session.view.camera = if c == "persp" { None } else { Some(c) };
                            self.mark_dirty();
                            ui.close_menu();
                        }
                    }
                }
            });
            ui.menu_button("Predefined Bookmarks", |ui| {
                for (l, yaw, pitch) in [("Perspective", 45.0, 26.8), ("Front", 0.0, 0.0), ("Side", 90.0, 0.0), ("Top", 0.0, 89.0)] {
                    if ui.button(l).clicked() {
                        self.session.view.camera = None;
                        self.session.view.yaw = yaw;
                        self.session.view.pitch = pitch;
                        self.mark_dirty();
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Bookmarks", |ui| {
                if ui.button("Add Bookmark").clicked() {
                    let n = format!("bookmark{}", self.session.bookmarks.len() + 1);
                    self.exec(&format!("cameraBookmark -add {}", n));
                    ui.close_menu();
                }
                let names: Vec<String> = self.session.bookmarks.iter().map(|b| b.0.clone()).collect();
                if !names.is_empty() {
                    ui.separator();
                }
                for n in names {
                    if ui.button(&n).clicked() {
                        self.exec(&format!("cameraBookmark -go {}", n));
                        self.mark_dirty();
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Lighting", |ui| {
                if ui.radio(self.session.scene.use_all_lights, "Use All Lights").on_hover_text("Shaded views are lit by the scene's lights").clicked() {
                    self.session.scene.use_all_lights = true;
                    self.mark_dirty();
                    ui.close_menu();
                }
                if ui.radio(!self.session.scene.use_all_lights, "Use Default Lighting").clicked() {
                    self.session.scene.use_all_lights = false;
                    self.mark_dirty();
                    ui.close_menu();
                }
                ui.separator();
                ui.checkbox(&mut self.res_gate, "Resolution Gate").on_hover_text("Frame the render size in this view");
            });
            ui.menu_button("Shading", |ui| {
                let mut xj = self.session.scene.show.xray_joints;
                if ui.checkbox(&mut xj, "X-Ray Joints").on_hover_text("Joints show through the surfaces").changed() {
                    self.session.scene.show.xray_joints = xj;
                    self.mark_dirty();
                }
                ui.separator();
                for (m, l) in [(DisplayMode::Wireframe, "Wireframe"), (DisplayMode::Shaded, "Smooth Shade All"), (DisplayMode::Textured, "Textured"), (DisplayMode::Anaglyph, "Stereo Anaglyph")] {
                    if ui.radio(self.session.view.mode == m, l).clicked() {
                        self.set_mode(m);
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Show", |ui| {
                let mut wos = self.session.view.wire_on_shaded;
                if ui.checkbox(&mut wos, "Wireframe on Shaded").changed() {
                    self.session.view.wire_on_shaded = wos;
                    self.mark_dirty();
                }
                ui.checkbox(&mut self.hud_poly, "Poly Count");
                let mut g = self.session.view.grid;
                if ui.checkbox(&mut g, "Grid").changed() {
                    self.session.view.grid = g;
                    self.mark_dirty();
                }
                ui.separator();
                self.show_type_items(ui);
            });
            ui.menu_button("Layouts", |ui| {
                for (l, m) in [("Single Pane", LayoutMode::Single), ("Four Panes", LayoutMode::Four), ("Persp / Outliner", LayoutMode::PerspOutliner)] {
                    if ui.radio(self.layout == m, l).clicked() {
                        self.layout = m;
                        self.mark_dirty();
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Save Current Layout").clicked() {
                    let n = format!("layout{}", self.saved_layouts.len() + 1);
                    self.saved_layouts.push((n, self.layout, self.single_kind));
                    ui.close_menu();
                }
                let saved = self.saved_layouts.clone();
                if !saved.is_empty() {
                    ui.menu_button("Saved Layouts", |ui| {
                        for (n, m, k) in saved {
                            if ui.button(&n).clicked() {
                                self.layout = m;
                                self.single_kind = k;
                                self.mark_dirty();
                                ui.close_menu();
                            }
                        }
                    });
                }
            });
            if self.layout != LayoutMode::Four {
                ui.menu_button("Panels", |ui| {
                    for k in KINDS {
                        if ui.radio(self.single_kind == k, k.label()).clicked() {
                            self.single_kind = k;
                            self.mark_dirty();
                            ui.close_menu();
                        }
                    }
                });
            }
        });
    }

    fn viewport_ui(&mut self, ui: &mut Ui, kind: ViewKind, outer: Rect) {
        let ctx = ui.ctx().clone();
        let bar_h = 22.0;
        let bar = Rect::from_min_size(outer.min, vec2(outer.width(), bar_h));
        let rect = Rect::from_min_max(pos2(outer.min.x, outer.min.y + bar_h), outer.max);
        if rect.width() < 40.0 || rect.height() < 40.0 {
            return;
        }
        let idx = kind as usize;
        let painter = ui.painter_at(outer);
        painter.rect_filled(bar, 0.0, CHROME);
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar).id_salt(("vpbar", idx)), |ui| {
            self.viewport_bar(ui, kind);
        });

        let resp = ui.interact(rect, ui.id().with(("vpview", idx)), Sense::click_and_drag());
        if resp.hovered() || resp.dragged() {
            if self.layout == LayoutMode::Four {
                self.active = kind;
            }
        }
        let aspect = rect.width() / rect.height();
        let view = self.session.view.clone();
        let ortho = if kind == ViewKind::Persp { Ortho::default() } else { self.ortho[idx - 1] };
        let cam = match make_cam(&mut self.session.scene, &view, kind, &ortho, aspect) {
            Some(c) => c,
            None => return,
        };

        // ---- render
        let ppp = ctx.pixels_per_point();
        let (wf, hf) = (rect.width() * ppp, rect.height() * ppp);
        let hq = !self.interacting;
        let scale = if hq { 1.0 } else { (350_000.0 / (wf * hf)).sqrt().clamp(0.3, 1.0) };
        let (w, h) = ((wf * scale).max(8.0) as usize, (hf * scale).max(8.0) as usize);
        let ss = if hq && w * h <= 1_300_000 { 2 } else { 1 };
        let need = {
            let p = &self.panels[idx];
            p.tex.is_none() || p.size != (w, h) || p.gen != self.gen || (hq && !p.hq)
        };
        if need {
            let mut rs = self.rs();
            rs.use_textures = view.textured();
            let out = viewport::render(&rs, &cam, view.mode, view.grid, true, view.wire_on_shaded, self.stereo_scale, w, h, ss);
            self.put_rs(rs);
            let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &out.rgba);
            let p = &mut self.panels[idx];
            match &mut p.tex {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => p.tex = Some(ctx.load_texture(format!("vp{}", idx), img, TextureOptions::LINEAR)),
            }
            p.size = (w, h);
            p.hq = hq;
            p.gen = self.gen;
            p.depth = out.depth;
            p.dsize = (out.dw, out.dh);
        }
        if let Some(t) = &self.panels[idx].tex {
            painter.image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }

        // ---- HUD
        let cam_name = match (kind, &view.camera) {
            (ViewKind::Persp, Some(c)) => c.clone(),
            _ => kind.label().to_string(),
        };
        painter.text(pos2(rect.center().x, rect.bottom() - 6.0), Align2::CENTER_BOTTOM, cam_name, FontId::proportional(13.0), Color32::from_rgb(0x18, 0x18, 0x18));
        painter.text(
            rect.min + vec2(8.0, 6.0),
            Align2::LEFT_TOP,
            format!("{}   frame {}", view.mode.name(), self.session.scene.timeline.frame()),
            FontId::proportional(11.0),
            Color32::from_rgb(0x1c, 0x1c, 0x1c),
        );
        // axis indicator
        {
            let o = pos2(rect.left() + 34.0, rect.bottom() - 34.0);
            for (i, (name, a)) in [("X", Vec3::new(1.0, 0.0, 0.0)), ("Y", Vec3::new(0.0, 1.0, 0.0)), ("Z", Vec3::new(0.0, 0.0, 1.0))].iter().enumerate() {
                let d = cam.view.transform_dir(*a);
                let e = o + vec2(d.x, -d.y) * 24.0;
                painter.line_segment([o, e], Stroke::new(2.0_f32, AXIS_COLORS[i]));
                painter.text(e + vec2(d.x, -d.y) * 7.0, Align2::CENTER_CENTER, *name, FontId::proportional(10.0), AXIS_COLORS[i]);
            }
        }
        let active_panel = self.layout != LayoutMode::Four || self.active == kind;
        painter.rect_stroke(outer, 0.0, Stroke::new(if active_panel && self.layout == LayoutMode::Four { 2.0 } else { 1.0 }, if active_panel { Color32::from_rgb(0x30, 0x30, 0x30) } else { CHROME_DARK }), eframe::egui::StrokeKind::Inside);

        // ---- components
        let (alt, shift, ctrl) = ctx.input(|i| (i.modifiers.alt, i.modifiers.shift, i.modifiers.command));
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let comp_active = self.session.comp.mode != SelMode::Object;
        let tool_geo = matches!(self.tool, Tool::SplitPoly | Tool::EdgeLoop);
        let cv = if comp_active || tool_geo {
            self.comp_view(&cam, rect, idx, view.mode == DisplayMode::Wireframe)
        } else {
            None
        };
        let hit_mode = if comp_active { self.session.comp.mode } else { SelMode::Edge };
        if let Some(cv) = &cv {
            let hov = match hover {
                Some(hp) if rect.contains(hp) && matches!(self.drag, Drag::None) && !alt => {
                    if self.tool == Tool::EdgeLoop {
                        self.comp_hit(cv, hp, SelMode::Edge)
                    } else if tool_geo || matches!(self.tool, Tool::Select) || matches!(self.tool, Tool::Move | Tool::Rotate | Tool::Scale) {
                        self.comp_hit(cv, hp, hit_mode)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            self.draw_components(&painter, cv, hit_mode, comp_active, hov.as_ref());
            self.draw_tool_preview(&painter, cv, &cam, rect, hover);
        }
        if self.tool == Tool::Curve && !self.nurbs_ui.pts.is_empty() {
            self.draw_curve_preview(&painter, &cam, rect);
        }
        if self.hud_poly && (self.layout != LayoutMode::Four || self.active == kind) {
            self.draw_poly_hud(&painter, rect);
        }
        self.draw_path_markers(&painter, &cam, rect);
        if self.res_gate && kind == ViewKind::Persp {
            // the area the render will cover, from the Render Settings size
            let st = crate::render::pipeline::Settings::from_scene(&mut self.session.scene, None);
            let ra = st.w as f32 / st.h as f32;
            let va = rect.width() / rect.height();
            let (gw, gh) = if ra > va { (rect.width(), rect.width() / ra) } else { (rect.height() * ra, rect.height()) };
            let gate = Rect::from_center_size(rect.center(), vec2(gw, gh));
            let shade = Color32::from_black_alpha(90);
            for r in [Rect::from_min_max(rect.min, pos2(rect.max.x, gate.min.y)), Rect::from_min_max(pos2(rect.min.x, gate.max.y), rect.max), Rect::from_min_max(pos2(rect.min.x, gate.min.y), pos2(gate.min.x, gate.max.y)), Rect::from_min_max(pos2(gate.max.x, gate.min.y), pos2(rect.max.x, gate.max.y))] {
                if r.width() > 0.0 && r.height() > 0.0 {
                    painter.rect_filled(r, 0.0, shade);
                }
            }
            painter.rect_stroke(gate, 0.0, Stroke::new(1.0, Color32::from_rgb(0xe0, 0xe0, 0x60)), egui::StrokeKind::Inside);
        }

        // ---- manipulator
        let mf = self.manip_frame(&cam, rect);
        let mut hot: Option<usize> = None;
        if let Drag::Manip(d) = &self.drag {
            hot = Some(d.axis);
        } else if let (Some(m), Some(hp)) = (&mf, hover) {
            if rect.contains(hp) {
                hot = self.manip_hit(m, &cam, rect, hp);
            }
        }
        if let Some(m) = &mf {
            self.draw_manip(&painter, m, &cam, rect, hot);
        }

        // ---- navigation
        let d = resp.drag_delta();
        let panning = resp.dragged_by(PointerButton::Middle);
        let zooming = resp.dragged_by(PointerButton::Secondary);
        let orbiting = resp.dragged_by(PointerButton::Primary) && alt;
        if panning || zooming || orbiting {
            self.nav(kind, d, panning, zooming, &cam, rect);
        }
        if resp.hovered() {
            let sc = ctx.input(|i| i.smooth_scroll_delta.y);
            if sc.abs() > 0.0 {
                self.nav_zoom(kind, (-sc * 0.0025).exp());
            }
        }

        // ---- tools
        let manip_tool = matches!(self.tool, Tool::Move | Tool::Rotate | Tool::Scale);
        if resp.drag_started_by(PointerButton::Primary) && !alt {
            let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or_else(|| hover.unwrap_or(rect.center()));
            self.drag = Drag::None;
            if manip_tool {
                if let Some(m) = &mf {
                    if let Some(axis) = self.manip_hit(m, &cam, rect, origin) {
                        if let Some(md) = self.begin_manip(m, axis, origin) {
                            self.drag = Drag::Manip(md);
                        }
                    }
                }
            }
            if matches!(self.drag, Drag::None) {
                match self.tool {
                    Tool::EdgeLoop => {
                        if let Some(cv) = &cv {
                            if let Some(CompHit::Edge(e)) = self.comp_hit(cv, origin, SelMode::Edge) {
                                self.loop_drag = Some((e, 0.5));
                            }
                        }
                    }
                    Tool::SplitPoly => {}
                    Tool::Paint => self.paint_at(&cam, rect, origin, true),
                    Tool::Curve => {}
                    Tool::PaintSkin => self.skin_paint_at(&cam, rect, origin),
                    Tool::PaintFx => {
                        self.pfx_ui.path.clear();
                        self.pfx_paint_at(&cam, rect, origin);
                    }
                    Tool::Sculpt => self.sculpt_at(&cam, rect, origin, true),
                    _ => self.drag = Drag::Marquee { start: origin, kind },
                }
            }
        }
        if resp.dragged_by(PointerButton::Primary) && !alt {
            if let Some(pos) = resp.interact_pointer_pos() {
                if let (Some((e, _)), Some(cv)) = (self.loop_drag, &cv) {
                    let t = self.edge_param(cv, e, pos);
                    self.loop_drag = Some((e, t));
                }
                if self.tool == Tool::Paint {
                    self.paint_at(&cam, rect, pos, false);
                }
                if self.tool == Tool::PaintSkin {
                    self.skin_paint_at(&cam, rect, pos);
                }
                if self.tool == Tool::PaintFx {
                    self.pfx_paint_at(&cam, rect, pos);
                }
                if self.tool == Tool::Sculpt {
                    self.sculpt_at(&cam, rect, pos, false);
                }
                match std::mem::replace(&mut self.drag, Drag::None) {
                    Drag::Manip(mut md) => {
                        self.apply_manip(&mut md, pos);
                        self.drag = Drag::Manip(md);
                    }
                    Drag::Marquee { start, kind: k } => {
                        if k == kind {
                            let r = Rect::from_two_pos(start, pos).intersect(rect);
                            painter.rect(r, 0.0, Color32::from_rgba_unmultiplied(0x40, 0x60, 0xc0, 40), Stroke::new(1.0_f32, Color32::from_rgb(0xf0, 0xf0, 0xf0)), eframe::egui::StrokeKind::Inside);
                        }
                        self.drag = Drag::Marquee { start, kind: k };
                    }
                    Drag::None => {}
                }
            }
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            if self.tool == Tool::PaintFx {
                self.pfx_finish_stroke();
            }
            if let Some((e, t)) = self.loop_drag.take() {
                self.model(ModelOp::InsertLoop { edge: e, t });
            }
            match std::mem::replace(&mut self.drag, Drag::None) {
                Drag::Manip(md) => self.auto_key_manip(&md),
                Drag::Marquee { start, kind: k } if k == kind => {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        if comp_active {
                            if let Some(cv) = &cv {
                                let op = if ctrl { SelOp::Remove } else if shift { SelOp::Toggle } else { SelOp::Replace };
                                self.comp_marquee(cv, Rect::from_two_pos(start, pos), op);
                            }
                        } else {
                            self.marquee_select(&cam, rect, start, pos, shift);
                        }
                    }
                }
                _ => {}
            }
        }
        if resp.clicked_by(PointerButton::Primary) && !alt {
            if let Some(pos) = resp.interact_pointer_pos() {
                match self.tool {
                    Tool::Joint => self.joint_click(&cam, rect, pos),
                    Tool::Prim => self.prim_click(&cam, rect, pos),
                    Tool::Curve => self.curve_click(&cam, rect, pos),
                    Tool::IkHandle => self.ik_click(&cam, rect, pos),
                    Tool::SplitPoly => {
                        if let Some(cv) = &cv {
                            self.split_click(cv, pos);
                        }
                    }
                    Tool::EdgeLoop => {
                        if let Some(cv) = &cv {
                            if let Some(CompHit::Edge(e)) = self.comp_hit(cv, pos, SelMode::Edge) {
                                let t = self.edge_param(cv, e, pos);
                                self.model(ModelOp::InsertLoop { edge: e, t });
                            }
                        }
                    }
                    _ if comp_active => {
                        if let Some(cv) = &cv {
                            let op = if ctrl { SelOp::Remove } else if shift { SelOp::Toggle } else { SelOp::Replace };
                            self.comp_click(cv, pos, op);
                        }
                    }
                    _ => self.pick_at(&cam, rect, pos, shift),
                }
            }
        }
        if resp.double_clicked_by(PointerButton::Primary) && comp_active && self.session.comp.mode == SelMode::Edge && self.tool == Tool::Select {
            self.model(ModelOp::SelectLoop);
        }
        if resp.secondary_clicked() && !alt && self.session.comp.mode == SelMode::Object {
            if let Some(pos) = resp.interact_pointer_pos() {
                let (nx, ny) = ((pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height());
                let rs = self.rs();
                let hit = vp_render::pick_matrix(&rs, &cam.view_proj(), nx, ny);
                self.put_rs(rs);
                if let Some(m) = hit {
                    let t = self.parent_transform(m);
                    if !self.session.scene.selection.contains(&t) {
                        self.session.scene.selection = vec![t];
                        self.mark_dirty();
                    }
                }
            }
        }
        resp.context_menu(|ui| {
            self.context_menu(ui);
        });
        if resp.hovered() {
            self.help_line = match self.tool {
                Tool::Select if comp_active => "Select Tool: click a component, drag for a marquee, Shift toggles, Ctrl removes, double-click an edge for its loop.".into(),
                Tool::Select => "Select Tool: click to select, drag for a marquee, Shift adds. Alt+drag orbits, middle drag pans, right drag or wheel zooms.".into(),
                Tool::Move => "Move Tool: drag an arrow to move along an axis, the center square moves in the view plane.".into(),
                Tool::Rotate => "Rotate Tool: drag a ring to rotate about its axis.".into(),
                Tool::Scale => "Scale Tool: drag a handle to scale along an axis, the center box scales uniformly.".into(),
                Tool::SplitPoly => "Split Polygon Tool: click on edges or vertices to place points, Enter to cut, Esc to cancel.".into(),
                Tool::EdgeLoop => "Insert Edge Loop Tool: click or drag across an edge to place a loop, release to insert.".into(),
                Tool::Sculpt => "Sculpt Geometry Tool: drag over the selected surface (settings in Mesh > Sculpt Geometry Tool).".into(),
                Tool::PaintFx => "Paint Effects Tool: drag over a paintable surface (Paint Effects > Make Paintable) to paint a stroke with the current brush.".into(),
                Tool::PaintSkin => "Paint Skin Weights: drag over the mesh (Window: Skin > Paint Skin Weights Tool).".into(),
                Tool::Curve => "Curve Tool: click points on the grid, Enter creates the curve, Backspace-free: Esc clears the points.".into(),
                Tool::Paint => "3D Paint Tool: drag over the object to paint its texture (Window > 3D Paint Tool for the brush).".into(),
                Tool::Prim => "Interactive creation: click the first corner (the center for round shapes), then click again to set the size. Esc cancels.".into(),
                Tool::Joint => "Joint Tool: click to place joints in a chain (grid plane in perspective). Enter finishes the chain.".into(),
                Tool::IkHandle => "IK Handle Tool: click the start joint, then the end joint of the chain.".into(),
            };
        }
    }

    fn nav(&mut self, kind: ViewKind, d: egui::Vec2, pan: bool, zoom: bool, cam: &Cam, rect: Rect) {
        match kind {
            ViewKind::Persp => {
                let fov = cam.info.as_ref().map(|i| i.fov_deg).unwrap_or(45.0);
                let f = 2.0 * (fov.to_radians() * 0.5).tan() / rect.height();
                let v = &mut self.session.view;
                if pan {
                    v.pan(-d.x * f, d.y * f);
                } else if zoom {
                    v.zoom(((-d.x + d.y) * 0.006).exp());
                } else {
                    v.orbit(-d.x * 0.35, d.y * 0.35);
                }
                v.apply(&mut self.session.scene);
            }
            k => {
                let o = &mut self.ortho[k as usize - 1];
                let upp = 2.0 * o.size / rect.height();
                if zoom {
                    o.size = (o.size * ((-d.x + d.y) * 0.006).exp()).clamp(0.2, 3000.0);
                } else {
                    let (right, up) = match k {
                        ViewKind::Top => (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -1.0)),
                        ViewKind::Front => (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)),
                        _ => (Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 1.0, 0.0)),
                    };
                    o.target += right * (-d.x * upp) + up * (d.y * upp);
                }
            }
        }
        self.mark_dirty();
    }

    fn nav_zoom(&mut self, kind: ViewKind, f: f32) {
        match kind {
            ViewKind::Persp => {
                self.session.view.zoom(f);
                let v = self.session.view.clone();
                v.apply(&mut self.session.scene);
            }
            k => {
                let o = &mut self.ortho[k as usize - 1];
                o.size = (o.size * f).clamp(0.2, 3000.0);
            }
        }
        self.mark_dirty();
    }
}

// ====================================================================== windows and frame loop

const HELP_ROWS: &[(&str, &str)] = &[
    ("Q / W / E / R", "Select / Move / Rotate / Scale tool"),
    ("Alt + left drag", "Orbit (perspective view)"),
    ("Middle drag", "Pan"),
    ("Right drag / wheel", "Zoom"),
    ("F / A", "Frame selection / frame all"),
    ("4 / 5 / 6 / 7", "Wireframe / smooth shaded / textured (lit, with color maps) / stereo anaglyph"),
    ("Space", "Toggle single and four view"),
    ("S", "Set key on the selection"),
    ("Alt + V", "Play / pause"),
    (", and .", "Previous / next frame"),
    ("Ctrl + Z / Ctrl + Y", "Undo / redo"),
    ("Ctrl + D / Ctrl + G", "Duplicate / group"),
    ("Delete", "Delete selection"),
    ("Ctrl + N / O / S", "New / open / save scene"),
    ("Ctrl + A", "Toggle the Attribute Editor"),
    ("F2 / F3 / F6", "Menu set: Animation / Polygons / Rendering"),
    ("F8", "Toggle object / component selection"),
    ("F9 / F10 / F11", "Vertex / edge / face selection"),
    ("Ctrl + F9 / F10 / F11", "Convert selection to vertices / edges / faces"),
    ("Ctrl + E", "Extrude faces (edges in edge mode)"),
    ("Ctrl + B", "Bevel selected edges"),
    ("Delete", "Delete faces, edges or vertices in component mode"),
    ("P / Shift + P", "Parent selection to the last selected / unparent"),
    ("Double-click edge", "Select the edge loop"),
    ("G", "Repeat last modeling operation"),
    ("Enter / Esc", "Finish / cancel Split Polygon tool"),
];

impl App {
    fn toggle_layout(&mut self) {
        if self.layout == LayoutMode::Four {
            self.single_kind = self.active;
            self.layout = LayoutMode::Single;
        } else {
            self.layout = LayoutMode::Four;
            self.show_outliner = false;
        }
        self.mark_dirty();
    }

    /// Custom hotkeys from the preferences: true when one fired.
    fn run_hotkeys(&mut self, ctx: &egui::Context) -> bool {
        let keys: Vec<crate::session::Hotkey> = self.session.prefs.hotkeys.clone();
        for h in keys {
            let Some(key) = egui::Key::from_name(&h.key) else { continue };
            let hit = ctx.input(|i| i.key_pressed(key) && i.modifiers.command == h.ctrl && i.modifiers.alt == h.alt && i.modifiers.shift == h.shift);
            if hit {
                self.exec(&h.command);
                return true;
            }
        }
        false
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        // Space: a tap toggles the four view layout, holding it opens the hotbox (as in Maya)
        let (down, now) = ctx.input(|i| (i.key_down(Key::Space), i.time));
        if down && !self.space_prev {
            self.space_at = now;
        }
        if down && self.hotbox.is_none() && now - self.space_at > 0.25 {
            self.hotbox = Some(ctx.input(|i| i.pointer.latest_pos()).unwrap_or(pos2(400.0, 300.0)));
        }
        if down {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
        if !down && self.space_prev {
            if self.hotbox.take().is_none() {
                self.toggle_layout();
            }
        }
        self.space_prev = down;
        if self.run_hotkeys(ctx) {
            return;
        }
        struct K {
            q: bool,
            w: bool,
            e: bool,
            r: bool,
            f: bool,
            a: bool,
            s: bool,
            k4: bool,
            k1: bool,
            k2: bool,
            k3: bool,
            k5: bool,
            k6: bool,
            k7: bool,
            space: bool,
            del: bool,
            comma: bool,
            period: bool,
            z: bool,
            y: bool,
            d: bool,
            g: bool,
            h: bool,
            ins: bool,
            n: bool,
            o: bool,
            sv: bool,
            v: bool,
            quit: bool,
            ctrl: bool,
            alt: bool,
            shift: bool,
            f2: bool,
            f3: bool,
            f4: bool,
            f5: bool,
            f6: bool,
            f8: bool,
            f9: bool,
            f10: bool,
            f11: bool,
            enter: bool,
            esc: bool,
            b: bool,
            p: bool,
        }
        let k = ctx.input(|i| K {
            q: i.key_pressed(Key::Q),
            w: i.key_pressed(Key::W),
            e: i.key_pressed(Key::E),
            r: i.key_pressed(Key::R),
            f: i.key_pressed(Key::F),
            a: i.key_pressed(Key::A),
            s: i.key_pressed(Key::S),
            k4: i.key_pressed(Key::Num4),
            k1: i.key_pressed(Key::Num1),
            k2: i.key_pressed(Key::Num2),
            k3: i.key_pressed(Key::Num3),
            k5: i.key_pressed(Key::Num5),
            k6: i.key_pressed(Key::Num6),
            k7: i.key_pressed(Key::Num7),
            space: i.key_pressed(Key::Space),
            del: i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
            comma: i.key_pressed(Key::Comma),
            period: i.key_pressed(Key::Period),
            z: i.key_pressed(Key::Z),
            y: i.key_pressed(Key::Y),
            d: i.key_pressed(Key::D),
            g: i.key_pressed(Key::G),
            h: i.key_pressed(Key::H),
            ins: i.key_pressed(Key::Insert),
            n: i.key_pressed(Key::N),
            o: i.key_pressed(Key::O),
            sv: i.key_pressed(Key::S),
            v: i.key_pressed(Key::V),
            quit: i.key_pressed(Key::Q),
            ctrl: i.modifiers.command,
            alt: i.modifiers.alt,
            shift: i.modifiers.shift,
            f2: i.key_pressed(Key::F2),
            f3: i.key_pressed(Key::F3),
            f4: i.key_pressed(Key::F4),
            f5: i.key_pressed(Key::F5),
            f6: i.key_pressed(Key::F6),
            f8: i.key_pressed(Key::F8),
            f9: i.key_pressed(Key::F9),
            f10: i.key_pressed(Key::F10),
            f11: i.key_pressed(Key::F11),
            enter: i.key_pressed(Key::Enter),
            esc: i.key_pressed(Key::Escape),
            b: i.key_pressed(Key::B),
            p: i.key_pressed(Key::P),
        });
        // menu sets and component masks (Maya: F2 / F3 / F6, F8 .. F11)
        if !k.ctrl {
            if k.f2 {
                self.menu_set = MenuSet::Animation;
            } else if k.f3 {
                self.menu_set = MenuSet::Polygons;
            } else if k.f4 {
                self.menu_set = MenuSet::Surfaces;
            } else if k.f5 {
                self.menu_set = MenuSet::Dynamics;
            } else if k.f6 {
                self.menu_set = MenuSet::Rendering;
            } else if k.f8 {
                if self.session.comp.mode == SelMode::Object {
                    let m = self.last_comp_mode;
                    self.set_sel_mode(m);
                } else {
                    self.set_sel_mode(SelMode::Object);
                }
            } else if k.f9 {
                self.set_sel_mode(SelMode::Vertex);
            } else if k.f10 {
                self.set_sel_mode(SelMode::Edge);
            } else if k.f11 {
                self.set_sel_mode(SelMode::Face);
            }
        }
        if self.tool == Tool::SplitPoly {
            if k.enter {
                self.finish_split();
                return;
            }
            if k.esc {
                if self.split_pts.is_empty() {
                    self.start_tool(Tool::Select);
                } else {
                    self.split_pts.clear();
                }
                return;
            }
        } else if self.tool == Tool::Curve {
            if k.enter {
                self.finish_curve();
                return;
            }
            if k.esc {
                if self.nurbs_ui.pts.is_empty() {
                    self.start_tool(Tool::Select);
                } else {
                    self.nurbs_ui.pts.clear();
                }
                return;
            }
        } else if k.esc && matches!(self.tool, Tool::EdgeLoop | Tool::IkHandle | Tool::Prim | Tool::Paint | Tool::PaintSkin | Tool::PaintFx | Tool::Sculpt) {
            self.start_tool(Tool::Select);
            return;
        } else if self.tool == Tool::Joint {
            if k.enter {
                self.joint_last = None;
                self.say("Joint chain finished");
                return;
            }
            if k.esc {
                self.start_tool(Tool::Select);
                return;
            }
        }
        if k.ctrl {
            if k.f9 {
                self.model(ModelOp::Convert(SelMode::Vertex));
            } else if k.f10 {
                self.model(ModelOp::Convert(SelMode::Edge));
            } else if k.f11 {
                self.model(ModelOp::Convert(SelMode::Face));
            } else if k.a {
                self.show_ae = !self.show_ae;
            } else if k.e {
                self.model(ModelOp::Extrude { ltz: 0.0 });
            } else if k.b {
                self.model(ModelOp::Bevel { offset: 0.2 });
            } else if k.del {
                if self.session.comp.mode != SelMode::Object {
                    self.model(ModelOp::Delete { clean: true });
                }
            } else if k.quit {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if k.z && k.shift {
                self.do_redo();
            } else if k.z {
                self.do_undo();
            } else if k.y {
                self.do_redo();
            } else if k.d {
                self.exec("duplicate");
            } else if k.g {
                self.exec("group");
            } else if k.h {
                self.exec("hide");
            } else if k.n {
                self.exec("file -new");
                self.session.view = ViewState::new();
                self.session.view.apply(&mut self.session.scene);
                self.undo.clear();
                self.redo.clear();
                self.last_state = serialize::to_ascii(&self.session.scene);
            } else if k.o {
                self.open_file_dialog(false);
            } else if k.sv {
                if self.session.scene.file_path.is_some() {
                    self.exec("file -save");
                } else {
                    self.open_file_dialog(true);
                }
            }
            return;
        }
        if k.alt {
            if k.v {
                self.session.scene.timeline.toggle();
                self.mark_dirty();
            }
            return;
        }
        if k.q {
            self.start_tool(Tool::Select);
        } else if k.w {
            self.start_tool(Tool::Move);
            self.axis_normal = false;
        } else if k.e {
            self.start_tool(Tool::Rotate);
        } else if k.r {
            self.start_tool(Tool::Scale);
        } else if k.f {
            self.frame(true);
        } else if k.a {
            self.frame(false);
        } else if k.s {
            self.key_selected();
        } else if k.ins {
            self.pivot_edit = !self.pivot_edit;
            self.say(if self.pivot_edit { "Pivot edit mode: drag the Move handles to move the pivot" } else { "Pivot edit mode off" });
        } else if k.k1 || k.k2 || k.k3 {
            // Smooth Mesh Preview of the selected polygon objects
            let lvl = if k.k1 { 1 } else if k.k2 { 2 } else { 3 };
            self.exec(&format!("smoothPreview -l {}", lvl));
        } else if k.k4 {
            self.set_mode(DisplayMode::Wireframe);
        } else if k.k5 {
            self.set_mode(DisplayMode::Shaded);
        } else if k.k6 {
            self.set_mode(DisplayMode::Textured);
        } else if k.k7 {
            self.set_mode(DisplayMode::Anaglyph);
        } else if k.p {
            if k.shift {
                self.unparent_selection();
            } else {
                self.parent_selection();
            }
        } else if k.g {
            if let Some(op) = self.last_op.clone() {
                self.model(op);
            }
        } else if k.del {
            if self.session.comp.mode != SelMode::Object {
                self.model(ModelOp::Delete { clean: true });
            } else {
                self.exec("delete");
            }
        } else if k.comma {
            self.step_frame(-1.0);
        } else if k.period {
            self.step_frame(1.0);
        }
    }

    /// Names of the attributes of a node (and of its shape for transforms) that can be driven or drive.
    fn sdk_attrs(&self, node: NodeId) -> Vec<String> {
        let sc = &self.session.scene;
        let mut v: Vec<String> = Vec::new();
        if let Some(n) = sc.node(node) {
            v.extend(n.plugs.iter().filter(|p| !p.readonly && !p.name.starts_with("pnts_")).map(|p| p.name.clone()));
        }
        v
    }

    fn graph_editor_ui(&mut self, ctx: &egui::Context) {
        let Some(mut g) = self.graph.take() else { return };
        let mut open = true;
        let mut edited = false;
        let sel_nodes = self.session.scene.key_owner_ids(&self.session.scene.selection.clone());
        // channels to show: (binding index, label)
        let chans: Vec<(usize, String, NodeId, String)> = self
            .session
            .scene
            .anim
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| g.all_nodes || sel_nodes.contains(&b.node))
            .map(|(i, b)| (i, format!("{}.{}", self.session.scene.name_of(b.node), b.attr), b.node, b.attr.clone()))
            .collect();
        g.sel.retain(|(bi, ki)| self.session.scene.anim.bindings.get(*bi).map(|b| *ki < b.curve.keys.len()).unwrap_or(false));
        let cur = self.session.scene.timeline.current;
        let mut set_time: Option<f32> = None;
        egui::Window::new("Graph Editor").open(&mut open).default_size([780.0, 440.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut g.all_nodes, "All objects");
                if ui.button("Frame All").clicked() {
                    g.fit_next = true;
                }
                ui.separator();
                for (label, ip) in [("Spline", crate::anim::Interp::Bezier), ("Linear", crate::anim::Interp::Linear), ("Stepped", crate::anim::Interp::Step)] {
                    if ui.add_enabled(!g.sel.is_empty(), egui::Button::new(label)).clicked() {
                        for (bi, ki) in g.sel.clone() {
                            if let Some(b) = self.session.scene.anim.bindings.get_mut(bi) {
                                let t = b.curve.keys[ki].t;
                                let tan = ip.tan();
                                b.curve.set_tangents_at(t, Some(if tan == crate::anim::keyframes::Tan::Step { crate::anim::keyframes::Tan::Clamped } else { tan }), Some(tan));
                            }
                        }
                        edited = true;
                    }
                }
                ui.menu_button("Tangents", |ui| {
                    use crate::anim::keyframes::Tan;
                    for (l, t) in [("Spline", Tan::Spline), ("Clamped", Tan::Clamped), ("Linear", Tan::Linear), ("Flat", Tan::Flat), ("Plateau", Tan::Plateau)] {
                        if ui.add_enabled(!g.sel.is_empty(), egui::Button::new(l)).clicked() {
                            for (bi, ki) in g.sel.clone() {
                                if let Some(b) = self.session.scene.anim.bindings.get_mut(bi) {
                                    let tm = b.curve.keys[ki].t;
                                    b.curve.set_tangents_at(tm, Some(t), Some(t));
                                }
                            }
                            edited = true;
                            ui.close_menu();
                        }
                    }
                    if ui.add_enabled(!g.sel.is_empty(), egui::Button::new("Stepped (out)")).clicked() {
                        for (bi, ki) in g.sel.clone() {
                            if let Some(b) = self.session.scene.anim.bindings.get_mut(bi) {
                                let tm = b.curve.keys[ki].t;
                                b.curve.set_tangents_at(tm, None, Some(Tan::Step));
                            }
                        }
                        edited = true;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Curves", |ui| {
                    use crate::anim::keyframes::Infinity;
                    // the curves of the selected keys, else every visible curve
                    let targets: Vec<usize> = if g.sel.is_empty() { chans.iter().filter(|c| !g.hidden.contains(&(c.2, c.3.clone()))).map(|c| c.0).collect() } else { let mut v: Vec<usize> = g.sel.iter().map(|s| s.0).collect(); v.dedup(); v };
                    for (side, label) in [(true, "Pre Infinity"), (false, "Post Infinity")] {
                        ui.menu_button(label, |ui| {
                            for m in [Infinity::Constant, Infinity::Linear, Infinity::Cycle, Infinity::CycleOffset, Infinity::Oscillate] {
                                if ui.button(m.name()).clicked() {
                                    for &bi in &targets {
                                        if let Some(b) = self.session.scene.anim.bindings.get_mut(bi) {
                                            if side {
                                                b.curve.pre = m;
                                            } else {
                                                b.curve.post = m;
                                            }
                                        }
                                    }
                                    edited = true;
                                    ui.close_menu();
                                }
                            }
                        });
                    }
                });
                if ui.add_enabled(!g.sel.is_empty(), egui::Button::new("Delete Key")).clicked() {
                    let mut by_binding: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
                    for (bi, ki) in &g.sel {
                        by_binding.entry(*bi).or_default().push(*ki);
                    }
                    for (bi, mut ks) in by_binding {
                        ks.sort_unstable();
                        if let Some(b) = self.session.scene.anim.bindings.get_mut(bi) {
                            for k in ks.into_iter().rev() {
                                b.curve.keys.remove(k);
                            }
                            b.curve.recompute_tangents();
                        }
                    }
                    self.session.scene.anim.bindings.retain(|b| !b.curve.keys.is_empty());
                    g.sel.clear();
                    edited = true;
                }
                if g.sel.len() == 1 {
                    let (bi, ki) = g.sel[0];
                    if let Some(k) = self.session.scene.anim.bindings.get(bi).map(|b| b.curve.keys[ki].clone()) {
                        let (mut t, mut v) = (k.t, k.v);
                        ui.separator();
                        ui.label("Frame");
                        let a = ui.add(egui::DragValue::new(&mut t).speed(0.1)).changed();
                        ui.label("Value");
                        let b2 = ui.add(egui::DragValue::new(&mut v).speed(0.05)).changed();
                        if a || b2 {
                            if let Some(b) = self.session.scene.anim.bindings.get_mut(bi) {
                                let lo = if ki > 0 { b.curve.keys[ki - 1].t + 0.01 } else { f32::MIN };
                                let hi = if ki + 1 < b.curve.keys.len() { b.curve.keys[ki + 1].t - 0.01 } else { f32::MAX };
                                b.curve.keys[ki].t = t.clamp(lo, hi);
                                b.curve.keys[ki].v = v;
                                b.curve.recompute_tangents();
                            }
                            edited = true;
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal_top(|ui| {
                egui::ScrollArea::vertical().id_salt("gechans").max_width(170.0).show(ui, |ui| {
                    ui.set_min_width(160.0);
                    if chans.is_empty() {
                        ui.label(egui::RichText::new("Select an animated object").color(TEXT_DIM));
                    }
                    for (_, label, node, attr) in &chans {
                        let key = (*node, attr.clone());
                        let mut on = !g.hidden.contains(&key);
                        if ui.checkbox(&mut on, label).changed() {
                            if on {
                                g.hidden.remove(&key);
                            } else {
                                g.hidden.insert(key);
                            }
                        }
                    }
                });
                let size = ui.available_size().max(vec2(200.0, 200.0));
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, Color32::from_rgb(0x3a, 0x3a, 0x3a));
                let visible: Vec<&(usize, String, NodeId, String)> = chans.iter().filter(|c| !g.hidden.contains(&(c.2, c.3.clone()))).collect();
                if g.fit_next {
                    g.fit_next = false;
                    let (mut t0, mut t1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
                    for c in &visible {
                        for k in &self.session.scene.anim.bindings[c.0].curve.keys {
                            t0 = t0.min(k.t);
                            t1 = t1.max(k.t);
                            v0 = v0.min(k.v);
                            v1 = v1.max(k.v);
                        }
                    }
                    if t0 <= t1 {
                        let (pt, pv) = (((t1 - t0) * 0.08).max(1.0), ((v1 - v0) * 0.1).max(0.5));
                        g.x0 = t0 - pt;
                        g.x1 = t1 + pt;
                        g.y0 = v0 - pv;
                        g.y1 = v1 + pv;
                    }
                }
                let sx = |t: f32, g: &GraphEd| rect.left() + (t - g.x0) / (g.x1 - g.x0).max(1e-6) * rect.width();
                let sy = |v: f32, g: &GraphEd| rect.bottom() - (v - g.y0) / (g.y1 - g.y0).max(1e-6) * rect.height();
                let tx = |x: f32, g: &GraphEd| g.x0 + (x - rect.left()) / rect.width() * (g.x1 - g.x0);
                let vy = |y: f32, g: &GraphEd| g.y0 + (rect.bottom() - y) / rect.height() * (g.y1 - g.y0);
                // grid
                let nice = |span: f32| -> f32 {
                    let raw = span / 8.0;
                    let p = 10f32.powf(raw.max(1e-6).log10().floor());
                    [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * p).find(|s| *s >= raw).unwrap_or(p * 10.0)
                };
                let (gx, gy) = (nice(g.x1 - g.x0), nice(g.y1 - g.y0));
                let mut t = (g.x0 / gx).ceil() * gx;
                while t <= g.x1 {
                    let x = sx(t, &g);
                    painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0_f32, Color32::from_rgb(0x48, 0x48, 0x48)));
                    painter.text(pos2(x + 2.0, rect.bottom() - 12.0), Align2::LEFT_TOP, format!("{}", (t * 100.0).round() / 100.0), FontId::proportional(10.0), TEXT_DIM);
                    t += gx;
                }
                let mut v = (g.y0 / gy).ceil() * gy;
                while v <= g.y1 {
                    let y = sy(v, &g);
                    painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0_f32, if v.abs() < gy * 0.01 { Color32::from_rgb(0x70, 0x70, 0x70) } else { Color32::from_rgb(0x48, 0x48, 0x48) }));
                    painter.text(pos2(rect.left() + 2.0, y - 12.0), Align2::LEFT_TOP, format!("{}", (v * 100.0).round() / 100.0), FontId::proportional(10.0), TEXT_DIM);
                    v += gy;
                }
                // current time
                let cx = sx(cur, &g);
                painter.line_segment([pos2(cx, rect.top()), pos2(cx, rect.bottom())], Stroke::new(1.5_f32, Color32::from_rgb(0x60, 0x90, 0xe0)));
                // curves and keys
                let mut hit_key: Option<(usize, usize)> = None;
                let pointer = resp.hover_pos();
                let mut best = 9.0f32;
                for c in &visible {
                    let col = if c.3.ends_with('X') {
                        Color32::from_rgb(0xe0, 0x50, 0x50)
                    } else if c.3.ends_with('Y') {
                        Color32::from_rgb(0x60, 0xd0, 0x60)
                    } else if c.3.ends_with('Z') {
                        Color32::from_rgb(0x60, 0x90, 0xf0)
                    } else {
                        Color32::from_rgb(0xe0, 0xc0, 0x50)
                    };
                    let b = &self.session.scene.anim.bindings[c.0];
                    let n = rect.width() as usize / 3;
                    let mut last: Option<Pos2> = None;
                    for i in 0..=n {
                        let t = g.x0 + (g.x1 - g.x0) * i as f32 / n as f32;
                        if let Some(val) = b.curve.eval(t) {
                            let p = pos2(sx(t, &g), sy(val, &g));
                            // the part before the first and after the last key is the infinity: drawn thinner
                            let beyond = b.curve.keys.first().map(|k| t < k.t).unwrap_or(false) || b.curve.keys.last().map(|k| t > k.t).unwrap_or(false);
                            if let Some(l) = last {
                                if p.y.is_finite() && l.y.is_finite() && (p.y - l.y).abs() < 4000.0 {
                                    painter.line_segment([l, p], Stroke::new(if beyond { 0.8 } else { 1.5 }, if beyond { col.gamma_multiply(0.6) } else { col }));
                                }
                            }
                            last = Some(p);
                        }
                    }
                    for (ki, k) in b.curve.keys.iter().enumerate() {
                        let p = pos2(sx(k.t, &g), sy(k.v, &g));
                        let selected = g.sel.contains(&(c.0, ki));
                        painter.rect_filled(Rect::from_center_size(p, vec2(7.0, 7.0)), 0.0, if selected { Color32::from_rgb(0xff, 0xf0, 0x40) } else { col });
                        if let Some(m) = pointer {
                            let d = (m - p).length();
                            if d < best {
                                best = d;
                                hit_key = Some((c.0, ki));
                            }
                        }
                    }
                }
                let shift = ui.ctx().input(|i| i.modifiers.shift);
                // zoom and pan
                if resp.hovered() {
                    let (scroll, mods) = ui.ctx().input(|i| (i.raw_scroll_delta.y, i.modifiers));
                    if scroll != 0.0 {
                        let f = (1.0 - scroll * 0.002).clamp(0.5, 1.5);
                        if let Some(m) = pointer {
                            let (ct, cv) = (tx(m.x, &g), vy(m.y, &g));
                            if !mods.ctrl {
                                g.x0 = ct + (g.x0 - ct) * f;
                                g.x1 = ct + (g.x1 - ct) * f;
                            }
                            if !mods.shift {
                                g.y0 = cv + (g.y0 - cv) * f;
                                g.y1 = cv + (g.y1 - cv) * f;
                            }
                        }
                    }
                }
                let panning = ui.ctx().input(|i| i.pointer.middle_down() || (i.modifiers.alt && i.pointer.primary_down()));
                if resp.dragged() && panning {
                    let d = resp.drag_delta();
                    let (dt, dv) = (-d.x / rect.width() * (g.x1 - g.x0), d.y / rect.height() * (g.y1 - g.y0));
                    g.x0 += dt;
                    g.x1 += dt;
                    g.y0 += dv;
                    g.y1 += dv;
                } else if resp.drag_started() {
                    if let Some(k) = hit_key {
                        if !g.sel.contains(&k) {
                            if !shift {
                                g.sel.clear();
                            }
                            g.sel.push(k);
                        }
                        g.drag = Some((0.0, 0.0));
                    } else {
                        g.drag = None;
                        if let Some(m) = pointer {
                            set_time = Some(tx(m.x, &g).round());
                        }
                    }
                } else if resp.dragged() {
                    if g.drag.is_some() {
                        let d = resp.drag_delta();
                        let (dt, dv) = (d.x / rect.width() * (g.x1 - g.x0), -d.y / rect.height() * (g.y1 - g.y0));
                        for (bi, ki) in g.sel.clone() {
                            if let Some(b) = self.session.scene.anim.bindings.get_mut(bi) {
                                let lo = if ki > 0 { b.curve.keys[ki - 1].t + 0.01 } else { f32::MIN };
                                let hi = if ki + 1 < b.curve.keys.len() { b.curve.keys[ki + 1].t - 0.01 } else { f32::MAX };
                                b.curve.keys[ki].t = (b.curve.keys[ki].t + dt).clamp(lo, hi);
                                b.curve.keys[ki].v += dv;
                                b.curve.recompute_tangents();
                            }
                        }
                        edited = true;
                    } else if let Some(m) = pointer {
                        set_time = Some(tx(m.x, &g).round());
                    }
                }
                if resp.drag_stopped() {
                    g.drag = None;
                }
                if resp.clicked() {
                    match hit_key {
                        Some(k) => {
                            if shift {
                                if let Some(i) = g.sel.iter().position(|s| *s == k) {
                                    g.sel.remove(i);
                                } else {
                                    g.sel.push(k);
                                }
                            } else {
                                g.sel = vec![k];
                            }
                        }
                        None => g.sel.clear(),
                    }
                }
            });
            ui.label(egui::RichText::new("Click a key to select (Shift adds), drag to move, wheel zooms (Ctrl: time only, Shift: value only), Alt+drag or middle drag pans, drag empty space to scrub.").color(TEXT_DIM));
        });
        if let Some(t) = set_time {
            self.set_time(t);
        }
        if edited {
            self.session.scene.evaluate_frame();
            self.scene_edited();
        }
        if open {
            self.graph = Some(g);
        }
    }

    fn sdk_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.sdk.is_some();
        let mut key = false;
        let sel = self.session.scene.selection.first().copied();
        let mut load: Option<bool> = None;
        if let Some(w) = &mut self.sdk {
            let sc = &self.session.scene;
            egui::Window::new("Set Driven Key").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("1. Select the object that drives, press Load Driver. 2. Same for the driven object.");
                ui.label("3. Set the driver to a value, pose the driven attribute, press Key. Repeat for other poses.");
                ui.separator();
                for (is_driver, title) in [(true, "Driver"), (false, "Driven")] {
                    let (node, attr) = if is_driver { (w.driver, &mut w.driver_attr) } else { (w.driven, &mut w.driven_attr) };
                    ui.horizontal(|ui| {
                        if ui.button(format!("Load {}", title)).clicked() {
                            load = Some(is_driver);
                        }
                        ui.label(node.map(|n| sc.name_of(n)).unwrap_or_else(|| "(none)".to_string()));
                    });
                    if let Some(n) = node.and_then(|n| sc.node(n)) {
                        egui::ComboBox::from_id_salt(("sdkattr", is_driver)).selected_text(attr.clone()).show_ui(ui, |ui| {
                            for p in n.plugs.iter().filter(|p| !p.readonly && !p.name.starts_with("pnts_")) {
                                ui.selectable_value(attr, p.name.clone(), &p.name);
                            }
                        });
                    }
                }
                ui.separator();
                ui.add_enabled_ui(w.driver.is_some() && w.driven.is_some(), |ui| {
                    if ui.button("Key").clicked() {
                        key = true;
                    }
                });
            });
        }
        if let (Some(is_driver), Some(id)) = (load, sel) {
            if let Some(w) = &mut self.sdk {
                if is_driver {
                    w.driver = Some(id);
                } else {
                    w.driven = Some(id);
                }
            }
        }
        if key {
            if let Some(w) = &self.sdk {
                if let (Some(a), Some(b)) = (w.driver, w.driven) {
                    let (na, nb) = (self.session.scene.name_of(a), self.session.scene.name_of(b));
                    let cmd = format!("setDrivenKeyframe -cd \"{}.{}\" \"{}.{}\"", na, w.driver_attr, nb, w.driven_attr);
                    self.exec(&cmd);
                }
            }
        }
        if !open {
            self.sdk = None;
        }
    }

    fn prefs_ui(&mut self, ctx: &egui::Context) {
        if !self.show_prefs {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut reload = false;
        egui::Window::new("Preferences").open(&mut open).default_width(380.0).show(ctx, |ui| {
            let p = &mut self.session.prefs;
            egui::CollapsingHeader::new("Interface").default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Manipulator size");
                    ui.add(egui::Slider::new(&mut p.manip_size, 0.5..=3.0));
                });
                ui.label(egui::RichText::new("Show and hide panels in Display > UI Elements.").small().color(TEXT_DIM));
            });
            egui::CollapsingHeader::new("Display").default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Shading when the program starts");
                    egui::ComboBox::from_id_salt("pref_display").selected_text(p.default_display.clone()).show_ui(ui, |ui| {
                        for m in ["shaded", "textured", "wireframe"] {
                            ui.selectable_value(&mut p.default_display, m.to_string(), m);
                        }
                    });
                });
            });
            egui::CollapsingHeader::new("Settings").default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Time unit of new scenes");
                    egui::ComboBox::from_id_salt("pref_time").selected_text(p.time_unit.clone()).show_ui(ui, |ui| {
                        for (n, l) in [("film", "film 24 fps"), ("pal", "pal 25 fps"), ("ntsc", "ntsc 30 fps"), ("show", "show 48 fps"), ("game", "game 15 fps"), ("palf", "palf 50 fps"), ("ntscf", "ntscf 60 fps")] {
                            ui.selectable_value(&mut p.time_unit, n.to_string(), l);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Undo levels (0 = unlimited)");
                    ui.add(egui::DragValue::new(&mut p.undo_limit).range(0..=1000));
                });
                ui.horizontal(|ui| {
                    ui.label("Autosave every (minutes, 0 = off)");
                    ui.add(egui::DragValue::new(&mut p.autosave_minutes).range(0.0..=120.0).speed(0.5));
                });
            });
            egui::CollapsingHeader::new("Snapping").default_open(true).show(ui, |ui| {
                ui.checkbox(&mut p.snap_grid, "Grid snap (moves, rotation and scale step)");
                ui.checkbox(&mut p.snap_point, "Point snap (vertices, joints, locators)");
                for (l, v) in [("Move step (units)", &mut p.move_step), ("Rotate step (degrees)", &mut p.rotate_step), ("Scale step", &mut p.scale_step)] {
                    ui.horizontal(|ui| {
                        ui.label(l);
                        ui.add(egui::DragValue::new(v).speed(0.05).range(0.001..=1000.0));
                    });
                }
            });
            egui::CollapsingHeader::new("Animation").default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Default key tangent");
                    egui::ComboBox::from_id_salt("pref_tan").selected_text(p.key_tangent.clone()).show_ui(ui, |ui| {
                        for m in ["spline", "linear", "step"] {
                            ui.selectable_value(&mut p.key_tangent, m.to_string(), m);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Default in tangent");
                    egui::ComboBox::from_id_salt("pref_tan_in").selected_text(p.tangent_in.clone()).show_ui(ui, |ui| {
                        for m in ["clamped", "spline", "linear", "flat", "plateau"] {
                            ui.selectable_value(&mut p.tangent_in, m.to_string(), m);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Default out tangent");
                    egui::ComboBox::from_id_salt("pref_tan_out").selected_text(p.tangent_out.clone()).show_ui(ui, |ui| {
                        for m in ["clamped", "spline", "linear", "flat", "plateau", "step"] {
                            ui.selectable_value(&mut p.tangent_out, m.to_string(), m);
                        }
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save Preferences").clicked() {
                    save = true;
                }
                if ui.button("Reload Saved").clicked() {
                    reload = true;
                }
            });
            if let Some(path) = crate::session::Prefs::path() {
                ui.label(egui::RichText::new(path.to_string_lossy().to_string()).small().color(TEXT_DIM));
            }
        });
        if reload {
            self.session.prefs = crate::session::Prefs::load();
        }
        if save {
            match self.session.prefs.save() {
                Ok(p) => self.say(&format!("Saved preferences to {}", p.display())),
                Err(e) => self.say(&e),
            }
        }
        if !open {
            self.show_prefs = false;
        }
    }

    fn hotkeys_ui(&mut self, ctx: &egui::Context) {
        if !self.show_hotkeys {
            return;
        }
        let mut open = true;
        let mut remove: Option<usize> = None;
        let mut add = false;
        let mut save = false;
        egui::Window::new("Hotkey Editor").open(&mut open).default_size([520.0, 440.0]).show(ctx, |ui| {
            ui.label("Assign a key to any command. Examples: ctrl+shift+K runs polyCube; F5 runs viewport -mode textured.");
            ui.separator();
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                if self.session.prefs.hotkeys.is_empty() {
                    ui.label(egui::RichText::new("No custom hotkeys yet.").color(TEXT_DIM));
                }
                for (i, h) in self.session.prefs.hotkeys.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let mut label = h.label();
                        if ui.add(egui::TextEdit::singleline(&mut label).desired_width(110.0)).changed() {
                            if let Some(n) = crate::session::Hotkey::parse_label(&label, &h.command) {
                                *h = n;
                            }
                        }
                        ui.add(egui::TextEdit::singleline(&mut h.command).desired_width(280.0));
                        if ui.button("Delete").clicked() {
                            remove = Some(i);
                        }
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.hotkey_new.0).hint_text("ctrl+alt+K").desired_width(110.0));
                ui.add(egui::TextEdit::singleline(&mut self.hotkey_new.1).hint_text("command").desired_width(280.0));
                if ui.button("Add").clicked() {
                    add = true;
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Save Preferences").clicked() {
                    save = true;
                }
            });
            egui::CollapsingHeader::new("Built-in keys").show(ui, |ui| {
                for (k, a) in [
                    ("Q W E R", "Select, Move, Rotate, Scale"),
                    ("F / A", "Frame selection / all"),
                    ("Space (tap)", "Four view layout"),
                    ("Space (hold)", "Hotbox"),
                    ("Insert", "Edit pivot mode"),
                    ("4 5 6 7", "Wireframe, shaded, textured, anaglyph"),
                    ("F8 F9 F10 F11", "Object, vertex, edge, face mode"),
                    ("S", "Set key"),
                    ("Ctrl+D / G / H", "Duplicate / group / hide"),
                    ("Ctrl+Z / Y", "Undo / redo"),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_sized(vec2(130.0, 16.0), egui::Label::new(egui::RichText::new(k).monospace()));
                        ui.label(a);
                    });
                }
            });
        });
        if let Some(i) = remove {
            self.session.prefs.hotkeys.remove(i);
        }
        if add {
            match crate::session::Hotkey::parse_label(&self.hotkey_new.0, &self.hotkey_new.1) {
                Some(h) if egui::Key::from_name(&h.key).is_some() => {
                    self.session.prefs.hotkeys.push(h);
                    self.hotkey_new = (String::new(), String::new());
                }
                Some(h) => self.say(&format!("'{}' is not a key name (try A, F5, Insert, Num1)", h.key)),
                None => self.say("Type a key such as ctrl+alt+K and a command"),
            }
        }
        if save {
            match self.session.prefs.save() {
                Ok(p) => self.say(&format!("Saved hotkeys to {}", p.display())),
                Err(e) => self.say(&e),
            }
        }
        if !open {
            self.show_hotkeys = false;
        }
    }

    /// File > Create Reference / Reference Editor: bring in other scenes under a namespace, unload, reload or remove them.
    fn reference_ui(&mut self, ctx: &egui::Context) {
        if !self.show_refs {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        egui::Window::new("Reference Editor").open(&mut open).default_width(480.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Scene file");
                ui.add(egui::TextEdit::singleline(&mut self.ref_path).desired_width(300.0));
            });
            ui.horizontal(|ui| {
                ui.label("Namespace");
                ui.add(egui::TextEdit::singleline(&mut self.ref_ns).desired_width(120.0));
                if ui.button("Create Reference").clicked() && !self.ref_path.trim().is_empty() {
                    let ns = if self.ref_ns.trim().is_empty() { "ref".to_string() } else { self.ref_ns.trim().to_string() };
                    cmds.push(format!("file -reference -namespace {} {}", ns, Self::quote(self.ref_path.trim())));
                }
            });
            ui.separator();
            let refs: Vec<(String, String, bool)> = self.session.scene.references.iter().map(|r| (r.namespace.clone(), r.path.clone(), r.loaded)).collect();
            if refs.is_empty() {
                ui.label(egui::RichText::new("No references in this scene.").color(TEXT_DIM));
            }
            for (ns, path, loaded) in refs {
                ui.horizontal(|ui| {
                    ui.label(format!("{}  {}  ({})", ns, path, if loaded { "loaded" } else { "unloaded" }));
                    let (l, c) = if loaded { ("Unload", "unloadReference") } else { ("Load", "loadReference") };
                    if ui.small_button(l).clicked() {
                        cmds.push(format!("file -{} {}", c, ns));
                    }
                    if ui.small_button("Remove").clicked() {
                        cmds.push(format!("file -removeReference {}", ns));
                    }
                });
            }
            if !self.ref_msg.is_empty() {
                ui.label(egui::RichText::new(&self.ref_msg).color(TEXT_DIM));
            }
        });
        for c in cmds {
            let o = execute_line(&mut self.session, &c);
            self.ref_msg = o.error.unwrap_or_else(|| o.lines.join(" "));
        }
        self.show_refs = open;
    }

    /// Skin > Edit Rigid Skin > Create Flexor option window.
    fn flexor_ui(&mut self, ctx: &egui::Context) {
        if !self.show_flexor {
            return;
        }
        let mut open = true;
        let mut cmd: Option<String> = None;
        egui::Window::new("Create Flexor Options").open(&mut open).default_width(340.0).show(ctx, |ui| {
            ui.label("Select the joint the flexor belongs to (a joint of a rigid bound skeleton).");
            ui.horizontal(|ui| {
                ui.label("Flexor Type");
                ui.radio(true, "Lattice");
                ui.add_enabled(false, egui::RadioButton::new(false, "Sculpt")).on_disabled_hover_text("Not available");
                ui.add_enabled(false, egui::RadioButton::new(false, "Joint Cluster")).on_disabled_hover_text("Not available");
            });
            ui.checkbox(&mut self.flexor_pos, "Position the Flexor").on_hover_text("Center it on the joint and size it to the neighboring bones");
            ui.horizontal(|ui| {
                ui.label("Divisions S T U");
                for d in self.flexor_divs.iter_mut() {
                    ui.add(egui::DragValue::new(d).range(2..=12));
                }
            });
            if ui.button("Create").clicked() {
                let d = self.flexor_divs;
                cmd = Some(format!("flexor -t lattice -dv {} {} {} -pos {}", d[0], d[1], d[2], self.flexor_pos as u8));
            }
            ui.label(egui::RichText::new("Move and scale the flexor over the bending region; rounding, length and creasing are channels of its transform. Show > Deformers hides it.").small().color(TEXT_DIM));
            if !self.flexor_msg.is_empty() {
                ui.label(egui::RichText::new(&self.flexor_msg).color(TEXT_DIM));
            }
        });
        if let Some(c) = cmd {
            let o = execute_line(&mut self.session, &c);
            self.flexor_msg = o.error.unwrap_or_else(|| format!("Created {}", o.lines.join(" ")));
            self.mark_dirty();
        }
        self.show_flexor = open;
    }

    /// Edit Deformers > Edit Membership Tool / Paint Set Membership / Paint Jiggle Weights: the selected vertices become members or get a weight.
    fn member_ui(&mut self, ctx: &egui::Context) {
        if !self.show_member {
            return;
        }
        let mut open = true;
        let mut cmd: Option<String> = None;
        egui::Window::new("Deformer Membership and Weights").open(&mut open).default_width(380.0).show(ctx, |ui| {
            ui.label("Select vertices of the deformed mesh in component mode, name the deformer node (a cluster, jiggle, softMod ...), then choose.");
            ui.horizontal(|ui| {
                ui.label("Deformer node");
                ui.add(egui::TextEdit::singleline(&mut self.member_node).desired_width(180.0));
                if ui.button("Use selected").clicked() {
                    if let Some(i) = self.session.scene.selection.first() {
                        self.member_node = self.session.scene.name_of(*i);
                    }
                }
            });
            let n = Self::quote(self.member_node.trim());
            ui.horizontal(|ui| {
                if ui.button("Add to deformer").clicked() {
                    cmd = Some(format!("deformerMembership -n {} -add", n));
                }
                if ui.button("Remove from deformer").clicked() {
                    cmd = Some(format!("deformerMembership -n {} -remove", n));
                }
                if ui.button("Set members").clicked() {
                    cmd = Some(format!("deformerMembership -n {} -set", n));
                }
                if ui.button("List members").clicked() {
                    cmd = Some(format!("deformerMembership -q -n {}", n));
                }
            });
            ui.separator();
            ui.add(egui::Slider::new(&mut self.member_value, 0.0..=1.0).text("Weight"));
            ui.horizontal(|ui| {
                if ui.button("Set weight").clicked() {
                    cmd = Some(format!("deformerWeights -n {} -v {}", n, self.member_value));
                }
                if ui.button("Smooth weights").clicked() {
                    cmd = Some(format!("deformerWeights -n {} -v {} -smooth", n, self.member_value));
                }
            });
            if !self.member_msg.is_empty() {
                ui.label(egui::RichText::new(&self.member_msg).color(TEXT_DIM));
            }
        });
        if let Some(c) = cmd {
            let o = execute_line(&mut self.session, &c);
            self.member_msg = o.error.unwrap_or_else(|| o.lines.join(" "));
            self.mark_dirty();
        }
        self.show_member = open;
    }

    fn project_ui(&mut self, ctx: &egui::Context) {
        if !self.show_project {
            return;
        }
        let mut open = true;
        let mut action: Option<&str> = None;
        egui::Window::new("Project").open(&mut open).default_width(460.0).show(ctx, |ui| {
            let cur = self.session.scene.project_dir.as_ref().map(|d| d.to_string_lossy().to_string()).unwrap_or_else(|| "none".to_string());
            ui.label(format!("Current project: {}", cur));
            ui.horizontal(|ui| {
                ui.label("Folder");
                ui.add(egui::TextEdit::singleline(&mut self.project_path).desired_width(330.0));
            });
            ui.horizontal(|ui| {
                if ui.button("Set Project").on_hover_text("Use an existing project folder (it has a workspace.mel)").clicked() {
                    action = Some("set");
                }
                if ui.button("New Project").on_hover_text("Create the folder with scenes, images, sourceimages ... and use it").clicked() {
                    action = Some("new");
                }
            });
            if !self.project_msg.is_empty() {
                ui.label(egui::RichText::new(&self.project_msg).color(TEXT_DIM));
            }
            ui.label(egui::RichText::new("Open and Save dialogs start in the project's scenes folder; image files are found in sourceimages.").small().color(TEXT_DIM));
        });
        if let Some(a) = action {
            let cmd = format!("project -{} {}", a, Self::quote(&self.project_path));
            let o = execute_line(&mut self.session, &cmd);
            self.project_msg = o.error.unwrap_or_else(|| o.lines.join(" "));
        }
        if !open {
            self.show_project = false;
        }
    }

    fn hotbox_ui(&mut self, ctx: &egui::Context) {
        let Some(pos) = self.hotbox else { return };
        let mut cmds: Vec<String> = Vec::new();
        let mut mode: Option<DisplayMode> = None;
        let mut tool: Option<Tool> = None;
        let mut menu_set: Option<MenuSet> = None;
        let mut ui_all: Option<bool> = None;
        let mut layout = false;
        egui::Area::new(egui::Id::new("hotbox")).order(egui::Order::Foreground).fixed_pos(pos - vec2(250.0, 90.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(egui::RichText::new("Hotbox").strong());
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Display").color(TEXT_DIM));
                        for (l, m) in [("Wireframe", DisplayMode::Wireframe), ("Smooth Shade", DisplayMode::Shaded), ("Textured", DisplayMode::Textured)] {
                            if ui.button(l).clicked() {
                                mode = Some(m);
                            }
                        }
                        if ui.button("Four View").clicked() {
                            layout = true;
                        }
                        if ui.button("Hide Selection").clicked() {
                            cmds.push("hide".into());
                        }
                        if ui.button("Show All").clicked() {
                            cmds.push("showHidden -all".into());
                        }
                        if ui.button("Isolate Select").clicked() {
                            cmds.push("isolateSelect".into());
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Select").color(TEXT_DIM));
                        for (l, c) in [("All", "select -all"), ("Deselect", "select -cl"), ("Polygons", "selectAllByType polygon"), ("Lights", "selectAllByType light"), ("Joints", "selectAllByType joint"), ("Hierarchy", "select -hi")] {
                            if ui.button(l).clicked() {
                                cmds.push(c.into());
                            }
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Modify").color(TEXT_DIM));
                        for (l, c) in [("Center Pivot", "xform -cp"), ("Freeze Transformations", "makeIdentity -apply true"), ("Group", "group"), ("Ungroup", "ungroup"), ("Duplicate", "duplicate"), ("Delete History", "deleteHistory"), ("Delete", "delete")] {
                            if ui.button(l).clicked() {
                                cmds.push(c.into());
                            }
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Tools").color(TEXT_DIM));
                        for (l, t) in [("Select", Tool::Select), ("Move", Tool::Move), ("Rotate", Tool::Rotate), ("Scale", Tool::Scale)] {
                            if ui.button(l).clicked() {
                                tool = Some(t);
                            }
                        }
                        ui.label(egui::RichText::new("Menu set").color(TEXT_DIM));
                        for (l, m) in [("Animation", MenuSet::Animation), ("Polygons", MenuSet::Polygons), ("Dynamics", MenuSet::Dynamics), ("Rendering", MenuSet::Rendering)] {
                            if ui.button(l).clicked() {
                                menu_set = Some(m);
                            }
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Interface").color(TEXT_DIM));
                        if ui.button("Hide All UI").clicked() {
                            ui_all = Some(false);
                        }
                        if ui.button("Show All UI").clicked() {
                            ui_all = Some(true);
                        }
                        if ui.button("Frame All").clicked() {
                            cmds.push("frameAll".into());
                        }
                    });
                });
                ui.label(egui::RichText::new("Release Space to close").small().color(TEXT_DIM));
            });
        });
        if let Some(m) = mode {
            self.set_mode(m);
        }
        if let Some(t) = tool {
            self.tool = t;
        }
        if let Some(m) = menu_set {
            self.menu_set = m;
        }
        if let Some(on) = ui_all {
            self.set_ui_elements(on);
        }
        if layout {
            self.toggle_layout();
        }
        for c in cmds {
            self.exec(&c);
        }
    }

    /// Write `name.autosave.aztec` next to the open scene every few minutes while it has unsaved changes.
    fn autosave_tick(&mut self) {
        let mins = self.session.prefs.autosave_minutes;
        if mins <= 0.0 || self.last_autosave.elapsed().as_secs_f32() < mins * 60.0 {
            return;
        }
        self.last_autosave = std::time::Instant::now();
        let Some(path) = self.session.scene.file_path.clone() else { return };
        if !self.scene_edits && self.undo.is_empty() {
            return;
        }
        let dir = match &self.session.scene.project_dir {
            Some(d) if d.join("autosave").is_dir() => d.join("autosave"),
            _ => std::path::Path::new(&path).parent().map(|p| p.to_path_buf()).unwrap_or_default(),
        };
        let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "scene".into());
        let out = dir.join(format!("{}.autosave.aztec", stem));
        if std::fs::write(&out, serialize::to_ascii(&self.session.scene)).is_ok() {
            self.say(&format!("Autosaved {}", out.display()));
        }
    }

    fn dup_special_ui(&mut self, ctx: &egui::Context) {
        let mut run: Option<String> = None;
        let mut close = false;
        let mut open = self.dup_special.is_some();
        if let Some(d) = &mut self.dup_special {
            egui::Window::new("Duplicate Special").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                egui::Grid::new("dupspecial").num_columns(4).show(ui, |ui| {
                    ui.label("Number of copies");
                    ui.add(egui::DragValue::new(&mut d.copies).range(1..=100));
                    ui.end_row();
                    for (label, v) in [("Translate", &mut d.t), ("Rotate", &mut d.r), ("Scale", &mut d.s)] {
                        ui.label(label);
                        for c in v.iter_mut() {
                            ui.add(egui::DragValue::new(c).speed(0.05));
                        }
                        ui.end_row();
                    }
                });
                ui.checkbox(&mut d.group, "Group the copies");
                ui.checkbox(&mut d.instance, "Instance (copies follow the original geometry)");
                ui.horizontal(|ui| {
                    if ui.button("Duplicate Special").clicked() {
                        run = Some(format!(
                            "duplicateSpecial -n {} -t {} {} {} -ro {} {} {} -s {} {} {}{}",
                            d.copies, d.t[0], d.t[1], d.t[2], d.r[0], d.r[1], d.r[2], d.s[0], d.s[1], d.s[2],
                            if d.group { " -group" } else { "" }
                        ) + if d.instance { " -instance" } else { "" });
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        }
        if let Some(c) = run {
            self.exec(&c);
        }
        if !open || close {
            self.dup_special = None;
        }
    }

    fn file_dialog_ui(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut accept: Option<String> = None;
        if let Some(fd) = &mut self.file_dialog {
            let title = if fd.image_plane {
                "Create Image Plane"
            } else if fd.tex_faces.is_some() {
                "Assign Color Map to Faces"
            } else if fd.tex_for.is_some() {
                "Assign Color Map"
            } else if fd.io == 1 {
                "Import"
            } else if fd.io == 2 {
                "Export All"
            } else if fd.io == 3 {
                "Export Selection"
            } else if fd.save {
                "Save Scene"
            } else {
                "Open Scene"
            };
            let tex_mode = fd.tex_for.is_some() || fd.image_plane || fd.tex_faces.is_some();
            let mut open = true;
            egui::Window::new(title).open(&mut open).collapsible(false).default_size([520.0, 380.0]).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Up").clicked() {
                        if let Some(p) = fd.dir.parent() {
                            fd.dir = p.to_path_buf();
                        }
                    }
                    let mut s = fd.dir.to_string_lossy().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY)).changed() {
                        let p = PathBuf::from(&s);
                        if p.is_dir() {
                            fd.dir = p;
                        }
                    }
                });
                let mut entries: Vec<(String, bool)> = std::fs::read_dir(&fd.dir)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .filter_map(|e| {
                                let name = e.file_name().to_string_lossy().to_string();
                                if name.starts_with('.') {
                                    return None;
                                }
                                let is_dir = e.path().is_dir();
                                let lower = name.to_lowercase();
                                let ok = is_dir
                                    || if tex_mode {
                                        [".png", ".jpg", ".jpeg", ".bmp", ".tga", ".gif"].iter().any(|e| lower.ends_with(e))
                                    } else if fd.io != 0 {
                                        lower.ends_with(".obj") || lower.ends_with(".fbx") || (fd.io == 1 && lower.ends_with(".ma"))
                                    } else {
                                        lower.ends_with(".aztec") || lower.ends_with(".mel") || lower.ends_with(".ma")
                                    };
                                ok.then_some((name, is_dir))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
                egui::Frame::NONE.fill(LIST_BG).inner_margin(2.0).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        for (name, is_dir) in &entries {
                            let label = if *is_dir { format!("[{}]", name) } else { name.clone() };
                            let r = ui.selectable_label(!*is_dir && fd.name == *name, label);
                            if r.clicked() {
                                if *is_dir {
                                    fd.dir = fd.dir.join(name);
                                } else {
                                    fd.name = name.clone();
                                }
                            }
                            if r.double_clicked() && !*is_dir {
                                accept = Some(fd.dir.join(name).to_string_lossy().to_string());
                            }
                        }
                    });
                });
                if tex_mode {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Built-in patterns:");
                        for b in crate::render::texture::BUILTINS {
                            if ui.button(b).clicked() {
                                accept = Some(format!("builtin:{}", b));
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("File name");
                    ui.add(egui::TextEdit::singleline(&mut fd.name).desired_width(260.0));
                    if ui.button(match (fd.io, fd.save) { (1, _) => "Import", (2..=3, _) => "Export", (_, true) => "Save", _ => "Open" }).clicked() && !fd.name.is_empty() {
                        accept = Some(fd.dir.join(&fd.name).to_string_lossy().to_string());
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if !open {
                close = true;
            }
        }
        let save = self.file_dialog.as_ref().map(|f| f.save).unwrap_or(false);
        let tex_for = self.file_dialog.as_ref().and_then(|f| f.tex_for);
        let image_plane = self.file_dialog.as_ref().map(|f| f.image_plane).unwrap_or(false);
        if let (Some(path), true) = (accept.clone(), image_plane) {
            self.exec(&format!("imagePlane {}", Self::quote(&path)));
            self.file_dialog = None;
            return;
        }
        let tex_faces = self.file_dialog.as_ref().and_then(|f| f.tex_faces.clone());
        if let (Some(path), Some(comp)) = (accept.clone(), tex_faces) {
            let q = Self::quote(&path);
            self.exec(&format!("assignTexture \"{}\" {}", comp, q));
            self.file_dialog = None;
            return;
        }
        if let (Some(path), Some(mat)) = (accept.clone(), tex_for) {
            let q = Self::quote(&path);
            let mname = self.session.scene.name_of(mat);
            self.exec(&format!("assignTexture \"{}\" {}", mname, q));
            self.file_dialog = None;
            return;
        }
        let io_mode = self.file_dialog.as_ref().map(|f| f.io).unwrap_or(0);
        if let (Some(path), true) = (accept.clone(), io_mode != 0) {
            let q = Self::quote(&path);
            let lower = path.to_lowercase();
            if io_mode == 1 {
                self.exec(&format!("file -import {}", q));
            } else if lower.ends_with(".obj") || lower.ends_with(".fbx") {
                self.exec(&format!("file -export {}{}", q, if io_mode == 3 { " -selected" } else { "" }));
            } else {
                self.say("Choose a file name ending in .fbx or .obj");
                return;
            }
            self.file_dialog = None;
            return;
        }
        if let Some(path) = accept {
            let q = Self::quote(&path);
            if save {
                self.exec(&format!("file -save {}", q));
            } else if path.ends_with(".mel") {
                self.exec(&format!("source {}", q));
            } else {
                self.exec(&format!("file -open {}", q));
                self.session.view.camera = None;
                self.undo.clear();
                self.redo.clear();
                self.last_state = serialize::to_ascii(&self.session.scene);
            }
            close = true;
        }
        if close {
            self.file_dialog = None;
        }
    }

    fn render_view_ui(&mut self, ctx: &egui::Context) {
        let mut do_render = false;
        let mut do_save: Option<String> = None;
        let mut keep = true;
        let mut do_region_render = false;
        let mut do_snapshot = false;
        let mut do_keep = false;
        let mut do_remove = false;
        let mut do_open: Option<String> = None;
        let mut new_region: Option<(usize, usize, usize, usize)> = None;
        let mut click_px: Option<(usize, usize)> = None;
        let mut ipr_changed = false;
        let job_info: Option<(usize, usize)> = self
            .render_job
            .as_ref()
            .map(|j| (j.progress.load(std::sync::atomic::Ordering::Relaxed), j.h));
        if let Some(rv) = &mut self.render_view {
            if rv.tex.is_none() && !rv.rgba.is_empty() {
                let (kw, kh, src) = match rv.kept_view.and_then(|i| rv.kept.get(i)) {
                    Some((_, w, h, d)) => (*w, *h, d.clone()),
                    None => (rv.w, rv.h, rv.rgba.clone()),
                };
                // the window shows opaque colors, or the alpha channel as grey
                let mut view = src;
                for px in view.chunks_exact_mut(4) {
                    if rv.show_alpha {
                        let a = px[3];
                        px[0] = a;
                        px[1] = a;
                        px[2] = a;
                    }
                    px[3] = 255;
                }
                let img = egui::ColorImage::from_rgba_unmultiplied([kw, kh], &view);
                rv.tex = Some(ctx.load_texture("renderview", img, TextureOptions::LINEAR));
            }
            egui::Window::new("Render View").open(&mut keep).default_size([700.0, 520.0]).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Render").clicked() {
                        do_render = true;
                    }
                    egui::ComboBox::from_id_salt("rvres")
                        .selected_text(["640 x 360", "960 x 540", "1280 x 720", "1920 x 1080"][rv.res.min(3)])
                        .show_ui(ui, |ui| {
                            for (i, l) in ["640 x 360", "960 x 540", "1280 x 720", "1920 x 1080"].iter().enumerate() {
                                ui.selectable_value(&mut rv.res, i, *l);
                            }
                        });
                    ui.checkbox(&mut rv.anaglyph, "Stereo anaglyph");
                });
                ui.horizontal(|ui| {
                    ui.label("Renderer");
                    egui::ComboBox::from_id_salt("rvren").selected_text(if rv.pipe { "Render Settings" } else if rv.ray { "Raytrace" } else { "Software (fast)" }).show_ui(ui, |ui| {
                        if ui.selectable_label(rv.pipe, "Render Settings (layers, passes, effects)").clicked() {
                            rv.pipe = true;
                        }
                        if ui.selectable_label(!rv.pipe && !rv.ray, "Software (fast)").clicked() {
                            rv.pipe = false;
                            rv.ray = false;
                        }
                        if ui.selectable_label(!rv.pipe && rv.ray, "Raytrace").clicked() {
                            rv.pipe = false;
                            rv.ray = true;
                        }
                    });
                    if rv.ray && !rv.pipe {
                        egui::ComboBox::from_id_salt("rvq").selected_text(["", "Draft (1x1)", "Normal (2x2)", "High (3x3)", "Production (4x4)"][rv.quality.clamp(1, 4)]).show_ui(ui, |ui| {
                            for (i, l) in ["Draft (1x1)", "Normal (2x2)", "High (3x3)", "Production (4x4)"].iter().enumerate() {
                                ui.selectable_value(&mut rv.quality, i + 1, *l);
                            }
                        });
                    }
                });
                if let Some(j) = &job_info {
                    let done = j.0 as f32 / j.1.max(1) as f32;
                    ui.add(egui::ProgressBar::new(done).text(format!("Raytracing {}%", (done * 100.0) as i32)));
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut rv.save_name).desired_width(200.0));
                    if ui.button("Save PNG").clicked() {
                        do_save = Some(rv.save_name.clone());
                    }
                    ui.label(egui::RichText::new(&rv.note).color(TEXT_DIM));
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Render Region").on_hover_text("Drag a rectangle on the picture first").clicked() {
                        do_region_render = true;
                    }
                    ui.checkbox(&mut rv.auto_region, "Auto Render Region");
                    if ui.button("Snapshot").on_hover_text("A wireframe picture to choose a region on").clicked() {
                        do_snapshot = true;
                    }
                    if ui.button("Keep Image").clicked() {
                        do_keep = true;
                    }
                    if ui.add_enabled(!rv.kept.is_empty(), egui::Button::new("Remove Image")).clicked() {
                        do_remove = true;
                    }
                    if ui.selectable_label(rv.show_alpha, "Alpha").on_hover_text("Display the alpha channel").clicked() {
                        rv.show_alpha = !rv.show_alpha;
                        rv.tex = None;
                    }
                    if ui.checkbox(&mut rv.ipr, "IPR").on_hover_text("Interactive photorealistic rendering: the chosen region is rendered again whenever you run a command").changed() {
                        ipr_changed = true;
                    }
                });
                if !rv.kept.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("Kept images");
                        let n = rv.kept.len();
                        let mut cur = rv.kept_view.map(|i| i + 1).unwrap_or(0);
                        let before = cur;
                        let label = if cur == 0 { "current render".to_string() } else { rv.kept[cur - 1].0.clone() };
                        ui.add(egui::Slider::new(&mut cur, 0..=n).text(label));
                        if cur != before {
                            rv.kept_view = if cur == 0 { None } else { Some(cur - 1) };
                            rv.tex = None;
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut rv.open_path).desired_width(200.0).hint_text("image to open"));
                    if ui.button("Open Image").clicked() {
                        do_open = Some(rv.open_path.clone());
                    }
                });
                ui.separator();
                if let Some(t) = &rv.tex {
                    let tsz = t.size_vec2();
                    let avail = ui.available_size();
                    let aspect = tsz.x / tsz.y.max(1.0);
                    let mut w = avail.x.max(100.0);
                    let mut h = w / aspect;
                    if h > avail.y.max(100.0) {
                        h = avail.y.max(100.0);
                        w = h * aspect;
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click_and_drag());
                    ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    let to_px = |p: Pos2| -> (usize, usize) { (((p.x - r.min.x) / r.width() * tsz.x).clamp(0.0, tsz.x - 1.0) as usize, ((p.y - r.min.y) / r.height() * tsz.y).clamp(0.0, tsz.y - 1.0) as usize) };
                    let from_px = |x: usize, y: usize| -> Pos2 { pos2(r.min.x + x as f32 / tsz.x * r.width(), r.min.y + y as f32 / tsz.y * r.height()) };
                    if let Some((a, b, c, d)) = rv.region {
                        ui.painter().rect_stroke(Rect::from_min_max(from_px(a, b), from_px(c, d)), 0.0, Stroke::new(1.5, Color32::from_rgb(0xff, 0xd0, 0x40)), egui::StrokeKind::Inside);
                    }
                    if resp.drag_stopped() {
                        if let Some(a) = resp.interact_pointer_pos() {
                            let start = ui.input(|i| i.pointer.press_origin()).unwrap_or(a);
                            let (p0, p1) = (to_px(start), to_px(a));
                            let (x0, x1, y0, y1) = (p0.0.min(p1.0), p0.0.max(p1.0), p0.1.min(p1.1), p0.1.max(p1.1));
                            if x1 > x0 + 2 && y1 > y0 + 2 {
                                new_region = Some((x0, y0, x1, y1));
                            }
                        }
                    } else if resp.clicked() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            click_px = Some(to_px(p));
                        }
                    }
                }
            });
            if let Some(name) = do_save {
                let bytes = png::encode_rgba(rv.w, rv.h, &rv.rgba);
                rv.note = match std::fs::write(&name, bytes) {
                    Ok(_) => format!("Saved {}", name),
                    Err(e) => format!("Cannot save: {}", e),
                };
            }
        }
        if do_render {
            self.render_still();
        }
        if let Some(r) = new_region {
            if let Some(rv) = &mut self.render_view {
                rv.region = Some(r);
            }
            if self.render_view.as_ref().map(|r| r.auto_region || r.ipr).unwrap_or(false) {
                do_region_render = true;
            }
        }
        if do_region_render {
            if let Some(r) = self.render_view.as_ref().and_then(|r| r.region) {
                self.start_pipe_render(Some(r));
            } else {
                self.say("Drag a region on the rendered picture first");
            }
        }
        if do_keep {
            if let Some(rv) = &mut self.render_view {
                if !rv.rgba.is_empty() {
                    let n = format!("{} ({}x{})", rv.note.split(" in ").next().unwrap_or("image"), rv.w, rv.h);
                    rv.kept.push((n, rv.w, rv.h, rv.rgba.clone()));
                }
            }
        }
        if do_remove {
            if let Some(rv) = &mut self.render_view {
                if let Some(i) = rv.kept_view.filter(|i| *i < rv.kept.len()).or(rv.kept.len().checked_sub(1)) {
                    rv.kept.remove(i);
                    rv.kept_view = None;
                    rv.tex = None;
                }
            }
        }
        if do_snapshot {
            // a wireframe picture of the view at the render size
            let (w, h) = match &self.render_view {
                Some(rv) if rv.w > 0 => (rv.w, rv.h),
                _ => (640, 480),
            };
            let st = crate::render::pipeline::Settings::from_scene(&mut self.session.scene, None);
            let (w, h) = if self.render_view.as_ref().map(|r| r.pipe).unwrap_or(true) { (st.w, st.h) } else { (w, h) };
            let mut rs = viewport::scene_snapshot(&mut self.session.scene);
            rs.use_textures = false;
            let view = self.session.view.clone();
            if let Some(cam) = make_cam(&mut self.session.scene, &view, ViewKind::Persp, &Ortho::default(), w as f32 / h as f32) {
                let rgba = viewport::render(&rs, &cam, DisplayMode::Wireframe, false, false, true, self.stereo_scale, w, h, 1).rgba;
                if let Some(rv) = &mut self.render_view {
                    rv.rgba = rgba;
                    rv.w = w;
                    rv.h = h;
                    rv.tex = None;
                    rv.kept_view = None;
                    rv.note = format!("Snapshot {}x{}: drag a region, then Render Region", w, h);
                }
            }
        }
        if let Some(path) = do_open {
            match image::open(&path) {
                Ok(img) => {
                    let img = img.to_rgba8();
                    if let Some(rv) = &mut self.render_view {
                        let (w, h) = (img.width() as usize, img.height() as usize);
                        rv.kept.push((format!("{}", std::path::Path::new(&path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or(path.clone())), w, h, img.into_raw()));
                        rv.kept_view = Some(rv.kept.len() - 1);
                        rv.tex = None;
                        rv.note = format!("Opened {}", path);
                    }
                }
                Err(e) => {
                    if let Some(rv) = &mut self.render_view {
                        rv.note = format!("Cannot open: {}", e);
                    }
                }
            }
        }
        if let Some((px, py)) = click_px {
            // clicking an IPR picture selects what is under the pointer and its material
            if self.render_view.as_ref().map(|r| r.ipr).unwrap_or(false) {
                let (w, h) = self.render_view.as_ref().map(|r| (r.w, r.h)).unwrap_or((1, 1));
                let view = self.session.view.clone();
                let hit = crate::ui::vp_render::pick(&mut self.session.scene, &view, px as f32 / w as f32, py as f32 / h as f32, w as f32 / h as f32);
                if let Some(id) = hit {
                    let mat = match self.session.scene.node(id).map(|n| &n.data) {
                        Some(NodeData::Mesh(md)) => md.material,
                        _ => None,
                    };
                    self.session.scene.selection = vec![mat.unwrap_or(id)];
                    self.mark_dirty();
                }
            }
        }
        let _ = ipr_changed;
        if !keep {
            self.render_view = None;
        }
    }

    fn script_editor_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.show_script;
        let mut run: Option<String> = None;
        let mut shelf: Option<(String, String)> = None;
        let mut save: Option<(String, String)> = None;
        let mut source: Option<String> = None;
        egui::Window::new("Script Editor").open(&mut open).default_size([680.0, 460.0]).show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.label("Source Script: type the name or path below");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.script_ui.file).desired_width(160.0));
                        if ui.button("Source Script").clicked() {
                            source = Some(self.script_ui.file.clone());
                            ui.close_menu();
                        }
                    });
                    if ui.button("Save Script (to the scripts folder)").clicked() {
                        save = Some((self.script_ui.file.clone(), self.script_ui.text.clone()));
                        ui.close_menu();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Clear History").clicked() {
                        self.log.clear();
                        ui.close_menu();
                    }
                    if ui.button("Clear Input").clicked() {
                        self.script_ui.text.clear();
                        ui.close_menu();
                    }
                });
                ui.menu_button("History", |ui| {
                    ui.checkbox(&mut self.session.echo_all, "Echo All Commands");
                    ui.checkbox(&mut self.session.err_lines, "Line numbers in errors");
                });
                ui.menu_button("Command", |ui| {
                    if ui.add(egui::Button::new("Execute").shortcut_text("Ctrl+Enter")).clicked() {
                        run = Some(self.script_ui.text.clone());
                        ui.close_menu();
                    }
                    if ui.button("Execute Selection").clicked() {
                        run = Some(self.script_ui.selection());
                        ui.close_menu();
                    }
                });
            });
            let h = ui.available_height();
            egui::Frame::NONE.fill(FIELD).inner_margin(4.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("scripthist").stick_to_bottom(true).max_height((h * 0.5).max(80.0)).auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    for (l, err) in &self.log {
                        let c = if *err { Color32::from_rgb(0xa0, 0x10, 0x10) } else { TEXT };
                        ui.label(egui::RichText::new(l).monospace().color(c));
                    }
                });
            });
            ui.separator();
            let out = egui::ScrollArea::vertical().id_salt("scriptin").max_height(ui.available_height() - 52.0).show(ui, |ui| {
                egui::TextEdit::multiline(&mut self.script_ui.text).code_editor().desired_width(f32::INFINITY).desired_rows(6).lock_focus(true).show(ui)
            });
            if let Some(r) = out.inner.cursor_range {
                let (a, b) = (r.primary.ccursor.index, r.secondary.ccursor.index);
                self.script_ui.sel = (a.min(b), a.max(b));
            }
            let ctrl_enter = ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter));
            ui.horizontal(|ui| {
                if ui.button("Execute").on_hover_text("Ctrl+Enter").clicked() || ctrl_enter {
                    run = Some(self.script_ui.text.clone());
                }
                if ui.button("Execute Selection").clicked() {
                    run = Some(self.script_ui.selection());
                }
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut self.script_ui.shelf_label).desired_width(90.0).hint_text("shelf label"));
                if ui.button("Add to Shelf").on_hover_text("The selected text (or all of it) becomes a button on the Custom shelf").clicked() {
                    let text = self.script_ui.selection();
                    let label = if self.script_ui.shelf_label.trim().is_empty() { text.lines().next().unwrap_or("script").chars().take(14).collect() } else { self.script_ui.shelf_label.clone() };
                    shelf = Some((label, text));
                }
            });
        });
        self.show_script = open;
        if let Some(src) = source {
            self.exec(&format!("source {}", Self::quote(&src)));
        }
        if let Some((name, text)) = save {
            let dir = match &self.session.scene.project_dir {
                Some(d) => d.join("scripts"),
                None => crate::mel::cmd_mel::script_dirs(&self.session).into_iter().find(|d| d.is_dir()).unwrap_or_else(|| std::path::PathBuf::from("scripts")),
            };
            let file = if name.contains('/') { std::path::PathBuf::from(&name) } else { dir.join(if name.ends_with(".mel") { name.clone() } else { format!("{}.mel", name) }) };
            let r = file.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).and_then(|_| std::fs::write(&file, text));
            self.say(&match r {
                Ok(_) => format!("Saved {}", file.display()),
                Err(e) => format!("Cannot save the script: {}", e),
            });
        }
        if let Some((label, cmd)) = shelf {
            if !cmd.trim().is_empty() {
                self.exec(&format!("shelfButton -label {} -command {}", Self::quote(&label), Self::quote(&cmd.replace('\n', " "))));
                let _ = self.session.prefs.save();
                self.shelf_tab = 4;
            }
        }
        if let Some(c) = run {
            if !c.trim().is_empty() {
                self.exec(&c);
            }
        }
    }

    /// Windows made by scripts (window, columnLayout, intSliderGrp, button, ...).
    fn custom_ui_windows(&mut self, ctx: &egui::Context) {
        let wins = self.session.ui.windows();
        let mut actions: Vec<(usize, Option<f64>, Option<String>)> = Vec::new();
        for wid in wins {
            let Some(w) = self.session.ui.get(wid).cloned() else { continue };
            if !w.shown {
                continue;
            }
            let mut open = true;
            let title = if w.label.is_empty() { w.name.clone() } else { w.label.clone() };
            let mut win = egui::Window::new(title).id(egui::Id::new(("scriptwin", wid))).open(&mut open);
            if w.width > 0.0 && w.height > 0.0 {
                win = win.default_size([w.width, w.height]);
            }
            win.show(ctx, |ui| {
                for c in w.children.clone() {
                    self.draw_ui_widget(ui, c, &mut actions);
                }
            });
            if !open {
                if let Some(m) = self.session.ui.get_mut(wid) {
                    m.shown = false;
                }
            }
        }
        for (id, v, t) in actions {
            match crate::mel::ui::invoke(&mut self.session, id, v, t) {
                Ok(lines) => {
                    for l in lines {
                        self.log.push((l, false));
                    }
                }
                Err(e) => {
                    self.log.push((format!("// Error: {}", e), true));
                    self.mel_result = format!("// Error: {}", e);
                    self.mel_error = true;
                }
            }
            self.mark_dirty();
            self.scene_edited();
        }
    }

    fn draw_ui_widget(&mut self, ui: &mut Ui, id: usize, actions: &mut Vec<(usize, Option<f64>, Option<String>)>) {
        use crate::mel::ui::Kind;
        let Some(mut w) = self.session.ui.get(id).cloned() else { return };
        if !w.visible {
            return;
        }
        if w.kind == Kind::AttrSlider && !w.attribute.is_empty() {
            if let Ok((n, a)) = self.session.scene.resolve_plug(&w.attribute) {
                if let Ok(v) = self.session.scene.get_attr(n, &a) {
                    w.value = v as f64;
                }
            }
        }
        let wd = if w.width > 0.0 { Some(w.width) } else { None };
        match w.kind {
            Kind::Column | Kind::Window => {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = w.row_spacing;
                    if let Some(x) = wd {
                        ui.set_min_width(x);
                    }
                    for c in w.children.clone() {
                        self.draw_ui_widget(ui, c, actions);
                    }
                });
            }
            Kind::Row => {
                ui.horizontal(|ui| {
                    for c in w.children.clone() {
                        self.draw_ui_widget(ui, c, actions);
                    }
                });
            }
            Kind::Frame => {
                let h = egui::CollapsingHeader::new(&w.label).id_salt(("frame", id)).default_open(!w.collapsed);
                h.show(ui, |ui| {
                    for c in w.children.clone() {
                        self.draw_ui_widget(ui, c, actions);
                    }
                });
            }
            Kind::Scroll => {
                egui::ScrollArea::vertical().id_salt(("scroll", id)).show(ui, |ui| {
                    for c in w.children.clone() {
                        self.draw_ui_widget(ui, c, actions);
                    }
                });
            }
            Kind::Text => {
                ui.label(if w.label.is_empty() { &w.text } else { &w.label });
            }
            Kind::Separator => {
                ui.separator();
            }
            Kind::Button => {
                let b = egui::Button::new(&w.label);
                let r = match wd {
                    Some(x) => ui.add_enabled(w.enabled, b.min_size(vec2(x, 0.0))),
                    None => ui.add_enabled(w.enabled, b),
                };
                if r.clicked() {
                    actions.push((id, None, None));
                }
            }
            Kind::Check => {
                let mut on = w.value > 0.5;
                if ui.add_enabled(w.enabled, egui::Checkbox::new(&mut on, &w.label)).changed() {
                    actions.push((id, Some(on as u8 as f64), None));
                }
            }
            Kind::IntSlider | Kind::FloatSlider | Kind::AttrSlider => {
                ui.horizontal(|ui| {
                    if !w.label.is_empty() {
                        ui.label(&w.label);
                    }
                    let mut v = w.value;
                    let int = w.kind == Kind::IntSlider;
                    let (lo, hi) = (w.min.min(w.max), w.max.max(w.min));
                    let mut changed = false;
                    ui.add_enabled_ui(w.enabled, |ui| {
                        let mut s = egui::Slider::new(&mut v, lo..=hi).show_value(false);
                        if int {
                            s = s.step_by(1.0);
                        }
                        changed |= ui.add(s).changed();
                        if w.field || w.kind != Kind::IntSlider || true {
                            let mut d = egui::DragValue::new(&mut v).speed(if int { 0.2 } else { 0.01 });
                            if int {
                                d = d.fixed_decimals(0);
                            }
                            if w.field_min > f64::MIN / 2.0 || w.field_max < f64::MAX / 2.0 {
                                d = d.range(w.field_min.max(-1e9)..=w.field_max.min(1e9));
                            }
                            changed |= ui.add(d).changed();
                        }
                    });
                    if changed {
                        actions.push((id, Some(v), None));
                    }
                });
            }
            Kind::IntField | Kind::FloatField => {
                ui.horizontal(|ui| {
                    if !w.label.is_empty() {
                        ui.label(&w.label);
                    }
                    let mut v = w.value;
                    let mut d = egui::DragValue::new(&mut v).speed(if w.kind == Kind::IntField { 0.2 } else { 0.01 });
                    if w.kind == Kind::IntField {
                        d = d.fixed_decimals(0);
                    }
                    if ui.add_enabled(w.enabled, d).changed() {
                        actions.push((id, Some(v), None));
                    }
                });
            }
            Kind::TextField => {
                ui.horizontal(|ui| {
                    if !w.label.is_empty() {
                        ui.label(&w.label);
                    }
                    let mut t = w.text.clone();
                    let r = ui.add_enabled(w.enabled, egui::TextEdit::singleline(&mut t).desired_width(wd.unwrap_or(140.0)));
                    if r.changed() {
                        if let Some(m) = self.session.ui.get_mut(id) {
                            m.text = t.clone();
                        }
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        actions.push((id, None, Some(t)));
                    }
                });
            }
            Kind::OptionMenu => {
                let items: Vec<String> = w.children.iter().filter_map(|c| self.session.ui.get(*c)).map(|m| m.label.clone()).collect();
                ui.horizontal(|ui| {
                    if !w.label.is_empty() {
                        ui.label(&w.label);
                    }
                    egui::ComboBox::from_id_salt(("opt", id)).selected_text(w.text.clone()).show_ui(ui, |ui| {
                        for it in &items {
                            if ui.selectable_label(&w.text == it, it).clicked() {
                                actions.push((id, None, Some(it.clone())));
                            }
                        }
                    });
                });
            }
            Kind::MenuItem => {}
        }
    }
}

#[derive(Default)]
struct ScriptUi {
    text: String,
    sel: (usize, usize),
    file: String,
    shelf_label: String,
}

impl ScriptUi {
    /// The selected text of the input, or all of it.
    fn selection(&self) -> String {
        let chars: Vec<char> = self.text.chars().collect();
        let (a, b) = (self.sel.0.min(chars.len()), self.sel.1.min(chars.len()));
        if b > a {
            chars[a..b].iter().collect()
        } else {
            self.text.clone()
        }
    }
}

#[derive(Default)]
struct MmUi {
    show: bool,
    name: String,
    new_name: String,
    dir: usize,
    label: String,
    command: String,
}

#[derive(Default)]
struct ExprUi {
    show: bool,
    selected: String,
    text: String,
    name: String,
    note: String,
}

impl App {
    /// Window > Settings/Preferences > Marking Menu Editor: eight items around a menu that opens at the pointer.
    fn marking_menu_editor(&mut self, ctx: &egui::Context) {
        if !self.mm_ui.show {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let menus = self.session.prefs.marking_menus.clone();
        egui::Window::new("Marking Menu Editor").open(&mut open).default_width(420.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.mm_ui.new_name).desired_width(120.0).hint_text("menu name"));
                if ui.button("Create Marking Menu").clicked() && !self.mm_ui.new_name.trim().is_empty() {
                    cmds.push(format!("markingMenu -create {}", self.mm_ui.new_name.trim()));
                    self.mm_ui.name = self.mm_ui.new_name.trim().to_string();
                    self.mm_ui.new_name.clear();
                }
            });
            ui.horizontal_wrapped(|ui| {
                for m in &menus {
                    if ui.selectable_label(self.mm_ui.name == m.name, &m.name).clicked() {
                        self.mm_ui.name = m.name.clone();
                    }
                }
            });
            if let Some(m) = menus.iter().find(|m| m.name == self.mm_ui.name) {
                ui.separator();
                // the eight positions laid out around the middle
                let grid = [[7usize, 0, 1], [6, 99, 2], [5, 4, 3]];
                egui::Grid::new("mmgrid").spacing([4.0, 4.0]).show(ui, |ui| {
                    for row in grid {
                        for cell in row {
                            if cell == 99 {
                                ui.label(egui::RichText::new(&m.name).strong());
                            } else {
                                let label = m.items[cell].as_ref().map(|i| i.0.clone()).unwrap_or_else(|| format!("({})", crate::mel::ui::DIRECTIONS[cell]));
                                if ui.add(egui::Button::new(label).min_size(vec2(100.0, 24.0)).selected(self.mm_ui.dir == cell)).clicked() {
                                    self.mm_ui.dir = cell;
                                    let it = m.items[cell].clone().unwrap_or_default();
                                    self.mm_ui.label = it.0;
                                    self.mm_ui.command = it.1;
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.label(format!("Item {}", crate::mel::ui::DIRECTIONS[self.mm_ui.dir]));
                ui.horizontal(|ui| {
                    ui.label("Label");
                    ui.add(egui::TextEdit::singleline(&mut self.mm_ui.label).desired_width(160.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Command");
                    ui.add(egui::TextEdit::singleline(&mut self.mm_ui.command).desired_width(240.0));
                });
                ui.horizontal(|ui| {
                    if ui.button("Save Item").clicked() {
                        cmds.push(format!("markingMenu -menu {} -item {} {} {}", m.name, crate::mel::ui::DIRECTIONS[self.mm_ui.dir], Self::quote(&self.mm_ui.label), Self::quote(&self.mm_ui.command)));
                    }
                    if ui.button("Try It").on_hover_text("Open the menu at the pointer").clicked() {
                        cmds.push(format!("markingMenu -popup {}", m.name));
                    }
                    if ui.button("Delete Menu").clicked() {
                        cmds.push(format!("markingMenu -delete {}", m.name));
                    }
                });
                ui.label(egui::RichText::new("Assign it to a key in the Hotkey Editor with the command  markingMenu -popup NAME").color(TEXT_DIM));
            }
        });
        if !open {
            self.mm_ui.show = false;
        }
        if !cmds.is_empty() {
            for c in cmds {
                self.exec(&c);
            }
            let _ = self.session.prefs.save();
        }
    }

    /// The marking menu a hotkey opened: eight buttons around the pointer, a click picks, Esc closes.
    fn marking_menu_popup(&mut self, ctx: &egui::Context) {
        let Some(name) = self.session.popup_menu.clone() else { return };
        let at = *self.popup_at.get_or_insert_with(|| ctx.input(|i| i.pointer.latest_pos()).unwrap_or(pos2(400.0, 300.0)));
        let Some(menu) = self.session.prefs.marking_menus.iter().find(|m| m.name == name).cloned() else {
            self.session.popup_menu = None;
            self.popup_at = None;
            return;
        };
        let mut picked: Option<String> = None;
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        egui::Area::new(egui::Id::new("markingpopup")).order(egui::Order::Foreground).fixed_pos(at - vec2(110.0, 90.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(egui::RichText::new(&menu.name).strong());
                let grid = [[7usize, 0, 1], [6, 99, 2], [5, 4, 3]];
                egui::Grid::new("mmpop").spacing([4.0, 4.0]).show(ui, |ui| {
                    for row in grid {
                        for cell in row {
                            if cell == 99 {
                                if ui.button("x").clicked() {
                                    close = true;
                                }
                            } else if let Some((l, c)) = &menu.items[cell] {
                                if ui.add(egui::Button::new(l).min_size(vec2(90.0, 24.0))).clicked() {
                                    picked = Some(c.clone());
                                }
                            } else {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        });
        if picked.is_some() || close {
            self.session.popup_menu = None;
            self.popup_at = None;
        }
        if let Some(c) = picked {
            self.exec(&c);
        }
    }

    /// Window > Animation Editors > Expression Editor.
    fn expression_editor(&mut self, ctx: &egui::Context) {
        if !self.expr_ui.show {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let exprs: Vec<(String, bool, Option<String>)> = self.session.scene.expressions.iter().map(|e| (e.name.clone(), e.enabled, e.error.clone())).collect();
        egui::Window::new("Expression Editor").open(&mut open).default_size([520.0, 360.0]).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (n, on, err) in &exprs {
                    let label = format!("{}{}{}", n, if *on { "" } else { " (off)" }, if err.is_some() { " !" } else { "" });
                    if ui.selectable_label(self.expr_ui.selected == *n, label).clicked() {
                        self.expr_ui.selected = n.clone();
                        self.expr_ui.text = self.session.scene.expressions.iter().find(|e| &e.name == n).map(|e| e.text.clone()).unwrap_or_default();
                        self.expr_ui.name = n.clone();
                    }
                }
                if ui.button("New").clicked() {
                    self.expr_ui.selected.clear();
                    self.expr_ui.text.clear();
                    self.expr_ui.name.clear();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Expression Name");
                ui.add(egui::TextEdit::singleline(&mut self.expr_ui.name).desired_width(160.0));
            });
            ui.add(egui::TextEdit::multiline(&mut self.expr_ui.text).code_editor().desired_width(f32::INFINITY).desired_rows(10).hint_text("eyeball.sy = lookAt.blink;"));
            ui.horizontal(|ui| {
                let exists = exprs.iter().any(|(n, _, _)| *n == self.expr_ui.selected);
                if ui.button(if exists { "Edit" } else { "Create" }).clicked() {
                    let text = Self::quote(&self.expr_ui.text);
                    if exists {
                        cmds.push(format!("expression -e -s {} {}", text, self.expr_ui.selected));
                    } else {
                        let nm = if self.expr_ui.name.trim().is_empty() { String::new() } else { format!("-n {} ", self.expr_ui.name.trim()) };
                        cmds.push(format!("expression {}-s {}", nm, text));
                    }
                }
                if ui.add_enabled(exists, egui::Button::new("Delete")).clicked() {
                    cmds.push(format!("expression -delete {}", self.expr_ui.selected));
                    self.expr_ui.selected.clear();
                    self.expr_ui.text.clear();
                }
                if let Some((_, on, _)) = exprs.iter().find(|(n, _, _)| *n == self.expr_ui.selected) {
                    let mut en = *on;
                    if ui.checkbox(&mut en, "Enabled").changed() {
                        cmds.push(format!("expression -e -enable {} {}", en as u8, self.expr_ui.selected));
                    }
                }
            });
            if let Some((_, _, Some(err))) = exprs.iter().find(|(n, _, _)| *n == self.expr_ui.selected) {
                ui.label(egui::RichText::new(err).color(Color32::from_rgb(0xc0, 0x30, 0x30)));
            }
            ui.label(egui::RichText::new("Assignments like  node.attr = expression;  with + - * / %, ?:, if / else, $variables, sin cos abs min max clamp noise rand ..., time and frame.").color(TEXT_DIM));
        });
        if !open {
            self.expr_ui.show = false;
        }
        for c in cmds {
            self.exec(&c);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_gen = self.gen;
        self.session.sync_anim_prefs();
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let playing = self.session.scene.timeline.playing;
        if playing {
            if self.session.scene.advance_playback(dt) {
                self.mark_dirty();
            }
            ctx.request_repaint();
        }
        let ptr_down = ctx.input(|i| i.pointer.any_down());
        self.interacting = playing || ptr_down;
        self.help_line = String::new();
        // keep the component selection attached to the selected object
        if self.session.comp.mode != SelMode::Object {
            let sel_shape = {
                let sc = &self.session.scene;
                sc.selection.iter().find_map(|id| {
                    if matches!(sc.node(*id).map(|n| &n.data), Some(NodeData::Mesh(_))) {
                        Some(*id)
                    } else {
                        sc.mesh_shapes_under(*id).first().copied()
                    }
                })
            };
            match sel_shape {
                Some(sh) if self.session.comp.node != Some(sh) => {
                    self.session.comp.node = Some(sh);
                    self.session.comp.sel = Sel::default();
                    self.mark_dirty();
                }
                None => {
                    self.session.comp.mode = SelMode::Object;
                    self.session.comp.node = None;
                    self.session.comp.sel = Sel::default();
                    self.mark_dirty();
                }
                _ => {}
            }
        }
        self.handle_keys(ctx);

        let title = {
            let sc = &self.session.scene;
            let name = sc.file_path.clone().unwrap_or_else(|| "untitled".to_string());
            format!("Aztec 2009: {}", name)
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        let panel_frame = egui::Frame::NONE.fill(CHROME).inner_margin(egui::Margin::symmetric(4, 2));
        egui::TopBottomPanel::top("menubar").frame(panel_frame).show(ctx, |ui| self.menu_bar(ui));
        if self.ui_status {
            egui::TopBottomPanel::top("statusline").frame(panel_frame).exact_height(30.0).show(ctx, |ui| self.status_line(ui));
        }
        if self.ui_shelf {
            egui::TopBottomPanel::top("shelf").frame(panel_frame).exact_height(36.0).show(ctx, |ui| self.shelf(ui));
        }

        if self.ui_helpline {
            egui::TopBottomPanel::bottom("helpline").frame(panel_frame).exact_height(20.0).show(ctx, |ui| {
                ui.label(egui::RichText::new(&self.help_line).color(TEXT_DIM).size(11.0));
            });
        }
        if self.ui_cmdline {
            egui::TopBottomPanel::bottom("cmdline").frame(panel_frame).exact_height(28.0).show(ctx, |ui| self.command_line(ui));
        }
        if self.ui_rangeslider {
            egui::TopBottomPanel::bottom("rangeslider").frame(panel_frame).exact_height(30.0).show(ctx, |ui| self.range_slider(ui));
        }
        if self.ui_timeslider {
            egui::TopBottomPanel::bottom("timeslider").frame(panel_frame).exact_height(46.0).show(ctx, |ui| self.time_slider(ui));
        }

        if self.ui_toolbox {
            egui::SidePanel::left("toolbox").frame(panel_frame).exact_width(46.0).resizable(false).show(ctx, |ui| self.tool_box(ui));
        }
        if self.show_outliner {
            egui::SidePanel::left("outliner").frame(panel_frame).default_width(190.0).width_range(120.0..=400.0).show(ctx, |ui| self.outliner(ui));
        }
        if self.show_channels || self.show_layers {
            egui::SidePanel::right("channelbox").frame(panel_frame).default_width(250.0).width_range(180.0..=500.0).show(ctx, |ui| {
                if self.show_layers {
                    egui::TopBottomPanel::bottom("layers").frame(egui::Frame::NONE).resizable(true).default_height(150.0).show_inside(ui, |ui| {
                        self.layer_editor(ui);
                    });
                }
                if self.show_channels {
                    self.channel_box(ui);
                }
            });
        }

        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_rgb(0x38, 0x38, 0x38))).show(ctx, |ui| {
            let full = ui.available_rect_before_wrap();
            if self.layout == LayoutMode::Four {
                let gap = 2.0;
                let hw = (full.width() - gap) * 0.5;
                let hh = (full.height() - gap) * 0.5;
                let cell = |cx: f32, cy: f32| Rect::from_min_size(full.min + vec2(cx * (hw + gap), cy * (hh + gap)), vec2(hw, hh));
                self.viewport_ui(ui, ViewKind::Top, cell(0.0, 0.0));
                self.viewport_ui(ui, ViewKind::Persp, cell(1.0, 0.0));
                self.viewport_ui(ui, ViewKind::Front, cell(0.0, 1.0));
                self.viewport_ui(ui, ViewKind::Side, cell(1.0, 1.0));
            } else {
                let kind = self.single_kind;
                self.viewport_ui(ui, kind, full);
            }
        });

        self.poll_render_job(ctx);
        self.poll_pipe_jobs(ctx);
        self.render_settings_window(ctx);
        self.render_layers_window(ctx);
        self.fcheck_window(ctx);
        self.attribute_editor(ctx);
        self.uv_editor(ctx);
        // image files dropped on the window become color maps (or an image plane)
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for path in dropped {
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let p = path.to_string_lossy().to_string();
            if ["png", "jpg", "jpeg", "bmp", "tga", "gif"].contains(&ext.as_str()) {
                if self.session.scene.selection.is_empty() {
                    self.exec(&format!("imagePlane {}", Self::quote(&p)));
                } else {
                    let n = self.session.scene.name_of(self.session.scene.selection[0]);
                    self.exec(&format!("assignTexture \"{}\" {}", n, Self::quote(&p)));
                }
            } else if ext == "aztec" {
                self.exec(&format!("file -open {}", Self::quote(&p)));
            } else if ext == "mel" {
                self.exec(&format!("source {}", Self::quote(&p)));
            }
        }
        self.file_dialog_ui(ctx);
        self.dup_special_ui(ctx);
        self.sdk_ui(ctx);
        self.prim_opts_ui(ctx);
        self.prefs_ui(ctx);
        self.hotkeys_ui(ctx);
        self.project_ui(ctx);
        self.reference_ui(ctx);
        self.member_ui(ctx);
        self.flexor_ui(ctx);
        self.hotbox_ui(ctx);
        self.autosave_tick();
        self.graph_editor_ui(ctx);
        self.node_editors_ui(ctx);
        self.paint_window(ctx);
        self.dope_sheet_ui(ctx);
        self.skin_window(ctx);
        self.visor_window(ctx);
        self.sculpt_window(ctx);
        self.custom_ui_windows(ctx);
        self.marking_menu_editor(ctx);
        self.marking_menu_popup(ctx);
        self.expression_editor(ctx);
        self.trax_window(ctx);
        self.particle_event_window(ctx);
        self.nurbs_window(ctx);
        self.placement_window(ctx);
        self.curve_window(ctx);
        self.render_view_ui(ctx);
        if self.show_script {
            self.script_editor_ui(ctx);
        }
        if self.show_help {
            let mut open = true;
            egui::Window::new("Keyboard Shortcuts").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                egui::Grid::new("helpgrid").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                    for (k, d) in HELP_ROWS {
                        ui.label(egui::RichText::new(*k).strong());
                        ui.label(*d);
                        ui.end_row();
                    }
                });
            });
            self.show_help = open;
        }
        if self.show_about {
            let mut open = true;
            egui::Window::new("About Aztec 2009").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label(egui::RichText::new("Aztec 2009").heading());
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("A Maya 2009 style 3D animation tool written in Rust.");
                ui.label("Dependency graph, nParticles, animation layers and a stereo camera rig.");
            });
            self.show_about = open;
        }

        if !ptr_down {
            self.commit();
        }
        if self.gen != self.frame_gen {
            ctx.request_repaint();
        }
    }
}

// ====================================================================== polygon modeling UI

fn point_in_poly(p: Pos2, pts: &[Pos2]) -> bool {
    let mut inside = false;
    let n = pts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (pts[i], pts[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl App {
    fn mesh_poly(&self, shape: NodeId) -> Option<(Poly, Option<NodeId>)> {
        self.session.scene.shape_poly(shape)
    }

    /// Pivot (world), average normal (world) and world matrix of the component selection.
    fn comp_pivot(&mut self) -> Option<(Vec3, Vec3, Mat4)> {
        let shape = self.session.comp.node?;
        let (poly, parent) = self.mesh_poly(shape)?;
        let world = match parent {
            Some(p) => self.session.scene.world_matrix(p),
            None => Mat4::identity(),
        };
        let c = &self.session.comp;
        let verts = cmd_model::verts_of(&poly, &c.sel, c.mode);
        let verts: Vec<u32> = verts.into_iter().filter(|&v| (v as usize) < poly.pos.len()).collect();
        if verts.is_empty() {
            return None;
        }
        let mut cen = Vec3::ZERO;
        for &v in &verts {
            cen += poly.pos[v as usize];
        }
        cen = cen * (1.0 / verts.len() as f32);
        let vs: BTreeSet<u32> = verts.iter().copied().collect();
        let mut n = Vec3::ZERO;
        for (fi, f) in poly.faces.iter().enumerate() {
            let hit = match c.mode {
                SelMode::Face => c.sel.faces.contains(&(fi as u32)),
                _ => f.iter().any(|v| vs.contains(v)),
            };
            if hit {
                n += poly.face_normal(fi);
            }
        }
        let n = world.transform_dir(if n.length_sq() > 0.0 { n } else { Vec3::UP }).normalized();
        Some((world.transform_point(cen), n, world))
    }

    fn set_sel_mode(&mut self, mode: SelMode) {
        if mode == SelMode::Object {
            self.session.comp.mode = SelMode::Object;
            self.session.comp.sel = Sel::default();
            self.axis_normal = false;
            self.mark_dirty();
            return;
        }
        let shape = match cmd_model::current_shape(&self.session) {
            Some(s) => s,
            None => {
                self.say("Select a polygon object first");
                return;
            }
        };
        let mut welded = false;
        if let Some(NodeData::Mesh(md)) = self.session.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
            if md.prim != "custom" && md.nurbs.is_none() {
                md.ensure_custom();
                welded = true;
            }
        }
        if welded {
            self.scene_edited();
        }
        let same = self.session.comp.node == Some(shape) && self.session.comp.mode != SelMode::Object;
        if same {
            let _ = cmd_model::run_op(&mut self.session, ModelOp::Convert(mode));
        } else {
            self.session.comp.node = Some(shape);
            self.session.comp.mode = mode;
            self.session.comp.sel = Sel::default();
        }
        self.session.comp.mode = mode;
        self.last_comp_mode = mode;
        self.axis_normal = false;
        self.mark_dirty();
    }

    fn model(&mut self, op: ModelOp) {
        let is_extrude = matches!(op, ModelOp::Extrude { .. });
        let needs_comp = matches!(op, ModelOp::Extrude { .. } | ModelOp::Bevel { .. } | ModelOp::Delete { .. } | ModelOp::Merge { .. } | ModelOp::MergeCenter | ModelOp::MergeEdge | ModelOp::Bridge | ModelOp::Collapse | ModelOp::ChipOff { .. });
        if needs_comp && self.session.comp.mode == SelMode::Object {
            self.mel_result = "// Switch to a component mode (F9 vertex, F10 edge, F11 face) and select components first".into();
            self.mel_error = true;
            return;
        }
        match cmd_model::run_op(&mut self.session, op.clone()) {
            Ok(msg) => {
                self.say(&msg);
                self.last_op = Some(op);
                if is_extrude {
                    self.tool = Tool::Move;
                    self.axis_normal = true;
                } else {
                    self.axis_normal = false;
                }
                if self.session.comp.mode != SelMode::Object {
                    self.last_comp_mode = self.session.comp.mode;
                }
                self.scene_edited();
            }
            Err(e) => {
                self.mel_result = format!("// Error: {}", e);
                self.mel_error = true;
            }
        }
    }

    fn comp_view(&mut self, cam: &Cam, rect: Rect, idx: usize, wire: bool) -> Option<CompView> {
        let shape = if self.session.comp.mode != SelMode::Object {
            self.session.comp.node?
        } else {
            cmd_model::current_shape(&self.session)?
        };
        let mut welded = false;
        if let Some(NodeData::Mesh(md)) = self.session.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
            if md.prim != "custom" {
                md.ensure_custom();
                welded = true;
            }
        }
        if welded {
            self.scene_edited();
        }
        let (poly, parent) = self.mesh_poly(shape)?;
        let world = match parent {
            Some(p) => self.session.scene.world_matrix(p),
            None => Mat4::identity(),
        };
        let vp = cam.view_proj();
        let (w, h) = (rect.width(), rect.height());
        let n = poly.pos.len();
        let wpos: Vec<Vec3> = poly.pos.iter().map(|p| world.transform_point(*p)).collect();
        let proj: Vec<Option<(f32, f32, f32)>> = wpos.iter().map(|p| project(&vp, *p, w, h)).collect();
        let panel = &self.panels[idx];
        let depth_ok = |x: f32, y: f32, d: f32| -> bool {
            if wire || panel.depth.is_empty() || panel.dsize.0 == 0 {
                return true;
            }
            let (dw, dh) = panel.dsize;
            let ix = ((x / w) * dw as f32) as isize;
            let iy = ((y / h) * dh as f32) as isize;
            if ix < 0 || iy < 0 || ix >= dw as isize || iy >= dh as isize {
                return false;
            }
            // look at a 3x3 neighbourhood (supersampled pixels) and accept the nearest
            let mut best = f32::INFINITY;
            for oy in -1..=1isize {
                for ox in -1..=1isize {
                    let (xx, yy) = (ix + ox, iy + oy);
                    if xx >= 0 && yy >= 0 && xx < dw as isize && yy < dh as isize {
                        best = best.min(panel.depth[yy as usize * dw + xx as usize]);
                    }
                }
            }
            let bias = if cam.kind == ViewKind::Persp { 0.04 + d * 0.015 } else { cam.size * 0.03 + 0.05 };
            !best.is_finite() || d <= best + bias
        };
        let mut vfront = vec![false; n];
        let mut fvis = vec![false; poly.faces.len()];
        let mut fctr = vec![None; poly.faces.len()];
        let mut fdepth = vec![f32::INFINITY; poly.faces.len()];
        for (fi, f) in poly.faces.iter().enumerate() {
            let nrm = world.transform_dir(poly.face_normal(fi));
            let ctr = world.transform_point(poly.face_center(fi));
            let front = wire || nrm.dot(cam.eye - ctr) > 0.0;
            if front {
                for &v in f {
                    vfront[v as usize] = true;
                }
            }
            if let Some((x, y, d)) = project(&vp, ctr, w, h) {
                fctr[fi] = Some(rect.min + vec2(x, y));
                fdepth[fi] = d;
                fvis[fi] = front && depth_ok(x, y, d);
            }
        }
        let sp: Vec<Option<Pos2>> = proj.iter().map(|p| p.map(|(x, y, _)| rect.min + vec2(x, y))).collect();
        let vvis: Vec<bool> = (0..n)
            .map(|i| match proj[i] {
                Some((x, y, d)) => vfront[i] && depth_ok(x, y, d),
                None => false,
            })
            .collect();
        let ef = poly.edge_faces();
        let mut edges: Vec<Edge> = ef.keys().copied().collect();
        edges.sort();
        let evis: Vec<bool> = edges
            .iter()
            .map(|&(a, b)| match (proj[a as usize], proj[b as usize]) {
                (Some(pa), Some(pb)) => {
                    let front = ef[&(a, b)].iter().any(|&f| {
                        let nrm = world.transform_dir(poly.face_normal(f));
                        let ctr = world.transform_point(poly.face_center(f));
                        wire || nrm.dot(cam.eye - ctr) > 0.0
                    });
                    let mx = (pa.0 + pb.0) * 0.5;
                    let my = (pa.1 + pb.1) * 0.5;
                    let md = (pa.2 + pb.2) * 0.5;
                    front && depth_ok(mx, my, md)
                }
                _ => false,
            })
            .collect();
        Some(CompView { shape, poly, sp, vvis, edges, evis, fvis, fctr, fdepth })
    }

    fn comp_hit(&self, cv: &CompView, pos: Pos2, mode: SelMode) -> Option<CompHit> {
        match mode {
            SelMode::Vertex => {
                let mut best: Option<(f32, u32)> = None;
                for (i, p) in cv.sp.iter().enumerate() {
                    if let (Some(p), true) = (p, cv.vvis[i]) {
                        let d = (*p - pos).length();
                        if d <= 12.0 && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                            best = Some((d, i as u32));
                        }
                    }
                }
                best.map(|(_, v)| CompHit::Vert(v))
            }
            SelMode::Edge => {
                let mut best: Option<(f32, Edge)> = None;
                for (i, e) in cv.edges.iter().enumerate() {
                    if !cv.evis[i] {
                        continue;
                    }
                    if let (Some(a), Some(b)) = (cv.sp[e.0 as usize], cv.sp[e.1 as usize]) {
                        let d = seg_dist(pos, a, b);
                        if d <= 9.0 && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                            best = Some((d, *e));
                        }
                    }
                }
                best.map(|(_, e)| CompHit::Edge(e))
            }
            _ => {
                let mut best: Option<(f32, u32)> = None;
                for (fi, f) in cv.poly.faces.iter().enumerate() {
                    if !cv.fvis[fi] {
                        continue;
                    }
                    let pts: Vec<Pos2> = f.iter().filter_map(|&v| cv.sp[v as usize]).collect();
                    if pts.len() == f.len() && point_in_poly(pos, &pts) {
                        let d = cv.fdepth[fi];
                        if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                            best = Some((d, fi as u32));
                        }
                    }
                }
                best.map(|(_, f)| CompHit::Face(f))
            }
        }
    }

    fn comp_click(&mut self, cv: &CompView, pos: Pos2, op: SelOp) {
        let mode = self.session.comp.mode;
        let hit = self.comp_hit(cv, pos, mode);
        if self.session.comp.node != Some(cv.shape) {
            self.session.comp.node = Some(cv.shape);
            self.session.comp.sel = Sel::default();
        }
        self.axis_normal = false;
        let sel = &mut self.session.comp.sel;
        match hit {
            Some(h) => {
                if op == SelOp::Replace {
                    *sel = Sel::default();
                }
                match h {
                    CompHit::Vert(v) => {
                        if op == SelOp::Remove || (op == SelOp::Toggle && sel.verts.contains(&v)) {
                            sel.verts.remove(&v);
                        } else {
                            sel.verts.insert(v);
                        }
                    }
                    CompHit::Edge(e) => {
                        if op == SelOp::Remove || (op == SelOp::Toggle && sel.edges.contains(&e)) {
                            sel.edges.remove(&e);
                        } else {
                            sel.edges.insert(e);
                        }
                    }
                    CompHit::Face(f) => {
                        if op == SelOp::Remove || (op == SelOp::Toggle && sel.faces.contains(&f)) {
                            sel.faces.remove(&f);
                        } else {
                            sel.faces.insert(f);
                        }
                    }
                }
            }
            None => {
                if op == SelOp::Replace {
                    *sel = Sel::default();
                }
            }
        }
        self.mark_dirty();
    }

    fn comp_marquee(&mut self, cv: &CompView, r: Rect, op: SelOp) {
        let mode = self.session.comp.mode;
        let mut found = Sel::default();
        match mode {
            SelMode::Vertex => {
                for (i, p) in cv.sp.iter().enumerate() {
                    if let (Some(p), true) = (p, cv.vvis[i]) {
                        if r.contains(*p) {
                            found.verts.insert(i as u32);
                        }
                    }
                }
            }
            SelMode::Edge => {
                for (i, e) in cv.edges.iter().enumerate() {
                    if !cv.evis[i] {
                        continue;
                    }
                    if let (Some(a), Some(b)) = (cv.sp[e.0 as usize], cv.sp[e.1 as usize]) {
                        if r.contains(a) && r.contains(b) || r.contains(a.lerp(b, 0.5)) {
                            found.edges.insert(*e);
                        }
                    }
                }
            }
            _ => {
                for (fi, c) in cv.fctr.iter().enumerate() {
                    if let (Some(c), true) = (c, cv.fvis[fi]) {
                        if r.contains(*c) {
                            found.faces.insert(fi as u32);
                        }
                    }
                }
            }
        }
        if self.session.comp.node != Some(cv.shape) {
            self.session.comp.node = Some(cv.shape);
            self.session.comp.sel = Sel::default();
        }
        self.axis_normal = false;
        let sel = &mut self.session.comp.sel;
        match op {
            SelOp::Replace => *sel = found,
            SelOp::Toggle => {
                sel.verts.extend(found.verts);
                sel.edges.extend(found.edges);
                sel.faces.extend(found.faces);
            }
            SelOp::Remove => {
                for v in found.verts {
                    sel.verts.remove(&v);
                }
                for e in found.edges {
                    sel.edges.remove(&e);
                }
                for f in found.faces {
                    sel.faces.remove(&f);
                }
            }
        }
        self.mark_dirty();
    }

    fn draw_components(&self, painter: &egui::Painter, cv: &CompView, mode: SelMode, comp_active: bool, hover: Option<&CompHit>) {
        let sel = &self.session.comp.sel;
        let sel_on = comp_active && self.session.comp.node == Some(cv.shape);
        let edge_col = Color32::from_rgb(0x1c, 0x3c, 0xb4);
        let hot = Color32::from_rgb(0xff, 0xff, 0x70);
        let orange = Color32::from_rgb(0xff, 0x96, 0x14);
        let purple = Color32::from_rgb(0x8c, 0x3c, 0xc8);
        // edges
        for (i, e) in cv.edges.iter().enumerate() {
            if !cv.evis[i] {
                continue;
            }
            if let (Some(a), Some(b)) = (cv.sp[e.0 as usize], cv.sp[e.1 as usize]) {
                let selected = sel_on && (sel.edges.contains(e) || (mode == SelMode::Vertex && false));
                let is_hot = matches!(hover, Some(CompHit::Edge(he)) if he == e);
                let (c, w) = if selected {
                    (orange, 2.6)
                } else if is_hot {
                    (hot, 2.4)
                } else {
                    (edge_col, 1.0)
                };
                painter.line_segment([a, b], Stroke::new(w, c));
            }
        }
        // faces
        if sel_on || matches!(hover, Some(CompHit::Face(_))) {
            let mut mesh = egui::epaint::Mesh::default();
            let mut add_face = |fi: usize, col: Color32, mesh: &mut egui::epaint::Mesh| {
                let f = &cv.poly.faces[fi];
                let pts: Vec<Pos2> = f.iter().filter_map(|&v| cv.sp[v as usize]).collect();
                if pts.len() != f.len() || pts.len() < 3 {
                    return;
                }
                let base = mesh.vertices.len() as u32;
                for p in &pts {
                    mesh.colored_vertex(*p, col);
                }
                for k in 1..pts.len() as u32 - 1 {
                    mesh.add_triangle(base, base + k, base + k + 1);
                }
            };
            if sel_on {
                for &fi in &sel.faces {
                    if (fi as usize) < cv.poly.faces.len() {
                        add_face(fi as usize, Color32::from_rgba_unmultiplied(0xff, 0x96, 0x14, 110), &mut mesh);
                    }
                }
            }
            if let Some(CompHit::Face(fi)) = hover {
                add_face(*fi as usize, Color32::from_rgba_unmultiplied(0xff, 0xff, 0x70, 90), &mut mesh);
            }
            painter.add(egui::Shape::mesh(mesh));
            if sel_on {
                for &fi in &sel.faces {
                    if let Some(f) = cv.poly.faces.get(fi as usize) {
                        for i in 0..f.len() {
                            if let (Some(a), Some(b)) = (cv.sp[f[i] as usize], cv.sp[f[(i + 1) % f.len()] as usize]) {
                                painter.line_segment([a, b], Stroke::new(2.2, orange));
                            }
                        }
                    }
                }
            }
        }
        // face centers
        if comp_active && mode == SelMode::Face {
            for (fi, c) in cv.fctr.iter().enumerate() {
                if let (Some(c), true) = (c, cv.fvis[fi]) {
                    let s = sel_on && sel.faces.contains(&(fi as u32));
                    painter.rect_filled(Rect::from_center_size(*c, vec2(5.0, 5.0)), 0.0, if s { Color32::from_rgb(0xff, 0xe0, 0x30) } else { purple });
                }
            }
        }
        // vertices
        if comp_active && matches!(mode, SelMode::Vertex) {
            for (i, p) in cv.sp.iter().enumerate() {
                if let (Some(p), true) = (p, cv.vvis[i]) {
                    let s = sel_on && sel.verts.contains(&(i as u32));
                    let is_hot = matches!(hover, Some(CompHit::Vert(hv)) if *hv as usize == i);
                    let (col, sz) = if s {
                        (Color32::from_rgb(0xff, 0xe0, 0x30), 7.0)
                    } else if is_hot {
                        (hot, 7.0)
                    } else {
                        (purple, 5.0)
                    };
                    painter.rect(Rect::from_center_size(*p, vec2(sz, sz)), 0.0, col, Stroke::new(1.0_f32, Color32::BLACK), eframe::egui::StrokeKind::Inside);
                }
            }
        } else if comp_active && sel_on && mode != SelMode::Vertex {
            // show the vertices of the selection so the pivot is easy to judge
            let verts = cmd_model::verts_of(&cv.poly, sel, mode);
            for v in verts {
                if let (Some(Some(p)), true) = (cv.sp.get(v as usize), cv.vvis.get(v as usize).copied().unwrap_or(false)) {
                    painter.rect_filled(Rect::from_center_size(*p, vec2(4.0, 4.0)), 0.0, Color32::from_rgb(0xff, 0xe0, 0x30));
                }
            }
        }
    }

    fn edge_param(&self, cv: &CompView, e: Edge, pos: Pos2) -> f32 {
        match (cv.sp[e.0 as usize], cv.sp[e.1 as usize]) {
            (Some(a), Some(b)) => {
                let ab = b - a;
                let l2 = ab.length_sq().max(1e-6);
                ((pos - a).dot(ab) / l2).clamp(0.05, 0.95)
            }
            _ => 0.5,
        }
    }

    fn draw_tool_preview(&self, painter: &egui::Painter, cv: &CompView, cam: &Cam, rect: Rect, hover: Option<Pos2>) {
        let accent = Color32::from_rgb(0xff, 0xd0, 0x20);
        match self.tool {
            Tool::EdgeLoop => {
                let target: Option<(Edge, f32)> = match (self.loop_drag, hover) {
                    (Some(ld), _) => Some(ld),
                    (None, Some(hp)) if rect.contains(hp) => match self.comp_hit(cv, hp, SelMode::Edge) {
                        Some(CompHit::Edge(e)) => Some((e, self.edge_param(cv, e, hp))),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some((e, t)) = target {
                    let ring = cv.poly.edge_ring(e);
                    let vp = cam.view_proj();
                    let (parent_world, _) = (Mat4::identity(), ());
                    let _ = parent_world;
                    let world = match self.session.scene.node(cv.shape).and_then(|n| n.parent) {
                        Some(p) => self.world_of(p),
                        None => Mat4::identity(),
                    };
                    let pts: Vec<Pos2> = ring
                        .iter()
                        .filter_map(|&(s, en)| {
                            let p = world.transform_point(cv.poly.pos[s as usize].lerp(cv.poly.pos[en as usize], t));
                            project(&vp, p, rect.width(), rect.height()).map(|(x, y, _)| rect.min + vec2(x, y))
                        })
                        .collect();
                    if pts.len() >= 2 {
                        painter.add(egui::Shape::line(pts, Stroke::new(2.0_f32, accent)));
                    }
                }
            }
            Tool::SplitPoly => {
                let mut pts: Vec<Pos2> = Vec::new();
                for p in &self.split_pts {
                    let pos = match p {
                        SplitPt::Vertex(v) => cv.sp.get(*v as usize).copied().flatten(),
                        SplitPt::OnEdge((a, b), t) => match (cv.sp.get(*a as usize).copied().flatten(), cv.sp.get(*b as usize).copied().flatten()) {
                            (Some(pa), Some(pb)) => Some(pa.lerp(pb, *t)),
                            _ => None,
                        },
                    };
                    if let Some(pos) = pos {
                        painter.circle_filled(pos, 4.0, accent);
                        pts.push(pos);
                    }
                }
                if let (Some(last), Some(hp)) = (pts.last().copied(), hover) {
                    if rect.contains(hp) {
                        painter.line_segment([last, hp], Stroke::new(1.5_f32, accent));
                    }
                }
                if pts.len() >= 2 {
                    painter.add(egui::Shape::line(pts, Stroke::new(2.0_f32, accent)));
                }
            }
            _ => {}
        }
    }

    fn world_of(&self, id: NodeId) -> Mat4 {
        // world matrices are cached lazily in the scene; walk the parent chain without mutation
        let mut m = Mat4::identity();
        let mut cur = Some(id);
        let mut chain = Vec::new();
        while let Some(c) = cur {
            chain.push(c);
            cur = self.session.scene.node(c).and_then(|n| n.parent);
        }
        for c in chain.into_iter().rev() {
            let g = |a: &str| self.session.scene.node(c).and_then(|n| n.plug(a)).map(|p| p.value).unwrap_or(0.0);
            let t = Vec3::new(g("translateX"), g("translateY"), g("translateZ"));
            let r = Vec3::new(g("rotateX"), g("rotateY"), g("rotateZ"));
            let sc = Vec3::new(
                self.session.scene.node(c).and_then(|n| n.plug("scaleX")).map(|p| p.value).unwrap_or(1.0),
                self.session.scene.node(c).and_then(|n| n.plug("scaleY")).map(|p| p.value).unwrap_or(1.0),
                self.session.scene.node(c).and_then(|n| n.plug("scaleZ")).map(|p| p.value).unwrap_or(1.0),
            );
            m = m * Mat4::trs(t, crate::math::Quat::from_euler_deg(r), sc);
        }
        m
    }

    fn split_click(&mut self, cv: &CompView, pos: Pos2) {
        let pt = match self.comp_hit(cv, pos, SelMode::Vertex) {
            Some(CompHit::Vert(v)) => Some(SplitPt::Vertex(v)),
            _ => match self.comp_hit(cv, pos, SelMode::Edge) {
                Some(CompHit::Edge(e)) => {
                    let t = self.edge_param(cv, e, pos);
                    Some(SplitPt::OnEdge(e, t))
                }
                _ => None,
            },
        };
        if let Some(p) = pt {
            self.split_pts.push(p);
        }
    }

    fn finish_split(&mut self) {
        let pts = std::mem::take(&mut self.split_pts);
        if pts.len() >= 2 {
            self.model(ModelOp::Split(pts));
        }
    }

    fn compute_poly_stats(&mut self) -> PolyStats {
        if let Some((g, st)) = self.poly_stats {
            if g == self.gen {
                return st;
            }
        }
        let mut st = PolyStats::default();
        let mut shape_counts: Option<[usize; 4]> = None;
        let cur = cmd_model::current_shape(&self.session);
        let ids = self.session.scene.ids_where(|d| matches!(d, NodeData::Mesh(_)));
        for id in ids {
            if let Some(NodeData::Mesh(md)) = self.session.scene.nodes.get_mut(id).map(|n| &mut n.data) {
                let faces = md.mesh.faces.len();
                let verts = md.mesh.positions.len();
                let edges = md.edge_count();
                let tris = md.mesh.triangle_count();
                st.total[0] += verts;
                st.total[1] += edges;
                st.total[2] += faces;
                st.total[3] += tris;
                if Some(id) == cur {
                    shape_counts = Some([verts, edges, faces, tris]);
                }
            }
        }
        if let Some(c) = shape_counts {
            st.object = c;
        }
        if let (Some(shape), true) = (cur, self.session.comp.mode != SelMode::Object) {
            if self.session.comp.node == Some(shape) {
                if let Some((poly, _)) = self.mesh_poly(shape) {
                    let c = &self.session.comp;
                    st.selected[0] = cmd_model::verts_of(&poly, &c.sel, c.mode).len();
                    st.selected[1] = match c.mode {
                        SelMode::Edge => c.sel.edges.len(),
                        _ => cmd_model::edges_of(&poly, &c.sel, c.mode).len(),
                    };
                    st.selected[2] = match c.mode {
                        SelMode::Face => c.sel.faces.len(),
                        _ => cmd_model::faces_of(&poly, &c.sel, c.mode).len(),
                    };
                }
            }
        }
        self.poly_stats = Some((self.gen, st));
        st
    }

    fn draw_poly_hud(&mut self, painter: &egui::Painter, rect: Rect) {
        let st = self.compute_poly_stats();
        let font = FontId::monospace(11.0);
        let col = Color32::from_rgb(0x18, 0x18, 0x18);
        let rows = [("Verts:", 0usize), ("Edges:", 1), ("Faces:", 2), ("Tris:", 3)];
        let x_right = rect.right() - 10.0;
        let mut y = rect.top() + 6.0;
        for (label, i) in rows {
            let sel = if i < 3 { st.selected[i] } else { 0 };
            painter.text(pos2(x_right - 150.0, y), Align2::LEFT_TOP, label, font.clone(), col);
            painter.text(pos2(x_right - 70.0, y), Align2::RIGHT_TOP, format!("{}", st.total[i]), font.clone(), col);
            painter.text(pos2(x_right - 35.0, y), Align2::RIGHT_TOP, format!("{}", st.object[i]), font.clone(), col);
            painter.text(pos2(x_right, y), Align2::RIGHT_TOP, format!("{}", sel), font.clone(), col);
            y += 14.0;
        }
    }

    fn context_menu(&mut self, ui: &mut Ui) {
        let cur = self.session.comp.mode;
        let obj = self.session.scene.selection.first().map(|i| self.session.scene.name_of(*i));
        ui.label(egui::RichText::new(obj.clone().unwrap_or_else(|| "no object".to_string())).strong());
        ui.separator();
        for (m, l) in [(SelMode::Object, "Object Mode"), (SelMode::Vertex, "Vertex"), (SelMode::Edge, "Edge"), (SelMode::Face, "Face")] {
            if ui.radio(cur == m, l).clicked() {
                self.set_sel_mode(m);
                ui.close_menu();
            }
        }
        ui.separator();
        if cur == SelMode::Object {
            let has = obj.is_some();
            if ui.add_enabled(has, egui::Button::new("Duplicate").shortcut_text("Ctrl+D")).clicked() {
                self.exec("duplicate");
                ui.close_menu();
            }
            if ui.add_enabled(has, egui::Button::new("Group").shortcut_text("Ctrl+G")).clicked() {
                self.exec("group");
                ui.close_menu();
            }
            if ui.add_enabled(has, egui::Button::new("Delete").shortcut_text("Del")).clicked() {
                self.exec("delete");
                ui.close_menu();
            }
            ui.separator();
            if ui.add_enabled(has, egui::Button::new("Delete History")).clicked() {
                self.exec("deleteHistory");
                ui.close_menu();
            }
            if ui.add_enabled(has, egui::Button::new("Assign Color Map...")).clicked() {
                self.assign_texture_dialog();
                ui.close_menu();
            }
            if ui.add_enabled(has, egui::Button::new("Frame Selection").shortcut_text("F")).clicked() {
                self.frame(true);
                ui.close_menu();
            }
        } else {
            if ui.add(egui::Button::new("Extrude").shortcut_text("Ctrl+E")).clicked() {
                self.model(ModelOp::Extrude { ltz: 0.0 });
                ui.close_menu();
            }
            if ui.add(egui::Button::new("Bevel").shortcut_text("Ctrl+B")).clicked() {
                self.model(ModelOp::Bevel { offset: 0.2 });
                ui.close_menu();
            }
            if ui.button("Fill Hole").clicked() {
                self.model(ModelOp::FillHole);
                ui.close_menu();
            }
            if ui.button("Merge Vertices").clicked() {
                self.model(ModelOp::Merge { dist: 0.001 });
                ui.close_menu();
            }
            if ui.button("Merge to Center").clicked() {
                self.model(ModelOp::MergeCenter);
                ui.close_menu();
            }
            if ui.button("Merge Edges").clicked() {
                self.model(ModelOp::MergeEdge);
                ui.close_menu();
            }
            if cur == SelMode::Face && ui.button("Assign Color Map...").clicked() {
                self.assign_texture_dialog();
                ui.close_menu();
            }
            if ui.button("Delete").clicked() {
                self.model(ModelOp::Delete { clean: true });
                ui.close_menu();
            }
        }
    }

    /// `shape.f[0:3 7]` style component string for the selected faces, when in face mode.
    fn face_component(&self) -> Option<String> {
        if self.session.comp.mode != SelMode::Face {
            return None;
        }
        let shape = self.session.comp.node?;
        let faces: Vec<u32> = self.session.comp.sel.faces.iter().copied().collect();
        if faces.is_empty() {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        let mut i = 0;
        while i < faces.len() {
            let mut j = i;
            while j + 1 < faces.len() && faces[j + 1] == faces[j] + 1 {
                j += 1;
            }
            parts.push(if i == j { format!("{}", faces[i]) } else { format!("{}:{}", faces[i], faces[j]) });
            i = j + 1;
        }
        Some(format!("{}.f[{}]", self.session.scene.name_of(shape), parts.join(" ")))
    }

    fn assign_texture_dialog(&mut self) {
        if let Some(c) = self.face_component() {
            self.open_file_dialog(false);
            let images = self.session.scene.project_dir.as_ref().map(|d| d.join("sourceimages")).filter(|d| d.is_dir());
            if let Some(fd) = &mut self.file_dialog {
                fd.tex_faces = Some(c);
                fd.name = String::new();
                if let Some(d) = images {
                    fd.dir = d;
                }
            }
            return;
        }
        match self.session.scene.selection.first().copied() {
            Some(o) => self.open_texture_dialog(o),
            None => self.say("Select an object first"),
        }
    }

    fn polygon_menus(&mut self, ui: &mut Ui) {
        ui.menu_button("Select", |ui| {
            let cur = self.session.comp.mode;
            for (m, l, k) in [
                (SelMode::Object, "Object Mode", "F8"),
                (SelMode::Vertex, "Vertex", "F9"),
                (SelMode::Edge, "Edge", "F10"),
                (SelMode::Face, "Face", "F11"),
            ] {
                if ui.add(egui::Button::new(l).shortcut_text(k).selected(cur == m)).clicked() {
                    self.set_sel_mode(m);
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui.button("Select All").clicked() {
                self.exec("select -all");
                ui.close_menu();
            }
            if ui.button("Deselect").clicked() {
                if self.session.comp.mode == SelMode::Object {
                    self.exec("select -cl");
                } else {
                    self.session.comp.sel = Sel::default();
                    self.mark_dirty();
                }
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Grow Selection").clicked() {
                self.model(ModelOp::Grow);
                ui.close_menu();
            }
            if ui.button("Select Edge Loop").clicked() {
                self.model(ModelOp::SelectLoop);
                ui.close_menu();
            }
            ui.menu_button("Convert Selection", |ui| {
                for (m, l, k) in [(SelMode::Vertex, "To Vertices", "Ctrl+F9"), (SelMode::Edge, "To Edges", "Ctrl+F10"), (SelMode::Face, "To Faces", "Ctrl+F11")] {
                    if ui.add(egui::Button::new(l).shortcut_text(k)).clicked() {
                        self.model(ModelOp::Convert(m));
                        ui.close_menu();
                    }
                }
            });
        });
        ui.menu_button("Mesh", |ui| {
            if ui.button("Sculpt Geometry Tool...").on_hover_text("Push, pull, smooth, relax and erase on the selected surface").clicked() {
                self.sculpt_ui.show = true;
                self.start_tool(Tool::Sculpt);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Combine").clicked() {
                self.model(ModelOp::Combine);
                ui.close_menu();
            }
            if ui.button("Separate").clicked() {
                self.model(ModelOp::Separate);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Smooth").clicked() {
                self.model(ModelOp::Smooth { levels: 1 });
                ui.close_menu();
            }
            if ui.button("Triangulate").clicked() {
                self.model(ModelOp::Triangulate);
                ui.close_menu();
            }
            ui.menu_button("Mirror Cut", |ui| {
                for (i, l) in ["X axis (keep +X)", "Y axis (keep +Y)", "Z axis (keep +Z)"].iter().enumerate() {
                    if ui.button(*l).clicked() {
                        self.model(ModelOp::MirrorCut { axis: i, offset: 0.0, negative: false });
                        ui.close_menu();
                    }
                }
            });
            if ui.button("Smooth Mesh Preview to Polygons").clicked() {
                self.exec("polyConvertPreview");
                ui.close_menu();
            }
            ui.menu_button("Mirror Geometry", |ui| {
                for (i, l) in ["X axis", "Y axis", "Z axis"].iter().enumerate() {
                    if ui.button(*l).clicked() {
                        self.model(ModelOp::Mirror { axis: i });
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Booleans", |ui| {
                for (l, o) in [("Union", 1), ("Difference", 2), ("Intersection", 3)] {
                    if ui.button(l).on_hover_text("Select two closed polygon objects (the second is subtracted for Difference)").clicked() {
                        self.exec(&format!("polyCBoolOp -op {}", o));
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Normals", |ui| {
                for (l, a) in [("Soften Edge (smooth)", 180), ("Harden Edge (faceted)", 0), ("Default (40 degrees)", 40)] {
                    if ui.button(l).clicked() {
                        self.exec(&format!("polySoftEdge -a {}", a));
                        ui.close_menu();
                    }
                }
            });
            if ui.button("Average Vertices").on_hover_text("Smooth the shape by moving vertices toward their neighbours").clicked() {
                self.exec("polyAverageVertex");
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Extract").on_hover_text("Select faces first: detach them into a new object").clicked() {
                self.exec("polyExtract");
                ui.close_menu();
            }
            if ui.button("Duplicate Face").on_hover_text("Select faces first: copy them as a new shell").clicked() {
                self.model(ModelOp::ChipOff { dup: true });
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Fill Hole").on_hover_text("Select the border edges around a hole first").clicked() {
                self.model(ModelOp::FillHole);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Reverse Normals").clicked() {
                self.model(ModelOp::Reverse);
                ui.close_menu();
            }
        });
        ui.menu_button("Create UVs", |ui| {
            ui.label(egui::RichText::new("Applies to the selected faces in Face mode, else to the whole object").small().color(TEXT_DIM));
            for (c, l) in [
                ("polyPlanarProjection -md b", "Planar Mapping: Best Plane (fit to faces)"),
                ("polyAutoProjection", "Automatic Mapping"),
                ("polyPlanarProjection -md x", "Planar Mapping X"),
                ("polyPlanarProjection -md y", "Planar Mapping Y"),
                ("polyPlanarProjection -md z", "Planar Mapping Z"),
                ("polyCylindricalProjection", "Cylindrical Mapping"),
                ("polySphericalProjection", "Spherical Mapping"),
            ] {
                if ui.button(l).clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui.button("Assign Color Map...").on_hover_text("An image file or a built-in pattern, on the selected faces or the object").clicked() {
                self.assign_texture_dialog();
                ui.close_menu();
            }
            ui.menu_button("Built-in Pattern", |ui| {
                for b in crate::render::texture::BUILTINS {
                    if ui.button(b).clicked() {
                        match self.face_component() {
                            Some(c) => self.exec(&format!("assignTexture \"{}\" \"builtin:{}\"", c, b)),
                            None => self.exec(&format!("assignTexture \"builtin:{}\"", b)),
                        }
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Edit Texture", |ui| {
                for (l, c) in [("Flip U", "polyEditUV -flipU"), ("Flip V", "polyEditUV -flipV"), ("Rotate 90 degrees", "polyEditUV -a 90"), ("Tile x2", "polyEditUV -su 2 -sv 2"), ("Tile /2", "polyEditUV -su 0.5 -sv 0.5"), ("Reset Tiling, Offset and Rotation", "polyEditUV -tu 1 -tv 1 -ou 0 -ov 0 -rot 0")] {
                    if ui.button(l).clicked() {
                        self.exec(c);
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Create UVs", |ui| self.create_uvs_menu(ui));
            if ui.button("3D Paint Tool...").clicked() {
                self.uv_ui.show_paint = true;
                ui.close_menu();
            }
            if ui.button("UV Texture Editor...").clicked() {
                self.show_uv = true;
                ui.close_menu();
            }
            if ui.button("Remove Face Textures (all zones)").clicked() {
                self.exec("textureZones -clear");
                ui.close_menu();
            }
        });
        ui.menu_button("Edit Mesh", |ui| {
            if ui.add(egui::Button::new("Extrude").shortcut_text("Ctrl+E")).clicked() {
                self.model(ModelOp::Extrude { ltz: 0.0 });
                ui.close_menu();
            }
            if ui.add(egui::Button::new("Bevel").shortcut_text("Ctrl+B")).clicked() {
                self.model(ModelOp::Bevel { offset: 0.2 });
                ui.close_menu();
            }
            ui.separator();
            if ui.add(egui::Button::new("Delete Edge/Vertex").shortcut_text("Ctrl+Del")).clicked() {
                self.model(ModelOp::Delete { clean: true });
                ui.close_menu();
            }
            if ui.button("Merge Vertices").on_hover_text("Merge selected vertices closer than 0.001").clicked() {
                self.model(ModelOp::Merge { dist: 0.001 });
                ui.close_menu();
            }
            if ui.button("Merge to Center").on_hover_text("Merge all selected vertices (or the ends of selected edges) into one vertex").clicked() {
                self.model(ModelOp::MergeCenter);
                ui.close_menu();
            }
            if ui.button("Merge Edges").on_hover_text("Edge mode: select two border edges (the two sides of a seam) and join them").clicked() {
                self.model(ModelOp::MergeEdge);
                ui.close_menu();
            }
            if ui.button("Fill Hole").on_hover_text("Select edges on the rim of a gap (or nothing to fill every hole)").clicked() {
                self.model(ModelOp::FillHole);
                ui.close_menu();
            }
            if ui.button("Bridge").on_hover_text("Select the border edges of two holes (same number of edges) and connect them").clicked() {
                self.model(ModelOp::Bridge);
                ui.close_menu();
            }
            if ui.button("Offset Edge Loop").on_hover_text("Select an edge of a loop: adds a loop on each side of it").clicked() {
                self.model(ModelOp::OffsetLoop { offset: 0.25 });
                ui.close_menu();
            }
            ui.menu_button("Slide Edge", |ui| {
                for (l, t) in [("Toward first side 25%", 0.25), ("Toward first side 50%", 0.5), ("Toward other side 25%", -0.25), ("Toward other side 50%", -0.5)] {
                    if ui.button(l).clicked() {
                        self.model(ModelOp::SlideEdge { amount: t });
                        ui.close_menu();
                    }
                }
            });
            if ui.button("Select Border Edges").clicked() {
                self.model(ModelOp::SelectBorder);
                ui.close_menu();
            }
            ui.menu_button("Reflection (symmetry)", |ui| {
                let cur = self.session.scene.reflection;
                if ui.radio(cur.is_none(), "Off").clicked() {
                    self.exec("symmetricModelling -s 0");
                    ui.close_menu();
                }
                for (i, l) in ["X", "Y", "Z"].iter().enumerate() {
                    if ui.radio(cur == Some(i), *l).clicked() {
                        self.exec(&format!("symmetricModelling -s 1 -axis {}", l.to_lowercase()));
                        ui.close_menu();
                    }
                }
            });
            let mut kft = self.session.prefs.keep_faces_together;
            if ui.checkbox(&mut kft, "Keep Faces Together").on_hover_text("Off: every extruded face moves on its own").changed() {
                self.session.prefs.keep_faces_together = kft;
            }
            if ui.button("Collapse").clicked() {
                self.model(ModelOp::Collapse);
                ui.close_menu();
            }
            ui.separator();
            if ui.add(egui::Button::new("Split Polygon Tool").selected(self.tool == Tool::SplitPoly)).clicked() {
                self.start_tool(Tool::SplitPoly);
                ui.close_menu();
            }
            if ui.add(egui::Button::new("Insert Edge Loop Tool").selected(self.tool == Tool::EdgeLoop)).clicked() {
                self.start_tool(Tool::EdgeLoop);
                ui.close_menu();
            }
            ui.separator();
            if ui.add_enabled(self.last_op.is_some(), egui::Button::new("Repeat Last").shortcut_text("G")).clicked() {
                if let Some(op) = self.last_op.clone() {
                    self.model(op);
                }
                ui.close_menu();
            }
        });
    }

    /// World ray through a viewport pixel.
    fn view_ray(&self, cam: &Cam, rect: Rect, pos: Pos2) -> Option<(Vec3, Vec3)> {
        let nx = (pos.x - rect.min.x) / rect.width() * 2.0 - 1.0;
        let ny = 1.0 - (pos.y - rect.min.y) / rect.height() * 2.0;
        let inv = cam.view_proj().inverse();
        let un = |z: f32| {
            let v = inv * crate::math::Vec4::new(nx, ny, z, 1.0);
            if v.w.abs() > 1e-12 {
                v.xyz() / v.w
            } else {
                v.xyz()
            }
        };
        let (n, f) = (un(-1.0), un(1.0));
        let d = (f - n).normalized();
        if d.length_sq() < 0.5 {
            None
        } else {
            Some((n, d))
        }
    }

    /// The point of the ground plane (y = 0) under the cursor.
    fn ground_point(&self, cam: &Cam, rect: Rect, pos: Pos2) -> Option<Vec3> {
        let (o, d) = self.view_ray(cam, rect, pos)?;
        let fwd = Vec3::new(-cam.view.m[0][2], -cam.view.m[1][2], -cam.view.m[2][2]);
        let (pn, pp) = if cam.kind == ViewKind::Persp || d.y.abs() > 0.08 { (Vec3::new(0.0, 1.0, 0.0), Vec3::ZERO) } else { (fwd, Vec3::ZERO) };
        let denom = pn.dot(d);
        if denom.abs() < 1e-5 {
            return None;
        }
        let t = pn.dot(pp - o) / denom;
        (t >= 0.0).then(|| o + d * t)
    }

    /// Interactive creation: the first click is one corner (or the center of a round primitive), the second sizes it.
    fn prim_click(&mut self, cam: &Cam, rect: Rect, pos: Pos2) {
        let Some(p) = self.ground_point(cam, rect, pos) else { return };
        let snap = |v: f32, st: f32| if self.session.prefs.snap_grid && st > 1e-6 { (v / st).round() * st } else { v };
        let st = self.session.prefs.move_step;
        let p = Vec3::new(snap(p.x, st), 0.0, snap(p.z, st));
        let Some(a) = self.prim_first else {
            self.prim_first = Some(p);
            self.say("Click again to set the size");
            return;
        };
        self.prim_first = None;
        let cmd = self.prim_cmd.clone();
        let (dx, dz) = ((p.x - a.x).abs().max(0.01), (p.z - a.z).abs().max(0.01));
        let center = (a + p) * 0.5;
        let round = matches!(cmd.as_str(), "polySphere" | "polyCylinder" | "polyCone" | "polyTorus");
        let (line, at) = if round {
            // the first click is the center, the distance to the second the radius
            let r = ((p.x - a.x).powi(2) + (p.z - a.z).powi(2)).sqrt().max(0.01);
            let extra = match cmd.as_str() {
                "polyCylinder" | "polyCone" => format!(" -h {}", r * 2.0),
                "polyTorus" => format!(" -sr {}", r * 0.3),
                _ => String::new(),
            };
            (format!("{} -r {}{}", cmd, r, extra), Vec3::new(a.x, if cmd == "polyTorus" { r * 0.3 } else { r }, a.z))
        } else if cmd == "polyPlane" {
            (format!("{} -w {} -h {}", cmd, dx, dz), Vec3::new(center.x, 0.0, center.z))
        } else {
            let h = dx.min(dz);
            (format!("{} -w {} -h {} -d {}", cmd, dx, h, dz), Vec3::new(center.x, h * 0.5, center.z))
        };
        self.exec(&line);
        if let Some(n) = self.session.scene.selection.first().map(|i| self.session.scene.name_of(*i)) {
            self.exec(&format!("move {} {} {} \"{}\"", at.x, at.y, at.z, n));
        }
        self.tool = Tool::Select;
    }

    fn prim_opts_ui(&mut self, ctx: &egui::Context) {
        if !self.show_prim_opts {
            return;
        }
        let mut open = true;
        let mut create = false;
        let mut close = false;
        egui::Window::new("Polygon Primitive Options").open(&mut open).resizable(false).show(ctx, |ui| {
            let o = &mut self.prim_opts;
            ui.horizontal(|ui| {
                ui.label("Primitive");
                egui::ComboBox::from_id_salt("primkind").selected_text(PRIM_KINDS[o.kind].0).show_ui(ui, |ui| {
                    for (i, (l, _)) in PRIM_KINDS.iter().enumerate() {
                        ui.selectable_value(&mut o.kind, i, *l);
                    }
                });
            });
            let cmd = PRIM_KINDS[o.kind].1;
            egui::Grid::new("primgrid").num_columns(2).show(ui, |ui| {
                match cmd {
                    "polyCube" => {
                        for (l, i) in [("Width", 0), ("Height", 1), ("Depth", 2)] {
                            ui.label(l);
                            ui.add(egui::DragValue::new(&mut o.size[i]).speed(0.05).range(0.001..=10000.0));
                            ui.end_row();
                        }
                    }
                    "polyPlane" => {
                        for (l, i) in [("Width", 0), ("Height", 1)] {
                            ui.label(l);
                            ui.add(egui::DragValue::new(&mut o.size[i]).speed(0.05).range(0.001..=10000.0));
                            ui.end_row();
                        }
                    }
                    _ => {
                        ui.label("Radius");
                        ui.add(egui::DragValue::new(&mut o.radius).speed(0.05).range(0.001..=10000.0));
                        ui.end_row();
                        if matches!(cmd, "polyCylinder" | "polyCone") {
                            ui.label("Height");
                            ui.add(egui::DragValue::new(&mut o.size[1]).speed(0.05).range(0.001..=10000.0));
                            ui.end_row();
                        }
                        if cmd == "polyTorus" {
                            ui.label("Section radius");
                            ui.add(egui::DragValue::new(&mut o.section).speed(0.02).range(0.001..=10000.0));
                            ui.end_row();
                        }
                    }
                }
                ui.label("Subdivisions");
                ui.add(egui::DragValue::new(&mut o.subdivisions).speed(0.2).range(1.0..=200.0));
                ui.end_row();
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut o.name).desired_width(120.0));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button("Create").clicked() {
                    create = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
        if create {
            let o = self.prim_opts.clone();
            let cmd = PRIM_KINDS[o.kind].1;
            let mut line = match cmd {
                "polyCube" => format!("{} -w {} -h {} -d {}", cmd, o.size[0], o.size[1], o.size[2]),
                "polyPlane" => format!("{} -w {} -h {}", cmd, o.size[0], o.size[1]),
                "polyCylinder" | "polyCone" => format!("{} -r {} -h {}", cmd, o.radius, o.size[1]),
                "polyTorus" => format!("{} -r {} -sr {}", cmd, o.radius, o.section),
                _ => format!("{} -r {}", cmd, o.radius),
            };
            let round = matches!(cmd, "polySphere" | "polyCylinder" | "polyCone" | "polyTorus");
            // 1 means "the primitive's default smoothness" for round shapes
            let sub = if round && o.subdivisions <= 1.0 { if cmd == "polyTorus" { 24.0 } else { 20.0 } } else { o.subdivisions };
            if round || sub > 1.0 {
                line.push_str(&format!(" -sx {}", sub.round()));
            }
            if !o.name.trim().is_empty() {
                line.push_str(&format!(" -name \"{}\"", o.name.trim()));
            }
            self.exec(&line);
        }
        if !open || close {
            self.show_prim_opts = false;
        }
    }

    fn joint_click(&mut self, cam: &Cam, rect: Rect, pos: Pos2) {
        let anchor = match self.joint_last {
            Some(j) => self.session.scene.world_matrix(j).translation_part(),
            None => Vec3::ZERO,
        };
        let (o, d) = match self.view_ray(cam, rect, pos) {
            Some(r) => r,
            None => return,
        };
        // grid plane in perspective, the view plane through the last joint otherwise
        let fwd = Vec3::new(-cam.view.m[0][2], -cam.view.m[1][2], -cam.view.m[2][2]);
        let (pn, pp) = if cam.kind == ViewKind::Persp && d.y.abs() > 0.08 {
            (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, if self.joint_last.is_some() { anchor.y } else { 0.0 }, 0.0))
        } else {
            (fwd, anchor)
        };
        let denom = pn.dot(d);
        if denom.abs() < 1e-5 {
            return;
        }
        let t = pn.dot(pp - o) / denom;
        if t < 0.0 {
            return;
        }
        let w = o + d * t;
        match cmd_rig::create_joint(&mut self.session, w, None, self.joint_last) {
            Ok(id) => {
                self.joint_last = Some(id);
                self.session.scene.selection = vec![id];
                self.say(&format!("Created {}", self.session.scene.name_of(id)));
                self.scene_edited();
            }
            Err(e) => self.say(&e),
        }
    }

    fn ik_click(&mut self, cam: &Cam, rect: Rect, pos: Pos2) {
        let vp = cam.view_proj();
        let joints = self.session.scene.ids_where(|d| matches!(d, NodeData::Joint));
        let mut best: Option<(f32, NodeId)> = None;
        for j in joints {
            let p = self.session.scene.world_matrix(j).translation_part();
            if let Some((x, y, _)) = project(&vp, p, rect.width(), rect.height()) {
                let d = (rect.min + vec2(x, y) - pos).length();
                if d < 18.0 && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                    best = Some((d, j));
                }
            }
        }
        let j = match best {
            Some((_, j)) => j,
            None => return,
        };
        match self.ik_start {
            None => {
                self.ik_start = Some(j);
                self.session.scene.selection = vec![j];
                self.say("Click the end joint of the chain");
            }
            Some(a) => {
                match cmd_rig::create_ik(&mut self.session, a, j, None) {
                    Ok(h) => {
                        self.session.scene.selection = vec![h];
                        self.say(&format!("Created {}", self.session.scene.name_of(h)));
                        self.session.scene.solve_iks();
                        self.scene_edited();
                    }
                    Err(e) => self.say(&e),
                }
                self.ik_start = None;
            }
        }
    }

    fn rig_menus(&mut self, ui: &mut Ui) {
        ui.menu_button("Skeleton", |ui| {
            if ui.add(egui::Button::new("Joint Tool").selected(self.tool == Tool::Joint)).clicked() {
                self.start_tool(Tool::Joint);
                ui.close_menu();
            }
            if ui.add(egui::Button::new("IK Handle Tool").selected(self.tool == Tool::IkHandle)).clicked() {
                self.start_tool(Tool::IkHandle);
                ui.close_menu();
            }
            ui.menu_button("IK Handle (selected joints)", |ui| {
                ui.label(egui::RichText::new("Select the start joint, then the end joint").small().color(TEXT_DIM));
                for (l, sol) in [("Rotate Plane Solver", "ikRPsolver"), ("Single Chain Solver", "ikSCsolver")] {
                    if ui.button(l).clicked() {
                        let sel: Vec<String> = self.session.scene.selection.iter().filter(|i| self.session.scene.is_joint(**i)).map(|i| self.session.scene.name_of(*i)).collect();
                        if sel.len() >= 2 {
                            self.exec(&format!("ikHandle -sol {} -sj {} -ee {}", sol, sel[0], sel[sel.len() - 1]));
                        } else {
                            self.say("Select the start joint and the end joint");
                        }
                        ui.close_menu();
                    }
                }
            });
            ui.separator();
            for (l, c) in [
                ("Connect Joint (parent to last selected)", "connectJoint"),
                ("Disconnect Joint", "disconnectJoint"),
                ("Insert Joint", "insertJoint"),
                ("Remove Joint", "removeJoint"),
                ("Reroot Skeleton", "reroot"),
                ("Orient Joint (xyz, y up)", "orientJoint -oj xyz -sao yup"),
                ("Orient Joint (yzx, z up)", "orientJoint -oj yzx -sao zup"),
                ("Set Preferred Angle", "setPreferredAngle"),
                ("Assume Preferred Angle", "assumePreferredAngle"),
            ] {
                if ui.button(l).clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
            ui.menu_button("Mirror Joint", |ui| {
                for (l, plane) in [("YZ plane (X flips)", "-mirrorYZ"), ("XY plane (Z flips)", "-mirrorXY"), ("XZ plane (Y flips)", "-mirrorXZ")] {
                    if ui.button(format!("{}, Behavior", l)).clicked() {
                        self.exec(&format!("mirrorJoint {} -mirrorBehavior -searchReplace l_ r_", plane));
                        ui.close_menu();
                    }
                    if ui.button(format!("{}, Orientation", l)).clicked() {
                        self.exec(&format!("mirrorJoint {} -searchReplace l_ r_", plane));
                        ui.close_menu();
                    }
                }
            });
            ui.separator();
            let mut axes = self.session.scene.show.local_axes;
            if ui.checkbox(&mut axes, "Show Local Rotation Axes").on_hover_text("Of the selected joints").changed() {
                self.session.scene.show.local_axes = axes;
                self.mark_dirty();
            }
            ui.menu_button("Joint Size", |ui| {
                for v in [0.1, 0.25, 0.5, 1.0, 2.0] {
                    if ui.button(format!("{}", v)).clicked() {
                        self.exec(&format!("jointDisplayScale {}", v));
                        ui.close_menu();
                    }
                }
            });
        });
        ui.menu_button("Constrain", |ui| {
            ui.checkbox(&mut self.constrain_offset, "Maintain Offset");
            ui.separator();
            for (l, c) in [("Point", "pointConstraint"), ("Orient", "orientConstraint"), ("Scale", "scaleConstraint"), ("Aim", "aimConstraint"), ("Parent", "parentConstraint")] {
                if ui.button(l).on_hover_text("Select the target(s), then the object to constrain").clicked() {
                    let cmd = format!("{}{}", c, if self.constrain_offset { " -mo" } else { "" });
                    self.exec(&cmd);
                    ui.close_menu();
                }
            }
            if ui.button("Normal").on_hover_text("Select the surface, then the object: its Y axis follows the surface normal").clicked() {
                self.exec("normalConstraint -aim 0 1 0 -u 1 0 0");
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Pole Vector").on_hover_text("Select the locator, then the IK handle").clicked() {
                self.exec("poleVectorConstraint");
                ui.close_menu();
            }
        });
        ui.menu_button("Deform", |ui| {
            ui.menu_button("Create Deformers", |ui| {
                for (l, c, tip) in [
                    ("Cluster", "cluster", "Select vertices (or curve CVs) first"),
                    ("Soft Modification", "softMod -radius 1.5", "Select the vertices where the soft modification starts"),
                    ("Wire Tool", "wire -dds 1", "Select the geometry, then the NURBS curve"),
                    ("Point on Curve", "pointOnCurve -ep 0", "Select a curve: a locator is made at its first edit point"),
                    ("Jiggle Deformer", "jiggle -s 0.1 -d 0.2", "Select the mesh or some of its vertices; play the animation forward from the start"),
                    ("Wave", "nonLinear -type wave", "A live wave deformer on the selected mesh"),
                ] {
                    if ui.button(l).on_hover_text(tip).clicked() {
                        self.exec(c);
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Edit Deformers", |ui| {
                for (l, c, tip) in [
                    ("Deformer Order: Move Up", "reorderDeformer %SEL% up", "Select the deformer node in the Channel Box / outliner first"),
                    ("Deformer Order: Move Down", "reorderDeformer %SEL% down", ""),
                    ("List Deformers", "listDeformers", ""),
                ] {
                    if ui.button(l).on_hover_text(tip).clicked() {
                        let sel = self.session.scene.selection.first().map(|i| self.session.scene.name_of(*i)).unwrap_or_default();
                        self.exec(&c.replace("%SEL%", &sel));
                        ui.close_menu();
                    }
                }
                for l in ["Edit Membership Tool...", "Paint Set Membership Tool...", "Paint Jiggle Weights Tool..."] {
                    if ui.button(l).on_hover_text("Opens the membership and weights window").clicked() {
                        self.show_member = true;
                        ui.close_menu();
                    }
                }
            });
            ui.separator();
            if ui.button("Create Blend Shape").on_hover_text("Select the target shapes, then the base shape").clicked() {
                self.exec("blendShape");
                ui.close_menu();
            }
            if ui.button("Create Lattice").on_hover_text("A lattice fitted to the selected mesh; edit its points in component mode").clicked() {
                self.exec("lattice -dv 2 5 2");
                ui.close_menu();
            }
            ui.separator();
            ui.menu_button("Nonlinear", |ui| {
                for (l, ty) in [("Bend", "bend"), ("Twist", "twist"), ("Flare", "flare"), ("Squash", "squash"), ("Sine", "sine")] {
                    if ui.button(l).on_hover_text("Adds a live deformer node: edit it in the Channel Box").clicked() {
                        self.exec(&format!("nonLinear -type {}", ty));
                        ui.close_menu();
                    }
                }
            });
        });
        ui.menu_button("Paint Effects", |ui| {
            if ui.button("Visor...").on_hover_text("Brush presets, brush settings and the canvas").clicked() {
                self.pfx_ui.show = true;
                ui.close_menu();
            }
            if ui.button("Paint Effects Tool").clicked() {
                self.start_tool(Tool::PaintFx);
                ui.close_menu();
            }
            if ui.button("Get Brush...").on_hover_text("Pick a brush preset in the Visor").clicked() {
                self.pfx_ui.show = true;
                ui.close_menu();
            }
            if ui.button("Save Brush Preset").on_hover_text("Write the current brush to <brush>.brush in the project's folder").clicked() {
                let b = self.pfx_ui.brush.clone();
                if b.is_empty() {
                    self.say("Paint a stroke or pick a brush in the Visor first");
                } else {
                    self.exec(&format!("brushPreset -save {} {}", Self::quote(&b), Self::quote(&format!("{}.brush", b))));
                }
                ui.close_menu();
            }
            if ui.button("Paint Canvas").on_hover_text("The 2D canvas in the Visor window").clicked() {
                self.pfx_ui.show = true;
                ui.close_menu();
            }
            for (l, c) in [("Make Paintable", "makePaintable"), ("Make Not Paintable", "makePaintable -off"), ("Share One Brush", "shareBrush"), ("Get Settings from Selected Stroke", "brushFromStroke"), ("Convert Paint Effects to Polygons", "strokeToPoly")] {
                if ui.button(l).clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
        });
        ui.menu_button("Skin", |ui| {
            for (l, c) in [("Bind Skin: Smooth", "bindSkin -smooth"), ("Bind Skin: Rigid (also binds a selected lattice)", "bindSkin -rigid"), ("Bind to Selected Joints (smooth)", "bindSkin -toSelectedBones")] {
                if ui.button(l).on_hover_text("Select the skeleton root, then the mesh").clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
            if ui.button("Detach Skin").clicked() {
                self.exec("detachSkin");
                ui.close_menu();
            }
            if ui.button("Go to Bind Pose").clicked() {
                self.exec("goToBindPose");
                ui.close_menu();
            }
            ui.separator();
            ui.menu_button("Edit Rigid Skin", |ui| {
                if ui.button("Create Flexor...").clicked() {
                    self.show_flexor = true;
                    ui.close_menu();
                }
            });
            if ui.button("Paint Skin Weights Tool...").clicked() {
                self.skin_ui.show = true;
                ui.close_menu();
            }
            for (l, c) in [("Normalize Weights", "skinNormalize"), ("Prune Small Weights", "skinPrune -t 0.01"), ("Smooth Skin Weights", "skinSmooth -i 3"), ("Reset Weights to Default", "skinReset"), ("Mirror Skin Weights (+X to -X)", "skinMirror -axis x -direction pos"), ("Skin Info", "skinInfo")] {
                if ui.button(l).clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
        });
        ui.menu_button("Character", |ui| {
            if ui.button("Create Character Set").on_hover_text("From the selected objects").clicked() {
                self.exec("character -n character");
                ui.close_menu();
            }
            ui.menu_button("Redirect", |ui| {
                if ui.button("Translation only").on_hover_text("Select the master node first").clicked() {
                    self.exec("redirect -translation");
                    ui.close_menu();
                }
                if ui.button("Translation and rotation").clicked() {
                    self.exec("redirect");
                    ui.close_menu();
                }
                if ui.button("Remove redirect").clicked() {
                    self.exec("redirect -remove");
                    ui.close_menu();
                }
            });
            if ui.button("Trax Editor...").clicked() {
                self.trax_ui.show = true;
                ui.close_menu();
            }
            ui.separator();
            let chars: Vec<(NodeId, String)> = self.session.scene.ids_where(|d| matches!(d, NodeData::Character(_))).into_iter().map(|c| (c, self.session.scene.name_of(c))).collect();
            if ui.radio(self.session.current_character.is_none(), "No current character").clicked() {
                self.exec("setCurrentCharacter -none");
                ui.close_menu();
            }
            for (c, n) in chars {
                if ui.radio(self.session.current_character == Some(c), &n).on_hover_text("Set Key keys the whole character").clicked() {
                    self.exec(&format!("setCurrentCharacter {}", n));
                    ui.close_menu();
                }
            }
        });
    }

    /// Paint Skin Weights window: joint, brush and operation.
    fn skin_window(&mut self, ctx: &egui::Context) {
        if !self.skin_ui.show {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let shape = cmd_model::current_shape(&self.session).filter(|s| self.session.scene.skin_of(*s).is_some());
        let joints: Vec<(NodeId, String)> = shape.and_then(|s| self.session.scene.skin_data(s)).map(|sk| sk.joints.iter().map(|j| (*j, self.session.scene.name_of(*j))).collect()).unwrap_or_default();
        egui::Window::new("Paint Skin Weights").open(&mut open).default_size([300.0, 320.0]).show(ctx, |ui| {
            let Some(sh) = shape else {
                ui.label("Select a skinned mesh.");
                return;
            };
            ui.label(format!("Mesh: {}", self.session.scene.name_of(sh)));
            egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                for (j, n) in &joints {
                    if ui.selectable_label(self.skin_ui.joint == Some(*j), n).clicked() {
                        self.skin_ui.joint = Some(*j);
                        self.session.scene.skin_vis = Some((sh, *j));
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                for (i, l) in ["Replace", "Add", "Scale", "Smooth"].iter().enumerate() {
                    ui.selectable_value(&mut self.skin_ui.op, i, *l);
                }
            });
            ui.add(egui::Slider::new(&mut self.skin_ui.value, 0.0..=1.0).text("Value"));
            ui.add(egui::Slider::new(&mut self.skin_ui.radius, 0.05..=3.0).text("Radius"));
            ui.add(egui::Slider::new(&mut self.skin_ui.strength, 0.05..=1.0).text("Opacity"));
            ui.horizontal(|ui| {
                if ui.add_enabled(self.skin_ui.joint.is_some(), egui::Button::new("Paint (drag on the mesh)")).clicked() {
                    self.start_tool(Tool::PaintSkin);
                }
                if ui.button("Flood Smooth").clicked() {
                    cmds.push("skinSmooth -i 1".to_string());
                }
            });
            ui.label(egui::RichText::new("Blue: no weight, red: full weight of the selected joint.").small().color(TEXT_DIM));
        });
        for c in cmds {
            self.exec(&c);
        }
        if !open {
            self.skin_ui.show = false;
            self.session.scene.skin_vis = None;
            if self.tool == Tool::PaintSkin {
                self.tool = Tool::Select;
            }
        }
    }

    /// One dab of the skin paint brush under the pointer.
    fn skin_paint_at(&mut self, cam: &Cam, rect: Rect, pos: Pos2) {
        let Some(shape) = cmd_model::current_shape(&self.session).filter(|s| self.session.scene.skin_of(*s).is_some()) else { return };
        let Some(joint) = self.skin_ui.joint else {
            self.say("Pick a joint in the Paint Skin Weights window");
            return;
        };
        let (nx, ny) = ((pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height());
        let inv = cam.view_proj().inverse();
        let un = |z: f32| {
            let c = inv * Vec3::new(nx * 2.0 - 1.0, 1.0 - ny * 2.0, z).extend(1.0);
            Vec3::new(c.x / c.w, c.y / c.w, c.z / c.w)
        };
        let (near, far) = (un(-1.0), un(1.0));
        let owner = self.session.scene.node(shape).and_then(|n| n.parent).unwrap_or(shape);
        let wi = self.session.scene.world_matrix(owner).inverse();
        let (o, d) = (wi.transform_point(near), wi.transform_dir((far - near).normalized()));
        // nearest triangle hit on the deformed mesh
        let hit = match self.session.scene.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => {
                let mut best: Option<f32> = None;
                for t in md.mesh.triangulate() {
                    let (a, b, c) = (md.mesh.positions[t[0] as usize], md.mesh.positions[t[1] as usize], md.mesh.positions[t[2] as usize]);
                    let (e1, e2) = (b - a, c - a);
                    let pv = d.cross(e2);
                    let det = e1.dot(pv);
                    if det.abs() < 1e-12 {
                        continue;
                    }
                    let iv = 1.0 / det;
                    let tv = o - a;
                    let u = tv.dot(pv) * iv;
                    if !(0.0..=1.0).contains(&u) {
                        continue;
                    }
                    let qv = tv.cross(e1);
                    let v = d.dot(qv) * iv;
                    if v < 0.0 || u + v > 1.0 {
                        continue;
                    }
                    let tt = e2.dot(qv) * iv;
                    if tt > 1e-5 && best.map(|b| tt < b).unwrap_or(true) {
                        best = Some(tt);
                    }
                }
                best.map(|t| o + d * t)
            }
            _ => None,
        };
        if let Some(p) = hit {
            let op = [crate::dag::skinning::PaintOp::Replace, crate::dag::skinning::PaintOp::Add, crate::dag::skinning::PaintOp::Scale, crate::dag::skinning::PaintOp::Smooth][self.skin_ui.op.min(3)];
            match self.session.scene.paint_skin(shape, p, self.skin_ui.radius, joint, self.skin_ui.value, op, self.skin_ui.strength) {
                Ok(_) => self.mark_dirty(),
                Err(e) => self.say(&e),
            }
        }
    }

    fn start_tool(&mut self, t: Tool) {
        self.tool = t;
        self.split_pts.clear();
        self.loop_drag = None;
        self.ik_start = None;
        if t == Tool::Joint {
            // continue from the selected joint when there is one
            self.joint_last = self.session.scene.selection.first().copied().filter(|i| self.session.scene.is_joint(*i));
        } else {
            self.joint_last = None;
        }
        if matches!(t, Tool::SplitPoly | Tool::EdgeLoop) && self.session.comp.mode == SelMode::Object {
            if cmd_model::current_shape(&self.session).is_none() {
                self.say("Select a polygon object first");
                self.tool = Tool::Select;
            }
        }
    }
}

// ====================================================================== attribute editor and UV editor

impl App {
    fn ae_float(&mut self, ui: &mut Ui, node: NodeId, attr: &str, label: &str, speed: f64, range: Option<(f64, f64)>) {
        let (cur, keyed, driven) = {
            let sc = &mut self.session.scene;
            let keyed = sc.anim.is_animated(node, attr);
            let driven = sc.node(node).and_then(|n| n.plug(attr)).map(|p| p.source.is_some()).unwrap_or(false);
            (sc.get_attr(node, attr).unwrap_or(0.0), keyed, driven)
        };
        let mut v = cur as f64;
        let mut changed = false;
        ui.horizontal(|ui| {
            let bg = if keyed { KEYED } else if driven { CONNECTED } else { Color32::TRANSPARENT };
            egui::Frame::NONE.fill(bg).inner_margin(egui::Margin::symmetric(2, 0)).show(ui, |ui| {
                ui.add_sized(vec2(110.0, 18.0), egui::Label::new(label).truncate());
            });
            ui.add_enabled_ui(!driven, |ui| match range {
                Some((lo, hi)) => {
                    changed = ui.add(egui::Slider::new(&mut v, lo..=hi).max_decimals(3)).changed();
                }
                None => {
                    changed = ui.add(egui::DragValue::new(&mut v).speed(speed).max_decimals(4)).changed();
                }
            });
        });
        if changed {
            let layer = self.session.scene.anim.active_layer;
            let _ = self.session.scene.set_attr(node, attr, v as f32);
            if self.auto_key {
                let _ = self.session.scene.set_keyframe(node, attr, None, Some(v as f32), crate::anim::Interp::Bezier, layer);
            }
            self.scene_edited();
        }
    }

    fn ae_triple(&mut self, ui: &mut Ui, node: NodeId, base: &str, label: &str, speed: f64) {
        ui.horizontal(|ui| {
            ui.add_sized(vec2(70.0, 18.0), egui::Label::new(label));
            for axis in ["X", "Y", "Z"] {
                let attr = format!("{}{}", base, axis);
                let (cur, keyed) = {
                    let sc = &mut self.session.scene;
                    (sc.get_attr(node, &attr).unwrap_or(0.0), sc.anim.is_animated(node, &attr))
                };
                let mut v = cur;
                let bg = if keyed { KEYED } else { Color32::TRANSPARENT };
                let mut ch = false;
                egui::Frame::NONE.fill(bg).show(ui, |ui| {
                    ch = ui.add(egui::DragValue::new(&mut v).speed(speed).max_decimals(3).prefix(format!("{} ", axis))).changed();
                });
                if ch {
                    let layer = self.session.scene.anim.active_layer;
                    let _ = self.session.scene.set_attr(node, &attr, v);
                    if self.auto_key {
                        let _ = self.session.scene.set_keyframe(node, &attr, None, Some(v), self.default_interp(), layer);
                    }
                    self.scene_edited();
                }
            }
        });
    }

    fn ae_color(&mut self, ui: &mut Ui, node: NodeId, label: &str) {
        self.ae_color_attr(ui, node, label, "color");
    }

    /// Color swatch editing `<prefix>R`, `<prefix>G` and `<prefix>B`.
    fn ae_color_attr(&mut self, ui: &mut Ui, node: NodeId, label: &str, prefix: &str) {
        let g = |s: &mut Scene, a: &str| s.get_attr(node, &format!("{}{}", prefix, a)).unwrap_or(0.0);
        let mut c = [g(&mut self.session.scene, "R"), g(&mut self.session.scene, "G"), g(&mut self.session.scene, "B")];
        ui.horizontal(|ui| {
            ui.add_sized(vec2(110.0, 18.0), egui::Label::new(label));
            if ui.color_edit_button_rgb(&mut c).changed() {
                for (a, v) in [("R", c[0]), ("G", c[1]), ("B", c[2])] {
                    let _ = self.session.scene.set_attr(node, &format!("{}{}", prefix, a), v);
                }
                self.scene_edited();
            }
        });
    }

    fn ae_toggle(&mut self, ui: &mut Ui, node: NodeId, attr: &str, label: &str) {
        let mut on = self.session.scene.get_attr(node, attr).unwrap_or(0.0) > 0.5;
        if ui.checkbox(&mut on, label).changed() {
            let _ = self.session.scene.set_attr(node, attr, if on { 1.0 } else { 0.0 });
            self.scene_edited();
        }
    }

    fn ae_choice(&mut self, ui: &mut Ui, node: NodeId, attr: &str, label: &str, names: &[&str]) {
        let cur = self.session.scene.get_attr(node, attr).unwrap_or(0.0).round() as usize;
        let mut v = cur.min(names.len() - 1);
        ui.horizontal(|ui| {
            ui.add_sized(vec2(110.0, 18.0), egui::Label::new(label));
            egui::ComboBox::from_id_salt(("aechoice", attr)).selected_text(names[v]).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    ui.selectable_value(&mut v, i, *n);
                }
            });
        });
        if v != cur {
            let _ = self.session.scene.set_attr(node, attr, v as f32);
            self.scene_edited();
        }
    }

    fn attribute_editor(&mut self, ctx: &egui::Context) {
        if !self.show_ae {
            return;
        }
        let mut open = true;
        // tabs: the object, its shapes, history nodes and the material
        let mut tabs: Vec<(String, NodeId)> = Vec::new();
        let sel = self.session.scene.selection.first().copied();
        if let Some(id) = sel {
            let sc = &self.session.scene;
            if sc.node(id).is_some() {
                tabs.push((sc.name_of(id), id));
                let kids: Vec<NodeId> = sc.node(id).map(|n| n.children.clone()).unwrap_or_default();
                for c in kids {
                    tabs.push((sc.name_of(c), c));
                    if let Some(NodeData::Mesh(md)) = sc.node(c).map(|n| &n.data) {
                        for h in md.history.iter().rev() {
                            tabs.push((sc.name_of(*h), *h));
                        }
                        if let Some(m) = md.material {
                            tabs.push((sc.name_of(m), m));
                        }
                    }
                }
            }
        }
        if self.ae_tab.map(|t| !tabs.iter().any(|(_, n)| *n == t)).unwrap_or(true) {
            self.ae_tab = tabs.first().map(|t| t.1);
        }
        egui::Window::new("Attribute Editor").open(&mut open).default_size([420.0, 560.0]).show(ctx, |ui| {
            if tabs.is_empty() {
                ui.label(egui::RichText::new("Select an object").color(TEXT_DIM));
                return;
            }
            egui::ScrollArea::horizontal().id_salt("aetabs").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (name, id) in &tabs {
                        if ui.selectable_label(self.ae_tab == Some(*id), name).clicked() {
                            self.ae_tab = Some(*id);
                        }
                    }
                });
            });
            ui.separator();
            if let Some(node) = self.ae_tab {
                egui::ScrollArea::vertical().id_salt("aebody").auto_shrink([false, false]).show(ui, |ui| {
                    self.ae_node_ui(ui, node);
                });
            }
        });
        if !open {
            self.show_ae = false;
        }
    }

    fn ae_node_ui(&mut self, ui: &mut Ui, node: NodeId) {
        let (name, tname, is_transform) = match self.session.scene.node(node) {
            Some(n) => (n.name.clone(), n.data.type_name(), n.data.is_transform()),
            None => return,
        };
        ui.horizontal(|ui| {
            ui.label("Node name");
            let mut buf = name.clone();
            let r = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(180.0));
            if r.lost_focus() && buf != name && ui.input(|i| i.key_pressed(Key::Enter)) && !buf.is_empty() {
                let _ = self.session.scene.rename(node, &buf);
                self.scene_edited();
            }
            ui.label(egui::RichText::new(tname).small().color(TEXT_DIM));
        });
        ui.add_space(4.0);
        enum K {
            Transform,
            Joint,
            Ik,
            Mesh,
            Light,
            Emitter,
            Constraint,
            Camera(bool),
            Material,
            Texture,
            Poly,
            Other,
        }
        let kind = match self.session.scene.node(node).map(|n| &n.data) {
            Some(NodeData::Transform) => K::Transform,
            Some(NodeData::Joint) => K::Joint,
            Some(NodeData::IkHandle(_)) => K::Ik,
            Some(NodeData::Mesh(_)) => K::Mesh,
            Some(NodeData::Light) => K::Light,
            Some(NodeData::Emitter) => K::Emitter,
            Some(NodeData::Constraint(_)) => K::Constraint,
            Some(NodeData::Texture(_)) => K::Texture,
            Some(NodeData::Camera) => K::Camera(false),
            Some(NodeData::StereoCamera) => K::Camera(true),
            Some(NodeData::Material) => K::Material,
            Some(NodeData::PolyOp(_)) => K::Poly,
            _ => K::Other,
        };
        if is_transform {
            egui::CollapsingHeader::new("Transform Attributes").default_open(true).show(ui, |ui| {
                self.ae_triple(ui, node, "translate", "Translate", 0.05);
                self.ae_triple(ui, node, "rotate", "Rotate", 0.5);
                self.ae_triple(ui, node, "scale", "Scale", 0.02);
                let mut vis = self.session.scene.get_attr(node, "visibility").unwrap_or(1.0) > 0.5;
                if ui.checkbox(&mut vis, "Visibility").changed() {
                    let _ = self.session.scene.set_attr(node, "visibility", if vis { 1.0 } else { 0.0 });
                    self.scene_edited();
                }
            });
        }
        match kind {
            K::Joint => {
                egui::CollapsingHeader::new("Joint").default_open(true).show(ui, |ui| {
                    self.ae_float(ui, node, "radius", "Joint Radius", 0.02, Some((0.01, 5.0)));
                });
            }
            K::Ik => {
                egui::CollapsingHeader::new("IK Handle").default_open(true).show(ui, |ui| {
                    self.ae_float(ui, node, "enable", "Enable (0/1)", 1.0, Some((0.0, 1.0)));
                    if let Some(NodeData::IkHandle(d)) = self.session.scene.node(node).map(|n| &n.data) {
                        let nm = |o: Option<NodeId>| o.map(|i| self.session.scene.name_of(i)).unwrap_or_else(|| "-".into());
                        ui.label(format!("Start joint: {}", nm(d.start)));
                        ui.label(format!("End joint: {}", nm(d.end)));
                    }
                });
            }
            K::Mesh => {
                egui::CollapsingHeader::new("Mesh").default_open(true).show(ui, |ui| {
                    if let Some(NodeData::Mesh(md)) = self.session.scene.node(node).map(|n| &n.data) {
                        ui.label(format!("{} vertices, {} faces, {} triangles", md.mesh.positions.len(), md.mesh.faces.len(), md.mesh.triangle_count()));
                        ui.label(format!("{} history node(s)", md.history.len()));
                    }
                    self.ae_float(ui, node, "smoothAngle", "Smooth Angle", 1.0, Some((0.0, 180.0)));
                    egui::CollapsingHeader::new("Render Stats").default_open(false).show(ui, |ui| {
                        self.ae_toggle(ui, node, "castsShadows", "Casts Shadows");
                        self.ae_toggle(ui, node, "receiveShadows", "Receive Shadows");
                        self.ae_toggle(ui, node, "motionBlur", "Motion Blur");
                        self.ae_toggle(ui, node, "primaryVisibility", "Primary Visibility");
                        self.ae_toggle(ui, node, "visibleInReflections", "Visible In Reflections");
                    });
                    if let Some(NodeData::Mesh(md)) = self.session.scene.node(node).map(|n| &n.data) {
                        let nz = md.zones.len();
                        ui.label(if nz == 0 { "No face textures (one material)".to_string() } else { format!("{} texture zone(s) on faces", nz) });
                    }
                    if let Some(sk) = self.session.scene.skin_of(node) {
                        ui.label(format!("Bound by {}", self.session.scene.name_of(sk)));
                    }
                });
                egui::CollapsingHeader::new("UV Mapping").default_open(true).show(ui, |ui| {
                    let mode = self.session.scene.get_attr(node, "uvMode").unwrap_or(0.0) as i32;
                    let names = ["Automatic (box)", "Planar X", "Planar Y", "Planar Z", "Cylindrical", "Spherical"];
                    let mut m = mode;
                    egui::ComboBox::from_id_salt("aeuvmode").selected_text(names[(m.clamp(0, 5)) as usize]).show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            ui.selectable_value(&mut m, i as i32, *n);
                        }
                    });
                    if m != mode {
                        let _ = self.session.scene.set_attr(node, "uvMode", m as f32);
                        self.scene_edited();
                    }
                    self.ae_float(ui, node, "uvTilingU", "Tiling U", 0.05, None);
                    self.ae_float(ui, node, "uvTilingV", "Tiling V", 0.05, None);
                    self.ae_float(ui, node, "uvOffsetU", "Offset U", 0.01, None);
                    self.ae_float(ui, node, "uvOffsetV", "Offset V", 0.01, None);
                    self.ae_float(ui, node, "uvRotate", "Rotate", 0.5, Some((-180.0, 180.0)));
                    if ui.button("Open UV Texture Editor").clicked() {
                        self.show_uv = true;
                    }
                });
            }
            K::Constraint => {
                let (kind, n) = match self.session.scene.node(node).map(|n| &n.data) {
                    Some(NodeData::Constraint(c)) => (c.kind, c.targets.len()),
                    _ => return,
                };
                egui::CollapsingHeader::new(format!("{} Constraint", kind.name())).default_open(true).show(ui, |ui| {
                    self.ae_toggle(ui, node, "enable", "Enabled");
                    for i in 0..n {
                        let tname = match self.session.scene.node(node).map(|n| &n.data) {
                            Some(NodeData::Constraint(c)) => c.targets.get(i).map(|t| self.session.scene.name_of(*t)).unwrap_or_default(),
                            _ => String::new(),
                        };
                        self.ae_float(ui, node, &format!("w{}", i), &format!("{} W", tname), 0.02, Some((0.0, 10.0)));
                    }
                    use crate::dag::nodes::ConstraintKind as CK;
                    match kind {
                        CK::Point => self.ae_triple(ui, node, "offset", "Offset", 0.05),
                        CK::Orient => self.ae_triple(ui, node, "offset", "Offset (deg)", 0.5),
                        CK::Scale => self.ae_triple(ui, node, "offset", "Scale Offset", 0.02),
                        CK::Aim | CK::Normal => {
                            self.ae_triple(ui, node, "aim", "Aim Vector", 0.05);
                            self.ae_triple(ui, node, "up", "Up Vector", 0.05);
                            self.ae_choice(ui, node, "worldUpType", "World Up Type", &["Scene Up", "Object Up (second target)", "Vector", "None"]);
                            self.ae_triple(ui, node, "worldUp", "World Up", 0.05);
                        }
                        CK::Parent => {}
                    }
                });
            }
            K::Emitter => {
                egui::CollapsingHeader::new("Emitter").default_open(true).show(ui, |ui| {
                    self.ae_toggle(ui, node, "enabled", "Emit");
                    self.ae_float(ui, node, "rate", "Rate (per second)", 0.5, Some((0.0, 2000.0)));
                    self.ae_float(ui, node, "speed", "Speed", 0.05, Some((0.0, 100.0)));
                    self.ae_float(ui, node, "spread", "Spread", 0.01, Some((0.0, 1.5)));
                    self.ae_float(ui, node, "lifespan", "Lifespan (s, 0 = forever)", 0.05, Some((0.0, 60.0)));
                    self.ae_float(ui, node, "maxParticles", "Max Particles", 1.0, Some((0.0, 20000.0)));
                    self.ae_float(ui, node, "startFrame", "Start Frame", 1.0, None);
                    self.ae_float(ui, node, "seed", "Random Seed", 1.0, Some((0.0, 100000.0)));
                    ui.label(egui::RichText::new("Move and rotate the emitter object to place and aim it (particles leave along +Y).").color(TEXT_DIM));
                });
            }
            K::Light => {
                let lt = self.session.scene.get_attr(node, "lightType").unwrap_or(0.0).round() as i32;
                egui::CollapsingHeader::new("Light").default_open(true).show(ui, |ui| {
                    self.ae_choice(ui, node, "lightType", "Type", &["Point", "Directional", "Spot", "Ambient", "Area"]);
                    self.ae_color(ui, node, "Color");
                    self.ae_float(ui, node, "intensity", "Intensity", 0.05, Some((0.0, 10.0)));
                    if lt == 0 || lt == 2 || lt == 4 {
                        self.ae_choice(ui, node, "decayRate", "Decay Rate", &["No Decay", "Linear", "Quadratic", "Cubic", "Soft (default)"]);
                    }
                    self.ae_toggle(ui, node, "emitDiffuse", "Emit Diffuse");
                    self.ae_toggle(ui, node, "emitSpecular", "Emit Specular");
                });
                if lt == 2 {
                    egui::CollapsingHeader::new("Spot Light").default_open(true).show(ui, |ui| {
                        self.ae_float(ui, node, "coneAngle", "Cone Angle", 0.5, Some((2.0, 179.0)));
                        self.ae_float(ui, node, "penumbraAngle", "Penumbra Angle", 0.25, Some((0.0, 60.0)));
                        self.ae_float(ui, node, "dropoff", "Dropoff", 0.1, Some((0.0, 255.0)));
                    });
                }
                if lt == 4 {
                    egui::CollapsingHeader::new("Area Light").default_open(true).show(ui, |ui| {
                        self.ae_float(ui, node, "areaWidth", "Width", 0.05, Some((0.05, 100.0)));
                        self.ae_float(ui, node, "areaHeight", "Height", 0.05, Some((0.05, 100.0)));
                        ui.label(egui::RichText::new("Scale the light's transform to resize it too.").color(TEXT_DIM));
                    });
                }
                if lt != 3 {
                    egui::CollapsingHeader::new("Shadows").default_open(true).show(ui, |ui| {
                        self.ae_toggle(ui, node, "shadows", "Cast Shadows");
                        if lt != 4 {
                            let unit = if lt == 1 { "Light Angle" } else { "Light Radius" };
                            self.ae_float(ui, node, "shadowSoftness", unit, 0.02, Some((0.0, if lt == 1 { 20.0 } else { 10.0 })));
                            ui.label(egui::RichText::new(if self.session.scene.get_attr(node, "shadowSoftness").unwrap_or(0.0) > 0.0 { "Feathered (soft) shadow edges" } else { "Hard shadow edges" }).color(TEXT_DIM));
                        } else {
                            ui.label(egui::RichText::new("Area lights always give soft shadows.").color(TEXT_DIM));
                        }
                        self.ae_float(ui, node, "shadowSamples", "Shadow Rays", 1.0, Some((1.0, 64.0)));
                        self.ae_color_attr(ui, node, "Shadow Color", "shadowColor");
                        ui.label(egui::RichText::new("Shadows appear in Render View with the Raytrace renderer.").color(TEXT_DIM));
                        ui.separator();
                        self.ae_toggle(ui, node, "useDepthMapShadows", "Use Depth Map Shadows");
                        self.ae_float(ui, node, "dmapResolution", "Resolution", 32.0, Some((32.0, 8192.0)));
                        self.ae_float(ui, node, "dmapFilterSize", "Filter Size", 0.1, Some((0.0, 20.0)));
                        ui.label(egui::RichText::new("A smaller map and a larger filter blur the shadow edge.").color(TEXT_DIM));
                    });
                }
                if lt != 3 {
                    egui::CollapsingHeader::new("Light Effects").default_open(false).show(ui, |ui| {
                        ui.label("Light Glow (optical FX)");
                        self.ae_toggle(ui, node, "lightGlow", "Glow");
                        self.ae_float(ui, node, "glowType", "Glow Type (1 linear, 2 exp)", 1.0, Some((0.0, 2.0)));
                        self.ae_float(ui, node, "glowIntensity", "Glow Intensity", 0.05, Some((0.0, 10.0)));
                        self.ae_float(ui, node, "glowSpread", "Glow Spread", 0.05, Some((0.05, 10.0)));
                        self.ae_float(ui, node, "haloType", "Halo Type (0 none, 1 lin, 2 exp)", 1.0, Some((0.0, 2.0)));
                        self.ae_float(ui, node, "haloIntensity", "Halo Intensity", 0.05, Some((0.0, 10.0)));
                        self.ae_toggle(ui, node, "lensFlare", "Lens Flare");
                        self.ae_float(ui, node, "flareIntensity", "Flare Intensity", 0.05, Some((0.0, 10.0)));
                        ui.separator();
                        ui.label("Light Fog");
                        self.ae_float(ui, node, "fogType", "Fog Type (0 none, 1 normal, 2 linear)", 1.0, Some((0.0, 2.0)));
                        self.ae_float(ui, node, "fogRadius", "Fog Radius", 0.05, Some((0.05, 50.0)));
                        self.ae_float(ui, node, "fogIntensity", "Fog Intensity", 0.05, Some((0.0, 10.0)));
                        ui.label(egui::RichText::new("The fog fills a sphere (a cone for spots) around the light; scale the light's transform to grow it.").color(TEXT_DIM));
                    });
                }
            }
            K::Camera(stereo) => {
                egui::CollapsingHeader::new("Camera").default_open(true).show(ui, |ui| {
                    self.ae_float(ui, node, "fieldOfView", "Field of View", 0.5, Some((5.0, 120.0)));
                    self.ae_float(ui, node, "nearClip", "Near Clip", 0.01, None);
                    self.ae_float(ui, node, "farClip", "Far Clip", 1.0, None);
                    self.ae_choice(ui, node, "filmFit", "Fit Resolution Gate", &["Fill", "Horizontal", "Vertical", "Overscan"]);
                    self.ae_toggle(ui, node, "renderable", "Renderable");
                    if stereo {
                        self.ae_float(ui, node, "interaxialSeparation", "Interaxial", 0.02, None);
                        self.ae_float(ui, node, "zeroParallax", "Zero Parallax", 0.1, None);
                    }
                });
            }
            K::Texture => {
                egui::CollapsingHeader::new("2D Texture Placement").default_open(true).show(ui, |ui| {
                    self.ae_float(ui, node, "repeatU", "Repeat U", 0.05, None);
                    self.ae_float(ui, node, "repeatV", "Repeat V", 0.05, None);
                    self.ae_float(ui, node, "offsetU", "Offset U", 0.01, None);
                    self.ae_float(ui, node, "offsetV", "Offset V", 0.01, None);
                    self.ae_float(ui, node, "rotateUV", "Rotate UV", 0.5, Some((-360.0, 360.0)));
                });
                egui::CollapsingHeader::new("Color Balance").default_open(true).show(ui, |ui| {
                    self.ae_color_attr(ui, node, "Color Gain", "colorGain");
                    self.ae_color_attr(ui, node, "Color Offset", "colorOffset");
                    self.ae_color_attr(ui, node, "Default Color", "defaultColor");
                    self.ae_toggle(ui, node, "wrapU", "Wrap U");
                    self.ae_toggle(ui, node, "wrapV", "Wrap V");
                });
                let kind = match self.session.scene.node(node).map(|n| &n.data) {
                    Some(NodeData::Texture(td)) => td.kind.clone(),
                    _ => String::new(),
                };
                match kind.as_str() {
                    "ramp" => {
                        egui::CollapsingHeader::new("Ramp Attributes").default_open(true).show(ui, |ui| {
                            for (attr, label, names) in [("rampType", "Type", &["V Ramp", "U Ramp", "Diagonal", "Radial", "Circular", "Box"][..]), ("interpolation", "Interpolation", &["None", "Linear", "Exponential Up", "Exponential Down", "Smooth"][..])] {
                                let mut cur = self.session.scene.get_attr(node, attr).unwrap_or(0.0).round() as usize;
                                let before = cur;
                                egui::ComboBox::from_label(label).selected_text(names[cur.min(names.len() - 1)]).show_ui(ui, |ui| {
                                    for (i, n) in names.iter().enumerate() {
                                        ui.selectable_value(&mut cur, i, *n);
                                    }
                                });
                                if cur != before {
                                    let nm = self.session.scene.name_of(node);
                                    self.exec(&format!("setAttr \"{}.{}\" {}", nm, attr, cur));
                                }
                            }
                            let count = self.session.scene.get_attr(node, "entryCount").unwrap_or(3.0).round() as usize;
                            self.ae_float(ui, node, "entryCount", "Entries", 0.1, Some((2.0, 5.0)));
                            for i in 0..count.clamp(2, 5) {
                                self.ae_float(ui, node, &format!("ePos{}", i), &format!("Position {}", i + 1), 0.01, Some((0.0, 1.0)));
                                self.ae_color_attr(ui, node, &format!("Color {}", i + 1), &format!("eCol{}", i));
                            }
                        });
                    }
                    "noise" | "fractal" | "brownian" => {
                        egui::CollapsingHeader::new("Noise Attributes").default_open(true).show(ui, |ui| {
                            for (a, l) in [("amplitude", "Amplitude"), ("ratio", "Ratio"), ("frequencyRatio", "Frequency Ratio"), ("octaves", "Octaves"), ("threshold", "Threshold"), ("frequency", "Frequency"), ("bias", "Bias")] {
                                if self.session.scene.node(node).and_then(|n| n.plug(a)).is_some() {
                                    self.ae_float(ui, node, a, l, 0.02, None);
                                }
                            }
                        });
                    }
                    "cloth" => {
                        egui::CollapsingHeader::new("Cloth Attributes").default_open(true).show(ui, |ui| {
                            self.ae_color_attr(ui, node, "U Color", "uColor");
                            self.ae_color_attr(ui, node, "V Color", "vColor");
                            for (a, l) in [("uWidth", "U Width"), ("vWidth", "V Width"), ("uWave", "U Wave"), ("vWave", "V Wave"), ("brightSpread", "Bright Spread"), ("randomness", "Randomness")] {
                                self.ae_float(ui, node, a, l, 0.01, None);
                            }
                        });
                    }
                    _ => {}
                }
            }
            K::Material => {
                egui::CollapsingHeader::new("Common Material Attributes").default_open(true).show(ui, |ui| {
                    let mut model = self.session.scene.get_attr(node, "shaderModel").unwrap_or(1.0).round() as usize;
                    let before = model;
                    egui::ComboBox::from_label("Shading model").selected_text(crate::dag::nodes::SHADER_MODELS[model.min(3)]).show_ui(ui, |ui| {
                        for (i, n) in crate::dag::nodes::SHADER_MODELS.iter().enumerate() {
                            ui.selectable_value(&mut model, i, *n);
                        }
                    });
                    if model != before {
                        let nm = self.session.scene.name_of(node);
                        self.exec(&format!("setAttr \"{}.shaderModel\" {}", nm, model));
                    }
                    self.ae_color(ui, node, "Color");
                    self.ae_color_attr(ui, node, "Incandescence", "incandescence");
                    self.ae_float(ui, node, "transparency", "Transparency", 0.01, Some((0.0, 1.0)));
                    self.ae_float(ui, node, "ambient", "Ambient", 0.01, Some((0.0, 1.0)));
                    self.ae_float(ui, node, "diffuse", "Diffuse", 0.01, Some((0.0, 1.0)));
                    self.ae_float(ui, node, "specular", "Specular", 0.01, Some((0.0, 1.0)));
                    self.ae_float(ui, node, "shininess", "Shininess", 0.5, Some((1.0, 120.0)));
                    self.ae_color_attr(ui, node, "Specular Color", "specularColor");
                    self.ae_float(ui, node, "reflectivity", "Reflectivity", 0.01, Some((0.0, 1.0)));
                });
                egui::CollapsingHeader::new("Color Map").default_open(true).show(ui, |ui| {
                    let path = self.session.scene.color_map(node);
                    ui.label(path.clone().unwrap_or_else(|| "none".to_string()));
                    ui.horizontal(|ui| {
                        if ui.button("Browse...").clicked() {
                            self.open_texture_dialog(node);
                        }
                        if ui.button("Checker").clicked() {
                            let n = self.session.scene.name_of(node);
                            self.exec(&format!("assignTexture \"{}\" \"builtin:checker\"", n));
                        }
                        if path.is_some() && ui.button("Clear").clicked() {
                            let n = self.session.scene.name_of(node);
                            self.exec(&format!("assignTexture -clear \"{}\"", n));
                        }
                    });
                    ui.label(egui::RichText::new("Tip: drop a PNG or JPEG on the window to apply it to the selection.").small().color(TEXT_DIM));
                });
            }
            K::Poly | K::Other | K::Transform => {
                let plugs: Vec<String> = self
                    .session
                    .scene
                    .node(node)
                    .map(|n| n.plugs.iter().map(|p| p.name.clone()).filter(|n| !n.starts_with("pnts_")).collect())
                    .unwrap_or_default();
                if matches!(kind, K::Poly) || matches!(kind, K::Other) {
                    egui::CollapsingHeader::new("Attributes").default_open(true).show(ui, |ui| {
                        for a in &plugs {
                            self.ae_float(ui, node, a, &nice_name(a), 0.05, None);
                        }
                    });
                }
            }
        }
        // custom attributes
        let custom: Vec<String> = self
            .session
            .scene
            .node(node)
            .map(|n| n.plugs.iter().filter(|p| p.custom && !p.name.starts_with("pnts_") && !p.name.starts_with('t')).map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        if !custom.is_empty() {
            egui::CollapsingHeader::new("Extra Attributes").default_open(true).show(ui, |ui| {
                for a in &custom {
                    self.ae_float(ui, node, a, a, 0.05, None);
                }
            });
        }
        egui::CollapsingHeader::new("Notes").default_open(false).show(ui, |ui| {
            let mut text = self.session.scene.notes.get(&node).cloned().unwrap_or_default();
            if ui.add(egui::TextEdit::multiline(&mut text).desired_rows(4).desired_width(f32::INFINITY)).changed() {
                if text.is_empty() {
                    self.session.scene.notes.remove(&node);
                } else {
                    self.session.scene.notes.insert(node, text);
                }
                self.scene_edits = true;
            }
        });
    }

    fn uv_editor(&mut self, ctx: &egui::Context) {
        if !self.show_uv {
            return;
        }
        if let Some(sh) = cmd_model::current_shape(&self.session) {
            if self.uv_set_window(ctx, sh) {
                return;
            }
        }
        let mut open = true;
        let shape = cmd_model::current_shape(&self.session);
        // zones of the shape: (zone, material, projection, face count, color map)
        let mut rows: Vec<(u16, Option<NodeId>, crate::geo::edit::UvMap, usize, Option<String>)> = Vec::new();
        // layout: (zone, selected, corner uvs) per face
        let mut layout: Vec<(u16, bool, Vec<(f32, f32)>)> = Vec::new();
        if let Some(sh) = shape {
            if let Ok(fz) = self.session.scene.face_zones(sh) {
                let nz = match self.session.scene.node(sh).map(|n| &n.data) {
                    Some(NodeData::Mesh(md)) => md.zones.len(),
                    _ => 0,
                };
                for z in 0..=nz {
                    if let Ok((m, uv)) = self.session.scene.zone_info(sh, z as u16) {
                        let tex = m.and_then(|m| self.session.scene.color_map(m));
                        rows.push((z as u16, m, uv, fz.iter().filter(|x| **x as usize == z).count(), tex));
                    }
                }
                let maps: Vec<Option<crate::geo::edit::UvMap>> = rows.iter().map(|r| Some(r.2)).collect();
                let selected: std::collections::BTreeSet<u32> = if self.session.comp.mode == SelMode::Face && self.session.comp.node == Some(sh) { self.session.comp.sel.faces.clone() } else { Default::default() };
                let angle = self.session.scene.get_attr(sh, "smoothAngle").unwrap_or(40.0);
                if let Some(NodeData::Mesh(md)) = self.session.scene.node(sh).map(|n| &n.data) {
                    let m = crate::geo::edit::shading_split_zoned(&md.mesh, angle, &fz, &maps);
                    for (fi, f) in m.faces.iter().enumerate() {
                        let poly: Vec<(f32, f32)> = f.iter().map(|&v| m.uvs.get(v as usize).map(|u| (u.x, u.y)).unwrap_or((0.0, 0.0))).collect();
                        layout.push((fz.get(fi).copied().unwrap_or(0), selected.contains(&(fi as u32)), poly));
                    }
                }
            }
        }
        if rows.is_empty() {
            self.uv_zone = 0;
        } else if self.uv_zone as usize >= rows.len() {
            self.uv_zone = 0;
        }
        let zone = self.uv_zone;
        let cur = rows.get(zone as usize).cloned();
        let tex_path = cur.as_ref().and_then(|r| r.4.clone());
        // texture preview handle
        if let Some(p) = &tex_path {
            if self.uv_tex.as_ref().map(|(k, _)| k != p).unwrap_or(true) {
                if let Some(t) = crate::render::texture::load(&self.session.scene.texture_file(p)) {
                    let (w, h) = (t.w.min(512), t.h.min(512));
                    let mut px = Vec::with_capacity(w * h * 4);
                    for y in 0..h {
                        for x in 0..w {
                            let c = t.rgb[(y * t.h / h) * t.w + x * t.w / w];
                            for k in 0..3 {
                                px.push((c[k].powf(1.0 / 2.2).clamp(0.0, 1.0) * 255.0) as u8);
                            }
                            px.push(255);
                        }
                    }
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
                    self.uv_tex = Some((p.clone(), ctx.load_texture("uvtex", img, TextureOptions::LINEAR)));
                }
            }
        } else {
            self.uv_tex = None;
        }
        let mats: Vec<(NodeId, String)> = {
            let sc = &self.session.scene;
            sc.ids_where(|d| matches!(d, NodeData::Material)).into_iter().map(|m| (m, sc.name_of(m))).collect()
        };
        let mut new_map: Option<crate::geo::edit::UvMap> = None;
        let mut new_mat: Option<NodeId> = None;
        let mut pick_zone: Option<u16> = None;
        let mut cmd: Option<String> = None;
        let mut browse: Option<NodeId> = None;
        egui::Window::new("UV Texture Editor").open(&mut open).default_size([440.0, 640.0]).show(ctx, |ui| {
            let Some(sh) = shape else {
                ui.label(egui::RichText::new("Select a polygon object").color(TEXT_DIM));
                return;
            };
            let Some((_, zmat, uv, _, _)) = cur.clone() else { return };
            // zone chooser
            ui.horizontal(|ui| {
                ui.label("Texture zone");
                let label = |r: &(u16, Option<NodeId>, crate::geo::edit::UvMap, usize, Option<String>), sc: &Scene| format!("{}{}: {} ({} faces)", r.0, if r.0 == 0 { " base" } else { "" }, r.1.map(|m| sc.name_of(m)).unwrap_or_else(|| "default".into()), r.3);
                let sc = &self.session.scene;
                egui::ComboBox::from_id_salt("uvzone").width(230.0).selected_text(label(&rows[zone as usize], sc)).show_ui(ui, |ui| {
                    for r in &rows {
                        if r.0 == 0 || r.3 > 0 {
                            if ui.selectable_label(r.0 == zone, label(r, sc)).clicked() {
                                pick_zone = Some(r.0);
                            }
                        }
                    }
                });
            });
            let side = ui.available_width().min(400.0).max(200.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 0.0, Color32::from_rgb(0x40, 0x40, 0x40));
            // map [-0.25, 1.25] to the square so out-of-range UVs stay visible
            let (lo, hi) = (-0.25f32, 1.25f32);
            let to = |u: f32, v: f32| pos2(rect.left() + (u - lo) / (hi - lo) * rect.width(), rect.bottom() - (v - lo) / (hi - lo) * rect.height());
            let unit = Rect::from_two_pos(to(0.0, 1.0), to(1.0, 0.0));
            if let Some((_, t)) = &self.uv_tex {
                p.image(t.id(), unit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_white_alpha(200));
            } else {
                p.rect_filled(unit, 0.0, Color32::from_rgb(0x58, 0x58, 0x58));
            }
            p.rect_stroke(unit, 0.0, Stroke::new(1.5_f32, Color32::from_rgb(0xf0, 0xf0, 0xf0)), eframe::egui::StrokeKind::Outside);
            for (z, selected, poly) in &layout {
                let (col, w) = if *selected {
                    (Color32::from_rgb(0xff, 0xf0, 0x40), 2.0)
                } else if *z == zone {
                    (Color32::from_rgb(0xff, 0xb0, 0x30), 1.0)
                } else {
                    (Color32::from_rgb(0x90, 0x90, 0x90), 1.0)
                };
                let n = poly.len();
                for i in 0..n {
                    let (a, b) = (poly[i], poly[(i + 1) % n]);
                    p.line_segment([to(a.0, a.1), to(b.0, b.1)], Stroke::new(w, col));
                }
            }
            // dragging the layout moves the zone's texture
            if resp.dragged() {
                let d = resp.drag_delta();
                let (du, dv) = (d.x / rect.width() * (hi - lo), -d.y / rect.height() * (hi - lo));
                let mut m = uv;
                m.off_u += du;
                m.off_v += dv;
                new_map = Some(m);
            }
            ui.label(egui::RichText::new("Drag in the layout to slide the texture of this zone.").small().color(TEXT_DIM));
            ui.add_space(4.0);
            // mapping controls of the zone
            let names = ["Automatic (box)", "Planar X", "Planar Y", "Planar Z", "Cylindrical", "Spherical", "Best plane (fit to faces)"];
            let mut m = uv;
            ui.horizontal(|ui| {
                ui.label("Mapping");
                egui::ComboBox::from_id_salt("uvmode").selected_text(names[(m.mode as usize).min(6)]).show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut m.mode, i as u8, *n);
                    }
                });
            });
            let mut dv = |ui: &mut Ui, label: &str, v: &mut f32, speed: f64| -> bool {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(110.0, 18.0), egui::Label::new(label));
                    ui.add(egui::DragValue::new(v).speed(speed).max_decimals(3)).changed()
                })
                .inner
            };
            let mut changed = m != uv;
            changed |= dv(ui, "Tiling U", &mut m.tile_u, 0.05);
            changed |= dv(ui, "Tiling V", &mut m.tile_v, 0.05);
            changed |= dv(ui, "Offset U", &mut m.off_u, 0.01);
            changed |= dv(ui, "Offset V", &mut m.off_v, 0.01);
            changed |= dv(ui, "Rotate", &mut m.rot_deg, 0.5);
            ui.horizontal(|ui| {
                if ui.button("Flip U").clicked() {
                    m.tile_u = -m.tile_u;
                    changed = true;
                }
                if ui.button("Flip V").clicked() {
                    m.tile_v = -m.tile_v;
                    changed = true;
                }
                if ui.button("Rotate 90").clicked() {
                    m.rot_deg += 90.0;
                    changed = true;
                }
                if ui.button("Reset").clicked() {
                    m = crate::geo::edit::UvMap { mode: m.mode, ..Default::default() };
                    changed = true;
                }
            });
            if changed && new_map.is_none() {
                new_map = Some(m);
            }
            ui.separator();
            // material of the zone
            ui.horizontal(|ui| {
                ui.label("Material");
                egui::ComboBox::from_id_salt("uvmat").selected_text(zmat.and_then(|m| mats.iter().find(|x| x.0 == m)).map(|x| x.1.clone()).unwrap_or_else(|| "default".into())).show_ui(ui, |ui| {
                    for (id, n) in &mats {
                        if ui.selectable_label(Some(*id) == zmat, n).clicked() {
                            new_mat = Some(*id);
                        }
                    }
                });
                if let Some(mt) = zmat {
                    if ui.button("Color Map...").clicked() {
                        browse = Some(mt);
                    }
                }
            });
            if tex_path.is_none() {
                ui.label(egui::RichText::new("This zone has no color map yet: press Color Map... to choose an image or a built-in pattern.").small().color(TEXT_DIM));
            }
            ui.separator();
            ui.label(egui::RichText::new("Selected faces (Face mode, F11)").strong());
            ui.horizontal_wrapped(|ui| {
                for (l, c) in [("Fit (best plane)", "polyPlanarProjection -md b"), ("Planar X", "polyPlanarProjection -md x"), ("Planar Y", "polyPlanarProjection -md y"), ("Planar Z", "polyPlanarProjection -md z"), ("Box", "polyAutoProjection"), ("Cylinder", "polyCylindricalProjection"), ("Sphere", "polySphericalProjection")] {
                    if ui.button(l).clicked() {
                        cmd = Some(c.to_string());
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Select zone faces").clicked() {
                    cmd = Some(format!("@select_zone {}", zone));
                }
                if ui.button("Clear all zones").on_hover_text("Everything uses the base material and mapping again").clicked() {
                    cmd = Some(format!("textureZones -clear {}", self.session.scene.name_of(sh)));
                }
            });
        });
        if let Some(z) = pick_zone {
            self.uv_zone = z;
        }
        if let (Some(sh), true) = (shape, new_map.is_some() || new_mat.is_some()) {
            let _ = self.session.scene.set_zone(sh, zone, new_mat, new_map);
            if new_mat.is_some() {
                self.session.view.show_textures();
            }
            self.scene_edited();
        }
        if let Some(mt) = browse {
            self.open_texture_dialog(mt);
        }
        if let Some(c) = cmd {
            if let Some(z) = c.strip_prefix("@select_zone ") {
                if let (Some(sh), Ok(z)) = (shape, z.parse::<u16>()) {
                    if let Ok(fz) = self.session.scene.face_zones(sh) {
                        self.session.comp.node = Some(sh);
                        self.session.comp.mode = SelMode::Face;
                        self.session.comp.sel = Sel { faces: fz.iter().enumerate().filter(|(_, x)| **x == z).map(|(i, _)| i as u32).collect(), ..Default::default() };
                        self.mark_dirty();
                    }
                }
            } else {
                self.exec(&c);
            }
        }
        if !open {
            self.show_uv = false;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Phase 2: dependency graph editors (Hypergraph, Hypershade, Connection Editor, Add Attribute,
// Channel Control, Relationship Editor)

/// Node in a Hypergraph: a real scene node or the animation curve behind an animated channel.
#[derive(Clone, PartialEq)]
enum GNode {
    Real(NodeId),
    Curve(NodeId, String),
}

struct GEdge {
    from: GNode,
    to: GNode,
    label: String,
    conn: Option<(NodeId, String, NodeId, String)>,
}

#[derive(Default)]
struct NodeEd {
    hypergraph: bool,
    hyper_connections: bool,
    hyper_shapes: bool,
    hyper_vertical: bool,
    hyper_sel_edge: Option<usize>,
    hypershade: bool,
    shade_tab: usize,
    conn_editor: bool,
    left: Option<NodeId>,
    right: Option<NodeId>,
    left_attr: Option<String>,
    right_attr: Option<String>,
    add_attr: bool,
    attr_name: String,
    attr_type: usize,
    attr_min: String,
    attr_max: String,
    attr_default: String,
    attr_keyable: bool,
    chan_control: bool,
    rel_editor: bool,
    rel_set: Option<NodeId>,
    rename: String,
}

impl App {
    fn node_ed_menu(&mut self, ui: &mut Ui) {
        let ed = &mut self.node_ed;
        if ui.button("Hypergraph").on_hover_text("Scene hierarchy and input/output connections").clicked() {
            ed.hypergraph = true;
            ui.close_menu();
        }
        if ui.button("Hypershade").on_hover_text("Create and connect materials and textures").clicked() {
            ed.hypershade = true;
            ui.close_menu();
        }
        if ui.button("Connection Editor").clicked() {
            ed.conn_editor = true;
            ui.close_menu();
        }
        if ui.button("Add Attribute...").clicked() {
            ed.add_attr = true;
            if ed.attr_default.is_empty() {
                ed.attr_default = "0".to_string();
                ed.attr_keyable = true;
            }
            ui.close_menu();
        }
        if ui.button("Channel Control").clicked() {
            ed.chan_control = true;
            ui.close_menu();
        }
        if ui.button("Relationship Editor (Sets)").clicked() {
            ed.rel_editor = true;
            ui.close_menu();
        }
    }

    fn hyper_graph_data(&self) -> (Vec<GNode>, Vec<GEdge>) {
        let sc = &self.session.scene;
        let mut nodes: Vec<GNode> = Vec::new();
        let mut edges: Vec<GEdge> = Vec::new();
        let push = |nodes: &mut Vec<GNode>, n: GNode| {
            if !nodes.contains(&n) {
                nodes.push(n);
            }
        };
        let show = |id: NodeId, shapes: bool| sc.node(id).map(|n| shapes || n.data.is_transform() || !matches!(n.data, NodeData::Mesh(_) | NodeData::Camera | NodeData::Light)).unwrap_or(false);
        if !self.node_ed.hyper_connections {
            for id in sc.ids_where(|_| true) {
                if !show(id, self.node_ed.hyper_shapes) {
                    continue;
                }
                let Some(n) = sc.node(id) else { continue };
                if matches!(n.data, NodeData::PolyOp(_) | NodeData::DrivenCurve(_)) {
                    continue;
                }
                push(&mut nodes, GNode::Real(id));
                if let Some(p) = n.parent {
                    edges.push(GEdge { from: GNode::Real(p), to: GNode::Real(id), label: "parent".to_string(), conn: None });
                }
            }
            edges.retain(|e| nodes.contains(&e.from));
            return (nodes, edges);
        }
        // input/output connections of the selection
        let mut focus: Vec<NodeId> = sc.selection.clone();
        for id in sc.selection.clone() {
            if let Some(n) = sc.node(id) {
                for c in &n.children {
                    if !show(*c, true) {
                        continue;
                    }
                    if self.node_ed.hyper_shapes {
                        focus.push(*c);
                    }
                }
            }
        }
        for id in focus.clone() {
            push(&mut nodes, GNode::Real(id));
        }
        for (src, dst) in sc.connections() {
            if focus.contains(&src.node) || focus.contains(&dst.node) {
                let (a, b) = (GNode::Real(src.node), GNode::Real(dst.node));
                push(&mut nodes, a.clone());
                push(&mut nodes, b.clone());
                let label = format!("{}.{} -> {}.{}", sc.name_of(src.node), src.attr, sc.name_of(dst.node), dst.attr);
                if let Some(e) = edges.iter_mut().find(|e| e.from == a && e.to == b) {
                    e.label.push_str(&format!("\n{}", label));
                    e.conn = None;
                } else {
                    edges.push(GEdge { from: a, to: b, label, conn: Some((src.node, src.attr, dst.node, dst.attr)) });
                }
            }
        }
        // history, material assignment and animation curves
        for id in focus.clone() {
            let Some(n) = sc.node(id) else { continue };
            if let NodeData::Mesh(md) = &n.data {
                for h in &md.history {
                    push(&mut nodes, GNode::Real(*h));
                    edges.push(GEdge { from: GNode::Real(*h), to: GNode::Real(id), label: "history (outMesh -> inMesh)".to_string(), conn: None });
                }
                let mut mats: Vec<NodeId> = md.zones.iter().filter_map(|z| z.material).collect();
                if let Some(m) = md.material {
                    mats.push(m);
                }
                for m in mats {
                    push(&mut nodes, GNode::Real(m));
                    edges.push(GEdge { from: GNode::Real(id), to: GNode::Real(m), label: "shading group assignment".to_string(), conn: None });
                }
            }
            for b in sc.anim.bindings.iter().filter(|b| b.node == id) {
                let c = GNode::Curve(id, b.attr.clone());
                push(&mut nodes, c.clone());
                edges.push(GEdge { from: c, to: GNode::Real(id), label: format!("output -> {}", b.attr), conn: None });
            }
        }
        (nodes, edges)
    }

    fn hypergraph_ui(&mut self, ctx: &egui::Context) {
        if !self.node_ed.hypergraph {
            return;
        }
        let (nodes, edges) = self.hyper_graph_data();
        // columns by longest path from the sources
        let mut depth = vec![0usize; nodes.len()];
        for _ in 0..nodes.len().min(12) {
            for e in &edges {
                if let (Some(a), Some(b)) = (nodes.iter().position(|n| *n == e.from), nodes.iter().position(|n| *n == e.to)) {
                    if depth[b] < depth[a] + 1 {
                        depth[b] = (depth[a] + 1).min(12);
                    }
                }
            }
        }
        let mut open = true;
        let mut clicked: Option<GNode> = None;
        let mut del_edge: Option<(NodeId, String, NodeId, String)> = None;
        let mut sel_edge = self.node_ed.hyper_sel_edge;
        egui::Window::new("Hypergraph").open(&mut open).default_size([760.0, 440.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.node_ed.hyper_connections, false, "Scene Hierarchy");
                ui.selectable_value(&mut self.node_ed.hyper_connections, true, "Input and Output Connections");
                ui.separator();
                ui.checkbox(&mut self.node_ed.hyper_shapes, "Shape nodes");
                ui.checkbox(&mut self.node_ed.hyper_vertical, "Vertical");
                if let Some(s) = sel_edge.filter(|s| *s < edges.len()) {
                    if edges[s].conn.is_some() && ui.button("Delete Connection").clicked() {
                        del_edge = edges[s].conn.clone();
                    }
                }
            });
            let (resp, painter) = ui.allocate_painter(ui.available_size().max(egui::vec2(400.0, 300.0)), egui::Sense::click());
            let r = resp.rect;
            let mut per_col = vec![0usize; 14];
            let mut pos: Vec<egui::Pos2> = Vec::new();
            let (bw, bh) = (110.0, 22.0);
            for d in depth.iter() {
                let k = per_col[*d];
                per_col[*d] += 1;
                let (a, b) = (*d as f32 * (bw + 50.0) + 14.0, k as f32 * (bh + 14.0) + 14.0);
                pos.push(if self.node_ed.hyper_vertical { r.min + egui::vec2(b, a) } else { r.min + egui::vec2(a, b) });
            }
            let vis = ui.visuals().clone();
            let mut hit_edge: Option<usize> = None;
            let pointer = resp.hover_pos();
            for (i, e) in edges.iter().enumerate() {
                let (Some(a), Some(b)) = (nodes.iter().position(|n| *n == e.from), nodes.iter().position(|n| *n == e.to)) else { continue };
                let size = egui::vec2(bw, bh);
                let (p0, p1) = if self.node_ed.hyper_vertical { (pos[a] + egui::vec2(bw / 2.0, bh), pos[b] + egui::vec2(bw / 2.0, 0.0)) } else { (pos[a] + egui::vec2(size.x, bh / 2.0), pos[b] + egui::vec2(0.0, bh / 2.0)) };
                let on = Some(i) == sel_edge;
                let near = pointer.map(|p| {
                    let (v, w) = (p1 - p0, p - p0);
                    let t = (w.dot(v) / v.dot(v).max(1.0)).clamp(0.0, 1.0);
                    (p - (p0 + v * t)).length() < 5.0
                }).unwrap_or(false);
                let col = if on { egui::Color32::YELLOW } else if near { egui::Color32::WHITE } else { vis.weak_text_color() };
                painter.arrow(p0, p1 - p0, egui::Stroke::new(if on || near { 2.0 } else { 1.0 }, col));
                if near {
                    hit_edge = Some(i);
                    egui::show_tooltip_at_pointer(ctx, egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("hg_tip")), egui::Id::new("hg_tip"), |ui| {
                        ui.label(&e.label);
                    });
                }
            }
            let mut node_hit = false;
            for (i, n) in nodes.iter().enumerate() {
                let rect = egui::Rect::from_min_size(pos[i], egui::vec2(bw, bh));
                let (name, color, selected) = match n {
                    GNode::Real(id) => {
                        let (_, c) = node_icon(self.session.scene.node(*id).map(|x| &x.data).unwrap_or(&NodeData::Locator));
                        (self.session.scene.name_of(*id), c, self.session.scene.selection.contains(id))
                    }
                    GNode::Curve(id, a) => (format!("{}_{}", self.session.scene.name_of(*id), a), egui::Color32::from_rgb(0x60, 0xc0, 0x60), false),
                };
                painter.rect_filled(rect, 3.0, if selected { egui::Color32::from_rgb(0x50, 0x70, 0x40) } else { vis.extreme_bg_color });
                painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.5, color), egui::StrokeKind::Inside);
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(11.0), vis.text_color());
                if resp.clicked() && pointer.map(|p| rect.contains(p)).unwrap_or(false) {
                    clicked = Some(n.clone());
                    node_hit = true;
                }
            }
            if resp.clicked() && !node_hit {
                sel_edge = hit_edge;
            }
            if sel_edge.is_some() && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
                if let Some(c) = sel_edge.and_then(|s| edges.get(s)).and_then(|e| e.conn.clone()) {
                    del_edge = Some(c);
                }
            }
        });
        self.node_ed.hyper_sel_edge = sel_edge;
        if let Some(GNode::Real(id)) = clicked {
            self.session.scene.selection = vec![id];
        }
        if let Some((a, aa, b, ba)) = del_edge {
            let cmd = format!("disconnectAttr \"{}.{}\" \"{}.{}\"", self.session.scene.name_of(a), aa, self.session.scene.name_of(b), ba);
            self.exec(&cmd);
            self.node_ed.hyper_sel_edge = None;
            self.mark_dirty();
        }
        self.node_ed.hypergraph = open;
    }

    fn hypershade_ui(&mut self, ctx: &egui::Context) {
        if !self.node_ed.hypershade {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        egui::Window::new("Hypershade").open(&mut open).default_size([640.0, 420.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, t) in ["Materials", "Textures", "All"].iter().enumerate() {
                    ui.selectable_value(&mut self.node_ed.shade_tab, i, *t);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Create:");
                for (t, c) in [("Lambert", "shadingNode -asShader lambert"), ("Blinn", "shadingNode -asShader blinn"), ("Phong", "shadingNode -asShader phong"), ("Surface Shader", "shadingNode -asShader surfaceShader"), ("File", "shadingNode -asTexture file"), ("Checker", "shadingNode -asTexture checker"), ("Grid", "shadingNode -asTexture grid"), ("Bricks", "shadingNode -asTexture bricks"), ("Cloth", "shadingNode -asTexture cloth"), ("Dots", "shadingNode -asTexture dots"), ("Noise", "shadingNode -asTexture noise"), ("Fractal", "shadingNode -asTexture fractal"), ("Cloth", "shadingNode -asTexture cloth"), ("Brownian", "shadingNode -asTexture brownian"), ("Ramp", "shadingNode -asTexture ramp")] {
                    if ui.button(t).clicked() {
                        cmds.push(c.to_string());
                    }
                }
            });
            ui.separator();
            let sc = &self.session.scene;
            let list = sc.ids_where(|d| match self.node_ed.shade_tab {
                0 => matches!(d, NodeData::Material),
                1 => matches!(d, NodeData::Texture(_)),
                _ => matches!(d, NodeData::Material | NodeData::Texture(_)),
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for id in list {
                    let name = sc.name_of(id);
                    ui.horizontal(|ui| {
                        let sel = sc.selection.contains(&id);
                        if ui.selectable_label(sel, &name).clicked() {
                            cmds.push(format!("select {}", name));
                        }
                        if matches!(sc.node(id).map(|n| &n.data), Some(NodeData::Material)) {
                            if ui.small_button("Assign to selection").clicked() {
                                cmds.push(format!("hyperShade -assign {}", name));
                            }
                            if ui.small_button("Select objects").clicked() {
                                cmds.push(format!("hyperShade -objects {}", name));
                            }
                            if ui.small_button("Graph").clicked() {
                                cmds.push(format!("select {}", name));
                            }
                        }
                        if let Some(NodeData::Texture(t)) = sc.node(id).map(|n| &n.data) {
                            ui.weak(if t.kind == "file" { t.path.clone() } else { format!("{} pattern", t.kind) });
                        }
                    });
                }
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui.button("Delete Unused Nodes").clicked() {
                    cmds.push("hyperShade -deleteUnusedNodes".to_string());
                }
                if ui.button("Duplicate Shading Network").on_hover_text("Copy the selected material and its textures").clicked() {
                    cmds.push("duplicateShadingNetwork".to_string());
                }
                if ui.button("Convert to File Texture").on_hover_text("Bake the selected material's color network into sourceimages/").clicked() {
                    cmds.push("convertToFileTexture -res 512".to_string());
                }
                if ui.button("Place Texture...").clicked() {
                    cmds.push("@place".to_string());
                }
            });
        });
        for c in cmds {
            if c == "@place" {
                self.nurbs_ui.show_place = true;
                continue;
            }
            self.exec(&c);
            if c.starts_with("select") {
                self.node_ed.hypergraph = true;
                self.node_ed.hyper_connections = true;
            }
        }
        self.node_ed.hypershade = open;
    }

    fn connection_editor_ui(&mut self, ctx: &egui::Context) {
        if !self.node_ed.conn_editor {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let sel = self.session.scene.selection.first().copied();
        egui::Window::new("Connection Editor").open(&mut open).default_size([520.0, 380.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Reload Left").clicked() {
                    self.node_ed.left = sel;
                    self.node_ed.left_attr = None;
                }
                if ui.button("Reload Right").clicked() {
                    self.node_ed.right = sel;
                    self.node_ed.right_attr = None;
                }
                if ui.button("Clear All").clicked() {
                    self.node_ed.left = None;
                    self.node_ed.right = None;
                    self.node_ed.left_attr = None;
                    self.node_ed.right_attr = None;
                }
            });
            ui.label("Select an Outputs attribute on the left, then an Inputs attribute on the right to connect them.");
            ui.separator();
            let sc = &self.session.scene;
            ui.columns(2, |cols| {
                for (side, col) in cols.iter_mut().enumerate() {
                    let node = if side == 0 { self.node_ed.left } else { self.node_ed.right };
                    col.strong(if side == 0 { "Outputs" } else { "Inputs" });
                    let Some(id) = node.filter(|i| sc.node(*i).is_some()) else {
                        col.weak("(empty)");
                        continue;
                    };
                    col.label(sc.name_of(id));
                    egui::ScrollArea::vertical().id_source(side).max_height(240.0).show(col, |ui| {
                        for p in &sc.node(id).unwrap().plugs {
                            if side == 1 && (p.readonly || p.locked) {
                                continue;
                            }
                            let cur = if side == 0 { &self.node_ed.left_attr } else { &self.node_ed.right_attr };
                            let mut label = p.name.clone();
                            if side == 1 && p.source.is_some() {
                                label.push_str("  (connected)");
                            }
                            if ui.selectable_label(cur.as_deref() == Some(p.name.as_str()), label).clicked() {
                                if side == 0 {
                                    self.node_ed.left_attr = Some(p.name.clone());
                                } else {
                                    self.node_ed.right_attr = Some(p.name.clone());
                                }
                            }
                        }
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                let ready = self.node_ed.left_attr.is_some() && self.node_ed.right_attr.is_some();
                if ui.add_enabled(ready, egui::Button::new("Connect")).clicked() {
                    if let (Some(l), Some(r), Some(la), Some(ra)) = (self.node_ed.left, self.node_ed.right, self.node_ed.left_attr.clone(), self.node_ed.right_attr.clone()) {
                        cmds.push(format!("connectAttr -force \"{}.{}\" \"{}.{}\"", self.session.scene.name_of(l), la, self.session.scene.name_of(r), ra));
                    }
                }
                if ui.add_enabled(ready, egui::Button::new("Disconnect")).clicked() {
                    if let (Some(l), Some(r), Some(la), Some(ra)) = (self.node_ed.left, self.node_ed.right, self.node_ed.left_attr.clone(), self.node_ed.right_attr.clone()) {
                        cmds.push(format!("disconnectAttr \"{}.{}\" \"{}.{}\"", self.session.scene.name_of(l), la, self.session.scene.name_of(r), ra));
                    }
                }
            });
        });
        for c in cmds {
            self.exec(&c);
            self.mark_dirty();
        }
        self.node_ed.conn_editor = open;
    }

    fn add_attribute_ui(&mut self, ctx: &egui::Context) {
        if !self.node_ed.add_attr {
            return;
        }
        let mut open = true;
        let mut cmd: Option<String> = None;
        let target = self.session.scene.selection.first().map(|i| self.session.scene.name_of(*i));
        egui::Window::new("Add Attribute").open(&mut open).default_size([300.0, 230.0]).show(ctx, |ui| {
            ui.label(format!("Node: {}", target.clone().unwrap_or_else(|| "(select an object)".to_string())));
            ui.horizontal(|ui| {
                ui.label("Attribute name");
                ui.text_edit_singleline(&mut self.node_ed.attr_name);
            });
            ui.horizontal(|ui| {
                ui.label("Data type");
                ui.selectable_value(&mut self.node_ed.attr_type, 0, "Float");
                ui.selectable_value(&mut self.node_ed.attr_type, 1, "Integer");
                ui.selectable_value(&mut self.node_ed.attr_type, 2, "Boolean");
            });
            ui.horizontal(|ui| {
                ui.label("Min");
                ui.add(egui::TextEdit::singleline(&mut self.node_ed.attr_min).desired_width(50.0));
                ui.label("Max");
                ui.add(egui::TextEdit::singleline(&mut self.node_ed.attr_max).desired_width(50.0));
                ui.label("Default");
                ui.add(egui::TextEdit::singleline(&mut self.node_ed.attr_default).desired_width(50.0));
            });
            ui.checkbox(&mut self.node_ed.attr_keyable, "Make attribute keyable");
            if ui.add_enabled(target.is_some() && !self.node_ed.attr_name.is_empty(), egui::Button::new("Add")).clicked() {
                let ed = &self.node_ed;
                let mut c = format!("addAttr -ln {} -at {} -dv {} -k {}", ed.attr_name, ["float", "long", "bool"][ed.attr_type.min(2)], if ed.attr_default.is_empty() { "0" } else { &ed.attr_default }, ed.attr_keyable as u8);
                if !ed.attr_min.is_empty() {
                    c.push_str(&format!(" -min {}", ed.attr_min));
                }
                if !ed.attr_max.is_empty() {
                    c.push_str(&format!(" -max {}", ed.attr_max));
                }
                cmd = Some(format!("{} {}", c, target.unwrap()));
            }
        });
        if let Some(c) = cmd {
            self.exec(&c);
            self.mark_dirty();
        }
        self.node_ed.add_attr = open;
    }

    fn channel_control_ui(&mut self, ctx: &egui::Context) {
        if !self.node_ed.chan_control {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let node = self.session.scene.selection.first().copied();
        egui::Window::new("Channel Control").open(&mut open).default_size([460.0, 360.0]).show(ctx, |ui| {
            let Some(id) = node.filter(|i| self.session.scene.node(*i).is_some()) else {
                ui.label("Select an object.");
                return;
            };
            let name = self.session.scene.name_of(id);
            ui.label(format!("Node: {}   (Keyable attributes appear in the Channel Box)", name));
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("chan_ctl").striped(true).show(ui, |ui| {
                    ui.strong("Attribute");
                    ui.strong("Keyable");
                    ui.strong("Locked");
                    ui.end_row();
                    for p in &self.session.scene.node(id).unwrap().plugs {
                        if p.readonly {
                            continue;
                        }
                        ui.label(&p.name);
                        let mut k = p.keyable;
                        if ui.checkbox(&mut k, "").changed() {
                            cmds.push(format!("setAttr -keyable {} \"{}.{}\"", k as u8, name, p.name));
                        }
                        let mut l = p.locked;
                        if ui.checkbox(&mut l, "").changed() {
                            cmds.push(format!("setAttr -lock {} \"{}.{}\"", l as u8, name, p.name));
                        }
                        ui.end_row();
                    }
                });
            });
        });
        for c in cmds {
            self.exec(&c);
            self.mark_dirty();
        }
        self.node_ed.chan_control = open;
    }

    fn relationship_editor_ui(&mut self, ctx: &egui::Context) {
        if !self.node_ed.rel_editor {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        egui::Window::new("Relationship Editor - Sets").open(&mut open).default_size([460.0, 340.0]).show(ctx, |ui| {
            let sc = &self.session.scene;
            let sets = sc.ids_where(|d| matches!(d, NodeData::ObjectSet(_)));
            ui.horizontal(|ui| {
                ui.label("New set name");
                ui.text_edit_singleline(&mut self.node_ed.rename);
                if ui.button("Create from selection").clicked() {
                    cmds.push(if self.node_ed.rename.is_empty() { "sets".to_string() } else { format!("sets -name {}", self.node_ed.rename) });
                }
            });
            ui.separator();
            ui.columns(2, |cols| {
                cols[0].strong("Sets");
                for s in &sets {
                    if cols[0].selectable_label(self.node_ed.rel_set == Some(*s), sc.name_of(*s)).clicked() {
                        self.node_ed.rel_set = Some(*s);
                    }
                }
                cols[1].strong("Members");
                if let Some(s) = self.node_ed.rel_set.filter(|s| sc.node(*s).is_some()) {
                    let sname = sc.name_of(s);
                    for m in sc.set_members(s) {
                        cols[1].label(sc.name_of(m));
                    }
                    cols[1].horizontal(|ui| {
                        if ui.button("Add selection").clicked() {
                            cmds.push(format!("sets -e -add {} {}", sname, sc.selection.iter().map(|i| sc.name_of(*i)).collect::<Vec<_>>().join(" ")));
                        }
                        if ui.button("Remove selection").clicked() {
                            cmds.push(format!("sets -e -rm {} {}", sname, sc.selection.iter().map(|i| sc.name_of(*i)).collect::<Vec<_>>().join(" ")));
                        }
                        if ui.button("Select members").clicked() {
                            cmds.push(format!("select {}", sname));
                        }
                    });
                }
            });
        });
        for c in cmds {
            self.exec(&c);
            self.mark_dirty();
        }
        self.node_ed.rel_editor = open;
    }

    fn node_editors_ui(&mut self, ctx: &egui::Context) {
        self.hypergraph_ui(ctx);
        self.hypershade_ui(ctx);
        self.connection_editor_ui(ctx);
        self.add_attribute_ui(ctx);
        self.channel_control_ui(ctx);
        self.relationship_editor_ui(ctx);
    }
}

// ---------------------------------------------------------------------------------------------
// Phase 4: UV Texture Editor for editable UV sets, Create UVs, 3D Paint Tool

#[derive(Default)]
struct UvUi {
    /// Drag in the editor: marquee start or moving the selection (last position).
    marquee: Option<Pos2>,
    moving: bool,
    show_image: bool,
    paint_res: usize,
    show_paint: bool,
    last_uv: Option<crate::math::Vec2>,
}

impl App {
    fn create_uvs_menu(&mut self, ui: &mut Ui) {
        for (l, c) in [
            ("Planar Mapping X", "polyProjection -type planar -md x"),
            ("Planar Mapping Y", "polyProjection -type planar -md y"),
            ("Planar Mapping Z", "polyProjection -type planar -md z"),
            ("Planar Mapping, best plane", "polyProjection -type planar -md b"),
            ("Cylindrical Mapping", "polyProjection -type cylindrical"),
            ("Spherical Mapping", "polyProjection -type spherical"),
            ("Automatic Mapping", "polyAutoProjection"),
        ] {
            if ui.button(l).on_hover_text("Projects the selected faces, or the whole object when no faces are selected").clicked() {
                self.exec(c);
                self.show_uv = true;
                ui.close_menu();
            }
        }
        ui.separator();
        for (l, c) in [("Cut UV Edges", "polyMapCut"), ("Sew UV Edges", "polyMapSew"), ("Layout UVs", "polyLayoutUV -sf"), ("Normalize UVs", "polyNormalizeUV"), ("Unfold (relax)", "unfold -i 20"), ("Delete UVs", "polyMapDel")] {
            if ui.button(l).clicked() {
                self.exec(c);
                ui.close_menu();
            }
        }
    }

    /// The UV Texture Editor for a shape that has its own UV set. Returns false when the shape has none.
    fn uv_set_window(&mut self, ctx: &egui::Context, shape: NodeId) -> bool {
        struct View {
            faces: Vec<Vec<u32>>,
            uvs: Vec<(f32, f32)>,
            shells: usize,
        }
        let view = match self.session.scene.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.uv_set.as_ref().map(|u| View { faces: u.fuv.clone(), uvs: u.uvs.iter().map(|p| (p.x, p.y)).collect(), shells: u.shells().len() }),
            _ => None,
        };
        let Some(view) = view else { return false };
        // the picture shown behind the UVs: the color map of the object's material
        let tex_path = match self.session.scene.node(shape).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.material.and_then(|m| self.session.scene.color_map(m)),
            _ => None,
        };
        if let (Some(p), true) = (&tex_path, self.uv_ui.show_image) {
            let file = self.session.scene.texture_file(p);
            let stamp = format!("{}:{}", file, self.session.paint.dirty as u8 + self.frame_gen as u8);
            if self.uv_tex.as_ref().map(|(k, _)| *k != stamp).unwrap_or(true) {
                if let Some(t) = crate::render::texture::load(&file) {
                    let (w, h) = (t.w.min(512), t.h.min(512));
                    let mut px = Vec::with_capacity(w * h * 4);
                    for y in 0..h {
                        for x in 0..w {
                            let c = t.rgb[(y * t.h / h) * t.w + x * t.w / w];
                            for k in 0..3 {
                                px.push((c[k].powf(1.0 / 2.2).clamp(0.0, 1.0) * 255.0) as u8);
                            }
                            px.push(255);
                        }
                    }
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
                    self.uv_tex = Some((stamp, ctx.load_texture("uvtex", img, TextureOptions::LINEAR)));
                }
            }
        }
        let sel = self.session.comp.uv_sel.clone();
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let mut move_by: Option<(f32, f32)> = None;
        let mut box_sel: Option<(f32, f32, f32, f32, bool)> = None;
        let mut click_sel: Option<(u32, bool)> = None;
        egui::Window::new("UV Texture Editor").open(&mut open).default_size([460.0, 640.0]).show(ctx, |ui| {
            ui.label(format!("{} UVs, {} shell(s), {} selected", view.uvs.len(), view.shells, sel.len()));
            ui.horizontal_wrapped(|ui| {
                for (l, c) in [("Select All", "uvSelect -all"), ("Select Shell", "uvSelect -shell"), ("From Faces", "uvSelect -faces"), ("Invert", "uvSelect -invert"), ("Clear", "uvSelect -clear")] {
                    if ui.button(l).clicked() {
                        cmds.push(c.to_string());
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (l, c) in [("Flip U", "polyEditUV -flipU"), ("Flip V", "polyEditUV -flipV"), ("Rotate 90", "polyEditUV -a 90"), ("Scale x2", "polyEditUV -su 2 -sv 2"), ("Scale /2", "polyEditUV -su 0.5 -sv 0.5"), ("Normalize", "polyNormalizeUV"), ("Layout", "polyLayoutUV -sf"), ("Unfold", "unfold -i 20"), ("Cut", "polyMapCut"), ("Sew", "polyMapSew")] {
                    if ui.button(l).clicked() {
                        cmds.push(c.to_string());
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.uv_ui.show_image, "Display image");
                if ui.button("Checker").on_hover_text("Map the checker pattern to see stretching").clicked() {
                    cmds.push("assignTexture builtin:checker".to_string());
                }
                if ui.button("Delete UVs").clicked() {
                    cmds.push("polyMapDel".to_string());
                }
            });
            let side = ui.available_width().min(440.0).max(220.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 0.0, Color32::from_rgb(0x40, 0x40, 0x40));
            let (lo, hi) = (-0.25f32, 1.25f32);
            let to = |u: f32, v: f32| pos2(rect.left() + (u - lo) / (hi - lo) * rect.width(), rect.bottom() - (v - lo) / (hi - lo) * rect.height());
            let from = |q: Pos2| ((q.x - rect.left()) / rect.width() * (hi - lo) + lo, (rect.bottom() - q.y) / rect.height() * (hi - lo) + lo);
            let unit = Rect::from_two_pos(to(0.0, 1.0), to(1.0, 0.0));
            match (&self.uv_tex, self.uv_ui.show_image) {
                (Some((_, t)), true) => {
                    p.image(t.id(), unit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_white_alpha(210));
                }
                _ => {
                    p.rect_filled(unit, 0.0, Color32::from_rgb(0x58, 0x58, 0x58));
                }
            }
            p.rect_stroke(unit, 0.0, Stroke::new(1.5_f32, Color32::from_rgb(0xf0, 0xf0, 0xf0)), eframe::egui::StrokeKind::Outside);
            for f in &view.faces {
                let n = f.len();
                let all_sel = f.iter().all(|u| sel.contains(u));
                for i in 0..n {
                    let (a, b) = (view.uvs[f[i] as usize], view.uvs[f[(i + 1) % n] as usize]);
                    let col = if all_sel { Color32::from_rgb(0xff, 0xf0, 0x40) } else { Color32::from_rgb(0x70, 0xc0, 0xff) };
                    p.line_segment([to(a.0, a.1), to(b.0, b.1)], Stroke::new(1.0, col));
                }
            }
            for &u in &sel {
                if let Some(q) = view.uvs.get(u as usize) {
                    p.circle_filled(to(q.0, q.1), 3.5, Color32::from_rgb(0xff, 0x80, 0x20));
                }
            }
            let near = |q: Pos2| -> Option<u32> {
                view.uvs.iter().enumerate().map(|(i, c)| (i, to(c.0, c.1).distance(q))).filter(|(_, d)| *d < 9.0).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|(i, _)| i as u32)
            };
            let shift = ui.input(|i| i.modifiers.shift);
            if resp.drag_started() {
                if let Some(q) = resp.interact_pointer_pos() {
                    self.uv_ui.moving = near(q).map(|u| sel.contains(&u)).unwrap_or(false);
                    self.uv_ui.marquee = Some(q);
                }
            }
            if resp.dragged() {
                if self.uv_ui.moving {
                    let d = resp.drag_delta();
                    move_by = Some((d.x / rect.width() * (hi - lo), -d.y / rect.height() * (hi - lo)));
                } else if let (Some(a), Some(b)) = (self.uv_ui.marquee, resp.interact_pointer_pos()) {
                    p.rect_stroke(Rect::from_two_pos(a, b), 0.0, Stroke::new(1.0_f32, Color32::WHITE), eframe::egui::StrokeKind::Inside);
                }
            }
            if resp.drag_stopped() {
                if !self.uv_ui.moving {
                    if let (Some(a), Some(b)) = (self.uv_ui.marquee, resp.interact_pointer_pos()) {
                        let (p0, p1) = (from(a), from(b));
                        box_sel = Some((p0.0, p0.1, p1.0, p1.1, shift));
                    }
                }
                self.uv_ui.marquee = None;
                self.uv_ui.moving = false;
            }
            if resp.clicked() {
                if let Some(q) = resp.interact_pointer_pos() {
                    match near(q) {
                        Some(u) => click_sel = Some((u, shift)),
                        None => cmds.push("uvSelect -clear".to_string()),
                    }
                }
            }
            ui.label(egui::RichText::new("Click or drag a box to select UVs (Shift adds), drag a selected UV to move the selection.").small().color(TEXT_DIM));
        });
        if let Some((du, dv)) = move_by {
            if let Some(NodeData::Mesh(md)) = self.session.scene.nodes.get_mut(shape).map(|n| &mut n.data) {
                if let Some(set) = md.uv_set.as_mut() {
                    set.transform(&sel, |q| crate::math::Vec2::new(q.x + du, q.y + dv));
                }
                md.touch();
            }
            self.mark_dirty();
        }
        if let Some((a, b, c, d, add)) = box_sel {
            cmds.push(format!("uvSelect -box {} {} {} {}{}", a, b, c, d, if add { " -add" } else { "" }));
        }
        if let Some((u, add)) = click_sel {
            cmds.push(format!("uvSelect {}{}", u, if add { " -add" } else { "" }));
        }
        for c in cmds {
            self.exec(&c);
        }
        if !open {
            self.show_uv = false;
        }
        true
    }

    /// Window > 3D Paint Tool: brush settings and texture handling.
    fn paint_window(&mut self, ctx: &egui::Context) {
        if !self.uv_ui.show_paint {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        if self.uv_ui.paint_res == 0 {
            self.uv_ui.paint_res = 512;
        }
        egui::Window::new("3D Paint Tool").open(&mut open).default_size([300.0, 300.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Resolution");
                egui::ComboBox::from_id_salt("paintres").selected_text(self.uv_ui.paint_res.to_string()).show_ui(ui, |ui| {
                    for r in [128usize, 256, 512, 1024, 2048] {
                        ui.selectable_value(&mut self.uv_ui.paint_res, r, r.to_string());
                    }
                });
            });
            if ui.button("Assign/Edit Textures").on_hover_text("Gives the selected object UVs and its own texture in 3dpainttextures/").clicked() {
                cmds.push(format!("paintAssign -res {}", self.uv_ui.paint_res));
                self.start_tool(Tool::Paint);
            }
            ui.separator();
            let mut b = (self.session.paint.color, self.session.paint.radius, self.session.paint.opacity, self.session.paint.erase, self.session.paint.save_on_stroke);
            ui.horizontal(|ui| {
                ui.label("Color");
                ui.color_edit_button_rgb(&mut b.0);
            });
            ui.add(egui::Slider::new(&mut b.1, 0.002..=0.3).text("Radius"));
            ui.add(egui::Slider::new(&mut b.2, 0.0..=1.0).text("Opacity"));
            ui.checkbox(&mut b.3, "Erase (back to the assigned texture)");
            ui.checkbox(&mut b.4, "Save texture on stroke");
            let cur = (self.session.paint.color, self.session.paint.radius, self.session.paint.opacity, self.session.paint.erase, self.session.paint.save_on_stroke);
            if b != cur {
                self.session.paint.color = b.0;
                self.session.paint.radius = b.1;
                self.session.paint.opacity = b.2;
                self.session.paint.erase = b.3;
                self.session.paint.save_on_stroke = b.4;
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(self.session.paint.active(), egui::Button::new("Save Textures")).clicked() {
                    cmds.push("paintSave".to_string());
                }
                if ui.add_enabled(self.session.paint.active(), egui::Button::new("Reload File Textures")).clicked() {
                    cmds.push("paintReload".to_string());
                }
            });
            ui.label(if self.session.paint.active() { format!("Painting {}", self.session.paint.path) } else { "Select an object and press Assign/Edit Textures, then drag over it in the viewport.".to_string() });
        });
        for c in cmds {
            self.exec(&c);
        }
        if !open {
            self.uv_ui.show_paint = false;
            if self.tool == Tool::Paint {
                self.tool = Tool::Select;
            }
        }
    }

    /// One paint step under the pointer: `new_stroke` starts a stroke, otherwise it continues the last one.
    fn paint_at(&mut self, cam: &Cam, rect: Rect, pos: Pos2, new_stroke: bool) {
        let Some(shape) = cmd_model::current_shape(&self.session) else { return };
        let (nx, ny) = ((pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height());
        let inv = cam.view_proj().inverse();
        let un = |z: f32| {
            let c = inv * Vec3::new(nx * 2.0 - 1.0, 1.0 - ny * 2.0, z).extend(1.0);
            Vec3::new(c.x / c.w, c.y / c.w, c.z / c.w)
        };
        let (near, far) = (un(-1.0), un(1.0));
        let from = if new_stroke { None } else { self.uv_ui.last_uv };
        match crate::mel::cmd_paint::paint_ray(&mut self.session, shape, near, (far - near).normalized(), from) {
            Ok(uv) => self.uv_ui.last_uv = uv,
            Err(e) => {
                self.say(&e);
                self.tool = Tool::Select;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Phase 5: NURBS curves and surfaces

#[derive(Default)]
struct NurbsUi {
    ep: bool,
    degree: usize,
    pts: Vec<Vec3>,
    show_edit: bool,
    show_place: bool,
    place: [f32; 5],
    /// Isoparm parameter and direction for the edit window.
    param: f32,
    along_v: bool,
    seam: u32,
    spans: u32,
}

impl App {
    fn surface_menus(&mut self, ui: &mut Ui) {
        ui.menu_button("Edit Curves", |ui| {
            for (l, c) in [("Attach Curves", "attachCurve"), ("Curve Fillet", "filletCurve -r 1"), ("Duplicate Surface Curves...", ""), ("Open/Close Curves (close)", "closeCurve"), ("Open/Close Curves (open)", "openCurve"), ("Rebuild Curve (4 spans)", "rebuildCurve -s 4")] {
                if ui.button(l).clicked() {
                    if c.is_empty() {
                        self.nurbs_ui.show_edit = true;
                    } else {
                        self.exec(c);
                    }
                    ui.close_menu();
                }
            }
        });
        ui.menu_button("Surfaces", |ui| {
            for (l, c) in [("Revolve", "revolve"), ("Loft", "loft"), ("Extrude (select profile, then path)", "extrude")] {
                if ui.button(l).on_hover_text("Uses the selected curves; the result keeps its history").clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
        });
        ui.menu_button("Edit NURBS", |ui| {
            if ui.button("Isoparm tools...").on_hover_text("Insert isoparms, detach, duplicate surface curves, move seam").clicked() {
                self.nurbs_ui.show_edit = true;
                ui.close_menu();
            }
            if ui.button("Attach Surfaces").clicked() {
                self.exec("attachSurface");
                ui.close_menu();
            }
            for (l, c) in [("Project Curve on Surface (down)", "projectCurve"), ("Trim Tool (selected curves + surface)", "trim"), ("Untrim Surfaces", "untrim"), ("Reverse Surface Direction", "reverseSurface"), ("Planar (closed curve)", "planarSurface"), ("Rebuild Surfaces (same spans, cubic)", "rebuildSurface -du 3 -dv 3")] {
                if ui.button(l).clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
            if ui.button("Texture Placement...").on_hover_text("Interactive Placement of the selected object's texture").clicked() {
                self.nurbs_ui.show_place = true;
                ui.close_menu();
            }
            if ui.button("Convert to Polygons").clicked() {
                self.exec("nurbsToPoly");
                ui.close_menu();
            }
            if ui.button("Info").clicked() {
                self.exec("nurbsInfo");
                ui.close_menu();
            }
        });
    }

    /// Edit NURBS window: isoparm operations at a chosen parameter.
    fn nurbs_window(&mut self, ctx: &egui::Context) {
        if !self.nurbs_ui.show_edit {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        // parameter range of the selected surface
        let range = self.session.scene.selection.first().and_then(|t| self.session.scene.surface_shapes_under(*t).first().copied()).and_then(|sh| match self.session.scene.node(sh).map(|n| &n.data) {
            Some(NodeData::Mesh(md)) => md.nurbs.as_ref().map(|n| (n.surf.u.domain(), n.surf.v.domain(), n.surf.u.periodic, n.surf.v.periodic)),
            _ => None,
        });
        egui::Window::new("Edit NURBS").open(&mut open).default_size([320.0, 260.0]).show(ctx, |ui| {
            let Some((du, dv, pu, pv)) = range else {
                ui.label("Select a NURBS surface.");
                return;
            };
            ui.horizontal(|ui| {
                ui.label("Isoparm");
                ui.selectable_value(&mut self.nurbs_ui.along_v, false, "U (constant u)");
                ui.selectable_value(&mut self.nurbs_ui.along_v, true, "V (constant v)");
            });
            let (a, b) = if self.nurbs_ui.along_v { dv } else { du };
            self.nurbs_ui.param = self.nurbs_ui.param.clamp(a, b);
            ui.add(egui::Slider::new(&mut self.nurbs_ui.param, a..=b).text("Parameter"));
            let flag = if self.nurbs_ui.along_v { "-v" } else { "-u" };
            let p = self.nurbs_ui.param;
            ui.horizontal_wrapped(|ui| {
                if ui.button("Insert Isoparm").clicked() {
                    cmds.push(format!("insertKnotSurface {} {}", flag, p));
                }
                if ui.button("Detach Surface").clicked() {
                    cmds.push(format!("detachSurface {} {}", flag, p));
                }
                if ui.button("Duplicate Surface Curve").clicked() {
                    cmds.push(format!("duplicateCurve {} {}", flag, p));
                }
            });
            ui.separator();
            if pu || pv {
                ui.horizontal(|ui| {
                    ui.label("Move seam by");
                    ui.add(egui::DragValue::new(&mut self.nurbs_ui.seam).range(0..=64));
                    ui.label("isoparms");
                    if ui.button("Move Seam").clicked() {
                        cmds.push(format!("moveSeam {} -isoparm {}", if pu && !pv { "-u" } else { "-v" }, self.nurbs_ui.seam));
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("Rebuild spans");
                ui.add(egui::DragValue::new(&mut self.nurbs_ui.spans).range(1..=64));
                if ui.button("Rebuild Curve").clicked() {
                    cmds.push(format!("rebuildCurve -s {}", self.nurbs_ui.spans.max(1)));
                }
            });
        });
        for c in cmds {
            self.exec(&c);
        }
        if !open {
            self.nurbs_ui.show_edit = false;
        }
    }

    fn curve_click(&mut self, cam: &Cam, rect: Rect, pos: Pos2) {
        // with a live surface the click lands on the surface under the pointer
        if let Some(live) = self.session.scene.live.filter(|l| self.session.scene.node(*l).is_some()) {
            let (nx, ny) = ((pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height());
            let inv = cam.view_proj().inverse();
            let un = |z: f32| {
                let c = inv * Vec3::new(nx * 2.0 - 1.0, 1.0 - ny * 2.0, z).extend(1.0);
                Vec3::new(c.x / c.w, c.y / c.w, c.z / c.w)
            };
            let (near, far) = (un(-1.0), un(1.0));
            let dir = (far - near).normalized();
            if let Some(h) = self.session.scene.ray_hit_surface(live, near, dir) {
                self.nurbs_ui.pts.push(h);
                return;
            }
        }
        let Some(mut p) = self.ground_point(cam, rect, pos) else { return };
        if self.session.prefs.snap_grid && self.session.prefs.move_step > 1e-6 {
            let st = self.session.prefs.move_step;
            p = Vec3::new((p.x / st).round() * st, (p.y / st).round() * st, (p.z / st).round() * st);
        }
        self.nurbs_ui.pts.push(p);
    }

    fn finish_curve(&mut self) {
        let pts = std::mem::take(&mut self.nurbs_ui.pts);
        if pts.len() < 2 {
            self.say("A curve needs at least two points");
            return;
        }
        let deg = self.nurbs_ui.degree.max(1).min(pts.len() - 1);
        let flag = if self.nurbs_ui.ep { "-ep" } else { "-p" };
        let mut cmd = format!("curve -d {}", deg);
        for p in pts {
            cmd.push_str(&format!(" {} {} {} {}", flag, p.x, p.y, p.z));
        }
        self.exec(&cmd);
        self.mark_dirty();
    }

    fn draw_curve_preview(&self, painter: &egui::Painter, cam: &Cam, rect: Rect) {
        let vp = cam.view_proj();
        let pts: Vec<Pos2> = self.nurbs_ui.pts.iter().filter_map(|p| project(&vp, *p, rect.width(), rect.height()).map(|(x, y, _)| rect.min + vec2(x, y))).collect();
        for p in &pts {
            painter.circle_filled(*p, 3.5, Color32::from_rgb(0xff, 0xd0, 0x20));
        }
        if pts.len() >= 2 {
            painter.add(egui::Shape::line(pts, Stroke::new(1.5_f32, Color32::from_rgb(0xff, 0xd0, 0x20))));
        }
    }
}

impl App {
    /// NURBS Texture Placement: translate, scale and rotate the texture on the selected object.
    fn placement_window(&mut self, ctx: &egui::Context) {
        if !self.nurbs_ui.show_place {
            return;
        }
        let mut open = true;
        let mut cmd: Option<String> = None;
        if self.nurbs_ui.place == [0.0; 5] {
            self.nurbs_ui.place = [0.5, 0.5, 1.0, 1.0, 0.0];
        }
        egui::Window::new("Texture Placement").open(&mut open).default_size([320.0, 220.0]).show(ctx, |ui| {
            ui.label("Puts the middle of the texture at a point of the surface (U, V from 0 to 1).");
            let p = &mut self.nurbs_ui.place;
            let mut changed = false;
            changed |= ui.add(egui::Slider::new(&mut p[0], 0.0..=1.0).text("Center U")).changed();
            changed |= ui.add(egui::Slider::new(&mut p[1], 0.0..=1.0).text("Center V")).changed();
            changed |= ui.add(egui::Slider::new(&mut p[2], 0.1..=20.0).logarithmic(true).text("Repeat U")).changed();
            changed |= ui.add(egui::Slider::new(&mut p[3], 0.1..=20.0).logarithmic(true).text("Repeat V")).changed();
            changed |= ui.add(egui::Slider::new(&mut p[4], -180.0..=180.0).text("Rotate")).changed();
            ui.horizontal(|ui| {
                if ui.button("No wrap (default color outside)").clicked() {
                    cmd = Some("placeTexture -wrap 0".to_string());
                }
                if ui.button("Wrap").clicked() {
                    cmd = Some("placeTexture -wrap 1".to_string());
                }
            });
            if changed {
                cmd = Some(format!("placeTexture -s {} {} -r {} -at {} {}", p[2], p[3], p[4], p[0], p[1]));
            }
        });
        if let Some(c) = cmd {
            self.exec(&c);
        }
        if !open {
            self.nurbs_ui.show_place = false;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Phase 7: Dope Sheet and motion paths

/// Window > Animation Editors > Dope Sheet state.
struct DopeEd {
    x0: f32,
    x1: f32,
    /// Selected keys: (binding index, key index).
    sel: Vec<(usize, usize)>,
    all_nodes: bool,
    /// Frame the pointer last moved selected keys to while dragging.
    drag: Option<f32>,
}

impl App {
    fn dope_sheet_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.dope.take() else { return };
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let sel_nodes = self.session.scene.key_owner_ids(&self.session.scene.selection.clone());
        let rows: Vec<(usize, String)> = self
            .session
            .scene
            .anim
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| d.all_nodes || sel_nodes.contains(&b.node))
            .map(|(i, b)| (i, format!("{}.{}", self.session.scene.name_of(b.node), b.attr)))
            .collect();
        d.sel.retain(|(bi, ki)| self.session.scene.anim.bindings.get(*bi).map(|b| *ki < b.curve.keys.len()).unwrap_or(false));
        let cur = self.session.scene.timeline.current;
        let mut set_time: Option<f32> = None;
        let mut edited = false;
        egui::Window::new("Dope Sheet").open(&mut open).default_size([760.0, 380.0]).show(ctx, |ui| {
            // names and times of the selected keys, for the key commands
            let sel_names = |sc: &Scene, sel: &[(usize, usize)]| -> (Vec<String>, Option<(f32, f32)>) {
                let mut names: Vec<String> = Vec::new();
                let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                for (bi, ki) in sel {
                    if let Some(b) = sc.anim.bindings.get(*bi) {
                        let n = format!("{}.{}", sc.name_of(b.node), b.attr);
                        if !names.contains(&n) {
                            names.push(n);
                        }
                        let t = b.curve.keys[*ki].t;
                        lo = lo.min(t);
                        hi = hi.max(t);
                    }
                }
                (names, if lo <= hi { Some((lo, hi)) } else { None })
            };
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.all_nodes, "All objects");
                ui.separator();
                let (names, range) = sel_names(&self.session.scene, &d.sel);
                let has = !d.sel.is_empty();
                let rng = range.map(|(a, b)| format!("\"{}:{}\"", a, b)).unwrap_or_default();
                let list = names.iter().map(|n| format!("\"{}\"", n)).collect::<Vec<_>>().join(" ");
                if ui.add_enabled(has, egui::Button::new("Copy")).clicked() {
                    cmds.push(format!("copyKey -time {} {}", rng, list));
                }
                if ui.add_enabled(has, egui::Button::new("Cut")).clicked() {
                    cmds.push(format!("cutKeys -time {} {}", rng, list));
                    d.sel.clear();
                }
                if ui.button("Paste").on_hover_text("Paste the key clipboard at the current time").clicked() {
                    cmds.push("pasteKey".to_string());
                }
                ui.separator();
                for (l, dt) in [("<", -1.0), (">", 1.0)] {
                    if ui.add_enabled(has, egui::Button::new(l)).on_hover_text("Move the selected keys one frame").clicked() {
                        cmds.push(format!("moveKey -dt {} -t {} {}", dt, rng, list));
                    }
                }
                for (l, k) in [("Scale x2", 2.0), ("Scale /2", 0.5)] {
                    if ui.add_enabled(has, egui::Button::new(l)).on_hover_text("Retime the selected keys around the first one").clicked() {
                        cmds.push(format!("scaleKey -ts {} -t {} {}", k, rng, list));
                    }
                }
                if ui.add_enabled(has, egui::Button::new("Delete")).clicked() {
                    let mut by: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
                    for (bi, ki) in &d.sel {
                        by.entry(*bi).or_default().push(*ki);
                    }
                    for (bi, mut ks) in by {
                        ks.sort_unstable();
                        if let Some(b) = self.session.scene.anim.bindings.get_mut(bi) {
                            for k in ks.into_iter().rev() {
                                b.curve.keys.remove(k);
                            }
                            b.curve.recompute_tangents();
                        }
                    }
                    self.session.scene.anim.bindings.retain(|b| !b.curve.keys.is_empty());
                    d.sel.clear();
                    edited = true;
                }
            });
            let row_h = 20.0;
            let label_w = 170.0;
            let h = (rows.len() as f32 + 1.0) * row_h + 24.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(300.0), h.max(120.0)), Sense::click_and_drag());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 0.0, Color32::from_rgb(0x30, 0x30, 0x30));
            let area = Rect::from_min_max(pos2(rect.left() + label_w, rect.top()), rect.max);
            let sx = |t: f32, d: &DopeEd| area.left() + (t - d.x0) / (d.x1 - d.x0) * area.width();
            let tx = |x: f32, d: &DopeEd| d.x0 + (x - area.left()) / area.width() * (d.x1 - d.x0);
            // frame ruler
            let step = if d.x1 - d.x0 > 120.0 { 10.0 } else if d.x1 - d.x0 > 40.0 { 5.0 } else { 1.0 };
            let mut f = (d.x0 / step).ceil() * step;
            while f <= d.x1 {
                let x = sx(f, &d);
                p.line_segment([pos2(x, rect.top() + 16.0), pos2(x, rect.bottom())], Stroke::new(0.5, Color32::from_gray(0x50)));
                p.text(pos2(x + 2.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, format!("{}", f), egui::FontId::proportional(10.0), Color32::from_gray(0xb0));
                f += step;
            }
            let (s0, s1) = (self.session.scene.timeline.start, self.session.scene.timeline.end);
            p.rect_filled(Rect::from_min_max(pos2(sx(s0, &d), rect.top() + 16.0), pos2(sx(s1, &d), rect.bottom())), 0.0, Color32::from_rgba_unmultiplied(0x60, 0x80, 0xc0, 14));
            let cx = sx(cur, &d);
            p.line_segment([pos2(cx, rect.top()), pos2(cx, rect.bottom())], Stroke::new(1.5, Color32::from_rgb(0x60, 0x90, 0xe0)));
            let mut hit: Option<(usize, usize)> = None;
            let pointer = resp.hover_pos();
            for (r, (bi, label)) in rows.iter().enumerate() {
                let y = rect.top() + 24.0 + r as f32 * row_h;
                p.text(pos2(rect.left() + 4.0, y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(11.0), Color32::from_gray(0xd0));
                p.line_segment([pos2(rect.left(), y + row_h * 0.5), pos2(rect.right(), y + row_h * 0.5)], Stroke::new(0.5, Color32::from_gray(0x3a)));
                let b = &self.session.scene.anim.bindings[*bi];
                for (ki, k) in b.curve.keys.iter().enumerate() {
                    let c = pos2(sx(k.t, &d), y);
                    if c.x < area.left() - 6.0 || c.x > area.right() + 6.0 {
                        continue;
                    }
                    let selected = d.sel.contains(&(*bi, ki));
                    let col = if selected { Color32::from_rgb(0xff, 0xf0, 0x40) } else { Color32::from_rgb(0xe0, 0x60, 0x50) };
                    p.add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -5.0), c + vec2(5.0, 0.0), c + vec2(0.0, 5.0), c + vec2(-5.0, 0.0)], col, Stroke::new(0.8, Color32::BLACK)));
                    if let Some(m) = pointer {
                        if (m - c).length() < 8.0 {
                            hit = Some((*bi, ki));
                        }
                    }
                }
            }
            let shift = ui.ctx().input(|i| i.modifiers.shift);
            if resp.drag_started() {
                if let Some(h) = hit {
                    if !d.sel.contains(&h) {
                        if shift {
                            d.sel.push(h);
                        } else {
                            d.sel = vec![h];
                        }
                    }
                    d.drag = resp.interact_pointer_pos().map(|q| tx(q.x, &d).round());
                }
            }
            if resp.dragged() && !d.sel.is_empty() {
                if let (Some(last), Some(q)) = (d.drag, resp.interact_pointer_pos()) {
                    let now = tx(q.x, &d).round();
                    let delta = now - last;
                    if delta.abs() >= 1.0 {
                        // move the selected keys together; the key list order may change, so track by time
                        let mut moved: Vec<(usize, f32)> = Vec::new();
                        for (bi, ki) in &d.sel {
                            if let Some(b) = self.session.scene.anim.bindings.get(*bi) {
                                moved.push((*bi, b.curve.keys[*ki].t));
                            }
                        }
                        for (bi, t) in &moved {
                            if let Some(b) = self.session.scene.anim.bindings.get_mut(*bi) {
                                for k in b.curve.keys.iter_mut().filter(|k| (k.t - *t).abs() < 1e-3) {
                                    k.t = (k.t + delta).max(0.0);
                                }
                                b.curve.keys.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
                                b.curve.recompute_tangents();
                            }
                        }
                        d.sel = moved
                            .iter()
                            .filter_map(|(bi, t)| self.session.scene.anim.bindings.get(*bi).and_then(|b| b.curve.keys.iter().position(|k| (k.t - (t + delta).max(0.0)).abs() < 1e-3)).map(|ki| (*bi, ki)))
                            .collect();
                        d.drag = Some(now);
                        edited = true;
                    }
                }
            }
            if resp.drag_stopped() {
                d.drag = None;
            }
            if resp.clicked() {
                match hit {
                    Some(h) => {
                        if shift {
                            if let Some(i) = d.sel.iter().position(|x| *x == h) {
                                d.sel.remove(i);
                            } else {
                                d.sel.push(h);
                            }
                        } else {
                            d.sel = vec![h];
                        }
                    }
                    None => {
                        if let Some(q) = resp.interact_pointer_pos() {
                            if q.y < rect.top() + 18.0 && q.x > area.left() {
                                set_time = Some(tx(q.x, &d).round());
                            } else if !shift {
                                d.sel.clear();
                            }
                        }
                    }
                }
            }
            // zoom and pan with the wheel / middle drag
            if resp.hovered() {
                let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
                if scroll.abs() > 0.1 {
                    let f = (1.0 - scroll * 0.002).clamp(0.5, 2.0);
                    let c = d.x0 + (d.x1 - d.x0) * 0.5;
                    let half = (d.x1 - d.x0) * 0.5 * f;
                    d.x0 = c - half;
                    d.x1 = c + half;
                }
            }
            if resp.dragged_by(egui::PointerButton::Middle) {
                let dx = resp.drag_delta().x / area.width() * (d.x1 - d.x0);
                d.x0 -= dx;
                d.x1 -= dx;
            }
            ui.label(egui::RichText::new("Click a key, Shift adds; drag to retime; click the ruler to set the time. Wheel zooms, middle drag pans.").small().color(TEXT_DIM));
        });
        for c in cmds {
            self.exec(&c);
            edited = true;
        }
        if let Some(t) = set_time {
            self.session.scene.set_time(t);
            self.mark_dirty();
        }
        if edited {
            self.session.scene.evaluate_frame();
            self.mark_dirty();
        }
        if open {
            self.dope = Some(d);
        }
    }
}

#[derive(Default)]
struct SkinUi {
    show: bool,
    joint: Option<NodeId>,
    op: usize,
    value: f32,
    radius: f32,
    strength: f32,
}

#[derive(Default)]
struct PfxUi {
    show: bool,
    brush: String,
    path: Vec<(Vec3, Vec3)>,
}

impl App {
    /// Add the surface point under the pointer to the stroke being painted.
    fn pfx_paint_at(&mut self, cam: &Cam, rect: Rect, pos: Pos2) {
        let (nx, ny) = ((pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height());
        let inv = cam.view_proj().inverse();
        let un = |z: f32| {
            let c = inv * Vec3::new(nx * 2.0 - 1.0, 1.0 - ny * 2.0, z).extend(1.0);
            Vec3::new(c.x / c.w, c.y / c.w, c.z / c.w)
        };
        let (near, far) = (un(-1.0), un(1.0));
        let dir = (far - near).normalized();
        let shapes: Vec<NodeId> = self.session.scene.ids_where(|d| matches!(d, NodeData::Mesh(md) if md.stroke.is_none()));
        let mut best: Option<(f32, (Vec3, Vec3))> = None;
        for sh in shapes {
            let paintable = self.session.scene.node(sh).and_then(|n| n.plug("paintable")).map(|p| p.value > 0.5).unwrap_or(false);
            if !paintable {
                continue;
            }
            if let Some(h) = crate::mel::cmd_paintfx::surface_hit(&mut self.session, sh, near, dir) {
                let d = h.0.distance(near);
                if best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, h));
                }
            }
        }
        match best {
            Some((_, h)) => {
                if self.pfx_ui.path.last().map(|l| l.0.distance(h.0) > 0.05).unwrap_or(true) {
                    self.pfx_ui.path.push(h);
                }
            }
            None => self.say("Paint Effects: no paintable surface under the pointer (Paint Effects > Make Paintable)"),
        }
    }

    fn pfx_finish_stroke(&mut self) {
        let path = std::mem::take(&mut self.pfx_ui.path);
        if path.is_empty() {
            return;
        }
        let brush = match self.session.scene.find(&self.pfx_ui.brush).filter(|b| matches!(self.session.scene.node(*b).map(|n| &n.data), Some(NodeData::Brush(_)))) {
            Some(b) => b,
            None => match self.session.scene.create_brush(Some("gloppy")) {
                Ok(b) => {
                    self.pfx_ui.brush = self.session.scene.name_of(b);
                    b
                }
                Err(_) => return,
            },
        };
        let seed = self.session.scene.stroke_shapes().len() as u32 + 1;
        let shape = self.session.scene.create_stroke(brush, path, seed);
        let t = self.session.scene.node(shape).and_then(|n| n.parent).unwrap_or(shape);
        self.session.scene.selection = vec![t];
    }

    /// The Visor (brush presets), brush settings and the Paint Effects canvas.
    fn visor_window(&mut self, ctx: &egui::Context) {
        if !self.pfx_ui.show {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let mut tool = false;
        egui::Window::new("Visor / Paint Effects").open(&mut open).default_width(300.0).show(ctx, |ui| {
            for (folder, brushes) in crate::geo::paintfx::PRESET_FOLDERS {
                ui.collapsing(*folder, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for b in *brushes {
                            if ui.button(*b).on_hover_text("Create this brush and make it the current one").clicked() {
                                cmds.push(format!("paintBrush -preset {}", b));
                                self.pfx_ui.brush = b.to_string();
                            }
                        }
                    });
                });
            }
            ui.separator();
            ui.label(format!("Current brush: {}", if self.pfx_ui.brush.is_empty() { "(none)" } else { &self.pfx_ui.brush }));
            if let Some(b) = self.session.scene.find(&self.pfx_ui.brush) {
                for k in ["brushWidth", "globalScale", "tubesPerStep", "lengthMin", "lengthMax", "tubeWidth1", "tubeWidth2", "gravity", "segments"] {
                    let mut v = self.session.scene.get_attr(b, k).unwrap_or(0.0);
                    let old = v;
                    ui.add(egui::Slider::new(&mut v, 0.0..=if k == "tubesPerStep" || k == "segments" { 20.0 } else { 4.0 }).text(k));
                    if (v - old).abs() > 1e-6 {
                        cmds.push(format!("setAttr {}.{} {}", self.pfx_ui.brush, k, v));
                    }
                }
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui.button("Paint Effects Tool").clicked() {
                    tool = true;
                }
                if ui.button("Make Paintable").clicked() {
                    cmds.push("makePaintable".into());
                }
                if ui.button("Auto Paint").clicked() {
                    cmds.push(format!("autoPaint -b {} -count 10", self.pfx_ui.brush));
                }
                if ui.button("Share One Brush").clicked() {
                    cmds.push("shareBrush".into());
                }
                if ui.button("Get Settings from Selected Stroke").clicked() {
                    cmds.push("brushFromStroke".into());
                }
                if ui.button("Convert to Polygons").clicked() {
                    cmds.push("strokeToPoly".into());
                }
            });
            ui.separator();
            ui.label("Canvas");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Clear").clicked() {
                    cmds.push("paintCanvas -clear \"1 1 1\"".into());
                }
                if ui.button("Save canvas.png").clicked() {
                    cmds.push("paintCanvas -save canvas.png".into());
                }
                if ui.button("Stroke across").clicked() {
                    cmds.push(format!("paintCanvas -b {} -pts \"40 60 200 120 360 60\"", self.pfx_ui.brush));
                }
            });
        });
        if !open {
            self.pfx_ui.show = false;
        }
        if tool {
            self.start_tool(Tool::PaintFx);
        }
        for c in cmds {
            self.exec(&c);
        }
    }
}

impl App {
    /// Start `aztec2009` on a copy of the scene with `cmds` (project, scene and then the render commands).
    fn spawn_render_process(&mut self, cmds: &str) -> Result<std::process::Child, String> {
        self.session.view.apply(&mut self.session.scene);
        let stem = self.session.scene.file_path.as_ref().and_then(|p| std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().to_string())).unwrap_or_else(|| "untitled".to_string());
        let dir = std::env::temp_dir().join(format!("aztec_render_{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot make {}: {}", dir.display(), e))?;
        let file = dir.join(format!("{}.aztec", stem));
        std::fs::write(&file, crate::dag::serialize::to_ascii(&self.session.scene)).map_err(|e| format!("Cannot write {}: {}", file.display(), e))?;
        let mut script = String::new();
        if let Some(p) = &self.session.scene.project_dir {
            script.push_str(&format!("project -set {}; ", Self::quote(&p.to_string_lossy())));
        }
        script.push_str(&format!("file -open {}; {}", Self::quote(&file.to_string_lossy()), cmds));
        let exe = std::env::current_exe().map_err(|e| format!("Cannot find the program: {}", e))?;
        std::process::Command::new(exe).arg("-e").arg(script).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map_err(|e| format!("Cannot start the renderer: {}", e))
    }

    /// Render Current Frame with the Render Settings, in the background.
    fn start_pipe_render(&mut self, region: Option<(usize, usize, usize, usize)>) {
        if self.pipe_job.is_some() {
            self.say("A render is already running");
            return;
        }
        let out = std::env::temp_dir().join(format!("aztec_render_{}", std::process::id())).join(if region.is_some() { "region.png" } else { "frame.png" });
        let _ = std::fs::remove_file(&out);
        let cam = self.session.view.camera.clone().unwrap_or_else(|| "persp".to_string());
        let mut cmd = format!("renderImage -cam {} -o {}", cam, Self::quote(&out.to_string_lossy()));
        if let Some(l) = self.rset_ui.layer.clone().into_some_nonempty() {
            cmd.push_str(&format!(" -layer {}", l));
        }
        if let Some((a, b, c, d)) = region {
            cmd.push_str(&format!(" -region \"{} {} {} {}\"", a, b, c, d));
        }
        let frame = self.session.scene.timeline.frame();
        match self.spawn_render_process(&cmd) {
            Ok(child) => {
                self.pipe_job = Some(PipeJob { child, out, started: std::time::Instant::now(), region, frame });
                if let Some(rv) = &mut self.render_view {
                    rv.note = "Rendering...".to_string();
                }
            }
            Err(e) => self.say(&e),
        }
    }

    fn poll_pipe_jobs(&mut self, ctx: &egui::Context) {
        if let Some(job) = &mut self.pipe_job {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            if let Ok(Some(status)) = job.child.try_wait() {
                let job = self.pipe_job.take().unwrap();
                let secs = job.started.elapsed().as_secs_f32();
                match (status.success(), image::open(&job.out)) {
                    (true, Ok(img)) => {
                        let img = img.to_rgba8();
                        let (w, h) = (img.width() as usize, img.height() as usize);
                        let data = img.into_raw();
                        if let Some(rv) = &mut self.render_view {
                            match (job.region, rv.w == w && rv.h == h && rv.rgba.len() == data.len()) {
                                (Some((x0, y0, x1, y1)), true) => {
                                    for y in y0.min(h)..y1.min(h) {
                                        for x in x0.min(w)..x1.min(w) {
                                            let i = (y * w + x) * 4;
                                            rv.rgba[i..i + 4].copy_from_slice(&data[i..i + 4]);
                                        }
                                    }
                                }
                                _ => rv.rgba = data,
                            }
                            rv.w = w;
                            rv.h = h;
                            rv.tex = None;
                            rv.note = format!("Rendered frame {} at {}x{} in {:.1}s", job.frame, w, h, secs);
                        }
                    }
                    _ => {
                        if let Some(rv) = &mut self.render_view {
                            rv.note = "The render failed (see the Script Editor: renderImage)".to_string();
                        }
                    }
                }
            }
        }
        if let Some((child, started)) = &mut self.batch_job {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
            if let Ok(Some(status)) = child.try_wait() {
                let secs = started.elapsed().as_secs_f32();
                self.batch_job = None;
                self.say(&if status.success() { format!("Batch render finished in {:.1}s: Render > Show Batch Render", secs) } else { "Batch render failed".to_string() });
            }
        }
    }

    /// Render > Batch Render: a separate process renders every frame to the images folder.
    fn start_batch_render(&mut self) {
        if self.batch_job.is_some() {
            self.say("A batch render is already running");
            return;
        }
        match self.spawn_render_process("batchRender") {
            Ok(child) => {
                self.batch_job = Some((child, std::time::Instant::now()));
                self.say("Batch render started (you can keep working; Render > Cancel Batch Render stops it)");
            }
            Err(e) => self.say(&e),
        }
    }

    fn images_dir(&self) -> std::path::PathBuf {
        match &self.session.scene.project_dir {
            Some(d) => d.join("images"),
            None => std::path::PathBuf::from("images"),
        }
    }

    /// Menu items shared by the Render menu: settings, batch rendering, layers and FCheck.
    fn render_menu_extras(&mut self, ui: &mut Ui) {
        ui.separator();
        if ui.button("Render Settings...").clicked() {
            self.rset_ui.show = true;
            ui.close_menu();
        }
        if ui.button("Render Layers...").clicked() {
            self.rset_ui.show_layers = true;
            ui.close_menu();
        }
        ui.separator();
        if ui.add_enabled(self.batch_job.is_none(), egui::Button::new("Batch Render")).on_hover_text("Renders every frame, layer and pass to the images folder in a separate process").clicked() {
            self.start_batch_render();
            ui.close_menu();
        }
        if ui.add_enabled(self.batch_job.is_some(), egui::Button::new("Cancel Batch Render")).clicked() {
            if let Some((c, _)) = &mut self.batch_job {
                let _ = c.kill();
            }
            self.batch_job = None;
            ui.close_menu();
        }
        if ui.button("Show Batch Render...").on_hover_text("Open the rendered sequence in FCheck").clicked() {
            let dir = self.images_dir();
            let mut found: Vec<String> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path().to_string_lossy().to_string()).filter(|p| p.ends_with(".png") || p.ends_with(".jpg")).collect()).unwrap_or_default();
            found.sort();
            if let Some(first) = found.first() {
                self.fcheck_ui.path = first.clone();
            } else {
                self.fcheck_ui.path = dir.join("name.0001.png").to_string_lossy().to_string();
            }
            self.fcheck_load();
            self.fcheck_ui.show = true;
            ui.close_menu();
        }
        if ui.button("FCheck...").clicked() {
            self.fcheck_ui.show = true;
            ui.close_menu();
        }
    }

    fn fcheck_load(&mut self) {
        let p = self.fcheck_ui.path.clone();
        self.fcheck_ui.frames = crate::render::pipeline::find_sequence(&p);
        self.fcheck_ui.idx = 0;
        self.fcheck_ui.shown = None;
        self.fcheck_ui.note = if self.fcheck_ui.frames.is_empty() { format!("No sequence found for {}", p) } else { format!("{} frame(s)", self.fcheck_ui.frames.len()) };
    }

    /// FCheck: open an image sequence and play it back.
    fn fcheck_window(&mut self, ctx: &egui::Context) {
        if !self.fcheck_ui.show {
            return;
        }
        let mut open = true;
        let fps = self.session.scene.timeline.fps.max(1.0);
        let mut load = false;
        egui::Window::new("FCheck").open(&mut open).default_size([520.0, 420.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.fcheck_ui.path).desired_width(ui.available_width() - 130.0).hint_text("images/name.0001.png"));
                if ui.button("Open Sequence").clicked() {
                    load = true;
                }
            });
            let n = self.fcheck_ui.frames.len();
            ui.horizontal(|ui| {
                if ui.button(if self.fcheck_ui.playing { "Pause" } else { "Play" }).clicked() {
                    self.fcheck_ui.playing = !self.fcheck_ui.playing;
                    self.fcheck_ui.last = Some(std::time::Instant::now());
                }
                if n > 0 {
                    let mut i = self.fcheck_ui.idx;
                    ui.add(egui::Slider::new(&mut i, 0..=n - 1).text("frame"));
                    self.fcheck_ui.idx = i;
                }
                ui.label(egui::RichText::new(&self.fcheck_ui.note).color(TEXT_DIM));
            });
            if n > 0 {
                if self.fcheck_ui.playing {
                    ctx.request_repaint();
                    let due = self.fcheck_ui.last.map(|t| t.elapsed().as_secs_f32() >= 1.0 / fps).unwrap_or(true);
                    if due {
                        self.fcheck_ui.idx = (self.fcheck_ui.idx + 1) % n;
                        self.fcheck_ui.last = Some(std::time::Instant::now());
                    }
                }
                if self.fcheck_ui.shown != Some(self.fcheck_ui.idx) {
                    let (num, path) = self.fcheck_ui.frames[self.fcheck_ui.idx].clone();
                    if let Ok(img) = image::open(&path) {
                        let rgba = img.to_rgba8();
                        let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                        self.fcheck_ui.tex = Some(ctx.load_texture("fcheck", ci, TextureOptions::LINEAR));
                        self.fcheck_ui.note = format!("frame {} of {} ({})", num, n, path);
                    }
                    self.fcheck_ui.shown = Some(self.fcheck_ui.idx);
                }
            }
            if let Some(t) = &self.fcheck_ui.tex {
                let sz = t.size_vec2();
                let avail = ui.available_size();
                let k = (avail.x / sz.x).min(avail.y.max(100.0) / sz.y).max(0.05);
                ui.image((t.id(), sz * k));
            }
        });
        if load {
            self.fcheck_load();
        }
        if !open {
            self.fcheck_ui.show = false;
            self.fcheck_ui.playing = false;
        }
    }

    /// The Render Settings window: Common, Quality, Motion Blur, Vector and Passes.
    fn render_settings_window(&mut self, ctx: &egui::Context) {
        if !self.rset_ui.show {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let layer = self.rset_ui.layer.clone();
        let layer_data = self.session.scene.render_layers.iter().find(|l| l.name == layer).cloned();
        let st = crate::render::pipeline::Settings::from_scene(&mut self.session.scene, layer_data.as_ref());
        let layers: Vec<String> = self.session.scene.render_layers.iter().map(|l| l.name.clone()).collect();
        egui::Window::new(if layer.is_empty() { "Render Settings".to_string() } else { format!("Render Settings ({})", layer) }).open(&mut open).default_width(420.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Render Layer");
                egui::ComboBox::from_id_salt("rsl").selected_text(if layer.is_empty() { "masterLayer" } else { &layer }).show_ui(ui, |ui| {
                    if ui.selectable_label(layer.is_empty(), "masterLayer").clicked() {
                        self.rset_ui.layer.clear();
                    }
                    for l in &layers {
                        if ui.selectable_label(&layer == l, l).clicked() {
                            self.rset_ui.layer = l.clone();
                        }
                    }
                });
                ui.label("Render Using");
                let mut r = st.renderer;
                egui::ComboBox::from_id_salt("rsr").selected_text(["Software", "Hardware (no ray tracing)", "Vector (flat fills and outlines)"][r as usize]).show_ui(ui, |ui| {
                    for (i, l) in ["Software", "Hardware (no ray tracing)", "Vector (flat fills and outlines)"].iter().enumerate() {
                        ui.selectable_value(&mut r, i as u8, *l);
                    }
                });
                if r != st.renderer {
                    cmds.push(format!("renderSettings{} -renderer {}", if layer.is_empty() { String::new() } else { format!(" -layer {}", layer) }, ["software", "hardware", "vector"][r as usize]));
                }
            });
            if !layer.is_empty() {
                ui.label(egui::RichText::new("Changes here are overrides for this layer only").color(TEXT_DIM));
            }
            ui.horizontal(|ui| {
                for (i, t) in ["Common", "Quality", "Motion Blur", "Vector", "Passes"].iter().enumerate() {
                    ui.selectable_value(&mut self.rset_ui.tab, i, *t);
                }
            });
            ui.separator();
            let lf = if layer.is_empty() { String::new() } else { format!(" -layer {}", layer) };
            let mut num = |ui: &mut Ui, label: &str, flag: &str, v: f32, range: std::ops::RangeInclusive<f32>, cmds: &mut Vec<String>| {
                let mut x = v;
                ui.horizontal(|ui| {
                    ui.add_sized([130.0, 18.0], egui::Label::new(label));
                    ui.add(egui::DragValue::new(&mut x).range(range).speed(1.0));
                });
                if (x - v).abs() > 1e-6 {
                    cmds.push(format!("renderSettings{} -{} {}", lf, flag, x));
                }
            };
            match self.rset_ui.tab {
                0 => {
                    let mut prefix = st.prefix.clone();
                    ui.horizontal(|ui| {
                        ui.label("File name prefix");
                        if ui.add(egui::TextEdit::singleline(&mut prefix).desired_width(220.0).hint_text("<Scene>")).lost_focus() && prefix != st.prefix {
                            cmds.push(format!("renderSettings{} -prefix {}", lf, Self::quote(&prefix)));
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for tok in ["<Scene>", "<RenderLayer>", "<Camera>", "<RenderPass>"] {
                            if ui.small_button(tok).on_hover_text("Insert this token at the end of the prefix").clicked() {
                                cmds.push(format!("renderSettings{} -prefix {}", lf, Self::quote(&format!("{}{}", st.prefix, tok))));
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Image format");
                        for e in ["png", "jpg", "bmp", "tga"] {
                            if ui.selectable_label(st.ext == e, e).clicked() {
                                cmds.push(format!("renderSettings{} -ext {}", lf, e));
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Frame/Animation ext");
                        for (i, l) in ["name.ext", "name.#.ext", "name.ext.#"].iter().enumerate() {
                            if ui.selectable_label(st.animation == i as u8, *l).clicked() {
                                cmds.push(format!("renderSettings -animation {}", i));
                            }
                        }
                    });
                    num(ui, "Start frame", "s", st.start as f32, -9999.0..=99999.0, &mut cmds);
                    num(ui, "End frame", "e", st.end as f32, -9999.0..=99999.0, &mut cmds);
                    num(ui, "By frame", "b", st.by as f32, 1.0..=1000.0, &mut cmds);
                    num(ui, "Width", "w", st.w as f32, 16.0..=4096.0, &mut cmds);
                    num(ui, "Height", "h", st.h as f32, 16.0..=4096.0, &mut cmds);
                    ui.horizontal(|ui| {
                        for (l, w, h) in [("320x240", 320, 240), ("640x480", 640, 480), ("HD 720", 1280, 720), ("HD 1080", 1920, 1080)] {
                            if ui.small_button(l).clicked() {
                                cmds.push(format!("renderSettings{} -w {} -h {}", lf, w, h));
                            }
                        }
                        if ui.checkbox(&mut self.res_gate, "Resolution gate").changed() {}
                    });
                    let mut a = st.alpha;
                    if ui.checkbox(&mut a, "Alpha channel (mask)").changed() {
                        cmds.push(format!("renderSettings{} -alpha {}", lf, a as u8));
                    }
                    let mut dch = st.depth;
                    if ui.checkbox(&mut dch, "Depth channel (z depth)").changed() {
                        cmds.push(format!("renderSettings{} -depth {}", lf, dch as u8));
                    }
                    ui.horizontal(|ui| {
                        ui.label("Camera");
                        let mut cam = st.camera.clone();
                        if ui.add(egui::TextEdit::singleline(&mut cam).desired_width(140.0).hint_text("renderable cameras")).lost_focus() && cam != st.camera {
                            cmds.push(format!("renderSettings{} -cam {}", lf, Self::quote(&cam)));
                        }
                    });
                }
                1 => {
                    ui.horizontal(|ui| {
                        ui.label("Anti-aliasing quality");
                        for (i, l) in ["Preview", "Intermediate", "Production", "High"].iter().enumerate() {
                            if ui.selectable_label(st.quality == i as u8, *l).clicked() {
                                cmds.push(format!("renderSettings{} -quality {}", lf, i));
                            }
                        }
                    });
                    let mut rt = st.raytracing;
                    if ui.checkbox(&mut rt, "Raytracing (reflections and refractions)").changed() {
                        cmds.push(format!("renderSettings{} -raytracing {}", lf, rt as u8));
                    }
                    ui.label(egui::RichText::new("Shadows are traced; depth map shadow blur is set on each light (resolution, filter size).").color(TEXT_DIM));
                }
                2 => {
                    let mut mb = st.motion_blur;
                    if ui.checkbox(&mut mb, "Motion blur").changed() {
                        cmds.push(format!("renderSettings{} -mb {}", lf, mb as u8));
                    }
                    ui.horizontal(|ui| {
                        ui.label("Motion blur type");
                        for (i, l) in ["2D", "3D"].iter().enumerate() {
                            if ui.selectable_label(st.blur_3d == (i == 1), *l).clicked() {
                                cmds.push(format!("renderSettings{} -blurType {}", lf, l.to_lowercase()));
                            }
                        }
                    });
                    num(ui, "Blur length", "blurLength", st.blur_length, 0.0..=10.0, &mut cmds);
                    num(ui, "3D samples", "blurSamples", st.blur_samples as f32, 2.0..=16.0, &mut cmds);
                    ui.label(egui::RichText::new("Objects choose Motion Blur in their Render Stats (renderStats -blur 0|1).").color(TEXT_DIM));
                }
                3 => {
                    ui.horizontal(|ui| {
                        ui.label("Fill style");
                        for (i, l) in ["Single color", "Full color", "Two color"].iter().enumerate() {
                            if ui.selectable_label(st.vector_fill == i as u8, *l).clicked() {
                                cmds.push(format!("renderSettings{} -attr \"vectorFill {}\"", lf, i));
                            }
                        }
                    });
                    for (l, k, v) in [("Include edges", "vectorEdges", st.vector_edges), ("Shadows", "vectorShadows", st.vector_shadows), ("Highlights", "vectorHighlights", st.vector_highlights), ("Reflections", "vectorReflections", st.vector_reflections)] {
                        let mut x = v;
                        if ui.checkbox(&mut x, l).changed() {
                            cmds.push(format!("renderSettings{} -attr \"{} {}\"", lf, k, x as u8));
                        }
                    }
                    let mut ew = st.edge_weight;
                    ui.horizontal(|ui| {
                        ui.add_sized([130.0, 18.0], egui::Label::new("Edge weight (pt)"));
                        ui.add(egui::DragValue::new(&mut ew).range(0.0..=10.0).speed(0.1));
                    });
                    if (ew - st.edge_weight).abs() > 1e-6 {
                        cmds.push(format!("renderSettings{} -attr \"vectorEdgeWeight {}\"", lf, ew));
                    }
                    ui.label(egui::RichText::new("The vector renderer writes bitmaps (no SVG or SWF).").color(TEXT_DIM));
                }
                _ => {
                    if layer.is_empty() {
                        ui.label("Passes belong to render layers: choose a render layer above.");
                    } else if let Some(ld) = &layer_data {
                        ui.label("Render passes for this layer (the beauty pass is always rendered):");
                        let mut chosen = ld.passes.clone();
                        let before = chosen.clone();
                        for pass in &crate::render::pipeline::PASS_NAMES[1..] {
                            let mut on = chosen.iter().any(|c| c == pass);
                            if ui.checkbox(&mut on, *pass).changed() {
                                chosen.retain(|c| c != pass);
                                if on {
                                    chosen.push(pass.to_string());
                                }
                            }
                        }
                        if chosen != before {
                            cmds.push(format!("renderLayer -passes {} {}", layer, if chosen.is_empty() { "\"\"".to_string() } else { Self::quote(&chosen.join(" ")) }));
                        }
                    }
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Render Current Frame").clicked() {
                    cmds.push("__render".to_string());
                }
                if ui.button("Batch Render").clicked() {
                    cmds.push("__batch".to_string());
                }
            });
        });
        if !open {
            self.rset_ui.show = false;
        }
        for c in cmds {
            match c.as_str() {
                "__render" => {
                    if self.render_view.is_none() {
                        self.render_still();
                    } else {
                        self.render_still();
                    }
                }
                "__batch" => self.start_batch_render(),
                _ => self.exec(&c),
            }
        }
    }

    /// The Layer Editor's Render tab: create layers, add the selected objects and lights, renderable flags.
    fn render_layers_window(&mut self, ctx: &egui::Context) {
        if !self.rset_ui.show_layers {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let layers = self.session.scene.render_layers.clone();
        egui::Window::new("Render Layers").open(&mut open).default_width(360.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.rset_ui.new_layer).desired_width(150.0).hint_text("layer name"));
                if ui.button("Create Empty Layer").clicked() && !self.rset_ui.new_layer.trim().is_empty() {
                    cmds.push(format!("renderLayer -create {}", self.rset_ui.new_layer.trim()));
                    self.rset_ui.new_layer.clear();
                }
            });
            ui.label(egui::RichText::new("masterLayer holds everything. A layer renders only its own objects and lights.").color(TEXT_DIM));
            ui.separator();
            for l in &layers {
                ui.horizontal(|ui| {
                    let mut r = l.renderable;
                    if ui.checkbox(&mut r, "").on_hover_text("Renderable").changed() {
                        cmds.push(format!("renderLayer -renderable {} {}", l.name, r as u8));
                    }
                    ui.label(format!("{}  ({} objects, passes: {})", l.name, l.members.len(), if l.passes.is_empty() { "beauty".to_string() } else { l.passes.join(" ") }));
                });
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    if ui.small_button("Add Selected Objects").clicked() {
                        cmds.push(format!("renderLayer -add {}", l.name));
                    }
                    if ui.small_button("Remove Selected").clicked() {
                        cmds.push(format!("renderLayer -remove {}", l.name));
                    }
                    if ui.small_button("Settings...").clicked() {
                        self.rset_ui.layer = l.name.clone();
                        self.rset_ui.show = true;
                    }
                    if ui.small_button("Render").clicked() {
                        self.rset_ui.layer = l.name.clone();
                        cmds.push("__render".to_string());
                    }
                    if ui.small_button("Delete").clicked() {
                        cmds.push(format!("renderLayer -delete {}", l.name));
                    }
                });
            }
            if layers.is_empty() {
                ui.label("No render layers yet.");
            }
        });
        if !open {
            self.rset_ui.show_layers = false;
        }
        for c in cmds {
            if c == "__render" {
                self.render_still();
            } else {
                self.exec(&c);
            }
        }
    }
}

trait IntoSomeNonEmpty {
    fn into_some_nonempty(self) -> Option<String>;
}

impl IntoSomeNonEmpty for String {
    fn into_some_nonempty(self) -> Option<String> {
        if self.is_empty() {
            None
        } else {
            Some(self)
        }
    }
}

impl App {
    /// IPR: after any command the tuned region is rendered again.
    fn ipr_after_command(&mut self) {
        let region = match &self.render_view {
            Some(rv) if rv.ipr && rv.pipe => rv.region,
            _ => None,
        };
        if let (Some(r), None) = (region, &self.pipe_job) {
            self.start_pipe_render(Some(r));
        }
    }
}

#[derive(Default)]
struct EventUi {
    show: bool,
    target: String,
    num: u32,
    spread: f32,
    inherit: f32,
    dies: bool,
}

impl App {
    /// The Dynamics menu set: Fields, Soft/Rigid Bodies, Particles, Effects and Solvers.
    fn dynamics_menus(&mut self, ui: &mut Ui) {
        ui.menu_button("Fields", |ui| {
            for (l, c, tip) in [
                ("Air", "air", "Wind: pushes toward its speed along its direction"),
                ("Drag", "drag", "Slows the objects it affects"),
                ("Gravity", "gravity", "9.8 downwards: select the objects first to connect it to them"),
                ("Newton", "newton", "Pulls toward the field (attenuation sets the falloff)"),
                ("Radial", "radial", "Pushes away from the field"),
                ("Turbulence", "turbulence", "A random swirling force"),
                ("Uniform", "uniform", "The same push everywhere"),
                ("Vortex", "vortex", "Spins around its direction axis"),
            ] {
                if ui.button(l).on_hover_text(tip).clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui.button("Affect Selected Object(s)").on_hover_text("Select the objects, then the field last").clicked() {
                let field = self.session.scene.selection.iter().rev().copied().find(|i| {
                    let sc = &self.session.scene;
                    matches!(sc.node(*i).map(|n| &n.data), Some(NodeData::Field(_))) || sc.node(*i).map(|n| n.children.iter().any(|c| matches!(sc.node(*c).map(|x| &x.data), Some(NodeData::Field(_))))).unwrap_or(false)
                });
                match field {
                    Some(f) => {
                        let fname = self.session.scene.name_of(f);
                        let objs: Vec<String> = self.session.scene.selection.iter().filter(|i| **i != f).map(|i| self.session.scene.name_of(*i)).collect();
                        self.exec(&format!("connectDynamic -f {} {}", fname, objs.join(" ")));
                    }
                    None => self.say("Select the objects, then the field"),
                }
                ui.close_menu();
            }
        });
        ui.menu_button("Soft/Rigid Bodies", |ui| {
            for (l, c) in [("Create Active Rigid Body", "rigidBody -active"), ("Create Passive Rigid Body", "rigidBody -passive")] {
                if ui.button(l).on_hover_text("Go to the first frame before creating a body on an animated object").clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
            ui.menu_button("Create Constraint", |ui| {
                for (l, ty) in [("Nail", "nail"), ("Pin", "pin"), ("Hinge", "hinge"), ("Spring", "spring"), ("Barrier", "barrier")] {
                    if ui.button(l).on_hover_text("Select one or two rigid bodies first").clicked() {
                        self.exec(&format!("rigidConstraint -type {}", ty));
                        ui.close_menu();
                    }
                }
            });
            ui.separator();
            for (l, c) in [("Set Active Key", "setRigidKey -active"), ("Set Passive Key", "setRigidKey -passive")] {
                if ui.button(l).on_hover_text("Keys the Active attribute at the current frame").clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
            if ui.button("Delete Rigid Body").clicked() {
                self.exec("rigidBody -delete");
                ui.close_menu();
            }
        });
        ui.menu_button("Particles", |ui| {
            for (l, c) in [("Create Emitter (omni)", "emitter -type omni -r 100"), ("Create Emitter (directional)", "emitter -type directional -r 100 -spd 3"), ("Create Emitter (volume)", "emitter -type volume -r 100"), ("Emit from Object (surface)", "emitFromObject -type surface -r 100"), ("Emit from Object (curve)", "emitFromObject -type curve -r 100")] {
                if ui.button(l).clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui.button("Make Collide").on_hover_text("Select the particles, then the geometry last").clicked() {
                self.exec("makeCollide");
                ui.close_menu();
            }
            if ui.button("Particle Collision Event Editor...").clicked() {
                self.event_ui.show = true;
                ui.close_menu();
            }
            ui.menu_button("Particle Render Type", |ui| {
                for (l, t) in [("Points", "points"), ("Spheres", "spheres"), ("Blobby Surface (software)", "blobby"), ("Sprites (hardware)", "sprites"), ("Streaks (hardware)", "streaks")] {
                    if ui.button(l).clicked() {
                        let sel: Vec<String> = self.session.scene.selection.iter().map(|i| self.session.scene.name_of(*i)).collect();
                        if let Some(p) = sel.first() {
                            self.exec(&format!("particle {} -type {}", p, t));
                        }
                        ui.close_menu();
                    }
                }
            });
        });
        ui.menu_button("Effects", |ui| {
            for (l, c) in [("Create Smoke", "particleEffect -type smoke"), ("Create Fire", "particleEffect -type fire")] {
                if ui.button(l).on_hover_text("Uses the selected curve or surface as the emitter").clicked() {
                    self.exec(c);
                    ui.close_menu();
                }
            }
        });
        ui.menu_button("Solvers", |ui| {
            if ui.button("Rigid Body Solver Attributes").clicked() {
                let id = self.session.scene.ensure_dyn_solver();
                self.session.scene.selection = vec![id];
                self.show_ae = true;
                self.mark_dirty();
                ui.close_menu();
            }
            ui.menu_button("Initial State", |ui| {
                for (l, c) in [("Set for Selected", "dynInitialState"), ("Set for All Dynamic", "dynInitialState -all"), ("Clear", "dynInitialState -clear")] {
                    if ui.button(l).clicked() {
                        self.exec(c);
                        ui.close_menu();
                    }
                }
            });
            if ui.button("Cache Data On").clicked() {
                self.exec("rigidSolver -cache 1");
                ui.close_menu();
            }
            if ui.button("Cache Data Off").clicked() {
                self.exec("rigidSolver -cache 0");
                ui.close_menu();
            }
            if ui.button("Delete Cache").clicked() {
                self.exec("rigidSolver -deleteCache");
                ui.close_menu();
            }
        });
    }

    /// Particle Collision Event Editor: the selected particles emit new particles when they hit something.
    fn particle_event_window(&mut self, ctx: &egui::Context) {
        if !self.event_ui.show {
            return;
        }
        let mut open = true;
        let mut cmd: Option<String> = None;
        let sel: Vec<String> = self.session.scene.selection.iter().map(|i| self.session.scene.name_of(*i)).collect();
        egui::Window::new("Particle Collision Event Editor").open(&mut open).default_width(320.0).show(ctx, |ui| {
            ui.label(format!("Particles: {}", sel.first().cloned().unwrap_or_else(|| "(select particles)".to_string())));
            ui.horizontal(|ui| {
                ui.label("Target particles");
                ui.add(egui::TextEdit::singleline(&mut self.event_ui.target).desired_width(140.0).hint_text("splash"));
            });
            ui.add(egui::Slider::new(&mut self.event_ui.num, 1..=20).text("Num particles"));
            ui.add(egui::Slider::new(&mut self.event_ui.spread, 0.0..=3.0).text("Spread"));
            ui.add(egui::Slider::new(&mut self.event_ui.inherit, 0.0..=2.0).text("Inherit velocity"));
            ui.checkbox(&mut self.event_ui.dies, "Original particle dies");
            ui.horizontal(|ui| {
                if ui.button("Create Event").clicked() {
                    if let Some(p) = sel.first() {
                        cmd = Some(format!("particleEvent -p {} -target {} -num {} -spread {} -inherit {} -dies {}", p, if self.event_ui.target.is_empty() { p.clone() } else { self.event_ui.target.clone() }, self.event_ui.num, self.event_ui.spread, self.event_ui.inherit, self.event_ui.dies as u8));
                    }
                }
                if ui.button("Delete Events").clicked() {
                    if let Some(p) = sel.first() {
                        cmd = Some(format!("particleEvent -p {} -clear", p));
                    }
                }
            });
        });
        if !open {
            self.event_ui.show = false;
        }
        if let Some(c) = cmd {
            self.exec(&c);
        }
    }
}

#[derive(Default)]
struct TraxUi {
    show: bool,
    selected: Vec<String>,
    /// Clip being dragged and the grab offset in frames.
    drag: Option<(String, f32)>,
    view_start: f32,
    view_end: f32,
    file: String,
    note: String,
    new_name: String,
}

impl App {
    /// The Trax Editor: tracks of clips per character, with the clip tools.
    fn trax_window(&mut self, ctx: &egui::Context) {
        if !self.trax_ui.show {
            return;
        }
        if self.trax_ui.view_end <= self.trax_ui.view_start {
            let tl = &self.session.scene.timeline;
            self.trax_ui.view_start = tl.start - 5.0;
            self.trax_ui.view_end = tl.end + 5.0;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let now = self.session.scene.timeline.current;
        let character = self.session.current_character.map(|c| self.session.scene.name_of(c));
        let clips = self.session.scene.trax.clips.clone();
        let sources: Vec<String> = self.session.scene.trax.sources.iter().map(|s| s.name.clone()).collect();
        let sounds: Vec<(NodeId, String, f32)> = self.session.scene.ids_where(|d| matches!(d, NodeData::Sound(_))).into_iter().map(|i| (i, self.session.scene.name_of(i), self.session.scene.node(i).and_then(|n| n.plug("offset")).map(|p| p.value).unwrap_or(1.0))).collect();
        egui::Window::new("Trax Editor").open(&mut open).default_size([760.0, 420.0]).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Character: {}", character.clone().unwrap_or_else(|| "(none: choose Character > current)".to_string())));
                ui.add(egui::TextEdit::singleline(&mut self.trax_ui.new_name).desired_width(90.0).hint_text("clip name"));
                if ui.button("Create Clip").on_hover_text("The keys of the current character become a clip").clicked() && !self.trax_ui.new_name.trim().is_empty() {
                    cmds.push(format!("clip -create {}", self.trax_ui.new_name.trim()));
                    self.trax_ui.new_name.clear();
                }
                egui::ComboBox::from_id_salt("traxlib").selected_text("Library: Insert Clip").show_ui(ui, |ui| {
                    for s in &sources {
                        if ui.selectable_label(false, s).clicked() {
                            cmds.push(format!("clipInsert {} -start {}", s, now));
                        }
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                let sel = self.trax_ui.selected.clone();
                let one = sel.first().cloned();
                let two = if sel.len() == 2 { Some((sel[0].clone(), sel[1].clone())) } else { None };
                if ui.add_enabled(one.is_some(), egui::Button::new("Trim Before")).on_hover_text("At the current frame").clicked() {
                    cmds.push(format!("trimClip -before {} {}", now, one.clone().unwrap()));
                }
                if ui.add_enabled(one.is_some(), egui::Button::new("Trim After")).clicked() {
                    cmds.push(format!("trimClip -after {} {}", now, one.clone().unwrap()));
                }
                if ui.add_enabled(one.is_some(), egui::Button::new("Split")).clicked() {
                    cmds.push(format!("splitClip -at {} {}", now, one.clone().unwrap()));
                }
                if ui.add_enabled(two.is_some(), egui::Button::new("Blend")).on_hover_text("Select two clips: the first fades into the second").clicked() {
                    let (a, b) = two.clone().unwrap();
                    cmds.push(format!("blendClips -shape easeInOut {} {}", a, b));
                }
                if ui.add_enabled(sel.len() >= 2, egui::Button::new("Merge")).clicked() {
                    cmds.push(format!("mergeClips -name merged {}", sel.join(" ")));
                }
                if ui.add_enabled(one.is_some(), egui::Button::new("Time Warp")).clicked() {
                    cmds.push(format!("clipTimeWarp {} -create", one.clone().unwrap()));
                }
                if ui.add_enabled(one.is_some(), egui::Button::new("Activate Keys")).on_hover_text("Put the keys of the clip back in the timeline and remove the clip").clicked() {
                    cmds.push(format!("clip -activate {}", one.clone().unwrap()));
                }
                if ui.add_enabled(one.is_some(), egui::Button::new("Delete")).clicked() {
                    for c in &sel {
                        cmds.push(format!("clip -delete {}", c));
                    }
                    self.trax_ui.selected.clear();
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.trax_ui.file).desired_width(220.0).hint_text("clip file (project clips folder)"));
                if !clips.is_empty() && ui.button("Set Range To Enabled Clips").on_hover_text("Time Slider range covers every clip").clicked() {
                    let lo = clips.iter().map(|c| c.start).fold(f32::MAX, f32::min);
                    let hi = clips.iter().map(|c| c.end()).fold(f32::MIN, f32::max);
                    cmds.push(format!("playbackOptions -min {} -max {}", lo.floor(), hi.ceil()));
                }
                if ui.button("Import Animation").clicked() {
                    cmds.push(format!("clipImport {} -insert -start {}", Self::quote(&self.trax_ui.file), now));
                }
                if ui.button("Export Source").clicked() {
                    if let Some(c) = clips.iter().find(|c| self.trax_ui.selected.contains(&c.name)) {
                        cmds.push(format!("clipExport {} {}", c.source, Self::quote(&self.trax_ui.file)));
                    }
                }
            });
            ui.separator();
            // the timeline of tracks
            let tracks = clips.iter().map(|c| c.track + 1).max().unwrap_or(1).max(2);
            let height = 24.0 * (tracks as f32 + sounds.len() as f32) + 26.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(300.0), height.max(120.0)), Sense::click_and_drag());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 0.0, FIELD);
            let (v0, v1) = (self.trax_ui.view_start, self.trax_ui.view_end);
            let x_of = |f: f32| rect.left() + 6.0 + (f - v0) / (v1 - v0).max(1.0) * (rect.width() - 12.0);
            let f_of = |x: f32| v0 + (x - rect.left() - 6.0) / (rect.width() - 12.0).max(1.0) * (v1 - v0);
            let step = [1.0f32, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0].iter().copied().find(|s| s * (rect.width() / (v1 - v0).max(1.0)) >= 38.0).unwrap_or(100.0);
            let mut f = (v0 / step).ceil() * step;
            while f <= v1 {
                let x = x_of(f);
                p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, Color32::from_gray(0x44)));
                p.text(pos2(x + 2.0, rect.top() + 2.0), Align2::LEFT_TOP, format!("{}", f), FontId::proportional(10.0), TEXT_DIM);
                f += step;
            }
            let row_y = |track: usize| rect.top() + 18.0 + track as f32 * 24.0;
            for (i, c) in clips.iter().enumerate() {
                let r = Rect::from_min_max(pos2(x_of(c.start), row_y(c.track)), pos2(x_of(c.end()).max(x_of(c.start) + 6.0), row_y(c.track) + 20.0));
                let sel = self.trax_ui.selected.contains(&c.name);
                let col = if !c.enabled { Color32::from_gray(0x60) } else if c.relative { Color32::from_rgb(0x58, 0x90, 0xc8) } else { Color32::from_rgb(0x70, 0x9a, 0x58) };
                p.rect_filled(r, 3.0, col);
                p.rect_stroke(r, 3.0, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { Color32::WHITE } else { Color32::BLACK }), egui::StrokeKind::Inside);
                p.text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, &c.name, FontId::proportional(11.0), Color32::BLACK);
                if c.warp.is_some() {
                    p.text(r.right_center() - vec2(4.0, 0.0), Align2::RIGHT_CENTER, "~", FontId::proportional(11.0), Color32::BLACK);
                }
                let _ = i;
            }
            // blends as a ramp over the overlap
            for b in &self.session.scene.trax.blends {
                if let (Some(a), Some(bb)) = (clips.iter().find(|c| c.name == b.a), clips.iter().find(|c| c.name == b.b)) {
                    let r = Rect::from_min_max(pos2(x_of(bb.start), row_y(a.track)), pos2(x_of(a.end()).max(x_of(bb.start) + 2.0), row_y(a.track) + 20.0));
                    p.rect_stroke(r, 0.0, Stroke::new(1.5, Color32::from_rgb(0xe0, 0xc0, 0x40)), egui::StrokeKind::Inside);
                    p.line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.0, Color32::from_rgb(0xe0, 0xc0, 0x40)));
                }
            }
            // the soundtrack
            for (k, (_, name, off)) in sounds.iter().enumerate() {
                let y = row_y(tracks + k);
                let r = Rect::from_min_max(pos2(x_of(*off), y), pos2(x_of(*off) + 60.0, y + 20.0));
                p.rect_filled(r, 3.0, Color32::from_rgb(0x40, 0xa0, 0x70));
                p.text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, format!("\u{266a} {}", name), FontId::proportional(11.0), Color32::BLACK);
            }
            // the time indicator
            let xt = x_of(now);
            p.line_segment([pos2(xt, rect.top()), pos2(xt, rect.bottom())], Stroke::new(1.5, Color32::from_rgb(0xff, 0x80, 0x40)));
            // mouse: select, drag clips, scrub on the ruler
            if let Some(pos) = resp.interact_pointer_pos() {
                let frame = f_of(pos.x);
                if resp.drag_started() || resp.clicked() {
                    let hit = clips.iter().find(|c| pos.y >= row_y(c.track) && pos.y <= row_y(c.track) + 20.0 && frame >= c.start && frame <= c.end());
                    match hit {
                        Some(c) => {
                            let add = ui.input(|i| i.modifiers.shift);
                            if add {
                                if !self.trax_ui.selected.contains(&c.name) {
                                    self.trax_ui.selected.push(c.name.clone());
                                }
                            } else if !self.trax_ui.selected.contains(&c.name) {
                                self.trax_ui.selected = vec![c.name.clone()];
                            }
                            if resp.drag_started() {
                                self.trax_ui.drag = Some((c.name.clone(), frame - c.start));
                            }
                        }
                        None => {
                            if pos.y < rect.top() + 18.0 {
                                cmds.push(format!("currentTime {}", frame.round()));
                            } else if !ui.input(|i| i.modifiers.shift) {
                                self.trax_ui.selected.clear();
                            }
                        }
                    }
                }
                if resp.dragged() {
                    if let Some((name, grab)) = &self.trax_ui.drag {
                        let start = (frame - grab).round();
                        if let Some(c) = clips.iter().find(|c| &c.name == name) {
                            if (c.start - start).abs() >= 1.0 {
                                cmds.push(format!("clip -edit {} -start {}", name, start));
                            }
                        }
                    } else if pos.y < rect.top() + 18.0 {
                        cmds.push(format!("currentTime {}", frame.round()));
                    }
                }
            }
            if resp.drag_stopped() {
                self.trax_ui.drag = None;
            }
            // attributes of the selected clip
            if let Some(c) = clips.iter().find(|c| self.trax_ui.selected.first() == Some(&c.name)) {
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(&c.name).strong());
                    ui.label(format!("source {}  frames {}..{}", c.source, c.start, c.end()));
                    let mut rel = c.relative;
                    if ui.checkbox(&mut rel, "Relative offsets").on_hover_text("Continue from the end of the clip before instead of the keyed values").changed() {
                        cmds.push(format!("clip -edit {} -relative {}", c.name, rel as u8));
                    }
                    let mut en = c.enabled;
                    if ui.checkbox(&mut en, "Enabled").changed() {
                        cmds.push(format!("clip -edit {} -enable {}", c.name, en as u8));
                    }
                    let (mut w, mut sc, mut cy) = (c.weight, c.scale, c.cycles);
                    ui.add(egui::DragValue::new(&mut w).speed(0.01).range(0.0..=1.0).prefix("weight "));
                    ui.add(egui::DragValue::new(&mut sc).speed(0.02).range(0.05..=20.0).prefix("scale "));
                    ui.add(egui::DragValue::new(&mut cy).speed(0.05).range(1.0..=50.0).prefix("cycles "));
                    if (w - c.weight).abs() > 1e-6 {
                        cmds.push(format!("clip -edit {} -weight {}", c.name, w));
                    }
                    if (sc - c.scale).abs() > 1e-6 {
                        cmds.push(format!("clip -edit {} -scale {}", c.name, sc));
                    }
                    if (cy - c.cycles).abs() > 1e-6 {
                        cmds.push(format!("clip -edit {} -cycles {}", c.name, cy));
                    }
                });
            }
            ui.label(egui::RichText::new(&self.trax_ui.note).color(TEXT_DIM));
        });
        if !open {
            self.trax_ui.show = false;
        }
        for c in cmds {
            self.exec(&c);
            self.scene_edited();
        }
    }
}

impl App {
    /// Frame numbers along motion paths: every key of the U value is a marker.
    fn draw_path_markers(&mut self, painter: &egui::Painter, cam: &Cam, rect: Rect) {
        let mps = self.session.scene.ids_where(|d| matches!(d, NodeData::MotionPath(_)));
        if mps.is_empty() {
            return;
        }
        let vp = cam.view_proj();
        for mp in mps {
            let curve_shape = match self.session.scene.node(mp).map(|n| &n.data) {
                Some(NodeData::MotionPath(d)) => d.curve,
                _ => None,
            };
            let Some(cs) = curve_shape else { continue };
            let Some((curve, _)) = self.session.scene.input_curve_world(cs) else { continue };
            let frac = self.session.scene.get_attr(mp, "fractionMode").unwrap_or(0.0) > 0.5;
            let (a, b) = curve.dir.domain();
            let keys: Vec<(f32, f32)> = self.session.scene.anim.bindings.iter().filter(|x| x.node == mp && x.attr == "uValue" && x.layer == 0).flat_map(|x| x.curve.keys.iter().map(|k| (k.t, k.v))).collect();
            for (frame, u) in keys {
                let param = if frac { a + (b - a) * u } else { u }.clamp(a, b);
                let w = curve.point(param);
                let c = vp * w.extend(1.0);
                if c.w <= 1e-4 {
                    continue;
                }
                let sp = pos2(rect.min.x + (c.x / c.w * 0.5 + 0.5) * rect.width(), rect.min.y + (1.0 - (c.y / c.w * 0.5 + 0.5)) * rect.height());
                if !rect.contains(sp) {
                    continue;
                }
                painter.circle_filled(sp, 3.0, Color32::from_rgb(0xff, 0xd0, 0x40));
                painter.text(sp + vec2(5.0, -5.0), Align2::LEFT_BOTTOM, format!("{}", frame.round()), FontId::proportional(11.0), Color32::from_rgb(0xff, 0xe0, 0x80));
            }
        }
    }
}

#[derive(Default)]
struct SculptUi {
    show: bool,
}

impl App {
    /// A dab of the Sculpt Geometry Tool where the pointer meets the selected surface.
    fn sculpt_at(&mut self, cam: &Cam, rect: Rect, pos: Pos2, first: bool) {
        let Some(shape) = cmd_model::current_shape(&self.session).or_else(|| self.session.scene.selection.iter().find_map(|i| self.session.scene.mesh_shapes_under(*i).first().copied())) else {
            self.say("Select the surface to sculpt first");
            return;
        };
        let (nx, ny) = ((pos.x - rect.min.x) / rect.width(), (pos.y - rect.min.y) / rect.height());
        let inv = cam.view_proj().inverse();
        let un = |z: f32| {
            let c = inv * Vec3::new(nx * 2.0 - 1.0, 1.0 - ny * 2.0, z).extend(1.0);
            Vec3::new(c.x / c.w, c.y / c.w, c.z / c.w)
        };
        let (near, far) = (un(-1.0), un(1.0));
        let dir = (far - near).normalized();
        let Some((hit, _)) = crate::mel::cmd_paintfx::surface_hit(&mut self.session, shape, near, dir) else { return };
        let mut st = std::mem::take(&mut self.session.sculpt);
        if first {
            let _ = st.begin_stroke(&self.session, shape);
        }
        let r = st.dab(&mut self.session, shape, hit, false);
        self.session.sculpt = st;
        match r {
            Ok(_) => {
                self.session.scene.flush_meshes();
                self.mark_dirty();
                self.scene_edited();
            }
            Err(e) => self.say(&e),
        }
    }

    /// Tool Settings of the Sculpt Geometry Tool.
    fn sculpt_window(&mut self, ctx: &egui::Context) {
        if !self.sculpt_ui.show {
            return;
        }
        let mut open = true;
        let mut cmds: Vec<String> = Vec::new();
        let (mut op, mut radius, mut opacity, mut md, mut ss, mut ur) = {
            let s = &self.session.sculpt;
            (s.op.clone(), s.radius, s.opacity, s.max_disp, s.smooth_strength as f32, s.update_ref)
        };
        egui::Window::new("Sculpt Geometry Tool").open(&mut open).default_width(320.0).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for o in crate::mel::cmd_sculpt::OPS {
                    if ui.selectable_label(op == *o, *o).clicked() {
                        op = o.to_string();
                    }
                }
            });
            ui.add(egui::Slider::new(&mut radius, 0.01..=10.0).logarithmic(true).text("Radius"));
            ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("Opacity"));
            ui.add(egui::Slider::new(&mut md, 0.0..=2.0).text("Max displacement"));
            ui.add(egui::Slider::new(&mut ss, 1.0..=5.0).step_by(1.0).text("Smooth strength"));
            ui.checkbox(&mut ur, "Reference surface: update on each stroke");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Flood").clicked() {
                    cmds.push("sculpt -flood".to_string());
                }
                if ui.button("Update Reference").clicked() {
                    cmds.push("sculpt -updateReference".to_string());
                }
                if ui.button("Update Erase Surface").clicked() {
                    cmds.push("sculpt -updateEraseSurface".to_string());
                }
                if ui.button("Reset Tool").clicked() {
                    cmds.push("sculpt -reset".to_string());
                }
                if ui.button("Use the tool").clicked() {
                    self.start_tool(Tool::Sculpt);
                }
            });
        });
        {
            let s = &mut self.session.sculpt;
            s.op = op;
            s.radius = radius;
            s.opacity = opacity;
            s.max_disp = md;
            s.smooth_strength = ss as u32;
            s.update_ref = ur;
        }
        if !open {
            self.sculpt_ui.show = false;
        }
        for c in cmds {
            self.exec(&c);
        }
    }
}
