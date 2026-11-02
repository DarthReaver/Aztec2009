//! Custom user interface built from scripts: windows, layouts and controls (window, columnLayout, intSliderGrp, button, ...),
//! custom shelf buttons and marking menus. The windowed workspace draws what is registered here.

use super::commands::{execute_script, fmt_f, Out};
use super::lexer::Token;
use super::parser::{parse_args, FlagSpec, Parsed};
use crate::session::Session;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Window,
    Column,
    Row,
    Frame,
    Scroll,
    Text,
    Button,
    Check,
    IntField,
    FloatField,
    TextField,
    IntSlider,
    FloatSlider,
    AttrSlider,
    Separator,
    OptionMenu,
    MenuItem,
}

impl Kind {
    pub fn is_layout(self) -> bool {
        matches!(self, Kind::Window | Kind::Column | Kind::Row | Kind::Frame | Kind::Scroll)
    }
    fn base_name(self) -> &'static str {
        match self {
            Kind::Window => "window",
            Kind::Column => "columnLayout",
            Kind::Row => "rowLayout",
            Kind::Frame => "frameLayout",
            Kind::Scroll => "scrollLayout",
            Kind::Text => "text",
            Kind::Button => "button",
            Kind::Check => "checkBox",
            Kind::IntField => "intField",
            Kind::FloatField => "floatField",
            Kind::TextField => "textField",
            Kind::IntSlider => "intSliderGrp",
            Kind::FloatSlider => "floatSliderGrp",
            Kind::AttrSlider => "attrFieldSliderGrp",
            Kind::Separator => "separator",
            Kind::OptionMenu => "optionMenu",
            Kind::MenuItem => "menuItem",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Widget {
    pub kind: Kind,
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub label: String,
    pub text: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub field_min: f64,
    pub field_max: f64,
    pub field: bool,
    pub command: String,
    pub on_command: String,
    pub off_command: String,
    pub change_command: String,
    pub enabled: bool,
    pub visible: bool,
    pub width: f32,
    pub height: f32,
    pub row_spacing: f32,
    pub columns: usize,
    pub collapsed: bool,
    pub collapsable: bool,
    pub attribute: String,
    pub annotation: String,
    /// Windows: shown with showWindow.
    pub shown: bool,
}

#[derive(Clone, Debug, Default)]
pub struct UiState {
    pub widgets: Vec<Option<Widget>>,
    /// The layout new controls go into (setParent changes it).
    pub parent: Option<usize>,
    counters: Vec<(String, usize)>,
}

impl UiState {
    pub fn get(&self, id: usize) -> Option<&Widget> {
        self.widgets.get(id).and_then(|w| w.as_ref())
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut Widget> {
        self.widgets.get_mut(id).and_then(|w| w.as_mut())
    }

    pub fn path(&self, id: usize) -> String {
        match self.get(id) {
            Some(w) => match w.parent {
                Some(p) => format!("{}|{}", self.path(p), w.name),
                None => w.name.clone(),
            },
            None => String::new(),
        }
    }

    pub fn ids(&self) -> Vec<usize> {
        self.widgets.iter().enumerate().filter(|(_, w)| w.is_some()).map(|(i, _)| i).collect()
    }

    pub fn windows(&self) -> Vec<usize> {
        self.ids().into_iter().filter(|i| self.get(*i).map(|w| w.kind == Kind::Window).unwrap_or(false)).collect()
    }

    /// A control by full path, by the tail of its path, or by its own name.
    pub fn find(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        let ids = self.ids();
        if let Some(i) = ids.iter().rev().find(|i| self.path(**i) == name) {
            return Some(*i);
        }
        if name.contains('|') {
            return ids.iter().rev().find(|i| {
                let p = self.path(**i);
                p.ends_with(&format!("|{}", name)) || p.ends_with(name) && p.len() > name.len() && p.as_bytes()[p.len() - name.len() - 1] == b'|'
            }).copied();
        }
        ids.iter().rev().find(|i| self.get(**i).map(|w| w.name == name).unwrap_or(false)).copied()
    }

    fn unique_child_name(&mut self, parent: Option<usize>, base: &str) -> String {
        let taken = |s: &UiState, n: &str| s.widgets.iter().flatten().any(|w| w.parent == parent && w.name == n);
        if !taken(self, base) {
            return base.to_string();
        }
        let mut i = 1;
        loop {
            let n = format!("{}{}", base, i);
            if !taken(self, &n) {
                return n;
            }
            i += 1;
        }
    }

    pub fn add(&mut self, kind: Kind, name: Option<String>, parent: Option<usize>) -> Result<usize, String> {
        let name = match name {
            Some(n) => {
                if self.widgets.iter().flatten().any(|w| w.parent == parent && w.name == n) {
                    return Err(format!("Object's name is not unique: {}", n));
                }
                n
            }
            None => {
                let c = match self.counters.iter_mut().find(|c| c.0 == kind.base_name()) {
                    Some(c) => {
                        c.1 += 1;
                        c.1
                    }
                    None => {
                        self.counters.push((kind.base_name().to_string(), 1));
                        1
                    }
                };
                self.unique_child_name(parent, &format!("{}{}", kind.base_name(), c))
            }
        };
        let w = Widget {
            kind,
            name,
            parent,
            children: Vec::new(),
            label: String::new(),
            text: String::new(),
            value: 0.0,
            min: 0.0,
            max: 100.0,
            field_min: f64::MIN,
            field_max: f64::MAX,
            field: false,
            command: String::new(),
            on_command: String::new(),
            off_command: String::new(),
            change_command: String::new(),
            enabled: true,
            visible: true,
            width: 0.0,
            height: 0.0,
            row_spacing: 4.0,
            columns: 1,
            collapsed: false,
            collapsable: false,
            attribute: String::new(),
            annotation: String::new(),
            shown: false,
        };
        let id = match self.widgets.iter().position(|w| w.is_none()) {
            Some(i) => {
                self.widgets[i] = Some(w);
                i
            }
            None => {
                self.widgets.push(Some(w));
                self.widgets.len() - 1
            }
        };
        if let Some(p) = parent {
            if let Some(pw) = self.get_mut(p) {
                pw.children.push(id);
            }
        }
        Ok(id)
    }

    pub fn delete(&mut self, id: usize) {
        let kids: Vec<usize> = self.get(id).map(|w| w.children.clone()).unwrap_or_default();
        for k in kids {
            self.delete(k);
        }
        if let Some(p) = self.get(id).and_then(|w| w.parent) {
            if let Some(pw) = self.get_mut(p) {
                pw.children.retain(|c| *c != id);
            }
        }
        if self.parent == Some(id) {
            self.parent = self.get(id).and_then(|w| w.parent);
        }
        if let Some(slot) = self.widgets.get_mut(id) {
            *slot = None;
        }
    }
}

// ---------------------------------------------------------------------------------------------- the commands

macro_rules! f {
    ($($n:expr),+ ; $a:expr) => { FlagSpec { names: &[$($n),+], arity: $a } };
}

/// Flags shared by the interface commands. `-columnWidth` and `-columnAttach` change arity per command.
fn base_spec(cmd: &str) -> Vec<FlagSpec> {
    let grp = cmd.ends_with("Grp");
    vec![
        f!("edit", "e"; 0),
        f!("query", "q"; 0),
        f!("exists", "ex"; 0),
        f!("label", "l"; 1),
        f!("width", "w"; 1),
        f!("height", "h"; 1),
        f!("enable", "en"; 1),
        f!("visible", "vis"; 1),
        f!("parent", "p"; 1),
        f!("annotation", "ann"; 1),
        f!("command", "c"; 1),
        f!("changeCommand", "cc"; 1),
        f!("dragCommand", "dc"; 1),
        f!("onCommand", "onc"; 1),
        f!("offCommand", "ofc"; 1),
        f!("value", "v"; 1),
        f!("minValue", "min"; 1),
        f!("maxValue", "max"; 1),
        f!("fieldMinValue", "fmn"; 1),
        f!("fieldMaxValue", "fmx"; 1),
        f!("field", "f"; 1),
        f!("text", "tx"; 1),
        f!("title", "t"; 1),
        f!("columnWidth", "cw"; if grp { 2 } else { 1 }),
        f!("columnAttach", "ca"; if grp { 3 } else { 2 }),
        f!("rowSpacing", "rs"; 1),
        f!("numberOfColumns", "nc"; 1),
        f!("columnAlign", "cal"; if grp { 2 } else { 1 }),
        f!("adjustableColumn", "adj"; 1),
        f!("collapsable", "cll"; 1),
        f!("collapse", "cl"; 1),
        f!("attribute", "at"; 1),
        f!("menuBar", "mb"; 1),
        f!("sizeable", "s"; 1),
        f!("step", "ss"; 1),
        f!("precision", "pre"; 1),
        f!("marginWidth", "mw"; 1),
        f!("marginHeight", "mh"; 1),
        f!("docTag", "dtg"; 1),
        f!("style", "st"; 1),
        f!("backgroundColor", "bgc"; 3),
        f!("enableBackground", "ebg"; 1),
        f!("numberOfFields", "nf"; 1),
        f!("borderVisible", "bv"; 1),
        f!("horizontalScrollBarThickness", "hst"; 1),
        f!("verticalScrollBarThickness", "vst"; 1),
        f!("childResizable", "cr"; 1),
        f!("noBackground", "nbg"; 1),
        f!("window", "win"; 0),
        f!("control", "ctl"; 0),
        f!("layout", "lay"; 0),
        f!("top", "top"; 0),
        f!("upLevel", "u"; 0),
    ]
}

fn kind_of(cmd: &str) -> Option<Kind> {
    Some(match cmd {
        "window" => Kind::Window,
        "columnLayout" | "formLayout" | "flowLayout" | "rowColumnLayout" | "tabLayout" | "paneLayout" => Kind::Column,
        "rowLayout" => Kind::Row,
        "frameLayout" => Kind::Frame,
        "scrollLayout" => Kind::Scroll,
        "text" => Kind::Text,
        "button" | "iconTextButton" | "symbolButton" => Kind::Button,
        "checkBox" => Kind::Check,
        "intField" => Kind::IntField,
        "floatField" => Kind::FloatField,
        "textField" => Kind::TextField,
        "intSliderGrp" | "intSlider" => Kind::IntSlider,
        "floatSliderGrp" | "floatSlider" => Kind::FloatSlider,
        "attrFieldSliderGrp" | "attrControlGrp" => Kind::AttrSlider,
        "separator" => Kind::Separator,
        "optionMenu" => Kind::OptionMenu,
        "menuItem" => Kind::MenuItem,
        _ => return None,
    })
}

pub const UI_COMMANDS: &[&str] = &[
    "window", "columnLayout", "formLayout", "flowLayout", "rowColumnLayout", "tabLayout", "paneLayout", "rowLayout", "frameLayout", "scrollLayout", "text", "button", "iconTextButton", "symbolButton", "checkBox", "intField", "floatField", "textField", "intSliderGrp", "intSlider", "floatSliderGrp", "floatSlider", "attrFieldSliderGrp", "attrControlGrp", "separator", "optionMenu", "menuItem", "showWindow", "deleteUI", "setParent", "invokeControl", "lsUI",
];

fn truthy(v: &str) -> bool {
    !matches!(v.to_lowercase().as_str(), "false" | "0" | "off" | "no")
}

fn fmt_value(w: &Widget) -> String {
    match w.kind {
        Kind::IntField | Kind::IntSlider => format!("{}", w.value.round() as i64),
        Kind::Check => format!("{}", (w.value > 0.5) as u8),
        Kind::TextField | Kind::Text => w.text.clone(),
        Kind::OptionMenu => w.text.clone(),
        _ => fmt_f(w.value as f32),
    }
}

/// The interface commands: create (`name` at the end), edit (`-e`), query (`-q`) and exists (`-ex`).
pub fn command(s: &mut Session, cmd: &str, rest: &[Token]) -> Out {
    match cmd {
        "showWindow" => return show_window(s, rest),
        "deleteUI" => return delete_ui(s, rest),
        "setParent" => return set_parent(s, rest),
        "invokeControl" => return invoke_control(s, rest),
        "lsUI" => {
            return Ok(s.ui.ids().into_iter().map(|i| format!("{} ({})", s.ui.path(i), s.ui.get(i).map(|w| w.kind.base_name()).unwrap_or(""))).collect());
        }
        _ => {}
    }
    let kind = kind_of(cmd).ok_or("Unknown interface command")?;
    let mut spec = base_spec(cmd);
    // a query reads one attribute: its flag takes no value
    if rest.iter().any(|t| matches!(t, Token::Flag(f) if f == "q" || f == "query")) {
        for f in spec.iter_mut() {
            f.arity = 0;
        }
    }
    let p = parse_args(rest, &spec)?;
    let name = p.args.last().map(|a| a.as_string());
    let exists = p.has("exists");
    if exists {
        let found = name.as_ref().and_then(|n| s.ui.find(n)).and_then(|i| s.ui.get(i)).map(|w| w.kind == kind || (kind != Kind::Window && w.kind != Kind::Window)).unwrap_or(false);
        return Ok(vec![(found as u8).to_string()]);
    }
    if p.has("query") {
        let n = name.ok_or("Name the control to query")?;
        let id = s.ui.find(&n).ok_or_else(|| format!("Object not found: {}", n))?;
        return query(s, id, &p);
    }
    if p.has("edit") {
        let n = name.ok_or("Name the control to edit")?;
        let id = s.ui.find(&n).ok_or_else(|| format!("Object not found: {}", n))?;
        apply_flags(s, id, &p)?;
        return Ok(vec![]);
    }
    // create
    let parent = if kind == Kind::Window {
        None
    } else {
        match p.string("parent") {
            Some(pp) => Some(s.ui.find(&pp).ok_or_else(|| format!("Object not found: {}", pp))?),
            None => s.ui.parent,
        }
    };
    if kind != Kind::Window && parent.is_none() && !matches!(kind, Kind::MenuItem) {
        return Err(format!("There is no window to put the {} in: create a window first", cmd));
    }
    if kind == Kind::MenuItem {
        // menu items belong to the option menu they follow
        if parent.and_then(|pp| s.ui.get(pp)).map(|w| w.kind != Kind::OptionMenu).unwrap_or(true) {
            return Err("menuItem belongs inside an optionMenu".to_string());
        }
    }
    let id = s.ui.add(kind, name, parent)?;
    // defaults
    if let Some(w) = s.ui.get_mut(id) {
        match kind {
            Kind::IntSlider | Kind::FloatSlider | Kind::AttrSlider => {
                w.min = 0.0;
                w.max = 100.0;
            }
            Kind::Row => w.columns = 2,
            _ => {}
        }
    }
    apply_flags(s, id, &p)?;
    // layouts and option menus receive what follows
    if kind.is_layout() || kind == Kind::OptionMenu {
        s.ui.parent = Some(id);
    }
    Ok(vec![s.ui.path(id)])
}

fn num(p: &Parsed, k: &str) -> Result<Option<f64>, String> {
    Ok(p.num(k)?.map(|v| v as f64))
}

fn apply_flags(s: &mut Session, id: usize, p: &Parsed) -> Result<(), String> {
    let kind = s.ui.get(id).map(|w| w.kind).ok_or("The control is gone")?;
    let attr_value = |s: &mut Session, attr: &str| -> Option<f64> {
        let (n, a) = s.scene.resolve_plug(attr).ok()?;
        s.scene.get_attr(n, &a).ok().map(|v| v as f64)
    };
    // numbers first (they can be needed by the attribute link)
    let (mn, mx, fmn, fmx, v) = (num(p, "minValue")?, num(p, "maxValue")?, num(p, "fieldMinValue")?, num(p, "fieldMaxValue")?, num(p, "value")?);
    let w = s.ui.get_mut(id).unwrap();
    if let Some(l) = p.string("label") {
        w.label = l;
    }
    if let Some(t) = p.string("title") {
        w.label = t;
    }
    if let Some(t) = p.string("text") {
        w.text = t;
    }
    if let Some(x) = num(p, "width")? {
        w.width = x as f32;
    }
    if let Some(x) = num(p, "height")? {
        w.height = x as f32;
    }
    if let Some(x) = p.string("enable") {
        w.enabled = truthy(&x);
    }
    if let Some(x) = p.string("visible") {
        w.visible = truthy(&x);
    }
    if let Some(c) = p.string("command") {
        w.command = c;
    }
    if let Some(c) = p.string("changeCommand") {
        w.change_command = c;
    }
    if let Some(c) = p.string("dragCommand") {
        w.change_command = c;
    }
    if let Some(c) = p.string("onCommand") {
        w.on_command = c;
    }
    if let Some(c) = p.string("offCommand") {
        w.off_command = c;
    }
    if let Some(a) = p.string("annotation") {
        w.annotation = a;
    }
    if let Some(a) = p.string("field") {
        w.field = truthy(&a);
    }
    if let Some(x) = mn {
        w.min = x;
    }
    if let Some(x) = mx {
        w.max = x;
    }
    if let Some(x) = fmn {
        w.field_min = x;
    }
    if let Some(x) = fmx {
        w.field_max = x;
    }
    if let Some(x) = num(p, "rowSpacing")? {
        w.row_spacing = x as f32;
    }
    if let Some(x) = num(p, "numberOfColumns")? {
        w.columns = (x as usize).clamp(1, 20);
    }
    if let Some(x) = p.string("collapsable") {
        w.collapsable = truthy(&x);
    }
    if let Some(x) = p.string("collapse") {
        w.collapsed = truthy(&x);
    }
    if let Some(a) = p.string("attribute") {
        w.attribute = a;
    }
    if let Some(x) = v {
        w.value = match kind {
            Kind::IntSlider | Kind::IntField => x.round(),
            Kind::Check => (x > 0.5) as u8 as f64,
            _ => x,
        };
    }
    // a string value of a check box ("true")
    if kind == Kind::Check {
        if let Some(sv) = p.string("value") {
            w.value = truthy(&sv) as u8 as f64;
        }
    }
    if kind == Kind::OptionMenu {
        if let Some(sv) = p.string("value") {
            w.text = sv;
        }
    }
    if kind == Kind::MenuItem {
        // a menu item joins its option menu's list
        let (label, parent) = (w.label.clone(), w.parent);
        if let Some(pw) = parent.and_then(|pp| s.ui.get_mut(pp)) {
            if pw.text.is_empty() {
                pw.text = label;
            }
        }
    }
    // an attribute slider shows and drives an attribute
    let attr = s.ui.get(id).map(|w| w.attribute.clone()).unwrap_or_default();
    if kind == Kind::AttrSlider && !attr.is_empty() {
        if let Some(x) = attr_value(s, &attr) {
            if let Some(w) = s.ui.get_mut(id) {
                if v.is_none() {
                    w.value = x;
                }
            }
        } else {
            return Err(format!("The attribute {} does not exist", attr));
        }
        if let Some(x) = v {
            let (n, a) = s.scene.resolve_plug(&attr)?;
            s.scene.set_attr(n, &a, x as f32)?;
        }
    }
    Ok(())
}

fn query(s: &mut Session, id: usize, p: &Parsed) -> Out {
    let w = s.ui.get(id).ok_or("The control is gone")?.clone();
    for (flag, v) in [("value", fmt_value(&w)), ("title", w.label.clone()), ("label", w.label.clone()), ("text", w.text.clone()), ("command", w.command.clone()), ("annotation", w.annotation.clone())] {
        if p.has(flag) {
            return Ok(vec![v]);
        }
    }
    for (flag, v) in [("enable", w.enabled), ("visible", w.visible), ("collapse", w.collapsed), ("field", w.field)] {
        if p.has(flag) {
            return Ok(vec![(v as u8).to_string()]);
        }
    }
    for (flag, v) in [("minValue", w.min), ("maxValue", w.max), ("fieldMinValue", w.field_min), ("fieldMaxValue", w.field_max), ("width", w.width as f64), ("height", w.height as f64)] {
        if p.has(flag) {
            return Ok(vec![fmt_f(v as f32)]);
        }
    }
    if p.has("numberOfChildren") || p.has("childArray") {
        return Ok(w.children.iter().map(|c| s.ui.path(*c)).collect());
    }
    Err("Say what to query: -value, -label, -text, -enable, -visible, -minValue ...".to_string())
}

fn show_window(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[])?;
    let n = p.args.first().map(|a| a.as_string()).ok_or("Usage: showWindow NAME")?;
    let id = s.ui.find(&n).ok_or_else(|| format!("Object not found: {}", n))?;
    let w = s.ui.get_mut(id).ok_or("The window is gone")?;
    if w.kind != Kind::Window {
        return Err(format!("{} is not a window", n));
    }
    w.shown = true;
    Ok(vec![])
}

fn delete_ui(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[f!("window"; 0), f!("control"; 0), f!("layout"; 0), f!("menuItem"; 0)])?;
    let names: Vec<String> = p.args.iter().map(|a| a.as_string()).collect();
    if names.is_empty() {
        return Err("Usage: deleteUI NAME".to_string());
    }
    for n in names {
        let id = s.ui.find(&n).ok_or_else(|| format!("Object not found: {}", n))?;
        s.ui.delete(id);
    }
    Ok(vec![])
}

fn set_parent(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[f!("top", "topLevel"; 0), f!("query", "q"; 0), f!("menu"; 0)])?;
    if p.has("query") {
        return Ok(vec![s.ui.parent.map(|i| s.ui.path(i)).unwrap_or_default()]);
    }
    let target = p.args.first().map(|a| a.as_string());
    match target.as_deref() {
        Some("..") => {
            s.ui.parent = s.ui.parent.and_then(|i| s.ui.get(i)).and_then(|w| w.parent);
        }
        Some(n) => {
            let id = s.ui.find(n).ok_or_else(|| format!("Object not found: {}", n))?;
            s.ui.parent = Some(id);
        }
        None => {
            // setParent with no name goes up one level, or out of the window
            s.ui.parent = s.ui.parent.and_then(|i| s.ui.get(i)).and_then(|w| w.parent);
        }
    }
    if p.has("top") {
        let mut cur = s.ui.parent;
        while let Some(c) = cur {
            match s.ui.get(c).and_then(|w| w.parent) {
                Some(pp) => cur = Some(pp),
                None => break,
            }
        }
        s.ui.parent = cur;
    }
    Ok(vec![])
}

