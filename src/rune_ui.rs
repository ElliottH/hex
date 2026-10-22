//! The Linux window on Rune, the default on builds with `rune-ui`.
//!
//! It runs in the Hex process beside the host (`linux_app::HostHandle`):
//! edits save `settings.json` through the host, which adopts them at once,
//! and listening, shortcuts, models, and updates are direct host requests.
//! The host wakes the window when its state changes. It has the same panes
//! Linux shows in GPUI (Settings, Modes, Voice Action, History), the
//! transcription picker, and first-run setup, drawn with `rune-kit`'s
//! components in OpenCode's design language (`rune-theme-opencode`), not
//! Hex's GPUI design tokens.

mod controls;
mod history;
mod models;
mod modes;
mod onboarding;
mod picker;
mod settings;
mod voice_action;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use rune::{AppSpec, Cx, El, Icon, ScrollSpec, TextEdit, Tx, Waker, col, el, icon, row, text};

use crate::app_settings::{AppSettings, HotkeyBinding, HotkeyKey, HotkeyModifiers};
use crate::desktop_host::{
    DesktopAction, DesktopGrantedShortcut, DesktopListenerSnapshot, DesktopShortcuts,
    DesktopTranscriptionSnapshot, DesktopUpdateStatus,
};
use crate::desktop_i18n::{tr, tr_fill};
use crate::linux_app::{HostHandle, HostRequest};
use crate::linux_service::{Activation, ShortcutTarget};
use crate::transcription_models::TranscriptionModelId;
use controls::Paint;
use settings::Capture;

/// How long typing pauses before a text field's edit is saved, as in GPUI.
const DRAFT_DELAY: Duration = Duration::from_millis(250);

static SETTINGS_ICON: Icon = Icon {
    name: "hex-settings",
    view: 16.0,
    body: include_str!("../assets/icons/settings.svg"),
};
static MODES_ICON: Icon = Icon {
    name: "hex-modes",
    view: 16.0,
    body: include_str!("../assets/icons/modes.svg"),
};
static VOICE_ACTION_ICON: Icon = Icon {
    name: "hex-voice-action",
    view: 16.0,
    body: include_str!("../assets/icons/voice-action.svg"),
};
static HISTORY_ICON: Icon = Icon {
    name: "hex-history",
    view: 16.0,
    body: include_str!("../assets/icons/history.svg"),
};

/// The host the next window opens on; `AppSpec::init` takes no arguments.
static HOST: Mutex<Option<HostHandle>> = Mutex::new(None);

/// Opens the window beside `host` and returns when it closes. The seam
/// `linux_app::run_windows` drives; Rune's windowless app lifetime will let
/// it reopen and focus the window and consume `activation`.
pub(crate) fn run_window(host: HostHandle, _activation: Activation) {
    struct Closed;
    impl Drop for Closed {
        fn drop(&mut self) {
            HOST.lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
        }
    }
    *HOST.lock().unwrap() = Some(host);
    let _closed = Closed;
    rune::set_app_id("hex");
    rune::run(spec());
}

/// `hex app -- --snap out.png --scene …`: a headless render of a fixture
/// scene, or `live` for the running Hex's status.
pub(crate) fn render() {
    rune::run(spec());
}

fn spec() -> AppSpec<Ui> {
    AppSpec {
        title: "Hex",
        size: (980.0, 760.0),
        init: Ui::load,
        view,
        scenes: SCENES,
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
    }
}

/// Where the window sends its requests.
enum Service {
    Live(HostHandle),
    /// Snapshots and tests: requests are recorded, never sent.
    Fixture {
        sent: Vec<HostRequest>,
        /// Settings saves.
        saved: usize,
        quit: bool,
    },
}

impl Service {
    fn fixture() -> Self {
        Self::Fixture {
            sent: Vec::new(),
            saved: 0,
            quit: false,
        }
    }
}

/// What the window shows of the host, from its latest snapshot.
#[derive(Clone, PartialEq)]
struct ServiceView {
    listener: DesktopListenerSnapshot,
    operation_error: Option<String>,
    transcription: DesktopTranscriptionSnapshot,
    update_status: DesktopUpdateStatus,
    /// The host is recording or saving a shortcut.
    editing: bool,
}

