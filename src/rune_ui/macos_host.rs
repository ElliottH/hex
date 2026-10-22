//! The macOS window's host: the runtime thread `macos_rune` starts in this
//! process, which owns model preparation and publishes the listener, model,
//! update, and settings state, plus the OS-only work the panes ask for
//! (permissions, the login item, Hex Microphone, meetings, commands).
//!
//! Shortcut capture stays in the window, as in GPUI: an AppKit local event
//! monitor records the chord, because a modifier-only binding such as Option
//! or Fn never reaches Rune as a key event. The listener's event tap is
//! suspended meanwhile so the current binding can be recorded again.

use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use block2::RcBlock;
use color_eyre::Result;
use color_eyre::eyre::eyre;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType};

use super::platform::{
    CatalogCommand, CommandModel, DemoRequest, HudDemo, HudLab, MeetingRequest, NativeHost,
    NotesAvailability, PlatformRequest, PlatformState, Workspace,
};
use super::settings::Chord;
use super::{HostState, ServiceView};
use crate::app_settings::{HotkeyBinding, HotkeyKey, HotkeyModifiers, modifiers_from_flags};
use crate::commands::{CommandInfo, CommandScope};
use crate::desktop_host::DesktopAction;
use crate::desktop_i18n::tr;
use crate::login_item::{LoginItemRequest, LoginItemWorker};
use crate::macos_rune::RuntimeHandle;
pub(super) use crate::macos_rune::{Request, ShortcutTarget};
use crate::onboarding::{PermissionAction, PermissionKind, PermissionState};

/// The window saves a recorded shortcut; the listener follows `settings.json`.
pub(super) const HOST_SAVES_SHORTCUTS: bool = false;
pub(super) const RECORDS_KEY_EVENTS: bool = false;
/// Sparkle offers "Check now".
pub(super) const MANUAL_UPDATE_CHECK: bool = true;
pub(super) const TERMINAL_PASTE: bool = false;
/// Quit Hex lives in the menu bar and the app menu (⌘Q), as under GPUI.
pub(super) const QUIT_IN_WINDOW: bool = false;

/// How often the native host re-reads permissions and driver state.
const PLATFORM_REFRESH: Duration = Duration::from_secs(1);
const ESCAPE_KEY_CODE: u16 = 53;

pub(super) struct Backend {
    runtime: Option<RuntimeHandle>,
    wake: Arc<dyn Fn() + Send + Sync>,
    chords: Option<ChordWatch>,
}

impl Backend {
    pub(super) fn connect(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let runtime = crate::macos_rune::runtime();
        if let Some(runtime) = &runtime {
            runtime.subscribe(wake.clone());
        }
        Self {
            runtime,
            wake,
            chords: None,
        }
    }

    /// Runs on the main thread (Rune's handlers), where Sparkle and the
    /// event monitor live.
    pub(super) fn send(&mut self, request: Request) -> Result<()> {
        match request {
            Request::CaptureShortcutFor(_) => {
                self.chords = Some(ChordWatch::start(self.wake.clone())?);
                Ok(())
            }
            Request::CancelShortcut => {
                self.chords = None;
                Ok(())
            }
            Request::Desktop(
                DesktopAction::CheckForUpdates
                | DesktopAction::OpenUpdate
                | DesktopAction::RestartIntoUpdate,
            ) => {
                crate::sparkle::check_for_updates();
                Ok(())
            }
            request => match &self.runtime {
                Some(runtime) => runtime.send(request),
                None => Err(eyre!("Hex's runtime is not running")),
            },
        }
    }

    /// Saves `settings.json`, then asks the runtime to reload it. The
    /// listener follows the saved file itself.
    pub(super) fn save(&mut self, settings: &crate::app_settings::AppSettings) -> Result<()> {
        settings.save()?;
        self.send(Request::ReloadSettings)
    }

    pub(super) fn quit(&mut self) {
        crate::macos_rune::quit();
    }

    /// Rune hides the window (`keep_running`); the window's own close hook
    /// restores the Dock policy (`macos_rune::window_closed`).
    pub(super) fn closed(&mut self) {}

    /// The runtime publishes into shared memory; nothing to pull.
    pub(super) fn refresh(&mut self) {}

    pub(super) fn error(&self) -> Option<String> {
        self.runtime
            .is_none()
            .then(|| "Hex's runtime is not running".into())
    }

