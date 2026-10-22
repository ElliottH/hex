//! Settings rows only some OSes show, chosen by
//! [`Capabilities`](super::platform::Capabilities): macOS's permission
//! warnings, Meetings section, Show Dock icon, and the experimental Hex
//! Microphone; Launch at login on macOS and Windows; and the voice command
//! model notice. Their state is [`PlatformState`](super::platform::PlatformState)
//! data and their actions are [`PlatformRequest`]s.

use rune::{El, col, row, text};
use rune_kit::Switch;

use super::Ui;
use super::controls::{
    ButtonKind, DESCRIPTION, Paint, button, note, section, segmented, setting_row, toggle_row,
};
use super::lists::{PermissionControl, Tone, permission_control, small_button, tone_tag};
use super::platform::{MeetingRequest, NotesAvailability, PlatformRequest};
use crate::desktop_i18n::{tr, tr_fill};
use crate::hex_microphone::DriverActionState;
use crate::login_item::LoginItemStatus;
use crate::onboarding::{PermissionKind, PermissionWarning};
use crate::voice_filter::VoiceFilterMode;

/// A permission's name as System Settings lists it, and what Hex cannot do
/// without it.
pub(super) fn permission_copy(kind: PermissionKind) -> (&'static str, &'static str) {
    match kind {
        PermissionKind::Microphone => (
            tr("Microphone"),
            tr("Hex cannot record dictation until microphone access is restored."),
        ),
        PermissionKind::InputMonitoring => (
            tr("Input Monitoring"),
            tr("Hex cannot recognize the dictation shortcut in other apps."),
        ),
        PermissionKind::Accessibility => (
            tr("Accessibility"),
            tr("Hex cannot paste completed dictation into the foreground app."),
        ),
        PermissionKind::SystemAudio => (
            tr("Screen & System Audio Recording"),
            tr("Required to hear the other participants."),
        ),
    }
}

const fn permission_key(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Microphone => "permission-microphone",
        PermissionKind::InputMonitoring => "permission-input-monitoring",
        PermissionKind::Accessibility => "permission-accessibility",
        PermissionKind::SystemAudio => "permission-system-audio",
    }
}

fn perform(ui: &mut Ui, warning: PermissionWarning) {
    ui.ask(PlatformRequest::Permission(warning.action));
    ui.platform.note_asked(warning.kind);
}

/// The missing permissions above everything else, while the tour is closed.
pub(super) fn permissions_section(ui: &Ui, p: Paint) -> Option<El> {
    if !ui.platform.caps.permissions || ui.tour.is_some() {
        return None;
    }
    let warnings = crate::onboarding::permission_warnings(ui.platform.permissions);
    if warnings.is_empty() {
        return None;
    }
    let rows = warnings
        .into_iter()
        .map(|warning| {
            let (name, description) = permission_copy(warning.kind);
            let key = permission_key(warning.kind);
            let control = permission_control(
                p,
                &format!("{key}-allow"),
                if ui.platform.asked(warning.kind) {
                    PermissionControl::Waiting
                } else {
                    PermissionControl::Allow
                },
                ButtonKind::Secondary,
                move |ui: &mut Ui| perform(ui, warning),
            );
            setting_row(p, key, name, Some(description), control)
        })
        .collect();
    Some(section(p, "permissions", tr("Permissions needed"), rows))
}

/// The voice command model's notice, when commands are on but not ready.
pub(super) fn command_model_notice(ui: &Ui, p: Paint) -> Option<El> {
    let (title, description) = super::commands::model_notice(ui)?;
    Some(super::commands::model_notice_panel(
        ui,
        p,
        title,
        description,
    ))
}

/// A save error where the OS has no listener card to show it in.
pub(super) fn error_section(ui: &Ui, p: Paint) -> Option<El> {
    if ui.platform.caps.listener_control {
        return None;
    }
    let error = ui.error.as_deref()?;
    Some(section(
        p,
        "save-error",
        tr("Status"),
        vec![
            note(
                "error-title",
                tr("Hex could not apply this change."),
                p.danger(),
            ),
            note("error", error, p.muted()),
        ],
    ))
}

