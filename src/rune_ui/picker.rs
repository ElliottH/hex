//! The local transcription picker: a modal with languages on the left and
//! that language's recommended models on the right. Choosing a model asks
//! the service to download, verify, and load it (`ChooseModel`); its
//! progress comes back in the service snapshot, and the service saves the
//! selection when it is ready. Downloaded models other than the active one can be deleted after a
//! confirmation.

use rune::{El, col, el, row, text};
use rune_kit::{Badge, ButtonSize, IconButton, IconButtonVariant, Progress};

use super::Request;
use super::Ui;
use super::controls::{ButtonKind, DESCRIPTION, Paint, button};
use crate::desktop_i18n::{tr, tr_fill};
use crate::transcription_models::{
    self, ModelDefinition, ModelPreparationStage, ModelRuntime, TranscriptionModelId,
    TranscriptionSelection,
};

const MODAL_WIDTH: f32 = 780.0;
const MODAL_HEIGHT: f32 = 520.0;

/// A model and language is the active selection only once it is installed.
fn transcription_selection_is_active(
    selection: &TranscriptionSelection,
    model: &ModelDefinition,
    language: &str,
    installed: bool,
) -> bool {
    installed && selection.model == model.id && selection.language == language
}

#[derive(Default)]
pub(super) struct Picker {
    /// The language the open picker shows; `None` when closed.
    pub(super) language: Option<String>,
    /// The model whose Delete is armed.
    deletion: Option<TranscriptionModelId>,
    deletion_error: Option<String>,
    /// Installed models for fixtures; live windows ask the filesystem.
    pub(super) installed: Option<Vec<TranscriptionModelId>>,
}

impl Picker {
    pub(super) fn installed(&self, model: &ModelDefinition, language: &str) -> bool {
        match &self.installed {
            Some(installed) => installed.contains(&model.id),
            None => transcription_models::is_installed(model, language),
        }
    }
}

pub(super) fn open(ui: &mut Ui, language: String) {
    ui.open = None;
    ui.picker.language = Some(language);
    ui.picker.deletion = None;
    ui.picker.deletion_error = None;
}

pub(super) fn close(ui: &mut Ui) {
    ui.picker.language = None;
    ui.picker.deletion = None;
    ui.picker.deletion_error = None;
}

fn select_language(ui: &mut Ui, language: String) {
    if ui.picker.language.as_deref() != Some(&language) {
        cancel(ui);
        ui.picker.language = Some(language);
        ui.picker.deletion = None;
        ui.picker.deletion_error = None;
    }
}

fn cancel(ui: &mut Ui) {
    if ui.preparing.is_some() {
        ui.send(Request::CancelModel);
    }
}

fn choose(ui: &mut Ui, model: &'static ModelDefinition, language: String) {
    ui.picker.deletion = None;
    ui.picker.deletion_error = None;
    let installed = ui.picker.installed(model, &language);
    if transcription_selection_is_active(&ui.settings.transcription, model, &language, installed) {
        close(ui);
        return;
    }
    ui.send(Request::ChooseModel {
        model: model.id,
        language,
    });
}

fn delete(ui: &mut Ui, model: &'static ModelDefinition) {
    ui.picker.deletion_error = None;
    if ui.picker.deletion != Some(model.id) {
        ui.picker.deletion = Some(model.id);
        return;
    }
    ui.picker.deletion = None;
    if !ui.live() {
        return;
    }
    match transcription_models::remove_installed(model) {
        Ok(()) => tracing::info!(model = ?model.id, "deleted transcription model"),
        Err(error) => ui.picker.deletion_error = Some(error.to_string()),
    }
}

