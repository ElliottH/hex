//! Settings controls on `rune-kit`, themed with OpenCode's design language
//! (`rune-theme-opencode`): the card, row, switch, segmented control,
//! select-style menus, buttons, keycaps, note, and text field. Colours,
//! type, radii, elevation, and motion come from the kit's tokens.

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
/// Narrower than this, a row moves its control below the copy instead of
/// squeezing the description to a word per line (tiling desktops ignore
/// the window's 860 px minimum width).
const COPY_MIN_WIDTH: f32 = 100.0;

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
                .font(p.k.ty.ui_medium.lh(16.0))
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
        .gaps(8.0, 24.0)
        .center_y()
        .justify_between()
        .flex_wrap()
        .radius(p.k.radius.md)
        .label(title)
        .child(
            col()
                .key("copy")
                .w(COPY_MIN_WIDTH)
                .min_w(COPY_MIN_WIDTH)
                .grow(1.0)
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
    pub(super) label: String,
    pub(super) selected: bool,
    pub(super) choose: Box<dyn Fn(&mut Ui)>,
}

/// A select field whose choices open in an anchored menu above the page.
pub(super) fn select(
    p: Paint,
    key: &str,
    label: &str,
    open: bool,
    choices: Vec<Choice>,
    toggle: impl Fn(&mut Ui) + 'static,
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
        .on_close(|ui: &mut Ui| ui.open = None);
    for choice in choices {
        menu = menu.item(
            MenuItem::new(choice.label)
                .checked(choice.selected)
                .on_select(move |ui: &mut Ui| (choice.choose)(ui)),
        );
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
                        .font(p.k.ty.small_medium.lh(12.0))
                        .color(c.text_base),
                )
        }))
}

pub(super) fn status_dot(color: Color) -> El {
    el().key("dot").size(8.0, 8.0).radius(4.0).fill(color)
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

/// The keyboard highlight in a search popover's results: Up and Down move
/// it, Enter picks it, and it returns to the first result whenever the query
/// changes.
#[derive(Default)]
pub(super) struct Highlight {
    query: String,
    index: usize,
}

impl Highlight {
    /// The highlighted result among `count` for `query`.
    pub(super) fn index(&self, query: &str, count: usize) -> Option<usize> {
        if count == 0 {
            None
        } else if self.query == query {
            Some(self.index.min(count - 1))
        } else {
            Some(0)
        }
    }

    /// Moves the highlight for Up or Down; false for any other key.
    pub(super) fn key(&mut self, key: &rune::Key, query: &str, count: usize) -> bool {
        let down = match key {
            rune::Key::Down => true,
            rune::Key::Up => false,
            _ => return false,
        };
        let current = self.index(query, count).unwrap_or(0);
        self.query = query.to_string();
        self.index = if down {
            (current + 1).min(count.saturating_sub(1))
        } else {
            current.saturating_sub(1)
        };
        true
    }
}