    pub(super) fn state(&self) -> Option<HostState> {
        let published = self.runtime.as_ref()?.published()?;
        Some(HostState {
            view: ServiceView {
                listener: published.listener.clone(),
                operation_error: published.operation_error.clone(),
                transcription: published.transcription.clone(),
                update_status: published.update_status,
                editing: false,
            },
            settings: published.settings.clone(),
        })
    }

    pub(super) fn take_chord(&mut self) -> Option<Chord> {
        let chord = self.chords.as_ref()?.results.try_recv().ok();
        if matches!(chord, Some(Chord::Cancelled)) {
            self.chords = None;
        }
        chord
    }
}

/// Records chords from the window's keyboard events while Settings listens,
/// with the listener's event tap suspended. Removing the monitor ends it.
struct ChordWatch {
    results: Receiver<Chord>,
    monitor: Retained<AnyObject>,
}

impl ChordWatch {
    fn start(wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self> {
        crate::app_settings::set_hotkey_capture_active(true);
        let (sender, results) = mpsc::channel();
        let capture = RefCell::new(ChordCapture::default());
        let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a live event for the handler's duration.
            let event_ref = unsafe { event.as_ref() };
            let flags = event_ref.modifierFlags().bits() as u64;
            let chord = match event_ref.r#type() {
                NSEventType::KeyDown => {
                    let characters = event_ref
                        .charactersIgnoringModifiers()
                        .map(|characters| characters.to_string());
                    capture.borrow_mut().key_down(
                        event_ref.keyCode(),
                        event_ref.isARepeat(),
                        characters.as_deref(),
                        flags,
                    )
                }
                NSEventType::FlagsChanged => capture.borrow_mut().flags_changed(flags),
                // Key releases still reach the window so its key state
                // stays balanced.
                _ => return event.as_ptr(),
            };
            if let Some(chord) = chord
                && sender.send(chord).is_ok()
            {
                wake();
            }
            std::ptr::null_mut()
        });
        // SAFETY: the block lives as long as AppKit retains it; the monitor
        // is removed on drop, on the main thread that installed it.
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::KeyDown | NSEventMask::FlagsChanged,
                &block,
            )
        };
        match monitor {
            Some(monitor) => Ok(Self { results, monitor }),
            None => {
                crate::app_settings::set_hotkey_capture_active(false);
                Err(eyre!("could not watch the keyboard to record a shortcut"))
            }
        }
    }
}

impl Drop for ChordWatch {
    fn drop(&mut self) {
        // SAFETY: the monitor came from addLocalMonitor on this thread.
        unsafe { NSEvent::removeMonitor(&self.monitor) };
        crate::app_settings::set_hotkey_capture_active(false);
    }
}

/// Turns key and modifier events into a binding the way GPUI's Settings
/// does: a key with its held modifiers, or the most modifiers held at once
/// when they are all released without a key.
#[derive(Default)]
struct ChordCapture {
    peak: HotkeyModifiers,
    /// A key went down with these modifiers; their release records nothing.
    keyed: bool,
}

impl ChordCapture {
    fn key_down(
        &mut self,
        code: u16,
        repeat: bool,
        characters: Option<&str>,
        flags: u64,
    ) -> Option<Chord> {
        if repeat {
            return None;
        }
        if code == ESCAPE_KEY_CODE {
            return Some(Chord::Cancelled);
        }
        self.keyed = true;
        self.peak = HotkeyModifiers::default();
        let key = match hotkey_key(code, characters) {
            Ok(key) => key,
            Err(message) => return Some(Chord::Captured(Err(message))),
        };
        let mut modifiers = modifiers_from_flags(flags).without_side_constraints();
        // Arrow and function keys report Fn; a chord never binds it.
        modifiers.function = false;
        if modifiers.is_empty() && !is_function_key(&key.label) {
            return Some(Chord::Captured(Err(tr("Add a modifier"))));
        }
        Some(Chord::Captured(Ok(HotkeyBinding {
            modifiers,
            key: Some(key),
        })))
    }

