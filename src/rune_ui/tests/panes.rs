//! The macOS and Windows surfaces, driven headlessly on Linux from preview
//! data: Commands, Meetings, the OS-only Settings rows, and the full tour.

use super::*;
use crate::login_item::LoginItemStatus;
use crate::onboarding::{PermissionAction, PermissionKind, Practice, Step};
use platform::{MeetingRequest, PlatformHost, PlatformRequest};

fn asked(rt: &Runtime<Ui>) -> &[PlatformRequest] {
    match &rt.state.host {
        PlatformHost::Fixture(sent) => sent,
        _ => panic!("scenes record platform requests"),
    }
}

/// Redraws after a test changes state directly, as the host would.
fn refresh(rt: &mut Runtime<Ui>) {
    rt.invalidate();
    settle(rt);
}

fn texts(rt: &Runtime<Ui>) -> Vec<String> {
    rt.elements().into_iter().filter_map(|e| e.text).collect()
}

#[test]
fn each_os_lists_only_the_panes_it_can_run() {
    let linux = runtime("portal", true, PANE_SIZE);
    assert!(linux.find("nav-commands").is_none());
    assert!(linux.find("nav-meetings").is_none());
    let mut mac = runtime("mac-settings", true, PANE_SIZE);
    for (key, pane) in [
        ("nav-commands", Pane::Commands),
        ("nav-meetings", Pane::Meetings),
        ("nav-settings", Pane::Settings),
    ] {
        tap(&mut mac, key);
        assert_eq!(mac.state.pane, pane);
    }
    let windows = runtime("windows-settings", true, PANE_SIZE);
    assert!(windows.find("nav-commands").is_none());
    assert!(windows.find("nav-meetings").is_none());
}

#[test]
fn settings_rows_follow_each_os() {
    let present = |rt: &Runtime<Ui>, key: &str| rt.find(key).is_some();
    let linux = runtime("portal", true, GOLDEN_SIZE);
    assert!(present(&linux, "terminal-paste"));
    assert!(present(&linux, "toggle-listening"));
    assert!(present(&linux, "enable-on-launch"));
    assert!(!present(&linux, "launch-at-login"));
    assert!(!present(&linux, "dock-icon"));
    assert!(!present(&linux, "meetings-enabled"));

    let mac = runtime("mac-settings", true, (980.0, 2400.0));
    for key in [
        "launch-at-login",
        "dock-icon",
        "meetings-enabled",
        "meeting-system-audio",
        "never-offer",
        "hex-mic-driver",
        "check-for-updates",
    ] {
        assert!(present(&mac, key), "macOS shows {key}");
    }
    for key in ["terminal-paste", "toggle-listening", "enable-on-launch"] {
        assert!(!present(&mac, key), "macOS has no {key}");
    }

    let windows = runtime("windows-settings", true, GOLDEN_SIZE);
    assert!(present(&windows, "launch-at-login"));
    assert!(present(&windows, "toggle-listening"));
    assert!(!present(&windows, "dock-icon"));
    assert!(!present(&windows, "terminal-paste"));
    assert!(!present(&windows, "meetings-enabled"));
}

#[test]
fn launch_at_login_asks_the_login_item_and_needs_approval_first() {
    let mut rt = runtime("mac-settings", true, (980.0, 2400.0));
    tap(&mut rt, "launch-at-login");
    assert_eq!(asked(&rt), [PlatformRequest::SetLaunchAtLogin(true)]);
    assert_eq!(rt.state.platform.login_item, Some(LoginItemStatus::Enabled));

    let mut rt = runtime("mac-settings-permissions", true, (980.0, 2400.0));
    tap(&mut rt, "launch-at-login-approval");
    assert_eq!(asked(&rt), [PlatformRequest::OpenLoginItemSettings]);
}

#[test]
fn missing_permissions_lead_settings_and_open_the_guide() {
    let mut rt = runtime("mac-settings-permissions", true, (980.0, 2400.0));
    let shown = texts(&rt);
    assert!(shown.iter().any(|text| text == "Permissions needed"));
    assert!(rt.find("permission-microphone").is_none(), "granted");
    tap(&mut rt, "permission-input-monitoring-allow");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Permission(PermissionAction::ShowGuide(
            PermissionKind::InputMonitoring
        ))]
    );
    assert!(rt.state.platform.asked(PermissionKind::InputMonitoring));
    assert!(
        rt.find("permission-input-monitoring-allow-waiting")
            .is_some(),
        "asked permissions wait with a dot"
    );
}