/// The picker over the window with its scrim, when open.
pub(super) fn modal(ui: &Ui, p: Paint) -> Option<El> {
    let language = ui.picker.language.clone()?;
    let c = &p.k.c;
    let mut languages = col().key("languages").gap(1.0);
    for (index, (code, name)) in transcription_models::LANGUAGES.iter().enumerate() {
        let selected = *code == language;
        let code = code.to_string();
        let mut item = row()
            .key(format!("language{index}"))
            .role("button")
            .label(*name)
            .h(28.0)
            .px(8.0)
            .center_y()
            .radius(p.k.radius.md)
            .hover(c.hover)
            .press(c.pressed)
            .on_press(move |ui: &mut Ui| select_language(ui, code.clone()))
            .child(
                text(*name)
                    .key("name")
                    .font(p.k.ty.ui)
                    .color(if selected { c.text_base } else { c.text_muted })
                    .truncate(),
            );
        if selected {
            item = item.fill(c.layer_02);
        }
        languages = languages.child(item);
    }
    let mut models = col().key("models").gap(8.0);
    let transcription = ui.view.as_ref().map(|view| &view.transcription);
    if let Some(error) = transcription.and_then(|t| t.error.clone()) {
        models = models.child(error_card(
            p,
            "install-error",
            tr("Model could not be installed."),
            &error,
        ));
    }
    if let Some(error) = &ui.picker.deletion_error {
        models = models.child(error_card(
            p,
            "delete-error",
            tr("Model could not be deleted."),
            error,
        ));
    }
    for (index, choice) in transcription_models::choices_for_runtime(&language)
        .into_iter()
        .enumerate()
    {
        models = models.child(model_card(ui, p, index, choice, &language));
    }
    let header = row()
        .key("picker-header")
        .h(52.0)
        .pl(20.0)
        .pr(12.0)
        .center_y()
        .child(
            text(tr("Local transcription"))
                .key("title")
                .font(p.k.ty.body_medium)
                .color(p.text())
                .grow(1.0),
        )
        .child(
            IconButton::new(p.k.glyph.close)
                .variant(IconButtonVariant::Ghost)
                .size(ButtonSize::Normal)
                .label(tr("Close"))
                .on_press(close)
                .build(p.k)
                .key("close-picker"),
        );
    let mut panel = col()
        .key("transcription-picker")
        .role("dialog")
        .w(MODAL_WIDTH)
        .h(MODAL_HEIGHT)
        .radius(p.k.radius.xl)
        .fill(c.bg_base)
        .border(p.hair, c.border_weak)
        .clip_soft(0.0)
        // Presses inside the panel stay inside.
        .on_press(|_: &mut Ui| {})
        .child(header)
        .child(el().key("rule").h(p.hair).w_full().fill(c.border_weak))
        .child(
            row()
                .key("picker-body")
                .grow(1.0)
                .shrink()
                .child(
                    col()
                        .key("language-column")
                        .w(220.0)
                        .h_full()
                        .padding(12.0)
                        .fill(c.bg_deep)
                        .child(super::scroll_body("language-scroll", languages)),
                )
                .child(
                    col()
                        .key("model-column")
                        .grow(1.0)
                        .shrink()
                        .h_full()
                        .padding(16.0)
                        .child(super::scroll_body("model-scroll", models)),
                ),
        );
    for shadow in &p.k.elevation.overlay {
        panel = panel.box_shadow(*shadow);
    }
    Some(
        el().key("picker-scrim")
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(c.scrim)
            .on_press(close)
            .child(panel),
    )
}

fn error_card(p: Paint, key: &str, title: &str, error: &str) -> El {
    let danger = &p.k.c.danger;
    col()
        .key(key)
        .padding(12.0)
        .gap(4.0)
        .radius(p.k.radius.md)
        .fill(danger.bg)
        .border(p.hair, danger.border)
        .child(
            text(title)
                .key("title")
                .font(p.k.ty.ui_medium)
                .color(danger.fg),
        )
        .child(
            text(error)
                .key("error")
                .font(DESCRIPTION)
                .color(p.text())
                .wrap(),
        )
}

enum Status {
    Active,
    Available {
        installed: bool,
    },
    Preparing {
        label: String,
        progress: Option<f32>,
    },
}