    fn flags_changed(&mut self, flags: u64) -> Option<Chord> {
        let current = normalize_captured_modifiers(modifiers_from_flags(flags));
        if !current.is_empty() {
            if !self.keyed && current.count() >= self.peak.count() {
                self.peak = current;
            }
            return None;
        }
        let keyed = std::mem::take(&mut self.keyed);
        let peak = std::mem::take(&mut self.peak);
        (!keyed && !peak.is_empty()).then_some(Chord::Captured(Ok(HotkeyBinding {
            modifiers: peak,
            key: None,
        })))
    }
}

/// One side for every modifier when they all share it; otherwise either
/// side, since the listener can't require mixed sides.
fn normalize_captured_modifiers(modifiers: HotkeyModifiers) -> HotkeyModifiers {
    let mut sides = [
        modifiers.control,
        modifiers.option,
        modifiers.shift,
        modifiers.command,
    ]
    .into_iter()
    .flatten();
    let Some(first) = sides.next() else {
        return modifiers;
    };
    if sides.all(|side| side == first) {
        modifiers
    } else {
        modifiers.without_side_constraints()
    }
}

fn is_function_key(label: &str) -> bool {
    label
        .strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=20).contains(&number))
}

/// The key a virtual key code records, labeled as GPUI's Settings labels it.
fn hotkey_key(code: u16, characters: Option<&str>) -> Result<HotkeyKey, &'static str> {
    let special = match code {
        49 => Some("Space"),
        48 => Some("Tab"),
        36 => Some("Return"),
        51 => Some(tr("Delete")),
        126 => Some("Up"),
        125 => Some("Down"),
        123 => Some(tr("Left")),
        124 => Some(tr("Right")),
        116 => Some("Page Up"),
        121 => Some("Page Down"),
        115 => Some("Home"),
        119 => Some("End"),
        117 => Some("Forward Delete"),
        114 => Some("Help"),
        122 => Some("F1"),
        120 => Some("F2"),
        99 => Some("F3"),
        118 => Some("F4"),
        96 => Some("F5"),
        97 => Some("F6"),
        98 => Some("F7"),
        100 => Some("F8"),
        101 => Some("F9"),
        109 => Some("F10"),
        103 => Some("F11"),
        111 => Some("F12"),
        105 => Some("F13"),
        107 => Some("F14"),
        113 => Some("F15"),
        106 => Some("F16"),
        64 => Some("F17"),
        79 => Some("F18"),
        80 => Some("F19"),
        90 => Some("F20"),
        _ => None,
    };
    if let Some(label) = special {
        return Ok(HotkeyKey {
            code,
            label: label.into(),
        });
    }
    let mut chars = characters.unwrap_or_default().chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) if !character.is_control() => Ok(HotkeyKey {
            code,
            label: character.to_uppercase().collect(),
        }),
        _ => Err(tr("Unsupported key")),
    }
}

/// macOS accepts every binding the capture can produce.
pub(super) fn validate_hotkey(_: ShortcutTarget, _: &HotkeyBinding) -> Result<(), &'static str> {
    Ok(())
}

/// The listener's retained history, shared with the window.
pub(super) fn history(
    retention: crate::history::HistoryRetention,
) -> Option<crate::history::History> {
    match crate::macos_rune::runtime() {
        Some(runtime) => runtime.history(),
        None => crate::history::History::open_default(retention)
            .inspect_err(|error| tracing::warn!(%error, "could not open retained history"))
            .ok(),
    }
}

/// The listener knows the frontmost application.
pub(super) fn focused_application_known() -> bool {
    true
}

/// The native host reports the login item (`PlatformState::login_item`).
pub(super) fn launch_at_login() -> Option<Result<bool, String>> {
    None
}

pub(super) fn set_launch_at_login(_: bool) -> Result<bool, String> {
    Err("Launch at login goes through the macOS host".into())
}

pub(super) fn native_host() -> Box<dyn NativeHost> {
    Box::new(MacHost::new())
}

/// Performs the panes' OS-only requests and refreshes their state once a
/// second (permission checks and the driver status are not free per frame).
struct MacHost {
    login: Option<LoginItemWorker>,
    login_checking: bool,
    compiled: Vec<CatalogCommand>,
    personal: Vec<CatalogCommand>,
    workspace_path: PathBuf,
    workspace: Option<Receiver<Result<PathBuf, String>>>,
    workspace_error: Option<String>,
    relaunch_failed: bool,
    folder_error: Option<String>,
    refresh_at: Instant,
    notes: Option<NotesAvailability>,
}

