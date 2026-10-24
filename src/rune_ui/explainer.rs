//! "How the shortcut works": a modal from General's Shortcut mode row that
//! shows Hold and Toggle side by side, and Double-tap to lock under Hold. Each
//! choice draws a keycap and two lanes on one playhead, the key above and what
//! Hex records below. Picking a card is the setting.
//!
//! The lanes loop on the window's poll clock while the modal is open; Reduce
//! Motion shows the finished frame.

use std::cell::Cell;
use std::time::Duration;

use rune::{
    Color, Cursor, Cx, El, LineCap, Shape, SpringPair, Transition, Tx, canvas, col, el, row, text,
};

use super::Ui;
use super::controls::{ButtonKind, DESCRIPTION, Paint, button, row_switch};
use crate::desktop_i18n::tr;
use crate::dictation::ShortcutMode;

const MODAL_WIDTH: f32 = 440.0;
const PADDING: f32 = 18.0;
const CARD_PADDING: f32 = 12.0;
const CARD_GAP: f32 = 10.0;
const KEYCAP_WIDTH: f32 = 28.0;
const KEY_HEIGHT: f32 = 26.0;
/// How far the face stands above its base: the lip showing below it.
const KEY_DEPTH: f32 = 2.5;
/// A press sinks the face most of the way, leaving a hairline of lip.
const KEY_TRAVEL: f32 = 2.0;
/// Seconds for the face to sink; it rises on the controls' spring.
const KEY_PRESS: f32 = 0.07;
/// Between the keycaps of a chord.
const KEY_GAP: f32 = 3.0;
const VISUAL_GAP: f32 = 10.0;
/// The lanes' width with one one-glyph keycap, until layout measures them.
const CARD_LANES: f32 =
    (MODAL_WIDTH - 2.0 * PADDING - CARD_GAP) / 2.0 - 2.0 * CARD_PADDING - KEYCAP_WIDTH - VISUAL_GAP;
const LOCK_LANES: f32 =
    MODAL_WIDTH - 2.0 * PADDING - 2.0 * CARD_PADDING - KEYCAP_WIDTH - VISUAL_GAP;
/// The playhead overhangs the lanes by 4 points above and below.
const LANES_HEIGHT: f32 = 25.0;
const LANE: f32 = 6.0;
const KEY_LANE_Y: f32 = 7.0;
const RECORD_LANE_Y: f32 = 18.0;

// One loop, in milliseconds: the playhead crosses the lanes from T0 to T1,
// the finished frame holds, the drawn segments fade, and it starts again.
const LOOP: f32 = 3800.0;
const T0: f32 = 200.0;
const T1: f32 = 2740.0;
/// The finished frame: the key up and both lanes fully drawn.
pub(super) const DONE: f32 = 3000.0;
const FADE_FROM: f32 = 3300.0;
const FADE: f32 = 400.0;
/// A press widens to at least this once the playhead passes, so a tap is
/// never a sliver.
const MIN_PRESS: f32 = 9.0;
/// Frozen snapshots show the playhead mid-loop.
pub(super) const SNAPSHOT_MS: f32 = 1800.0;
const FRAME: Duration = Duration::from_micros(16_667);

/// A press under 300 ms never records on its own, and a second tap released
/// within 300 ms of the first locks; each script stays inside those rules.
struct Script {
    presses: &'static [(f32, f32)],
    recording: (f32, f32),
    /// The hands-free stretch after a lock, drawn striped.
    locked: Option<(f32, f32)>,
}

const HOLD: Script = Script {
    presses: &[(400.0, 2400.0)],
    recording: (400.0, 2400.0),
    locked: None,
};
const TOGGLE: Script = Script {
    presses: &[(400.0, 540.0), (2400.0, 2540.0)],
    recording: (400.0, 2400.0),
    locked: None,
};
const LOCK: Script = Script {
    presses: &[(300.0, 420.0), (520.0, 640.0), (2400.0, 2540.0)],
    recording: (300.0, 640.0),
    locked: Some((640.0, 2400.0)),
};

#[derive(Default)]
pub(super) struct Explainer {
    pub(super) open: bool,
    /// The poll clock when the loop started.
    start: Option<f64>,
    elapsed_ms: f32,
    /// The poll clock when the next frame's wake is due, so each frame asks
    /// for one.
    next_frame: f64,
    /// A fixed frame for snapshots.
    held_ms: Option<f32>,
    /// Reduce Motion, as the last view saw it: the loop stops ticking.
    reduced: Cell<bool>,
}

