//! First-run setup on Linux, driven by the same `onboarding::Flow` as GPUI's
//! tour. Linux has no privacy permissions and this window cannot observe a
//! practice dictation, so the steps are Welcome, the speech model download
//! (through the service), and Done. It shows until the selected model is
//! installed and completion is recorded, or when a relaunch resumes it.

use std::time::{Duration, Instant};

use rune::{El, col, el, row, text};
use rune_kit::Progress;

use super::Ui;
use super::controls::{ButtonKind, DESCRIPTION, Paint, button, card, keycaps, setting_row};
use crate::app_settings::AppSettings;
use crate::desktop_i18n::{tr, tr_fill};
use crate::linux_app::HostRequest;
use crate::onboarding::{Effect, Flow, Input, ItemStatus, ModelProgress, SetupItem, Step, View};

const WIDTH: f32 = 520.0;
/// How long the ready check shows before setup moves on.
const ADVANCE_BEAT: Duration = Duration::from_millis(700);

pub(super) struct Tour {
    pub(super) flow: Flow,
    advance_at: Option<Instant>,
    completed: bool,
}

impl Tour {
    /// The tour when this run still needs it.
    pub(super) fn start(settings: &AppSettings) -> Option<Tour> {
        let installed = selected_installed(settings, None);
        let status = crate::onboarding::status_with_transcription_model(installed);
        let completed = crate::onboarding::completion_recorded();
        let resume = crate::onboarding::take_resume();
        (resume.is_some() || (!completed && !status.ready())).then(|| Tour {
            flow: Flow::new(resume.unwrap_or(Step::Welcome), status, false, false),
            advance_at: None,
            completed,
        })
    }
}

fn selected_installed(
    settings: &AppSettings,
    fixture: Option<&Vec<crate::transcription_models::TranscriptionModelId>>,
) -> bool {
    crate::transcription_models::validate(&settings.transcription).is_ok_and(
        |model| match fixture {
            Some(installed) => installed.contains(&model.id),
            None => {
                crate::transcription_models::is_installed(model, &settings.transcription.language)
            }
        },
    )
}

fn progress(ui: &Ui) -> (bool, ModelProgress) {
    let installed = selected_installed(&ui.settings, ui.picker.installed.as_ref());
    let selected = crate::transcription_models::validate(&ui.settings.transcription).ok();
    let transcription = ui.view.as_ref().map(|view| &view.transcription);
    let downloading = selected.and_then(|model| {
        (transcription.and_then(|t| t.preparing) == Some(model.id)).then(|| {
            model.download_bytes().map_or(0.0, |bytes| {
                (transcription.map_or(0, |t| t.downloaded_bytes) as f32 / bytes as f32)
                    .clamp(0.0, 1.0)
            })
        })
    });
    (
        installed,
        ModelProgress {
            installed,
            downloading,
            failed: transcription.is_some_and(|t| t.error.is_some()),
        },
    )
}

/// Feeds the flow fresh model state; true when the tour changed.
pub(super) fn poll(ui: &mut Ui) -> bool {
    let Some(tour) = &ui.tour else {
        return false;
    };
    let before = tour.flow.view();
    let (installed, model) = progress(ui);
    let status = crate::onboarding::status_with_transcription_model(installed);
    let live = ui.live();
    let tour = ui.tour.as_mut().expect("checked above");
    let mut effect = tour.flow.on(Input::Status(status, model));
    if tour
        .advance_at
        .is_some_and(|at| at.elapsed() >= ADVANCE_BEAT)
    {
        tour.advance_at = None;
        if tour.flow.step() == Step::Setup {
            effect = tour.flow.on(Input::Continue);
        }
    }
    if status.ready() && !tour.completed {
        tour.completed = true;
        if live && let Err(error) = crate::onboarding::record_completion() {
            tracing::warn!(%error, "could not record onboarding completion");
        }
    }
    let changed = before != tour.flow.view();
    apply(ui, effect);
    changed || ui.tour.is_none()
}

fn input(ui: &mut Ui, input: Input) {
    let Some(tour) = &mut ui.tour else {
        return;
    };
    let effect = tour.flow.on(input);
    apply(ui, effect);
}

