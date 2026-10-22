//! Settings controls on `rune-kit`, themed with OpenCode's design language
//! (`rune-theme-opencode`): the card, row, switch, segmented control,
//! select-style menus, buttons, keycaps, tag, note, and text field. Colours,
//! type, radii, elevation, and motion come from the kit's tokens, never from
//! Hex's GPUI design tokens.

use rune::{Color, El, TextEdit, TextStyle, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonSize, ButtonVariant, Kit, Menu, MenuItem, Placement, SegmentedControl, Select,
    SelectAppearance, Switch, TextInput,
};

use super::Ui;

/// Row descriptions and secondary copy.
pub(super) const DESCRIPTION: TextStyle = TextStyle::new(12.0, 440.0).lh(16.0);
const ROW_HEIGHT: f32 = 44.0;
const ROW_HEIGHT_DESCRIBED: f32 = 56.0;
const SELECT_WIDTH: f32 = 220.0;

/// Everything a control needs to paint itself.
#[derive(Clone, Copy)]
pub(super) struct Paint {
    pub(super) k: &'static Kit,
    pub(super) hair: f32,
}

impl Paint {
    pub(super) fn new(dark: bool, scale: f32) -> Self {
        Self {
            k: rune_theme_opencode::kit(dark),
            hair: 1.0 / scale,
        }
    }
    pub(super) fn text(self) -> Color {
        self.k.c.text_base
    }
    pub(super) fn muted(self) -> Color {
        self.k.c.text_muted
    }
    pub(super) fn faint(self) -> Color {
        self.k.c.text_faint
    }
    pub(super) fn danger(self) -> Color {
        self.k.c.danger.fg
    }
    pub(super) fn success(self) -> Color {
        self.k.c.success.fg
    }
    pub(super) fn accent(self) -> Color {
        self.k.c.text_accent
    }
}

/// A titled card of rows with hairline separators between them.
pub(super) fn section(p: Paint, key: &str, title: &str, rows: Vec<El>) -> El {
    col()
        .key(key)
        .gap(8.0)
        .child(
            text(title)
                .key("header")
                .font(TextStyle {
                    line_height: 16.0,
                    ..p.k.ty.ui_medium
                })
                .color(p.k.c.text_muted)
                .ml(12.0),
        )
        .child(card(p, "card", rows))
}

/// A raised card of rows with hairline separators between them.
pub(super) fn card(p: Paint, key: &str, rows: Vec<El>) -> El {
    let c = &p.k.c;
    let count = rows.len();
    let mut card = col()
        .key(key)
        .padding(4.0)
        .fill(c.bg_base)
        .radius(p.k.radius.lg);
    for shadow in &p.k.elevation.raised {
        card = card.box_shadow(*shadow);
    }
    for (index, row) in rows.into_iter().enumerate() {
        card = card.child(row);
        if index + 1 < count {
            card = card.child(separator(p, index));
        }
    }
    card
}

fn separator(p: Paint, index: usize) -> El {
    el().key(format!("sep{index}"))
        .h(p.hair)
        .ml(12.0)
        .mr(12.0)
        .fill(p.k.c.border_muted)
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
        description.map(|text| (text.to_string(), p.muted())),
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
        .px(12.0)
        .py(8.0)
        .gap(24.0)
        .center_y()
        .justify_between()
        .radius(p.k.radius.md)
        .label(title)
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(text(title).key("label").font(p.k.ty.ui).color(p.text()))
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
    setting_row(
        p,
        key,
        title,
        Some(description),
        Switch::new(on).build(p.k).key("switch"),
    )
    .role("toggle")
    .hover(p.k.c.hover)
    .press(p.k.c.pressed)
    .on_press(flip)
}

/// A few exclusive choices: equal segments sized for the longest label.
pub(super) fn segmented(
    p: Paint,
    key: &str,
    labels: &[&str],
    selected: usize,
    pick: fn(&mut Ui, usize),
) -> El {
    // Wide (CJK) characters take about twice a Latin letter's advance.
    let width = |label: &str| {
        label
            .chars()
            .map(|c| if c as u32 >= 0x2E80 { 13.5 } else { 8.0 })
            .sum::<f32>()
            + 28.0
    };
    let widest = labels
        .iter()
        .map(|label| width(label))
        .fold(60.0, f32::max)
        .round();
    SegmentedControl::new(labels, Some(selected))
        .width(Some(widest * labels.len() as f32))
        .on_pick(move |ui: &mut Ui, index: usize, _: &mut Tx| pick(ui, index))
        .build(p.k)
        .key(key)
        .role("segmented")
}

