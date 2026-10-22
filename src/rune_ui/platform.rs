//! What the current OS's runtime can do, the OS-only state the panes show,
//! and the requests they make of it.
//!
//! The panes never call macOS or Windows APIs. Permissions, the login item,
//! the meetings store, the command catalog, and the Hex Microphone driver
//! reach them as plain data in [`PlatformState`], and every action they take
//! goes out as a [`PlatformRequest`] through [`PlatformHost`]. Linux scenes
//! and tests feed preview data and record the requests, so macOS and Windows
//! surfaces render and are tested on Linux; a native host fills the same
//! data on its own OS.

use crate::commands::CommandScope;
use crate::hex_microphone::{
    DemoPlaybackTrack, DemoSnapshot, DemoSourceKind, HexMicrophoneStatus, TunedFilterConfig,
};
use crate::login_item::LoginItemStatus;
use crate::onboarding::{PermissionAction, PermissionKind, PermissionState, SetupStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Os {
    Linux,
    Macos,
    Windows,
}

/// What the runtime can do. A missing capability omits its pane or row; the
/// window never shows an imitation of a feature the runtime cannot perform.
/// Mirrors GPUI's `app_window::Capabilities` for the rows Rune draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Capabilities {
    pub(super) os: Os,
    /// The voice command catalog and its opt-in (macOS).
    pub(super) commands: bool,
    /// Meeting recording, its pane, and its Settings section (macOS beta).
    pub(super) meetings: bool,
    /// Privacy permissions gate dictation (macOS).
    pub(super) permissions: bool,
    pub(super) launch_at_login: bool,
    pub(super) dock_icon: bool,
    /// The listener can be off: the status card and Enable on launch.
    pub(super) listener_control: bool,
    /// Paste with Ctrl-Shift-V for terminals.
    pub(super) terminal_paste: bool,
    /// The experimental Hex Microphone virtual driver (macOS).
    pub(super) hex_microphone: bool,
    /// The window sees dictation events, so the tour can end with a
    /// practice dictation into its own field.
    pub(super) practice_dictation: bool,
    /// Sparkle offers a manual "Check now".
    pub(super) manual_update_check: bool,
    /// The Fun pane: tuning and trying the Hex Microphone (macOS, while
    /// its experiment is on).
    pub(super) fun: bool,
    /// The HUD Lab developer pane drives the dictation indicator (macOS).
    pub(super) hud_lab: bool,
    /// Modifier-only shortcuts can require the left or right key (macOS).
    pub(super) modifier_sides: bool,
}

impl Capabilities {
    pub(super) const LINUX: Self = Self {
        os: Os::Linux,
        commands: false,
        meetings: false,
        permissions: false,
        launch_at_login: false,
        dock_icon: false,
        listener_control: true,
        terminal_paste: true,
        hex_microphone: false,
        practice_dictation: false,
        manual_update_check: false,
        fun: false,
        hud_lab: false,
        modifier_sides: false,
    };

    pub(super) const MACOS: Self = Self {
        os: Os::Macos,
        commands: true,
        meetings: true,
        permissions: true,
        launch_at_login: true,
        dock_icon: true,
        listener_control: false,
        terminal_paste: false,
        hex_microphone: true,
        practice_dictation: true,
        manual_update_check: true,
        fun: true,
        hud_lab: true,
        modifier_sides: true,
    };

    pub(super) const WINDOWS: Self = Self {
        os: Os::Windows,
        commands: false,
        meetings: false,
        permissions: false,
        launch_at_login: true,
        dock_icon: false,
        listener_control: true,
        terminal_paste: false,
        hex_microphone: false,
        practice_dictation: false,
        manual_update_check: true,
        fun: false,
        hud_lab: false,
        modifier_sides: false,
    };

    /// This OS's capabilities, with the rows its window host decides.
    pub(super) fn current() -> Self {
        let os = if cfg!(target_os = "macos") {
            Self::MACOS
        } else if cfg!(target_os = "windows") {
            Self::WINDOWS
        } else {
            Self::LINUX
        };
        Self {
            terminal_paste: super::host::TERMINAL_PASTE,
            manual_update_check: super::host::MANUAL_UPDATE_CHECK,
            ..os
        }
    }
}

