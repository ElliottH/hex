//! The experimental Rune front end for Linux Settings (`hex app --ui rune`).
//!
//! It is a client of the per-user `hex.service`, exactly like the GPUI
//! Settings process: it saves `settings.json`, asks the service to reload it,
//! and sends the same `linux_service::Request`s for listening, shortcuts,
//! models, and updates. It shows the Settings pane only; Modes, Voice Action,
//! and History stay in the GPUI window. It uses Rune's own design language
//! (`rune::tok`), not Hex's GPUI design tokens.

mod controls;

use std::sync::Arc;
use std::time::{Duration, Instant};

use rune::tok::{motion, space, type_};
use rune::{AppSpec, Cx, El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, Waker, col, row, text};

use crate::app_settings::{
    AppSettings, HotkeyBinding, HotkeyKey, HotkeyModifiers, IndicatorPosition, ModifierSide,
    RecordingAudioBehavior,
};
use crate::desktop_host::{
    DesktopAction, DesktopGrantedShortcut, DesktopListenerSnapshot, DesktopShortcut,
    DesktopShortcuts, DesktopTranscriptionSnapshot, DesktopUpdateStatus,
};
use crate::desktop_i18n::{tr, tr_fill};
use crate::dictation::ShortcutMode;
use crate::linux_input::LinuxHotkey;
use crate::linux_service::{Client, Request, ShortcutTarget};
use crate::transcription_models::{self, ModelPreparationStage, TranscriptionModelId};
use controls::{
    ButtonKind, DESCRIPTION, Paint, button, described_row, disclosure, keycaps, menu, menu_item,
    note, section, segmented, setting_row, status_dot, tag, text_field, toggle_row,
};

const SOUND_VOLUME_STEPS: [(&str, f32); 5] = [
    ("Off", 0.0),
    ("25%", 0.25),
    ("50%", 0.5),
    ("75%", 0.75),
    ("100%", 1.0),
];
/// How long a newly saved shortcut shows "Saved".
const SAVED_FOR: Duration = Duration::from_millis(700);
/// How long to wait for the service to start a requested shortcut edit.
const EDIT_START_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn run() {
    rune::set_app_id("hex");
    rune::run(spec());
}

fn spec() -> AppSpec<Ui> {
    AppSpec {
        title: "Hex",
        size: (720.0, 860.0),
        init: Ui::load,
        view,
        scenes: SCENES,
        fonts: &[],
        families: ("Inter", "Inter"),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
    }
}

/// Where Settings sends its requests.
enum Service {
    /// Not connected yet: `setup` connects unless a scene chose a fixture.
    Pending,
    Live(Box<Client>),
    /// Snapshots and tests: requests are recorded, never sent.
    Fixture(Vec<Request>),
}

/// What Settings shows of the service, from its latest snapshot.
#[derive(Clone, PartialEq)]
struct ServiceView {
    listener: DesktopListenerSnapshot,
    operation_error: Option<String>,
    transcription: DesktopTranscriptionSnapshot,
    update_status: DesktopUpdateStatus,
    /// The service is recording or saving a shortcut.
    editing: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Menu {
    Transcription,
    Microphone,
    Language,
}

#[derive(Clone, PartialEq, Debug)]
enum Capture {
    Idle,
    /// Waiting for the chord; `message` explains a rejected one.
    Listening {
        target: ShortcutTarget,
        message: Option<&'static str>,
    },
    /// Sent to the service, which validates, registers, and saves it.
    Saving {
        target: ShortcutTarget,
        since: Instant,
        seen_edit: bool,
    },
    Saved {
        target: ShortcutTarget,
        at: Instant,
    },
}

pub(crate) struct Ui {
    settings: AppSettings,
    service: Service,
    view: Option<ServiceView>,
    /// A save or policy change that failed here.
    error: Option<String>,
    /// Why the client cannot reach the service.
    connection_error: Option<String>,
    /// The settings the service last reported, to adopt only its changes.
    service_settings: Option<AppSettings>,
    open: Option<Menu>,
    microphones: Result<Vec<String>, String>,
    confirm_release: bool,
    capture: Capture,
    hints: TextEdit,
    /// The model the service was last seen preparing.
    preparing: Option<TranscriptionModelId>,
}

impl Ui {
    fn load() -> Self {
        let (settings, error) = match AppSettings::load() {
            Ok(settings) => (settings, None),
            Err(error) => (AppSettings::default(), Some(format!("{error:#}"))),
        };
        crate::desktop_i18n::apply(settings.ui_language.as_deref());
        Self::with(settings, Service::Pending, error)
    }

    fn with(settings: AppSettings, service: Service, error: Option<String>) -> Self {
        let hints = TextEdit::new(settings.transcription.recognition_hints.clone());
        Self {
            settings,
            service,
            view: None,
            error,
            connection_error: None,
            service_settings: None,
            open: None,
            microphones: Ok(Vec::new()),
            confirm_release: false,
            capture: Capture::Idle,
            hints,
            preparing: None,
        }
    }

