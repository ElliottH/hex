//! Meeting detection and recording control for the macOS app: watches which
//! applications use the microphone, offers a recording for supported calls,
//! runs the recorder, reports when the call app leaves, and starts notes. The
//! menu bar and the meeting capsules show its events.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, eyre};

use crate::calendar::{CalendarMeeting, ReminderScheduler, ReminderUpdate};
use crate::context::ContextSnapshot;
use crate::meeting::{self, MeetingCalendar, MeetingRequest, MeetingStart};
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
    /// Record a calendar meeting from its reminder, first opening its call
    /// link when `join`.
    RecordMeeting {
        meeting: CalendarMeeting,
        join: bool,
    },
    /// Open the meeting's call link without recording (Always record
    /// starts once the call is detected).
    JoinMeeting(CalendarMeeting),
    /// Not now, or Don't record: the meeting's call is neither offered nor
    /// recorded automatically.
    DismissReminder(CalendarMeeting),
}

pub(crate) enum ControllerEvent {
    Offer(MeetingCandidate),
    /// A calendar meeting is about to start. `automatic` when its call
    /// service is set to Always record.
    Reminder {
        meeting: CalendarMeeting,
        automatic: bool,
    },
    /// Close this occurrence's reminder: it was cancelled, moved, is long
    /// past, or reminders were turned off.
    ReminderWithdrawn {
        occurrence: String,
    },
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
) -> Result<()> {
    let mut detector = MeetingDetector::default();
    let mut recording: Option<ActiveRecording> = None;
    let mut reminders = Reminders::default();
    let mut observed_microphone_apps = Vec::new();
    tracing::info!("meeting watcher started");

    while !shutdown.load(Ordering::Relaxed) {
        let enabled = crate::app_settings::meetings_enabled();
        let requested_commands = meeting_requests
            .try_iter()
            .take(8)
            .map(|request| match request {
                MeetingRequest::Start { title } => {
                    ControllerCommand::Record(MeetingCandidate::manual_titled(title))
                }
                MeetingRequest::Stop => ControllerCommand::Stop,
                MeetingRequest::Summarize(id) => ControllerCommand::Summarize(id),
            });
        for command in commands.try_iter().take(8).chain(requested_commands) {
            handle_command(
                &mut recording,
                &mut detector,
                &mut reminders,
                command,
                enabled,
                &events,
            );
        }

        poll_recording(&mut recording, &events)?;
        reminders.poll(enabled, recording.is_some(), &events);

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
                    let start = reminders.start_for(&candidate);
                    start_recording(&mut recording, &mut reminders, start, true, &events);
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
    detector: &mut MeetingDetector,
    reminders: &mut Reminders,
    command: ControllerCommand,
    enabled: bool,
    events: &SyncSender<ControllerEvent>,
) {
    match command {
        ControllerCommand::Record(_) | ControllerCommand::RecordMeeting { .. } if !enabled => {
            tracing::warn!("ignored a meeting recording request while meetings are off");
        }
        ControllerCommand::Record(candidate) if recording.is_none() => {
            let start = reminders.start_for(&candidate);
            start_recording(recording, reminders, start, false, events);
        }
        ControllerCommand::RecordMeeting { meeting, join } => {
            reminders.scheduler.answered(&meeting.occurrence());
            if join && let Some(link) = &meeting.link {
                crate::calendar::open_link(&link.url);
            }
            snooze_calendar_call(detector, &meeting);
            if recording.is_some() {
                tracing::warn!("ignored a calendar recording request while recording");
                return;
            }
            tracing::info!(
                provider = meeting.link.as_ref().map(|link| link.provider.label()),
                join,
                "calendar meeting recording started from its reminder"
            );
            start_recording(
                recording,
                reminders,
                calendar_start(&meeting),
                false,
                events,
            );
        }
        ControllerCommand::JoinMeeting(meeting) => {
            reminders.scheduler.answered(&meeting.occurrence());
            if let Some(link) = &meeting.link {
                crate::calendar::open_link(&link.url);
            }
        }
        ControllerCommand::DismissReminder(meeting) => {
            reminders.scheduler.answered(&meeting.occurrence());
            snooze_calendar_call(detector, &meeting);
            tracing::info!("calendar meeting reminder dismissed");
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

/// What a recording starts with, and what the controller watches while it
/// runs.
struct Start {
    meeting: MeetingStart,
    title: String,
    /// The call app whose leaving the microphone ends the call.
    application: Option<String>,
    app_label: String,
}

impl Start {
    fn from_candidate(candidate: &MeetingCandidate) -> Self {
        Self {
            meeting: candidate.recording_start(),
            title: candidate.title.clone(),
            application: candidate.application.clone(),
            app_label: candidate.source.label().into(),
        }
    }
}

/// A recording for a calendar meeting: its title, attendees, and call app.
fn calendar_start(meeting: &CalendarMeeting) -> Start {
    let provider = meeting.link.as_ref().map(|link| link.provider);
    Start {
        meeting: MeetingStart {
            title: Some(meeting.display_title()),
            app: provider.map(|provider| provider.label().to_string()),
            calendar: Some(calendar_context(meeting)),
        },
        title: meeting.display_title(),
        application: provider
            .and_then(|provider| provider.application())
            .map(str::to_string),
        app_label: provider
            .map_or("Meeting", |provider| provider.label())
            .into(),
    }
}

fn calendar_context(meeting: &CalendarMeeting) -> MeetingCalendar {
    MeetingCalendar {
        title: meeting.display_title(),
        attendees: meeting.attendees.clone(),
    }
}

/// What a calendar meeting's answered reminder covers: its call service's
/// detected-call keys until the meeting ends. Without a call link, the next
/// call of any service, but only around the scheduled start, so one
/// dismissal never silences other calls for the rest of a long event.
fn calendar_snooze(meeting: &CalendarMeeting) -> (Vec<String>, i64) {
    match &meeting.link {
        Some(link) => (
            link.provider
                .detection_keys()
                .iter()
                .map(|key| key.to_string())
                .collect(),
            meeting.end_ms,
        ),
        None => (
            Vec::new(),
            (meeting.start_ms + crate::calendar::LATE_GRACE_MS).min(meeting.end_ms),
        ),
    }
}

fn snooze_calendar_call(detector: &mut MeetingDetector, meeting: &CalendarMeeting) {
    let (keys, until_ms) = calendar_snooze(meeting);
    detector.snooze_calendar_call(keys, until_ms);
}

/// How often the reminder poll rereads the calendar without a change
/// notification.
const CALENDAR_REREAD_MS: i64 = 60_000;

/// Calendar reminders on the controller thread: a bounded EventKit read on
/// each store change or once a minute, and a cheap scheduler pass on every
/// loop. No network; never on the audio or UI thread.
#[derive(Default)]
struct Reminders {
    scheduler: ReminderScheduler,
    meetings: Vec<CalendarMeeting>,
    change_count: Option<u64>,
    /// Wall-clock time of the last read, so a wake from sleep rereads.
    read_at_ms: Option<i64>,
    read_failed: bool,
}

impl Reminders {
    fn poll(&mut self, enabled: bool, recording: bool, events: &SyncSender<ControllerEvent>) {
        let settings = crate::app_settings::calendar_settings();
        if !enabled || !settings.reminders_active() {
            self.meetings.clear();
            self.change_count = None;
            self.read_at_ms = None;
            if let Some(occurrence) = self.scheduler.clear() {
                let _ = events.send(ControllerEvent::ReminderWithdrawn { occurrence });
            }
            return;
        }
        let now_ms = crate::calendar::wall_clock_ms();
        let changes = crate::calendar::change_count();
        let stale = self
            .read_at_ms
            .is_none_or(|read_at| (now_ms - read_at).abs() >= CALENDAR_REREAD_MS);
        if stale || self.change_count != Some(changes) {
            self.change_count = Some(changes);
            self.read_at_ms = Some(now_ms);
            match crate::calendar::read_meetings(&settings, now_ms) {
                Ok(meetings) => {
                    self.meetings = meetings;
                    self.read_failed = false;
                }
                Err(error) => {
                    if !self.read_failed {
                        tracing::warn!(%error, "could not read calendar meetings");
                    }
                    self.read_failed = true;
                    self.meetings.clear();
                }
            }
        }
        let lead_ms = settings.lead().as_millis() as i64;
        match self.step(recording, now_ms, lead_ms) {
            Some(ReminderUpdate::Show(meeting)) => {
                let automatic = records_automatically(
                    &meeting,
                    &crate::app_settings::meeting_provider_choices(),
                );
                tracing::info!(
                    provider = meeting.link.as_ref().map(|link| link.provider.label()),
                    automatic,
                    "calendar meeting reminder shown"
                );
                let _ = events.send(ControllerEvent::Reminder { meeting, automatic });
            }
            Some(ReminderUpdate::Withdraw(occurrence)) => {
                let _ = events.send(ControllerEvent::ReminderWithdrawn { occurrence });
            }
            None => {}
        }
    }

    /// The scheduler pass over the meetings last read. A reminder waits
    /// while Hex records; it shows after, if still due.
    fn step(&mut self, recording: bool, now_ms: i64, lead_ms: i64) -> Option<ReminderUpdate> {
        if recording {
            return None;
        }
        self.scheduler.poll(&self.meetings, now_ms, lead_ms)
    }

    /// A detected call's recording, titled from the calendar meeting it
    /// belongs to when there is one.
    fn start_for(&self, candidate: &MeetingCandidate) -> Start {
        let mut start = Start::from_candidate(candidate);
        let keys = [candidate.site.as_deref(), candidate.application.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let lead_ms = crate::app_settings::calendar_settings().lead().as_millis() as i64;
        if let Some(meeting) = crate::calendar::current_for_call(
            &self.meetings,
            &keys,
            crate::calendar::wall_clock_ms(),
            lead_ms,
        ) {
            start.meeting.calendar = Some(calendar_context(meeting));
            // A title the user gave (`hex meetings record --title`) wins.
            if !(candidate.named
                && candidate.source == crate::meeting_detection::MeetingSource::Manual)
            {
                start.meeting.title = Some(meeting.display_title());
                start.title = meeting.display_title();
            }
        }
        start
    }
}

/// Whether the meeting's call service is set to Always record, so its call
/// records once detected.
fn records_automatically(
    meeting: &CalendarMeeting,
    choices: &[crate::app_settings::MeetingProviderChoice],
) -> bool {
    meeting
        .link
        .as_ref()
        .and_then(|link| link.provider.choice_key())
        .is_some_and(|key| {
            choices.iter().any(|choice| {
                choice.key == key
                    && choice.policy == crate::app_settings::MeetingPolicy::AlwaysRecord
            })
        })
}

fn start_recording(
    recording: &mut Option<ActiveRecording>,
    reminders: &mut Reminders,
    start: Start,
    automatic: bool,
    events: &SyncSender<ControllerEvent>,
) {
    // `Starting` closes a showing reminder; no reminder shows over a
    // recording.
    reminders.scheduler.clear();
    if automatic {
        tracing::info!(
            source = start.app_label,
            title = start.title,
            "meeting recording started automatically"
        );
    }
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let (update_sender, updates) = mpsc::sync_channel(4);
    let meeting_start = start.meeting;
    let worker =
        thread::spawn(move || meeting::record(meeting_start, &worker_stop, Some(update_sender)));
    let _ = events.send(ControllerEvent::Starting);
    *recording = Some(ActiveRecording {
        title: start.title,
        application: start.application,
        app_label: start.app_label,
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

    const MINUTE: i64 = 60_000;

    fn meeting(id: &str, start_minute: i64, minutes: i64) -> CalendarMeeting {
        CalendarMeeting {
            id: id.into(),
            title: id.into(),
            start_ms: start_minute * MINUTE,
            end_ms: (start_minute + minutes) * MINUTE,
            attendees: vec!["Ana Ruiz".into()],
            link: None,
        }
    }

    #[test]
    fn no_reminder_shows_over_a_recording() {
        let mut reminders = Reminders {
            meetings: vec![meeting("sync", 100, 30)],
            ..Reminders::default()
        };
        assert_eq!(reminders.step(true, 99 * MINUTE, MINUTE), None);
        // After the recording, it shows if still due.
        assert!(matches!(
            reminders.step(false, 100 * MINUTE, MINUTE),
            Some(ReminderUpdate::Show(_))
        ));

        // A recording that starts while one shows closes it for good.
        let mut reminders = Reminders {
            meetings: vec![meeting("sync", 100, 30)],
            ..Reminders::default()
        };
        assert!(reminders.step(false, 99 * MINUTE, MINUTE).is_some());
        reminders.scheduler.clear();
        assert_eq!(reminders.step(true, 99 * MINUTE + 1_000, MINUTE), None);
        assert_eq!(reminders.step(false, 102 * MINUTE, MINUTE), None);
    }

    #[test]
    fn a_dismissed_reminder_without_a_link_snoozes_only_around_its_start() {
        let all_day_workshop = meeting("workshop", 100, 240);
        assert_eq!(
            calendar_snooze(&all_day_workshop),
            (Vec::new(), 100 * MINUTE + crate::calendar::LATE_GRACE_MS)
        );
        let short = meeting("standup", 100, 3);
        assert_eq!(calendar_snooze(&short), (Vec::new(), 103 * MINUTE));

        let mut zoom = meeting("zoom", 100, 240);
        zoom.link = Some(crate::calendar::CallLink {
            provider: crate::calendar::CallProvider::Zoom,
            url: "https://zoom.us/j/1".into(),
        });
        let (keys, until_ms) = calendar_snooze(&zoom);
        assert!(keys.iter().any(|key| key == "us.zoom.xos"), "{keys:?}");
        assert_eq!(until_ms, zoom.end_ms);
    }
}
