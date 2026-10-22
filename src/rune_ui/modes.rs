//! The Modes pane: the Global mode and custom modes chosen by the frontmost
//! application, each with corrections, optional OpenCode rewriting, and
//! TypeScript transformations, as in GPUI. Text fields are drafts saved when
//! typing pauses; every other edit saves at once and asks the service to
//! reload.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use rune::{Cursor, Drag, El, Phase, TextEdit, Tx, Waker, col, edit_access, el, row, spacer, text};
use rune_kit::{ButtonSize, IconButton, IconButtonVariant, Switch, TextArea, TextInput};

use super::Ui;
use super::controls::{ButtonKind, DESCRIPTION, Highlight, Paint, button, card, note, setting_row};
use super::models::{self, Target};
use crate::app_settings::{AppSettings, DictationMode};
use crate::app_window::{OpenCodeFeature, reorder_transformation};
use crate::application_catalog::InstalledApplication;
use crate::desktop_i18n::{tr, tr_fill, tr_named};
use crate::personal_commands::{HostState, StatusSnapshot, StatusTransformation};
use crate::text_replacements::TextReplacement;

const LIST_WIDTH: f32 = 240.0;
const FIELD_WIDTH: f32 = 300.0;
/// How often the transformation host's status file is reread.
const STATUS_INTERVAL: Duration = Duration::from_secs(1);

static PLUS: &rune::Icon = &rune_theme_opencode::icons::PLUS;
static CLOSE: &rune::Icon = &rune_theme_opencode::icons::CLOSE;
static GRIP: rune::Icon = rune::Icon {
    name: "hex-grip",
    view: 16.0,
    body: r#"<path d="M4 5.5H12M4 8H12M4 10.5H12" stroke="currentColor"/>"#,
};
/// A transformation row's height; dragging moves one place per row.
const TRANSFORMATION_ROW: f32 = 36.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Selection {
    Default,
    Custom(usize),
}

pub(super) fn mode(settings: &AppSettings, selection: Selection) -> &DictationMode {
    match selection {
        Selection::Default => &settings.dictation_processing.default_mode,
        Selection::Custom(index) => &settings.dictation_processing.modes[index],
    }
}

pub(super) fn mode_mut(settings: &mut AppSettings, selection: Selection) -> &mut DictationMode {
    match selection {
        Selection::Default => &mut settings.dictation_processing.default_mode,
        Selection::Custom(index) => &mut settings.dictation_processing.modes[index],
    }
}

/// The selected mode's text fields.
#[derive(Default)]
struct Editor {
    name: TextEdit,
    prompt: TextEdit,
    deadline: TextEdit,
    replacements: Vec<(TextEdit, TextEdit)>,
}

impl Editor {
    fn of(mode: &DictationMode) -> Self {
        Self {
            name: TextEdit::new(mode.name.clone()),
            prompt: TextEdit::new(mode.post_processing.prompt.clone()),
            deadline: TextEdit::new(mode.post_processing.deadline_seconds.to_string()),
            replacements: mode
                .replacements
                .iter()
                .map(|r| {
                    (
                        TextEdit::new(r.matched_phrase.clone()),
                        TextEdit::new(r.output.clone()),
                    )
                })
                .collect(),
        }
    }
}

/// A transformation being dragged: the order it started from, the order
/// shown while dragging, and where it started.
struct TransformationDrag {
    id: String,
    start: usize,
    committed: Vec<String>,
    order: Vec<String>,
}

pub(super) type ChosenApplication = Result<Option<InstalledApplication>, String>;

enum Applications {
    NotLoaded,
    Loading(Receiver<Vec<InstalledApplication>>),
    Loaded(Vec<InstalledApplication>),
}

pub(super) struct ModesPane {
    pub(super) selected: Selection,
    editor: Editor,
    pub(super) delete_armed: bool,
    pub(super) transformation_picker_open: bool,
    pub(super) application_picker_open: bool,
    application_search: TextEdit,
    application_highlight: Highlight,
    application_error: Option<String>,
    applications: Applications,
    /// The desktop's file chooser, while it is open.
    chooser: Option<Receiver<ChosenApplication>>,
    drag: Option<TransformationDrag>,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(super) waker: Option<Waker>,
    /// Whether the listener can tell which app is frontmost (X11, not Wayland).
    focused_app_known: bool,
    status: StatusSnapshot,
    status_read: Option<Instant>,
    workspace: PathBuf,
    config_exists: bool,
    workspace_receiver: Option<Receiver<Result<PathBuf, String>>>,
    workspace_error: Option<String>,
}

impl ModesPane {
    pub(super) fn new(settings: &AppSettings) -> Self {
        let mut status = StatusSnapshot::default();
        crate::personal_commands::include_builtin_transformations(&mut status);
        Self {
            selected: Selection::Default,
            editor: Editor::of(&settings.dictation_processing.default_mode),
            delete_armed: false,
            transformation_picker_open: false,
            application_picker_open: false,
            application_search: TextEdit::default(),
            application_highlight: Highlight::default(),
            application_error: None,
            applications: Applications::NotLoaded,
            chooser: None,
            drag: None,
            waker: None,
            focused_app_known: true,
            status,
            status_read: None,
            workspace: PathBuf::new(),
            config_exists: true,
            workspace_receiver: None,
            workspace_error: None,
        }
    }