#[test]
fn a_failed_command_model_offers_retry_and_turning_commands_off() {
    let mut rt = runtime("mac-settings-permissions", true, (980.0, 2400.0));
    tap(&mut rt, "retry-commands");
    assert_eq!(asked(&rt), [PlatformRequest::RetryCommandModel]);
    tap(&mut rt, "disable-commands");
    assert!(!rt.state.settings.commands_enabled);
    assert!(rt.find("command-model-notice").is_none());
}

#[test]
fn meetings_settings_save_and_ask_for_system_audio() {
    let mut rt = runtime("mac-settings", true, (980.0, 2400.0));
    rt.state.platform.system_audio = false;
    refresh(&mut rt);
    tap(&mut rt, "meeting-system-audio-allow");
    assert_eq!(asked(&rt), [PlatformRequest::SystemAudio]);
    tap(&mut rt, "never-offer0/remove");
    assert!(rt.state.settings.meetings.never_offer.is_empty());
    tap(&mut rt, "meetings-enabled");
    assert!(!rt.state.settings.meetings.enabled);
    assert!(rt.find("meeting-folder").is_none());
}

#[test]
fn the_hex_microphone_section_saves_its_mode_and_drives_the_driver() {
    let mut rt = runtime("mac-settings", true, (980.0, 2400.0));
    tap(&mut rt, "hex-mic-mode-control/segment2");
    assert_eq!(
        rt.state.settings.experimental.hex_microphone_mode,
        crate::voice_filter::VoiceFilterMode::Quiet
    );
    tap(&mut rt, "hex-mic-driver/controls/uninstall");
    assert_eq!(asked(&rt), [PlatformRequest::UninstallHexMicrophone]);
}

#[test]
fn commands_search_select_and_show_their_detail() {
    let mut rt = runtime("mac-commands", true, PANE_SIZE);
    assert!(texts(&rt).iter().any(|text| text == "Voice control"));
    tap(&mut rt, "command-github.pulls");
    assert_eq!(rt.state.commands.selected.as_deref(), Some("github.pulls"));
    let shown = texts(&rt);
    assert!(shown.iter().any(|text| text == "github.com"));
    assert!(shown.iter().any(|text| text == "pull requests\nopen pulls"));
    rt.state.commands.search = TextEdit::new("slack");
    refresh(&mut rt);
    assert!(rt.find("command-open.slack").is_some());
    assert!(rt.find("command-mode.sleep").is_none());
    rt.state.commands.search = TextEdit::new("nothing like this");
    refresh(&mut rt);
    assert!(
        texts(&rt)
            .iter()
            .any(|text| text == "No matching commands.")
    );
}

#[test]
fn enabling_commands_with_a_released_microphone_asks_first() {
    let mut rt = runtime("mac-commands", true, PANE_SIZE);
    tap(&mut rt, "commands-enabled");
    assert!(!rt.state.settings.commands_enabled);
    rt.state.settings.release_microphone_while_idle = true;
    refresh(&mut rt);
    tap(&mut rt, "commands-enabled");
    assert!(rt.state.commands.confirm_enable);
    assert!(!rt.state.settings.commands_enabled);
    tap(&mut rt, "confirm-enable-commands");
    assert!(rt.state.settings.commands_enabled);
    assert!(!rt.state.settings.release_microphone_while_idle);
}

#[test]
fn commands_copy_the_agent_prompt_and_open_the_config() {
    let mut rt = runtime("mac-commands", true, PANE_SIZE);
    tap(&mut rt, "commands-copy-prompt");
    assert!(
        rt.copy_request
            .as_deref()
            .is_some_and(|prompt| prompt.contains("hex.config.ts"))
    );
    tap(&mut rt, "commands-open-config");
    assert_eq!(asked(&rt), [PlatformRequest::OpenCommandsConfig]);
}