impl MacHost {
    fn new() -> Self {
        let login = LoginItemWorker::new()
            .inspect_err(|error| tracing::warn!(%error, "login item worker is unavailable"))
            .ok();
        let compiled = crate::config::voice_control()
            .catalog()
            .into_iter()
            .map(|command| CatalogCommand {
                task: command_task(&command),
                scope: command.scope,
                phrase: command.phrase,
                aliases: command.aliases,
                description: command.description,
                id: command.id,
            })
            .collect();
        let mut host = Self {
            login,
            login_checking: false,
            compiled,
            personal: Vec::new(),
            workspace_path: crate::app_paths::personal_commands_workspace().unwrap_or_default(),
            workspace: None,
            workspace_error: None,
            relaunch_failed: false,
            folder_error: None,
            refresh_at: Instant::now(),
            notes: None,
        };
        host.reload_personal_commands();
        host.request_login(LoginItemRequest::Status);
        host
    }

    fn reload_personal_commands(&mut self) {
        self.personal = match crate::personal_commands::load_status() {
            Ok(status) => personal_catalog(&status),
            Err(error) => {
                tracing::debug!(%error, "personal commands are unavailable");
                Vec::new()
            }
        };
    }

    fn request_login(&mut self, request: LoginItemRequest) {
        if let Some(login) = &mut self.login {
            match login.request(request) {
                Ok(()) => self.login_checking = true,
                Err(error) => tracing::warn!(%error, "could not ask for the login item"),
            }
        }
    }

    fn runtime(&self) -> Result<RuntimeHandle, String> {
        crate::macos_rune::runtime().ok_or_else(|| "Hex's runtime is not running".into())
    }

    fn meeting(&mut self, request: MeetingRequest) -> Result<(), String> {
        match request {
            MeetingRequest::Start => self
                .runtime()?
                .meeting(crate::meeting::MeetingRequest::Start),
            MeetingRequest::Stop => self
                .runtime()?
                .meeting(crate::meeting::MeetingRequest::Stop),
            MeetingRequest::Summarize(id) => self
                .runtime()?
                .meeting(crate::meeting::MeetingRequest::Summarize(id)),
            MeetingRequest::Rename { id, title } => crate::meeting::rename(&id, &title)
                .map(|_| ())
                .map_err(|error| format!("{error:#}")),
            MeetingRequest::Trash(id) => {
                crate::meeting::trash(&id).map_err(|error| format!("{error:#}"))
            }
            MeetingRequest::Reveal(id) => {
                let root = crate::meeting::root().map_err(|error| format!("{error:#}"))?;
                open_path(&root.join(id));
                Ok(())
            }
            MeetingRequest::RevealFolder => {
                let folder = crate::app_settings::meeting_output_folder();
                match std::fs::create_dir_all(&folder) {
                    Ok(()) => {
                        crate::meeting::reveal(&folder);
                        self.folder_error = None;
                    }
                    Err(error) => self.folder_error = Some(error.to_string()),
                }
                Ok(())
            }
            MeetingRequest::ChooseFolder => {
                Err("Choosing a meetings folder is not available in this window yet".into())
            }
        }
    }
}

