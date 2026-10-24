//! `hex` control: the typed command/reply protocol an agent uses to drive the
//! one running Hex, the settings registry behind `hex settings`, and the CLI
//! client. Transports (the macOS local API's `POST /control`) only move
//! [`ControlRequest`]s and [`ControlResponse`]s; the app's host carries out
//! each command, and every rule shared between the window and the CLI lives
//! here or in the app's own setters.

pub mod cli;
pub mod settings;

use std::sync::mpsc::SyncSender;

use serde::{Deserialize, Serialize};

/// The wire version of [`ControlRequest`] and [`ControlResponse`]. A change
/// that breaks either shape increments it.
pub const CONTROL_VERSION: u32 = 1;

/// What a capture command does. Commands are idempotent: starting while
/// recording or stopping while idle changes nothing and succeeds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureAction {
    /// Start recording, locked as after a double tap: the shortcut or
    /// `stop` finishes it, Escape or `cancel` discards it.
    Start,
    /// Finish the recording and transcribe it.
    Stop,
    /// Discard the recording, or the newest unfinished dictation.
    Cancel,
    /// Start when idle, otherwise stop.
    Toggle,
}

/// The window's panes, by the names `hex window show` takes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Pane {
    Settings,
    /// Settings with the transcription model picker open.
    Models,
    #[value(alias = "replacements")]
    Modes,
    VoiceAction,
    History,
    Commands,
    Meetings,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum ControlCommand {
    Status,
    Quit {
        #[serde(default)]
        force: bool,
    },
    Restart {
        #[serde(default)]
        force: bool,
    },
    Dictation {
        action: CaptureAction,
    },
    VoiceAction {
        action: CaptureAction,
    },
    PasteLast,
    MeetingRecord {
        #[serde(default)]
        title: Option<String>,
    },
    MeetingStop,
    MeetingCopyLast,
    SettingsGet {
        #[serde(default)]
        key: Option<String>,
    },
    SettingsSet {
        key: String,
        value: String,
        #[serde(default)]
        confirm: bool,
    },
    CommandsRetry,
    ModelList,
    ModelUse {
        model: String,
        #[serde(default)]
        language: Option<String>,
    },
    ModelInstall {
        model: String,
        #[serde(default)]
        language: Option<String>,
    },
    ModelCancel,
    WindowShow {
        #[serde(default)]
        pane: Option<Pane>,
    },
    WindowHide,
    HistoryClear,
    CheckForUpdates,
    /// Report Launch at Login, or turn it on or off.
    LaunchAtLogin {
        #[serde(default)]
        enabled: Option<bool>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControlRequest {
    pub version: u32,
    #[serde(flatten)]
    pub command: ControlCommand,
}

impl ControlRequest {
    pub fn new(command: ControlCommand) -> Self {
        Self {
            version: CONTROL_VERSION,
            command,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ControlReply {
    /// The command ran. `changed` is false when it was already so.
    Done {
        message: String,
        changed: bool,
    },
    Status(Box<StatusReport>),
    Settings {
        settings: Vec<settings::SettingValue>,
    },
    Models {
        models: Vec<ModelInfo>,
    },
    Error {
        code: String,
        message: String,
    },
}

impl ControlReply {
    pub fn done(message: impl Into<String>) -> Self {
        Self::Done {
            message: message.into(),
            changed: true,
        }
    }

    pub fn unchanged(message: impl Into<String>) -> Self {
        Self::Done {
            message: message.into(),
            changed: false,
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<ControlError> for ControlReply {
    fn from(error: ControlError) -> Self {
        Self::error(error.code, error.message)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ControlResponse {
    pub version: u32,
    #[serde(flatten)]
    pub reply: ControlReply,
}

impl From<ControlReply> for ControlResponse {
    fn from(reply: ControlReply) -> Self {
        Self {
            version: CONTROL_VERSION,
            reply,
        }
    }
}

/// A refused or failed command, with a stable code (see `docs/cli.md`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlError {
    pub code: &'static str,
    pub message: String,
}

impl ControlError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// One request from a transport to the app's control host.
pub struct ControlCall {
    pub request: ControlRequest,
    pub reply: SyncSender<ControlResponse>,
}

/// Rejects a request from a `hex` built for another protocol version.
pub fn check_version(request: &ControlRequest) -> Result<(), ControlError> {
    if request.version == CONTROL_VERSION {
        return Ok(());
    }
    Err(version_mismatch(request.version.into()))
}

pub fn version_mismatch(version: u64) -> ControlError {
    ControlError::new(
        "version-mismatch",
        format!(
            "this hex speaks control version {version}, but the running Hex speaks \
             {CONTROL_VERSION}; use the hex inside the running app or restart Hex"
        ),
    )
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct StatusReport {
    pub app_version: String,
    pub pid: u32,
    /// The process's physical memory footprint, as Activity Monitor shows it.
    pub memory_footprint_bytes: Option<u64>,
    pub listener: ListenerReport,
    pub capture: CaptureReport,
    pub meeting: MeetingReport,
    pub model: ModelReport,
    pub features: FeatureReport,
    pub permissions: PermissionReport,
    pub window: WindowReport,
    /// current, checking, available, or unavailable.
    pub update: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ListenerReport {
    pub running: bool,
    pub status: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureState {
    #[default]
    Idle,
    Recording,
    Transcribing,
    /// The listener did not answer in time.
    Unknown,
}

/// What is recording.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureKind {
    Dictation,
    VoiceAction,
    /// A host-audio dictation through the local API (`POST /dictations`).
    Service,
    /// A voice-delimited dictation started by a spoken command.
    Voice,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CaptureReport {
    pub state: CaptureState,
    pub recording: Option<CaptureKind>,
    /// Accepted dictations still transcribing, processing, or pasting.
    pub pending_jobs: usize,
}

/// The listener's capture state, as it answers a status query.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CaptureSnapshot {
    pub recording: Option<CaptureKind>,
    pub pending_jobs: usize,
}

impl From<CaptureSnapshot> for CaptureReport {
    fn from(snapshot: CaptureSnapshot) -> Self {
        Self {
            state: if snapshot.recording.is_some() {
                CaptureState::Recording
            } else if snapshot.pending_jobs > 0 {
                CaptureState::Transcribing
            } else {
                CaptureState::Idle
            },
            recording: snapshot.recording,
            pending_jobs: snapshot.pending_jobs,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct MeetingReport {
    /// idle, starting, recording, or transcribing.
    pub state: String,
    pub title: Option<String>,
    pub elapsed_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ModelReport {
    pub id: String,
    pub name: String,
    pub language: String,
    pub installed: bool,
    /// The model being downloaded or loaded, if any.
    pub preparing: Option<String>,
    pub downloaded_bytes: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FeatureReport {
    pub commands: bool,
    /// disabled, loading, ready, or failed.
    pub command_model: String,
    pub voice_action: bool,
    pub meetings: bool,
    pub release_microphone_while_idle: bool,
    pub history_retention: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PermissionReport {
    /// Every permission granted and the selected model installed.
    pub ready: bool,
    /// ready, needs-request, needs-settings, or needs-restart.
    pub microphone: String,
    pub input_monitoring: String,
    pub accessibility: String,
    pub transcription_model: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WindowReport {
    pub open: bool,
    pub pane: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub languages: Vec<String>,
    pub size: String,
    pub installed: bool,
    pub selected: bool,
    pub streams: bool,
    pub supports_recognition_hints: bool,
}

/// Every model `hex model` can name, with what this Mac has installed.
pub fn model_infos(
    selection: &crate::transcription_models::TranscriptionSelection,
) -> Vec<ModelInfo> {
    crate::transcription_models::MODELS
        .iter()
        .filter(|model| model.available())
        .map(|model| ModelInfo {
            id: model.id.as_str().into(),
            name: model.name.into(),
            languages: model.languages.iter().map(|&code| code.into()).collect(),
            size: model.size_label(),
            installed: crate::transcription_models::is_installed(model, &selection.language),
            selected: model.id == selection.model,
            streams: model.streams,
            supports_recognition_hints: model.supports_recognition_hints,
        })
        .collect()
}

/// The selection `hex model use` or `install` asks for: a known, available
/// model, and a language it supports (the model's best match for the
/// current language when none is given).
pub fn model_selection(
    settings: &crate::app_settings::AppSettings,
    model: &str,
    language: Option<&str>,
) -> Result<crate::transcription_models::TranscriptionSelection, ControlError> {
    let id = model
        .parse::<crate::transcription_models::TranscriptionModelId>()
        .ok()
        .filter(|id| crate::transcription_models::definition(*id).available())
        .ok_or_else(|| {
            ControlError::new(
                "unknown-model",
                format!("unknown model `{model}`; run `hex model list` for the ids"),
            )
        })?;
    let mut selection = settings.transcription_for_model(id);
    if let Some(language) = language {
        selection.language = language.into();
    }
    let definition = crate::transcription_models::definition(id);
    if !definition.supports_language(&selection.language) {
        return Err(ControlError::new(
            "invalid-value",
            format!(
                "{} doesn't support language `{}`; it supports {}",
                definition.name,
                selection.language,
                definition.languages.join(", ")
            ),
        ));
    }
    if !definition.supports_recognition_hints {
        selection.recognition_hints.clear();
    }
    Ok(selection)
}

/// Refuses to stop Hex while it records, unless forced.
pub fn check_quit(
    meeting: &MeetingReport,
    capture: Option<CaptureSnapshot>,
    force: bool,
) -> Result<(), ControlError> {
    if force {
        return Ok(());
    }
    if meeting.state != "idle" {
        return Err(ControlError::new(
            "meeting-active",
            format!(
                "a meeting is {}; stop it first (`hex meetings stop`) or pass --force",
                meeting.state
            ),
        ));
    }
    if capture.is_some_and(|capture| capture.recording.is_some() || capture.pending_jobs > 0) {
        return Err(ControlError::new(
            "busy",
            "a dictation is recording or transcribing; wait for it or pass --force",
        ));
    }
    Ok(())
}

/// Process memory footprint on macOS; `None` where unmeasured.
pub fn memory_footprint() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: rusage_info_v4 is plain data that proc_pid_rusage fills.
        let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v4>() };
        // SAFETY: the pointer refers to a live, correctly sized buffer.
        let status = unsafe {
            libc::proc_pid_rusage(
                std::process::id() as i32,
                libc::RUSAGE_INFO_V4,
                (&mut info as *mut libc::rusage_info_v4).cast(),
            )
        };
        (status == 0).then_some(info.ri_phys_footprint)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_request(command: ControlCommand) {
        let request = ControlRequest::new(command);
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<ControlRequest>(&json).unwrap(),
            request
        );
    }

    #[test]
    fn every_command_round_trips() {
        for command in [
            ControlCommand::Status,
            ControlCommand::Quit { force: true },
            ControlCommand::Restart { force: false },
            ControlCommand::Dictation {
                action: CaptureAction::Toggle,
            },
            ControlCommand::VoiceAction {
                action: CaptureAction::Cancel,
            },
            ControlCommand::PasteLast,
            ControlCommand::MeetingRecord {
                title: Some("Design sync".into()),
            },
            ControlCommand::MeetingStop,
            ControlCommand::MeetingCopyLast,
            ControlCommand::SettingsGet { key: None },
            ControlCommand::SettingsSet {
                key: "commands_enabled".into(),
                value: "on".into(),
                confirm: true,
            },
            ControlCommand::CommandsRetry,
            ControlCommand::ModelList,
            ControlCommand::ModelUse {
                model: "parakeet_v3".into(),
                language: Some("pl".into()),
            },
            ControlCommand::ModelInstall {
                model: "parakeet_v2".into(),
                language: None,
            },
            ControlCommand::ModelCancel,
            ControlCommand::WindowShow {
                pane: Some(Pane::VoiceAction),
            },
            ControlCommand::WindowHide,
            ControlCommand::HistoryClear,
            ControlCommand::CheckForUpdates,
            ControlCommand::LaunchAtLogin {
                enabled: Some(true),
            },
        ] {
            round_trip_request(command);
        }
    }

    #[test]
    fn requests_are_flat_and_versioned() {
        let json = serde_json::to_value(ControlRequest::new(ControlCommand::MeetingRecord {
            title: Some("Standup".into()),
        }))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"version": 1, "command": "meeting-record", "title": "Standup"})
        );
        // Optional fields may be left out.
        let request: ControlRequest =
            serde_json::from_str(r#"{"version":1,"command":"quit"}"#).unwrap();
        assert_eq!(request.command, ControlCommand::Quit { force: false });
        assert!(serde_json::from_str::<ControlRequest>(r#"{"version":1,"command":"x"}"#).is_err());
    }

    #[test]
    fn every_reply_round_trips() {
        for reply in [
            ControlReply::done("Started dictation"),
            ControlReply::unchanged("Already recording"),
            ControlReply::Status(Box::default()),
            ControlReply::Settings {
                settings: vec![settings::SettingValue {
                    key: "sound_effects".into(),
                    value: serde_json::json!(true),
                    kind: "bool".into(),
                    choices: vec![],
                    writable: true,
                    about: "Play tones".into(),
                }],
            },
            ControlReply::Models { models: vec![] },
            ControlReply::error("busy", "Hex is busy"),
        ] {
            let response = ControlResponse::from(reply);
            let json = serde_json::to_string(&response).unwrap();
            assert_eq!(
                serde_json::from_str::<ControlResponse>(&json).unwrap(),
                response
            );
        }
        let json =
            serde_json::to_value(ControlResponse::from(ControlReply::error("busy", "x"))).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"version": 1, "status": "error", "code": "busy", "message": "x"})
        );
    }

    #[test]
    fn a_request_from_another_version_is_refused() {
        let mut request = ControlRequest::new(ControlCommand::Status);
        assert!(check_version(&request).is_ok());
        request.version = 99;
        assert_eq!(
            check_version(&request).unwrap_err().code,
            "version-mismatch"
        );
    }

    #[test]
    fn quitting_waits_for_recordings_unless_forced() {
        let idle = MeetingReport {
            state: "idle".into(),
            ..Default::default()
        };
        let recording = MeetingReport {
            state: "recording".into(),
            ..Default::default()
        };
        assert!(check_quit(&idle, None, false).is_ok());
        assert_eq!(
            check_quit(&recording, None, false).unwrap_err().code,
            "meeting-active"
        );
        assert!(check_quit(&recording, None, true).is_ok());
        let busy = CaptureSnapshot {
            recording: None,
            pending_jobs: 1,
        };
        assert_eq!(
            check_quit(&idle, Some(busy), false).unwrap_err().code,
            "busy"
        );
    }

    #[test]
    fn capture_reports_derive_their_state() {
        let report = CaptureReport::from(CaptureSnapshot {
            recording: Some(CaptureKind::Dictation),
            pending_jobs: 2,
        });
        assert_eq!(report.state, CaptureState::Recording);
        let report = CaptureReport::from(CaptureSnapshot {
            recording: None,
            pending_jobs: 2,
        });
        assert_eq!(report.state, CaptureState::Transcribing);
    }
}