    fn send(&mut self, request: Request) {
        match &mut self.service {
            Service::Live(client) => {
                if let Err(error) = client.send(request) {
                    self.error = Some(format!("{error:#}"));
                }
            }
            Service::Fixture(sent) => sent.push(request),
            Service::Pending => {}
        }
    }

    /// Saves `settings.json` before adopting the change, then asks the
    /// service to reload it. A failed save leaves the shown settings alone.
    fn update(&mut self, change: impl FnOnce(&mut AppSettings)) -> bool {
        let mut candidate = self.settings.clone();
        change(&mut candidate);
        self.commit(candidate)
    }

    fn commit(&mut self, candidate: AppSettings) -> bool {
        if matches!(self.service, Service::Live(_))
            && let Err(error) = candidate.save()
        {
            self.error = Some(format!("{error:#}"));
            return false;
        }
        self.error = None;
        self.settings = candidate;
        self.send(Request::ReloadSettings);
        true
    }

    fn running(&self) -> bool {
        self.view.as_ref().is_some_and(|view| view.listener.running)
    }

    fn shortcuts(&self) -> Option<&DesktopShortcuts> {
        self.view.as_ref().map(|view| &view.listener.shortcuts)
    }

    /// Without a shortcuts portal the desktop's keybindings own every
    /// shortcut, so Settings has none to record.
    fn records_shortcuts(&self) -> bool {
        self.shortcuts() != Some(&DesktopShortcuts::Commands)
    }

    fn adopt(&mut self, service: AppSettings) {
        if self.service_settings.as_ref() == Some(&service) {
            return;
        }
        if service != self.settings {
            if service.transcription.recognition_hints
                != self.settings.transcription.recognition_hints
            {
                self.hints = TextEdit::new(service.transcription.recognition_hints.clone());
            }
            self.settings = service.clone();
        }
        self.service_settings = Some(service);
    }
}

fn setup(ui: &mut Ui, waker: Waker) {
    if matches!(ui.service, Service::Pending) {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || waker());
        ui.service = Service::Live(Box::new(Client::with_waker(wake)));
    }
}

/// Runs before every frame: adopts service updates and advances shortcut
/// capture. True when anything shown changed.
fn poll(ui: &mut Ui, _clock: f64, _tx: &mut Tx) -> bool {
    let mut changed = advance_capture(ui);
    let Service::Live(client) = &mut ui.service else {
        return changed;
    };
    client.refresh();
    let error = client.error.clone();
    let snapshot = client.state.clone();
    let view = snapshot.as_ref().map(|state| ServiceView {
        listener: state.desktop.listener.clone(),
        operation_error: state.desktop.operation_error.clone(),
        transcription: state.desktop.transcription.clone(),
        update_status: state.desktop.update_status,
        editing: state.capturing || state.editing,
    });
    if error != ui.connection_error {
        ui.connection_error = error;
        changed = true;
    }
    if view != ui.view {
        ui.view = view;
        changed = true;
    }
    if let Some(state) = snapshot {
        let before = ui.settings.clone();
        ui.adopt(state.settings);
        changed |= before != ui.settings;
    }
    let preparing = ui
        .view
        .as_ref()
        .and_then(|view| view.transcription.preparing);
    if preparing != ui.preparing {
        ui.preparing = preparing;
        changed = true;
    }
    changed | advance_capture(ui)
}

/// Follows the service's side of a shortcut edit.
fn advance_capture(ui: &mut Ui) -> bool {
    let editing = ui.view.as_ref().is_some_and(|view| view.editing);
    match ui.capture.clone() {
        Capture::Saving {
            target,
            since,
            seen_edit,
        } => {
            let finished = if seen_edit {
                !editing
            } else if editing {
                ui.capture = Capture::Saving {
                    target,
                    since,
                    seen_edit: true,
                };
                return false;
            } else {
                since.elapsed() >= EDIT_START_TIMEOUT
            };
            if finished {
                ui.capture = Capture::Saved {
                    target,
                    at: Instant::now(),
                };
            }
            finished
        }
        Capture::Saved { at, .. } if at.elapsed() >= SAVED_FOR => {
            ui.capture = Capture::Idle;
            true
        }
        _ => false,
    }
}

/// Records a shortcut while capture listens; otherwise leaves keys alone.
fn on_key(ui: &mut Ui, event: &KeyEvent, _: &mut Tx) -> bool {
    let Capture::Listening { target, .. } = ui.capture else {
        return false;
    };
    if event.key == Key::Escape {
        cancel_capture(ui);
        return true;
    }
    let message = match recorded_binding(event) {
        Ok(binding) => match validate(&ui.settings, target, &binding) {
            Ok(()) => {
                ui.send(Request::SetShortcut {
                    target,
                    shortcut: desktop_shortcut(&binding),
                });
                if matches!(ui.service, Service::Fixture(_)) {
                    set_binding(&mut ui.settings, target, binding);
                }
                ui.capture = Capture::Saving {
                    target,
                    since: Instant::now(),
                    seen_edit: false,
                };
                return true;
            }
            Err(message) => message,
        },
        Err(message) => message,
    };
    ui.capture = Capture::Listening {
        target,
        message: Some(message),
    };
    true
}