impl Explainer {
    pub(super) fn time(&self) -> f32 {
        if self.reduced.get() {
            DONE
        } else {
            self.held_ms.unwrap_or(self.elapsed_ms)
        }
    }
}

/// Opens the modal over General, alone: it ends a shortcut recording and
/// closes the model picker and any menu.
pub(super) fn open(ui: &mut Ui) {
    super::settings::cancel_capture(ui);
    super::picker::close(ui);
    ui.open = None;
    ui.explainer.open = true;
    ui.explainer.start = None;
    ui.explainer.elapsed_ms = 0.0;
    ui.explainer.next_frame = 0.0;
}

pub(super) fn close(ui: &mut Ui) {
    ui.explainer.open = false;
}

/// Opens the modal, held at `held_ms` or looping when `None`.
pub(super) fn fixture(ui: &mut Ui, held_ms: Option<f32>) {
    open(ui);
    ui.explainer.held_ms = held_ms;
}

/// Advances the loop while the modal shows. True when it moved.
pub(super) fn poll(ui: &mut Ui, clock: f64, visible: bool, tx: &mut Tx) -> bool {
    let explainer = &mut ui.explainer;
    if !explainer.open || !visible || explainer.held_ms.is_some() || explainer.reduced.get() {
        return false;
    }
    let start = *explainer.start.get_or_insert(clock);
    explainer.elapsed_ms = ((clock - start) * 1000.0 % f64::from(LOOP)) as f32;
    if clock >= explainer.next_frame {
        explainer.next_frame = clock + FRAME.as_secs_f64();
        tx.wake_after(FRAME);
    }
    true
}

/// The "How it works" link under Shortcut mode. A focusable element rather
/// than a text link, so closing the modal can return focus to it.
pub(super) fn link(p: Paint) -> El {
    let label = tr("How it works");
    row()
        .key("shortcut-help")
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.k.c.border_focus)
        .radius(p.k.radius.sm)
        .cursor(Cursor::Pointer)
        .on_press(|ui: &mut Ui| open(ui))
        .child(text(label).key("label").font(DESCRIPTION).color(p.accent()))
}

