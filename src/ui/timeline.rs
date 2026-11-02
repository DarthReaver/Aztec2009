use super::screen::*;
use crate::dag::Scene;
use crate::mel::parser::fmt_num;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transport {
    Rewind,
    StepBack,
    PlayPause,
    StepFwd,
    End,
}

#[derive(Default)]
pub struct TimelineState {
    pub buttons: Vec<(Rect, Transport)>,
    pub track: Option<Rect>,
}

/// 24 fps timeline: ruler, keyframe tick track, and transport controls.
pub fn draw(screen: &mut Screen, r: Rect, scene: &Scene, st: &mut TimelineState) {
    st.buttons.clear();
    st.track = None;
    if r.h < 3 || r.w < 20 {
        return;
    }
    let tl = &scene.timeline;
    screen.fill(r, ' ', TEXT, PANEL_BG);
    let track = Rect::new(r.x + 1, r.y + 1, r.w - 2, 1);
    st.track = Some(Rect::new(track.x, r.y, track.w, 2));
    let span = (tl.end - tl.start).max(1.0);
    let to_x = |f: f32| -> usize { track.x + (((f - tl.start) / span) * (track.w - 1) as f32).round().clamp(0.0, (track.w - 1) as f32) as usize };

    // ruler
    screen.fill(Rect::new(r.x, r.y, r.w, 1), ' ', TEXT_DIM, PANEL_DARK);
    let frames = (tl.end - tl.start) as usize + 1;
    let px_per_frame = (track.w - 1) as f32 / span;
    let steps = [1usize, 2, 5, 10, 20, 25, 50, 100, 200];
    let step = steps.iter().copied().find(|s| *s as f32 * px_per_frame >= 6.0).unwrap_or(200);
    let mut f = tl.start as usize;
    if !f.is_multiple_of(step) {
        f += step - f % step;
    }
    while f < tl.start as usize + frames {
        let x = to_x(f as f32);
        let label = format!("{}", f);
        if x + label.len() <= r.x + r.w {
            screen.text(x, r.y, &label, TEXT_DIM, PANEL_DARK, r.x + r.w);
        }
        f += step;
    }

    // track with key ticks
    screen.fill(Rect::new(r.x, r.y + 1, r.w, 1), ' ', TEXT, PANEL_DARK);
    for x in track.x..track.x + track.w {
        screen.set(x, track.y, Cell { ch: '\u{2500}', fg: TEXT_DIM.scale(0.45), bg: PANEL_DARK, bold: false });
    }
    let sel = scene.selection.clone();
    let times = if sel.is_empty() { scene.anim.key_times(None) } else { scene.anim.key_times(Some(&sel)) };
    for t in times {
        if t >= tl.start && t <= tl.end {
            screen.set(to_x(t), track.y, Cell { ch: '\u{2502}', fg: RED, bg: PANEL_DARK, bold: true });
        }
    }
    let cx = to_x(tl.current);
    screen.set(cx, track.y, Cell { ch: ' ', fg: TEXT, bg: Rgb(230, 70, 60), bold: false });
    let tag = format!("{}", tl.frame());
    let tx = if cx + tag.len() + 1 < r.x + r.w { cx + 1 } else { cx.saturating_sub(tag.len()) };
    if r.y > 0 {
        screen.text_bold(tx, r.y, &tag, Rgb(255, 120, 110), PANEL_DARK, r.x + r.w);
    }

    // transport row
    let y = r.y + 2;
    screen.fill(Rect::new(r.x, y, r.w, 1), ' ', TEXT, PANEL_BG);
    let mut x = r.x + 1;
    let play_label = if tl.playing { "||" } else { ">" };
    let buttons: [(&str, Transport); 5] = [
        ("|<", Transport::Rewind),
        ("<", Transport::StepBack),
        (play_label, Transport::PlayPause),
        (">|", Transport::StepFwd),
        (">>", Transport::End),
    ];
    // StepFwd uses ">|" in classic transports; keep labels distinct and clickable.
    for (label, kind) in buttons {
        let text = format!(" {} ", label);
        let w = text.chars().count();
        let bg = if kind == Transport::PlayPause && tl.playing { SEL_BG } else { TITLE_BG };
        screen.text(x, y, &text, TEXT, bg, r.x + r.w);
        st.buttons.push((Rect::new(x, y, w, 1), kind));
        x += w + 1;
    }
    let layers = scene.anim.layers.len();
    let info = format!(
        " Frame {:>3}   Range {}-{}   {} fps   {}   Layers {}",
        tl.frame(),
        fmt_num(tl.start as f64),
        fmt_num(tl.end as f64),
        fmt_num(tl.fps as f64),
        if tl.playing { "PLAYING" } else { "stopped" },
        layers
    );
    screen.text(x + 1, y, &info, TEXT, PANEL_BG, r.x + r.w);
}

pub fn frame_at(st: &TimelineState, scene: &Scene, x: usize) -> Option<f32> {
    let t = st.track?;
    let tl = &scene.timeline;
    let span = (tl.end - tl.start).max(1.0);
    let rel = x.saturating_sub(t.x).min(t.w.saturating_sub(1)) as f32 / (t.w.saturating_sub(1)).max(1) as f32;
    Some((tl.start + rel * span).round())
}