/// The voice command model's state while commands are on (macOS).
// The macOS host reports the states fixtures do not show.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CommandModel {
    #[default]
    Disabled,
    Loading,
    Ready,
    Failed,
}

/// Whether Apple Intelligence can write meeting notes, as
/// `apple_intelligence::Availability` reports it.
// The macOS host reports the states fixtures do not show.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum NotesAvailability {
    #[default]
    Available,
    DeviceNotEligible,
    NotEnabled,
    ModelNotReady,
    UnsupportedSystem,
    Unknown,
}

impl NotesAvailability {
    pub(super) fn is_available(self) -> bool {
        self == Self::Available
    }
}

/// One command of the catalog: compiled commands and the user's TypeScript
/// commands, merged by the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CatalogCommand {
    pub(super) id: String,
    pub(super) task: String,
    pub(super) scope: CommandScope,
    pub(super) phrase: String,
    pub(super) aliases: Vec<String>,
    pub(super) description: String,
}

/// The personal commands workspace behind `hex.config.ts`.
// The macOS host reports the states fixtures do not show.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Workspace {
    #[default]
    Missing,
    Creating,
    Ready,
    Error,
}

/// The OS-only state the panes show; the host keeps it current.
#[derive(Clone, Debug)]
pub(super) struct PlatformState {
    pub(super) caps: Capabilities,
    /// Microphone, Input Monitoring, Accessibility, and whether the selected
    /// speech model is installed. Every permission is ready where the OS has
    /// none.
    pub(super) permissions: SetupStatus,
    /// Permissions whose Allow was pressed since they were last granted.
    pub(super) permissions_asked: Vec<PermissionKind>,
    /// Screen & System Audio Recording, for meetings.
    pub(super) system_audio: bool,
    /// `None` while the login item worker checks, or when it is unavailable.
    pub(super) login_item: Option<LoginItemStatus>,
    pub(super) login_item_checking: bool,
    pub(super) login_item_error: Option<String>,
    pub(super) hex_microphone: Option<HexMicrophoneStatus>,
    pub(super) command_model: CommandModel,
    pub(super) commands: Vec<CatalogCommand>,
    pub(super) workspace: Workspace,
    pub(super) workspace_path: String,
    pub(super) workspace_error: Option<String>,
    /// The frontmost application and browser host, for the command detail.
    pub(super) context: Option<String>,
    pub(super) notes: NotesAvailability,
    /// A meeting folder that could not be revealed or chosen.
    pub(super) meeting_folder_error: Option<String>,
    /// The tour's relaunch failed; the person must reopen Hex.
    pub(super) relaunch_failed: bool,
    /// Developer builds: the Activity and HUD Lab panes
    /// (`crate::DEVELOPER_FEATURES_ENABLED` on a native host).
    pub(super) developer: bool,
    /// macOS 27 lists Accessibility as "Device Control and Data Access"
    /// (`permission_guide::list_name`); the permission rows use that name.
    pub(super) accessibility_renamed: bool,
    /// The Hex Microphone's live filter tuning
    /// (`hex_microphone::current_tuned_config`).
    pub(super) tuning: TunedFilterConfig,
    /// The Fun pane's try-it demo (`hex_microphone::demo_snapshot`); `None`
    /// until the host has one. Refresh it every frame while it records or
    /// plays (`DemoSnapshot::is_recording`, `playing`).
    pub(super) demo: Option<DemoSnapshot>,
    /// What HUD Lab shows on the dictation indicator. The pane edits it and
    /// sends `PlatformRequest::ConfigureHud`; the host need not write it.
    pub(super) hud: HudLab,
    /// The dictation event log the Activity pane reads (`live.ndjson`);
    /// `None` where the host keeps none.
    pub(super) activity_log: Option<std::path::PathBuf>,
}

/// The dictation indicator's look, as `dictation_indicator::HudTuning`
/// (macOS-only) has it. All values are the indicator's own units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HudTuning {
    /// 0, 1, or 2: moving light, meridians, ribbons.
    pub(super) style: f32,
    pub(super) line_count: f32,
    pub(super) curvature: f32,
    pub(super) speed: f32,
    pub(super) sharpness: f32,
    pub(super) glow: f32,
    pub(super) depth: f32,
    pub(super) light_angle: f32,
    pub(super) outline: f32,
}

