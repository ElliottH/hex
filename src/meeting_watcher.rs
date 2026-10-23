//! Meeting detection and recording control for the macOS app: watches which
//! applications use the microphone, offers a recording for supported calls,
//! runs the recorder, reports when the call app leaves, and starts notes. The
//! menu bar and the meeting capsules show its events.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, eyre};

use crate::context::ContextSnapshot;
use crate::meeting::{self, MeetingRequest};
use crate::meeting_detection::{
    CallPresence, Detection, MeetingCandidate, MeetingDetector, candidate_from_microphone,
};
use crate::microphone_activity::active_microphone_applications;

pub(crate) enum ControllerCommand {
    Record(MeetingCandidate),
    Dismiss,
    Stop,
    Open(String),
    Summarize(String),
}

pub(crate) enum ControllerEvent {
    Offer(MeetingCandidate),
    Starting,
    /// `automatic` when the provider is set to Always record.
    RecordingStarted {
        id: String,
        title: String,
        automatic: bool,
    },
    Retitled {
        title: String,
    },
    OpenMeeting(String),
    Transcribing,
    CallEnded {
        app: String,
    },
    Finished(Box<meeting::MeetingManifest>),
    Failed {
        title: String,
        error: String,
    },
    NotesStarted,
    NotesFinished(Box<meeting::MeetingManifest>),
}

struct ActiveRecording {
    title: String,
    application: Option<String>,
    app_label: String,
    automatic: bool,
    presence: CallPresence,
    stop: Arc<AtomicBool>,
    updates: Receiver<meeting::RecordingUpdate>,
    reported_started: bool,
    worker: JoinHandle<Result<meeting::MeetingManifest>>,
}

/// The meeting state mirrored into the menu bar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeetingMenuState {
    Idle,
    Starting,
    Recording { started: Instant, title: String },
    Transcribing,
}

pub struct ListenerConfig {
    pub project_root: PathBuf,
    pub event_path: PathBuf,
    pub device: Option<String>,
}

pub(crate) fn should_open_app_on_launch(
    show_dock_icon: bool,
    setup_ready: bool,
    onboarding_completed: bool,
    status_item_available: bool,
) -> bool {
    show_dock_icon || !setup_ready || !onboarding_completed || !status_item_available
}