fn apply(ui: &mut Ui, effect: Effect) {
    match effect {
        Effect::DownloadModel => {
            if crate::transcription_models::validate(&ui.settings.transcription).is_ok() {
                let selection = ui.settings.transcription.clone();
                ui.send(HostRequest::ChooseModel {
                    model: selection.model,
                    language: selection.language,
                });
            }
        }
        Effect::AdvanceSoon => {
            if let Some(tour) = &mut ui.tour {
                tour.advance_at = Some(Instant::now());
            }
        }
        Effect::Finish => ui.tour = None,
        // No permissions, relaunches, or practice on Linux.
        Effect::None | Effect::RequestMicrophone | Effect::ShowGuide(_) | Effect::Relaunch => {}
    }
}

/// The tour over the whole window, while it shows.
pub(super) fn overlay(ui: &Ui, p: Paint) -> Option<El> {
    let tour = ui.tour.as_ref()?;
    let view = tour.flow.view();
    let page = match view.step {
        Step::Welcome => welcome(p),
        Step::Setup => setup(ui, p, &view),
        Step::TryIt | Step::Done => done(ui, p),
    };
    let (label, enabled) = match view.step {
        Step::Welcome => (tr("Get started"), true),
        Step::Setup => (tr("Continue"), view.ready),
        Step::TryIt | Step::Done => (tr("Start using Hex"), true),
    };
    let primary = if enabled {
        button(
            p,
            "tour-continue",
            label,
            ButtonKind::Primary,
            |ui: &mut Ui, _| input(ui, Input::Continue),
        )
    } else {
        rune_kit::Button::new(label)
            .disabled(true)
            .build(p.k)
            .key("tour-continue")
    };
    let mut footer = row().key("tour-footer").gap(8.0).center();
    if view.can_go_back {
        footer = footer.child(button(
            p,
            "tour-back",
            tr("Back"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| input(ui, Input::Back),
        ));
    }
    footer = footer.child(primary);
    let dots = step_dots(p, &view);
    Some(
        col()
            .key("tour")
            .role("dialog")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(p.k.c.bg_deep)
            .on_press(|_: &mut Ui| {})
            .child(
                col()
                    .key("tour-page")
                    .grow(1.0)
                    .shrink()
                    .center()
                    .child(col().key("tour-column").w(WIDTH).gap(24.0).child(page)),
            )
            .child(
                col()
                    .key("tour-bottom")
                    .pb(32.0)
                    .gap(20.0)
                    .center()
                    .child(footer)
                    .children(dots),
            ),
    )
}

fn step_dots(p: Paint, view: &View) -> Option<El> {
    if view.step == Step::Done || view.count < 2 {
        return None;
    }
    let mut dots = row().key("step-dots").gap(6.0).center_y();
    for index in 0..view.count {
        let current = index == view.index;
        dots = dots.child(
            el().key(format!("dot{index}"))
                .size(if current { 16.0 } else { 6.0 }, 6.0)
                .radius(3.0)
                .fill(if current { p.text() } else { p.k.c.border_base }),
        );
    }
    Some(dots)
}

fn title(p: Paint, title: &str, body: Option<String>) -> El {
    col()
        .key("title-block")
        .gap(8.0)
        .center()
        .child(
            text(title)
                .key("title")
                .font(p.k.ty.body_medium.lh(28.0))
                .color(p.text()),
        )
        .children(body.map(|body| {
            text(body)
                .key("body")
                .font(p.k.ty.ui)
                .color(p.muted())
                .align(rune::Align::Center)
                .wrap()
                .w(400.0)
        }))
}

fn welcome(p: Paint) -> El {
    title(
        p,
        tr("Welcome to Hex"),
        Some(tr(
            "Hold a key, say what you mean, and Hex types it wherever your cursor is. Your voice is transcribed on this computer.",
        )
        .into()),
    )
}

fn setup(ui: &Ui, p: Paint, view: &View) -> El {
    let model = crate::transcription_models::validate(&ui.settings.transcription).ok();
    let rows = view
        .rows
        .iter()
        .filter(|row| row.item == SetupItem::Model)
        .map(|row| {
            let description = match (row.status, model) {
                (ItemStatus::Failed, _) => {
                    tr("The download stopped. Check your connection and try again.").to_string()
                }
                (_, Some(model)) => format!("{} · {}", model.name, model.size_label()),
                (_, None) => tr("Runs entirely on this computer").to_string(),
            };
            let kind = if row.next {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            };
            let control = match row.status {
                ItemStatus::Ready => text(tr("Ready"))
                    .key("ready")
                    .font(p.k.ty.ui_medium)
                    .color(p.success()),
                ItemStatus::Downloading(fraction) => Progress::new(Some(fraction))
                    .show_value()
                    .build(p.k)
                    .key("download")
                    .w(160.0),
                ItemStatus::Failed => {
                    button(p, "setup-grant", tr("Retry"), kind, |ui: &mut Ui, _| {
                        input(ui, Input::Grant(SetupItem::Model))
                    })
                }
                _ => button(p, "setup-grant", tr("Download"), kind, |ui: &mut Ui, _| {
                    input(ui, Input::Grant(SetupItem::Model))
                }),
            };
            setting_row(
                p,
                "setup-model",
                tr("Speech model"),
                Some(&description),
                control,
            )
        })
        .collect();
    col()
        .key("setup")
        .gap(24.0)
        .child(title(
            p,
            tr("Download a speech model"),
            Some(tr("Hex transcribes on this computer, so it needs a local model first.").into()),
        ))
        .child(card(p, "setup-card", rows))
}

fn done(ui: &Ui, p: Paint) -> El {
    let shortcut = ui.settings.dictation_hotkey.keycaps().join("+");
    let mut tips = Vec::new();
    if let Some(binding) = &ui.settings.paste_last_hotkey {
        tips.push(setting_row(
            p,
            "tip-paste-last",
            tr("Paste your last dictation"),
            Some(tr("Missed the text field? Paste it again anywhere.")),
            keycaps(p, "keys", &binding.keycaps()),
        ));
    }
    tips.push(setting_row(
        p,
        "tip-voice-action",
        tr("Voice Action"),
        Some(tr(
            "Ask OpenCode to write or rewrite. Turn it on in Voice Action.",
        )),
        keycaps(p, "keys", &ui.settings.edit_hotkey.keycaps()),
    ));
    tips.push(setting_row(
        p,
        "tip-modes",
        tr("Modes"),
        Some(tr("Fix names, tone, and formatting per app.")),
        el().key("none"),
    ));
    col()
        .key("done")
        .gap(24.0)
        .child(title(
            p,
            tr("You're all set"),
            Some(tr_fill(
                "Hex keeps running after you close this window. Use {} in any app to dictate.",
                &shortcut,
            )),
        ))
        .child(
            col()
                .key("more")
                .gap(8.0)
                .child(
                    text(tr("When you want more"))
                        .key("label")
                        .font(DESCRIPTION)
                        .color(p.muted())
                        .ml(12.0),
                )
                .child(card(p, "tips-card", tips)),
        )
}

/// A first run at `step`, with the selected model not yet installed.
pub(super) fn fixture(ui: &mut Ui, step: Step, downloading: bool) {
    ui.picker.installed = Some(Vec::new());
    let status = crate::onboarding::status_with_transcription_model(false);
    let mut flow = Flow::new(Step::Welcome, status, false, false);
    if step != Step::Welcome {
        flow.on(Input::Continue);
    }
    ui.tour = Some(Tour {
        flow,
        advance_at: None,
        completed: false,
    });
    if downloading {
        input(ui, Input::Grant(SetupItem::Model));
        if let Some(view) = &mut ui.view {
            let model = ui.settings.transcription.model;
            let bytes = crate::transcription_models::definition(model)
                .download_bytes()
                .unwrap_or(0);
            view.transcription.preparing = Some(model);
            view.transcription.downloaded_bytes = bytes * 62 / 100;
        }
        poll(ui);
    }
}
