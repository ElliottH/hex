//! The Voice Action pane: the opt-in, its shortcut, and the OpenCode model
//! that turns spoken instructions into paste-ready text. Enabling checks the
//! saved shortcut against the others first, as GPUI does; the shortcut is
//! recorded through the service like every Linux shortcut.

use rune::{El, col};

use super::ShortcutTarget;
use super::Ui;
use super::controls::{Paint, card, note, section, setting_row, toggle_row};
use super::models::{self, Target};
use super::settings::{cancel_capture, hotkey_control};
use crate::app_window::{OpenCodeFeature, set_voice_action_enabled, voice_action_description};
use crate::desktop_i18n::tr;

fn toggle(ui: &mut Ui) {
    cancel_capture(ui);
    let enabled = !ui.settings.voice_action.enabled;
    let mut candidate = ui.drafted();
    match set_voice_action_enabled(&mut candidate, enabled) {
        Ok(()) => {
            if ui.commit(candidate) {
                ui.voice_action_error = None;
            } else {
                ui.voice_action_error = ui.error.clone();
            }
        }
        Err(error) => ui.voice_action_error = Some(error.to_string()),
    }
}

pub(super) fn pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let enabled = ui.settings.voice_action.enabled;
    let mut content = col().key("content").px(24.0).pt(8.0).pb(24.0).gap(24.0);
    let mut enable = vec![toggle_row(
        p,
        "voice-action-enabled",
        tr("Enable Voice Action"),
        voice_action_description(enabled),
        enabled,
        toggle,
    )];
    if let Some(error) = &ui.voice_action_error {
        enable.push(note(
            "voice-action-error",
            &format!("{} {error}", tr("Could not update Voice Action:")),
            p.danger(),
        ));
    }
    content = content.child(card(p, "enable", enable));
    if ui.records_shortcuts() {
        content = content.child(section(
            p,
            "capture",
            tr("Capture"),
            vec![setting_row(
                p,
                "voice-action-shortcut",
                tr("Shortcut"),
                Some(if enabled {
                    tr("Hold to speak; selected text is included automatically")
                } else {
                    tr("Saved for when Voice Action is enabled")
                }),
                rune::row()
                    .key("voice-action-shortcut-control")
                    .gap(8.0)
                    .center_y()
                    .children(super::platform_settings::side_selector(
                        ui,
                        p,
                        ShortcutTarget::VoiceAction,
                    ))
                    .child(hotkey_control(ui, p, ShortcutTarget::VoiceAction)),
            )],
        ));
    }
    if enabled {
        let rows = match models::unavailable(ui, p, OpenCodeFeature::VoiceAction) {
            Some(notice) => vec![notice],
            None => models::rows(ui, p, Target::VoiceAction, layer),
        };
        content = content.child(section(p, "processing", tr("Processing"), rows));
    }
    super::scroll_body("scroll", content)
}