pub(super) fn modal(ui: &Ui, cx: &Cx<Ui>, p: Paint) -> Option<[El; 2]> {
    // Kept while closed too, so the poll after opening already knows.
    ui.explainer.reduced.set(cx.system_reduced_motion());
    if !ui.explainer.open {
        return None;
    }
    let t = ui.explainer.time();
    let c = &p.k.c;
    let colors = Colors::new(p);
    let keys = ui.settings.dictation_hotkey.keycaps();
    let hold = ui.settings.shortcut_mode == ShortcutMode::Hold;
    // The lanes draw at the width they lay out at, whatever the keycaps take.
    let lanes = |owner: &str, estimate: f32| {
        cx.layout(&format!("{owner}/visual/lanes"))
            .map_or(estimate, |[_, _, width, _]| width)
    };
    let cards = row()
        .key("modes")
        .equal_columns(2)
        .gap(CARD_GAP)
        .child(card(
            ui,
            p,
            ShortcutMode::Hold,
            tr("Hold"),
            tr("Hold to talk. Let go to paste."),
            visual(p, &colors, &keys, &HOLD, lanes("mode-hold", CARD_LANES), t),
        ))
        .child(card(
            ui,
            p,
            ShortcutMode::Toggle,
            tr("Toggle"),
            tr("Press to start, press to stop."),
            visual(
                p,
                &colors,
                &keys,
                &TOGGLE,
                lanes("mode-toggle", CARD_LANES),
                t,
            ),
        ));
    let lock_lanes = lanes("double-tap", LOCK_LANES);
    let body = col()
        .key("explainer-body")
        .gap(CARD_GAP)
        .child(cards)
        .children(hold.then(|| lock_row(ui, p, &colors, &keys, lock_lanes, t)));
    let mut panel = col()
        .key("shortcut-explainer")
        .role("dialog")
        .label(tr("How the shortcut works"))
        .w(MODAL_WIDTH)
        .pt(PADDING)
        .px(PADDING)
        .pb(14.0)
        .gap(14.0)
        .radius(p.k.radius.xl)
        .fill(c.bg_base)
        .border(p.hair, c.border_weak)
        // Presses inside the panel stay inside.
        .on_press(|_: &mut Ui| {})
        .child(
            text(tr("How the shortcut works"))
                .key("title")
                .font(p.k.ty.body_medium)
                .color(p.text()),
        )
        .child(body)
        .child(row().key("footer").justify_end().child(button(
            p,
            "close-explainer",
            tr("Close"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| close(ui),
        )));
    for shadow in &p.k.elevation.overlay {
        panel = panel.box_shadow(*shadow);
    }
    let springs = SpringPair::new(p.k.motion.overlay, p.k.motion.overlay);
    let layer = |key: &str| {
        el().key(key)
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
    };
    Some([
        layer("explainer-scrim")
            .fill(c.scrim)
            .on_press(close)
            .transition(Transition::opacity().springs(springs)),
        layer("explainer-layer")
            .center()
            .transition(Transition::scale(0.96).springs(springs))
            // Keys stay inside, Escape or a press outside closes, and focus
            // returns to the link that opened it.
            .child(panel.modal(true, close)),
    ])
}

fn card(ui: &Ui, p: Paint, mode: ShortcutMode, title: &str, caption: &str, visual: El) -> El {
    let c = &p.k.c;
    let selected = ui.settings.shortcut_mode == mode;
    let key = match mode {
        ShortcutMode::Hold => "mode-hold",
        ShortcutMode::Toggle => "mode-toggle",
    };
    col()
        .key(key)
        .role("button")
        .label(title)
        .focusable(true)
        .focus_ring(2.0, 2.0, c.border_focus)
        .padding(CARD_PADDING)
        .gap(10.0)
        .radius(p.k.radius.lg)
        .border(
            if selected { 1.5 } else { p.hair },
            if selected {
                c.border_focus
            } else {
                c.border_weak
            },
        )
        .hover(c.hover)
        .press(c.pressed)
        .on_press(move |ui: &mut Ui| pick(ui, mode))
        .child(visual)
        .child(
            col()
                .key("copy")
                .gap(2.0)
                .child(
                    text(title)
                        .key("name")
                        .font(p.k.ty.ui_medium)
                        .color(p.text()),
                )
                .child(
                    text(caption)
                        .key("caption")
                        .font(DESCRIPTION)
                        .color(p.muted())
                        .wrap(),
                ),
        )
}

/// The same save path as Shortcut mode's segmented control.
fn pick(ui: &mut Ui, mode: ShortcutMode) {
    if ui.settings.shortcut_mode != mode {
        ui.update(|settings| settings.shortcut_mode = mode);
    }
}

fn lock_row(ui: &Ui, p: Paint, colors: &Colors, keys: &[String], width: f32, t: f32) -> El {
    let c = &p.k.c;
    // Like General's switch rows, the switch shows focus for the row.
    col()
        .key("double-tap")
        .role("toggle")
        .label(tr("Double-tap to lock"))
        .focusable(true)
        .py(10.0)
        .px(CARD_PADDING)
        .gap(8.0)
        .radius(p.k.radius.lg)
        .border(p.hair, c.border_weak)
        .hover(c.hover)
        .press(c.pressed)
        .on_press(flip_lock)
        .child(
            row()
                .key("top")
                .gap(12.0)
                .center_y()
                .child(
                    col()
                        .key("copy")
                        .grow(1.0)
                        .shrink()
                        .gap(2.0)
                        .child(
                            text(tr("Double-tap to lock"))
                                .key("name")
                                .font(p.k.ty.ui_medium)
                                .color(p.text()),
                        )
                        .child(
                            text(tr("Tap twice to talk hands-free. Press once to stop."))
                                .key("caption")
                                .font(DESCRIPTION)
                                .color(p.muted())
                                .wrap(),
                        ),
                )
                .child(row_switch(p, ui.settings.double_tap_lock)),
        )
        .child(visual(p, colors, keys, &LOCK, width, t))
}

/// Double-tap to lock, as General's row flips it.
pub(super) fn flip_lock(ui: &mut Ui) {
    let enabled = !ui.settings.double_tap_lock;
    ui.update(|settings| {
        settings.double_tap_lock = enabled;
        if !enabled {
            settings.double_tap_only = false;
        }
    });
}

/// One keycap per key of the shortcut, pressing together, beside the lanes
/// drawn `width` points wide.
fn visual(p: Paint, colors: &Colors, keys: &[String], script: &Script, width: f32, t: f32) -> El {
    let down = script.presses.iter().any(|&(d, u)| d <= t && t < u);
    row()
        .key("visual")
        .role("none")
        .h(28.0)
        .gap(VISUAL_GAP)
        .center_y()
        .child(
            row().key("keys").gap(KEY_GAP).children(
                keys.iter()
                    .enumerate()
                    .map(|(index, label)| keycap(p, index, label, down)),
            ),
        )
        .child(
            canvas(lanes(colors, script, width, t))
                .key("lanes")
                .grow(1.0)
                .shrink()
                .h(LANES_HEIGHT),
        )
}

/// One key of the shortcut, pressed while the script holds it: a face on a
/// base whose lip shows below it. A press sinks the face into the lip; the
/// key's bounds never change.
fn keycap(p: Paint, index: usize, label: &str, down: bool) -> El {
    let c = &p.k.c;
    let face_height = KEY_HEIGHT - KEY_DEPTH;
    // Each change springs with the spring of the state it heads to: in
    // fast, back up on the controls' spring.
    let spring = if down {
        p.k.motion.control.with_duration(KEY_PRESS)
    } else {
        p.k.motion.control
    };
    let base = el()
        .key("base")
        .role("none")
        .absolute(0.0, KEY_DEPTH)
        .w_full()
        .h(face_height)
        .radius(p.k.radius.md)
        .fill(c.border_base);
    let face = row()
        .key("cap")
        .min_w(KEYCAP_WIDTH)
        .h(face_height)
        .px(6.0)
        .center()
        .radius(p.k.radius.md)
        .mt(if down { KEY_TRAVEL } else { 0.0 })
        .fill(if down {
            c.bg_base.mix(c.black, 0.05)
        } else {
            c.bg_base
        })
        .border(p.hair, c.border_weak)
        .motion(spring)
        .child(
            text(label.to_string())
                .key("label")
                .font(DESCRIPTION)
                .color(if down { p.text() } else { p.muted() }),
        );
    col()
        .key(format!("keycap{index}"))
        .h(KEY_HEIGHT)
        .child(base)
        .child(face)
}

/// Opaque lane colours over the panel, so overlapping pieces never darken.
struct Colors {
    track: Color,
    key: Color,
    recording: Color,
    locked: Color,
    playhead: Color,
}

impl Colors {
    fn new(p: Paint) -> Self {
        let c = &p.k.c;
        let base = c.bg_base.alpha(1.0);
        let over =
            |under: Color, color: Color, amount: f32| under.mix(color.alpha(1.0), color.a * amount);
        let track = over(base, c.layer_03, 1.0);
        let recording = over(base, p.danger(), 1.0);
        Self {
            track,
            key: over(track, p.text(), 0.55),
            recording,
            locked: track.mix(recording, 0.35),
            playhead: p.text().alpha(p.text().a * 0.45),
        }
    }
}

/// Both lanes at `t`: each segment draws up to the playhead.
fn lanes(colors: &Colors, script: &Script, width: f32, t: f32) -> Shape {
    let x = |ms: f32| at(ms, width);
    let fade = ((t - FADE_FROM) / FADE).clamp(0.0, 1.0);
    let ink = |color: Color| color.mix(colors.track, fade);
    let drawn = |stretch: (f32, f32), min: f32| drawn(stretch, min, width, t);
    let mut shape = Shape::new();
    for y in [KEY_LANE_Y, RECORD_LANE_Y] {
        shape = bar(shape, 0.0, width, y, colors.track, (true, true));
    }
    for &press in script.presses {
        let start = x(press.0);
        shape = bar(
            shape,
            start,
            start + drawn(press, MIN_PRESS),
            KEY_LANE_Y,
            ink(colors.key),
            (true, true),
        );
    }
    let start = x(script.recording.0);
    let end = start + drawn(script.recording, 0.0);
    shape = bar(
        shape,
        start,
        end,
        RECORD_LANE_Y,
        ink(colors.recording),
        (true, script.locked.is_none()),
    );
    if let Some(locked) = script.locked {
        let start = x(locked.0);
        let end = start + drawn(locked, 0.0);
        shape = bar(
            shape,
            start,
            end,
            RECORD_LANE_Y,
            ink(colors.locked),
            (false, true),
        );
        shape = stripes(
            shape,
            start,
            end - LANE / 2.0,
            RECORD_LANE_Y,
            ink(colors.recording),
        );
    }
    if t < T1 {
        let head = x(t.clamp(T0, T1));
        shape = shape.line(
            [head, 0.75],
            [head, LANES_HEIGHT - 0.75],
            1.5,
            LineCap::Round,
            colors.playhead,
        );
    }
    shape.view_box(width, LANES_HEIGHT)
}

/// Where `ms` falls across lanes `width` points wide.
fn at(ms: f32, width: f32) -> f32 {
    (ms - T0) / (T1 - T0) * width
}

/// How much of the stretch from `d` to `u` shows at `t`: up to the playhead.
/// A tap widens to `min` as the playhead passes, never ahead of it.
fn drawn((d, u): (f32, f32), min: f32, width: f32, t: f32) -> f32 {
    if t < d {
        return 0.0;
    }
    let held = at(t.min(u), width) - at(d, width);
    held.max(min).min(at(t, width) - at(d, width))
}

/// A lane-thick bar from `start` to `end`, each end round or square.
fn bar(
    mut shape: Shape,
    start: f32,
    end: f32,
    y: f32,
    color: Color,
    (round_start, round_end): (bool, bool),
) -> Shape {
    let width = end - start;
    if width <= 0.0 {
        return shape;
    }
    let radius = (LANE / 2.0).min(width / 2.0);
    let middle = start + width / 2.0;
    // A bar narrower than the lane is an upright pill.
    shape = if width < LANE {
        let reach = (LANE / 2.0 - radius).max(0.01);
        shape.line(
            [middle, y - reach],
            [middle, y + reach],
            width,
            LineCap::Round,
            color,
        )
    } else {
        let a = start + radius;
        let b = (end - radius).max(a + 0.01);
        shape.line([a, y], [b, y], LANE, LineCap::Round, color)
    };
    let square = |shape: Shape, from: f32, to: f32| {
        shape.fill(
            [
                [from, y - LANE / 2.0],
                [to, y - LANE / 2.0],
                [to, y + LANE / 2.0],
                [from, y + LANE / 2.0],
            ],
            color,
        )
    };
    if !round_start {
        shape = square(shape, start, start + radius.max(0.01).min(width));
    }
    if !round_end {
        shape = square(shape, end - radius.max(0.01).min(width), end);
    }
    shape
}

/// Diagonal stripes over the hands-free stretch, anchored at its start so
/// they hold still while it grows.
fn stripes(mut shape: Shape, start: f32, end: f32, y: f32, color: Color) -> Shape {
    const PERIOD: f32 = 7.0;
    const BAND: f32 = 3.5;
    if end <= start {
        return shape;
    }
    let (top, bottom) = (y - LANE / 2.0, y + LANE / 2.0);
    let mut left = start - LANE;
    while left < end {
        let band = [
            [left, bottom],
            [left + BAND, bottom],
            [left + BAND + LANE, top],
            [left + LANE, top],
        ];
        let clipped = clip_x(&band, start, end);
        if clipped.len() >= 3 {
            shape = shape.fill(clipped, color);
        }
        left += PERIOD;
    }
    shape
}

/// A convex polygon cut to `left..=right`.
fn clip_x(points: &[[f32; 2]], left: f32, right: f32) -> Vec<[f32; 2]> {
    let cut = |points: Vec<[f32; 2]>, inside: &dyn Fn(f32) -> bool, edge: f32| {
        let mut out = Vec::with_capacity(points.len() + 2);
        for (i, &a) in points.iter().enumerate() {
            let b = points[(i + 1) % points.len()];
            if inside(a[0]) {
                out.push(a);
            }
            if inside(a[0]) != inside(b[0]) {
                let s = (edge - a[0]) / (b[0] - a[0]);
                out.push([edge, a[1] + (b[1] - a[1]) * s]);
            }
        }
        out
    };
    let points = cut(points.to_vec(), &|x| x >= left, left);
    if points.is_empty() {
        return points;
    }
    cut(points, &|x| x <= right, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripes_stay_inside_their_stretch() {
        let band = [[0.0, 6.0], [3.5, 6.0], [9.5, 0.0], [6.0, 0.0]];
        let clipped = clip_x(&band, 2.0, 8.0);
        assert!(clipped.iter().all(|[x, _]| (2.0..=8.0).contains(x)));
        assert!(clip_x(&band, 20.0, 30.0).is_empty());
    }

    #[test]
    fn a_tap_never_draws_ahead_of_the_playhead() {
        let width = 60.0;
        for &press in TOGGLE.presses.iter().chain(LOCK.presses) {
            let mut t = press.0;
            while t < DONE {
                let end = at(press.0, width) + drawn(press, MIN_PRESS, width, t);
                assert!(end <= at(t, width) + 1e-3, "{press:?} at {t} ms");
                t += 5.0;
            }
            let done = drawn(press, MIN_PRESS, width, DONE);
            assert!(done >= MIN_PRESS, "{press:?} finishes {done} wide");
        }
    }

    #[test]
    fn each_script_presses_the_key_while_it_records() {
        for script in [HOLD, TOGGLE, LOCK] {
            let (start, _) = script.recording;
            assert!(script.presses.iter().any(|&(d, _)| d == start));
            assert!(script.presses.iter().all(|&(d, u)| T0 <= d && u <= T1));
        }
    }
}
