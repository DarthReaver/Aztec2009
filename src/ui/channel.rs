//! Channel Box (editable attributes of the selection) and Attribute Editor (node info and layer sliders).

use super::screen::*;
use crate::dag::nodes::NodeData;
use crate::dag::{NodeId, Scene};
use crate::mel::parser::fmt_num;

pub struct ChanRow {
    pub header: Option<String>,
    pub node: Option<NodeId>,
    pub attr: String,
    pub value: f32,
    pub text: String,
    pub keyed: bool,
    pub connected: bool,
    pub editable: bool,
}

#[derive(Default)]
pub struct ChannelState {
    pub cursor: usize,
    pub scroll: usize,
    pub editing: Option<String>,
    pub sliders: Vec<(Rect, usize)>,
}

fn plug_rows(scene: &mut Scene, id: NodeId, out: &mut Vec<ChanRow>) {
    let names: Vec<(String, bool, bool)> = match scene.node(id) {
        Some(n) => n.plugs.iter().map(|p| (p.name.clone(), p.source.is_some(), p.readonly)).collect(),
        None => return,
    };
    for (name, connected, readonly) in names {
        let v = scene.get_attr(id, &name).unwrap_or(0.0);
        let keyed = scene.anim.is_animated(id, &name);
        out.push(ChanRow {
            header: None,
            node: Some(id),
            attr: name,
            value: v,
            text: fmt_num(v as f64),
            keyed,
            connected,
            editable: !connected && !readonly,
        });
    }
}

/// Rows describing the primary selection: transform channels, then each shape underneath.
pub fn rows(scene: &mut Scene) -> Vec<ChanRow> {
    let mut out = Vec::new();
    let id = match scene.selection.first().copied() {
        Some(i) if scene.node(i).is_some() => i,
        _ => return out,
    };
    plug_rows(scene, id, &mut out);
    let children: Vec<NodeId> = scene.node(id).map(|n| n.children.clone()).unwrap_or_default();
    for c in children {
        let (name, is_mesh) = match scene.node(c) {
            Some(n) => (n.name.clone(), matches!(n.data, NodeData::Mesh(_))),
            None => continue,
        };
        out.push(ChanRow {
            header: Some(name),
            node: None,
            attr: String::new(),
            value: 0.0,
            text: String::new(),
            keyed: false,
            connected: false,
            editable: false,
        });
        plug_rows(scene, c, &mut out);
        if is_mesh {
            if let Some(NodeData::Mesh(md)) = scene.node(c).map(|n| &n.data) {
                for (k, v) in &md.params {
                    out.push(ChanRow {
                        header: None,
                        node: None,
                        attr: k.clone(),
                        value: *v,
                        text: fmt_num(*v as f64),
                        keyed: false,
                        connected: false,
                        editable: false,
                    });
                }
            }
        }
    }
    out
}

impl ChannelState {
    /// Keep the cursor on an editable row.
    pub fn clamp(&mut self, rows: &[ChanRow]) {
        if rows.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = self.cursor.min(rows.len() - 1);
        if !rows[self.cursor].editable {
            if let Some(i) = rows.iter().position(|r| r.editable) {
                if rows.iter().skip(self.cursor).all(|r| !r.editable) {
                    self.cursor = i;
                }
            }
        }
    }

    pub fn move_cursor(&mut self, rows: &[ChanRow], delta: i32) {
        if rows.is_empty() {
            return;
        }
        let mut i = self.cursor as i32;
        loop {
            i += delta;
            if i < 0 || i >= rows.len() as i32 {
                return;
            }
            if rows[i as usize].editable || rows[i as usize].node.is_some() {
                self.cursor = i as usize;
                return;
            }
        }
    }
}

pub fn draw_channel(screen: &mut Screen, r: Rect, scene: &Scene, rows: &[ChanRow], st: &mut ChannelState, focus: bool) {
    if r.is_empty() {
        return;
    }
    screen.fill(r, ' ', TEXT, PANEL_BG);
    let title = match scene.selection.first().and_then(|i| scene.node(*i)) {
        Some(n) => format!("Channel Box: {}", n.name),
        None => "Channel Box".to_string(),
    };
    screen.title_bar(Rect::new(r.x, r.y, r.w, 1), &title, focus);
    let vis = r.h.saturating_sub(1);
    if rows.is_empty() {
        screen.text(r.x + 1, r.y + 2, "Nothing selected", TEXT_DIM, PANEL_BG, r.x + r.w);
        return;
    }
    if st.cursor < st.scroll {
        st.scroll = st.cursor;
    } else if vis > 0 && st.cursor >= st.scroll + vis {
        st.scroll = st.cursor + 1 - vis;
    }
    let name_w = (r.w * 5 / 9).max(8).min(r.w.saturating_sub(4));
    for (k, row) in rows.iter().skip(st.scroll).take(vis).enumerate() {
        let y = r.y + 1 + k;
        let idx = st.scroll + k;
        if let Some(h) = &row.header {
            screen.fill(Rect::new(r.x, y, r.w, 1), ' ', TEXT, PANEL_DARK);
            screen.text_bold(r.x + 1, y, h, CYAN, PANEL_DARK, r.x + r.w);
            continue;
        }
        let is_cursor = focus && idx == st.cursor;
        let mut bg = PANEL_BG;
        if row.keyed {
            bg = KEY_BG.scale(0.75);
        } else if row.connected {
            bg = CONN_BG.scale(0.75);
        }
        if is_cursor {
            bg = bg.mix(SEL_BG, 0.6);
        }
        screen.fill(Rect::new(r.x, y, r.w, 1), ' ', TEXT, bg);
        let fg = if row.editable || row.node.is_some() { TEXT } else { TEXT_DIM };
        screen.text(r.x + 1, y, &row.attr, fg, bg, r.x + name_w);
        let shown = match (&st.editing, is_cursor) {
            (Some(buf), true) => format!("{}_", buf),
            _ => row.text.clone(),
        };
        let vw = r.w.saturating_sub(name_w + 1);
        let vx = r.x + name_w + vw.saturating_sub(shown.chars().count());
        screen.text(vx.max(r.x + name_w), y, &shown, if st.editing.is_some() && is_cursor { YELLOW } else { fg }, bg, r.x + r.w);
    }
}