fn notes_description(availability: NotesAvailability) -> &'static str {
    match availability {
        NotesAvailability::Available => {
            tr("Meeting titles and short summaries, written on this Mac")
        }
        NotesAvailability::DeviceNotEligible => tr("This Mac does not support Apple Intelligence"),
        NotesAvailability::NotEnabled => {
            tr("Turn on Apple Intelligence in System Settings to use this")
        }
        NotesAvailability::ModelNotReady => tr("The Apple Intelligence model is still downloading"),
        NotesAvailability::UnsupportedSystem => tr("Requires macOS 26 or later"),
        NotesAvailability::Unknown => tr("Apple Intelligence is unavailable"),
    }
}

fn folder_label(ui: &Ui) -> String {
    let folder = ui.settings.meetings.resolved_output_folder();
    dirs::home_dir()
        .and_then(|home| {
            folder
                .strip_prefix(home)
                .ok()
                .map(|relative| format!("~/{}", relative.display()))
        })
        .unwrap_or_else(|| folder.display().to_string())
}

/// Meetings (beta): the opt-in, then its permission, indicator, folder,
/// never-offer list, and Apple Intelligence notes.
pub(super) fn meetings_section(ui: &Ui, p: Paint) -> Option<El> {
    if !ui.platform.caps.meetings {
        return None;
    }
    let meetings = &ui.settings.meetings;
    let mut rows = vec![toggle_row(
        p,
        "meetings-enabled",
        tr("Meetings (beta)"),
        tr("Offer to record calls and save local transcripts"),
        meetings.enabled,
        |ui: &mut Ui| {
            let enabled = !ui.settings.meetings.enabled;
            if ui.update(|settings| settings.meetings.enabled = enabled)
                && !enabled
                && ui.pane == super::Pane::Meetings
            {
                ui.pane = super::Pane::Settings;
            }
        },
    )];
    if meetings.enabled {
        let permitted = ui.platform.system_audio;
        rows.push(setting_row(
            p,
            "meeting-system-audio",
            tr("Screen & System Audio Recording"),
            Some(if permitted {
                tr("Captures the other participants' audio")
            } else {
                tr("Required to hear the other participants.")
            }),
            permission_control(
                p,
                "meeting-system-audio-allow",
                if permitted {
                    PermissionControl::Allowed
                } else if ui.platform.asked(PermissionKind::SystemAudio) {
                    PermissionControl::Waiting
                } else {
                    PermissionControl::Allow
                },
                ButtonKind::Secondary,
                |ui: &mut Ui| {
                    // macOS prompts once; after that the switch is in
                    // System Settings, with the guide beside it.
                    ui.ask(PlatformRequest::SystemAudio);
                    ui.platform.note_asked(PermissionKind::SystemAudio);
                },
            ),
        ));
        rows.push(toggle_row(
            p,
            "meeting-indicator",
            tr("Show recording indicator"),
            tr("Floating pill to copy live transcript turns or stop recording"),
            meetings.show_recording_indicator,
            |ui: &mut Ui| {
                let show = !ui.settings.meetings.show_recording_indicator;
                ui.update(|settings| settings.meetings.show_recording_indicator = show);
            },
        ));
        rows.push(setting_row(
            p,
            "meeting-folder",
            tr("Transcript folder"),
            Some(tr("Finished meetings are saved here as Markdown")),
            row()
                .key("controls")
                .gap(8.0)
                .center_y()
                .child(button(
                    p,
                    "reveal-meeting-folder",
                    tr("Reveal"),
                    ButtonKind::Secondary,
                    |ui: &mut Ui, _| ui.ask(PlatformRequest::Meeting(MeetingRequest::RevealFolder)),
                ))
                .child(button(
                    p,
                    "choose-meeting-folder",
                    &folder_label(ui),
                    ButtonKind::Secondary,
                    |ui: &mut Ui, _| ui.ask(PlatformRequest::Meeting(MeetingRequest::ChooseFolder)),
                )),
        ));
        if !meetings.never_offer.is_empty() {
            let entries = col().key("never-offer-list").gap(4.0).children(
                meetings
                    .never_offer
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        let key = entry.key.clone();
                        row()
                            .key(format!("never-offer{index}"))
                            .gap(8.0)
                            .center_y()
                            .justify_end()
                            .child(
                                text(entry.label.clone())
                                    .key("label")
                                    .font(DESCRIPTION)
                                    .color(p.muted()),
                            )
                            .child(small_button(
                                p,
                                "remove",
                                tr("Remove"),
                                move |ui: &mut Ui, _| {
                                    let key = key.clone();
                                    ui.update(|settings| {
                                        settings
                                            .meetings
                                            .never_offer
                                            .retain(|entry| entry.key != key)
                                    });
                                },
                            ))
                    }),
            );
            rows.push(setting_row(
                p,
                "never-offer",
                tr("Never offer"),
                Some(tr("No recording offers for these sites and apps")),
                entries,
            ));
        }
        rows.push(toggle_row(
            p,
            "meeting-notes",
            tr("Summarize with Apple Intelligence"),
            notes_description(ui.platform.notes),
            meetings.summarize_with_apple_intelligence,
            |ui: &mut Ui| {
                let enabled = !ui.settings.meetings.summarize_with_apple_intelligence;
                ui.update(|settings| settings.meetings.summarize_with_apple_intelligence = enabled);
            },
        ));
        if let Some(error) = &ui.platform.meeting_folder_error {
            rows.push(note("meeting-folder-error", error, p.danger()));
        }
    }
    Some(section(p, "meetings", tr("Meetings"), rows))
}

