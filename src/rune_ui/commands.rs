//! The Commands pane (macOS): the voice command catalog grouped by task,
//! search, a detail with the phrase's task, context, and aliases, the
//! personal `hex.config.ts` workspace, and the commands opt-in with its
//! microphone confirmation. The catalog and workspace come from the host as
//! [`PlatformState`](super::platform::PlatformState) data.

use rune::{El, TextEdit, Tx, col, el, row, text};
use rune_kit::{Switch, TextInput};

use super::Ui;
use super::controls::{ButtonKind, DESCRIPTION, Paint, button, card, setting_row};
use super::lists::{
    detail_header, detail_title, error_panel, fact, group_label, list_row, message, placeholder,
    split,
};
use super::platform::{CatalogCommand, CommandModel, PlatformRequest, Workspace};
use crate::commands::CommandScope;
use crate::desktop_i18n::tr;

#[derive(Default)]
pub(super) struct CommandsPane {
    pub(super) search: TextEdit,
    pub(super) selected: Option<String>,
    /// Commands are off and the microphone is released: enabling them asks
    /// to keep the microphone ready first.
    pub(super) confirm_enable: bool,
    prompt_copied: bool,
}

fn matches(command: &CatalogCommand, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let scope = match &command.scope {
        CommandScope::Sleeping | CommandScope::Global => "global",
        CommandScope::Application(name) => name,
        CommandScope::Browser(host) => host,
    };
    command.phrase.to_lowercase().contains(query)
        || command.description.to_lowercase().contains(query)
        || command.task.to_lowercase().contains(query)
        || scope.to_lowercase().contains(query)
        || command
            .aliases
            .iter()
            .any(|alias| alias.to_lowercase().contains(query))
}

fn scope_label(scope: &CommandScope) -> String {
    match scope {
        CommandScope::Sleeping | CommandScope::Global => tr("Global").into(),
        CommandScope::Application(application) => application.clone(),
        CommandScope::Browser(host) => host.clone(),
    }
}

/// The task heading a command is listed under.
fn task_label(task: &str) -> &str {
    match task {
        "Voice control" => tr("Voice control"),
        "Dictation" => tr("Dictation"),
        "Meetings" => tr("Meetings"),
        "Open applications" => tr("Open applications"),
        "Open websites" => tr("Open websites"),
        "Browser navigation" => tr("Browser navigation"),
        "Keyboard" => tr("Keyboard"),
        "Application navigation" => tr("Application navigation"),
        "Other" => tr("Other"),
        other => other,
    }
}

fn set_enabled(ui: &mut Ui, enabled: bool) {
    if enabled && ui.settings.release_microphone_while_idle {
        ui.commands.confirm_enable = true;
        return;
    }
    ui.commands.confirm_enable = false;
    let mut candidate = ui.drafted();
    match candidate.set_commands_enabled(enabled) {
        Ok(()) => {
            ui.commit(candidate);
        }
        Err(error) => ui.error = Some(format!("{error:#}")),
    }
}

fn keep_ready_and_enable(ui: &mut Ui) {
    let mut candidate = ui.drafted();
    candidate.keep_microphone_ready_and_enable_commands();
    if ui.commit(candidate) {
        ui.commands.confirm_enable = false;
    }
}

fn agent_prompt(workspace: &str) -> String {
    format!(
        "Help me configure my Hex custom voice commands.\n\n\
         Workspace: {workspace}\n\
         Config: {workspace}/hex.config.ts\n\n\
         Read AGENTS.md and .agents/skills/personal-commands/SKILL.md in the workspace \
         before editing. Ask me what commands or dictation phrases I want, update \
         hex.config.ts, and run `bun run check` from the workspace when finished. You \
         may add npm packages when a command needs them."
    )
}

