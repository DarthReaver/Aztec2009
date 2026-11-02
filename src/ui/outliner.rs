use super::screen::*;
use crate::dag::nodes::NodeData;
use crate::dag::{NodeId, Scene};

#[derive(Default)]
pub struct OutlinerState {
    pub scroll: usize,
}

pub fn glyph(data: &NodeData) -> (char, Rgb) {
    match data {
        NodeData::Transform => ('T', TEXT),
        NodeData::Mesh(_) => ('M', GREEN),
        NodeData::Camera => ('C', CYAN),
        NodeData::StereoCamera => ('S', Rgb(230, 140, 230)),
        NodeData::Light => ('L', YELLOW),
        NodeData::Material => ('m', Rgb(240, 160, 80)),
        NodeData::Emitter => ('E', Rgb(255, 170, 90)),
        NodeData::Locator => ('+', YELLOW),
        NodeData::Constraint(_) => ('K', Rgb(200, 120, 255)),
        NodeData::ObjectSet(_) => ('S', Rgb(120, 220, 200)),
        NodeData::Texture(_) => ('F', Rgb(240, 160, 80)),
        NodeData::NurbsCurve(_) => ('C', Rgb(90, 200, 255)),
        NodeData::MotionPath(_) => ('P', Rgb(120, 220, 120)),
        NodeData::Brush(_) => ('B', Rgb(120, 200, 120)),
        NodeData::RenderGlobals(_) => ('R', Rgb(200, 200, 120)),
        NodeData::RigidBody(_) => ('r', Rgb(220, 160, 100)),
        NodeData::DynSolver(_) => ('S', Rgb(220, 160, 100)),
        NodeData::Field(_) => ('F', Rgb(120, 180, 240)),
        NodeData::RigidJoint(_) => ('j', Rgb(220, 160, 100)),
        NodeData::PEmitter(_) => ('e', Rgb(240, 160, 60)),
        NodeData::Particle(_) => ('p', Rgb(120, 200, 240)),
        NodeData::Lattice(_) => ('L', Rgb(255, 160, 220)),
        NodeData::Character(_) => ('H', Rgb(255, 200, 120)),
        NodeData::Distance(_) => ('D', Rgb(200, 200, 120)),
        NodeData::Sound(_) => ('A', Rgb(240, 200, 100)),
        NodeData::DrivenCurve(_) => ('~', Rgb(200, 120, 255)),
        NodeData::Nucleus(_) => ('N', Rgb(120, 160, 255)),
        NodeData::PolyOp(_) => ('h', TEXT),
        NodeData::Joint => ('J', GREEN),
        NodeData::IkHandle(_) => ('I', CYAN),
    }
}

pub fn draw(screen: &mut Screen, r: Rect, scene: &Scene, st: &mut OutlinerState, focus: bool) {
    if r.is_empty() {
        return;
    }
    screen.fill(r, ' ', TEXT, PANEL_DARK);
    screen.title_bar(Rect::new(r.x, r.y, r.w, 1), "Outliner", focus);
    let rows = scene.outliner_rows();
    let vis = r.h.saturating_sub(1);
    if let Some(sel) = scene.selection.first() {
        if let Some(idx) = rows.iter().position(|(id, _)| id == sel) {
            if idx < st.scroll {
                st.scroll = idx;
            } else if vis > 0 && idx >= st.scroll + vis {
                st.scroll = idx + 1 - vis;
            }
        }
    }
    st.scroll = st.scroll.min(rows.len().saturating_sub(vis));
    for (k, (id, depth)) in rows.iter().skip(st.scroll).take(vis).enumerate() {
        let y = r.y + 1 + k;
        let node = match scene.node(*id) {
            Some(n) => n,
            None => continue,
        };
        let selected = scene.selection.contains(id);
        let bg = if selected {
            if focus {
                SEL_BG
            } else {
                SEL_BG.scale(0.7)
            }
        } else {
            PANEL_DARK
        };
        screen.fill(Rect::new(r.x, y, r.w, 1), ' ', TEXT, bg);
        let indent = (depth * 2).min(r.w.saturating_sub(6));
        let (g, gc) = glyph(&node.data);
        let x = r.x + 1 + indent;
        screen.text(x, y, &g.to_string(), gc, bg, r.x + r.w);
        let fg = if node.data.is_transform() { TEXT } else { TEXT_DIM };
        screen.text(x + 2, y, &node.name, if selected { Rgb(255, 255, 255) } else { fg }, bg, r.x + r.w);
    }
}

pub fn row_at(scene: &Scene, st: &OutlinerState, r: Rect, y: usize) -> Option<NodeId> {
    if y <= r.y || y >= r.y + r.h {
        return None;
    }
    scene.outliner_rows().get(st.scroll + y - r.y - 1).map(|(id, _)| *id)
}

/// Move the primary selection up or down the visible hierarchy.
pub fn step_selection(scene: &mut Scene, delta: i32) {
    let rows = scene.outliner_rows();
    if rows.is_empty() {
        return;
    }
    let cur = scene
        .selection
        .first()
        .and_then(|s| rows.iter().position(|(id, _)| id == s))
        .map(|i| i as i32)
        .unwrap_or(if delta > 0 { -1 } else { rows.len() as i32 });
    let next = (cur + delta).clamp(0, rows.len() as i32 - 1) as usize;
    scene.selection = vec![rows[next].0];
}