/// Launch at login, driven by the OS's login item: macOS's `SMAppService`
/// (which may need approval) or Windows's `Run` key.
pub(super) fn launch_at_login_rows(ui: &Ui, p: Paint) -> Vec<El> {
    if !ui.platform.caps.launch_at_login {
        return Vec::new();
    }
    let status = ui.platform.login_item;
    let control = match status {
        None => text(if ui.platform.login_item_checking {
            tr("Checking…")
        } else {
            tr("Unavailable")
        })
        .key("login-item-status")
        .font(DESCRIPTION)
        .color(p.muted()),
        Some(LoginItemStatus::RequiresApproval) => button(
            p,
            "launch-at-login-approval",
            tr("Open settings"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| ui.ask(PlatformRequest::OpenLoginItemSettings),
        ),
        Some(status) => Switch::new(status == LoginItemStatus::Enabled)
            .build(p.k)
            .key("switch"),
    };
    let mut row = setting_row(
        p,
        "launch-at-login",
        tr("Launch at login"),
        Some(tr("Start Hex when you log in")),
        control,
    );
    if matches!(
        status,
        Some(LoginItemStatus::Enabled | LoginItemStatus::Disabled)
    ) {
        row = row
            .role("toggle")
            .hover(p.k.c.hover)
            .press(p.k.c.pressed)
            .on_press(|ui: &mut Ui| {
                let enabled = ui.platform.login_item != Some(LoginItemStatus::Enabled);
                ui.ask(PlatformRequest::SetLaunchAtLogin(enabled));
                // macOS stays the source of truth; previews flip at once.
                if ui.host.is_fixture() {
                    ui.platform.login_item = Some(if enabled {
                        LoginItemStatus::Enabled
                    } else {
                        LoginItemStatus::Disabled
                    });
                }
            });
    }
    let mut rows = vec![row];
    if let Some(error) = &ui.platform.login_item_error {
        rows.push(note("launch-at-login-error", error, p.danger()));
    }
    rows
}

pub(super) fn dock_icon_row(ui: &Ui, p: Paint) -> Option<El> {
    ui.platform.caps.dock_icon.then(|| {
        toggle_row(
            p,
            "dock-icon",
            tr("Show Dock icon"),
            tr("Keep Hex in the menu bar when Settings is closed"),
            ui.settings.show_dock_icon,
            |ui: &mut Ui| {
                let show = !ui.settings.show_dock_icon;
                ui.update(|settings| settings.show_dock_icon = show);
            },
        )
    })
}

const HEX_MICROPHONE_MODES: [VoiceFilterMode; 4] = [
    VoiceFilterMode::Muffled,
    VoiceFilterMode::Muted,
    VoiceFilterMode::Quiet,
    VoiceFilterMode::Normal,
];

fn mode_label(mode: VoiceFilterMode) -> &'static str {
    match mode {
        VoiceFilterMode::Muffled => tr("Muffled"),
        VoiceFilterMode::Muted => tr("Muted"),
        VoiceFilterMode::Quiet => tr("Quiet"),
        VoiceFilterMode::Normal => tr("Normal"),
        VoiceFilterMode::Chipmunk => tr("Chipmunk"),
        VoiceFilterMode::Robot => tr("Robot"),
        VoiceFilterMode::Walkie => tr("Walkie"),
        VoiceFilterMode::Underwater => tr("Underwater"),
        VoiceFilterMode::Far => tr("Far"),
    }
}