/// The panes Linux shows, in navigation order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Pane {
    #[default]
    Settings,
    Modes,
    VoiceAction,
    History,
}

impl Pane {
    const ALL: [Pane; 4] = [
        Pane::Settings,
        Pane::Modes,
        Pane::VoiceAction,
        Pane::History,
    ];

    fn title(self) -> &'static str {
        match self {
            Pane::Settings => tr("Settings"),
            Pane::Modes => tr("Modes"),
            Pane::VoiceAction => tr("Voice Action"),
            Pane::History => tr("History"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Pane::Settings => "nav-settings",
            Pane::Modes => "nav-modes",
            Pane::VoiceAction => "nav-voice-action",
            Pane::History => "nav-history",
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Pane::Settings => &SETTINGS_ICON,
            Pane::Modes => &MODES_ICON,
            Pane::VoiceAction => &VOICE_ACTION_ICON,
            Pane::History => &HISTORY_ICON,
        }
    }
}

/// The one anchored menu or popover that is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Menu {
    Microphone,
    Language,
    HistoryRetention,
    /// The OpenCode model search for Voice Action or the selected mode.
    Model,
    /// The OpenCode thinking variant.
    Variant,
}

pub(crate) struct Ui {
    settings: AppSettings,
    service: Service,
    view: Option<ServiceView>,
    /// A save, policy change, or host request that failed here.
    error: Option<String>,
    /// The settings the host last reported, to adopt only its changes.
    service_settings: Option<AppSettings>,
    pane: Pane,
    open: Option<Menu>,
    // Settings
    microphones: Result<Vec<String>, String>,
    confirm_release: bool,
    capture: Capture,
    hints: TextEdit,
    /// The model the host was last seen preparing, and since when.
    preparing: Option<(TranscriptionModelId, Instant)>,
    /// When a text field last changed without being saved.
    draft_since: Option<Instant>,
    models: models::Models,
    voice_action_error: Option<String>,
    history: history::HistoryPane,
    modes: modes::ModesPane,
    picker: picker::Picker,
    tour: Option<onboarding::Tour>,
}

impl Ui {
    fn load() -> Self {
        let Some(host) = HOST.lock().unwrap().clone() else {
            // A headless render (`--snap`) without a scene shows defaults.
            return fixture(DesktopShortcuts::Registered);
        };
        let snapshot = host.snapshot();
        crate::desktop_i18n::apply(snapshot.settings.ui_language.as_deref());
        let history = host.history();
        let mut ui = Self::with(snapshot.settings, Service::Live(host), None);
        ui.history = history::HistoryPane::shared(history);
        ui.models = models::Models::load();
        ui.modes.load_personal_commands();
        ui.tour = onboarding::Tour::start(&ui.settings);
        poll(&mut ui, 0.0, &mut Tx::default());
        ui
    }

    fn with(settings: AppSettings, service: Service, error: Option<String>) -> Self {
        let hints = TextEdit::new(settings.transcription.recognition_hints.clone());
        let modes = modes::ModesPane::new(&settings);
        Self {
            settings,
            service,
            view: None,
            error,
            service_settings: None,
            pane: Pane::Settings,
            open: None,
            microphones: Ok(Vec::new()),
            confirm_release: false,
            capture: Capture::Idle,
            hints,
            preparing: None,
            draft_since: None,
            models: models::Models::default(),
            voice_action_error: None,
            history: history::HistoryPane::default(),
            modes,
            picker: picker::Picker::default(),
            tour: None,
        }
    }

    fn live(&self) -> bool {
        matches!(self.service, Service::Live(_))
    }

    fn send(&mut self, request: HostRequest) {
        match &mut self.service {
            Service::Live(host) => {
                if let Err(error) = host.request(request) {
                    self.error = Some(format!("{error:#}"));
                }
            }
            Service::Fixture { sent, .. } => sent.push(request),
        }
    }

    /// Saves the change through the host, which adopts it at once. A failed
    /// save leaves the shown settings alone.
    fn update(&mut self, change: impl FnOnce(&mut AppSettings)) -> bool {
        let mut candidate = self.drafted();
        change(&mut candidate);
        self.commit(candidate)
    }

