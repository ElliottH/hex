//! Linux parts of the app window (`hex app --ui gpui`). The window runs in
//! the Hex process beside the host that owns the listener, shortcut
//! registration, the HUD, and updates, and calls it directly.

use super::*;
use crate::desktop_host::{DesktopAction, DesktopShortcut, DesktopShortcuts, DesktopUpdateStatus};
use crate::linux_app::{HostHandle, HostRequest, HostSnapshot};
use crate::linux_input::LinuxHotkey;
use crate::linux_service::ShortcutTarget;

pub(super) const CAPABILITIES: Capabilities = Capabilities {
    meetings: false,
    commands: false,
    hud_lab: false,
    fun: false,
    modes: true,
    voice_action: true,
    history: true,
    permissions: false,
    dock_icon: false,
    launch_at_login: false,
    listener_control: true,
    paste_last: true,
    recording_audio: true,
    release_microphone: true,
    double_tap_only: true,
    modifier_sides: false,
    indicator_position: true,
    interface_language: true,
    software_updates: true,
    terminal_paste: true,
    practice_dictation: false,
};

/// The window records the chord from its key events; the host registers
/// it on X11 or asks the desktop's shortcuts portal for it on Wayland.
pub(super) fn records_key_events() -> bool {
    true
}

/// The host checks for updates on its own schedule.
/// How the Modes pane describes website rules; `None` hides them where
/// the listener can't read the browser's address.
pub(super) const WEB_PAGES_DESCRIPTION: Option<&str> = None;

/// Whether the listener knows which application is frontmost.
pub(super) fn focused_application_known() -> bool {
    !crate::linux_session::LinuxSession::detect().is_wayland()
}

/// The Modes application picker's button for an app it doesn't list.
pub(super) const CHOOSE_APPLICATION_FILE: &str = "Choose a file…";

pub(super) const MANUAL_UPDATE_CHECK: bool = false;

pub(super) struct Args(pub(super) HostHandle);

pub(super) struct Platform {
    host: HostHandle,
    state: Option<HostSnapshot>,
    error: Option<String>,
    /// What the window last showed of the host, to redraw only on change.
    shown: Option<(crate::desktop_host::DesktopSnapshot, Option<String>)>,
    capture: CapturePhase,
    /// The shortcut the host is recording or saving.
    capture_target: ShortcutTarget,
    /// The model the host is preparing, and since when.
    preparing: Option<(crate::transcription_models::TranscriptionModelId, Instant)>,
}

/// The host registers every shortcut.
fn shortcut_target(kind: HotkeyKind) -> Option<ShortcutTarget> {
    match kind {
        HotkeyKind::Dictation => Some(ShortcutTarget::Dictation),
        HotkeyKind::PasteLast => Some(ShortcutTarget::PasteLast),
        HotkeyKind::Edit => Some(ShortcutTarget::VoiceAction),
    }
}

/// The host's side of a shortcut change the window asked for.
#[derive(Clone, Copy, PartialEq)]
enum CapturePhase {
    Idle,
    /// Asked; the host has not reported the edit yet.
    Requested(Instant),
    /// The host is recording or saving the shortcut.
    Editing,
}

impl Platform {
    pub(super) fn new(
        Args(host): Args,
        _: Option<&AppWindowPreview>,
        _: &AppSettings,
        _: &StatusSnapshot,
        _: &mut Context<AppWindow>,
    ) -> Self {
        Self {
            state: Some(host.snapshot()),
            host,
            error: None,
            shown: None,
            capture: CapturePhase::Idle,
            capture_target: ShortcutTarget::Dictation,
            preparing: None,
        }
    }
}

/// Opens the shared window beside the host.
pub fn open(host: HostHandle, cx: &mut App) -> gpui::Result<WindowHandle<AppWindow>> {
    let history = host.history();
    open_new(host.event_path(), history, None, Args(host), cx)
}

pub(super) fn window_options(options: WindowOptions) -> WindowOptions {
    WindowOptions {
        app_id: Some("hex".into()),
        ..options
    }
}