impl Default for HudTuning {
    /// Style B, as `HudTuning::default()`.
    fn default() -> Self {
        Self {
            style: 1.0,
            line_count: 2.0,
            curvature: 0.47,
            speed: 17.08,
            sharpness: 0.29,
            glow: 0.77,
            depth: 0.65,
            light_angle: 0.35,
            outline: 1.0,
        }
    }
}

/// The indicator state HUD Lab demonstrates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum HudDemo {
    #[default]
    Recording,
    Transcribing,
    Processing,
}

/// Everything HUD Lab drives: the tuning, the demonstrated state, and how
/// many dictations wait behind it (0 through 3).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct HudLab {
    pub(super) tuning: HudTuning,
    pub(super) demo: HudDemo,
    pub(super) queued: usize,
}

/// The Fun pane's demo controls (`hex_microphone`'s demo functions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DemoRequest {
    /// `start_demo_recording` (five seconds of the microphone).
    Record,
    StopRecording,
    SelectSource(DemoSourceKind),
    Play(DemoPlaybackTrack),
    StopPlayback,
}

impl PlatformState {
    pub(super) fn new(caps: Capabilities) -> Self {
        Self {
            caps,
            permissions: crate::onboarding::status_with_transcription_model(false),
            permissions_asked: Vec::new(),
            system_audio: false,
            login_item: None,
            login_item_checking: false,
            login_item_error: None,
            hex_microphone: None,
            command_model: CommandModel::Disabled,
            commands: Vec::new(),
            workspace: Workspace::Missing,
            workspace_path: String::new(),
            workspace_error: None,
            context: None,
            notes: NotesAvailability::Available,
            meeting_folder_error: None,
            relaunch_failed: false,
            developer: false,
            accessibility_renamed: false,
            tuning: TunedFilterConfig::default(),
            demo: None,
            hud: HudLab::default(),
            activity_log: None,
        }
    }

    /// The permission state with `transcription_model` from the caller,
    /// which knows the selected model.
    pub(super) fn setup_status(&self, transcription_model: bool) -> SetupStatus {
        if !self.caps.permissions {
            return crate::onboarding::status_with_transcription_model(transcription_model);
        }
        SetupStatus {
            transcription_model,
            ..self.permissions
        }
    }

    /// Adopts what the window host's login item reported: `None` where the
    /// host has no such row (it stays unavailable), an error beside the last
    /// known state.
    pub(super) fn set_login_item(&mut self, result: Option<Result<bool, String>>) {
        match result {
            None => {}
            Some(Ok(enabled)) => {
                self.login_item = Some(if enabled {
                    LoginItemStatus::Enabled
                } else {
                    LoginItemStatus::Disabled
                });
                self.login_item_error = None;
            }
            Some(Err(error)) => {
                self.login_item.get_or_insert(LoginItemStatus::Disabled);
                self.login_item_error = Some(error);
            }
        }
    }

    pub(super) fn asked(&self, kind: PermissionKind) -> bool {
        self.permissions_asked.contains(&kind)
    }

    pub(super) fn note_asked(&mut self, kind: PermissionKind) {
        if !self.asked(kind) {
            self.permissions_asked.push(kind);
        }
    }

    /// Deterministic macOS state for scenes: every permission granted, the
    /// login item off, the driver installed, a small command catalog.
    pub(super) fn preview(caps: Capabilities) -> Self {
        let mut state = Self::new(caps);
        let ready = PermissionState::Ready;
        state.permissions = SetupStatus {
            microphone: ready,
            input_monitoring: ready,
            accessibility: ready,
            transcription_model: true,
        };
        state.system_audio = true;
        state.login_item = caps.launch_at_login.then_some(LoginItemStatus::Disabled);
        if caps.hex_microphone {
            let version = env!("CARGO_PKG_VERSION").to_string();
            state.hex_microphone = Some(HexMicrophoneStatus {
                driver_bundle_installed: true,
                driver_loaded: true,
                installed_version: Some(version.clone()),
                bundled_version: version,
                sink_streaming: true,
                mic_in_use: true,
                active_apps: vec!["Slack".into()],
                action_state: crate::hex_microphone::DriverActionState::Idle,
            });
        }
        if caps.fun {
            state.demo = Some(super::fun::preview_demo(false, None));
        }
        if caps.commands {
            state.commands = preview_commands();
            state.workspace = Workspace::Ready;
            state.workspace_path = "~/Library/Application Support/hex/commands".into();
            state.context = Some("Brave Browser / github.com".into());
        }
        state
    }
}