fn begin_capture(ui: &mut Ui, target: ShortcutTarget) {
    if let Capture::Listening { target: active, .. } = ui.capture {
        cancel_capture(ui);
        if active == target {
            return;
        }
    }
    ui.send(Request::CaptureShortcutFor(target));
    ui.capture = Capture::Listening {
        target,
        message: None,
    };
}

fn cancel_capture(ui: &mut Ui) {
    if matches!(ui.capture, Capture::Listening { .. }) {
        ui.send(Request::CancelShortcut);
    }
    ui.capture = Capture::Idle;
}

/// The chord a key press records, in the key names Linux registers.
fn recorded_binding(event: &KeyEvent) -> Result<HotkeyBinding, &'static str> {
    let name = match &event.key {
        Key::Text(text) if text == " " => "space".to_string(),
        Key::Text(text) if text.chars().count() == 1 => text.to_lowercase(),
        Key::Command(letter) => letter.to_string(),
        Key::Function(number) => format!("f{number}"),
        Key::Enter => "enter".into(),
        Key::Tab => "tab".into(),
        Key::Backspace => "backspace".into(),
        Key::Delete => "delete".into(),
        Key::Left => "left".into(),
        Key::Right => "right".into(),
        Key::Up => "up".into(),
        Key::Down => "down".into(),
        Key::Home => "home".into(),
        Key::End => "end".into(),
        _ => return Err(tr("Unsupported key")),
    };
    let held = |down: bool| down.then_some(ModifierSide::Either);
    let modifiers = HotkeyModifiers {
        control: held(event.mods.ctrl),
        option: held(event.mods.alt),
        shift: held(event.mods.shift),
        command: held(event.mods.logo),
        function: false,
    };
    if modifiers.is_empty() && !matches!(event.key, Key::Function(_)) {
        return Err(tr("Add a modifier"));
    }
    Ok(HotkeyBinding {
        modifiers,
        key: Some(HotkeyKey::named(&name)),
    })
}

fn validate(
    settings: &AppSettings,
    target: ShortcutTarget,
    binding: &HotkeyBinding,
) -> Result<(), &'static str> {
    LinuxHotkey::from(binding)
        .validate()
        .map_err(|_| tr("Add a key Linux can register"))?;
    let mut others = Vec::new();
    if target != ShortcutTarget::Dictation {
        others.push(settings.dictation_hotkey.clone());
    }
    if target != ShortcutTarget::VoiceAction && settings.voice_action.enabled {
        others.push(settings.edit_hotkey.clone());
    }
    if target != ShortcutTarget::PasteLast
        && let Some(paste) = &settings.paste_last_hotkey
    {
        others.push(paste.clone());
    }
    if crate::app_settings::hotkey_conflicts(binding, others) {
        return Err(tr("Already in use"));
    }
    Ok(())
}

fn desktop_shortcut(binding: &HotkeyBinding) -> DesktopShortcut {
    let modifiers = binding.modifiers;
    DesktopShortcut {
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
    }
}

fn set_binding(settings: &mut AppSettings, target: ShortcutTarget, binding: HotkeyBinding) {
    match target {
        ShortcutTarget::Dictation => settings.dictation_hotkey = binding,
        ShortcutTarget::PasteLast => settings.paste_last_hotkey = Some(binding),
        ShortcutTarget::VoiceAction => settings.edit_hotkey = binding,
    }
}

fn sound_volume_index(settings: &AppSettings) -> usize {
    if !settings.sound_effects {
        return 0;
    }
    SOUND_VOLUME_STEPS[1..]
        .iter()
        .enumerate()
        .min_by(|(_, (_, left)), (_, (_, right))| {
            (settings.sound_effect_volume - left)
                .abs()
                .total_cmp(&(settings.sound_effect_volume - right).abs())
        })
        .map_or(0, |(index, _)| index + 1)
}

fn toggle_menu(ui: &mut Ui, menu: Menu) {
    ui.open = (ui.open != Some(menu)).then_some(menu);
    if ui.open == Some(Menu::Microphone) && !matches!(ui.service, Service::Fixture(_)) {
        ui.microphones = crate::audio::input_device_names().map_err(|error| error.to_string());
    }
}

// ---------------------------------------------------------------------------
// View

fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let dark = cx.system_dark();
    let p = Paint::new(dark, cx.env().scale);
    let mut content = col()
        .key("content")
        .px(space::S6)
        .pt(space::S2)
        .pb(space::S6)
        .gap(space::S6);
    content = content.child(listener_card(ui, p));
    if let Some(shortcuts) = desktop_shortcuts(ui, p) {
        content = content.child(shortcuts);
    }
    content = content
        .child(dictation_section(ui, p))
        .child(behavior_section(ui, p))
        .child(application_section(ui, p));
    col()
        .key("window")
        .role("window")
        .fill(p.t.bg)
        .child(
            row().key("titlebar").h(44.0).w_full().center().child(
                text(tr("Settings"))
                    .key("title")
                    .font(type_::TITLE)
                    .color(p.t.text),
            ),
        )
        .child(
            col()
                .key("scroll")
                .grow(1.0)
                .shrink()
                .clip_fade(0.0, space::S1, 0.0, space::S6)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(content),
        )
}

