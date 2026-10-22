//! The one Linux Hex process: the dictation host (listener, shortcut
//! registration, model preparation, updates, History writes, paste), the
//! single-instance socket, the GTK HUD thread, and the window, opened on
//! demand and closed without stopping anything.
//!
//! Threads: the main thread runs the window toolkit; the host thread
//! refreshes [`LinuxDesktopHost`] every 20 ms and publishes its status;
//! the socket's workers answer `hex` commands from that published status
//! and hand shortcut commands straight to the listener.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use color_eyre::Result;

use crate::app_settings::{AppSettings, HotkeyBinding};
use crate::desktop_activity::DesktopActivity;
use crate::desktop_host::{
    DesktopAction, DesktopListenerSnapshot, DesktopShortcut, DesktopSnapshot,
    DesktopTranscriptionSnapshot, DesktopUpdateStatus,
};
use crate::events::EventReader;
use crate::linux_input::{LinuxHotkey, ShortcutCommand};
use crate::linux_service::{
    self, Activation, Control, Published, ShortcutRoute, ShortcutTarget, Status,
};
use crate::linux_updater::InstalledUpdate;

const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const REFRESH_INTERVAL: Duration = Duration::from_millis(20);
/// Left by "Update" so the restarted process shows the window again; a
/// crash restart finds none and stays in the background.
const REOPEN_MARKER: &str = "reopen-window";

type ListenerResult = std::result::Result<(), String>;
type PreparationResult = std::result::Result<PreparedTranscription, String>;
pub(crate) type Waker = Arc<dyn Fn() + Send + Sync>;

/// The window toolkit the process hosts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowUi {
    #[cfg_attr(not(feature = "rune-ui"), allow(dead_code))]
    Rune,
    Gpui,
}

impl WindowUi {
    pub(crate) fn default_for_build() -> Self {
        if cfg!(feature = "rune-ui") {
            Self::Rune
        } else {
            Self::Gpui
        }
    }
}

struct PreparedTranscription {
    selection: crate::transcription_models::TranscriptionSelection,
    transcriber: crate::local_transcriber::LocalTranscriber,
}

struct TranscriptionPreparation {
    canceled: Arc<AtomicBool>,
    model: crate::transcription_models::TranscriptionModelId,
    progress: Arc<AtomicU64>,
    stage: Arc<AtomicU8>,
    worker: JoinHandle<PreparationResult>,
}

#[derive(Clone, PartialEq)]
enum SettingsChange {
    Capture(ShortcutTarget),
    Hotkey(ShortcutTarget, LinuxHotkey),
    /// Settings the window already saved.
    Reload(Box<AppSettings>),
}

struct SettingsEdit {
    change: SettingsChange,
    resume: bool,
    canceled: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<AppSettings>>>,
}

struct Listener {
    stop: Arc<AtomicBool>,
    worker: JoinHandle<ListenerResult>,
    startup: ListenerStartup,
    hotkey_status: crate::linux_input::HotkeyStatus,
    /// `hex` commands for the listener's shortcut state machine.
    commands: mpsc::SyncSender<ShortcutCommand>,
}

enum ListenerStartup {
    Awaiting { previous_session: Option<u64> },
    Observed,
}

impl Listener {
    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn finish(self) -> ListenerResult {
        self.worker
            .join()
            .unwrap_or_else(|_| Err("listener worker stopped unexpectedly".into()))
    }

    fn observe_session(&mut self, session: Option<u64>) {
        if let ListenerStartup::Awaiting { previous_session } = self.startup
            && session != previous_session
        {
            self.startup = ListenerStartup::Observed;
        }
    }
}

struct LinuxDesktopHost {
    event_path: PathBuf,
    event_reader: EventReader,
    activity: DesktopActivity,
    listener: Option<Listener>,
    listen_when_ready: bool,
    status: String,
    error: Option<String>,
    dismissed_failure_at: Option<u64>,
    settings_error: Option<String>,
    settings: AppSettings,
    // Dismissing a load error must not authorize recording with fallback defaults.
    settings_valid: bool,
    settings_edit: Option<SettingsEdit>,
    prepared_transcriber: Option<crate::local_transcriber::LocalTranscriber>,
    transcription_preparation: Option<TranscriptionPreparation>,
    transcription_error: Option<String>,
    update: UpdateState,
    output: crate::linux_dictation::DictationOutput,
    /// Hosts the personal transformations workspace while a mode uses one.
    personal_commands: Option<crate::personal_commands::PersonalCommands>,
    personal_commands_checked_at: Instant,
}

enum UpdateState {
    Unmanaged,
    Checking(Receiver<Result<Option<InstalledUpdate>, String>>),
    Failed(Instant),
    Waiting(Instant),
    Ready(InstalledUpdate),
}

/// What the window shows of the host.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HostSnapshot {
    pub(crate) desktop: DesktopSnapshot,
    pub(crate) settings: AppSettings,
    /// The host is recording or saving a shortcut.
    pub(crate) editing: bool,
}

/// What the window asks of the host. Settings edits go through
/// [`HostHandle::save_settings`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HostRequest {
    Desktop(DesktopAction),
    /// Stops listening while the window records a shortcut.
    CaptureShortcutFor(ShortcutTarget),
    CancelShortcut,
    /// The chord the window recorded, for the host to register and save.
    SetShortcut {
        target: ShortcutTarget,
        shortcut: DesktopShortcut,
    },
    ChooseModel {
        model: crate::transcription_models::TranscriptionModelId,
        language: String,
    },
    CancelModel,
}

struct Shared {
    host: Mutex<LinuxDesktopHost>,
    waker: Mutex<Option<Waker>>,
    window_open: AtomicBool,
    shutdown: &'static AtomicBool,
}

/// The window's handle to the host it runs beside. Every call is a short
/// lock on the host; nothing here waits for the listener or a model.
#[derive(Clone)]
pub(crate) struct HostHandle(Arc<Shared>);

impl HostHandle {
    fn new(host: LinuxDesktopHost, shutdown: &'static AtomicBool) -> Self {
        Self(Arc::new(Shared {
            host: Mutex::new(host),
            waker: Mutex::new(None),
            window_open: AtomicBool::new(false),
            shutdown,
        }))
    }

    /// The window's "Quit Hex": drains accepted work, then exits.
    pub(crate) fn quit(&self) {
        self.0.shutdown.store(true, Ordering::Release);
    }

    fn lock(&self) -> MutexGuard<'_, LinuxDesktopHost> {
        self.0
            .host
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn snapshot(&self) -> HostSnapshot {
        let host = self.lock();
        HostSnapshot {
            desktop: host.snapshot(),
            settings: host.settings.clone(),
            editing: host.settings_edit.is_some(),
        }
    }

