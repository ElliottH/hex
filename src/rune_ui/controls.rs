//! Settings controls on `rune-kit`, themed with OpenCode's design language
//! (`rune-theme-opencode`): the card, row, switch, segmented control,
//! select-style menus, buttons, keycaps, note, and text field. Colours,
//! type, radii, elevation, and motion come from the kit's tokens.

use rune::{
    Color, El, LoopSpec, SpringPair, TextEdit, TextStyle, Transition, Tx, col, el, row, text,
};
#[allow(deprecated)]
use rune_kit::{
    Button, ButtonSize, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Menu,
    MenuItem, Placement, SegmentedControl, Select, SelectAppearance, Switch, TextInput,
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

/// A row of a [`card`]: always there, or a slot of rows that folds away.
pub(super) enum CardRow {
    Shown(Box<El>),
    /// The slot stays in the card and clips its rows, so they close under
    /// the rows below as those slide up, rather than fading over them, and
    /// the separator above folds with them.
    Folding {
        key: &'static str,
        rows: Vec<CardRow>,
    },
}

impl From<El> for CardRow {
    fn from(row: El) -> Self {
        CardRow::Shown(Box::new(row))
    }
}

/// A [`CardRow::Folding`] slot keyed `key`, open while it holds rows.
pub(super) fn folding<R: Into<CardRow>>(
    key: &'static str,
    rows: impl IntoIterator<Item = R>,
) -> CardRow {
    CardRow::Folding {
        key,
        rows: rows.into_iter().map(Into::into).collect(),
    }
}

/// A titled card of rows with hairline separators between them.
pub(super) fn section(p: Paint, key: &str, title: &str, rows: Vec<impl Into<CardRow>>) -> El {
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
pub(super) fn card(p: Paint, key: &str, rows: Vec<impl Into<CardRow>>) -> El {
    let c = &p.k.c;
    let mut card = col()
        .key(key)
        .padding(4.0)
        .fill(c.bg_base)
        .radius(p.k.radius.lg);
    for shadow in &p.k.elevation.raised {
        card = card.box_shadow(*shadow);
    }
    card.children(stack(p, rows.into_iter().map(Into::into).collect()))
}

/// `rows` as a card's children, a separator between each pair.
fn stack(p: Paint, rows: Vec<CardRow>) -> Vec<El> {
    let mut children = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        match row {
            CardRow::Shown(row) => {
                children.extend((index > 0).then(|| separator(p, format!("sep{}", index - 1))));
                children.push(*row);
            }
            CardRow::Folding { key, rows } => {
                let lead =
                    (index > 0 && !rows.is_empty()).then(|| separator(p, "sep-above".into()));
                let rows = lead.into_iter().chain(stack(p, rows));
                children.push(
                    col()
                        .key(key)
                        .clip_soft(0.0)
                        .children(rows.map(|row| row.transition(folds()))),
                );
            }
        }
    }
    children
}

/// How a folding slot's rows come and go. The slot's clip does most of the
/// work, so they fade only partway, and leave on a slower spring than the
/// slot closes on, so no sliver of them pops away.
fn folds() -> Transition {
    let mut transition = Transition::opacity().springs(SpringPair::new(
        rune::Spring::new(0.3, 0.0),
        rune::Spring::new(0.45, 0.0),
    ));
    transition.insertion.opacity = 0.4;
    transition.removal.opacity = 0.4;
    transition
}

fn separator(p: Paint, key: String) -> El {
    el().key(key)
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
    let description = description.map(|(copy, color)| {
        text(copy)
            .key("description")
            .font(DESCRIPTION)
            .color(color)
            .wrap()
    });
    copy_row(p, key, title, description, control, false)
}

/// A [`setting_row`] with a one-line link under its title, as tight as a
/// row without one so General still fits the window.
pub(super) fn link_row(p: Paint, key: &str, title: &str, link: El, control: El) -> El {
    copy_row(p, key, title, Some(link), control, true)
}

fn copy_row(
    p: Paint,
    key: &str,
    title: &str,
    description: Option<El>,
    control: El,
    compact: bool,
) -> El {
    let height = if description.is_some() && !compact {
        ROW_HEIGHT_DESCRIBED
    } else {
        ROW_HEIGHT
    };
    row()
        .key(key)
        .min_h(height)
        .px(12.0)
        .py(if compact { 4.0 } else { 8.0 })
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
                .gap(if compact { 0.0 } else { 2.0 })
                .child(text(title).key("label").font(p.k.ty.ui).color(p.text()))
                .children(description),
        )
        .child(control)
}

/// The switch of a row that toggles: the row takes focus and presses, and
/// the switch shows its state and the focus ring.
pub(super) fn row_switch(p: Paint, on: bool) -> El {
    Switch::new(on).build(p.k).key("switch").focusable(false)
}

/// A whole-row switch: the row presses, the switch shows the state.
pub(super) fn toggle_row<'a>(
    p: Paint,
    key: &str,
    title: &str,
    description: impl Into<Option<&'a str>>,
    on: bool,
    flip: impl Fn(&mut Ui) + 'static,
) -> El {
    setting_row(p, key, title, description.into(), row_switch(p, on))
        .role("toggle")
        .focusable(true)
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
    pick: impl Fn(&mut Ui, usize) + Clone + 'static,
) -> El {
    segmented_tx(p, key, labels, selected, move |ui, index, _| {
        pick(ui, index)
    })
}