pub(super) fn frame(frame: Div) -> Div {
    frame
}

pub(super) fn sidebar(sidebar: Div) -> Div {
    sidebar
}

/// Empty sidebar space moves the window.
/// Only macOS asks for Accessibility; the tour names it there.
pub(super) fn accessibility_title() -> &'static str {
    tr("Accessibility")
}

pub(super) fn window_drag_region(region: Div) -> Div {
    region.on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
}

pub(super) fn caption(_: &mut Window) -> Option<AnyElement> {
    None
}

/// The listener keeps its shortcut registered while the window records one;
/// the chord it already owns reaches the listener, not this window.
pub(super) fn set_hotkey_capture_active(_: bool) {}

pub(super) fn hotkey_modifiers(modifiers: GpuiModifiers) -> HotkeyModifiers {
    let held = |down: bool| down.then_some(ModifierSide::Either);
    HotkeyModifiers {
        control: held(modifiers.control),
        option: held(modifiers.alt),
        shift: held(modifiers.shift),
        command: held(modifiers.platform),
        function: false,
    }
}

pub(super) fn hotkey_key(key: &str) -> std::result::Result<HotkeyKey, &'static str> {
    if key.trim().is_empty() {
        Err(tr("Unsupported key"))
    } else {
        Ok(HotkeyKey::named(key))
    }
}

/// X11 and Wayland register a modifier chord with a key.
pub(super) fn validate_hotkey(
    _: HotkeyKind,
    binding: &HotkeyBinding,
) -> std::result::Result<(), &'static str> {
    LinuxHotkey::from(binding)
        .validate()
        .map_err(|_| tr("Add a key Linux can register"))
}

impl AppWindow {
    pub(super) fn poll_platform(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        // The window prepares the model, saves it, and the host loads it
        // when it adopts the settings.
        if !self.preview
            && let Some(result) = self.transcription_preparation.poll()
        {
            let result =
                result.and_then(|selection| self.apply_transcription_selection(selection, cx));
            if let Err(error) = result {
                self.transcription_preparation.set_error(Some(error));
            }
            self.poll_transcription_download();
            changed = true;
        }
        self.platform.state = Some(self.platform.host.snapshot());
        changed |= self.poll_service_capture();
        changed |= self.poll_service_preparation(cx);
        let shown = self
            .platform
            .state
            .as_ref()
            .map(|state| (state.desktop.clone(), self.platform.error.clone()));
        if shown != self.platform.shown {
            self.platform.shown = shown;
            changed = true;
        }
        changed
    }

    pub(super) fn platform_opened(&mut self, _: Option<&AppWindowPreview>) {}

    pub(super) fn platform_refresh(&mut self) {}

    pub(super) fn platform_select_pane(&mut self, _: Pane) {}