impl NativeHost for MacHost {
    fn send(&mut self, request: PlatformRequest) -> Result<(), String> {
        match request {
            PlatformRequest::Permission(PermissionAction::RequestMicrophone) => {
                crate::onboarding::request_microphone();
            }
            PlatformRequest::Permission(PermissionAction::ShowGuide(kind)) => {
                crate::permission_guide::show(kind);
            }
            PlatformRequest::SystemAudio => {
                // macOS prompts once; after that the switch is in System
                // Settings, with the guide beside it.
                if !crate::meeting::request_system_audio_access() {
                    crate::permission_guide::show(PermissionKind::SystemAudio);
                }
            }
            PlatformRequest::SetLaunchAtLogin(enabled) => {
                self.request_login(LoginItemRequest::SetEnabled(enabled));
            }
            PlatformRequest::OpenLoginItemSettings => {
                self.request_login(LoginItemRequest::OpenSettings);
            }
            PlatformRequest::InstallHexMicrophone => crate::hex_microphone::install_driver_async(),
            PlatformRequest::UninstallHexMicrophone => {
                crate::hex_microphone::uninstall_driver_async()
            }
            PlatformRequest::Meeting(request) => self.meeting(request)?,
            PlatformRequest::CreateCommandsConfig => {
                if self.workspace.is_none() {
                    let (sender, receiver) = mpsc::sync_channel(1);
                    std::thread::spawn(move || {
                        let result = crate::personal_commands::initialize_workspace()
                            .map_err(|error| format!("{error:#}"));
                        let _ = sender.send(result);
                    });
                    self.workspace = Some(receiver);
                    self.workspace_error = None;
                }
            }
            PlatformRequest::OpenCommandsConfig => open_path(&self.workspace_path),
            PlatformRequest::RetryCommandModel => self
                .runtime()?
                .recognition(crate::recognition::RecognitionControl::RetryCommands)?,
            PlatformRequest::Relaunch => {
                if crate::onboarding::spawn_relaunch() {
                    crate::macos_rune::quit();
                } else {
                    self.relaunch_failed = true;
                }
            }
            PlatformRequest::SetTuning(tuning) => {
                crate::hex_microphone::update_tuned_config(|current| *current = tuning);
            }
            PlatformRequest::HexMicrophoneDemo(request) => match request {
                DemoRequest::Record => crate::hex_microphone::start_demo_recording(),
                DemoRequest::StopRecording => crate::hex_microphone::stop_demo_recording(),
                DemoRequest::SelectSource(source) => {
                    crate::hex_microphone::select_demo_source(source)
                }
                DemoRequest::Play(track) => crate::hex_microphone::play_demo_track(track),
                DemoRequest::StopPlayback => crate::hex_microphone::stop_demo_playback(),
            },
            PlatformRequest::ConfigureHud(lab) => configure_hud(&self.runtime()?, lab),
        }
        // Show the result of an action at once, not on the next refresh.
        self.refresh_at = Instant::now();
        Ok(())
    }

    fn poll(&mut self, state: &mut PlatformState) -> bool {
        let mut changed = false;
        if let Some(login) = &mut self.login {
            match login.poll() {
                Ok(Some(response)) => {
                    self.login_checking = false;
                    match response.result {
                        Ok(status) => {
                            state.login_item = Some(status);
                            state.login_item_error = None;
                        }
                        Err(failure) => {
                            if failure.status.is_some() {
                                state.login_item = failure.status;
                            }
                            state.login_item_error = Some(failure.message);
                        }
                    }
                    changed = true;
                }
                Ok(None) => {}
                Err(error) => {
                    self.login = None;
                    self.login_checking = false;
                    state.login_item_error = Some(error);
                    changed = true;
                }
            }
        }
        if let Some(receiver) = &self.workspace
            && let Ok(result) = receiver.try_recv()
        {
            self.workspace = None;
            match result {
                Ok(path) => {
                    self.workspace_path = path;
                    self.workspace_error = None;
                    self.reload_personal_commands();
                }
                Err(error) => self.workspace_error = Some(error),
            }
            changed = true;
        }
        // The Fun pane's demo moves every frame while it records or plays.
        if state.demo.is_none()
            || crate::hex_microphone::demo_is_busy()
            || crate::hex_microphone::demo_recompute_is_running()
            || state.demo.as_ref().is_some_and(|demo| demo.is_recording || demo.playing.is_some())
        {
            state.demo = Some(crate::hex_microphone::demo_snapshot());
            changed = true;
        }
        if Instant::now() < self.refresh_at {
            return changed;
        }
        self.refresh_at = Instant::now() + PLATFORM_REFRESH;
        let before = state.clone();
        state.tuning = crate::hex_microphone::current_tuned_config();
        state.accessibility_renamed =
            crate::permission_guide::list_name(PermissionKind::Accessibility) != "Accessibility";
        state.activity_log = crate::app_paths::logs_dir()
            .ok()
            .map(|logs| logs.join("live.ndjson"));
        state.permissions = crate::onboarding::status_with_transcription_model(
            state.permissions.transcription_model,
        );
        state.system_audio = crate::meeting::system_audio_ready();
        let status = state.permissions;
        let system_audio = state.system_audio;
        state.permissions_asked.retain(|kind| match kind {
            PermissionKind::Microphone => status.microphone != PermissionState::Ready,
            PermissionKind::InputMonitoring => status.input_monitoring != PermissionState::Ready,
            PermissionKind::Accessibility => status.accessibility != PermissionState::Ready,
            PermissionKind::SystemAudio => !system_audio,
        });
        state.login_item_checking = self.login_checking;
        state.hex_microphone = Some(crate::hex_microphone::status());
        state.command_model = match crate::recognition::command_model_status() {
            crate::recognition::CommandModelStatus::Disabled => CommandModel::Disabled,
            crate::recognition::CommandModelStatus::Loading => CommandModel::Loading,
            crate::recognition::CommandModelStatus::Ready => CommandModel::Ready,
            crate::recognition::CommandModelStatus::Failed => CommandModel::Failed,
        };
        state.commands = self
            .compiled
            .iter()
            .chain(&self.personal)
            .cloned()
            .collect();
        state.workspace = if self.workspace.is_some() {
            Workspace::Creating
        } else if self.workspace_error.is_some() {
            Workspace::Error
        } else if crate::personal_commands::workspace_configured() {
            Workspace::Ready
        } else {
            Workspace::Missing
        };
        state.workspace_path = self.workspace_path.display().to_string();
        state.workspace_error = self.workspace_error.clone();
        state.notes = *self.notes.get_or_insert_with(notes_availability);
        state.meeting_folder_error = self.folder_error.clone();
        state.relaunch_failed = self.relaunch_failed;
        changed || !same_platform_state(&before, state)
    }
}