    pub(super) fn load_personal_commands(&mut self) {
        self.focused_app_known = super::host::focused_application_known();
        self.workspace = crate::app_paths::personal_commands_workspace()
            .unwrap_or_else(|_| PathBuf::from("~/.config/hex"));
        self.read_status();
    }

    fn read_status(&mut self) -> bool {
        self.status_read = Some(Instant::now());
        let exists = self.workspace.join("hex.config.ts").is_file();
        let mut changed = exists != self.config_exists;
        self.config_exists = exists;
        let mut status =
            crate::personal_commands::load_status().unwrap_or_else(|error| StatusSnapshot {
                host_state: HostState::Unavailable,
                last_reload_error: Some(error.to_string()),
                ..StatusSnapshot::default()
            });
        crate::personal_commands::include_builtin_transformations(&mut status);
        if status != self.status {
            self.status = status;
            changed = true;
        }
        changed
    }

    /// The selected mode's drafts applied to `settings`.
    pub(super) fn project(&self, settings: &mut AppSettings) {
        let selection = self.selected;
        if let Selection::Custom(index) = selection
            && index >= settings.dictation_processing.modes.len()
        {
            return;
        }
        let editor = &self.editor;
        let mode = mode_mut(settings, selection);
        if selection != Selection::Default {
            mode.name = editor.name.text.trim().to_string();
        }
        mode.replacements = editor
            .replacements
            .iter()
            .map(|(phrase, output)| TextReplacement {
                matched_phrase: phrase.text.clone(),
                output: output.text.clone(),
            })
            .collect();
        mode.post_processing.prompt = editor.prompt.text.trim().to_string();
        mode.post_processing.deadline_seconds = editor
            .deadline
            .text
            .trim()
            .parse::<u64>()
            .unwrap_or(mode.post_processing.deadline_seconds)
            .max(1);
    }

    /// Rebuilds the drafts from settings another client saved.
    pub(super) fn reload(&mut self, settings: &AppSettings) {
        if let Selection::Custom(index) = self.selected
            && index >= settings.dictation_processing.modes.len()
        {
            self.selected = Selection::Default;
        }
        self.editor = Editor::of(mode(settings, self.selected));
    }

    pub(super) fn select(&mut self, selection: Selection) {
        self.selected = selection;
        self.delete_armed = false;
        self.transformation_picker_open = false;
        self.application_picker_open = false;
        self.application_error = None;
        self.drag = None;
    }

    pub(super) fn poll(&mut self, live: bool) -> bool {
        let mut changed = false;
        if let Applications::Loading(receiver) = &self.applications
            && let Ok(applications) = receiver.try_recv()
        {
            self.applications = Applications::Loaded(applications);
            changed = true;
        }
        if let Some(receiver) = &self.workspace_receiver
            && let Ok(result) = receiver.try_recv()
        {
            self.workspace_receiver = None;
            match result {
                Ok(workspace) => {
                    self.workspace = workspace;
                    self.workspace_error = None;
                }
                Err(error) => self.workspace_error = Some(error),
            }
            changed = true;
        }
        if live
            && self
                .status_read
                .is_none_or(|at| at.elapsed() >= STATUS_INTERVAL)
        {
            changed |= self.read_status();
        }
        changed
    }

    fn ensure_applications(&mut self) {
        if matches!(self.applications, Applications::NotLoaded) {
            let (sender, receiver) = sync_channel(1);
            thread::spawn(move || {
                let _ = sender.send(crate::application_catalog::discover());
            });
            self.applications = Applications::Loading(receiver);
        }
    }