    pub(super) fn render_platform_pane(
        &mut self,
        _: Pane,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub(super) fn platform_settings_saved(&mut self) {
        self.platform.host.adopt_settings(self.settings.clone());
    }

    pub(super) fn platform_closed(&mut self) {
        self.platform.host.window_closed();
    }

    /// Wayland desktops grant the shortcuts, or leave them to `hex` commands.
    pub(super) fn platform_settings_notice(&mut self, _: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.platform.state.as_ref()?;
        let rows: Vec<AnyElement> = match &state.desktop.listener.shortcuts {
            DesktopShortcuts::Registered => return None,
            DesktopShortcuts::Portal(granted) => {
                let refused = granted.iter().any(|shortcut| shortcut.trigger.is_none());
                let mut rows = granted
                    .iter()
                    .map(|shortcut| {
                        let title = match shortcut.name.as_str() {
                            "Paste last dictation" => tr("Paste last dictation"),
                            "Voice Action" => tr("Voice Action"),
                            _ => tr("Dictation"),
                        };
                        let value = match &shortcut.trigger {
                            Some(trigger) => div()
                                .text_body()
                                .text_color(rgb(TEXT_SECONDARY))
                                .child(trigger.clone()),
                            None => div()
                                .text_body()
                                .text_color(rgb(DANGER))
                                .child(tr("Not granted")),
                        };
                        settings_row(title, tr("Granted by your desktop"), value).into_any_element()
                    })
                    .collect::<Vec<_>>();
                if refused {
                    rows.push(
                        shortcut_note(tr(
                            "Your desktop did not grant every shortcut. Record another one, or free it in your desktop's keyboard settings.",
                        ))
                        .text_color(rgb(DANGER))
                        .into_any_element(),
                    );
                }
                rows
            }
            DesktopShortcuts::Commands => {
                let mut rows = vec![
                    shortcut_note(tr(
                        "Your desktop has no shortcuts portal. Bind these commands in its keyboard settings.",
                    ))
                    .into_any_element(),
                    command_row(
                        tr("Hold to dictate"),
                        tr("Bind start to the key press and stop to its release"),
                        &["hex dictation start", "hex dictation stop"],
                    ),
                    command_row(
                        tr("Toggle dictation"),
                        tr("Press to start dictating, press again to transcribe"),
                        &["hex dictation toggle"],
                    ),
                    command_row(
                        tr("Cancel dictation"),
                        tr("Discard the current recording"),
                        &["hex dictation cancel"],
                    ),
                    command_row(
                        tr("Paste last dictation"),
                        tr("Re-paste the most recent dictation"),
                        &["hex paste-last"],
                    ),
                ];
                if self.settings.voice_action.enabled {
                    rows.push(command_row(
                        tr("Voice Action"),
                        tr("Hold to speak; selected text is included automatically"),
                        &["hex voice-action start", "hex voice-action stop"],
                    ));
                }
                rows
            }
        };
        Some(
            div()
                .child(settings_section_label(tr("Desktop shortcuts")))
                .child(panel().children(rows))
                .into_any_element(),
        )
    }

    /// Without a shortcuts portal the desktop's keybindings own every
    /// shortcut, so Settings has none to record.
    pub(super) fn platform_shortcut_rows(&self) -> bool {
        self.platform
            .state
            .as_ref()
            .is_none_or(|state| state.desktop.listener.shortcuts != DesktopShortcuts::Commands)
    }

    /// Linux has no tray: the window's way to stop Hex.
    pub(super) fn platform_settings_section(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        vec![
            panel()
                .child(settings_row(
                    tr("Quit Hex"),
                    tr("Closing the window keeps Hex dictating"),
                    crate::desktop_ui::button(
                        crate::desktop_ui::ButtonKind::Secondary,
                        tr("Quit Hex"),
                    )
                    .id("quit-hex")
                    .on_click(cx.listener(|this, _, _, _| this.platform.host.quit())),
                ))
                .into_any_element(),
        ]
    }

    pub(super) fn platform_model_notice(
        &mut self,
        _: &mut Context<Self>,
    ) -> Option<(&'static str, &'static str, AnyElement)> {
        None
    }

    pub(super) fn platform_tour_permission(&mut self, _: crate::onboarding::Effect) {}

    pub(super) fn platform_relaunch(&mut self) -> bool {
        false
    }

    pub(super) fn platform_awaits_setup(&self) -> bool {
        false
    }

    pub(super) fn platform_setup_ready(&mut self) {}

    pub(super) fn platform_poll_setup(&mut self) -> bool {
        false
    }

    pub(super) fn personal_commands_changed(&mut self) {}

    pub(super) fn platform_update_button(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Stateful<Div>> {
        matches!(
            self.platform_software_update(),
            Some(SoftwareUpdate::ReadyToRestart { .. })
        )
        .then(|| {
            sidebar_update_button().on_click(cx.listener(|this, _, _, cx| {
                this.install_update(cx);
            }))
        })
    }

    /// The host stops listening and records or awaits the chord.
    pub(super) fn platform_capture_started(&mut self, _: &mut Context<Self>) {
        if self.preview {
            return;
        }
        let HotkeyCaptureState::Listening { kind, .. } = self.hotkey_capture else {
            return;
        };
        let Some(target) = shortcut_target(kind) else {
            return;
        };
        self.send(HostRequest::CaptureShortcut(target));
        self.platform.capture_target = target;
        self.platform.capture = CapturePhase::Requested(Instant::now());
    }

    pub(super) fn platform_capture_cancelled(&mut self) {
        if self.platform.capture != CapturePhase::Idle {
            self.send(HostRequest::CancelShortcut);
            self.platform.capture = CapturePhase::Idle;
        }
    }

    /// The host validates, registers, and saves the chord recorded on X11.
    pub(super) fn platform_save_hotkey(
        &mut self,
        kind: HotkeyKind,
        binding: &HotkeyBinding,
    ) -> bool {
        let Some(target) = shortcut_target(kind) else {
            return false;
        };
        let modifiers = binding.modifiers;
        self.send(HostRequest::SetShortcut {
            target,
            shortcut: DesktopShortcut {
                alt: modifiers.option.is_some(),
                control: modifiers.control.is_some(),
                function: false,
                key: binding
                    .key
                    .as_ref()
                    .map(HotkeyKey::name)
                    .unwrap_or_default(),
                platform: modifiers.command.is_some(),
                shift: modifiers.shift.is_some(),
            },
        });
        true
    }

    /// The host prepares models so closing the window never cancels one.
    pub(super) fn platform_prepare_transcription(
        &mut self,
        selection: &TranscriptionSelection,
    ) -> bool {
        self.send(HostRequest::ChooseModel {
            model: selection.model,
            language: selection.language.clone(),
        });
        true
    }

    pub(super) fn platform_cancel_transcription(&mut self) -> bool {
        self.send(HostRequest::CancelModel);
        true
    }

    pub(super) fn platform_preparation_status(&self) -> Option<PreparationStatus> {
        let transcription = &self.platform.state.as_ref()?.desktop.transcription;
        Some(PreparationStatus {
            model: transcription.preparing,
            downloaded_bytes: transcription.downloaded_bytes,
            started: self.platform.preparing.map(|(_, started)| started),
            error: transcription.error.clone(),
        })
    }

    fn send(&mut self, request: HostRequest) {
        self.platform.error = self
            .platform
            .host
            .request(request)
            .err()
            .map(|error| format!("{error:#}"));
    }

    /// Follows the host's shortcut edit and adopts the shortcut it saved.
    fn poll_service_capture(&mut self) -> bool {
        let Some(state) = &self.platform.state else {
            return false;
        };
        let editing = state.editing;
        let finished = match self.platform.capture {
            CapturePhase::Idle => return false,
            CapturePhase::Requested(_) if editing => {
                self.platform.capture = CapturePhase::Editing;
                return false;
            }
            CapturePhase::Requested(at) => at.elapsed() >= Duration::from_secs(3),
            CapturePhase::Editing => !editing,
        };
        if !finished {
            return false;
        }
        self.platform.capture = CapturePhase::Idle;
        let (saved, current, kind) = match self.platform.capture_target {
            ShortcutTarget::Dictation => (
                Some(state.settings.dictation_hotkey.clone()),
                Some(self.settings.dictation_hotkey.clone()),
                HotkeyKind::Dictation,
            ),
            ShortcutTarget::PasteLast => (
                state.settings.paste_last_hotkey.clone(),
                self.settings.paste_last_hotkey.clone(),
                HotkeyKind::PasteLast,
            ),
            ShortcutTarget::VoiceAction => (
                Some(state.settings.edit_hotkey.clone()),
                Some(self.settings.edit_hotkey.clone()),
                HotkeyKind::Edit,
            ),
        };
        if let Some(binding) = saved.filter(|saved| Some(saved) != current.as_ref()) {
            let keycaps = binding.keycaps();
            match kind {
                HotkeyKind::PasteLast => self.settings.paste_last_hotkey = Some(binding),
                HotkeyKind::Edit => self.settings.edit_hotkey = binding,
                HotkeyKind::Dictation => self.settings.dictation_hotkey = binding,
            }
            self.hotkey_width_spring
                .set_target(hotkey_saved_width(keycaps.len()) + hotkey_label_overflow(&keycaps));
            self.hotkey_capture = HotkeyCaptureState::Saved {
                kind,
                saved_at: Instant::now(),
            };
        } else if !matches!(self.hotkey_capture, HotkeyCaptureState::Idle) {
            self.hotkey_capture = HotkeyCaptureState::Idle;
            self.hotkey_capture_animation.set_enabled(false);
        }
        true
    }

    /// Tracks the host's model preparation and adopts the selection it
    /// saved when it finishes.
    fn poll_service_preparation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(state) = &self.platform.state else {
            return false;
        };
        let transcription = &state.desktop.transcription;
        if let Some(model) = transcription.preparing {
            if self.platform.preparing.map(|(preparing, _)| preparing) != Some(model) {
                self.platform.preparing = Some((model, Instant::now()));
            }
            return false;
        }
        if self.platform.preparing.take().is_none()
            || transcription.error.is_some()
            || state.settings.transcription == self.settings.transcription
        {
            return false;
        }
        self.settings.transcription = state.settings.transcription.clone();
        self.settings.transcription_recents = state.settings.transcription_recents.clone();
        let hints = self.settings.transcription.recognition_hints.clone();
        self.transcription_hints
            .entity
            .update(cx, |input, cx| input.set_text(&hints, cx));
        self.transcription_picker_language = None;
        true
    }

    pub(super) fn platform_listener(&self) -> Option<ListenerView> {
        let error = self.platform.error.clone();
        let Some(state) = &self.platform.state else {
            return Some(ListenerView {
                running: false,
                status: tr("Service disconnected").into(),
                last_failure: None,
                error,
            });
        };
        let desktop = &state.desktop;
        Some(ListenerView {
            running: desktop.listener.running,
            status: desktop.listener.status.clone(),
            last_failure: desktop.listener.warning.clone(),
            error: error.or_else(|| desktop.operation_error.clone()),
        })
    }

    pub(super) fn set_listening(&mut self, enabled: bool) {
        let action = if enabled {
            DesktopAction::StartListening
        } else {
            DesktopAction::StopListening
        };
        self.send(HostRequest::Desktop(action));
    }

    /// Only managed installs update themselves.
    pub(super) fn platform_software_update(&self) -> Option<SoftwareUpdate> {
        let state = self.platform.state.as_ref()?;
        Some(match state.desktop.update_status {
            DesktopUpdateStatus::Unavailable => return None,
            DesktopUpdateStatus::Checking => SoftwareUpdate::Checking,
            DesktopUpdateStatus::Current => SoftwareUpdate::Current,
            DesktopUpdateStatus::Failed => SoftwareUpdate::Failed,
            DesktopUpdateStatus::ReadyToRestart => SoftwareUpdate::ReadyToRestart { version: None },
        })
    }

    /// The host checks on its own schedule.
    pub(super) fn check_for_updates(&mut self) {}

    pub(super) fn install_update(&mut self, _: &mut Context<Self>) {
        self.send(HostRequest::Desktop(DesktopAction::RestartIntoUpdate));
    }
}

fn shortcut_note(text: &'static str) -> Div {
    div()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(rgb(LINE))
        .text_footnote()
        .text_color(rgb(TEXT_TERTIARY))
        .child(text)
}

fn command_row(
    title: &'static str,
    description: &'static str,
    commands: &[&'static str],
) -> AnyElement {
    settings_row(
        title,
        description,
        div()
            .flex()
            .flex_col()
            .items_end()
            .children(commands.iter().map(|command| {
                div()
                    .text_footnote()
                    .text_monospace()
                    .text_color(rgb(TEXT_SECONDARY))
                    .child(*command)
            })),
    )
    .into_any_element()
}