fn mode_description(mode: VoiceFilterMode) -> &'static str {
    match mode {
        VoiceFilterMode::Muffled => tr("Sounds like a hand over the microphone while you dictate"),
        VoiceFilterMode::Muted => tr("Silences your voice with soft room tone while you dictate"),
        VoiceFilterMode::Quiet => tr("Turns your voice down to a whisper while you dictate"),
        VoiceFilterMode::Normal => tr("Passes your voice through unchanged while you dictate"),
        VoiceFilterMode::Chipmunk => tr("Sounds pitched up while you dictate"),
        VoiceFilterMode::Robot => tr("Sounds robotic while you dictate"),
        VoiceFilterMode::Walkie => tr("Sounds like a handheld radio while you dictate"),
        VoiceFilterMode::Underwater => tr("Sounds submerged while you dictate"),
        VoiceFilterMode::Far => tr("Sounds like you are across the room while you dictate"),
    }
}

/// The virtual microphone driver's row: install, update, or uninstall it.
fn driver_row(ui: &Ui, p: Paint) -> El {
    let title = tr("Virtual microphone driver");
    let Some(status) = &ui.platform.hex_microphone else {
        return setting_row(
            p,
            "hex-mic-driver",
            title,
            Some(tr("Checking…")),
            row().key("none"),
        );
    };
    let action = |key: &str, label: &str, request: PlatformRequest| {
        button(
            p,
            key,
            label,
            ButtonKind::Secondary,
            move |ui: &mut Ui, _| ui.ask(request.clone()),
        )
    };
    let install = || {
        action(
            "install",
            tr("Install"),
            PlatformRequest::InstallHexMicrophone,
        )
    };
    let uninstall = || {
        action(
            "uninstall",
            tr("Uninstall"),
            PlatformRequest::UninstallHexMicrophone,
        )
    };
    let controls = row().key("controls").gap(8.0).center_y();
    let (description, controls) = match &status.action_state {
        DriverActionState::Installing => (
            tr("Installing driver and restarting Core Audio…").to_string(),
            controls.child(tone_tag(p, "state", tr("Installing…"), Tone::Neutral)),
        ),
        DriverActionState::Uninstalling => (
            tr("Removing driver and restarting Core Audio…").to_string(),
            controls.child(tone_tag(p, "state", tr("Uninstalling…"), Tone::Neutral)),
        ),
        state if status.needs_update() => {
            let installed = status.installed_version.as_deref().unwrap_or("1.0.0");
            (
                match state {
                    DriverActionState::Cancelled => tr("Update cancelled").into(),
                    DriverActionState::Failed(reason) => {
                        tr_fill("Could not update driver: {}", reason)
                    }
                    _ => tr_fill("Update driver to v{}", &status.bundled_version),
                },
                controls
                    .child(tone_tag(
                        p,
                        "state",
                        &tr_fill("v{} installed", installed),
                        Tone::Accent,
                    ))
                    .child(action(
                        "update",
                        tr("Update"),
                        PlatformRequest::InstallHexMicrophone,
                    ))
                    .child(uninstall()),
            )
        }
        state if status.driver_loaded => {
            let version = status
                .installed_version
                .as_deref()
                .unwrap_or(&status.bundled_version);
            (
                match state {
                    DriverActionState::Cancelled => tr("Uninstall cancelled").into(),
                    DriverActionState::Failed(reason) => {
                        tr_fill("Could not update driver: {}", reason)
                    }
                    _ if !status.active_apps.is_empty() => tr_fill(
                        "Streaming 48 kHz audio to {}",
                        &status.active_apps.join(", "),
                    ),
                    _ => {
                        tr("Choose Hex Microphone as your input in Slack, Zoom, or Discord").into()
                    }
                },
                controls
                    .child(tone_tag(
                        p,
                        "state",
                        &tr_fill("Installed v{}", version),
                        Tone::Success,
                    ))
                    .child(uninstall()),
            )
        }
        DriverActionState::DoneUninstalled => (
            tr("Driver removed").into(),
            controls
                .child(tone_tag(p, "state", tr("Removed"), Tone::Neutral))
                .child(install()),
        ),
        DriverActionState::Cancelled => (
            tr("Installation cancelled").into(),
            controls
                .child(tone_tag(p, "state", tr("Cancelled"), Tone::Neutral))
                .child(install()),
        ),
        DriverActionState::Failed(reason) => (
            tr_fill("Could not install driver: {}", reason),
            controls
                .child(tone_tag(p, "state", tr("Failed"), Tone::Danger))
                .child(install()),
        ),
        _ => (
            tr("Install the CoreAudio driver to choose Hex Microphone in Slack, Zoom, or Discord")
                .into(),
            controls.child(install()),
        ),
    };
    setting_row(p, "hex-mic-driver", title, Some(&description), controls)
}