/// Whether Hex listens, how to dictate, and the one way to change it.
fn listener_card(ui: &Ui, p: Paint) -> El {
    let running = ui.running();
    let status = match &ui.view {
        Some(view) => tr(&view.listener.status).to_string(),
        None if matches!(ui.service, Service::Fixture(_)) => tr("Service disconnected").to_string(),
        None if ui.connection_error.is_some() => tr("Service disconnected").to_string(),
        None => tr("Connecting…").to_string(),
    };
    let shortcut = ui.settings.dictation_hotkey.keycaps().join("+");
    let hint = match (running, ui.settings.shortcut_mode) {
        _ if !ui.records_shortcuts() => {
            tr("Ready anywhere · Use your desktop's shortcuts to dictate").to_string()
        }
        (true, ShortcutMode::Toggle) => tr_fill(
            "Ready anywhere · Press {} to start and stop dictating",
            &shortcut,
        ),
        (true, ShortcutMode::Hold) => tr_fill("Ready anywhere · Hold {} to dictate", &shortcut),
        (false, _) => tr_fill("Enable Hex to dictate into any app with {}", &shortcut),
    };
    let failure = ui
        .view
        .as_ref()
        .and_then(|view| view.listener.warning.clone())
        .map(|failure| tr_fill("Last dictation failed: {}", &failure));
    let error = ui
        .error
        .clone()
        .or_else(|| ui.connection_error.clone())
        .or_else(|| {
            ui.view
                .as_ref()
                .and_then(|view| view.operation_error.clone())
        });
    let action = if running {
        button(
            p,
            "toggle-listening",
            tr("Disable"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| ui.send(Request::Desktop(DesktopAction::StopListening)),
        )
    } else {
        button(
            p,
            "toggle-listening",
            tr("Enable"),
            ButtonKind::Primary,
            |ui: &mut Ui, _| ui.send(Request::Desktop(DesktopAction::StartListening)),
        )
    };
    let summary = row()
        .key("listener")
        .min_h(64.0)
        .px(space::S3)
        .py(space::S2)
        .gap(space::S3)
        .center_y()
        .child(status_dot(if running {
            p.success
        } else {
            p.t.text_secondary
        }))
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(
                    text(status)
                        .key("status")
                        .font(type_::TITLE)
                        .color(p.t.text),
                )
                .child(
                    text(hint)
                        .key("hint")
                        .font(DESCRIPTION)
                        .color(p.t.text_secondary)
                        .wrap(),
                )
                .children(failure.map(|failure| {
                    text(failure)
                        .key("failure")
                        .font(DESCRIPTION)
                        .color(p.danger)
                        .wrap()
                })),
        )
        .child(action);
    let mut rows = vec![summary];
    if let Some(error) = error {
        rows.push(note(
            "error-title",
            tr("Hex could not apply this change."),
            p.danger,
        ));
        rows.push(note("error", &error, p.t.text_secondary));
    }
    section(p, "listening", tr("Status"), rows)
}

/// What the desktop granted (portal) or which commands to bind (none).
fn desktop_shortcuts(ui: &Ui, p: Paint) -> Option<El> {
    let rows = match ui.shortcuts()? {
        DesktopShortcuts::Registered => return None,
        DesktopShortcuts::Portal(granted) => {
            let mut rows: Vec<El> = granted
                .iter()
                .enumerate()
                .map(|(index, shortcut)| granted_row(p, index, shortcut))
                .collect();
            if granted.iter().any(|shortcut| shortcut.trigger.is_none()) {
                rows.push(note(
                    "refused",
                    tr("Your desktop did not grant every shortcut. Record another one, or free it in your desktop's keyboard settings."),
                    p.danger,
                ));
            }
            rows
        }
        DesktopShortcuts::Commands => {
            let mut rows = vec![
                note(
                    "no-portal",
                    tr(
                        "Your desktop has no shortcuts portal. Bind these commands in its keyboard settings.",
                    ),
                    p.t.text_secondary,
                ),
                command_row(
                    p,
                    "hold",
                    tr("Hold to dictate"),
                    tr("Bind start to the key press and stop to its release"),
                    &["hex dictation start", "hex dictation stop"],
                ),
                command_row(
                    p,
                    "toggle",
                    tr("Toggle dictation"),
                    tr("Press to start dictating, press again to transcribe"),
                    &["hex dictation toggle"],
                ),
                command_row(
                    p,
                    "cancel",
                    tr("Cancel dictation"),
                    tr("Discard the current recording"),
                    &["hex dictation cancel"],
                ),
                command_row(
                    p,
                    "paste-last",
                    tr("Paste last dictation"),
                    tr("Re-paste the most recent dictation"),
                    &["hex paste-last"],
                ),
            ];
            if ui.settings.voice_action.enabled {
                rows.push(command_row(
                    p,
                    "voice-action",
                    tr("Voice Action"),
                    tr("Hold to speak; selected text is included automatically"),
                    &["hex voice-action start", "hex voice-action stop"],
                ));
            }
            rows
        }
    };
    Some(section(
        p,
        "desktop-shortcuts",
        tr("Desktop shortcuts"),
        rows,
    ))
}

