//! Rune-style controls for Hex's Settings: the card, row, switch, segmented
//! control, disclosure and inline menu, buttons, keycaps, and text field of
//! Rune's own `examples/settings`, extended where Hex needs more. Colours,
//! spacing, type, and motion come from `rune::tok`, never from Hex's GPUI
//! design tokens.

use rune::tok::{self, Theme, motion, radius, space, type_};
use rune::{Color, El, Icon, TextEdit, TextStyle, Tx, col, el, icon, oklch, row, text, text_input};

use super::Ui;

/// Row descriptions and secondary copy: one step under `type_::BODY`.
pub(super) const DESCRIPTION: TextStyle = TextStyle::new(12.0, 400.0);
const ROW_HEIGHT: f32 = 44.0;
const ROW_HEIGHT_DESCRIBED: f32 = 56.0;
const CONTROL_HEIGHT: f32 = 28.0;
const DISCLOSURE_WIDTH: f32 = 220.0;

pub(super) static CHEVRON_DOWN: Icon = Icon {
    name: "chevron-down",
    view: 16.0,
    body: r#"<path d="M5 6.5L8 9.5L11 6.5" stroke="currentColor"/>"#,
};
pub(super) static CHEVRON_UP: Icon = Icon {
    name: "chevron-up",
    view: 16.0,
    body: r#"<path d="M5 9.5L8 6.5L11 9.5" stroke="currentColor"/>"#,
};
pub(super) static CHECK: Icon = Icon {
    name: "check-small",
    view: 20.0,
    body: r#"<path d="M6.5 11.4412L8.97059 13.5L13.5 6.5" stroke="currentColor" stroke-linecap="square"/>"#,
};

/// Everything a control needs to paint itself.
#[derive(Clone, Copy)]
pub(super) struct Paint {
    pub(super) t: Theme,
    pub(super) accent: Color,
    pub(super) success: Color,
    pub(super) danger: Color,
    pub(super) hair: f32,
}

impl Paint {
    pub(super) fn new(dark: bool, scale: f32) -> Self {
        Self {
            t: tok::theme(dark),
            accent: tok::accent(0, dark),
            success: tok::accent(4, dark),
            // Rune's palette has no danger role yet; this sits beside its
            // accents (same lightness and chroma family, red hue).
            danger: oklch(if dark { 0.7 } else { 0.58 }, 0.17, 25.0),
            hair: 1.0 / scale,
        }
    }
}

/// A titled card of rows with hairline separators between them.
pub(super) fn section(p: Paint, key: &str, title: &str, rows: Vec<El>) -> El {
    let count = rows.len();
    let mut card = col()
        .key("card")
        .padding(space::S1)
        .fill(p.t.card)
        .radius(radius::LG)
        .border(p.hair, p.t.card_border)
        .shadow(1.0, 2.0, p.t.shadow)
        .shadow(8.0, 24.0, p.t.shadow_soft);
    for (index, row) in rows.into_iter().enumerate() {
        card = card.child(row);
        if index + 1 < count {
            card = card.child(separator(p, index));
        }
    }
    col()
        .key(key)
        .gap(space::S2)
        .child(
            text(title)
                .key("header")
                .font(type_::SECTION)
                .color(p.t.text_secondary)
                .ml(space::S3),
        )
        .child(card)
}

fn separator(p: Paint, index: usize) -> El {
    el().key(format!("sep{index}"))
        .h(p.hair)
        .ml(space::S3)
        .mr(space::S3)
        .fill(p.t.separator)
}

/// A setting: title and optional description on the left, its control on
/// the right.
pub(super) fn setting_row(
    p: Paint,
    key: &str,
    title: &str,
    description: Option<&str>,
    control: El,
) -> El {
    described_row(
        p,
        key,
        title,
        description.map(|text| (text.to_string(), p.t.text_secondary)),
        control,
    )
}

/// A [`setting_row`] whose description has its own colour (errors, progress).
pub(super) fn described_row(
    p: Paint,
    key: &str,
    title: &str,
    description: Option<(String, Color)>,
    control: El,
) -> El {
    let height = if description.is_some() {
        ROW_HEIGHT_DESCRIBED
    } else {
        ROW_HEIGHT
    };
    row()
        .key(key)
        .min_h(height)
        .px(space::S3)
        .py(space::S2)
        .gap(space::S6)
        .center_y()
        .justify_between()
        .radius(radius::MD)
        .label(title)
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(text(title).key("label").font(type_::BODY).color(p.t.text))
                .children(description.map(|(copy, color)| {
                    text(copy)
                        .key("description")
                        .font(DESCRIPTION)
                        .color(color)
                        .wrap()
                })),
        )
        .child(control)
}

