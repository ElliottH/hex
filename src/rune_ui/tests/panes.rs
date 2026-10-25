//! The macOS and Windows surfaces, driven headlessly on Linux from preview
//! data: Commands, Meetings, the OS-only General rows, and the full tour.

use super::*;
use crate::login_item::LoginItemStatus;
use crate::onboarding::{Input, Observation, PermissionAction, PermissionKind, Practice, Step};
use platform::{CalendarRequest, MeetingRequest, PlatformHost, PlatformRequest};

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

#[test]
fn each_os_lists_only_the_panes_it_can_run() {
    let linux = runtime("portal", true, PANE_SIZE);
    assert!(linux.find("nav-commands").is_none());
    assert!(linux.find("nav-meetings").is_none());
    let mut mac = runtime("mac-settings", true, PANE_SIZE);
    for (key, pane) in [
        ("nav-commands", Pane::Commands),
        ("nav-meetings", Pane::Meetings),
        ("nav-general", Pane::General),
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

    let mac = runtime("mac-settings", true, PANE_SIZE);
    for key in ["launch-at-login", "dock-icon", "check-for-updates"] {
        assert!(present(&mac, key), "macOS shows {key}");
    }
    // Features own their settings: none of these are in General.
    for key in [
        "terminal-paste",
        "toggle-listening",
        "enable-on-launch",
        "meetings-enabled",
        "detected-calls",
        "hex-mic-driver",
        "command-model-notice",
    ] {
        assert!(!present(&mac, key), "macOS General has no {key}");
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
        rt.find("permission-input-monitoring-allow-control/mark/dot")
            .is_some(),
        "asked permissions wait with a dot"
    );
}

#[test]
fn a_grant_morphs_allow_into_allowed() {
    let mut rt = runtime("meetings-allow", true, PANE_SIZE);
    let pill = "meeting-system-audio-allow";
    tap(&mut rt, pill);
    assert_eq!(asked(&rt), [PlatformRequest::SystemAudio]);
    assert!(
        rt.find(&format!("{pill}-control/mark/dot")).is_some(),
        "waits with a dot"
    );
    let width = |rt: &Runtime<Ui>| rt.presented(pill).unwrap().0[2];
    let allow = width(&rt);

    // The fixture grants on the second ask.
    let (x, y) = rt.find(pill).unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    for _ in 0..5 {
        rt.frame(1.0 / 60.0);
    }
    let between = width(&rt);
    settle(&mut rt);
    let allowed = width(&rt);
    assert!(rt.state.platform.system_audio);
    assert!(allowed < allow, "the pill sheds its padding");
    assert!(
        allowed < between && between < allow,
        "the same pill narrows rather than swapping: {allow} → {between} → {allowed}"
    );
    assert!(rt.find(&format!("{pill}-control/mark/ink/check")).is_some());
    let shown = texts(&rt);
    assert!(shown.iter().any(|text| text == "Allow"));
    assert!(shown.iter().any(|text| text == "ed"));

    tap(&mut rt, pill);
    assert_eq!(asked(&rt).len(), 2, "Allowed is not a button");
}

#[test]
fn reduce_motion_swaps_allow_for_allowed_at_once() {
    let mut rt = runtime("meetings-allow", true, PANE_SIZE);
    let pill = "meeting-system-audio-allow";
    tap(&mut rt, pill);
    let mut settled = runtime("meetings-allow", true, PANE_SIZE);
    settled.state.platform.system_audio = true;
    refresh(&mut settled);
    let allowed = settled.presented(pill).unwrap().0;

    rt.set_system_reduced_motion(true);
    let (x, y) = rt.find(pill).unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.presented(pill).unwrap().0, allowed, "no travel");
}

#[test]
fn general_fits_the_default_window_and_hides_advanced_options() {
    let mut rt = runtime("mac-settings", true, PANE_SIZE);
    let height = |key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .map(|e| e.rect[3])
            .unwrap()
    };
    let (body, inset) = (height("scroll"), height("inset"));
    assert!(inset <= body, "General scrolls: {inset} > {body}");
    assert!(rt.find("microphone-mode").is_none());
    tap(&mut rt, "advanced-toggle");
    assert!(rt.find("microphone-mode").is_some());
    assert_eq!(
        rt.find("double-tap-only").is_some(),
        rt.state.settings.dictation_hotkey.key.is_some(),
        "double-tap only needs a key chord"
    );
    tap(&mut rt, "advanced-toggle");
    assert!(rt.find("microphone-mode").is_none());
}