    pub(crate) fn request(&self, request: HostRequest) -> Result<()> {
        let result = {
            let mut host = self.lock();
            match request {
                HostRequest::Desktop(action) => host.dispatch(action),
                HostRequest::CaptureShortcutFor(target) => {
                    host.begin_settings_edit(SettingsChange::Capture(target));
                    Ok(())
                }
                HostRequest::CancelShortcut => {
                    host.cancel_settings_edit();
                    Ok(())
                }
                HostRequest::SetShortcut { target, shortcut } => host.set_hotkey(target, shortcut),
                HostRequest::ChooseModel { model, language } => {
                    host.choose_transcription(model, language)
                }
                HostRequest::CancelModel => {
                    host.cancel_transcription_preparation();
                    Ok(())
                }
            }
        };
        self.wake();
        result
    }

    /// Saves the window's settings, then adopts them. A failed save leaves
    /// the host's settings alone.
    #[cfg_attr(not(feature = "rune-ui"), allow(dead_code))]
    pub(crate) fn save_settings(&self, settings: AppSettings) -> Result<()> {
        settings.save()?;
        self.lock().adopt_settings(settings);
        self.wake();
        Ok(())
    }

    /// Adopts settings the window already saved.
    pub(crate) fn adopt_settings(&self, settings: AppSettings) {
        self.lock().adopt_settings(settings);
        self.wake();
    }

    pub(crate) fn event_path(&self) -> PathBuf {
        self.lock().event_path.clone()
    }

    /// The host's retained-history store; one store per process.
    pub(crate) fn history(&self) -> Option<crate::history::History> {
        self.lock().output.history.clone()
    }

    /// The window's toolkit wakes on host changes through `waker`.
    #[cfg_attr(not(feature = "rune-ui"), allow(dead_code))]
    pub(crate) fn set_waker(&self, waker: Waker) {
        *self.0.waker.lock().unwrap() = Some(waker);
    }

    pub(crate) fn set_window_open(&self, open: bool) {
        self.0.window_open.store(open, Ordering::Release);
    }

    #[cfg_attr(not(feature = "rune-ui"), allow(dead_code))]
    pub(crate) fn window_open(&self) -> bool {
        self.0.window_open.load(Ordering::Acquire)
    }

    /// A closed window leaves no shortcut capture behind.
    pub(crate) fn window_closed(&self) {
        self.set_window_open(false);
        let mut host = self.lock();
        if host.capturing_hotkey() {
            host.cancel_settings_edit();
        }
    }

    fn wake(&self) {
        let waker = self.0.waker.lock().unwrap().clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    fn status(&self) -> Status {
        let snapshot = self.snapshot();
        Status {
            desktop: snapshot.desktop,
            settings: snapshot.settings,
            editing: snapshot.editing,
            hud_limitation: crate::linux_desktop::hud_limitation(),
            window_open: self.0.window_open.load(Ordering::Acquire),
            pid: std::process::id(),
            session: linux_service::Session::current(),
        }
    }
}

/// Becomes the Linux instance: claims the data root, starts the host, and
/// runs the window toolkit on this (the main) thread until `hex quit`, a
/// signal, or the window's Quit. `activation` opens the window at once.
pub fn run(
    ui: WindowUi,
    event_path: PathBuf,
    shutdown: &'static AtomicBool,
    activation: Option<Activation>,
) -> Result<()> {
    let mut server = linux_service::Server::bind()?;
    let (settings, error) = match AppSettings::load() {
        Ok(settings) => (settings, None),
        Err(error) => (
            AppSettings::default(),
            Some(format!("Could not load settings: {error:#}")),
        ),
    };
    crate::linux_dictation::initialize_feedback(&settings);
    let update = if crate::linux_updater::managed_install() {
        UpdateState::Checking(start_update_check())
    } else {
        UpdateState::Unmanaged
    };
    let enable_on_launch = settings.enable_on_launch;
    let mut host = LinuxDesktopHost::new(event_path, settings, error, update);
    if enable_on_launch {
        host.start();
    }
    let host = HostHandle::new(host, shutdown);
    let published = Arc::new(Published::default());
    *published.status.lock().unwrap() = Some(host.status());
    let (control, requests) = mpsc::sync_channel(8);
    server.serve(published.clone(), control.clone(), shutdown);
    linux_service::notify("READY=1");
    tracing::info!(pid = std::process::id(), "Hex instance ready");

    let worker = {
        let host = host.clone();
        std::thread::Builder::new()
            .name("hex-host".into())
            .spawn(move || run_host(host, published, server, control, shutdown))?
    };
    let reopen = take_reopen_marker();
    let first = activation.or(reopen.then(Activation::default));
    run_windows(ui, &host, requests, first, shutdown);
    shutdown.store(true, Ordering::Release);
    let _ = worker.join();
    Ok(())
}

/// The host thread: refreshes the host, publishes what the socket answers
/// with, wakes the window on changes, and drains accepted work on quit.
fn run_host(
    host: HostHandle,
    published: Arc<Published>,
    server: linux_service::Server,
    control: mpsc::SyncSender<Control>,
    shutdown: &'static AtomicBool,
) {
    let mut stopping = false;
    let mut shown: Option<HostSnapshot> = None;
    loop {
        let (done, route) = {
            let mut guard = host.lock();
            if shutdown.load(Ordering::Acquire) && !stopping {
                stopping = true;
                linux_service::notify("STOPPING=1");
                guard.stop();
                guard.cancel_transcription_preparation();
                // The main thread may be waiting for the next window request.
                let _ = control.try_send(Control::Quit);
            }
            guard.refresh();
            (
                stopping && !guard.has_pending_work(),
                guard.shortcut_route(),
            )
        };
        *published.shortcuts.lock().unwrap() = route;
        let status = host.status();
        let snapshot = HostSnapshot {
            desktop: status.desktop.clone(),
            settings: status.settings.clone(),
            editing: status.editing,
        };
        *published.status.lock().unwrap() = Some(status);
        if shown.as_ref() != Some(&snapshot) {
            shown = Some(snapshot);
            host.wake();
        }
        if done {
            break;
        }
        std::thread::sleep(REFRESH_INTERVAL);
    }
    *published.shortcuts.lock().unwrap() = None;
    drop(server);
}

/// The one seam between the process and its window toolkit, on the main
/// thread until Hex quits. `first` opens the window at once.
fn run_windows(
    ui: WindowUi,
    host: &HostHandle,
    requests: Receiver<Control>,
    first: Option<Activation>,
    shutdown: &'static AtomicBool,
) {
    match ui {
        #[cfg(feature = "rune-ui")]
        WindowUi::Rune => {
            let host = host.clone();
            // A missing display or GPU fails the window, never Hex.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::rune_ui::run_app(host, requests, first)
            }))
            .is_err()
            {
                tracing::error!("the Hex window failed; Hex keeps running without it");
                while !shutdown.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        #[cfg(not(feature = "rune-ui"))]
        WindowUi::Rune => {
            let _ = (first, shutdown);
            while let Ok(Control::Activate(_)) = requests.recv() {}
        }
        WindowUi::Gpui => run_gpui_windows(host, requests, first, shutdown),
    }
}

