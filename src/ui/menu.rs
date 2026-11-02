//! Menu bar (File, Edit, Create, Display, Modify, Render) with dropdowns.

use super::screen::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Special {
    Exit,
    ToggleGrid,
    FrameAll,
    NextMode,
    SetMode(&'static str),
    KeySelected,
    CutKeySelected,
    RenderBeauty,
    RenderAll,
    RenderAnaglyph,
    Help,
    ToggleShell,
    ResetTransform,
    PlayPause,
    SelectAll,
    Deselect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Cmd(&'static str),
    Special(Special),
}

pub struct Item {
    pub label: &'static str,
    pub hint: &'static str,
    pub action: Action,
}

pub struct MenuDef {
    pub title: &'static str,
    pub alt: char,
    pub items: &'static [Item],
}

macro_rules! item {
    ($l:expr, $h:expr, $a:expr) => {
        Item { label: $l, hint: $h, action: $a }
    };
}

pub static MENUS: &[MenuDef] = &[
    MenuDef {
        title: "File",
        alt: 'f',
        items: &[
            item!("New Scene", "", Action::Cmd("file -new")),
            item!("Open Scene", "scene.aztec", Action::Cmd("file -open \"scene.aztec\"")),
            item!("Save Scene", "", Action::Cmd("file -save")),
            item!("Save Scene As scene.aztec", "", Action::Cmd("file -save \"scene.aztec\"")),
            item!("Import OBJ/FBX", "model.fbx", Action::Cmd("file -import \"model.fbx\"")),
            item!("Export All", "model.fbx", Action::Cmd("file -export \"model.fbx\"")),
            item!("Export Selection", "model.obj", Action::Cmd("file -export \"model.obj\" -selected")),
            item!("Run examples/demo.mel", "", Action::Cmd("source \"examples/demo.mel\"")),
            item!("Exit", "Ctrl+Q", Action::Special(Special::Exit)),
        ],
    },
    MenuDef {
        title: "Edit",
        alt: 'e',
        items: &[
            item!("Select All", "", Action::Special(Special::SelectAll)),
            item!("Deselect", "", Action::Special(Special::Deselect)),
            item!("Duplicate", "", Action::Cmd("duplicate")),
            item!("Group", "", Action::Cmd("group")),
            item!("Delete", "Del", Action::Cmd("delete")),
            item!("Reset Transform", "", Action::Special(Special::ResetTransform)),
        ],
    },
    MenuDef {
        title: "Create",
        alt: 'c',
        items: &[
            item!("Polygon Cube", "", Action::Cmd("polyCube")),
            item!("Polygon Sphere", "", Action::Cmd("polySphere")),
            item!("Polygon Cylinder", "", Action::Cmd("polyCylinder")),
            item!("Polygon Plane", "", Action::Cmd("polyPlane")),
            item!("Camera", "", Action::Cmd("camera")),
            item!("Stereo Camera", "", Action::Cmd("stereoCamera")),
            item!("Point Light", "", Action::Cmd("pointLight")),
            item!("Directional Light", "", Action::Cmd("directionalLight")),
            item!("Material (Lambert)", "", Action::Cmd("lambert")),
            item!("Nucleus + nParticles", "", Action::Cmd("nParticle")),
            item!("Animation Layer", "", Action::Cmd("animLayer layer")),
        ],
    },
    MenuDef {
        title: "Display",
        alt: 'd',
        items: &[
            item!("Shaded", "1", Action::Special(Special::SetMode("shaded"))),
            item!("Wireframe", "2", Action::Special(Special::SetMode("wireframe"))),
            item!("Anaglyph (red/cyan)", "3", Action::Special(Special::SetMode("anaglyph"))),
            item!("Cycle Display Mode", "m", Action::Special(Special::NextMode)),
            item!("Toggle Grid", "g", Action::Special(Special::ToggleGrid)),
            item!("Frame All", "f", Action::Special(Special::FrameAll)),
            item!("Toggle Shell Size", "F2", Action::Special(Special::ToggleShell)),
            item!("Key Bindings", "F1", Action::Special(Special::Help)),
        ],
    },
    MenuDef {
        title: "Modify",
        alt: 'm',
        items: &[
            item!("Set Key (selected channels)", "k", Action::Special(Special::KeySelected)),
            item!("Cut Key at current time", "", Action::Special(Special::CutKeySelected)),
            item!("Play / Pause", "Space", Action::Special(Special::PlayPause)),
            item!("Rewind", "Home", Action::Cmd("rewind")),
            item!("Next Frame", ".", Action::Cmd("nextFrame")),
            item!("Previous Frame", ",", Action::Cmd("prevFrame")),
            item!("Scene Statistics", "", Action::Cmd("stats")),
        ],
    },
    MenuDef {
        title: "Render",
        alt: 'r',
        items: &[
            item!("Render Beauty (ASCII)", "", Action::Special(Special::RenderBeauty)),
            item!("Render All Passes", "", Action::Special(Special::RenderAll)),
            item!("Render Anaglyph", "", Action::Special(Special::RenderAnaglyph)),
        ],
    },
];

#[derive(Default)]
pub struct MenuState {
    pub open: Option<usize>,
    pub hover: usize,
    /// Screen rectangles of the titles on the bar, rebuilt on every draw.
    pub titles: Vec<Rect>,
    pub drop: Option<Rect>,
}

impl MenuState {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn open_menu(&mut self, i: usize) {
        self.open = Some(i % MENUS.len());
        self.hover = 0;
    }

    pub fn close(&mut self) {
        self.open = None;
        self.drop = None;
    }

    pub fn move_title(&mut self, d: i32) {
        if let Some(i) = self.open {
            let n = MENUS.len() as i32;
            self.open = Some(((i as i32 + d).rem_euclid(n)) as usize);
            self.hover = 0;
        }
    }

    pub fn move_item(&mut self, d: i32) {
        if let Some(i) = self.open {
            let n = MENUS[i].items.len() as i32;
            self.hover = ((self.hover as i32 + d).rem_euclid(n)) as usize;
        }
    }

    pub fn current(&self) -> Option<Action> {
        self.open.map(|i| MENUS[i].items[self.hover.min(MENUS[i].items.len() - 1)].action)
    }

    pub fn title_at(&self, x: usize, y: usize) -> Option<usize> {
        self.titles.iter().position(|r| r.contains(x, y))
    }

    /// Item index under the pointer inside the open dropdown.
    pub fn item_at(&self, x: usize, y: usize) -> Option<usize> {
        let r = self.drop?;
        let i = self.open?;
        if x > r.x && x + 1 < r.x + r.w && y > r.y && y + 1 < r.y + r.h {
            let k = y - r.y - 1;
            if k < MENUS[i].items.len() {
                return Some(k);
            }
        }
        None
    }

    pub fn in_dropdown(&self, x: usize, y: usize) -> bool {
        self.drop.map(|r| r.contains(x, y)).unwrap_or(false)
    }

    pub fn draw_bar(&mut self, screen: &mut Screen, width: usize, status: &str) {
        let bar = Rect::new(0, 0, width, 1);
        screen.fill(bar, ' ', TEXT, TITLE_BG);
        self.titles.clear();
        let mut x = 1;
        for (i, m) in MENUS.iter().enumerate() {
            let label = format!(" {} ", m.title);
            let w = label.chars().count();
            let active = self.open == Some(i);
            let bg = if active { SEL_BG } else { TITLE_BG };
            screen.fill(Rect::new(x, 0, w, 1), ' ', TEXT, bg);
            screen.text(x, 0, &label, TEXT, bg, x + w);
            // underline hint: highlight the accelerator letter
            screen.set(x + 1, 0, Cell { ch: m.title.chars().next().unwrap_or(' '), fg: YELLOW, bg, bold: true });
            self.titles.push(Rect::new(x, 0, w, 1));
            x += w;
        }
        let brand = "Aztec 2009";
        if width > x + brand.len() + status.len() + 6 {
            screen.text(x + 2, 0, status, TEXT_DIM, TITLE_BG, width);
            screen.text_bold(width - brand.len() - 1, 0, brand, TEXT, TITLE_BG, width);
        } else if width > x + brand.len() + 2 {
            screen.text_bold(width - brand.len() - 1, 0, brand, TEXT, TITLE_BG, width);
        }
    }

    pub fn draw_dropdown(&mut self, screen: &mut Screen) {
        self.drop = None;
        let i = match self.open {
            Some(i) => i,
            None => return,
        };
        let m = &MENUS[i];
        let tr = match self.titles.get(i) {
            Some(t) => *t,
            None => return,
        };
        let label_w = m.items.iter().map(|it| it.label.chars().count()).max().unwrap_or(8);
        let hint_w = m.items.iter().map(|it| it.hint.chars().count()).max().unwrap_or(0);
        let inner = label_w + if hint_w > 0 { hint_w + 3 } else { 0 };
        let w = inner + 4;
        let h = m.items.len() + 2;
        let x = tr.x.min(screen.w.saturating_sub(w));
        let r = Rect::new(x, 1, w.min(screen.w), h.min(screen.h.saturating_sub(1)));
        screen.draw_box(r, TEXT_DIM, PANEL_BG, None);
        for (k, it) in m.items.iter().enumerate() {
            let y = r.y + 1 + k;
            if y + 1 >= r.y + r.h {
                break;
            }
            let bg = if k == self.hover { SEL_BG } else { PANEL_BG };
            screen.fill(Rect::new(r.x + 1, y, r.w - 2, 1), ' ', TEXT, bg);
            screen.text(r.x + 2, y, it.label, TEXT, bg, r.x + r.w - 1);
            if !it.hint.is_empty() {
                let hx = r.x + r.w - 2 - it.hint.chars().count();
                screen.text(hx, y, it.hint, TEXT_DIM.mix(TEXT, 0.3), bg, r.x + r.w - 1);
            }
        }
        self.drop = Some(r);
    }
}

/// Find the menu whose Alt accelerator matches `c`.
pub fn menu_for_alt(c: char) -> Option<usize> {
    let c = c.to_ascii_lowercase();
    MENUS.iter().position(|m| m.alt == c)
}