#[test]
fn meetings_select_search_and_copy_a_transcript() {
    let mut rt = runtime("meetings", true, PANE_SIZE);
    tap(&mut rt, "meeting-preview-design");
    assert_eq!(
        rt.state.meetings.selected.as_deref(),
        Some("preview-design")
    );
    tap(&mut rt, "meeting-copy-transcript");
    let copied = rt.copy_request.clone().unwrap();
    assert!(copied.starts_with("**You** · 00:09\nOh nice."), "{copied}");
    rt.state.meetings.search = TextEdit::new("roblox");
    refresh(&mut rt);
    assert!(rt.find("meeting-preview-failed").is_none());
    assert!(
        rt.find("meeting-preview-notes").is_some(),
        "searches transcripts"
    );
}

#[test]
fn recording_starts_and_stops_from_the_header() {
    let mut rt = runtime("meetings", true, PANE_SIZE);
    tap(&mut rt, "stop-meeting");
    assert_eq!(asked(&rt), [PlatformRequest::Meeting(MeetingRequest::Stop)]);
    assert!(rt.find("stop-meeting").is_none(), "finalizing");

    let mut rt = runtime("meetings-empty", true, PANE_SIZE);
    assert!(texts(&rt).iter().any(|text| text == "No meetings yet."));
    tap(&mut rt, "start-meeting");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Meeting(MeetingRequest::Start)]
    );
}

#[test]
fn meetings_rename_from_the_title_field() {
    let mut rt = runtime("meetings", true, PANE_SIZE);
    tap(&mut rt, "meeting-rename");
    assert_eq!(rt.state.meetings.renaming.as_deref(), Some("preview-notes"));
    rt.state.meetings.rename = TextEdit::new("Arcade launch");
    refresh(&mut rt);
    tap(&mut rt, "meeting-save-title");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Meeting(MeetingRequest::Rename {
            id: "preview-notes".into(),
            title: "Arcade launch".into(),
        })]
    );
    assert_eq!(rt.state.meetings.meetings[1].title, "Arcade launch");
    assert!(rt.state.meetings.renaming.is_none());
}

#[test]
fn deleting_a_meeting_takes_the_menu_then_a_confirmation() {
    let mut rt = runtime("meetings", true, PANE_SIZE);
    tap(&mut rt, "meeting-more");
    assert_eq!(rt.state.open, Some(Menu::MeetingActions));
    // Copy summary, Reveal Markdown, Regenerate summary, a separator, Delete.
    tap(&mut rt, "meeting-actions-item4");
    assert_eq!(
        rt.state.meetings.delete_armed.as_deref(),
        Some("preview-notes")
    );
    assert!(asked(&rt).is_empty(), "the first step only arms");
    assert!(
        texts(&rt)
            .iter()
            .any(|text| text == "Trash recording & Markdown")
    );
    tap(&mut rt, "meeting-confirm-delete");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Meeting(MeetingRequest::Trash(
            "preview-notes".into()
        ))]
    );
    assert!(
        rt.state
            .meetings
            .meetings
            .iter()
            .all(|m| m.id != "preview-notes")
    );

    let mut rt = runtime("meetings-delete", true, PANE_SIZE);
    key(&mut rt, Key::Escape, Mods::default());
    assert!(rt.state.meetings.delete_armed.is_none(), "Escape cancels");
}

#[test]
fn the_overflow_menu_copies_the_summary_and_asks_for_new_notes() {
    let mut rt = runtime("meetings-actions", true, PANE_SIZE);
    tap(&mut rt, "meeting-actions-item0");
    assert!(
        rt.copy_request
            .as_deref()
            .is_some_and(|summary| summary.starts_with("One login"))
    );
    assert_eq!(rt.state.open, Some(Menu::MeetingActions), "stays open");
    tap(&mut rt, "meeting-actions-item2");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Meeting(MeetingRequest::Summarize(
            "preview-notes".into()
        ))]
    );
}

#[test]
fn active_and_failed_meetings_show_their_state() {
    let rt = runtime("meetings-recording", true, PANE_SIZE);
    let shown = texts(&rt);
    assert!(shown.iter().any(|text| text == "Live transcript"));
    assert!(shown.iter().any(|text| text == "Listening…"));
    assert!(
        rt.find("meeting-rename").is_none(),
        "no rename while recording"
    );
    let rt = runtime("meetings-failed", true, PANE_SIZE);
    let shown = texts(&rt);
    assert!(shown.iter().any(|text| text == "Recording failed."));
    assert!(shown.iter().any(|text| text == "Recovered draft"));
}