fn model_card(
    ui: &Ui,
    p: Paint,
    index: usize,
    choice: transcription_models::ModelChoice,
    language: &str,
) -> El {
    let c = &p.k.c;
    let model = choice.model;
    let installed = ui.picker.installed(model, language);
    let transcription = ui.view.as_ref().map(|view| &view.transcription);
    let preparing = transcription.and_then(|t| t.preparing) == Some(model.id);
    let active =
        transcription_selection_is_active(&ui.settings.transcription, model, language, installed);
    let status = if preparing {
        let transcription = transcription.expect("preparing comes from the snapshot");
        match transcription.preparation_stage {
            Some(ModelPreparationStage::Verifying) => Status::Preparing {
                label: tr("Verifying model").into(),
                progress: None,
            },
            Some(ModelPreparationStage::Loading) => {
                let elapsed = ui
                    .preparing
                    .map_or(0, |(_, since)| since.elapsed().as_secs());
                Status::Preparing {
                    label: tr_fill("Loading model · {}s", &elapsed.to_string()),
                    progress: None,
                }
            }
            _ => {
                let fraction = model.download_bytes().map_or(0.0, |bytes| {
                    (transcription.downloaded_bytes as f32 / bytes as f32).clamp(0.0, 1.0)
                });
                Status::Preparing {
                    label: tr_fill("Downloading {}%", &format!("{:.0}", fraction * 100.0)),
                    progress: matches!(model.runtime, ModelRuntime::Gguf(_)).then_some(fraction),
                }
            }
        }
    } else if active {
        Status::Active
    } else {
        Status::Available { installed }
    };
    let deletable = installed
        && !preparing
        && matches!(model.runtime, ModelRuntime::Gguf(_))
        && ui.settings.transcription.model != model.id;
    let confirming = ui.picker.deletion == Some(model.id);
    let metadata = model
        .streams
        .then(|| tr("Transcribes while you speak on a GPU"))
        .into_iter()
        .chain(
            (model.coverage != transcription_models::language_name(language))
                .then(|| tr(model.coverage)),
        )
        .chain([tr(model.quality_context), tr(model.timestamps)])
        .chain((installed && !active).then(|| tr("Installed")))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut actions = row().key("actions").gap(8.0).center_y();
    let lang = language.to_string();
    match &status {
        Status::Active => {
            actions = actions.child(
                row()
                    .key("active")
                    .gap(6.0)
                    .center_y()
                    .child(super::controls::status_dot(p.accent()))
                    .child(
                        text(tr("Active"))
                            .key("label")
                            .font(p.k.ty.ui_medium)
                            .color(p.text()),
                    ),
            );
        }
        Status::Available { installed } => {
            if deletable {
                let label = if confirming {
                    tr_fill("Delete {}?", &model.size_label())
                } else {
                    tr("Delete").into()
                };
                actions = actions.child(button(
                    p,
                    "delete-model",
                    &label,
                    if confirming {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Secondary
                    },
                    move |ui: &mut Ui, _| delete(ui, model),
                ));
            }
            actions = actions.child(button(
                p,
                "choose-model",
                if *installed {
                    tr("Use")
                } else {
                    tr("Download")
                },
                ButtonKind::Secondary,
                move |ui: &mut Ui, _| choose(ui, model, lang.clone()),
            ));
        }
        Status::Preparing { label, progress } => {
            if progress.is_none() {
                actions = actions.child(
                    row()
                        .key("state")
                        .gap(6.0)
                        .center_y()
                        .child(super::controls::status_dot(p.accent()))
                        .child(
                            text(label.clone())
                                .key("label")
                                .font(p.k.ty.ui_medium)
                                .color(p.text()),
                        ),
                );
            }
            actions = actions.child(button(
                p,
                "cancel-model",
                tr("Cancel"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| cancel(ui),
            ));
        }
    }
    let mut title = row()
        .key("title")
        .gap(8.0)
        .center_y()
        .grow(1.0)
        .shrink()
        .child(
            text(model.name)
                .key("name")
                .font(p.k.ty.body_medium)
                .color(p.text()),
        )
        .child(Badge::new(tr(choice.recommendation.label())).build(p.k));
    if model.streams {
        title = title.child(Badge::new(tr("Streams")).build(p.k));
    }
    let metric = |key: &str, label: &str, value: String| {
        row()
            .key(key)
            .gap(6.0)
            .child(text(label).key("label").font(p.k.ty.small).color(p.muted()))
            .child(
                text(value)
                    .key("value")
                    .font(p.k.ty.small_medium)
                    .color(p.text()),
            )
    };
    let mut card = col()
        .key(format!("model{index}"))
        .px(16.0)
        .py(12.0)
        .gap(8.0)
        .radius(p.k.radius.lg)
        .fill(if active { c.layer_02 } else { c.bg_base })
        .border(
            p.hair,
            if active {
                c.border_focus
            } else {
                c.border_weak
            },
        )
        .child(
            row()
                .key("top")
                .gap(12.0)
                .center_y()
                .child(title)
                .child(actions),
        )
        .child(
            text(metadata)
                .key("metadata")
                .font(p.k.ty.small)
                .color(p.muted())
                .wrap(),
        )
        .child(
            row()
                .key("metrics")
                .gap(20.0)
                .child(metric("speed", tr("Speed"), tr(model.realtime).into()))
                .child(metric("size", tr("Size"), model.size_label()))
                .child(metric("wer", "WER", model.quality.into())),
        );
    if model.id == TranscriptionModelId::ParakeetV3 {
        card = card.child(
            text(
                "Detects language from audio, not the selected language. \
                 For language-guided transcription, try Whisper.",
            )
            .key("note")
            .font(p.k.ty.small)
            .color(p.muted())
            .wrap(),
        );
    }
    if let Status::Preparing {
        label,
        progress: Some(progress),
    } = status
    {
        card = card.child(
            Progress::new(Some(progress))
                .label(label)
                .show_value()
                .build(p.k)
                .key("progress"),
        );
    }
    if !active && !preparing {
        let lang = language.to_string();
        card = card
            .role("button")
            .hover(c.hover)
            .press(c.pressed)
            .on_press(move |ui: &mut Ui| choose(ui, model, lang.clone()));
    }
    card
}

/// The picker open on English, with Parakeet active; `downloading` shows a
/// model mid-download.
pub(super) fn fixture(ui: &mut Ui, downloading: Option<TranscriptionModelId>) {
    ui.picker.language = Some("en".into());
    if let (Some(model), Some(view)) = (downloading, ui.view.as_mut()) {
        let bytes = transcription_models::definition(model)
            .download_bytes()
            .unwrap_or(0);
        view.transcription.preparing = Some(model);
        view.transcription.downloaded_bytes = bytes * 37 / 100;
        view.transcription.preparation_stage = Some(ModelPreparationStage::Downloading);
        ui.preparing = Some((model, std::time::Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_transcription_model_is_not_active_until_installed() {
        let selection = TranscriptionSelection::default();
        let model = transcription_models::definition(selection.model);

        assert!(!transcription_selection_is_active(
            &selection,
            model,
            &selection.language,
            false
        ));
        assert!(transcription_selection_is_active(
            &selection,
            model,
            &selection.language,
            true
        ));
    }
}