/// One choice of a [`select`].
pub(super) struct Choice {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) detail: Option<String>,
    pub(super) selected: bool,
    pub(super) choose: Box<dyn Fn(&mut Ui)>,
}

/// A select field whose choices open in an anchored menu above the page.
#[allow(clippy::too_many_arguments)] // one call per settings row; all are the row's data
pub(super) fn select(
    p: Paint,
    key: &str,
    label: &str,
    open: bool,
    choices: Vec<Choice>,
    toggle: impl Fn(&mut Ui) + 'static,
    close: fn(&mut Ui),
    layer: &mut Vec<El>,
) -> El {
    let mut discard = Vec::new();
    let trigger = Select::new::<&str>(format!("{key}-field"), &[])
        .appearance(SelectAppearance::Field)
        .width(SELECT_WIDTH)
        .current(Some(label))
        .open(open)
        .on_toggle(toggle)
        .build(p.k, &mut discard);
    let mut menu = Menu::new(key)
        .trigger(trigger)
        .placement(Placement::BottomEnd)
        .gutter(4.0)
        .min_width(SELECT_WIDTH)
        .open(open)
        .on_close(close);
    for choice in choices {
        let Choice {
            key,
            label,
            detail,
            selected,
            choose,
        } = choice;
        let mut item = MenuItem::new(label)
            .checked(selected)
            .on_select(move |ui: &mut Ui| choose(ui));
        if let Some(detail) = detail {
            item = item.shortcut(detail);
        }
        let _ = key;
        menu = menu.item(item);
    }
    menu.build(p.k, layer)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ButtonKind {
    /// The one main action on screen: the kit's contrast button.
    Primary,
    Secondary,
    /// Destructive or armed confirmations.
    Danger,
}

pub(super) fn button(
    p: Paint,
    key: &str,
    label: &str,
    kind: ButtonKind,
    press: impl Fn(&mut Ui, &mut Tx) + 'static,
) -> El {
    let variant = match kind {
        ButtonKind::Primary => ButtonVariant::Contrast,
        ButtonKind::Secondary => ButtonVariant::Neutral,
        ButtonKind::Danger => ButtonVariant::Danger,
    };
    Button::new(label)
        .variant(variant)
        .size(ButtonSize::Normal)
        .on_press_tx(press)
        .build(p.k)
        .key(key)
}

/// Keycaps for a shortcut, in the kit's keybind look at a readable size.
pub(super) fn keycaps(p: Paint, key: &str, parts: &[String]) -> El {
    let c = &p.k.c;
    row()
        .key(key)
        .gap(2.0)
        .children(parts.iter().enumerate().map(|(index, part)| {
            row()
                .key(format!("cap{index}"))
                .min_w(20.0)
                .h(20.0)
                .px(6.0)
                .center()
                .radius(p.k.radius.sm)
                .fill(c.layer_03)
                .child(
                    text(part.clone())
                        .key("label")
                        .font(TextStyle {
                            line_height: 12.0,
                            ..p.k.ty.small_medium
                        })
                        .color(c.text_base),
                )
        }))
}

pub(super) fn status_dot(color: Color) -> El {
    el().key("dot").size(8.0, 8.0).radius(4.0).fill(color)
}

/// A small status label: tinted, never pressable.
pub(super) fn tag(p: Paint, key: &str, label: &str) -> El {
    let s = &p.k.c.success;
    row()
        .key(key)
        .h(18.0)
        .px(6.0)
        .center()
        .radius(p.k.radius.xs)
        .fill(s.bg)
        .border(0.5, s.border)
        .child(
            text(label)
                .key("label")
                .font(TextStyle {
                    line_height: 12.0,
                    ..p.k.ty.small_medium
                })
                .color(s.fg),
        )
}

/// A one-line note inside a card, between rows.
pub(super) fn note(key: &str, copy: &str, color: Color) -> El {
    row().key(key).px(12.0).py(8.0).child(
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
    TextInput::new(edit)
        .placeholder(placeholder)
        .width(Some(280.0))
        .on_edit(access)
        .on_submit(submit)
        .build(p.k)
        .key(key)
}