#[test]
fn the_mac_tour_asks_each_permission_then_practices() {
    let mut rt = runtime("mac-onboarding-setup", true, PANE_SIZE);
    assert_eq!(rt.state.tour.as_ref().unwrap().flow.step(), Step::Setup);
    assert!(
        matches!(sent(&rt), [Request::ChooseModel { .. }]),
        "the model downloads behind the permissions"
    );
    tap(&mut rt, "setup-microphone/setup-grant");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Permission(
            PermissionAction::RequestMicrophone
        )]
    );
    // macOS grants everything and the model lands: setup moves on.
    use crate::onboarding::PermissionState::Ready;
    rt.state.platform.permissions.microphone = Ready;
    rt.state.platform.permissions.input_monitoring = Ready;
    rt.state.platform.permissions.accessibility = Ready;
    rt.state.picker.installed = Some(vec![rt.state.settings.transcription.model]);
    refresh(&mut rt);
    std::thread::sleep(std::time::Duration::from_millis(750));
    settle(&mut rt);
    assert_eq!(rt.state.tour.as_ref().unwrap().flow.step(), Step::TryIt);
    assert!(rt.find("tour-skip").is_some(), "practice can be skipped");
    onboarding::observe(&mut rt.state, crate::onboarding::Observation::Started);
    onboarding::observe(&mut rt.state, crate::onboarding::Observation::Pasted);
    refresh(&mut rt);
    assert_eq!(
        rt.state.tour.as_ref().unwrap().flow.view().practice,
        Practice::Landed
    );
    tap(&mut rt, "tour-continue");
    assert_eq!(rt.state.tour.as_ref().unwrap().flow.step(), Step::Done);
    tap(&mut rt, "tour-continue");
    assert!(rt.state.tour.is_none());
}

#[test]
fn a_listener_that_will_not_start_relaunches_once_then_asks_to_reopen() {
    let mut rt = runtime("mac-onboarding-try", true, PANE_SIZE);
    onboarding::listener_failed(&mut rt.state, true);
    assert_eq!(asked(&rt), [PlatformRequest::Relaunch]);
    let mut rt = runtime("mac-onboarding-reopen", true, PANE_SIZE);
    tap(&mut rt, "tour-reopen");
    assert_eq!(asked(&rt), [PlatformRequest::Relaunch]);
}

#[test]
fn a_refused_microphone_points_to_system_settings() {
    let mut rt = runtime("mac-onboarding-denied", true, PANE_SIZE);
    let shown = texts(&rt);
    assert!(
        shown
            .iter()
            .any(|text| text == "Turned off earlier. Switch Hex on in System Settings.")
    );
    tap(&mut rt, "setup-microphone/setup-grant");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Permission(PermissionAction::ShowGuide(
            PermissionKind::Microphone
        ))]
    );
}

/// The macOS and Windows surfaces in both appearances.
#[test]
fn platform_goldens() {
    for dark in [true, false] {
        check_golden("mac-settings", dark, (980.0, 2100.0));
        for scene in [
            "mac-commands",
            "meetings",
            "meetings-recording",
            "mac-onboarding-setup",
            "mac-onboarding-landed",
        ] {
            check_golden(scene, dark, PANE_SIZE);
        }
    }
    check_golden("mac-settings-permissions", true, PANE_SIZE);
    check_golden("windows-settings", false, GOLDEN_SIZE);
    check_golden("mac-commands-confirm", false, PANE_SIZE);
    check_golden("meetings-actions", true, PANE_SIZE);
    check_golden("meetings-delete", false, PANE_SIZE);
    check_golden("meetings-rename", true, PANE_SIZE);
    check_golden("meetings-empty", false, PANE_SIZE);
    check_golden("mac-onboarding", true, PANE_SIZE);
    check_golden("mac-onboarding-progress", false, PANE_SIZE);
    check_golden("mac-onboarding-denied", true, PANE_SIZE);
    check_golden("mac-onboarding-download-failed", false, PANE_SIZE);
    check_golden("mac-onboarding-listening", true, PANE_SIZE);
    check_golden("mac-onboarding-reopen", false, PANE_SIZE);
    check_golden("mac-onboarding-done", true, PANE_SIZE);
}