fn granted_row(p: Paint, index: usize, shortcut: &DesktopGrantedShortcut) -> El {
    let title = match shortcut.name.as_str() {
        "Paste last dictation" => tr("Paste last dictation"),
        "Voice Action" => tr("Voice Action"),
        _ => tr("Dictation"),
    };
    let value = match &shortcut.trigger {
        Some(trigger) => text(trigger.clone())
            .key("trigger")
            .font(type_::BODY)
            .color(p.t.text_secondary),
        None => text(tr("Not granted"))
            .key("trigger")
            .font(type_::BODY)
            .color(p.danger),
    };
    setting_row(
        p,
        &format!("granted{index}"),
        title,
        Some(tr("Granted by your desktop")),
        value,
    )
}

fn command_row(p: Paint, key: &str, title: &str, description: &str, commands: &[&str]) -> El {
    let lines = col()
        .key("commands")
        .gap(2.0)
        .children(commands.iter().enumerate().map(|(index, command)| {
            text(*command)
                .key(format!("command{index}"))
                .font(type_::MONO_CAPTION.mono())
                .color(p.t.text_secondary)
                .align(rune::Align::End)
        }));
    setting_row(p, key, title, Some(description), lines)
}

fn dictation_section(ui: &Ui, p: Paint) -> El {
    let selection = &ui.settings.transcription;
    let model = transcription_models::definition(selection.model);
    let label = format!(
        "{} · {}",
        transcription_models::language_name(&selection.language),
        model.name
    );
    let mut rows = vec![described_row(
        p,
        "transcription",
        tr("Local transcription"),
        Some(transcription_status(ui, p)),
        disclosure(
            p,
            "transcription-control",
            &label,
            ui.open == Some(Menu::Transcription),
            |ui: &mut Ui| toggle_menu(ui, Menu::Transcription),
        ),
    )];
    if ui.open == Some(Menu::Transcription) {
        let language = selection.language.clone();
        let items = transcription_models::choices_for_runtime(&language)
            .into_iter()
            .map(|choice| {
                let id = choice.model.id;
                let language = language.clone();
                menu_item(
                    p,
                    choice.model.id.as_str(),
                    choice.model.name,
                    Some(tr(choice.recommendation.label())),
                    id == selection.model,
                    move |ui: &mut Ui| {
                        ui.open = None;
                        if ui.settings.transcription.model != id {
                            ui.send(Request::ChooseModel {
                                model: id,
                                language: language.clone(),
                            });
                        }
                    },
                )
            })
            .collect();
        rows.push(menu(p, "transcription-menu", items));
    }
    let microphone = ui
        .settings
        .microphone
        .clone()
        .unwrap_or_else(|| tr("Automatic").into());
    rows.push(setting_row(
        p,
        "microphone",
        tr("Microphone"),
        Some(tr("Input device for dictation and commands")),
        disclosure(
            p,
            "microphone-control",
            &microphone,
            ui.open == Some(Menu::Microphone),
            |ui: &mut Ui| toggle_menu(ui, Menu::Microphone),
        ),
    ));
    if ui.open == Some(Menu::Microphone) {
        let mut items = vec![menu_item(
            p,
            "automatic",
            tr("Automatic"),
            None,
            ui.settings.microphone.is_none(),
            |ui: &mut Ui| {
                ui.open = None;
                ui.update(|settings| settings.microphone = None);
            },
        )];
        match &ui.microphones {
            Ok(devices) => items.extend(devices.iter().enumerate().map(|(index, device)| {
                let name = device.clone();
                menu_item(
                    p,
                    &format!("device{index}"),
                    device,
                    None,
                    ui.settings.microphone.as_deref() == Some(device),
                    move |ui: &mut Ui| {
                        ui.open = None;
                        let name = name.clone();
                        ui.update(|settings| settings.microphone = Some(name));
                    },
                )
            })),
            Err(error) => items.push(note("devices-error", error, p.danger)),
        }
        rows.push(menu(p, "microphone-menu", items));
    }
    if model.supports_recognition_hints {
        rows.push(setting_row(
            p,
            "hints",
            tr("Recognition hints"),
            Some(tr("Names and terms to prime the speech model")),
            text_field(
                p,
                "hints-field",
                &ui.hints,
                tr("Names and terms Whisper should expect, e.g. OpenCode, Effect…"),
                |ui: &mut Ui| &mut ui.hints,
                |ui: &mut Ui, _| {
                    let hints = ui.hints.text.clone();
                    ui.update(|settings| settings.transcription.recognition_hints = hints);
                },
            ),
        ));
    }
    let toggle_mode = ui.settings.shortcut_mode == ShortcutMode::Toggle;
    rows.push(setting_row(
        p,
        "shortcut-mode",
        tr("Shortcut mode"),
        Some(if toggle_mode {
            tr("Press to start dictating, press again to transcribe")
        } else {
            tr("Hold to dictate, release to transcribe")
        }),
        segmented(
            p,
            "shortcut-mode-control",
            &[tr("Hold"), tr("Toggle")],
            usize::from(toggle_mode),
            |ui, index| {
                let mode = if index == 1 {
                    ShortcutMode::Toggle
                } else {
                    ShortcutMode::Hold
                };
                if ui.settings.shortcut_mode != mode {
                    ui.update(|settings| settings.shortcut_mode = mode);
                }
            },
        ),
    ));
    if ui.records_shortcuts() {
        rows.push(setting_row(
            p,
            "dictation-shortcut",
            tr("Dictation shortcut"),
            Some(tr("Click to record a new shortcut")),
            hotkey_control(ui, p, ShortcutTarget::Dictation),
        ));
    }
    section(p, "dictation", tr("Dictation"), rows)
}