    fn commit(&mut self, candidate: AppSettings) -> bool {
        match &mut self.service {
            Service::Live(host) => {
                if let Err(error) = host.save_settings(candidate.clone()) {
                    self.error = Some(format!("{error:#}"));
                    return false;
                }
            }
            Service::Fixture { saved, .. } => *saved += 1,
        }
        self.error = None;
        self.settings = candidate;
        self.draft_since = None;
        true
    }

    /// The settings with every text field's current text applied.
    fn drafted(&self) -> AppSettings {
        let mut candidate = self.settings.clone();
        if crate::transcription_models::definition(candidate.transcription.model)
            .supports_recognition_hints
        {
            candidate.transcription.recognition_hints = self.hints.text.clone();
        }
        self.modes.project(&mut candidate);
        candidate
    }

    /// Saves text-field edits once typing pauses for [`DRAFT_DELAY`].
    fn save_drafts(&mut self, now: Instant) -> bool {
        let candidate = self.drafted();
        if candidate == self.settings {
            self.draft_since = None;
            return false;
        }
        let since = *self.draft_since.get_or_insert(now);
        if now.duration_since(since) < DRAFT_DELAY {
            return false;
        }
        self.commit(candidate)
    }

    /// Saves pending text-field edits now (leaving a pane or a mode).
    fn flush_drafts(&mut self) {
        let candidate = self.drafted();
        if candidate != self.settings {
            self.commit(candidate);
        }
    }

    fn running(&self) -> bool {
        self.view.as_ref().is_some_and(|view| view.listener.running)
    }

    fn shortcuts(&self) -> Option<&DesktopShortcuts> {
        self.view.as_ref().map(|view| &view.listener.shortcuts)
    }

    /// Without a shortcuts portal the desktop's keybindings own every
    /// shortcut, so the window has none to record.
    fn records_shortcuts(&self) -> bool {
        self.shortcuts() != Some(&DesktopShortcuts::Commands)
    }

    /// Adopts settings the host saved (a prepared model, a recorded
    /// shortcut), unless they are the stale copy of an edit this window just
    /// made.
    fn adopt(&mut self, service: AppSettings) {
        if self.service_settings.as_ref() == Some(&service) {
            return;
        }
        if service != self.settings && self.draft_since.is_none() {
            if service.transcription.recognition_hints
                != self.settings.transcription.recognition_hints
            {
                self.hints = TextEdit::new(service.transcription.recognition_hints.clone());
            }
            if service.dictation_processing != self.settings.dictation_processing {
                self.modes.reload(&service);
            }
            self.settings = service.clone();
        }
        self.service_settings = Some(service);
    }

    /// Saves pending edits, then lets the host drain its work and exit.
    fn quit(&mut self) {
        self.flush_drafts();
        settings::cancel_capture(self);
        match &mut self.service {
            Service::Live(host) => host.quit(),
            Service::Fixture { quit, .. } => *quit = true,
        }
    }

    fn select_pane(&mut self, pane: Pane) {
        if self.pane == pane {
            return;
        }
        self.flush_drafts();
        settings::cancel_capture(self);
        self.open = None;
        self.pane = pane;
        if pane == Pane::History {
            self.history.reload();
        }
    }
}

/// Shortcut recording takes every key; otherwise Escape closes the picker
/// or the open menu.
fn on_key(ui: &mut Ui, event: &rune::KeyEvent, tx: &mut Tx) -> bool {
    if settings::on_key(ui, event, tx) {
        return true;
    }
    if event.key != rune::Key::Escape {
        return false;
    }
    if ui.picker.language.is_some() {
        picker::close(ui);
        true
    } else if ui.open.is_some() {
        ui.open = None;
        true
    } else {
        false
    }
}

fn setup(ui: &mut Ui, waker: Waker) {
    if let Service::Live(host) = &ui.service {
        let tick = waker.clone();
        host.window_opened(waker);
        // Drafts, the OpenCode retry, and the setup beat are timed; until
        // Rune has `wake_after`, a quarter-second tick redraws them.
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(DRAFT_DELAY);
                if HOST.lock().unwrap().is_none() {
                    break;
                }
                tick();
            }
        });
    }
}