/// GPUI on Linux ends its loop with the last window, so it runs once per
/// opened window and the main thread waits for requests in between.
fn run_gpui_windows(
    host: &HostHandle,
    mut requests: Receiver<Control>,
    mut next: Option<Activation>,
    shutdown: &'static AtomicBool,
) {
    loop {
        if next.take().is_some() {
            host.set_window_open(true);
            let (quit, returned) = run_gpui_window(host, requests, shutdown);
            requests = returned;
            host.window_closed();
            if quit {
                break;
            }
        }
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match requests.recv() {
            Ok(Control::Activate(activation)) => next = Some(activation),
            Ok(Control::Quit) | Err(_) => break,
        }
    }
}

/// Runs GPUI for one window. GPUI on Linux ends its loop with the last
/// window, so closing it quits the loop. Returns whether Hex should quit,
/// and the request queue the window borrowed.
fn run_gpui_window(
    host: &HostHandle,
    requests: Receiver<Control>,
    shutdown: &'static AtomicBool,
) -> (bool, Receiver<Control>) {
    use gpui::{App, Application, Timer};
    let quit = Arc::new(AtomicBool::new(false));
    let (forward, forwarded) = mpsc::channel::<Control>();
    // GPUI's executor polls a channel of its own; the relay feeds it.
    let relay_stop = Arc::new(AtomicBool::new(false));
    let relay = {
        let relay_stop = relay_stop.clone();
        std::thread::spawn(move || {
            while !relay_stop.load(Ordering::Acquire) {
                match requests.recv_timeout(Duration::from_millis(100)) {
                    Ok(request) => {
                        if forward.send(request).is_err() {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            requests
        })
    };
    let host = host.clone();
    let quitting = quit.clone();
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys(crate::text_input::key_bindings());
        let window = match crate::app_window::open(host.clone(), cx) {
            Ok(window) => window,
            Err(error) => {
                tracing::error!(%error, "could not open the Hex window");
                cx.quit();
                return;
            }
        };
        window
            .update(cx, |_, window, cx| {
                window.set_window_title("Hex");
                // Quit without letting GPUI close the window here: on Wayland
                // it destroys the Vulkan surface at once, and a configure the
                // compositor queued behind the close (as sway does when it
                // retiles) then resizes the destroyed surface and segfaults.
                // Quitting stops the event loop after this dispatch instead.
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    false
                });
                window.activate_window();
            })
            .ok();
        cx.spawn(async move |cx| {
            loop {
                Timer::after(Duration::from_millis(50)).await;
                while let Ok(request) = forwarded.try_recv() {
                    match request {
                        Control::Activate(_) => {
                            let _ = window.update(cx, |_, window, _| window.activate_window());
                        }
                        Control::Quit => {
                            quitting.store(true, Ordering::Release);
                            let _ = cx.update(|cx| cx.quit());
                            return;
                        }
                    }
                }
                if shutdown.load(Ordering::Acquire) {
                    quitting.store(true, Ordering::Release);
                    let _ = cx.update(|cx| cx.quit());
                    return;
                }
            }
        })
        .detach();
        cx.activate(true);
    });
    relay_stop.store(true, Ordering::Release);
    let requests = relay.join().expect("the window request relay panicked");
    (quit.load(Ordering::Acquire), requests)
}

fn reopen_marker() -> Option<PathBuf> {
    crate::app_paths::support_dir()
        .ok()
        .map(|directory| directory.join(REOPEN_MARKER))
}

/// True when "Update" restarted this process with its window open.
fn take_reopen_marker() -> bool {
    let Some(path) = reopen_marker() else {
        return false;
    };
    let fresh = std::fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| {
            modified
                .elapsed()
                .is_ok_and(|age| age < Duration::from_secs(60))
        });
    let _ = std::fs::remove_file(&path);
    fresh
}

impl LinuxDesktopHost {
    fn new(
        event_path: PathBuf,
        settings: AppSettings,
        error: Option<String>,
        update: UpdateState,
    ) -> Self {
        let mut event_reader = EventReader::open(&event_path);
        let mut activity = DesktopActivity::default();
        activity.refresh(&mut event_reader);
        // Tests never touch the user's history.
        let history = if cfg!(test) {
            None
        } else {
            crate::history::History::open_default(settings.history_retention)
                .inspect_err(|error| tracing::warn!(%error, "could not open retained history"))
                .ok()
        };
        let output = crate::linux_dictation::DictationOutput::from_settings(&settings, history);
        let mut host = Self {
            event_reader,
            event_path,
            activity,
            listener: None,
            listen_when_ready: false,
            status: "Disabled".into(),
            error: None,
            dismissed_failure_at: None,
            settings_valid: error.is_none(),
            settings_error: error,
            settings,
            settings_edit: None,
            prepared_transcriber: None,
            transcription_preparation: None,
            transcription_error: None,
            update,
            output,
            personal_commands: None,
            personal_commands_checked_at: Instant::now(),
        };
        host.refresh_personal_commands();
        host
    }

    /// Applies saved settings the running listener reads live: modes,
    /// history retention, and the transformations workspace.
    fn apply_output_settings(&mut self) {
        *self
            .output
            .modes
            .write()
            .unwrap_or_else(|error| error.into_inner()) =
            crate::dictation_modes::ModeRuntime::from_settings(&self.settings);
        if let Some(history) = &self.output.history
            && let Err(error) = history.set_retention(self.settings.history_retention)
        {
            tracing::warn!(%error, "could not apply the history retention");
        }
        self.refresh_personal_commands();
    }

    fn refresh_personal_commands(&mut self) {
        if cfg!(test) {
            return;
        }
        let enabled =
            crate::app_settings::transformations_used(&self.settings.dictation_processing)
                || crate::personal_commands::workspace_configured();
        match (enabled, self.personal_commands.is_some()) {
            (true, false) => {
                self.personal_commands = crate::personal_commands::PersonalCommands::start(
                    crate::commands::CommandConfig::new(),
                    self.output.transformations.clone(),
                );
            }
            (false, true) => self.personal_commands = None,
            _ => {}
        }
    }

