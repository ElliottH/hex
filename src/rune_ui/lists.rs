//! Pieces the list-and-detail panes (Commands, Meetings) and the OS-only
//! Settings rows share: list rows and group labels, the detail header and
//! facts, tinted status tags, and the permission control.

use rune::{El, col, el, row, text};

use super::Ui;
use super::controls::{ButtonKind, DESCRIPTION, Paint, button, pulsing_dot};
use crate::desktop_i18n::tr;

const LIST_WIDTH: f32 = 320.0;
static CLOSE: &rune::Icon = &rune_theme_opencode::icons::CLOSE;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Tone {
    Neutral,
    Accent,
    Success,
    Danger,
}

/// A small tinted status label, never pressable.
pub(super) fn tone_tag(p: Paint, key: &str, label: &str, tone: Tone) -> El {
    let c = &p.k.c;
    let (fill, border, ink) = match tone {
        Tone::Neutral => (c.layer_02, c.border_base, c.text_muted),
        Tone::Accent => (c.info.bg, c.info.border, c.info.fg),
        Tone::Success => (c.success.bg, c.success.border, c.success.fg),
        Tone::Danger => (c.danger.bg, c.danger.border, c.danger.fg),
    };
    row()
        .key(key)
        .role("status")
        .label(label)
        .h(18.0)
        .px(6.0)
        .gap(4.0)
        .center()
        .radius(p.k.radius.xs)
        .fill(fill)
        .border(0.5, border)
        .child(
            text(label)
                .key("label")
                .font(p.k.ty.small_medium.lh(12.0))
                .color(ink),
        )
}

/// A list's group heading (a task, a day).
pub(super) fn group_label(p: Paint, key: &str, label: &str, first: bool) -> El {
    text(label)
        .key(key)
        .font(p.k.ty.small_medium)
        .color(p.faint())
        .px(10.0)
        .pt(if first { 4.0 } else { 16.0 })
        .pb(6.0)
}

/// A source list, as in a native sidebar: Tab enters at the selected row,
/// Up and Down select as they move, and typing selects a row by its name.
pub(super) fn source_list(list: El) -> El {
    list.role("list").focus_group().focus_follows()
}

/// A row of a [`source_list`]. Its ring is inset because rows touch and
/// their scroll view clips.
pub(super) fn source_item(p: Paint, item: El, selected: bool) -> El {
    item.focusable(true)
        .focus_selected(selected)
        .focus_ring(2.0, -2.0, p.k.c.border_focus)
}

/// A selectable list row: title (with trailing pieces) over a meta line.
pub(super) fn list_row(
    p: Paint,
    key: &str,
    title: &str,
    trailing: Vec<El>,
    meta: &str,
    selected: bool,
    select: impl Fn(&mut Ui) + 'static,
) -> El {
    let c = &p.k.c;
    let mut item = col()
        .key(key)
        .role("button")
        .label(title)
        .px(10.0)
        .py(8.0)
        .gap(2.0)
        .radius(p.k.radius.md)
        .hover(c.hover)
        .press(c.pressed)
        .on_press(select)
        .child(
            row()
                .key("top")
                .gap(8.0)
                .center_y()
                .child(
                    text(title)
                        .key("title")
                        .font(p.k.ty.ui_medium)
                        .color(p.text())
                        .grow(1.0)
                        .shrink()
                        .truncate(),
                )
                .children(trailing),
        )
        .child(
            text(meta)
                .key("meta")
                .font(p.k.ty.small)
                .color(p.muted())
                .truncate(),
        );
    if selected {
        item = item.fill(c.layer_02);
    }
    source_item(p, item, selected)
}

/// A quiet message in a list or detail (empty, no matches, failed load).
pub(super) fn message(p: Paint, key: &str, title: &str, detail: Option<&str>) -> El {
    col()
        .key(key)
        .px(10.0)
        .py(24.0)
        .gap(4.0)
        .child(
            text(title)
                .key("title")
                .font(p.k.ty.ui)
                .color(p.muted())
                .wrap(),
        )
        .children(detail.map(|detail| {
            text(detail)
                .key("detail")
                .font(DESCRIPTION)
                .color(p.danger())
                .wrap()
        }))
}

/// An error title and its detail, as a tinted panel.
pub(super) fn error_panel(p: Paint, key: &str, title: &str, detail: &str) -> El {
    let d = &p.k.c.danger;
    col()
        .key(key)
        .px(12.0)
        .py(10.0)
        .gap(2.0)
        .radius(p.k.radius.md)
        .fill(d.bg)
        .border(0.5, d.border)
        .child(text(title).key("title").font(p.k.ty.ui_medium).color(d.fg))
        .child(
            text(detail)
                .key("detail")
                .font(DESCRIPTION)
                .color(p.muted())
                .wrap(),
        )
}

/// The list column and the detail beside it, split by a hairline.
pub(super) fn split(p: Paint, key: &str, list: El, detail: El) -> El {
    row()
        .key(key)
        .grow(1.0)
        .shrink()
        .child(
            col()
                .key("list-column")
                .w(LIST_WIDTH)
                .h_full()
                .pl(16.0)
                .pr(8.0)
                .child(list),
        )
        .child(el().key("rule").w(p.hair).h_full().fill(p.k.c.border_weak))
        .child(detail)
}

pub(super) fn placeholder(p: Paint, key: &str, copy: &str) -> El {
    col().key(key).grow(1.0).shrink().center().child(
        text(copy)
            .key("placeholder")
            .font(p.k.ty.ui)
            .color(p.muted())
            .align(rune::Align::Center)
            .wrap()
            .w(280.0),
    )
}