#[test]
fn double_tap_to_lock_shows_only_for_hold() {
    let mut rt = runtime("mac-settings", true, PANE_SIZE);
    assert!(rt.find("double-tap").is_some());
    tap(&mut rt, "shortcut-mode-control/segment1");
    assert_eq!(
        rt.state.settings.shortcut_mode,
        crate::dictation::ShortcutMode::Toggle
    );
    assert!(rt.find("double-tap").is_none());
}

#[test]
fn a_failed_command_model_offers_retry_and_turning_commands_off() {
    let mut rt = runtime("mac-commands-unavailable", true, PANE_SIZE);
    assert!(
        runtime("mac-settings-permissions", true, (980.0, 2400.0))
            .find("command-model-notice")
            .is_none(),
        "the notice lives in Commands"
    );
    tap(&mut rt, "retry-commands");
    assert_eq!(asked(&rt), [PlatformRequest::RetryCommandModel]);
    tap(&mut rt, "disable-commands");
    assert!(!rt.state.settings.commands_enabled);
    assert!(rt.find("command-model-notice").is_none());
}

#[test]
fn meetings_switch_between_recordings_and_settings() {
    let mut rt = runtime("meetings", true, PANE_SIZE);
    assert!(rt.find("meeting-preview-design").is_some());
    assert!(rt.find("meetings-enabled").is_none());
    tap(&mut rt, "meetings-view/segment1");
    assert_eq!(rt.state.meetings.view, meetings::View::Settings);
    assert!(rt.find("meetings-enabled").is_some());
    assert!(rt.find("meeting-preview-design").is_none());
    tap(&mut rt, "meetings-view/segment0");
    assert!(rt.find("meeting-preview-design").is_some());
}

#[test]
fn the_meetings_switch_stays_put_and_a_swap_drops_pending_edits() {
    let mut rt = runtime("meetings", true, PANE_SIZE);
    let switch = |rt: &Runtime<Ui>| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("meetings-view"))
            .map(|e| e.rect)
            .unwrap()
    };
    let recordings = switch(&rt);
    tap(&mut rt, "meeting-rename");
    assert!(rt.state.meetings.renaming.is_some());
    rt.state.meetings.delete_armed = Some("preview-notes".into());
    tap(&mut rt, "meetings-view/segment1");
    assert_eq!(switch(&rt), recordings, "the search field never moves it");
    assert!(rt.state.meetings.renaming.is_none());
    assert!(rt.state.meetings.delete_armed.is_none());

    tap(&mut rt, "meetings-enabled");
    assert!(!rt.state.settings.meetings.enabled);
    assert_eq!(rt.state.meetings.view, meetings::View::Recordings);
}

#[test]
fn an_update_is_a_secondary_button_with_a_dot_and_its_own_label() {
    let mut rt = runtime("mac-update-ready", true, PANE_SIZE);
    assert!(rt.find("sidebar-update-control/dot").is_some());
    assert!(texts(&rt).iter().any(|text| text == "Restart to update"));
    tap(&mut rt, "sidebar-update");
    assert!(matches!(
        sent(&rt),
        [Request::Desktop(DesktopAction::RestartIntoUpdate)]
    ));
    if let Some(view) = &mut rt.state.view {
        view.update_status = DesktopUpdateStatus::Available;
    }
    refresh(&mut rt);
    assert!(texts(&rt).iter().any(|text| text == "Download update"));
}

#[test]
fn meetings_off_offers_the_opt_in_and_keeps_the_pane() {
    let mut rt = runtime("meetings-off", true, PANE_SIZE);
    assert!(rt.find("nav-meetings").is_some(), "always in the sidebar");
    assert!(rt.find("meetings-view").is_none());
    assert!(rt.find("start-meeting").is_none());
    tap(&mut rt, "meetings-enabled");
    assert!(rt.state.settings.meetings.enabled);
    assert!(rt.find("meetings-view").is_some());
}

#[test]
fn meetings_settings_save_and_ask_for_system_audio() {
    let mut rt = runtime("meetings-settings", true, (980.0, 2400.0));
    rt.state.platform.system_audio = false;
    refresh(&mut rt);
    tap(&mut rt, "meeting-system-audio-allow");
    assert_eq!(asked(&rt), [PlatformRequest::SystemAudio]);
    use crate::app_settings::MeetingPolicy;
    let policy = |rt: &Runtime<Ui>, key: &str| {
        rt.state
            .settings
            .meetings
            .providers
            .iter()
            .find(|choice| choice.key == key)
            .map(|choice| choice.policy)
    };
    // Known services first (Zoom is second), then the sites and apps chosen for.
    tap(&mut rt, "call1/policy/segment1");
    assert_eq!(
        policy(&rt, "us.zoom.xos"),
        Some(MeetingPolicy::AlwaysRecord)
    );
    tap(&mut rt, "call5/policy/segment0");
    assert_eq!(
        policy(&rt, "problems.kitlangton.com"),
        Some(MeetingPolicy::Ask)
    );
    assert!(rt.find("call5").is_some());
    tap(&mut rt, "meetings-enabled");
    assert!(!rt.state.settings.meetings.enabled);
    assert!(rt.find("meeting-folder").is_none());
    assert!(rt.find("meetings-enabled").is_some(), "the opt-in remains");
}