fn bar(screen: &mut Screen, x: usize, y: usize, w: usize, frac: f32, fg: Rgb) {
    let filled = ((w as f32) * frac.clamp(0.0, 1.0)).round() as usize;
    for i in 0..w {
        if i < filled {
            screen.set(x + i, y, Cell { ch: '\u{2588}', fg, bg: PANEL_DARK, bold: false });
        } else {
            screen.set(x + i, y, Cell { ch: '\u{2591}', fg: TEXT_DIM.scale(0.6), bg: PANEL_DARK, bold: false });
        }
    }
}

pub fn draw_editor(screen: &mut Screen, r: Rect, scene: &Scene, st: &mut ChannelState) {
    st.sliders.clear();
    if r.is_empty() {
        return;
    }
    screen.fill(r, ' ', TEXT, PANEL_DARK);
    screen.title_bar(Rect::new(r.x, r.y, r.w, 1), "Attribute Editor", false);
    let maxx = r.x + r.w;
    let layer_lines = scene.anim.layers.len() + 1;
    let show_layers = r.h >= layer_lines + 3;
    let info_end = if show_layers { r.y + r.h - layer_lines } else { r.y + r.h };
    let mut y = r.y + 1;
    let line = |screen: &mut Screen, y: &mut usize, s: String, fg: Rgb| {
        if *y < info_end {
            screen.text(r.x + 1, *y, &s, fg, PANEL_DARK, maxx);
            *y += 1;
        }
    };
    if let Some(id) = scene.selection.first().copied() {
        if let Some(n) = scene.node(id) {
            line(screen, &mut y, format!("{} ({})", n.name, n.data.type_name()), TEXT);
            if let Some(p) = n.parent {
                line(screen, &mut y, format!("parent: {}", scene.name_of(p)), TEXT_DIM);
            }
            if !n.children.is_empty() {
                line(screen, &mut y, format!("children: {}", n.children.len()), TEXT_DIM);
            }
            let mut ids = vec![id];
            ids.extend(n.children.iter().copied());
            for sid in &ids {
                if let Some(sn) = scene.node(*sid) {
                    match &sn.data {
                        NodeData::Mesh(md) => {
                            line(
                                screen,
                                &mut y,
                                format!("{}v {}f {}t", md.mesh.vertex_count(), md.mesh.face_count(), md.mesh.triangle_count()),
                                TEXT_DIM,
                            );
                            let m = md.material.map(|m| scene.name_of(m)).unwrap_or_else(|| "default".to_string());
                            line(screen, &mut y, format!("material: {}", m), TEXT_DIM);
                        }
                        NodeData::Nucleus(nd) => {
                            line(screen, &mut y, format!("particles: {}", nd.solver.particle_count()), TEXT_DIM);
                        }
                        _ => {}
                    }
                    for p in &sn.plugs {
                        if let Some(src) = &p.source {
                            line(
                                screen,
                                &mut y,
                                format!("\u{2190} {}.{} > {}", scene.name_of(src.node), src.attr, p.name),
                                Rgb(190, 150, 230),
                            );
                        }
                        for d in &p.dests {
                            line(
                                screen,
                                &mut y,
                                format!("{} > {}.{}", p.name, scene.name_of(d.node), d.attr),
                                Rgb(190, 150, 230),
                            );
                        }
                    }
                }
            }
        }
    } else {
        line(screen, &mut y, "No selection".to_string(), TEXT_DIM);
    }
    if show_layers {
        let ly = r.y + r.h - layer_lines;
        screen.fill(Rect::new(r.x, ly, r.w, 1), ' ', TEXT, TITLE_BG.scale(0.8));
        screen.text_bold(r.x + 1, ly, "Animation Layers", TEXT, TITLE_BG.scale(0.8), maxx);
        for (i, l) in scene.anim.layers.iter().enumerate() {
            let y = ly + 1 + i;
            let active = i == scene.anim.active_layer;
            let name_w = 9.min(r.w.saturating_sub(10));
            let label = format!("{}{}", if active { "*" } else { " " }, l.name);
            let fg = if l.mute { TEXT_DIM } else { TEXT };
            screen.text(r.x, y, &label, fg, PANEL_DARK, r.x + name_w + 1);
            let bx = r.x + name_w + 1;
            let val_w = 5;
            let bw = r.w.saturating_sub(name_w + 1 + val_w + 1).max(4);
            bar(screen, bx, y, bw, l.weight, if i == 0 { TEXT_DIM } else { KEY_BG });
            screen.text(bx + bw + 1, y, &format!("{:.2}", l.weight), fg, PANEL_DARK, maxx);
            if i > 0 {
                st.sliders.push((Rect::new(bx, y, bw, 1), i));
            }
        }
    }
}

/// Convert a click x position on a slider into a 0..1 weight rounded to 0.05.
pub fn slider_weight(r: Rect, x: usize) -> f32 {
    let t = (x.saturating_sub(r.x)) as f32 / (r.w.saturating_sub(1)).max(1) as f32;
    ((t.clamp(0.0, 1.0)) * 20.0).round() / 20.0
}