/// Press a control as the pointer would: a button runs its command, a check box toggles and runs its commands; `-value` sets the
/// value of a slider or field and runs its change command. Used by scripts and tests; the windows call the same function.
fn invoke_control(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, &[f!("value", "v"; 1), f!("text", "tx"; 1)])?;
    let n = p.args.last().map(|a| a.as_string()).ok_or("Usage: invokeControl [-value V] NAME")?;
    let id = s.ui.find(&n).ok_or_else(|| format!("Object not found: {}", n))?;
    let (v, t) = (p.num("value")?.map(|x| x as f64), p.string("text"));
    invoke(s, id, v, t)
}

/// Run the commands of a control after the user changed it (value or text) or pressed it.
pub fn invoke(s: &mut Session, id: usize, value: Option<f64>, text: Option<String>) -> Out {
    let w = s.ui.get(id).ok_or("The control is gone")?.clone();
    if !w.enabled {
        return Err(format!("{} is disabled", s.ui.path(id)));
    }
    let mut cmds: Vec<String> = Vec::new();
    match w.kind {
        Kind::Button | Kind::MenuItem => cmds.push(w.command.clone()),
        Kind::Check => {
            let on = value.map(|v| v > 0.5).unwrap_or(w.value <= 0.5);
            if let Some(m) = s.ui.get_mut(id) {
                m.value = on as u8 as f64;
            }
            cmds.push(if on { w.on_command.clone() } else { w.off_command.clone() });
            cmds.push(w.change_command.clone());
        }
        _ => {
            if let Some(v) = value {
                let v = v.clamp(w.field_min.min(w.min), w.field_max.max(w.max));
                if let Some(m) = s.ui.get_mut(id) {
                    m.value = if matches!(m.kind, Kind::IntSlider | Kind::IntField) { v.round() } else { v };
                }
                if w.kind == Kind::AttrSlider && !w.attribute.is_empty() {
                    let (n, a) = s.scene.resolve_plug(&w.attribute)?;
                    s.scene.set_attr(n, &a, v as f32)?;
                }
            }
            if let Some(t) = text {
                if let Some(m) = s.ui.get_mut(id) {
                    m.text = t;
                }
            }
            cmds.push(w.change_command.clone());
            if matches!(w.kind, Kind::TextField | Kind::IntField | Kind::FloatField) {
                cmds.push(w.command.clone());
            }
        }
    }
    let mut out = Vec::new();
    for c in cmds.into_iter().filter(|c| !c.trim().is_empty()) {
        let o = execute_script(s, &c);
        out.extend(o.lines);
        if let Some(e) = o.error {
            return Err(e);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------- shelf and marking menus

#[derive(Clone, Debug, PartialEq)]
pub struct ShelfButton {
    pub label: String,
    pub command: String,
    pub annotation: String,
}

/// Eight items in the directions N, NE, E, SE, S, SW, W, NW.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkingMenu {
    pub name: String,
    pub items: [Option<(String, String)>; 8],
}

pub const DIRECTIONS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

const SHELF_SPEC: &[FlagSpec] = &[f!("label", "l"; 1), f!("command", "c"; 1), f!("annotation", "ann"; 1), f!("delete"; 1), f!("list"; 0), f!("move"; 2)];

/// `shelfButton -label "Blink 10" -command "blink 10" [-annotation "..."]` adds a button to the custom shelf; `-delete LABEL` removes
/// it; `-list` shows the buttons. Save the shelf with `prefs -save`.
pub fn shelf_button(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, SHELF_SPEC)?;
    if p.has("list") {
        return Ok(s.prefs.shelf.iter().map(|b| format!("{}: {}", b.label, b.command)).collect());
    }
    if let Some(l) = p.string("delete") {
        let n = s.prefs.shelf.len();
        s.prefs.shelf.retain(|b| b.label != l);
        if s.prefs.shelf.len() == n {
            return Err(format!("No shelf button labelled {}", l));
        }
        return Ok(vec![]);
    }
    let label = p.string("label").ok_or("Usage: shelfButton -label LABEL -command COMMAND")?;
    let command = p.string("command").ok_or("A shelf button needs a -command")?;
    if let Some(b) = s.prefs.shelf.iter_mut().find(|b| b.label == label) {
        b.command = command;
        b.annotation = p.string("annotation").unwrap_or_default();
    } else {
        s.prefs.shelf.push(ShelfButton { label: label.clone(), command, annotation: p.string("annotation").unwrap_or_default() });
    }
    Ok(vec![label])
}

const MM_SPEC: &[FlagSpec] = &[f!("create"; 1), f!("item"; 3), f!("popup"; 1), f!("delete"; 1), f!("list"; 0), f!("query", "q"; 1), f!("menu", "m"; 1)];

/// `markingMenu -create NAME`, `markingMenu -menu NAME -item N "Blink 10" "blink 10"` (directions N NE E SE S SW W NW), `markingMenu -popup NAME`
/// (the workspace shows it at the pointer), `-delete NAME`, `-list`, `-q NAME`. Bind it to a key with the hotkey editor: command `markingMenu -popup NAME`.
pub fn marking_menu(s: &mut Session, rest: &[Token]) -> Out {
    let p = parse_args(rest, MM_SPEC)?;
    if p.has("list") {
        return Ok(s.prefs.marking_menus.iter().map(|m| format!("{}: {} item(s)", m.name, m.items.iter().flatten().count())).collect());
    }
    if let Some(n) = p.string("create") {
        if s.prefs.marking_menus.iter().any(|m| m.name == n) {
            return Err(format!("A marking menu named {} exists", n));
        }
        s.prefs.marking_menus.push(MarkingMenu { name: n.clone(), items: Default::default() });
        return Ok(vec![n]);
    }
    if let Some(n) = p.string("delete") {
        let k = s.prefs.marking_menus.len();
        s.prefs.marking_menus.retain(|m| m.name != n);
        if s.prefs.marking_menus.len() == k {
            return Err(format!("No marking menu named {}", n));
        }
        return Ok(vec![]);
    }
    if let Some(n) = p.string("query") {
        let m = s.prefs.marking_menus.iter().find(|m| m.name == n).ok_or_else(|| format!("No marking menu named {}", n))?;
        return Ok(m.items.iter().enumerate().filter_map(|(i, it)| it.as_ref().map(|(l, c)| format!("{}: {} -> {}", DIRECTIONS[i], l, c))).collect());
    }
    if let Some(n) = p.string("popup") {
        if !s.prefs.marking_menus.iter().any(|m| m.name == n) {
            return Err(format!("No marking menu named {}", n));
        }
        s.popup_menu = Some(n);
        return Ok(vec![]);
    }
    if let Some(n) = p.string("menu") {
        let v = p.get("item").ok_or("Usage: markingMenu -menu NAME -item N \"label\" \"command\"")?;
        let dir = v[0].as_string().to_uppercase();
        let idx = DIRECTIONS.iter().position(|d| *d == dir).ok_or_else(|| format!("Unknown direction '{}': {}", dir, DIRECTIONS.join(" ")))?;
        let m = s.prefs.marking_menus.iter_mut().find(|m| m.name == n).ok_or_else(|| format!("No marking menu named {}", n))?;
        let (label, command) = (v[1].as_string(), v[2].as_string());
        m.items[idx] = if label.is_empty() && command.is_empty() { None } else { Some((label, command)) };
        return Ok(vec![]);
    }
    Err("Usage: markingMenu -create NAME | -menu NAME -item DIR LABEL COMMAND | -popup NAME | -delete NAME | -list".to_string())
}