/// A whole-row switch: the row presses, the switch shows the state.
pub(super) fn toggle_row(
    p: Paint,
    key: &str,
    title: &str,
    description: &str,
    on: bool,
    flip: impl Fn(&mut Ui) + 'static,
) -> El {
    setting_row(p, key, title, Some(description), switch(p, on))
        .role("toggle")
        .hover(p.t.hover)
        .press(p.t.press)
        .on_press(flip)
}

fn switch(p: Paint, on: bool) -> El {
    let (w, h, k) = (38.0, 22.0, 18.0);
    let inset = (h - k) / 2.0;
    el().key("switch")
        .role("switch")
        .size(w, h)
        .radius(h / 2.0)
        .fill(if on { p.accent } else { p.t.control_off })
        .motion(motion::SNAPPY)
        .child(
            el().key("knob")
                .size(k, k)
                .absolute(if on { w - k - inset } else { inset }, inset)
                .radius(k / 2.0)
                .fill(p.t.knob)
                .shadow(1.0, 3.0, p.t.knob_shadow)
                .shadow(0.0, 0.5, p.t.edge),
        )
}

/// One segment's width for its label at `type_::LABEL`.
pub(super) fn segment_width(label: &str) -> f32 {
    (label.chars().count() as f32 * 6.8 + 24.0)
        .max(52.0)
        .round()
}

/// A few exclusive choices with a sliding indicator, as in Rune's Theme
/// control. `pick` receives the chosen index.
pub(super) fn segmented(
    p: Paint,
    key: &str,
    labels: &[&str],
    selected: usize,
    pick: fn(&mut Ui, usize),
) -> El {
    let (h, pad) = (28.0, 2.0);
    let widths: Vec<f32> = labels.iter().map(|label| segment_width(label)).collect();
    let total: f32 = widths.iter().sum();
    let left: f32 = widths[..selected.min(widths.len())].iter().sum();
    let mut segments = row().key("segments");
    for (index, label) in labels.iter().enumerate() {
        let chosen = index == selected;
        segments = segments.child(
            el().key(format!("segment{index}"))
                .role("segment")
                .label(*label)
                .size(widths[index], h - 2.0 * pad)
                .center()
                .radius(radius::SM)
                .hover(if chosen {
                    Color::TRANSPARENT
                } else {
                    p.t.hover
                })
                .on_press(move |ui: &mut Ui| pick(ui, index))
                .child(
                    text(*label)
                        .key("label")
                        .font(type_::LABEL)
                        .color(if chosen { p.t.text } else { p.t.text_secondary }),
                ),
        );
    }
    el().key(key)
        .role("segmented")
        .size(total + 2.0 * pad, h)
        .padding(pad)
        .radius(radius::MD)
        .fill(p.t.control_track)
        .motion(motion::SNAPPY)
        .child(
            el().key("indicator")
                .size(widths.get(selected).copied().unwrap_or(0.0), h - 2.0 * pad)
                .absolute(pad + left, pad)
                .radius(radius::SM)
                .fill(p.t.indicator)
                .shadow(1.0, 3.0, p.t.shadow)
                .shadow(0.0, 0.5, p.t.edge)
                .lead(motion::LEAD, motion::DEFAULT),
        )
        .child(segments)
}

/// A dropdown trigger. Its choices open inline under the row (see
/// [`menu`]): Rune has no anchored popover layer yet.
pub(super) fn disclosure(
    p: Paint,
    key: &str,
    label: &str,
    open: bool,
    toggle: impl Fn(&mut Ui) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .w(DISCLOSURE_WIDTH)
        .h(CONTROL_HEIGHT)
        .pl(space::S3)
        .pr(space::S2)
        .gap(space::S2)
        .center_y()
        .radius(radius::MD)
        .fill(p.t.control_track)
        .hover(p.t.hover)
        .press(p.t.press)
        .on_press(toggle)
        .child(
            text(label)
                .key("value")
                .font(type_::LABEL)
                .color(p.t.text)
                .grow(1.0)
                .truncate(),
        )
        .child(
            icon(if open { &CHEVRON_UP } else { &CHEVRON_DOWN }, 16.0)
                .key("chevron")
                .color(p.t.text_secondary),
        )
}