    fn start(&mut self) {
        if self.settings_edit.is_some() {
            return;
        }
        self.listen_when_ready = true;
        if self.is_running() || self.transcription_preparation.is_some() {
            return;
        }
        // Defaults are only an editable recovery projection after a failed load.
        // They must never become a recording configuration until saved successfully.
        if !self.settings_valid {
            self.status = "Couldn't enable".into();
            return;
        }
        let model = crate::transcription_models::definition(self.settings.transcription.model);
        if !crate::transcription_models::is_installed(model, &self.settings.transcription.language)
        {
            self.status = "Model required".into();
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let settings = self.settings.clone();
        let output = self.output.clone();
        let event_path = self.event_path.clone();
        let prepared_transcriber = self.prepared_transcriber.take();
        let hotkey_status = crate::linux_input::HotkeyStatus::default();
        let worker_hotkey_status = hotkey_status.clone();
        let (commands, command_receiver) = mpsc::sync_channel(16);
        let worker = std::thread::spawn(move || {
            let result = crate::instance::acquire("listener").and_then(|_instance| {
                crate::linux_dictation::run(
                    &event_path,
                    None,
                    &worker_stop,
                    &settings,
                    output,
                    prepared_transcriber,
                    worker_hotkey_status,
                    command_receiver,
                )
            });
            result.map_err(|error| format!("{error:#}"))
        });
        self.listener = Some(Listener {
            stop,
            worker,
            startup: ListenerStartup::Awaiting {
                previous_session: self.activity.session_started_at,
            },
            hotkey_status,
            commands,
        });
        self.status = "Enabling…".into();
    }

    fn stop(&mut self) {
        self.listen_when_ready = false;
        if let Some(edit) = &mut self.settings_edit {
            edit.resume = false;
            edit.canceled.store(true, Ordering::Release);
        }
        if let Some(listener) = &self.listener {
            listener.stop();
            self.status = "Disabling…".into();
        }
    }

    fn refresh(&mut self) {
        // Settings may set up the transformations workspace at any time.
        if self.personal_commands_checked_at.elapsed() >= Duration::from_secs(2) {
            self.personal_commands_checked_at = Instant::now();
            self.refresh_personal_commands();
        }
        if matches!(&self.update, UpdateState::Waiting(at) | UpdateState::Failed(at) if Instant::now() >= *at)
        {
            self.update = UpdateState::Checking(start_update_check());
        }
        let update_result = match &self.update {
            UpdateState::Checking(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err("update worker stopped unexpectedly".into()))
                }
                Err(TryRecvError::Empty) => None,
            },
            _ => None,
        };
        if let Some(result) = update_result {
            self.update = match result {
                Ok(Some(executable)) => UpdateState::Ready(executable),
                Ok(None) => UpdateState::Waiting(Instant::now() + UPDATE_INTERVAL),
                Err(error) => {
                    tracing::warn!(%error, "Linux update check failed");
                    UpdateState::Failed(Instant::now() + UPDATE_INTERVAL)
                }
            };
        }

        let mut start_listener = false;
        if let Some(preparation) = self
            .transcription_preparation
            .take_if(|preparation| preparation.worker.is_finished())
        {
            let was_canceled = preparation.canceled.load(Ordering::Relaxed);
            let result = preparation
                .worker
                .join()
                .unwrap_or_else(|_| Err("model preparation worker stopped unexpectedly".into()));
            if was_canceled {
                self.transcription_error = None;
            } else {
                match result {
                    Ok(prepared) => {
                        let selection = prepared.selection.clone();
                        match save_with(|settings| settings.remember_transcription(selection)) {
                            Ok(candidate) => {
                                self.settings = candidate;
                                self.settings_valid = true;
                                self.prepared_transcriber = Some(prepared.transcriber);
                                self.transcription_error = None;
                                self.settings_error = None;
                                self.status = "Disabled".into();
                                start_listener = self.listen_when_ready;
                            }
                            Err(error) => {
                                self.transcription_error = Some(format!(
                                    "Could not save transcription selection: {error:#}"
                                ));
                            }
                        }
                    }
                    Err(error) => {
                        self.transcription_error = Some(error);
                    }
                }
            }
        }
        if start_listener {
            self.start();
        }

        if let Some(listener) = self
            .listener
            .take_if(|listener| listener.worker.is_finished())
        {
            let result = listener.finish();
            match result {
                Ok(()) => self.status = "Disabled".into(),
                Err(error) => {
                    self.status = "Couldn't enable".into();
                    self.error = Some(error);
                }
            }
        }

        self.refresh_settings_edit();