/// A [`segmented`] whose pick also gets the transaction, to swap views
/// without animating.
pub(super) fn segmented_tx(
    p: Paint,
    key: &str,
    labels: &[&str],
    selected: usize,
    pick: impl Fn(&mut Ui, usize, &mut Tx) + Clone + 'static,
) -> El {
    SegmentedControl::new(labels, Some(selected))
        .fit()
        .on_pick(pick)
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
    let trigger = Select::new::<&str>(format!("{key}-field"), &[])
        .appearance(SelectAppearance::Field)
        .width(SELECT_WIDTH)
        .current(Some(label))
        .open(open)
        .on_toggle(toggle)
        .trigger(p.k);
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

// Hex's push buttons keep Rune's older `ButtonSize` metrics (28 pt, 11 pt
// padding, medium radius); `control_size` would widen and round them.
#[allow(deprecated)]
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

/// A Secondary button showing a value, with a trailing chevron because it
/// opens a sheet rather than a menu. It is as wide as its value.
#[allow(deprecated)]
pub(super) fn sheet_button(
    p: Paint,
    key: &str,
    value: &str,
    open: impl Fn(&mut Ui) + 'static,
) -> El {
    Button::new(value)
        .variant(ButtonVariant::Neutral)
        .size(ButtonSize::Normal)
        .on_press(open)
        .build(p.k)
        .key(key)
        .pr(6.0)
        .child(
            rune::icon(&rune_theme_opencode::icons::CHEVRON_RIGHT, 16.0)
                .key("chevron")
                .color(p.k.c.icon_muted),
        )
}

/// An icon-only Secondary button the height of [`button`].
pub(super) fn icon_button(
    p: Paint,
    key: &str,
    icon: &'static rune::Icon,
    label: &str,
    press: impl Fn(&mut Ui) + 'static,
) -> El {
    IconButton::new(icon)
        .variant(IconButtonVariant::Neutral)
        .control_size(ControlSize::Large)
        .label(label)
        .on_press(press)
        .build(p.k)
        .key(key)
}

/// How a modal's surface enters and leaves: scaled from 0.96 about its
/// centre, with a fade. Its text scales with its boxes, so the panel moves
/// as one surface.
pub(super) fn modal_transition(p: Paint) -> Transition {
    Transition::scale(0.96).springs(overlay_springs(p))
}

/// The scrim behind a modal: the surface's springs, fading only.
pub(super) fn modal_scrim_transition(p: Paint) -> Transition {
    Transition::opacity().springs(overlay_springs(p))
}

fn overlay_springs(p: Paint) -> SpringPair {
    SpringPair::new(p.k.motion.overlay, p.k.motion.overlay)
}

/// How big a keycap is: `Small` inside a control, `Large` on its own.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum KeySize {
    Small,
    Large,
}

impl KeySize {
    /// Height, including the lip below the face.
    pub(super) const fn height(self) -> f32 {
        match self {
            Self::Small => 22.0,
            Self::Large => 26.0,
        }
    }

    pub(super) const fn min_width(self) -> f32 {
        match self {
            Self::Small => 22.0,
            Self::Large => 28.0,
        }
    }
}

/// How far a keycap's face stands above its base: the lip showing below it.
const KEY_DEPTH: f32 = 2.5;
/// A press sinks the face most of the way, leaving a hairline of lip.
const KEY_TRAVEL: f32 = 2.0;
/// Seconds for the face to sink; it rises on the controls' spring.
const KEY_PRESS: f32 = 0.07;
/// Between the keycaps of a chord.
pub(super) const KEY_GAP: f32 = 3.0;

/// One keycap per key of a shortcut.
pub(super) fn keycaps(p: Paint, key: &str, parts: &[String]) -> El {
    row().key(key).gap(KEY_GAP).children(
        parts
            .iter()
            .enumerate()
            .map(|(index, part)| keycap(p, index, part, KeySize::Small, false)),
    )
}

/// One key: a face on a base whose lip shows below it. A press sinks the
/// face into the lip; the key's bounds never change.
pub(super) fn keycap(p: Paint, index: usize, label: &str, size: KeySize, down: bool) -> El {
    let c = &p.k.c;
    let face_height = size.height() - KEY_DEPTH;
    // Inside a control the cap nests in the control's corners.
    let radius = match size {
        KeySize::Small => p.k.radius.sm,
        KeySize::Large => p.k.radius.md,
    };
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
        .radius(radius)
        .fill(c.border_base);
    let face = row()
        .key("cap")
        .min_w(size.min_width())
        .h(face_height)
        .px(6.0)
        .center()
        .radius(radius)
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
        .h(size.height())
        .child(base)
        .child(face)
}

pub(super) fn status_dot(color: Color) -> El {
    el().key("dot").size(8.0, 8.0).radius(4.0).fill(color)
}

/// Breathes between full and this opacity every two seconds; full under
/// Reduce Motion.
const PULSE: LoopSpec = LoopSpec::opacity(2.0, 0.0, 1.0, 0.45);

/// A dot for something live: recording, listening, or waiting on macOS.
pub(super) fn pulsing_dot(color: Color) -> El {
    status_dot(color).looping(PULSE)
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