/// The open choices of a [`disclosure`], as rows inside the same card.
pub(super) fn menu(p: Paint, key: &str, items: Vec<El>) -> El {
    col()
        .key(key)
        .role("menu")
        .mt(space::S1)
        .mb(space::S1)
        .ml(space::S3)
        .mr(space::S3)
        .padding(space::S1)
        .radius(radius::MD)
        .fill(p.t.control_track)
        .children(items)
}

pub(super) fn menu_item(
    p: Paint,
    key: &str,
    label: &str,
    detail: Option<&str>,
    selected: bool,
    choose: impl Fn(&mut Ui) + 'static,
) -> El {
    row()
        .key(key)
        .role("menuitem")
        .label(label)
        .h(30.0)
        .px(space::S2)
        .gap(space::S2)
        .center_y()
        .radius(radius::SM)
        .hover(p.t.hover)
        .press(p.t.press)
        .on_press(choose)
        .child(
            el().key("check")
                .size(16.0, 16.0)
                .children(selected.then(|| icon(&CHECK, 16.0).key("icon").color(p.accent))),
        )
        .child(
            text(label)
                .key("label")
                .font(type_::BODY)
                .color(p.t.text)
                .grow(1.0)
                .truncate(),
        )
        .children(detail.map(|detail| {
            text(detail)
                .key("detail")
                .font(DESCRIPTION)
                .color(p.t.text_secondary)
        }))
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ButtonKind {
    /// The one main action on screen: accent fill.
    Primary,
    Secondary,
}

pub(super) fn button(
    p: Paint,
    key: &str,
    label: &str,
    kind: ButtonKind,
    press: impl Fn(&mut Ui, &mut Tx) + 'static,
) -> El {
    let (fill, ink) = match kind {
        ButtonKind::Primary => (p.accent, Color::WHITE),
        ButtonKind::Secondary => (p.t.control_track, p.t.text),
    };
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(CONTROL_HEIGHT)
        .px(space::S3)
        .center()
        .radius(radius::MD)
        .fill(fill)
        .hover(p.t.hover)
        .press(p.t.press)
        .on_press_tx(press)
        .child(text(label).key("label").font(type_::LABEL).color(ink))
}

pub(super) fn keycaps(p: Paint, key: &str, parts: &[String]) -> El {
    row()
        .key(key)
        .gap(3.0)
        .children(parts.iter().enumerate().map(|(index, part)| {
            row()
                .key(format!("cap{index}"))
                .h(22.0)
                .px(7.0)
                .center()
                .radius(radius::SM)
                .fill(p.t.card)
                .border(p.hair, p.t.card_border)
                .shadow(0.0, 0.5, p.t.edge)
                .child(
                    text(part.clone())
                        .key("label")
                        .font(type_::LABEL)
                        .color(p.t.text),
                )
        }))
}

pub(super) fn status_dot(color: Color) -> El {
    el().key("dot").size(8.0, 8.0).radius(4.0).fill(color)
}

/// A small status label: tinted, never pressable.
pub(super) fn tag(key: &str, label: &str, color: Color) -> El {
    row()
        .key(key)
        .h(20.0)
        .px(space::S2)
        .center()
        .radius(radius::SM)
        .fill(color.alpha(0.16))
        .child(text(label).key("label").font(type_::LABEL).color(color))
}

/// A one-line note inside a card, between rows.
pub(super) fn note(key: &str, copy: &str, color: Color) -> El {
    row().key(key).px(space::S3).py(space::S2).child(
        text(copy)
            .key("copy")
            .font(DESCRIPTION)
            .color(color)
            .wrap()
            .grow(1.0),
    )
}

pub(super) fn text_field(
    p: Paint,
    key: &str,
    edit: &TextEdit,
    placeholder: &str,
    access: fn(&mut Ui) -> &mut TextEdit,
    submit: fn(&mut Ui, &mut Tx),
) -> El {
    row()
        .key(key)
        .w(DISCLOSURE_WIDTH + 60.0)
        .min_h(CONTROL_HEIGHT)
        .px(space::S2)
        .py(5.0)
        .radius(radius::MD)
        .fill(p.t.control_track)
        .border(p.hair, p.t.card_border)
        .child(
            text_input(edit, type_::BODY)
                .key("input")
                .w_full()
                .color(p.t.text)
                .placeholder(placeholder, p.t.text_secondary)
                .caret(p.accent, p.accent.alpha(0.3))
                .on_edit(access)
                .on_submit(submit),
        )
}