        self.activity.refresh(&mut self.event_reader);
        if let Some(listener) = &mut self.listener {
            listener.observe_session(self.activity.session_started_at);
        }
        if self
            .listener
            .as_ref()
            .is_some_and(|listener| matches!(listener.startup, ListenerStartup::Observed))
            && self.listen_when_ready
            && let Some(status) = self.activity.state_label()
        {
            self.status = status.into();
        }
    }

    fn is_running(&self) -> bool {
        self.listener.is_some()
    }

    fn set_hotkey(&mut self, target: ShortcutTarget, shortcut: DesktopShortcut) -> Result<()> {
        if shortcut.function {
            self.error = Some("The Fn modifier cannot be registered on Linux".into());
            self.cancel_settings_edit();
            return Err(color_eyre::eyre::eyre!(
                "the Fn modifier cannot be registered on Linux"
            ));
        }
        let binding = LinuxHotkey {
            control: shortcut.control,
            alt: shortcut.alt,
            shift: shortcut.shift,
            super_key: shortcut.platform,
            key: if shortcut.key == " " {
                "space".into()
            } else {
                shortcut.key.to_ascii_lowercase()
            },
        };
        if let Some(edit) = &mut self.settings_edit {
            if edit.change == SettingsChange::Capture(target) && edit.worker.is_none() {
                edit.change = SettingsChange::Hotkey(target, binding);
            }
        } else {
            self.begin_settings_edit(SettingsChange::Hotkey(target, binding));
        }
        Ok(())
    }

    fn begin_settings_edit(&mut self, change: SettingsChange) {
        if self.settings_edit.is_some() {
            return;
        }
        if self.transcription_preparation.is_some() {
            self.error = Some("Wait for model preparation before editing shortcut settings".into());
            return;
        }
        // A listener waiting for a model still starts once one is ready.
        let resume = self.listen_when_ready;
        self.stop();
        self.error = None;
        self.settings_edit = Some(SettingsEdit {
            change,
            resume,
            canceled: Arc::new(AtomicBool::new(false)),
            worker: None,
        });
    }

    fn cancel_settings_edit(&mut self) {
        if let Some(edit) = &self.settings_edit {
            edit.canceled.store(true, Ordering::Release);
        }
    }

    /// Adopts settings the window saved. A running listener restarts when
    /// a setting it captured at start changed; the sound volume already
    /// applied when the window saved it.
    fn adopt_settings(&mut self, settings: AppSettings) {
        if self.settings_valid && settings == self.settings {
            return;
        }
        let model_changed = settings.transcription != self.settings.transcription;
        let restart = model_changed
            || settings.dictation_hotkey != self.settings.dictation_hotkey
            || settings.double_tap_lock != self.settings.double_tap_lock
            || settings.shortcut_mode != self.settings.shortcut_mode
            || settings.paste_with_shift != self.settings.paste_with_shift
            || settings.microphone != self.settings.microphone
            || settings.paste_last_hotkey != self.settings.paste_last_hotkey
            || settings.double_tap_only != self.settings.double_tap_only
            || settings.release_microphone_while_idle
                != self.settings.release_microphone_while_idle
            || settings.recording_audio_behavior != self.settings.recording_audio_behavior
            || settings.indicator_position != self.settings.indicator_position
            || settings.voice_action.enabled != self.settings.voice_action.enabled
            || settings.edit_hotkey != self.settings.edit_hotkey;
        if restart && self.is_running() && self.settings_edit.is_none() {
            self.begin_settings_edit(SettingsChange::Reload(Box::new(settings)));
            return;
        }
        self.settings = settings;
        self.settings_valid = true;
        self.settings_error = None;
        self.apply_output_settings();
        if model_changed && self.listen_when_ready && !self.is_running() {
            self.start();
        }
    }

    fn capturing_hotkey(&self) -> bool {
        self.settings_edit.as_ref().is_some_and(|edit| {
            !edit.canceled.load(Ordering::Acquire)
                && matches!(edit.change, SettingsChange::Capture(_))
        })
    }

    fn has_pending_work(&self) -> bool {
        self.is_running()
            || self.settings_edit.is_some()
            || self.transcription_preparation.is_some()
    }

    fn refresh_settings_edit(&mut self) {
        if self.is_running() {
            return;
        }
        let Some(edit) = &mut self.settings_edit else {
            return;
        };
        if edit
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return;
        }
        let canceled = edit.canceled.load(Ordering::Acquire);
        if let Some(worker) = edit.worker.take() {
            match worker.join() {
                // Once persistence completed, keep the in-memory projection consistent even
                // if Cancel raced with the final atomic save.
                Ok(Ok(settings)) => {
                    self.settings = settings;
                    self.settings_valid = true;
                    self.settings_error = None;
                    self.apply_output_settings();
                }
                Ok(Err(error)) if !canceled => {
                    self.error = Some(format!("Could not update dictation settings: {error:#}"));
                }
                Err(_) if !canceled => {
                    self.error = Some("Settings worker stopped unexpectedly".into())
                }
                _ => {}
            }
        } else if !canceled {
            if matches!(edit.change, SettingsChange::Capture(_)) {
                // Settings records the chord from its own key events.
                self.status = "Press a shortcut".into();
                return;
            }
            let change = edit.change.clone();
            let canceled = edit.canceled.clone();
            edit.worker = Some(std::thread::spawn(move || {
                let (target, binding) = match change {
                    SettingsChange::Capture(_) => unreachable!("Settings records the chord"),
                    SettingsChange::Hotkey(target, binding) => (target, binding),
                    SettingsChange::Reload(settings) => return Ok(*settings),
                };
                binding.validate()?;
                if crate::linux_session::LinuxSession::detect().is_wayland() {
                    // The listener asks the desktop again and reports what it granted.
                    binding.portal_trigger()?;
                } else {
                    // Fail before saving when another X11 client holds the chord.
                    crate::linux_input::LinuxHotkeyMonitor::start(
                        binding.clone(),
                        Default::default(),
                        Default::default(),
                        mpsc::channel().1,
                    )?;
                }
                if canceled.load(Ordering::Acquire) {
                    return Err(color_eyre::eyre::eyre!("Shortcut edit canceled"));
                }
                save_with(|settings| {
                    let binding = HotkeyBinding::from(&binding);
                    match target {
                        ShortcutTarget::Dictation => settings.dictation_hotkey = binding,
                        ShortcutTarget::PasteLast => settings.paste_last_hotkey = Some(binding),
                        ShortcutTarget::VoiceAction => settings.edit_hotkey = binding,
                    }
                })
            }));
            self.status = if self.capturing_hotkey() {
                "Press a shortcut"
            } else {
                "Saving settings"
            }
            .into();
            return;
        }
        let resume = self.settings_edit.take().is_some_and(|edit| edit.resume);
        self.status = "Disabled".into();
        if resume {
            self.start();
        }
    }

    fn restart_into_update(&self) -> Result<()> {
        let UpdateState::Ready(update) = &self.update else {
            return Err(color_eyre::eyre::eyre!("no installed update is ready"));
        };
        if let Some(marker) = reopen_marker() {
            let _ = std::fs::write(marker, b"");
        }
        crate::linux_updater::relaunch(update)
    }

    fn choose_transcription(
        &mut self,
        model: crate::transcription_models::TranscriptionModelId,
        language: String,
    ) -> Result<()> {
        if self.settings_edit.is_some() {
            let error = "Finish the shortcut change before changing the model".to_string();
            self.transcription_error = Some(error.clone());
            return Err(color_eyre::eyre::eyre!(error));
        }
        if self.transcription_preparation.is_some() {
            self.cancel_transcription_preparation();
            let error = "The previous model preparation is still stopping".to_string();
            self.transcription_error = Some(error.clone());
            return Err(color_eyre::eyre::eyre!(error));
        }
        let definition = crate::transcription_models::definition(model);
        let selection = crate::transcription_models::TranscriptionSelection {
            model,
            language,
            // Hints persist across models that can use them.
            recognition_hints: if definition.supports_recognition_hints {
                self.settings.transcription.recognition_hints.clone()
            } else {
                String::new()
            },
        };
        crate::transcription_models::validate(&selection)?;
        // A running listener stops for the swap and starts again with the
        // prepared model.
        if self.is_running() {
            self.stop();
            self.listen_when_ready = true;
        }
        let canceled = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let stage = Arc::new(AtomicU8::new(
            crate::transcription_models::ModelPreparationStage::Downloading as u8,
        ));
        let worker_canceled = canceled.clone();
        let worker_progress = progress.clone();
        let worker_stage = stage.clone();
        let worker = std::thread::spawn(move || {
            (|| {
                crate::transcription_models::download_with_stage_progress(
                    definition,
                    &worker_canceled,
                    &worker_progress,
                    &worker_stage,
                )?;
                if worker_canceled.load(Ordering::Relaxed) {
                    return Err(color_eyre::eyre::eyre!("model activation canceled"));
                }
                crate::transcription_models::ModelPreparationStage::Loading.store(&worker_stage);
                let transcriber = crate::local_transcriber::LocalTranscriber::load(&selection)?;
                if worker_canceled.load(Ordering::Relaxed) {
                    return Err(color_eyre::eyre::eyre!("model activation canceled"));
                }
                Ok(PreparedTranscription {
                    selection,
                    transcriber,
                })
            })()
            .map_err(|error| format!("{error:#}"))
        });
        self.transcription_preparation = Some(TranscriptionPreparation {
            canceled,
            model,
            progress,
            stage,
            worker,
        });
        self.transcription_error = None;
        Ok(())
    }

    fn cancel_transcription_preparation(&mut self) {
        if let Some(preparation) = &self.transcription_preparation {
            preparation.canceled.store(true, Ordering::Relaxed);
        }
        self.transcription_error = None;
    }
}