/// The transcription row's description: preparation progress or its error.
fn transcription_status(ui: &Ui, p: Paint) -> (String, rune::Color) {
    let fallback = (
        tr("Language and on-device speech model").to_string(),
        p.t.text_secondary,
    );
    let Some(view) = &ui.view else {
        return fallback;
    };
    let transcription = &view.transcription;
    if let Some(error) = &transcription.error {
        return (error.clone(), p.danger);
    }
    let Some(model) = transcription.preparing else {
        return fallback;
    };
    let name = transcription_models::definition(model).name;
    let stage = match transcription.preparation_stage {
        Some(ModelPreparationStage::Verifying) => tr("Verifying model").to_string(),
        Some(ModelPreparationStage::Loading) => tr("Loading model").to_string(),
        _ => format!(
            "{} {:.0} MB",
            tr("Downloading"),
            transcription.downloaded_bytes as f64 / 1_000_000.0
        ),
    };
    (format!("{name} · {stage}"), p.accent)
}

fn hotkey_control(ui: &Ui, p: Paint, target: ShortcutTarget) -> El {
    let binding = match target {
        ShortcutTarget::Dictation => Some(&ui.settings.dictation_hotkey),
        ShortcutTarget::PasteLast => ui.settings.paste_last_hotkey.as_ref(),
        ShortcutTarget::VoiceAction => Some(&ui.settings.edit_hotkey),
    };
    let caps = binding.map_or_else(|| vec![tr("Off").to_string()], HotkeyBinding::keycaps);
    let key = match target {
        ShortcutTarget::Dictation => "dictation-hotkey",
        ShortcutTarget::PasteLast => "paste-last-hotkey",
        ShortcutTarget::VoiceAction => "voice-action-hotkey",
    };
    let frame = row()
        .key(key)
        .role("button")
        .h(30.0)
        .pl(4.0)
        .pr(space::S3)
        .gap(space::S2)
        .center_y()
        .radius(rune::tok::radius::MD)
        .motion(motion::SNAPPY)
        .on_press(move |ui: &mut Ui| begin_capture(ui, target));
    match &ui.capture {
        Capture::Listening {
            target: active,
            message,
        } if *active == target => frame
            .fill(p.danger.alpha(0.14))
            .border(p.hair, p.danger.alpha(0.5))
            .child(status_dot(p.danger).ml(space::S2))
            .child(
                text(message.unwrap_or(tr("Press shortcut")))
                    .key("prompt")
                    .font(type_::LABEL)
                    .color(p.t.text),
            )
            .child(
                text(tr("Esc to cancel"))
                    .key("cancel")
                    .font(DESCRIPTION)
                    .color(p.t.text_secondary),
            ),
        Capture::Saving { target: active, .. } if *active == target => frame
            .fill(p.t.control_track)
            .child(keycaps(p, "keys", &caps))
            .child(
                text(tr("Saving…"))
                    .key("label")
                    .font(type_::LABEL)
                    .color(p.t.text_secondary),
            ),
        Capture::Saved { target: active, .. } if *active == target => frame
            .fill(p.t.control_track)
            .child(keycaps(p, "keys", &caps))
            .child(tag("saved", tr("Saved"), p.success)),
        _ => frame
            .fill(p.t.control_track)
            .hover(p.t.hover)
            .press(p.t.press)
            .child(keycaps(p, "keys", &caps))
            .child(
                text(tr("Change shortcut"))
                    .key("label")
                    .font(type_::LABEL)
                    .color(p.t.text_secondary),
            ),
    }
}