/// The header actions (workspace, prompt, opt-in) and the list + detail body.
pub(super) fn pane(ui: &Ui, p: Paint) -> (Vec<El>, El) {
    let state = &ui.platform;
    let mut actions = Vec::new();
    actions.push(match state.workspace {
        Workspace::Missing => button(
            p,
            "commands-create-config",
            tr("Create config"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| ui.ask(PlatformRequest::CreateCommandsConfig),
        ),
        Workspace::Creating => text(tr("Creating config…"))
            .key("commands-creating")
            .font(DESCRIPTION)
            .color(p.muted()),
        Workspace::Ready | Workspace::Error => button(
            p,
            "commands-open-config",
            tr("Edit config"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| ui.ask(PlatformRequest::OpenCommandsConfig),
        ),
    });
    if matches!(state.workspace, Workspace::Ready | Workspace::Error) {
        actions.push(button(
            p,
            "commands-copy-prompt",
            if ui.commands.prompt_copied {
                tr("Copied")
            } else {
                tr("Copy agent prompt")
            },
            ButtonKind::Secondary,
            |ui: &mut Ui, tx: &mut Tx| {
                tx.copy(agent_prompt(&ui.platform.workspace_path));
                ui.commands.prompt_copied = true;
            },
        ));
    }
    actions.push(
        el().key("commands-divider")
            .w(p.hair)
            .h(20.0)
            .ml(4.0)
            .mr(4.0)
            .fill(p.k.c.border_base),
    );
    actions.push(if ui.commands.confirm_enable {
        button(
            p,
            "confirm-enable-commands",
            tr("Keep microphone ready & enable commands"),
            ButtonKind::Primary,
            |ui: &mut Ui, _| keep_ready_and_enable(ui),
        )
    } else {
        row()
            .key("commands-enabled")
            .role("toggle")
            .label(tr("Voice commands"))
            .gap(8.0)
            .center_y()
            .on_press(|ui: &mut Ui| {
                let enabled = !ui.settings.commands_enabled;
                set_enabled(ui, enabled);
            })
            .child(
                text(tr("Voice commands"))
                    .key("label")
                    .font(DESCRIPTION)
                    .color(p.muted()),
            )
            .child(
                Switch::new(ui.settings.commands_enabled)
                    .build(p.k)
                    .key("switch"),
            )
    });

    let query = ui.commands.search.text.trim().to_lowercase();
    let visible: Vec<&CatalogCommand> = state
        .commands
        .iter()
        .filter(|command| matches(command, &query))
        .collect();
    let selected = ui
        .commands
        .selected
        .as_deref()
        .filter(|id| visible.iter().any(|command| command.id == *id))
        .or_else(|| visible.first().map(|command| command.id.as_str()));
    let mut tasks: Vec<&str> = Vec::new();
    for command in &visible {
        if !tasks.contains(&command.task.as_str()) {
            tasks.push(&command.task);
        }
    }
    let mut list = col().key("commands-list");
    if visible.is_empty() {
        list = list.child(message(
            p,
            "commands-empty",
            if query.is_empty() {
                tr("No commands yet.")
            } else {
                tr("No matching commands.")
            },
            None,
        ));
    }
    for (group, task) in tasks.iter().enumerate() {
        list = list.child(group_label(
            p,
            &format!("task{group}"),
            task_label(task),
            group == 0,
        ));
        for command in visible.iter().filter(|command| command.task == *task) {
            let id = command.id.clone();
            list = list.child(list_row(
                p,
                &format!("command-{}", command.id),
                &command.phrase,
                Vec::new(),
                &command.description,
                selected == Some(command.id.as_str()),
                move |ui: &mut Ui| ui.commands.selected = Some(id.clone()),
            ));
        }
    }
    let search = TextInput::new(&ui.commands.search)
        .placeholder(tr("Search commands"))
        .leading_icon(p.k.glyph.search)
        .on_edit(|ui: &mut Ui| &mut ui.commands.search)
        .build(p.k)
        .key("commands-search");
    let column = col()
        .key("commands-column")
        .h_full()
        .gap(8.0)
        .child(search)
        .child(super::scroll_body("commands-scroll", list));
    let detail = detail(
        ui,
        p,
        selected.and_then(|id| state.commands.iter().find(|c| c.id == id)),
    );
    let body = split(p, "commands-body", column, detail);

    let mut notices = Vec::new();
    if let Some(error) = state
        .workspace_error
        .as_deref()
        .filter(|_| state.workspace == Workspace::Error)
    {
        notices.push(error_panel(
            p,
            "commands-error",
            tr("Personal commands are not loading."),
            error,
        ));
    }
    if let Some((title, description)) = model_notice(ui) {
        notices.push(model_notice_panel(ui, p, title, description));
    }
    if notices.is_empty() {
        return (actions, body);
    }
    (
        actions,
        col()
            .key("commands-pane")
            .grow(1.0)
            .shrink()
            .child(
                col()
                    .key("notices")
                    .px(24.0)
                    .pb(12.0)
                    .gap(8.0)
                    .children(notices),
            )
            .child(body),
    )
}

/// The voice command model's notice while commands are on and it is not
/// ready. Hotkey dictation never waits for it.
pub(super) fn model_notice(ui: &Ui) -> Option<(&'static str, &'static str)> {
    if !ui.platform.caps.commands || !ui.settings.commands_enabled {
        return None;
    }
    match ui.platform.command_model {
        CommandModel::Disabled | CommandModel::Ready => None,
        CommandModel::Loading => Some((
            tr("Preparing voice commands"),
            tr("The command model is loading. Hotkey dictation does not require it."),
        )),
        CommandModel::Failed => Some((
            tr("Voice commands are unavailable"),
            tr(
                "The command model could not load. Retry or turn off commands; hotkey dictation is unaffected.",
            ),
        )),
    }
}

/// The notice with Turn off commands, and Retry once the model failed.
pub(super) fn model_notice_panel(ui: &Ui, p: Paint, title: &str, description: &str) -> El {
    let mut actions = row().key("actions").gap(8.0).center_y();
    if ui.platform.command_model == CommandModel::Failed {
        actions = actions.child(button(
            p,
            "retry-commands",
            tr("Retry"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| ui.ask(PlatformRequest::RetryCommandModel),
        ));
    }
    actions = actions.child(button(
        p,
        "disable-commands",
        tr("Turn off commands"),
        ButtonKind::Secondary,
        |ui: &mut Ui, _| set_enabled(ui, false),
    ));
    card(
        p,
        "command-model-notice",
        vec![setting_row(
            p,
            "command-model",
            title,
            Some(description),
            actions,
        )],
    )
}

fn detail(ui: &Ui, p: Paint, command: Option<&CatalogCommand>) -> El {
    let Some(command) = command else {
        return placeholder(p, "command-detail", tr("Select a command."));
    };
    let aliases = if command.aliases.is_empty() {
        tr("None").to_string()
    } else {
        command.aliases.join("\n")
    };
    let context = ui
        .platform
        .context
        .clone()
        .unwrap_or_else(|| tr("Unavailable").into());
    let facts = col()
        .key("facts")
        .gap(10.0)
        .pt(16.0)
        .child(fact(p, "task", tr("Task"), task_label(&command.task)))
        .child(fact(
            p,
            "context",
            tr("Context"),
            &scope_label(&command.scope),
        ))
        .child(fact(p, "aliases", tr("Aliases"), &aliases))
        .child(fact(p, "current-context", tr("Current context"), &context));
    col()
        .key("command-detail")
        .grow(1.0)
        .shrink()
        .px(24.0)
        .pb(16.0)
        .child(detail_header(
            detail_title(p, &command.phrase),
            vec![
                text(command.description.clone())
                    .key("description")
                    .font(DESCRIPTION)
                    .color(p.muted()),
            ],
            Vec::new(),
        ))
        .child(facts)
}