/// Saves one host-owned change on top of the current `settings.json`, so
/// edits the window saved meanwhile survive.
fn save_with(change: impl FnOnce(&mut AppSettings)) -> Result<AppSettings> {
    let mut settings = AppSettings::load()?;
    change(&mut settings);
    settings.save()?;
    Ok(settings)
}

fn start_update_check() -> Receiver<Result<Option<InstalledUpdate>, String>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = crate::linux_updater::install_latest().map_err(|error| format!("{error:#}"));
        let _ = sender.send(result);
    });
    receiver
}

impl Drop for LinuxDesktopHost {
    fn drop(&mut self) {
        self.stop();
        self.cancel_transcription_preparation();
        if let Some(listener) = self
            .listener
            .take_if(|listener| listener.worker.is_finished())
        {
            let _ = listener.finish();
        }
        if let Some(preparation) = self
            .transcription_preparation
            .take_if(|preparation| preparation.worker.is_finished())
        {
            let _ = preparation.worker.join();
        }
        if let Some(edit) = &mut self.settings_edit
            && let Some(worker) = edit.worker.take_if(|worker| worker.is_finished())
        {
            let _ = worker.join();
        }
    }
}

impl LinuxDesktopHost {
    fn snapshot(&self) -> DesktopSnapshot {
        let update_status = match &self.update {
            UpdateState::Unmanaged => DesktopUpdateStatus::Unavailable,
            UpdateState::Checking(_) => DesktopUpdateStatus::Checking,
            UpdateState::Failed(_) => DesktopUpdateStatus::Failed,
            UpdateState::Waiting(_) => DesktopUpdateStatus::Current,
            UpdateState::Ready(_) => DesktopUpdateStatus::ReadyToRestart,
        };
        DesktopSnapshot {
            activity: self.activity.clone(),
            listener: DesktopListenerSnapshot {
                running: self.is_running(),
                status: self.status.clone(),
                warning: self
                    .listener
                    .as_ref()
                    .and_then(|listener| listener.hotkey_status.warning()),
                shortcuts: self
                    .listener
                    .as_ref()
                    .map(|listener| listener.hotkey_status.shortcuts())
                    .unwrap_or_default(),
            },
            operation_error: self
                .error
                .clone()
                .or_else(|| self.settings_error.clone())
                .or_else(|| {
                    self.activity
                        .last_failure
                        .as_ref()
                        .filter(|(at, _)| Some(*at) != self.dismissed_failure_at)
                        .map(|(_, message)| message.clone())
                }),
            transcription: DesktopTranscriptionSnapshot {
                downloaded_bytes: self
                    .transcription_preparation
                    .as_ref()
                    .map_or(0, |preparation| {
                        preparation.progress.load(Ordering::Relaxed)
                    }),
                error: self.transcription_error.clone(),
                preparation_stage: self.transcription_preparation.as_ref().map(|preparation| {
                    crate::transcription_models::ModelPreparationStage::load(&preparation.stage)
                }),
                selection: self.settings.transcription.clone(),
                preparing: self
                    .transcription_preparation
                    .as_ref()
                    .map(|preparation| preparation.model),
            },
            update_status,
        }
    }

    /// Where `hex` shortcut commands go: the listener, once it reports its
    /// session (a listener still loading its model would replay them late).
    fn shortcut_route(&self) -> Option<ShortcutRoute> {
        self.listener
            .as_ref()
            .filter(|listener| matches!(listener.startup, ListenerStartup::Observed))
            .map(|listener| ShortcutRoute {
                commands: listener.commands.clone(),
                voice_action: self.settings.voice_action.enabled,
            })
    }