#[test]
fn the_calendar_asks_for_access_only_when_connect_is_pressed() {
    let mut rt = runtime("meetings-settings", true, (980.0, 2400.0));
    assert!(rt.find("calendar-connect").is_some());
    assert!(rt.find("calendar-remind").is_none());
    assert!(asked(&rt).is_empty());
    tap(&mut rt, "calendar-connect-button");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Calendar(CalendarRequest::Connect)]
    );

    // Meetings off: no calendar card, so nothing can ask for access.
    tap(&mut rt, "meetings-enabled");
    assert!(rt.find("calendar").is_none());
    for scene in ["portal", "windows-settings"] {
        let rt = runtime(scene, true, GOLDEN_SIZE);
        assert!(rt.find("calendar").is_none(), "{scene} has no calendar");
    }
}

#[test]
fn a_connected_calendar_saves_reminders_and_lists_meetings() {
    let mut rt = runtime("meetings-calendar", true, (980.0, 2600.0));
    let shown = texts(&rt);
    for text in [
        "Design sync",
        "Today · 2:30 PM",
        "Google Meet",
        "2 of 3 calendars",
    ] {
        assert!(shown.iter().any(|shown| shown == text), "shows {text}");
    }
    assert!(rt.find("upcoming2").is_some());
    assert!(
        rt.find("upcoming2/provider/icon").is_none(),
        "no link, no icon"
    );
    tap(&mut rt, "calendar-lead/lead/segment3");
    assert_eq!(rt.state.settings.meetings.calendar.lead_minutes, 5);
    tap(&mut rt, "calendar-remind");
    assert!(!rt.state.settings.meetings.calendar.remind);
    assert!(rt.find("calendar-lead").is_none());
    tap(&mut rt, "calendar-calendars-control-trigger");
    assert_eq!(rt.state.open, Some(Menu::Calendars));
}