/// Runs before every frame: adopts host changes, follows shortcut capture,
/// model preparation, the OpenCode catalog, history, and setup, and saves
/// paused text edits. True when anything shown changed.
fn poll(ui: &mut Ui, _clock: f64, _tx: &mut Tx) -> bool {
    let now = Instant::now();
    let mut changed = settings::advance_capture(ui);
    changed |= ui.save_drafts(now);
    changed |= ui.models.poll(ui.live());
    changed |= ui.history.poll(ui.pane == Pane::History);
    changed |= ui.modes.poll(ui.live());
    if let Service::Live(host) = &ui.service {
        if host.finished() {
            // Quit drained the host; Rune cannot end its event loop from
            // here until its windowless app lifetime lands.
            std::process::exit(0);
        }
        let snapshot = host.snapshot();
        let view = Some(ServiceView {
            listener: snapshot.desktop.listener.clone(),
            operation_error: snapshot.desktop.operation_error.clone(),
            transcription: snapshot.desktop.transcription.clone(),
            update_status: snapshot.desktop.update_status,
            editing: snapshot.editing,
        });
        if view != ui.view {
            ui.view = view;
            changed = true;
        }
        let before = ui.settings.clone();
        ui.adopt(snapshot.settings);
        changed |= before != ui.settings;
    }
    let preparing = ui
        .view
        .as_ref()
        .and_then(|view| view.transcription.preparing);
    if preparing != ui.preparing.map(|(model, _)| model) {
        ui.preparing = preparing.map(|model| (model, now));
        changed = true;
    }
    changed |= onboarding::poll(ui);
    changed | settings::advance_capture(ui)
}

// ---------------------------------------------------------------------------
// View

fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    // Menus and popovers mount above the page (rune-kit overlays).
    let mut layer = Vec::new();
    let (actions, body) = match ui.pane {
        Pane::Settings => (Vec::new(), settings::pane(ui, p, &mut layer)),
        Pane::Modes => modes::pane(ui, p, &mut layer),
        Pane::VoiceAction => (Vec::new(), voice_action::pane(ui, p, &mut layer)),
        Pane::History => history::pane(ui, p, &mut layer),
    };
    let panel = col()
        .key("panel")
        .grow(1.0)
        .shrink()
        .mt(8.0)
        .mr(8.0)
        .mb(8.0)
        .radius(p.k.radius.lg)
        .fill(p.k.c.bg_base)
        .border(p.hair, p.k.c.border_weak)
        .clip_soft(0.0)
        .child(pane_header(p, ui.pane.title(), actions))
        .child(body);
    let window = row()
        .key("window")
        .role("window")
        .fill(p.k.c.bg_deep)
        .child(sidebar(ui, p))
        .child(panel);
    if let Some(modal) = picker::modal(ui, p) {
        layer.push(modal);
    }
    if let Some(tour) = onboarding::overlay(ui, p) {
        layer.push(tour);
    }
    rune_kit::root(window, layer)
}

fn sidebar(ui: &Ui, p: Paint) -> El {
    let mut nav = col().key("nav").gap(1.0);
    for pane in Pane::ALL {
        nav = nav.child(nav_item(p, pane, ui.pane == pane));
    }
    let update = matches!(
        ui.view.as_ref().map(|view| view.update_status),
        Some(DesktopUpdateStatus::ReadyToRestart)
    )
    .then(|| {
        controls::button(
            p,
            "sidebar-update",
            tr("Update"),
            controls::ButtonKind::Secondary,
            |ui: &mut Ui, _| ui.send(HostRequest::Desktop(DesktopAction::RestartIntoUpdate)),
        )
    });
    col()
        .key("sidebar")
        .role("navigation")
        .w(220.0)
        .h_full()
        .pt(12.0)
        .pb(12.0)
        .pl(10.0)
        .pr(6.0)
        .child(nav)
        .child(el().key("spacer").grow(1.0))
        .children(update)
        .child(quit_item(p))
        .child(
            text(tr_fill("Version {}", env!("CARGO_PKG_VERSION")))
                .key("version")
                .font(p.k.ty.small)
                .color(p.faint())
                .pl(6.0)
                .pt(8.0),
        )
}