    fn dispatch(&mut self, action: DesktopAction) -> Result<()> {
        match action {
            DesktopAction::ClearError => {
                self.error = None;
                self.settings_error = None;
                self.dismissed_failure_at = self.activity.last_failure.as_ref().map(|(at, _)| *at);
            }
            DesktopAction::RestartIntoUpdate => {
                if let Err(error) = self.restart_into_update() {
                    self.error = Some(format!("Could not restart Hex: {error:#}"));
                    return Err(error);
                }
            }
            DesktopAction::SetDictationShortcut(shortcut) => {
                self.set_hotkey(ShortcutTarget::Dictation, shortcut)?;
            }
            DesktopAction::StartListening => {
                self.error = None;
                self.dismissed_failure_at = self.activity.last_failure.as_ref().map(|(at, _)| *at);
                self.start();
            }
            DesktopAction::StopListening => self.stop(),
            // Linux checks on its own schedule and applies what it finds.
            DesktopAction::CheckForUpdates | DesktopAction::OpenUpdate => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcription_models::{
        ModelPreparationStage, TranscriptionModelId, TranscriptionSelection,
    };

    fn host_for_edit(running: bool) -> LinuxDesktopHost {
        let settings = AppSettings {
            transcription: TranscriptionSelection {
                // Prevent the restart assertions from opening a real microphone.
                model: TranscriptionModelId::AppleSpeech,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut host =
            LinuxDesktopHost::new(PathBuf::new(), settings, None, UpdateState::Unmanaged);
        if running {
            host.listener = Some(Listener {
                worker: std::thread::spawn(|| Ok(())),
                stop: Arc::new(AtomicBool::new(false)),
                startup: ListenerStartup::Observed,
                hotkey_status: Default::default(),
                commands: mpsc::sync_channel(1).0,
            });
            host.listen_when_ready = true;
        }
        host
    }

    fn set_finished_edit(host: &mut LinuxDesktopHost, result: Result<AppSettings>) {
        let worker = std::thread::spawn(move || result);
        while !worker.is_finished() {
            std::thread::yield_now();
        }
        host.settings_edit.as_mut().unwrap().worker = Some(worker);
    }

    #[test]
    fn malformed_settings_cannot_start_even_after_dismissing_the_error() {
        let mut host = LinuxDesktopHost::new(
            PathBuf::new(),
            // A model never installed on Linux, so the resumed start below
            // cannot open the microphone where the real model is installed.
            host_for_edit(false).settings.clone(),
            Some("Could not load Linux settings: invalid JSON".into()),
            UpdateState::Unmanaged,
        );
        host.start();
        assert!(!host.is_running());
        assert_eq!(host.status, "Couldn't enable");
        host.dispatch(DesktopAction::ClearError).unwrap();
        host.start();
        assert!(!host.is_running());
        assert_eq!(host.status, "Couldn't enable");
        assert!(!host.settings_valid);

        let mut recovered = host.settings.clone();
        recovered.double_tap_lock = false;
        host.begin_settings_edit(SettingsChange::Reload(Box::new(recovered.clone())));
        set_finished_edit(&mut host, Ok(recovered.clone()));
        host.refresh_settings_edit();
        assert!(host.settings_valid);
        assert_eq!(host.settings, recovered);
        assert!(!host.is_running());
        assert_eq!(host.status, "Model required");
    }

    #[test]
    fn listener_startup_ignores_the_previous_session_until_a_new_one_arrives() {
        for previous_session in [None, Some(10)] {
            let mut listener = Listener {
                stop: Arc::new(AtomicBool::new(false)),
                worker: std::thread::spawn(|| Ok(())),
                startup: ListenerStartup::Awaiting { previous_session },
                hotkey_status: Default::default(),
                commands: mpsc::sync_channel(1).0,
            };
            listener.observe_session(previous_session);
            assert!(matches!(listener.startup, ListenerStartup::Awaiting { .. }));
            listener.observe_session(Some(20));
            assert!(matches!(listener.startup, ListenerStartup::Observed));
            listener.observe_session(previous_session);
            assert!(matches!(listener.startup, ListenerStartup::Observed));
            listener.stop();
            assert!(listener.stop.load(Ordering::Relaxed));
            listener.finish().unwrap();
        }
    }

    #[test]
    fn listener_warning_is_nonfatal_and_round_trips_in_status() {
        let mut host = host_for_edit(true);
        host.status = "Enabled".into();
        let status = host.listener.as_ref().unwrap().hotkey_status.clone();
        status.update_escape_availability(false);
        let serialized = serde_json::to_vec(&host.snapshot()).unwrap();
        let received: DesktopSnapshot = serde_json::from_slice(&serialized).unwrap();
        let listener = received.listener;
        assert!(listener.running);
        assert_eq!(listener.status, "Enabled");
        assert_eq!(
            listener.warning.as_deref(),
            Some(crate::linux_input::ESCAPE_CANCEL_WARNING),
        );
        assert!(received.operation_error.is_none());

        // Dismissing an unrelated failure must not hide an unresolved limitation.
        host.error = Some("previous transcription failed".into());
        assert_eq!(
            host.snapshot().operation_error.as_deref(),
            Some("previous transcription failed"),
        );
        host.dispatch(DesktopAction::ClearError).unwrap();
        assert!(host.snapshot().listener.warning.is_some());
        assert!(host.snapshot().operation_error.is_none());
        status.update_escape_availability(true);
        assert!(host.snapshot().listener.warning.is_none());
        status.update_escape_availability(false);
        host.stop();
        while !host.listener.as_ref().unwrap().worker.is_finished() {
            std::thread::yield_now();
        }
        host.refresh();
        let listener = host.snapshot().listener;
        assert!(!listener.running);
        assert!(listener.warning.is_none());
    }

    #[test]
    fn listener_completion_waits_for_the_worker_and_preserves_errors() {
        for outcome in [Some(Ok(())), Some(Err("listener failed".into())), None] {
            let expected = outcome
                .clone()
                .unwrap_or_else(|| Err("listener worker stopped unexpectedly".into()))
                .err();
            let mut host = host_for_edit(true);
            let (release, held) = mpsc::channel();
            host.listener.as_mut().unwrap().worker = std::thread::spawn(move || {
                held.recv_timeout(Duration::from_secs(5)).unwrap();
                outcome.expect("fixture listener panic")
            });
            host.refresh();
            assert!(host.is_running());
            assert!(!host.listener.as_ref().unwrap().stop.load(Ordering::Relaxed));
            release.send(()).unwrap();
            while !host.listener.as_ref().unwrap().worker.is_finished() {
                std::thread::yield_now();
            }
            host.refresh();
            assert!(!host.is_running());
            assert!(host.listener.is_none());
            assert_eq!(host.error, expected);
            assert_eq!(
                host.status,
                if expected.is_some() {
                    "Couldn't enable"
                } else {
                    "Disabled"
                }
            );
        }
    }

    #[test]
    fn preparation_completion_preserves_selection_and_honors_cancellation() {
        for (canceled, panics) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut host = host_for_edit(false);
            let original = host.settings.clone();
            let (release, held) = mpsc::channel();
            host.transcription_preparation = Some(TranscriptionPreparation {
                canceled: Arc::new(AtomicBool::new(false)),
                model: TranscriptionModelId::default(),
                progress: Arc::new(AtomicU64::new(0)),
                stage: Arc::new(AtomicU8::new(ModelPreparationStage::Loading as u8)),
                worker: std::thread::spawn(move || {
                    held.recv_timeout(Duration::from_secs(5)).unwrap();
                    assert!(!panics, "fixture preparation panic");
                    Err("model unavailable".into())
                }),
            });
            if canceled {
                host.cancel_transcription_preparation();
            }
            host.refresh();
            assert!(host.has_pending_work());
            release.send(()).unwrap();
            while !host
                .transcription_preparation
                .as_ref()
                .unwrap()
                .worker
                .is_finished()
            {
                std::thread::yield_now();
            }
            host.refresh();
            assert!(!host.has_pending_work());
            assert_eq!(host.settings, original);
            assert_eq!(
                host.transcription_error.as_deref(),
                if canceled {
                    None
                } else if panics {
                    Some("model preparation worker stopped unexpectedly")
                } else {
                    Some("model unavailable")
                }
            );
        }
    }

    #[test]
    fn preference_edits_wait_for_stop_and_only_restart_a_previously_running_listener() {
        for running in [false, true] {
            let mut host = host_for_edit(running);
            let mut candidate = host.settings.clone();
            candidate.paste_with_shift = !candidate.paste_with_shift;
            host.begin_settings_edit(SettingsChange::Reload(Box::new(candidate.clone())));
            assert_eq!(host.settings_edit.as_ref().unwrap().resume, running);
            if running {
                host.refresh_settings_edit();
                assert!(host.listener.as_ref().unwrap().stop.load(Ordering::Relaxed));
                assert!(host.settings_edit.as_ref().unwrap().worker.is_none());
                assert_ne!(host.settings, candidate);
            }
            host.listener = None;
            set_finished_edit(&mut host, Ok(candidate.clone()));
            host.refresh_settings_edit();
            assert_eq!(host.settings, candidate);
            assert!(host.settings_edit.is_none());
            assert_eq!(host.listen_when_ready, running);
        }
    }

    #[test]
    fn canceled_or_failed_capture_restores_prior_listening_without_changing_settings() {
        for running in [false, true] {
            for cancel in [false, true] {
                let mut host = host_for_edit(running);
                let original = host.settings.clone();
                host.begin_settings_edit(SettingsChange::Capture(ShortcutTarget::Dictation));
                assert!(host.capturing_hotkey());
                host.listener = None;
                if cancel {
                    host.cancel_settings_edit();
                } else {
                    set_finished_edit(
                        &mut host,
                        Err(color_eyre::eyre::eyre!("input permission denied")),
                    );
                }
                host.refresh_settings_edit();
                assert_eq!(host.settings, original);
                assert_eq!(host.listen_when_ready, running);
                assert_eq!(host.error.is_some(), !cancel);
                assert!(host.settings_edit.is_none());
                assert!(!host.capturing_hotkey());
            }
        }
    }

    #[test]
    fn cancel_signals_worker_and_does_not_wait_or_restart_until_it_finishes() {
        let mut host = host_for_edit(true);
        host.begin_settings_edit(SettingsChange::Capture(ShortcutTarget::Dictation));
        host.listener = None;
        let (release, held) = mpsc::channel();
        host.settings_edit.as_mut().unwrap().worker = Some(std::thread::spawn(move || {
            let _ = held.recv();
            Err(color_eyre::eyre::eyre!("capture canceled"))
        }));
        host.cancel_settings_edit();
        host.refresh_settings_edit();
        let edit = host.settings_edit.as_mut().unwrap();
        assert!(edit.canceled.load(Ordering::Acquire));
        assert!(edit.worker.is_some());
        assert!(!host.listen_when_ready);
        release.send(()).unwrap();
        let _ = host
            .settings_edit
            .as_mut()
            .unwrap()
            .worker
            .take()
            .unwrap()
            .join()
            .unwrap()
            .unwrap_err();
        host.refresh_settings_edit();
        assert!(host.listen_when_ready);
        assert!(host.settings_edit.is_none());
    }

    #[test]
    fn closing_the_window_cancels_only_an_uncommitted_capture() {
        static SHUTDOWN: AtomicBool = AtomicBool::new(false);
        let mut host = host_for_edit(true);
        host.begin_settings_edit(SettingsChange::Capture(ShortcutTarget::Dictation));
        let handle = HostHandle::new(host, &SHUTDOWN);
        handle.window_closed();
        {
            let mut host = handle.lock();
            let edit = host.settings_edit.as_ref().unwrap();
            assert!(edit.resume);
            assert!(edit.canceled.load(Ordering::Acquire));
            host.listener = None;
            host.refresh_settings_edit();
            assert!(host.listen_when_ready);
        }

        // Without a capture, closing the window leaves dictation alone.
        let handle = HostHandle::new(host_for_edit(true), &SHUTDOWN);
        handle.window_closed();
        let host = handle.lock();
        assert!(host.is_running());
        assert!(host.listen_when_ready);
        assert!(!host.listener.as_ref().unwrap().stop.load(Ordering::Acquire));
    }

    #[test]
    fn commands_route_only_to_a_started_listener() {
        let mut host = host_for_edit(false);
        assert!(host.shortcut_route().is_none());
        let (commands, received) = mpsc::sync_channel(4);
        host.listener = Some(Listener {
            worker: std::thread::spawn(|| Ok(())),
            stop: Arc::new(AtomicBool::new(false)),
            startup: ListenerStartup::Awaiting {
                previous_session: None,
            },
            hotkey_status: Default::default(),
            commands,
        });
        // A listener still loading its model would replay the command late.
        assert!(host.shortcut_route().is_none());
        host.listener.as_mut().unwrap().startup = ListenerStartup::Observed;
        let route = host.shortcut_route().unwrap();
        assert!(!route.voice_action);
        linux_service::route_shortcut(Some(&route), ShortcutCommand::PasteLast).unwrap();
        host.settings.voice_action.enabled = true;
        assert!(host.shortcut_route().unwrap().voice_action);
        assert_eq!(
            received.try_iter().collect::<Vec<_>>(),
            [ShortcutCommand::PasteLast]
        );
    }

    #[test]
    fn adopting_saved_settings_restarts_only_for_listener_settings() {
        let mut host = host_for_edit(true);
        let mut candidate = host.settings.clone();
        candidate.history_retention = crate::history::HistoryRetention::Off;
        host.adopt_settings(candidate.clone());
        assert!(host.settings_edit.is_none());
        assert_eq!(host.settings, candidate);
        candidate.paste_with_shift = !candidate.paste_with_shift;
        host.adopt_settings(candidate.clone());
        assert!(host.settings_edit.is_some());
        assert_ne!(host.settings, candidate);
    }

    #[test]
    fn dismiss_clears_operational_and_settings_errors() {
        let mut host = host_for_edit(false);
        host.error = Some("listener failed".into());
        host.settings_error = Some("settings failed".into());
        host.dispatch(DesktopAction::ClearError).unwrap();
        assert!(host.snapshot().operation_error.is_none());
    }

    #[test]
    fn output_failures_are_visible_and_dismissal_only_acknowledges_that_failure() {
        let mut host = host_for_edit(false);
        host.activity.last_failure = Some((10, "paste helper failed".into()));
        assert_eq!(
            host.snapshot().operation_error.as_deref(),
            Some("paste helper failed")
        );
        host.dispatch(DesktopAction::ClearError).unwrap();
        assert!(host.snapshot().operation_error.is_none());
        host.activity.last_failure = Some((11, "paste helper failed".into()));
        assert_eq!(
            host.snapshot().operation_error.as_deref(),
            Some("paste helper failed")
        );
        host.dispatch(DesktopAction::StartListening).unwrap();
        assert!(host.snapshot().operation_error.is_none());
    }

    #[test]
    fn start_during_preparation_defers_listening_and_stop_clears_the_request() {
        let settings = AppSettings {
            // An unavailable model prevents a regressed guard from opening audio.
            transcription: TranscriptionSelection {
                model: TranscriptionModelId::AppleSpeech,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut host =
            LinuxDesktopHost::new(PathBuf::new(), settings, None, UpdateState::Unmanaged);
        host.transcription_preparation = Some(TranscriptionPreparation {
            canceled: Arc::new(AtomicBool::new(false)),
            model: TranscriptionModelId::default(),
            progress: Arc::new(AtomicU64::new(0)),
            stage: Arc::new(AtomicU8::new(ModelPreparationStage::Downloading as u8)),
            worker: std::thread::spawn(|| Err("fixture preparation".into())),
        });

        host.dispatch(DesktopAction::StartListening).unwrap();

        assert!(host.listen_when_ready);
        assert_eq!(host.status, "Disabled");
        assert!(!host.is_running());
        assert!(host.listener.is_none());
        assert!(host.transcription_preparation.is_some());

        host.dispatch(DesktopAction::StopListening).unwrap();

        assert!(!host.listen_when_ready);
        assert!(host.transcription_preparation.is_some());
    }
}