#[test]
fn denied_calendar_access_links_to_privacy_settings() {
    let mut rt = runtime("meetings-calendar-denied", true, (980.0, 2400.0));
    assert!(rt.find("calendar-connect").is_none());
    tap(&mut rt, "calendar-privacy");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::Calendar(
            CalendarRequest::OpenPrivacySettings
        )]
    );
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
fn commands_turn_on_from_their_card() {
    let mut rt = runtime("mac-settings", true, PANE_SIZE);
    tap(&mut rt, "nav-commands");
    assert!(rt.find("commands-enabled").is_some());
    tap(&mut rt, "commands-enabled");
    assert!(rt.state.settings.commands_enabled);
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
    let copied = rt.copy_request.as_deref().unwrap().to_owned();
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
fn copy_section_copies_one_chapter() {
    let rt = runtime("meetings", true, PANE_SIZE);
    assert!(rt.find("chapters").is_none(), "no chapters, no section");

    let mut rt = runtime("meetings-chapters", true, PANE_SIZE);
    assert!(texts(&rt).iter().any(|text| text == "Chapters"));
    tap(&mut rt, "chapter1/copy-section");
    let copied = rt.copy_request.as_deref().unwrap().to_owned();
    assert!(
        copied.starts_with("## Gravity well\n\nA pulse and gravity well"),
        "{copied}"
    );
    assert!(copied.contains("Prototype it in WebGPU"), "{copied}");
    assert!(!copied.contains("Roblox") && !copied.contains("build screen"));
    assert!(texts(&rt).iter().any(|text| text == "Copied"));
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
    tap(&mut rt, "setup-microphone/setup-grant-control/setup-grant");
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
    onboarding::input(&mut rt.state, Input::Dictation(Observation::Started));
    onboarding::input(&mut rt.state, Input::Dictation(Observation::Pasted));
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
    onboarding::input(
        &mut rt.state,
        Input::ListenerFailed {
            relaunch_allowed: true,
        },
    );
    assert_eq!(asked(&rt), [PlatformRequest::Relaunch]);
    let mut rt = runtime("mac-onboarding-reopen", true, PANE_SIZE);
    tap(&mut rt, "tour-reopen");
    assert_eq!(asked(&rt), [PlatformRequest::Relaunch]);
}

#[test]
fn a_grant_waiting_for_a_restart_makes_setup_restart_hex() {
    let mut rt = runtime("mac-onboarding-restart", true, PANE_SIZE);
    assert_eq!(rt.state.tour.as_ref().unwrap().flow.step(), Step::Setup);
    tap(&mut rt, "tour-continue");
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
    tap(&mut rt, "setup-microphone/setup-grant-control/setup-grant");
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
    let _goldens = Goldens::collect();
    for dark in [true, false] {
        check_golden("meetings-calendar", dark, (980.0, 1800.0));
        for scene in [
            "mac-settings",
            "mac-commands",
            "meetings",
            "meetings-settings",
            "meetings-off",
            "meetings-recording",
            "mac-onboarding-setup",
            "mac-onboarding-landed",
        ] {
            check_golden(scene, dark, PANE_SIZE);
        }
    }
    check_golden("mac-settings-permissions", true, PANE_SIZE);
    check_golden("mac-settings-advanced", false, PANE_SIZE);
    check_golden("meetings-calendar-denied", true, PANE_SIZE);
    check_golden("mac-commands-unavailable", true, PANE_SIZE);
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

#[test]
fn voice_filter_saves_the_mode_and_applies_the_tuning_live() {
    use crate::voice_filter::VoiceFilterMode;
    let mut rt = runtime("mac-voice-filter", true, PANE_SIZE);
    tap(&mut rt, "fun-mode/segment1");
    assert_eq!(
        rt.state.settings.experimental.hex_microphone_mode,
        VoiceFilterMode::Muted
    );
    assert!(matches!(
        asked(&rt),
        [PlatformRequest::SetTuning(cfg)] if cfg.mode == VoiceFilterMode::Muted
    ));
    assert!(
        rt.find("fun-comfort-noise").is_some(),
        "Muted offers comfort noise"
    );
    tap(&mut rt, "fun-mode/segment3");
    assert!(
        rt.find("fun-slider-strength").is_none(),
        "Normal has no sliders"
    );
}

#[test]
fn voice_filter_sliders_set_the_filter_from_the_pointer() {
    let mut rt = runtime("mac-voice-filter", true, PANE_SIZE);
    let before = rt.state.platform.tuning.muffled_wall_hz;
    let (x, y) = rt.find("fun-slider-wall/track").unwrap();
    rt.pointer_down(x + 140.0, y);
    rt.pointer_up(x + 140.0, y);
    settle(&mut rt);
    let after = rt.state.platform.tuning.muffled_wall_hz;
    assert!(after > before && after >= 2900.0, "{before} -> {after}");
    assert!(matches!(
        asked(&rt).last(),
        Some(PlatformRequest::SetTuning(_))
    ));
    tap(&mut rt, "fun-reset-tuning");
    assert_eq!(rt.state.platform.tuning.muffled_wall_hz, before);
}

#[test]
fn try_it_records_and_plays_through_the_host() {
    use platform::DemoRequest;
    let mut rt = runtime("mac-voice-filter", true, PANE_SIZE);
    tap(&mut rt, "fun-demo-record");
    tap(&mut rt, "fun-play-call-hears");
    tap(&mut rt, "fun-demo-source/segment1");
    assert_eq!(
        asked(&rt),
        [
            PlatformRequest::HexMicrophoneDemo(DemoRequest::Record),
            PlatformRequest::HexMicrophoneDemo(DemoRequest::Play(
                crate::hex_microphone::DemoPlaybackTrack::CallHears
            )),
            PlatformRequest::HexMicrophoneDemo(DemoRequest::SelectSource(
                crate::hex_microphone::DemoSourceKind::Recording
            )),
        ]
    );
    let mut rt = runtime("mac-voice-filter-muted", true, PANE_SIZE);
    tap(&mut rt, "fun-play-call-hears");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::HexMicrophoneDemo(
            DemoRequest::StopPlayback
        )]
    );
}

#[test]
fn developer_panes_show_only_in_developer_builds() {
    let rt = runtime("mac-settings", true, PANE_SIZE);
    assert!(rt.find("nav-hud-lab").is_none());
    assert!(rt.find("nav-activity").is_none());
    assert!(
        rt.find("nav-voice-filter").is_some(),
        "Hex Microphone is on"
    );
    assert!(rt.find("nav-experiments").is_some());
    let rt = runtime("mac-hud-lab", true, PANE_SIZE);
    assert!(rt.find("nav-hud-lab").is_some());
    assert!(rt.find("nav-activity").is_some());
}