fn nav_item(p: Paint, pane: Pane, on: bool) -> El {
    let c = &p.k.c;
    let mut item = row()
        .key(pane.key())
        .role("button")
        .label(pane.title())
        .h(28.0)
        .px(6.0)
        .gap(8.0)
        .center_y()
        .radius(p.k.radius.md)
        .hover(c.hover)
        .press(c.pressed)
        .on_press(move |ui: &mut Ui| ui.select_pane(pane))
        .child(icon(pane.icon(), 16.0).key("icon").color(if on {
            c.icon_base
        } else {
            c.icon_muted
        }))
        .child(
            text(pane.title())
                .key("label")
                .font(p.k.ty.ui)
                .color(if on { c.text_base } else { c.text_muted })
                .truncate(),
        );
    if on {
        item = item.fill(c.layer_02);
    }
    item
}

/// Linux has no tray: the window's way to stop Hex. Closing the window
/// keeps Hex dictating.
fn quit_item(p: Paint) -> El {
    let c = &p.k.c;
    row()
        .key("quit")
        .role("button")
        .label(tr("Quit Hex"))
        .h(28.0)
        .px(6.0)
        .mt(4.0)
        .center_y()
        .radius(p.k.radius.md)
        .hover(c.hover)
        .press(c.pressed)
        .on_press(|ui: &mut Ui| ui.quit())
        .child(
            text(tr("Quit Hex"))
                .key("label")
                .font(p.k.ty.ui)
                .color(c.text_muted)
                .truncate(),
        )
}

/// The title bar of the content panel, with the pane's actions on the right.
fn pane_header(p: Paint, title: &str, actions: Vec<El>) -> El {
    row()
        .key("pane-header")
        .h(52.0)
        .px(24.0)
        .gap(8.0)
        .center_y()
        .child(
            text(title)
                .key("pane-title")
                .font(p.k.ty.body_medium)
                .color(p.text())
                .grow(1.0)
                .truncate(),
        )
        .children(actions)
}