fn behavior_section(ui: &Ui, p: Paint) -> El {
    let settings = &ui.settings;
    let audio_labels: Vec<&str> = RecordingAudioBehavior::ALL
        .iter()
        .map(|behavior| tr(behavior.label()))
        .collect();
    let audio_index = RecordingAudioBehavior::ALL
        .iter()
        .position(|behavior| *behavior == settings.recording_audio_behavior)
        .unwrap_or(2);
    let mut rows = vec![
        setting_row(
            p,
            "while-dictating",
            tr("While dictating"),
            Some(tr("Mute or pause other audio during dictation")),
            segmented(
                p,
                "while-dictating-control",
                &audio_labels,
                audio_index,
                |ui, index| {
                    let behavior = RecordingAudioBehavior::ALL[index];
                    if ui.settings.recording_audio_behavior != behavior {
                        ui.update(|settings| settings.recording_audio_behavior = behavior);
                    }
                },
            ),
        ),
        setting_row(
            p,
            "microphone-mode",
            tr("Microphone mode"),
            Some(if settings.release_microphone_while_idle {
                tr("Opens only while holding the shortcut. Disables pre-roll and commands.")
            } else {
                tr("Keeps the microphone open for instant start and pre-roll")
            }),
            segmented(
                p,
                "microphone-mode-control",
                &[tr("Keep ready (fast)"), tr("Release when idle")],
                usize::from(settings.release_microphone_while_idle),
                |ui, index| {
                    let release = index == 1;
                    if release && !ui.settings.release_microphone_while_idle {
                        ui.confirm_release = true;
                        return;
                    }
                    ui.confirm_release = false;
                    if ui.settings.release_microphone_while_idle != release {
                        let mut candidate = ui.settings.clone();
                        match candidate.set_release_microphone_while_idle(release) {
                            Ok(()) => {
                                ui.commit(candidate);
                            }
                            Err(error) => ui.error = Some(format!("{error:#}")),
                        }
                    }
                },
            ),
        ),
    ];
    if ui.confirm_release {
        rows.push(release_confirmation(ui, p));
    }
    let toggle_mode = settings.shortcut_mode == ShortcutMode::Toggle;
    if !toggle_mode {
        rows.push(toggle_row(
            p,
            "double-tap",
            tr("Double-tap to lock"),
            tr("Double-tap the shortcut for hands-free dictation"),
            settings.double_tap_lock,
            |ui: &mut Ui| {
                let enabled = !ui.settings.double_tap_lock;
                ui.update(|settings| {
                    settings.double_tap_lock = enabled;
                    if !enabled {
                        settings.double_tap_only = false;
                    }
                });
            },
        ));
        if settings.double_tap_lock && settings.dictation_hotkey.key.is_some() {
            rows.push(toggle_row(
                p,
                "double-tap-only",
                tr("Double-tap only"),
                tr("Wait for two complete shortcut taps before recording"),
                settings.double_tap_only,
                |ui: &mut Ui| {
                    let enabled = !ui.settings.double_tap_only;
                    ui.update(|settings| settings.double_tap_only = enabled);
                },
            ));
        }
    }
    let indicator_index = match settings.indicator_position {
        IndicatorPosition::Top => 0,
        IndicatorPosition::Bottom => 1,
        IndicatorPosition::Hidden => 2,
    };
    rows.push(setting_row(
        p,
        "indicator",
        tr("Recording indicator"),
        Some(tr(
            "Show the dictation pill at the top or bottom of the screen",
        )),
        segmented(
            p,
            "indicator-control",
            &[tr("Top"), tr("Bottom"), tr("Off")],
            indicator_index,
            |ui, index| {
                let position = [
                    IndicatorPosition::Top,
                    IndicatorPosition::Bottom,
                    IndicatorPosition::Hidden,
                ][index];
                if ui.settings.indicator_position != position {
                    ui.update(|settings| settings.indicator_position = position);
                }
            },
        ),
    ));
    rows.push(toggle_row(
        p,
        "terminal-paste",
        tr("Terminal paste"),
        tr("Paste with Ctrl+Shift+V so terminals accept it"),
        settings.paste_with_shift,
        |ui: &mut Ui| {
            let enabled = !ui.settings.paste_with_shift;
            ui.update(|settings| settings.paste_with_shift = enabled);
        },
    ));
    if ui.records_shortcuts() {
        let mut control = row()
            .key("paste-last-control")
            .gap(space::S2)
            .center_y()
            .child(hotkey_control(ui, p, ShortcutTarget::PasteLast));
        if settings.paste_last_hotkey.is_some() {
            control = control.child(button(
                p,
                "clear-paste-last",
                tr("Clear"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| {
                    ui.update(|settings| settings.paste_last_hotkey = None);
                },
            ));
        }
        rows.push(setting_row(
            p,
            "paste-last",
            tr("Paste last dictation"),
            Some(tr("Re-paste the most recent dictation")),
            control,
        ));
    }
    section(p, "behavior", tr("Behavior"), rows)
}

fn release_confirmation(ui: &Ui, p: Paint) -> El {
    let description = if ui.settings.commands_enabled {
        tr("Opens the microphone on shortcut press without pre-roll and turns Commands off.")
    } else {
        tr("Opens the microphone on shortcut press without pre-roll. Commands remain off.")
    };
    setting_row(
        p,
        "confirm-release",
        tr("Release the microphone when idle?"),
        Some(description),
        row()
            .key("actions")
            .gap(space::S2)
            .child(button(
                p,
                "cancel-release",
                tr("Cancel"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| {
                    ui.confirm_release = false;
                },
            ))
            .child(button(
                p,
                "confirm-release",
                tr("Release microphone"),
                ButtonKind::Primary,
                |ui: &mut Ui, _| {
                    let mut candidate = ui.settings.clone();
                    candidate.disable_commands_and_release_microphone();
                    if ui.commit(candidate) {
                        ui.confirm_release = false;
                    }
                },
            )),
    )
    .fill(p.t.hover)
}

fn application_section(ui: &Ui, p: Paint) -> El {
    let settings = &ui.settings;
    let mut rows = vec![toggle_row(
        p,
        "enable-on-launch",
        tr("Enable on launch"),
        tr("Turn Hex on in every app whenever it opens"),
        settings.enable_on_launch,
        |ui: &mut Ui| {
            let enabled = !ui.settings.enable_on_launch;
            ui.update(|settings| settings.enable_on_launch = enabled);
        },
    )];
    let language = tr(crate::desktop_i18n::choice_name(
        settings.ui_language.as_deref(),
    ));
    rows.push(setting_row(
        p,
        "interface-language",
        tr("Interface language"),
        Some(tr("Language of the Hex interface")),
        disclosure(
            p,
            "interface-language-control",
            language,
            ui.open == Some(Menu::Language),
            |ui: &mut Ui| toggle_menu(ui, Menu::Language),
        ),
    ));
    if ui.open == Some(Menu::Language) {
        let items = crate::desktop_i18n::LANGUAGE_CHOICES
            .iter()
            .enumerate()
            .map(|(index, (code, name))| {
                let code = code.map(str::to_string);
                let selected = settings.ui_language == code;
                menu_item(
                    p,
                    &format!("language{index}"),
                    tr(name),
                    None,
                    selected,
                    move |ui: &mut Ui| {
                        ui.open = None;
                        let code = code.clone();
                        if ui.update(|settings| settings.ui_language = code.clone()) {
                            crate::desktop_i18n::apply(code.as_deref());
                        }
                    },
                )
            })
            .collect();
        rows.push(menu(p, "language-menu", items));
    }
    let volume_labels: Vec<&str> = SOUND_VOLUME_STEPS
        .iter()
        .map(|(label, _)| tr(label))
        .collect();
    rows.push(setting_row(
        p,
        "sound-volume",
        tr("Sound volume"),
        Some(tr("Recording, cancellation, and error tones")),
        segmented(
            p,
            "sound-volume-control",
            &volume_labels,
            sound_volume_index(settings),
            |ui, index| {
                let volume = SOUND_VOLUME_STEPS[index].1;
                let saved = ui.update(|settings| {
                    settings.sound_effects = volume > 0.0;
                    if volume > 0.0 {
                        settings.sound_effect_volume = volume;
                    }
                });
                if saved && volume > 0.0 && matches!(ui.service, Service::Live(_)) {
                    crate::feedback::play(crate::feedback::Tone::DictationStart);
                }
            },
        ),
    ));
    if let Some(control) = software_update(ui, p) {
        rows.push(setting_row(
            p,
            "software-updates",
            tr("Software updates"),
            Some(tr("Checks for new Hex releases in the background")),
            control,
        ));
    }
    section(p, "application", tr("Application"), rows)
}

fn software_update(ui: &Ui, p: Paint) -> Option<El> {
    let status = |label: &str| {
        text(label)
            .key("update-status")
            .font(DESCRIPTION)
            .color(p.t.text_secondary)
    };
    Some(match ui.view.as_ref()?.update_status {
        DesktopUpdateStatus::Unavailable => return None,
        DesktopUpdateStatus::Checking => status(tr("Checking")),
        DesktopUpdateStatus::Current => status(tr("Up to date")),
        DesktopUpdateStatus::Failed => status(tr("Check failed; retrying later")),
        DesktopUpdateStatus::ReadyToRestart => button(
            p,
            "restart-update",
            tr("Restart to update"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| ui.send(Request::Desktop(DesktopAction::RestartIntoUpdate)),
        ),
    })
}

// ---------------------------------------------------------------------------
// Fixtures for snapshots and tests

/// A named starting state for `--scene`.
type Scene = (&'static str, fn(&mut Ui));

const SCENES: &[Scene] = &[
    // The running service's real state, for headless snapshots and scripts
    // (`--snap`): waits for its first snapshot instead of drawing
    // "Connecting…" at virtual time zero.
    ("live", |ui| {
        *ui = Ui::load();
        let mut client = Client::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.state.is_none() && client.error.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
            client.refresh();
        }
        ui.service = Service::Live(Box::new(client));
        poll(ui, 0.0, &mut Tx::default());
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

/// Deterministic settings and service state matching a typical Wayland
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
    let mut ui = Ui::with(settings, Service::Fixture(Vec::new()), None);
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