/// Shows HUD Lab's tuning and demonstrated state on the dictation HUD, as
/// GPUI's `apply_hud_lab`.
fn configure_hud(runtime: &RuntimeHandle, lab: HudLab) {
    use crate::dictation_indicator::{DictationIndicatorEvent as Event, HudTuning};
    let t = lab.tuning;
    runtime.indicator(Event::Configure(HudTuning {
        style: t.style,
        line_count: t.line_count,
        curvature: t.curvature,
        speed: t.speed,
        sharpness: t.sharpness,
        glow: t.glow,
        depth: t.depth,
        light_angle: t.light_angle,
        outline: t.outline,
    }));
    runtime.indicator(Event::Reset);
    match lab.demo {
        HudDemo::Recording => {
            for job_id in 0..lab.queued as u64 {
                runtime.indicator(Event::Submitted { job_id });
                runtime.indicator(Event::Transcribing { job_id });
            }
            runtime.indicator(Event::Started);
            runtime.indicator(Event::Meter {
                average: 0.08,
                peak: 0.42,
            });
        }
        HudDemo::Transcribing | HudDemo::Processing => {
            runtime.indicator(Event::Submitted { job_id: 0 });
            runtime.indicator(if lab.demo == HudDemo::Transcribing {
                Event::Transcribing { job_id: 0 }
            } else {
                Event::Processing { job_id: 0 }
            });
            for job_id in 1..=lab.queued as u64 {
                runtime.indicator(Event::Submitted { job_id });
                runtime.indicator(Event::Transcribing { job_id });
            }
        }
    }
}

/// `PlatformState` has no `PartialEq` (its driver status doesn't); compare
/// what the panes draw.
fn same_platform_state(left: &PlatformState, right: &PlatformState) -> bool {
    format!("{left:?}") == format!("{right:?}")
}

fn notes_availability() -> NotesAvailability {
    use crate::apple_intelligence::Availability;
    match crate::apple_intelligence::availability() {
        Availability::Available => NotesAvailability::Available,
        Availability::DeviceNotEligible => NotesAvailability::DeviceNotEligible,
        Availability::AppleIntelligenceNotEnabled => NotesAvailability::NotEnabled,
        Availability::ModelNotReady => NotesAvailability::ModelNotReady,
        Availability::UnsupportedSystem => NotesAvailability::UnsupportedSystem,
        Availability::Unknown => NotesAvailability::Unknown,
    }
}

fn open_path(path: &std::path::Path) {
    if let Err(error) = std::process::Command::new("/usr/bin/open")
        .arg(path)
        .spawn()
    {
        tracing::error!(%error, path = %path.display(), "could not open path");
    }
}