/// A vertically scrolling pane body with Settings' column inset.
fn scroll_body(key: &str, content: El) -> El {
    col()
        .key(key)
        .grow(1.0)
        .shrink()
        .clip_fade(0.0, 4.0, 0.0, 24.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(content)
}

// ---------------------------------------------------------------------------
// Fixtures for snapshots and tests

/// A named starting state for `--scene`.
type Scene = (&'static str, fn(&mut Ui));

const SCENES: &[Scene] = &[
    // The running Hex's real state, for headless snapshots and scripts
    // (`--snap`), read through `hex status`.
    ("live", |ui| {
        *ui = fixture(DesktopShortcuts::Registered);
        match crate::linux_service::request(crate::linux_service::Request::Status) {
            Ok(status) => {
                crate::desktop_i18n::apply(status.settings.ui_language.as_deref());
                ui.view = Some(ServiceView {
                    listener: status.desktop.listener,
                    operation_error: status.desktop.operation_error,
                    transcription: status.desktop.transcription,
                    update_status: status.desktop.update_status,
                    editing: status.editing,
                });
                ui.settings = status.settings;
                ui.modes.reload(&ui.settings);
            }
            Err(error) => {
                ui.view = None;
                ui.error = Some(error.to_string());
            }
        }
    }),
    ("portal", |ui| *ui = fixture(portal_shortcuts())),
    ("commands", |ui| *ui = fixture(DesktopShortcuts::Commands)),
    ("disabled", |ui| {
        *ui = fixture(portal_shortcuts());
        if let Some(view) = &mut ui.view {
            view.listener.running = false;
            view.listener.status = "Disabled".into();
        }
    }),
    ("capturing", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.capture = Capture::Listening {
            target: ShortcutTarget::Dictation,
            message: None,
        };
    }),
    ("chinese", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.settings.ui_language = Some("zh".into());
        crate::desktop_i18n::apply(Some("zh"));
    }),
    ("menus", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.open = Some(Menu::Microphone);
        ui.microphones = Ok(vec![
            "Built-in Audio Analog Stereo".into(),
            "Shure MV7".into(),
        ]);
        ui.confirm_release = true;
    }),
    ("voice-action", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.settings.voice_action.enabled = true;
        ui.pane = Pane::VoiceAction;
    }),
    ("voice-action-off", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.pane = Pane::VoiceAction;
    }),
    ("voice-action-unavailable", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.settings.voice_action.enabled = true;
        ui.models = models::Models::fixture_missing();
        ui.pane = Pane::VoiceAction;
    }),
    ("voice-action-models", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.settings.voice_action.enabled = true;
        ui.pane = Pane::VoiceAction;
        ui.open = Some(Menu::Model);
    }),
    ("history", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.history = history::HistoryPane::fixture();
        ui.pane = Pane::History;
    }),
    ("history-empty", |ui| {
        *ui = fixture(portal_shortcuts());
        ui.history = history::HistoryPane::fixture_empty();
        ui.pane = Pane::History;
    }),
    ("modes", |ui| {
        *ui = fixture(portal_shortcuts());
        modes::fixture(ui);
        ui.pane = Pane::Modes;
    }),
    ("modes-global", |ui| {
        *ui = fixture(portal_shortcuts());
        modes::fixture(ui);
        ui.modes.select(modes::Selection::Default);
        ui.pane = Pane::Modes;
    }),
    ("modes-picker", |ui| {
        *ui = fixture(portal_shortcuts());
        modes::fixture(ui);
        ui.modes.transformation_picker_open = true;
        ui.modes.application_picker_open = true;
        ui.pane = Pane::Modes;
    }),
    ("picker", |ui| {
        *ui = fixture(portal_shortcuts());
        picker::fixture(ui, None);
    }),
    ("picker-downloading", |ui| {
        *ui = fixture(portal_shortcuts());
        picker::fixture(ui, Some(TranscriptionModelId::CohereTranscribe));
    }),
    ("onboarding", |ui| {
        *ui = fixture(portal_shortcuts());
        onboarding::fixture(ui, crate::onboarding::Step::Welcome, false);
    }),
    ("onboarding-setup", |ui| {
        *ui = fixture(portal_shortcuts());
        onboarding::fixture(ui, crate::onboarding::Step::Setup, true);
    }),
];

fn portal_shortcuts() -> DesktopShortcuts {
    DesktopShortcuts::Portal(vec![
        DesktopGrantedShortcut {
            name: "Dictation".into(),
            trigger: Some("F12".into()),
        },
        DesktopGrantedShortcut {
            name: "Paste last dictation".into(),
            trigger: Some("Ctrl+Alt+V".into()),
        },
    ])
}

/// Deterministic settings and host state matching a typical Wayland
/// desktop with the shortcuts portal.
fn fixture(shortcuts: DesktopShortcuts) -> Ui {
    crate::desktop_i18n::apply(Some("en"));
    let settings = AppSettings {
        dictation_hotkey: HotkeyBinding {
            modifiers: HotkeyModifiers::default(),
            key: Some(HotkeyKey::named("f12")),
        },
        transcription: crate::transcription_models::TranscriptionSelection {
            model: TranscriptionModelId::ParakeetUnifiedEnglish,
            language: "en".into(),
            recognition_hints: String::new(),
        },
        paste_with_shift: true,
        ..AppSettings::default()
    };
    let mut ui = Ui::with(settings, Service::fixture(), None);
    ui.models = models::Models::fixture();
    ui.picker.installed = Some(vec![TranscriptionModelId::ParakeetUnifiedEnglish]);
    ui.view = Some(ServiceView {
        listener: DesktopListenerSnapshot {
            running: true,
            status: "Enabled".into(),
            warning: None,
            shortcuts,
        },
        operation_error: None,
        transcription: DesktopTranscriptionSnapshot {
            downloaded_bytes: 0,
            error: None,
            preparation_stage: None,
            selection: ui.settings.transcription.clone(),
            preparing: None,
        },
        update_status: DesktopUpdateStatus::Current,
        editing: false,
    });
    ui
}

#[cfg(test)]
mod tests;