/// Experimental: Hex Microphone, while its experiment is on. The Fun pane
/// it links to in GPUI is not ported, so "Try it" is omitted.
pub(super) fn hex_microphone_section(ui: &Ui, p: Paint) -> Option<El> {
    if !ui.platform.caps.hex_microphone || !ui.settings.experimental.hex_microphone_available() {
        return None;
    }
    let experimental = &ui.settings.experimental;
    let labels: Vec<&str> = HEX_MICROPHONE_MODES
        .iter()
        .map(|m| mode_label(*m))
        .collect();
    let selected = HEX_MICROPHONE_MODES
        .iter()
        .position(|mode| *mode == experimental.hex_microphone_mode)
        .unwrap_or(0);
    let rows = vec![
        driver_row(ui, p),
        setting_row(
            p,
            "hex-mic-mode",
            tr("Mode while dictating"),
            Some(mode_description(experimental.hex_microphone_mode)),
            segmented(p, "hex-mic-mode-control", &labels, selected, |ui, index| {
                let mode = HEX_MICROPHONE_MODES[index];
                ui.update(|settings| settings.experimental.hex_microphone_mode = mode);
            }),
        ),
        toggle_row(
            p,
            "hex-mic-comfort-noise",
            tr("Comfort noise"),
            tr("Keeps a quiet room tone while muted so callers know the line is live"),
            experimental.hex_microphone_comfort_noise,
            |ui: &mut Ui| {
                let enabled = !ui.settings.experimental.hex_microphone_comfort_noise;
                ui.update(|settings| settings.experimental.hex_microphone_comfort_noise = enabled);
            },
        ),
    ];
    Some(section(
        p,
        "hex-microphone",
        tr("Experimental: Hex microphone"),
        rows,
    ))
}