/// The catalog group GPUI shows for a compiled command.
fn command_task(command: &CommandInfo) -> String {
    if let Some(group) = &command.group {
        return group.clone();
    }
    if command.id.starts_with("mode.") {
        tr("Voice control").into()
    } else if command.id.starts_with("dictation.") {
        tr("Dictation").into()
    } else if command.id.starts_with("meeting.") {
        tr("Meetings").into()
    } else {
        match command.description.as_str() {
            "Open application" => tr("Open applications"),
            "Open website" => tr("Open websites"),
            "Navigate active tab" => tr("Browser navigation"),
            "Use keyboard shortcut" => tr("Keyboard"),
            "Use application shortcut" | "Open application destination" => {
                tr("Application navigation")
            }
            _ => tr("Other"),
        }
        .into()
    }
}

fn personal_catalog(status: &crate::personal_commands::StatusSnapshot) -> Vec<CatalogCommand> {
    use crate::personal_commands::StatusContext;
    status
        .catalog
        .iter()
        .filter_map(|command| {
            let (phrase, aliases) = command.phrases.split_first()?;
            Some(CatalogCommand {
                task: command.group.clone(),
                scope: match &command.context {
                    StatusContext::Global => CommandScope::Global,
                    StatusContext::Application { application } => {
                        CommandScope::Application(application.clone())
                    }
                    StatusContext::BrowserHost { browser_host } => {
                        CommandScope::Browser(browser_host.clone())
                    }
                },
                phrase: phrase.clone(),
                aliases: aliases.to_vec(),
                description: command.description.clone(),
                id: command.id.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::ModifierSide;

    const LEFT_OPTION: u64 = 0x0008_0020;
    const RIGHT_OPTION: u64 = 0x0008_0040;
    const LEFT_COMMAND: u64 = 0x0010_0008;
    const FUNCTION: u64 = 0x0080_0000;

    fn captured(chord: Option<Chord>) -> Result<HotkeyBinding, &'static str> {
        match chord {
            Some(Chord::Captured(result)) => result,
            Some(Chord::Cancelled) => panic!("cancelled"),
            None => panic!("nothing recorded"),
        }
    }

    #[test]
    fn releasing_modifiers_records_the_most_held_with_their_side() {
        let mut capture = ChordCapture::default();
        assert!(capture.flags_changed(LEFT_OPTION).is_none());
        assert!(capture.flags_changed(LEFT_OPTION | LEFT_COMMAND).is_none());
        assert!(capture.flags_changed(LEFT_OPTION).is_none());
        let binding = captured(capture.flags_changed(0)).unwrap();
        assert_eq!(binding.key, None);
        assert_eq!(binding.modifiers.option, Some(ModifierSide::Left));
        assert_eq!(binding.modifiers.command, Some(ModifierSide::Left));
        let binding = {
            assert!(capture.flags_changed(RIGHT_OPTION).is_none());
            captured(capture.flags_changed(0)).unwrap()
        };
        assert_eq!(binding.modifiers.option, Some(ModifierSide::Right));
        assert!(capture.flags_changed(FUNCTION).is_none());
        assert!(
            captured(capture.flags_changed(0))
                .unwrap()
                .modifiers
                .function
        );
    }

    #[test]
    fn a_key_records_its_modifiers_on_either_side_and_their_release_nothing() {
        let mut capture = ChordCapture::default();
        assert!(capture.flags_changed(LEFT_OPTION).is_none());
        let binding = captured(capture.key_down(9, false, Some("v"), LEFT_OPTION)).unwrap();
        assert_eq!(binding.modifiers.option, Some(ModifierSide::Either));
        assert_eq!(
            binding.key,
            Some(HotkeyKey {
                code: 9,
                label: "V".into()
            })
        );
        assert!(capture.flags_changed(0).is_none());
        assert!(capture.key_down(9, true, Some("v"), LEFT_OPTION).is_none());
    }

    #[test]
    fn keys_without_modifiers_need_one_except_function_keys() {
        let mut capture = ChordCapture::default();
        assert_eq!(
            captured(capture.key_down(0, false, Some("a"), 0)),
            Err(tr("Add a modifier"))
        );
        let f12 = captured(capture.key_down(111, false, None, FUNCTION)).unwrap();
        assert!(f12.modifiers.is_empty());
        assert_eq!(f12.key.unwrap().label, "F12");
        assert!(matches!(
            capture.key_down(ESCAPE_KEY_CODE, false, None, 0),
            Some(Chord::Cancelled)
        ));
    }
}