fn preview_commands() -> Vec<CatalogCommand> {
    let command =
        |id: &str, task: &str, scope, phrase: &str, aliases: &[&str], description| CatalogCommand {
            id: id.into(),
            task: task.into(),
            scope,
            phrase: phrase.into(),
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            description: String::from(description),
        };
    vec![
        command(
            "mode.sleep",
            "Voice control",
            CommandScope::Global,
            "go to sleep",
            &["stop listening"],
            "Stop listening for commands",
        ),
        command(
            "mode.wake",
            "Voice control",
            CommandScope::Sleeping,
            "wake up",
            &[],
            "Listen for commands again",
        ),
        command(
            "dictation.paste-last",
            "Dictation",
            CommandScope::Global,
            "paste that again",
            &["paste last"],
            "Paste the last dictation",
        ),
        command(
            "open.slack",
            "Open applications",
            CommandScope::Global,
            "open slack",
            &["show slack"],
            "Open application",
        ),
        command(
            "browser.back",
            "Browser navigation",
            CommandScope::Application("Brave Browser".into()),
            "go back",
            &["back"],
            "Navigate active tab",
        ),
        command(
            "github.pulls",
            "Application navigation",
            CommandScope::Browser("github.com".into()),
            "show pull requests",
            &["pull requests", "open pulls"],
            "Open application destination",
        ),
    ]
}

/// A meeting action the pane asks the meetings runtime for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MeetingRequest {
    Start,
    Stop,
    Rename { id: String, title: String },
    Trash(String),
    Summarize(String),
    Reveal(String),
    RevealFolder,
    ChooseFolder,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PlatformRequest {
    /// Request the microphone or open the System Settings guide.
    Permission(PermissionAction),
    /// Screen & System Audio Recording, for meetings.
    SystemAudio,
    SetLaunchAtLogin(bool),
    OpenLoginItemSettings,
    InstallHexMicrophone,
    UninstallHexMicrophone,
    Meeting(MeetingRequest),
    CreateCommandsConfig,
    OpenCommandsConfig,
    RetryCommandModel,
    /// Quit and reopen Hex so macOS applies new permissions.
    Relaunch,
    /// Apply this tuning to the live Hex Microphone filter
    /// (`hex_microphone::update_tuned_config`). The pane has already saved
    /// the mode, comfort noise, and strength in `settings.json`.
    SetTuning(TunedFilterConfig),
    HexMicrophoneDemo(DemoRequest),
    /// Show this on the dictation indicator: `DictationIndicatorEvent::
    /// Configure` with the tuning, then `Reset` and the demonstrated state
    /// with `queued` jobs behind it (GPUI's `apply_hud_lab`).
    ConfigureHud(HudLab),
}

/// The host that performs [`PlatformRequest`]s and refreshes
/// [`PlatformState`] on its own OS.
// The macOS and Windows hosts implement it; Linux builds have none yet.
#[allow(dead_code)]
pub(super) trait NativeHost {
    fn send(&mut self, request: PlatformRequest) -> Result<(), String>;
    /// Copies fresh state in; true when anything shown changed.
    fn poll(&mut self, state: &mut PlatformState) -> bool;
}

pub(super) enum PlatformHost {
    /// Linux: the service owns everything, so nothing is asked here.
    None,
    /// Scenes and tests: requests are recorded, never performed.
    Fixture(Vec<PlatformRequest>),
    #[allow(dead_code)]
    Native(Box<dyn NativeHost>),
}

impl PlatformHost {
    pub(super) fn send(&mut self, request: PlatformRequest) -> Result<(), String> {
        match self {
            Self::None => Ok(()),
            Self::Fixture(sent) => {
                sent.push(request);
                Ok(())
            }
            Self::Native(host) => host.send(request),
        }
    }

    pub(super) fn poll(&mut self, state: &mut PlatformState) -> bool {
        match self {
            Self::Native(host) => host.poll(state),
            _ => false,
        }
    }

    pub(super) fn is_fixture(&self) -> bool {
        matches!(self, Self::Fixture(_))
    }
}