/// The detail's title block: the title (or a rename field), a line of tags
/// and meta, and the actions on the right.
pub(super) fn detail_header(title: El, meta: Vec<El>, actions: Vec<El>) -> El {
    row()
        .key("detail-header")
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(
            col()
                .key("title")
                .grow(1.0)
                .shrink()
                .min_w(160.0)
                .gap(4.0)
                .child(title)
                .child(row().key("meta").gap(8.0).center_y().children(meta)),
        )
        .child(row().key("actions").gap(8.0).center_y().children(actions))
}

pub(super) fn detail_title(p: Paint, title: &str) -> El {
    text(title)
        .key("detail-title")
        .font(p.k.ty.body_medium)
        .color(p.text())
        .wrap()
}

pub(super) fn section_label(p: Paint, key: &str, label: &str) -> El {
    text(label)
        .key(key)
        .font(p.k.ty.small_medium)
        .color(p.muted())
}

/// A label and its value, side by side.
pub(super) fn fact(p: Paint, key: &str, label: &str, value: &str) -> El {
    row()
        .key(key)
        .gap(16.0)
        .child(
            text(label)
                .key("label")
                .font(DESCRIPTION)
                .color(p.muted())
                .w(120.0),
        )
        .child(
            text(value)
                .key("value")
                .font(DESCRIPTION)
                .color(p.text())
                .wrap()
                .grow(1.0)
                .shrink(),
        )
}

/// How a permission's control reads, everywhere Hex asks for one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PermissionControl {
    Allow,
    /// Asked: macOS's prompt or System Settings is up; Allow opens it again.
    Waiting,
    Allowed,
}

/// The one control every permission uses, in the tour and in Settings:
/// Allow, a pulsing dot while macOS is asked, then a green Allowed.
pub(super) fn permission_control(
    p: Paint,
    key: &str,
    control: PermissionControl,
    kind: ButtonKind,
    allow: impl Fn(&mut Ui) + 'static,
) -> El {
    match control {
        PermissionControl::Allowed => allowed(p, key, tr("Allowed")),
        PermissionControl::Allow => {
            button(p, key, tr("Allow"), kind, move |ui: &mut Ui, _| allow(ui))
        }
        PermissionControl::Waiting => row()
            .key(format!("{key}-waiting"))
            .gap(12.0)
            .center_y()
            .child(pulsing_dot(p.accent()))
            .child(button(
                p,
                key,
                tr("Allow"),
                ButtonKind::Secondary,
                move |ui: &mut Ui, _| allow(ui),
            )),
    }
}

/// A green check and label: a granted permission or a ready model.
pub(super) fn allowed(p: Paint, key: &str, label: &str) -> El {
    row()
        .key(key)
        .gap(6.0)
        .center_y()
        .child(
            rune::icon(p.k.glyph.check_small, 16.0)
                .key("check")
                .color(p.success()),
        )
        .child(
            text(label)
                .key("label")
                .font(DESCRIPTION)
                .color(p.success()),
        )
}

/// A compact secondary button for actions inside a list row.
pub(super) fn small_button(
    p: Paint,
    key: &str,
    label: &str,
    press: impl Fn(&mut Ui, &mut rune::Tx) + 'static,
) -> El {
    rune_kit::Button::new(label)
        .variant(rune_kit::ButtonVariant::Neutral)
        .size(rune_kit::ButtonSize::Small)
        .on_press_tx(press)
        .build(p.k)
        .key(key)
}

/// The × that removes one item of an editable list.
pub(super) fn remove_button(
    p: Paint,
    size: rune_kit::ButtonSize,
    press: impl Fn(&mut Ui) + 'static,
) -> El {
    rune_kit::IconButton::new(CLOSE)
        .variant(rune_kit::IconButtonVariant::Ghost)
        .size(size)
        .label(tr("Remove"))
        .on_press(press)
        .build(p.k)
        .key("remove")
}

/// A horizontal slider: the label and value over a track you press or drag.
/// `set` gets the position from zero through one and whether the gesture
/// ended (save then, not on every move).
pub(super) fn slider(
    p: Paint,
    key: &str,
    label: &str,
    value: f32,
    display: String,
    width: f32,
    set: impl Fn(&mut Ui, f32, bool) + 'static,
) -> El {
    let c = &p.k.c;
    let value = value.clamp(0.0, 1.0);
    let knob = 14.0;
    let track = row()
        .key("track")
        .role("slider")
        .label(label)
        .w(width)
        .h(22.0)
        .center_y()
        .on_drag(
            move |ui: &mut Ui, drag: &mut rune::Drag, _: &mut rune::Tx| {
                let position = ((drag.x - knob / 2.0) / (drag.w - knob).max(1.0)).clamp(0.0, 1.0);
                set(ui, position, drag.phase == rune::Phase::Up);
            },
        )
        .child(
            row()
                .key("rail")
                .w(width)
                .h(4.0)
                .radius(2.0)
                .fill(c.layer_03)
                .child(
                    el().key("fill")
                        .w((width - knob) * value + knob / 2.0)
                        .h(4.0)
                        .radius(2.0)
                        .fill(c.icon_accent),
                ),
        )
        .child(
            el().key("knob")
                .absolute((width - knob) * value, 4.0)
                .size(knob, knob)
                .radius(knob / 2.0)
                .fill(c.knob)
                .border(p.hair.max(0.5), c.border_strong),
        );
    col()
        .key(key)
        .w(width)
        .gap(4.0)
        .child(
            row()
                .key("heading")
                .justify_between()
                .child(text(label).key("label").font(DESCRIPTION).color(p.muted()))
                .child(
                    text(display)
                        .key("value")
                        .font(p.k.ty.mono)
                        .color(p.faint()),
                ),
        )
        .child(track)
}