    fn application_choices(&self, selected: &[String]) -> Vec<&InstalledApplication> {
        let Applications::Loaded(applications) = &self.applications else {
            return Vec::new();
        };
        let query = self.application_search.text.trim().to_lowercase();
        applications
            .iter()
            .filter(|application| {
                !selected.contains(&application.name)
                    && (query.is_empty()
                        || application.name.to_lowercase().contains(&query)
                        || application
                            .bundle_id
                            .as_deref()
                            .is_some_and(|bundle| bundle.to_lowercase().contains(&query)))
            })
            .take(10)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Edits

fn select_mode(ui: &mut Ui, selection: Selection) {
    if ui.modes.selected == selection {
        return;
    }
    ui.flush_drafts();
    ui.open = None;
    ui.modes.select(selection);
    ui.modes.editor = Editor::of(mode(&ui.settings, selection));
}

/// Saves one change to the selected mode.
fn update_mode(ui: &mut Ui, change: impl FnOnce(&mut DictationMode)) -> bool {
    let selection = ui.modes.selected;
    ui.update(|settings| change(mode_mut(settings, selection)))
}

fn add_mode(ui: &mut Ui) {
    ui.flush_drafts();
    let mut new = ui.settings.dictation_processing.default_mode.clone();
    new.name = tr_fill(
        "Mode {}",
        &(ui.settings.dictation_processing.modes.len() + 1).to_string(),
    );
    new.applications.clear();
    new.browser_hosts.clear();
    let index = ui.settings.dictation_processing.modes.len();
    if ui.update(|settings| settings.dictation_processing.modes.push(new)) {
        ui.modes.select(Selection::Custom(index));
        ui.modes.editor = Editor::of(mode(&ui.settings, Selection::Custom(index)));
    }
}

fn delete_mode(ui: &mut Ui) {
    let Selection::Custom(index) = ui.modes.selected else {
        return;
    };
    if !ui.modes.delete_armed {
        ui.modes.delete_armed = true;
        return;
    }
    // The drafts belong to the mode going away.
    ui.modes.editor = Editor::of(mode(&ui.settings, ui.modes.selected));
    if ui.update(|settings| {
        settings.dictation_processing.modes.remove(index);
    }) {
        ui.modes.select(Selection::Default);
        ui.modes.editor = Editor::of(mode(&ui.settings, Selection::Default));
    }
}

fn add_replacement(ui: &mut Ui) {
    if update_mode(ui, |mode| {
        mode.replacements.push(TextReplacement::default())
    }) {
        let mut phrase = TextEdit::default();
        phrase.focused = true;
        ui.modes
            .editor
            .replacements
            .push((phrase, TextEdit::default()));
    }
}

fn remove_replacement(ui: &mut Ui, index: usize) {
    if index < ui.modes.editor.replacements.len() {
        ui.modes.editor.replacements.remove(index);
        ui.flush_drafts();
    }
}

fn add_application(ui: &mut Ui, name: String) {
    let selection = ui.modes.selected;
    let conflict = ui
        .settings
        .dictation_processing
        .modes
        .iter()
        .enumerate()
        .find(|(index, mode)| {
            Selection::Custom(*index) != selection && mode.applications.contains(&name)
        })
        .map(|(_, mode)| mode.name.clone());
    if let Some(conflict) = conflict {
        ui.modes.application_error = Some(tr_named(
            "{name} already activates {conflict}. Remove it there before reassigning it.",
            &[("name", &name), ("conflict", &conflict)],
        ));
        return;
    }
    if !mode(&ui.settings, selection).applications.contains(&name)
        && update_mode(ui, |mode| {
            mode.applications.push(name);
            mode.applications.sort_by_key(|name| name.to_lowercase());
        })
    {
        ui.modes.application_search = TextEdit::default();
        ui.modes.application_highlight = Highlight::default();
        ui.modes.application_search.focused = true;
        ui.modes.application_error = None;
    }
}

/// Up and Down move the application search's highlight.
pub(super) fn on_key(ui: &mut Ui, event: &rune::KeyEvent) -> bool {
    if ui.pane != super::Pane::Modes
        || !ui.modes.application_picker_open
        || !ui.modes.application_search.focused
    {
        return false;
    }
    let selected = mode(&ui.settings, ui.modes.selected).applications.clone();
    let query = ui.modes.application_search.text.trim().to_lowercase();
    let count = ui.modes.application_choices(&selected).len();
    ui.modes
        .application_highlight
        .key(&event.key, &query, count)
}

/// Opens the desktop's file chooser for an application the catalog does
/// not list: a `.desktop` entry or an executable.
#[cfg(target_os = "linux")]
fn choose_application(ui: &mut Ui) {
    if !ui.live() || ui.modes.chooser.is_some() {
        return;
    }
    let (sender, receiver) = sync_channel(1);
    let waker = ui.modes.waker.clone();
    let (title, accept) = (tr("Add Application").to_string(), tr("Add").to_string());
    let filters = [
        crate::linux_file_chooser::Filter {
            label: tr("Applications").into(),
            patterns: vec!["*.desktop"],
        },
        crate::linux_file_chooser::Filter {
            label: tr("All files").into(),
            patterns: vec!["*"],
        },
    ];
    thread::spawn(move || {
        let result = crate::linux_file_chooser::open_file(&title, &accept, &filters)
            .and_then(|path| {
                path.map(|path| crate::application_catalog::metadata(&path))
                    .transpose()
            })
            .map_err(|error| format!("{error:#}"));
        let _ = sender.send(result);
        if let Some(waker) = waker {
            waker();
        }
    });
    ui.modes.chooser = Some(receiver);
}

/// Adds what the file chooser returned, as GPUI does: the application joins
/// the catalog and the selected mode.
pub(super) fn poll_chooser(ui: &mut Ui) -> bool {
    let Some(result) = ui.modes.chooser.as_ref().and_then(|r| r.try_recv().ok()) else {
        return false;
    };
    ui.modes.chooser = None;
    chosen_application(ui, result);
    true
}

pub(super) fn chosen_application(ui: &mut Ui, result: ChosenApplication) {
    match result {
        Ok(Some(application)) => {
            let name = application.name.clone();
            match &mut ui.modes.applications {
                Applications::Loaded(applications) => {
                    crate::application_catalog::insert(applications, application);
                }
                applications => *applications = Applications::Loaded(vec![application]),
            }
            ui.modes.application_error = None;
            add_application(ui, name);
        }
        Ok(None) => {}
        Err(error) => ui.modes.application_error = Some(error),
    }
}

/// Drags a transformation by its handle. Rows are keyed by position, so the
/// pressed slot stays put while the rows reorder under it; the new order is
/// saved once, on release.
fn drag_transformation(ui: &mut Ui, id: &str, start: usize, pitch: f32, d: &Drag) {
    match d.phase {
        Phase::Down => {
            let committed = mode(&ui.settings, ui.modes.selected)
                .transformations
                .clone();
            ui.modes.drag = Some(TransformationDrag {
                id: id.to_string(),
                start,
                order: committed.clone(),
                committed,
            });
        }
        Phase::Move => {
            let Some(drag) = &mut ui.modes.drag else {
                return;
            };
            let moved = ((d.y - d.start_y) / pitch).round() as isize;
            let last = drag.committed.len().saturating_sub(1) as isize;
            let target = (drag.start as isize + moved).clamp(0, last) as usize;
            let mut order = drag.committed.clone();
            reorder_transformation(&mut order, &drag.id, target);
            drag.order = order;
        }
        Phase::Up => {
            let Some(drag) = ui.modes.drag.take() else {
                return;
            };
            if drag.order != drag.committed {
                update_mode(ui, |mode| mode.transformations = drag.order);
            }
        }
    }
}

fn provision_workspace(ui: &mut Ui) {
    if !ui.live() || ui.modes.workspace_receiver.is_some() {
        return;
    }
    let (sender, receiver) = sync_channel(1);
    thread::spawn(move || {
        let result =
            crate::personal_commands::initialize_workspace().map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    ui.modes.workspace_receiver = Some(receiver);
}

// ---------------------------------------------------------------------------
// View

fn icon_button(p: Paint, key: &str, glyph: &'static rune::Icon, label: &str) -> IconButton {
    let _ = (p, key);
    IconButton::new(glyph)
        .variant(IconButtonVariant::Ghost)
        .size(ButtonSize::Normal)
        .label(label)
}

pub(super) fn pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> (Vec<El>, El) {
    let add = IconButton::new(PLUS)
        .variant(IconButtonVariant::Neutral)
        .size(ButtonSize::Normal)
        .label(tr("Add mode"))
        .on_press(add_mode)
        .build(p.k)
        .key("add-mode");
    let c = &p.k.c;
    let mut list = col().key("modes-list").gap(2.0);
    list = list.child(mode_row(
        p,
        "mode-global",
        tr("Global"),
        tr("Fallback for everything"),
        ui.modes.selected == Selection::Default,
        Selection::Default,
    ));
    for (index, mode) in ui.settings.dictation_processing.modes.iter().enumerate() {
        let summary = if mode.applications.is_empty() {
            tr("No activation rules").to_string()
        } else {
            mode.applications.join(", ")
        };
        list = list.child(mode_row(
            p,
            &format!("mode{index}"),
            &mode.name,
            &summary,
            ui.modes.selected == Selection::Custom(index),
            Selection::Custom(index),
        ));
    }
    let list = col()
        .key("modes-column")
        .w(LIST_WIDTH)
        .h_full()
        .pl(16.0)
        .pr(8.0)
        .child(super::scroll_body("modes-scroll", list));
    let body = row()
        .key("modes-body")
        .grow(1.0)
        .shrink()
        .child(list)
        .child(
            el().key("modes-rule")
                .w(p.hair)
                .h_full()
                .fill(c.border_weak),
        )
        .child(detail(ui, p, layer));
    (vec![add], body)
}

fn mode_row(
    p: Paint,
    key: &str,
    name: &str,
    summary: &str,
    selected: bool,
    selection: Selection,
) -> El {
    let c = &p.k.c;
    let mut item = col()
        .key(key)
        .role("button")
        .label(name)
        .px(10.0)
        .py(8.0)
        .gap(2.0)
        .radius(p.k.radius.md)
        .hover(c.hover)
        .press(c.pressed)
        .on_press(move |ui: &mut Ui| select_mode(ui, selection))
        .child(
            text(name)
                .key("name")
                .font(p.k.ty.ui_medium)
                .color(p.text())
                .truncate(),
        )
        .child(
            text(summary)
                .key("summary")
                .font(p.k.ty.small)
                .color(p.muted())
                .truncate(),
        );
    if selected {
        item = item.fill(c.layer_02);
    }
    item
}

fn detail(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let selection = ui.modes.selected;
    let current = mode(&ui.settings, selection);
    let mut content = col()
        .key("mode-detail-content")
        .px(24.0)
        .gap(16.0);
    content = content.child(basics(ui, p, selection, current));
    content = content.child(
        text(tr("Text processing"))
            .key("processing-label")
            .font(p.k.ty.ui_medium)
            .color(p.muted())
            .ml(12.0)
            .mt(8.0),
    );
    content = content.child(corrections(ui, p));
    content = content.child(processing(ui, p, selection, current, layer));
    content = content.child(transformations(ui, p, current));
    if selection != Selection::Default {
        let action = if ui.modes.delete_armed {
            row()
                .key("delete-actions")
                .gap(8.0)
                .child(button(
                    p,
                    "cancel-delete-mode",
                    tr("Cancel"),
                    ButtonKind::Secondary,
                    |ui: &mut Ui, _| ui.modes.delete_armed = false,
                ))
                .child(button(
                    p,
                    "confirm-delete-mode",
                    tr("Confirm delete"),
                    ButtonKind::Danger,
                    |ui: &mut Ui, _| delete_mode(ui),
                ))
        } else {
            button(
                p,
                "delete-mode",
                tr("Delete mode"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| delete_mode(ui),
            )
        };
        content = content.child(row().key("delete-row").justify_end().child(action));
    }
    col()
        .key("mode-detail")
        .grow(1.0)
        .shrink()
        .child(super::scroll_body("mode-detail-scroll", content))
}

fn basics(ui: &Ui, p: Paint, selection: Selection, current: &DictationMode) -> El {
    if selection == Selection::Default {
        return card(
            p,
            "basics-card",
            vec![setting_row(
                p,
                "global",
                tr("Global"),
                Some(tr("Used unless a more specific mode matches.")),
                el().key("none"),
            )],
        );
    }
    let name = TextInput::new(&ui.modes.editor.name)
        .placeholder(tr("Mode name"))
        .width(Some(FIELD_WIDTH))
        .on_edit(|ui: &mut Ui| &mut ui.modes.editor.name)
        .build(p.k)
        .key("mode-name");
    let mut chips = row()
        .key("applications")
        .gap(4.0)
        .center_y()
        .flex_wrap()
        .max_w(FIELD_WIDTH);
    for (index, name) in current.applications.iter().enumerate() {
        let remove = name.clone();
        chips = chips.child(
            row()
                .key(format!("application{index}"))
                .h(24.0)
                .pl(8.0)
                .gap(2.0)
                .center_y()
                .radius(p.k.radius.sm)
                .fill(p.k.c.layer_02)
                .child(
                    text(name.clone())
                        .key("name")
                        .font(p.k.ty.small_medium)
                        .color(p.text()),
                )
                .child(
                    icon_button(p, "remove", CLOSE, tr("Remove"))
                        .size(ButtonSize::Small)
                        .on_press(move |ui: &mut Ui| {
                            let remove = remove.clone();
                            update_mode(ui, |mode| {
                                mode.applications
                                    .retain(|application| *application != remove)
                            });
                        })
                        .build(p.k)
                        .key("remove"),
                ),
        );
    }
    chips = chips.child(
        IconButton::new(PLUS)
            .variant(IconButtonVariant::Neutral)
            .size(ButtonSize::Small)
            .label(tr("Add application"))
            .on_press(|ui: &mut Ui| {
                ui.modes.application_picker_open = !ui.modes.application_picker_open;
                ui.modes.application_error = None;
                ui.modes.ensure_applications();
                ui.modes.application_search = TextEdit::default();
                ui.modes.application_search.focused = ui.modes.application_picker_open;
            })
            .build(p.k)
            .key("open-application-picker"),
    );
    let mut rows = vec![
        setting_row(p, "name", tr("Name"), None, name),
        setting_row(
            p,
            "applications-row",
            tr("Applications"),
            Some(if ui.modes.focused_app_known {
                tr("Switch when an app is frontmost")
            } else {
                tr("Wayland doesn't report the frontmost app; these apply under X11")
            }),
            chips,
        ),
    ];
    if let Some(error) = &ui.modes.application_error {
        rows.push(note("application-error", error, p.danger()));
    }
    if ui.modes.application_picker_open {
        rows.push(application_picker(ui, p, current));
    }
    card(p, "basics-card", rows)
}

fn application_picker(ui: &Ui, p: Paint, current: &DictationMode) -> El {
    let c = &p.k.c;
    let search = TextInput::new(&ui.modes.application_search)
        .placeholder(tr("Search applications"))
        .leading_icon(p.k.glyph.search)
        .width(None)
        .on_edit(|ui: &mut Ui| &mut ui.modes.application_search)
        .on_submit(|ui: &mut Ui, _: &mut Tx| {
            let selected = mode(&ui.settings, ui.modes.selected).applications.clone();
            let query = ui.modes.application_search.text.trim().to_lowercase();
            let choices = ui.modes.application_choices(&selected);
            let picked = ui
                .modes
                .application_highlight
                .index(&query, choices.len())
                .map(|index| choices[index].name.clone());
            if let Some(name) = picked {
                add_application(ui, name);
            }
        })
        .build(p.k)
        .key("application-search");
    let choices = ui.modes.application_choices(&current.applications);
    let query = ui.modes.application_search.text.trim().to_lowercase();
    let highlighted = ui.modes.application_highlight.index(&query, choices.len());
    let status = match &ui.modes.applications {
        Applications::NotLoaded | Applications::Loading(_) => {
            Some(tr("Finding applications…").to_string())
        }
        Applications::Loaded(all) if all.is_empty() => {
            Some(tr("No applications found.").to_string())
        }
        Applications::Loaded(_) if choices.is_empty() && !query.is_empty() => {
            Some(tr_fill("No applications match \"{}\".", &query))
        }
        Applications::Loaded(_) => None,
    };
    let mut results = col().key("application-results").gap(1.0);
    for (index, application) in choices.into_iter().enumerate() {
        let name = application.name.clone();
        let location = application
            .path
            .parent()
            .map_or_else(String::new, |path| path.display().to_string());
        let fill = if highlighted == Some(index) {
            c.hover
        } else {
            rune::Color::TRANSPARENT
        };
        results = results.child(
            row()
                .key(format!("available{index}"))
                .fill(fill)
                .role("button")
                .label(application.name.clone())
                .px(8.0)
                .py(6.0)
                .gap(10.0)
                .center_y()
                .radius(p.k.radius.sm)
                .hover(c.hover)
                .press(c.pressed)
                .on_press(move |ui: &mut Ui| add_application(ui, name.clone()))
                .child(monogram(p, &application.name))
                .child(
                    col()
                        .key("copy")
                        .grow(1.0)
                        .shrink()
                        .child(
                            text(application.name.clone())
                                .key("name")
                                .font(p.k.ty.ui_medium)
                                .color(p.text())
                                .truncate(),
                        )
                        .child(
                            text(location)
                                .key("location")
                                .font(p.k.ty.small)
                                .color(p.muted())
                                .truncate(),
                        ),
                ),
        );
    }
    col()
        .key("application-picker")
        .ml(12.0)
        .mr(12.0)
        .mb(8.0)
        .padding(8.0)
        .gap(6.0)
        .radius(p.k.radius.md)
        .fill(c.layer_01)
        .child(search)
        .children(status.map(|status| {
            text(status)
                .key("status")
                .font(DESCRIPTION)
                .color(p.muted())
                .px(8.0)
                .py(6.0)
        }))
        .child(results)
        .child(
            row()
                .key("picker-actions")
                .justify_between()
                .children(choose_file_button(p))
                .child(spacer())
                .child(button(
                    p,
                    "close-application-picker",
                    tr("Done"),
                    ButtonKind::Secondary,
                    |ui: &mut Ui, _| {
                        ui.modes.application_picker_open = false;
                        ui.modes.application_search = TextEdit::default();
                    },
                )),
        )
}

/// "Choose a file…": Linux asks the desktop's file chooser portal; Windows
/// has no native dialog under Rune yet.
fn choose_file_button(p: Paint) -> Option<El> {
    #[cfg(target_os = "linux")]
    return Some(button(
        p,
        "choose-application-file",
        tr("Choose a file…"),
        ButtonKind::Secondary,
        |ui: &mut Ui, _| choose_application(ui),
    ));
    #[cfg(not(target_os = "linux"))]
    {
        let _ = p;
        None
    }
}

/// An app's initial on a tinted square: Rune has no raster images yet, so
/// the catalog's icons are not drawn.
fn monogram(p: Paint, name: &str) -> El {
    let letter = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or("?".into(), |c| c.to_uppercase().to_string());
    row()
        .key("monogram")
        .size(24.0, 24.0)
        .center()
        .radius(p.k.radius.sm)
        .fill(p.k.c.layer_03)
        .child(
            text(letter)
                .key("letter")
                .font(p.k.ty.small_medium)
                .color(p.text()),
        )
}

fn card_header(p: Paint, key: &str, title: &str, action: Option<El>) -> El {
    row()
        .key(key)
        .min_h(40.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .child(
            text(title)
                .key("title")
                .font(p.k.ty.ui_medium)
                .color(p.text())
                .grow(1.0),
        )
        .children(action)
}

fn corrections(ui: &Ui, p: Paint) -> El {
    let add = IconButton::new(PLUS)
        .variant(IconButtonVariant::Ghost)
        .size(ButtonSize::Normal)
        .label(tr("Add correction"))
        .on_press(add_replacement)
        .build(p.k)
        .key("add-correction");
    let mut rows = vec![card_header(
        p,
        "corrections-header",
        tr("Corrections"),
        Some(add),
    )];
    if ui.modes.editor.replacements.is_empty() {
        rows.push(note(
            "no-corrections",
            tr("No corrections in this mode."),
            p.muted(),
        ));
    }
    for (index, (phrase, output)) in ui.modes.editor.replacements.iter().enumerate() {
        rows.push(
            row()
                .key(format!("correction{index}"))
                .px(12.0)
                .py(6.0)
                .gap(8.0)
                .center_y()
                .child(
                    TextInput::new(phrase)
                        .placeholder(tr("Spoken phrase (e.g. open code)"))
                        .width(None)
                        .on_edit(edit_access(move |ui: &mut Ui| {
                            &mut ui.modes.editor.replacements[index].0
                        }))
                        .build(p.k)
                        .key("phrase")
                        .grow(1.0)
                        .shrink(),
                )
                .child(text("→").key("arrow").font(p.k.ty.ui).color(p.faint()))
                .child(
                    TextInput::new(output)
                        .placeholder(tr("Replacement (e.g. OpenCode)"))
                        .width(None)
                        .on_edit(edit_access(move |ui: &mut Ui| {
                            &mut ui.modes.editor.replacements[index].1
                        }))
                        .build(p.k)
                        .key("output")
                        .grow(1.0)
                        .shrink(),
                )
                .child(
                    icon_button(p, "remove", CLOSE, tr("Remove"))
                        .on_press(move |ui: &mut Ui| remove_replacement(ui, index))
                        .build(p.k)
                        .key("remove"),
                ),
        );
    }
    card(p, "corrections-card", rows)
}

fn processing(
    ui: &Ui,
    p: Paint,
    selection: Selection,
    current: &DictationMode,
    layer: &mut Vec<El>,
) -> El {
    let enabled = current.post_processing.enabled;
    let can_toggle = enabled || ui.models.available();
    let toggle = Switch::new(enabled)
        .disabled(!can_toggle)
        .on_toggle(|ui: &mut Ui| {
            let enabled = !mode(&ui.settings, ui.modes.selected)
                .post_processing
                .enabled;
            update_mode(ui, |mode| mode.post_processing.enabled = enabled);
        })
        .build(p.k)
        .key("processing-toggle");
    let mut rows = vec![card_header(
        p,
        "processing-header",
        tr("OpenCode transformation"),
        Some(toggle),
    )];
    if enabled {
        rows.extend(models::rows(ui, p, Target::Mode(selection), layer));
        rows.push(setting_row(
            p,
            "deadline",
            tr("Deadline"),
            Some(tr("Seconds")),
            TextInput::new(&ui.modes.editor.deadline)
                .placeholder("30")
                .width(Some(96.0))
                .on_edit(|ui: &mut Ui| &mut ui.modes.editor.deadline)
                .build(p.k)
                .key("deadline-field"),
        ));
        rows.push(
            col()
                .key("instructions")
                .px(12.0)
                .py(8.0)
                .gap(6.0)
                .child(
                    text(tr("Instructions"))
                        .key("title")
                        .font(p.k.ty.ui)
                        .color(p.text()),
                )
                .child(
                    text(tr(
                        "System instructions for rewriting dictation in this mode",
                    ))
                    .key("description")
                    .font(DESCRIPTION)
                    .color(p.muted()),
                )
                .child(
                    TextArea::new(&ui.modes.editor.prompt)
                        .placeholder(tr("Describe how OpenCode should rewrite your dictation…"))
                        .rows(4)
                        .width(None)
                        .on_edit(|ui: &mut Ui| &mut ui.modes.editor.prompt)
                        .build(p.k)
                        .key("prompt"),
                ),
        );
    }
    if let Some(notice) = models::unavailable(ui, p, OpenCodeFeature::Transformation) {
        rows.push(notice);
    }
    card(p, "processing-card", rows)
}

fn transformations(ui: &Ui, p: Paint, current: &DictationMode) -> El {
    let catalog = &ui.modes.status.transformations;
    let picker_open = ui.modes.transformation_picker_open;
    let action = (!catalog.is_empty()).then(|| {
        if picker_open {
            button(
                p,
                "toggle-transformation-picker",
                tr("Done"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| ui.modes.transformation_picker_open = false,
            )
        } else {
            IconButton::new(PLUS)
                .variant(IconButtonVariant::Ghost)
                .size(ButtonSize::Normal)
                .label(tr("Add transformation"))
                .on_press(|ui: &mut Ui| ui.modes.transformation_picker_open = true)
                .build(p.k)
                .key("toggle-transformation-picker")
        }
    });
    let mut rows = vec![card_header(
        p,
        "transformations-header",
        tr("Transformations"),
        action,
    )];
    let dragging = ui.modes.drag.as_ref();
    let order = dragging.map_or(&current.transformations, |drag| &drag.order);
    let pitch = TRANSFORMATION_ROW + p.hair;
    for (index, id) in order.iter().enumerate() {
        let name = catalog
            .iter()
            .find(|transformation| &transformation.id == id)
            .map_or_else(|| id.clone(), |transformation| transformation.name.clone());
        let held = dragging.is_some_and(|drag| &drag.id == id);
        let (dragged, remove) = (id.clone(), id.clone());
        let handle = row()
            .key("handle")
            .role("button")
            .label(tr("Reorder"))
            .w(20.0)
            .h_full()
            .center()
            .cursor(if dragging.is_some() {
                Cursor::Grabbing
            } else {
                Cursor::Grab
            })
            .on_drag(move |ui: &mut Ui, d: &mut Drag, _: &mut Tx| {
                drag_transformation(ui, &dragged, index, pitch, d)
            })
            .child(rune::icon(&GRIP, 16.0).key("grip").color(p.muted()));
        let mut item = row()
            .key(format!("transformation{index}"))
            .h(TRANSFORMATION_ROW)
            .pl(6.0)
            .pr(12.0)
            .gap(6.0)
            .center_y()
            .radius(p.k.radius.md)
            .child(handle)
            .child(
                text(name)
                    .key("name")
                    .font(p.k.ty.ui)
                    .color(p.text())
                    .grow(1.0)
                    .truncate(),
            )
            .child(
                icon_button(p, "remove", CLOSE, tr("Remove"))
                    .on_press(move |ui: &mut Ui| {
                        let id = remove.clone();
                        update_mode(ui, |mode| {
                            mode.transformations.retain(|candidate| *candidate != id)
                        });
                    })
                    .build(p.k)
                    .key("remove"),
            );
        if held {
            item = item.fill(p.k.c.layer_02);
        }
        rows.push(item);
    }
    if picker_open && !catalog.is_empty() {
        let available: Vec<&StatusTransformation> = catalog
            .iter()
            .filter(|transformation| !current.transformations.contains(&transformation.id))
            .collect();
        if available.is_empty() {
            rows.push(note(
                "all-included",
                tr("Every registered transformation is already included."),
                p.muted(),
            ));
        }
        for (index, transformation) in available.into_iter().enumerate() {
            let id = transformation.id.clone();
            rows.push(setting_row(
                p,
                &format!("available-transformation{index}"),
                &transformation.name,
                transformation.description.as_deref(),
                button(
                    p,
                    "add",
                    tr("Add"),
                    ButtonKind::Secondary,
                    move |ui: &mut Ui, _| {
                        let id = id.clone();
                        update_mode(ui, |mode| mode.transformations.push(id));
                    },
                ),
            ));
        }
    }
    if let Some(workspace) = workspace_status(ui, p) {
        rows.push(workspace);
    }
    card(p, "transformations-card", rows)
}

/// The TypeScript workspace behind custom transformations: setup, loading,
/// and failures with the action that recovers. Hidden once it is loaded.
fn workspace_status(ui: &Ui, p: Paint) -> Option<El> {
    if !ui.live() {
        return None;
    }
    let pane = &ui.modes;
    let (description, error, action) = if pane.workspace_receiver.is_some() {
        (
            tr("Preparing the TypeScript workspace and installing its dependencies…"),
            None,
            None,
        )
    } else if let Some(error) = pane.workspace_error.clone() {
        (
            tr("The TypeScript workspace could not be prepared."),
            Some(error),
            Some(tr("Retry setup")),
        )
    } else if !pane.config_exists {
        (
            tr("Create a TypeScript workspace to define reusable transformations."),
            None,
            Some(tr("Set up")),
        )
    } else {
        match pane.status.host_state {
            HostState::Active if pane.status.last_reload_error.is_none() => return None,
            HostState::Starting => (
                tr("Loading registered TypeScript transformations…"),
                None,
                None,
            ),
            _ => (
                tr("Registered TypeScript transformations are unavailable."),
                pane.status.last_reload_error.clone(),
                Some(tr("Retry")),
            ),
        }
    };
    let control = action.map_or_else(
        || el().key("none"),
        |label| {
            button(
                p,
                "workspace-action",
                label,
                ButtonKind::Secondary,
                |ui: &mut Ui, _| provision_workspace(ui),
            )
        },
    );
    let mut row = setting_row(
        p,
        "workspace",
        tr("Custom transformations"),
        Some(description),
        control,
    );
    if let Some(error) = error {
        row = col().key("workspace-status").child(row).child(note(
            "workspace-error",
            &error,
            p.danger(),
        ));
    }
    Some(row)
}

impl Ui {
    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn modes_application_error(&self) -> Option<&str> {
        self.modes.application_error.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Fixture

/// Representative modes: a chat mode with corrections, rewriting, and
/// transformations, and an empty one, as the GPUI Modes preview shows.
pub(super) fn fixture(ui: &mut Ui) {
    let mut chat = ui.settings.dictation_processing.default_mode.clone();
    chat.name = "Chat".into();
    chat.applications = vec!["Firefox".into(), "Slack".into()];
    chat.replacements = vec![
        TextReplacement {
            matched_phrase: "open code".into(),
            output: "OpenCode".into(),
        },
        TextReplacement {
            matched_phrase: "hex app".into(),
            output: "Hex".into(),
        },
    ];
    chat.transformations = vec!["smart-punctuation".into()];
    chat.post_processing.enabled = true;
    chat.post_processing.model = Some("anthropic/claude-sonnet-4-5".into());
    chat.post_processing.variant = Some("high".into());
    let mut code = ui.settings.dictation_processing.default_mode.clone();
    code.name = "Terminal".into();
    code.applications = vec!["WezTerm".into()];
    ui.settings.dictation_processing.modes = vec![chat, code];
    ui.modes.status.transformations = vec![
        StatusTransformation {
            id: "smart-punctuation".into(),
            name: "Smart punctuation".into(),
            description: Some("Curly quotes, dashes, and ellipses".into()),
        },
        StatusTransformation {
            id: "normalize-links".into(),
            name: "Normalize links".into(),
            description: None,
        },
    ];
    let app = |name: &str, path: &str| InstalledApplication {
        name: name.into(),
        bundle_id: None,
        path: PathBuf::from(path),
        icon: None,
    };
    ui.modes.applications = Applications::Loaded(vec![
        app("Firefox", "/usr/share/applications/firefox.desktop"),
        app("Slack", "/usr/share/applications/slack.desktop"),
        app("Visual Studio Code", "/usr/share/applications/code.desktop"),
        app(
            "WezTerm",
            "/usr/share/applications/org.wezfurlong.wezterm.desktop",
        ),
        app("Zed", "/usr/share/applications/dev.zed.Zed.desktop"),
    ]);
    ui.modes.select(Selection::Custom(0));
    ui.modes.editor = Editor::of(mode(&ui.settings, Selection::Custom(0)));
}