pub(crate) fn controller_loop(
    shutdown: &AtomicBool,
    events: SyncSender<ControllerEvent>,
    commands: Receiver<ControllerCommand>,
    meeting_requests: Receiver<MeetingRequest>,
    project_root: PathBuf,
) -> Result<()> {
    let mut detector = MeetingDetector::default();
    let mut recording: Option<ActiveRecording> = None;
    let mut observed_microphone_apps = Vec::new();
    tracing::info!("meeting watcher started");

    while !shutdown.load(Ordering::Relaxed) {
        let enabled = crate::app_settings::meetings_enabled();
        let requested_commands = meeting_requests
            .try_iter()
            .take(8)
            .map(|request| match request {
                MeetingRequest::Start => ControllerCommand::Record(MeetingCandidate::manual()),
                MeetingRequest::Stop => ControllerCommand::Stop,
                MeetingRequest::Summarize(id) => ControllerCommand::Summarize(id),
            });
        for command in commands.try_iter().take(8).chain(requested_commands) {
            handle_command(&mut recording, command, enabled, &project_root, &events);
        }

        poll_recording(&mut recording, &events)?;

        if recording.is_none() && enabled {
            let choices = crate::app_settings::meeting_provider_choices();
            match detect_meeting(&mut detector, &mut observed_microphone_apps, &choices) {
                Err(()) => {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
                Ok(Some(Detection::Offer(candidate))) => {
                    let _ = events.send(ControllerEvent::Offer(candidate));
                }
                Ok(Some(Detection::Record(candidate))) => {
                    start_recording(&mut recording, candidate, true, &project_root, &events);
                }
                Ok(None) => {}
            }
        }
        thread::sleep(Duration::from_millis(500));
    }

    if let Some(active) = recording {
        active.stop.store(true, Ordering::Relaxed);
        active
            .worker
            .join()
            .map_err(|_| eyre!("meeting recorder panicked"))??;
    }
    Ok(())
}

fn handle_command(
    recording: &mut Option<ActiveRecording>,
    command: ControllerCommand,
    enabled: bool,
    project_root: &Path,
    events: &SyncSender<ControllerEvent>,
) {
    match command {
        ControllerCommand::Record(_) if !enabled => {
            tracing::warn!("ignored a meeting recording request while meetings are off");
        }
        ControllerCommand::Record(candidate) if recording.is_none() => {
            start_recording(recording, candidate, false, project_root, events);
        }
        ControllerCommand::Stop => {
            if let Some(active) = recording.as_ref()
                && !active.stop.swap(true, Ordering::Relaxed)
            {
                let _ = events.send(ControllerEvent::Transcribing);
            }
        }
        ControllerCommand::Open(id) => {
            let _ = events.send(ControllerEvent::OpenMeeting(id));
        }
        ControllerCommand::Summarize(id) => start_notes(id, events),
        ControllerCommand::Dismiss | ControllerCommand::Record(_) => {}
    }
}

fn start_recording(
    recording: &mut Option<ActiveRecording>,
    candidate: MeetingCandidate,
    automatic: bool,
    project_root: &Path,
    events: &SyncSender<ControllerEvent>,
) {
    if automatic {
        tracing::info!(
            source = candidate.source.label(),
            title = candidate.title,
            "meeting recording started automatically"
        );
    }
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let (update_sender, updates) = mpsc::sync_channel(4);
    let start = candidate.recording_start();
    let worker_project_root = project_root.to_path_buf();
    let worker = thread::spawn(move || {
        meeting::record(
            start,
            &worker_stop,
            &worker_project_root,
            Some(update_sender),
        )
    });
    let _ = events.send(ControllerEvent::Starting);
    *recording = Some(ActiveRecording {
        title: candidate.title,
        application: candidate.application,
        app_label: candidate.source.label().into(),
        automatic,
        presence: CallPresence::default(),
        stop,
        updates,
        reported_started: false,
        worker,
    });
}

fn poll_recording(
    recording: &mut Option<ActiveRecording>,
    events: &SyncSender<ControllerEvent>,
) -> Result<()> {
    if let Some(active) = recording.as_mut() {
        for update in active.updates.try_iter() {
            match update {
                meeting::RecordingUpdate::Started { id, title } => {
                    active.reported_started = true;
                    active.title = title.clone();
                    if !active.stop.load(Ordering::Relaxed) {
                        let _ = events.send(ControllerEvent::RecordingStarted {
                            id,
                            title,
                            automatic: active.automatic,
                        });
                    }
                }
                meeting::RecordingUpdate::Retitled { title } => {
                    active.title = title.clone();
                    let _ = events.send(ControllerEvent::Retitled { title });
                }
                meeting::RecordingUpdate::Transcribing => {
                    if !active.stop.swap(true, Ordering::Relaxed) {
                        let _ = events.send(ControllerEvent::Transcribing);
                    }
                }
            }
        }
    }

    if recording
        .as_ref()
        .is_some_and(|active| active.worker.is_finished())
    {
        let active = recording
            .take()
            .ok_or_else(|| eyre!("recording disappeared"))?;
        let event = match active.worker.join() {
            Ok(Ok(manifest)) => {
                if manifest.status == meeting::MeetingStatus::Complete && meeting::notes_wanted() {
                    start_notes(manifest.id.clone(), events);
                }
                ControllerEvent::Finished(Box::new(manifest))
            }
            Ok(Err(error)) => ControllerEvent::Failed {
                title: active.title,
                error: error.to_string(),
            },
            Err(_) => ControllerEvent::Failed {
                title: active.title,
                error: "meeting recorder panicked".into(),
            },
        };
        let _ = events.send(event);
    }

    if let Some(active) = recording.as_mut()
        && active.reported_started
        && !active.stop.load(Ordering::Relaxed)
        && let Some(application) = active.application.as_deref()
    {
        // A failed query must not end the controller while it owns a recording.
        let present = match active_microphone_applications() {
            Ok(applications) => applications
                .iter()
                .any(|candidate| candidate.canonical_bundle_id() == Some(application)),
            Err(error) => {
                tracing::warn!(%error, "could not inspect microphone activity during a meeting");
                true
            }
        };
        if active.presence.observe(present) {
            tracing::info!(application, "meeting application released the microphone");
            let _ = events.send(ControllerEvent::CallEnded {
                app: active.app_label.clone(),
            });
        }
    }
    Ok(())
}

/// One detection pass; `Err` when microphone activity could not be read.
fn detect_meeting(
    detector: &mut MeetingDetector,
    observed_microphone_apps: &mut Vec<(i32, String)>,
    choices: &[crate::app_settings::MeetingProviderChoice],
) -> Result<Option<Detection>, ()> {
    let microphone_apps = match active_microphone_applications() {
        Ok(applications) => applications,
        Err(error) => {
            tracing::warn!(%error, "could not inspect microphone activity");
            return Err(());
        }
    };
    let observed = microphone_apps
        .iter()
        .map(|application| (application.pid, application.bundle_id.clone()))
        .collect::<Vec<_>>();
    if observed != *observed_microphone_apps {
        let applications = microphone_apps
            .iter()
            .map(|application| {
                format!(
                    "{} ({}, pid {})",
                    application.name, application.bundle_id, application.pid
                )
            })
            .collect::<Vec<_>>();
        tracing::info!(?applications, "microphone application activity changed");
        *observed_microphone_apps = observed;
    }
    let application = microphone_apps
        .into_iter()
        .filter(|application| application.is_supported_meeting_app())
        .min_by_key(|application| application.detection_priority());
    let context = match application.as_ref().and_then(|application| {
        application
            .is_browser()
            .then(|| application.canonical_bundle_id())
            .flatten()
            .filter(|bundle_id| !detector.is_application_snoozed(bundle_id))
    }) {
        Some(bundle_id) => ContextSnapshot::capture_for_meeting(bundle_id).unwrap_or_default(),
        None => ContextSnapshot::default(),
    };
    let candidate =
        application.and_then(|application| candidate_from_microphone(&application, &context));
    let detection = detector.detect(candidate, choices);
    if let Some(Detection::Offer(candidate)) = &detection {
        tracing::info!(
            source = candidate.source.label(),
            title = candidate.title,
            "meeting recording offer triggered"
        );
    }
    Ok(detection)
}

/// Writes notes on a background thread; the UI refreshes from the manifest
/// whether generation succeeds or fails.
fn start_notes(id: String, events: &SyncSender<ControllerEvent>) {
    let finished = events.clone();
    if meeting::spawn_summary(id, move |result| {
        if let Ok(manifest) = result {
            let _ = finished.send(ControllerEvent::NotesFinished(Box::new(manifest)));
        }
    }) {
        let _ = events.send(ControllerEvent::NotesStarted);
    }
}

#[cfg(debug_assertions)]
pub fn probe() -> Result<()> {
    for application in active_microphone_applications()? {
        println!(
            "{}\t{}\t{}\t{}",
            application.pid,
            if application.is_supported_meeting_app() {
                "meeting"
            } else {
                "other"
            },
            application.bundle_id,
            application.name
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dockless_startup_stays_quiet_only_when_setup_and_menu_bar_are_ready() {
        for setup_ready in [false, true] {
            for onboarding_completed in [false, true] {
                for status_item_available in [false, true] {
                    assert_eq!(
                        should_open_app_on_launch(
                            false,
                            setup_ready,
                            onboarding_completed,
                            status_item_available,
                        ),
                        !(setup_ready && onboarding_completed && status_item_available),
                        "setup_ready={setup_ready}, onboarding_completed={onboarding_completed}, status_item_available={status_item_available}",
                    );
                }
            }
        }
    }

    #[test]
    fn dock_visible_startup_always_opens_the_app() {
        for setup_ready in [false, true] {
            for onboarding_completed in [false, true] {
                for status_item_available in [false, true] {
                    assert!(should_open_app_on_launch(
                        true,
                        setup_ready,
                        onboarding_completed,
                        status_item_available,
                    ));
                }
            }
        }
    }
}