#[test]
fn voice_filter_needs_its_experiment_or_hex_microphone() {
    let mut rt = runtime("mac-settings", true, PANE_SIZE);
    rt.state.settings.experimental.hex_microphone = false;
    rt.state.platform.hex_microphone = None;
    refresh(&mut rt);
    assert!(rt.find("nav-voice-filter").is_none());
    assert!(rt.find("nav-experiments").is_none(), "no empty heading");
    rt.state.platform.voice_filter = true;
    refresh(&mut rt);
    assert!(rt.find("nav-voice-filter").is_some());
}

#[test]
fn hud_lab_drives_the_indicator() {
    let mut rt = runtime("mac-hud-lab", true, PANE_SIZE);
    tap(&mut rt, "hud-style2");
    tap(&mut rt, "hud-demo/segment2");
    let Some(PlatformRequest::ConfigureHud(lab)) = asked(&rt).last().cloned() else {
        panic!("{:?}", asked(&rt));
    };
    assert_eq!(lab.tuning, hud_lab::style(2));
    assert_eq!(lab.demo, platform::HudDemo::Processing);
    assert_eq!(lab.queued, 2);
    tap(&mut rt, "copy-hud-preset");
    assert!(
        rt.copy_request
            .as_deref()
            .is_some_and(|preset| preset.starts_with("hud-v1 style=2 lines=3"))
    );
    assert!(rt.state.hud_copied);
}

#[test]
fn hud_lab_controls_round_trip() {
    use hud_lab::Control;
    let mut tuning = platform::HudTuning::default();
    for (control, _, _) in Control::ALL {
        control.set(&mut tuning, 0.6);
        assert!((control.value(&tuning) - 0.6).abs() < 1e-4, "{control:?}");
    }
}

#[test]
fn activity_lists_events_and_shows_the_raw_one() {
    let mut rt = runtime("mac-activity", true, PANE_SIZE);
    assert!(texts(&rt).iter().any(|t| t == "Context changed to Slack"));
    tap(&mut rt, "activity-event4");
    assert_eq!(rt.state.activity.selected, Some(4));
    assert!(
        texts(&rt)
            .iter()
            .any(|t| t.contains("\"application\":\"Slack\""))
    );
}

#[test]
fn modifier_only_shortcuts_choose_their_side() {
    use crate::app_settings::ModifierSide;
    let mut rt = runtime("mac-settings-sides", true, (980.0, 2400.0));
    tap(&mut rt, "dictation-side/segment2");
    assert_eq!(
        rt.state.settings.dictation_hotkey.modifiers.option,
        Some(ModifierSide::Right)
    );
    // Key chords and Linux have no side choice.
    let rt = runtime("mac-settings", true, (980.0, 2400.0));
    assert!(rt.find("dictation-side").is_none());
    let rt = runtime("portal", true, GOLDEN_SIZE);
    assert!(rt.find("dictation-side").is_none());
}

#[test]
fn a_side_that_would_collide_is_refused() {
    use crate::app_settings::{HotkeyBinding, HotkeyModifiers, ModifierSide};
    let mut ui = fixture(DesktopShortcuts::Registered);
    let option = |side| HotkeyBinding {
        modifiers: HotkeyModifiers {
            option: Some(side),
            ..HotkeyModifiers::default()
        },
        key: None,
    };
    ui.settings.dictation_hotkey = option(ModifierSide::Left);
    ui.settings.paste_last_hotkey = Some(option(ModifierSide::Right));
    assert!(
        platform_settings::side_binding(
            &ui.settings,
            ShortcutTarget::Dictation,
            ModifierSide::Right
        )
        .is_none()
    );
    assert!(
        platform_settings::side_binding(
            &ui.settings,
            ShortcutTarget::Dictation,
            ModifierSide::Left
        )
        .is_some()
    );
}

#[test]
fn macos_27_names_accessibility_as_system_settings_does() {
    let rt = runtime("mac-settings-permissions", true, (980.0, 2400.0));
    assert!(
        texts(&rt)
            .iter()
            .any(|t| t == "Device Control and Data Access")
    );
}

#[test]
fn experiment_goldens() {
    let _goldens = Goldens::collect();
    for dark in [true, false] {
        check_golden("mac-voice-filter", dark, PANE_SIZE);
        check_golden("mac-hud-lab", dark, PANE_SIZE);
    }
    check_golden("mac-voice-filter-muted", true, PANE_SIZE);
    check_golden("mac-voice-filter-driver-missing", false, PANE_SIZE);
    check_golden("mac-activity", false, PANE_SIZE);
    check_golden("mac-settings-sides", true, PANE_SIZE);
}
