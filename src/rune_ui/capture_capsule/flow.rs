//! What the capture capsule shows, decided from what happens: the
//! capture's stages from `capture_host`, the router's answer, and the keys
//! and clicks the capture input sends. Pure: no Rune, no clock of its own.
//! [`Flow::input`] and [`Flow::tick`] move it, [`Flow::pres`] says what the
//! capsule draws, and each returns the [`Action`]s the host carries out.

use scrawl_capture::capture::Stage;

/// A recording's chips close by themselves this long after a tap.
pub(crate) const CHIPS_OPEN_S: f64 = 3.5;
/// The chips stay at least this long even when work starts at once, so
/// they are seen (Scrawl's `REVEAL_MIN_MS`).
pub(crate) const CHIPS_MIN_S: f64 = 0.9;
/// The chips never fold under the pointer, and stay this much longer once
/// it leaves (Scrawl's `REVEAL_LINGER_MS`).
pub(crate) const CHIPS_LINGER_S: f64 = 1.2;
/// A rule's send can be taken back with Escape for this long
/// (Scrawl's `SMART_SEND_UNDO`).
pub(crate) const UNDO_S: f64 = 3.0;
/// The finished capsule leaves by itself after this long, unless its list
/// is open.
pub(crate) const DONE_HOLD_S: f64 = 25.0;
/// "Taken back" and errors that need nothing more leave sooner.
pub(crate) const BRIEF_HOLD_S: f64 = 4.0;
/// The most rows each list section shows.
pub(crate) const MAX_SUGGESTED: usize = 3;
pub(crate) const MAX_RECENT: usize = 5;

/// What a capture records, picked from the capsule's chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Mode {
    Area,
    Window,
    Screen,
}

impl Mode {
    pub(crate) const ALL: [Mode; 3] = [Mode::Area, Mode::Window, Mode::Screen];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Mode::Area => "Area",
            Mode::Window => "Window",
            Mode::Screen => "Screen",
        }
    }

    /// Area and Window start once something is picked on screen; Screen
    /// needs a Record press.
    pub(crate) fn shows_record(self) -> bool {
        self == Mode::Screen
    }
}

/// A session the list offers, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dest {
    pub(crate) name: String,
    pub(crate) why: String,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Hex hosts no router or capture shortcut yet; tests drive these"
    )
)]
impl Dest {
    pub(crate) fn new(name: impl Into<String>, why: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            why: why.into(),
        }
    }
}

/// The router's answer for a finished capture (Scrawl's design f: only a
/// rule ever sends; a guess suggests).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Hex hosts no router or capture shortcut yet; tests drive these"
    )
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// A rule matched, so it was sent to `to`.
    Rule {
        to: String,
        suggested: Vec<Dest>,
        recent: Vec<Dest>,
    },
    /// Guesses, best first; nothing was sent. `site` is what a rule
    /// would match ("problems.kitlangton.com").
    Guess {
        suggested: Vec<Dest>,
        recent: Vec<Dest>,
        site: Option<String>,
    },
    /// Nothing fits: recent sessions only.
    Nothing { recent: Vec<Dest> },
    /// The router never answered.
    NoAnswer { recent: Vec<Dest> },
}

/// What the capture engine says became of the bundle
/// (`scrawl_capture::delivery::Delivery`).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Hex hosts no router or capture shortcut yet; tests drive these"
    )
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Delivered {
    Sent { to: String },
    Queued { to: String },
}

/// How a send the capsule asked to take back ended (Hex's router undo).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Hex hosts no router or capture shortcut yet; tests drive these"
    )
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Undone {
    /// Taken back before the session saw it, or reverted.
    TakenBack,
    /// Too late: the session already acted on it.
    TooLate,
}

/// Everything that moves the capsule.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Hex hosts no router or capture shortcut yet; tests drive these"
    )
)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input {
    /// A capture's stage, from `capture_host`'s engine events.
    Stage(Stage),
    Delivered(Delivered),
    Routed(Route),
    /// ⌥⇧S held: the picker, or S stepping it to the next mode.
    Pick(Mode),
    /// The capture failed, and why, when the host knows.
    Failed(Option<String>),
    /// ⌥⇧S tapped while recording: the chips spring open.
    Chips,
    /// The recording got its first click, scroll, or stroke: the chips
    /// fold, once they have been seen.
    Worked,
    /// The capture shortcut while the picker is open: record what it's on.
    Record,
    /// ⌥⇧V: the list opens (or, when open, nothing).
    OpenList,
    /// What's typed into the open list's field.
    Query(String),
    /// Up and Down in the open list.
    Move(i32),
    /// Return (or ⇧Return, `always`) in the open list.
    Send {
        always: bool,
    },
    /// 1–9 in the open list: send to that row.
    SendRow(usize),
    Escape,
    Undone(Undone),
    /// The ✕, or anything that should just close it.
    Dismiss,
}

/// What the host should do about an input or a click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Start recording in this mode.
    Record(Mode),
    /// Switch the recording's mode.
    Switch(Mode),
    Finish,
    Cancel,
    /// Drop the recording without an outcome: a chip turned the capsule
    /// into the picker in its place.
    Discard,
    /// Take back the send to `to`.
    Undo {
        to: String,
    },
    /// Send the capture to `to`; `always` saves a rule for `site`.
    Send {
        to: String,
        always: Option<String>,
    },
}

/// The glyph before the finished capsule's label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lead {
    Check,
    Warning,
}

/// The second footer hint in the open list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Footer {
    /// esc keeps it copied.
    KeepCopied,
    /// esc keeps it where the rule sent it.
    Keep(String),
    /// ⇧⏎ always sends this site to the selected session.
    Always(String),
}

/// The list under the open capsule.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct List {
    pub(crate) suggested: Vec<Dest>,
    pub(crate) recent: Vec<Dest>,
    /// Across both sections, suggested first.
    pub(crate) selected: usize,
    pub(crate) footer: Footer,
}

impl List {
    pub(crate) fn rows(&self) -> impl Iterator<Item = &Dest> {
        self.suggested.iter().chain(&self.recent)
    }

    pub(crate) fn selected_dest(&self) -> Option<&Dest> {
        self.rows().nth(self.selected)
    }
}

/// The finished capsule: its lead, a quiet word on why, the Undo
/// countdown, the ⌥⇧V chip, and (open) the field with its list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Done {
    pub(crate) lead: Lead,
    pub(crate) text: String,
    pub(crate) why: Option<String>,
    /// Seconds left to take it back.
    pub(crate) undo: Option<u32>,
    /// The ⌥⇧V chip's label.
    pub(crate) ask: Option<String>,
    /// Open: what's typed into the field, and the list under it.
    pub(crate) open: Option<(String, List)>,
}

impl Done {
    pub(crate) fn plain(lead: Lead, text: impl Into<String>) -> Self {
        Self {
            lead,
            text: text.into(),
            why: None,
            undo: None,
            ask: None,
            open: None,
        }
    }
}

/// What the capsule draws.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pres {
    Hidden,
    Selecting { mode: Mode },
    Recording { chips: bool },
    Processing,
    Done(Done),
}

impl Pres {
    pub(crate) fn phase(&self) -> &'static str {
        match self {
            Pres::Hidden => "hidden",
            Pres::Selecting { .. } => "selecting",
            Pres::Recording { .. } => "recording",
            Pres::Processing => "processing",
            Pres::Done(_) => "done",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Hidden,
    Selecting(Mode),
    Recording {
        chips_until: Option<f64>,
    },
    Processing,
    /// Written, with no word from the router (yet).
    Saved {
        at: f64,
    },
    Routed(Routed),
    /// Sent elsewhere from the list, or a rule saved.
    Sent(Sent),
    Queued {
        to: String,
        at: f64,
    },
    TakenBack {
        at: f64,
    },
    TooLate {
        to: String,
        at: f64,
    },
    Failed {
        at: f64,
        why: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Routed {
    route: Route,
    /// When the rule sent it, for the Undo countdown.
    at: f64,
    open: Option<Open>,
}

#[derive(Clone, Debug, PartialEq)]
struct Open {
    query: String,
    selected: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct Sent {
    to: String,
    rule_saved: bool,
    at: f64,
}

/// The capsule's state machine.
#[derive(Clone, Debug)]
pub(crate) struct Flow {
    phase: Phase,
    /// The pointer is on the capsule.
    hovered: bool,
}

impl Default for Flow {
    fn default() -> Self {
        Self::new()
    }
}

impl Flow {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Hidden,
            hovered: false,
        }
    }

    /// The pointer came onto the capsule or left it.
    pub(crate) fn hover(&mut self, on: bool, now: f64) {
        self.hovered = on;
        if let (
            false,
            Phase::Recording {
                chips_until: Some(until),
            },
        ) = (on, &mut self.phase)
        {
            *until = until.max(now + CHIPS_LINGER_S);
        }
    }

    pub(crate) fn input(&mut self, input: Input, now: f64) -> Vec<Action> {
        let mut actions = Vec::new();
        let phase = std::mem::replace(&mut self.phase, Phase::Hidden);
        self.phase = match (phase, input) {
            (_, Input::Stage(stage)) => match stage {
                Stage::Recording => Phase::Recording { chips_until: None },
                Stage::Stopping | Stage::Transcribing | Stage::Rendering | Stage::Writing => {
                    Phase::Processing
                }
                Stage::Done => Phase::Saved { at: now },
                Stage::Cancelled => Phase::Hidden,
                Stage::Failed => Phase::Failed { at: now, why: None },
            },
            (_, Input::Failed(why)) => Phase::Failed {
                at: now,
                why: why.as_deref().map(short_reason),
            },
            (Phase::Recording { .. }, Input::Chips) => Phase::Recording {
                chips_until: Some(now + CHIPS_OPEN_S),
            },
            (Phase::Recording { chips_until }, Input::Worked) => Phase::Recording {
                chips_until: chips_until
                    .map(|until| until.min(until - CHIPS_OPEN_S + CHIPS_MIN_S).max(now)),
            },
            (Phase::Selecting(mode), Input::Record) => {
                if mode.shows_record() {
                    actions.push(Action::Record(mode));
                }
                Phase::Selecting(mode)
            }
            (_, Input::Pick(mode)) => Phase::Selecting(mode),
            (_, Input::Delivered(delivered)) => match delivered {
                Delivered::Sent { to } => Phase::Sent(Sent {
                    to,
                    rule_saved: false,
                    at: now,
                }),
                Delivered::Queued { to } => Phase::Queued { to, at: now },
            },
            (_, Input::Routed(route)) => Phase::Routed(Routed {
                route,
                at: now,
                open: None,
            }),
            (Phase::Routed(mut routed), Input::OpenList) => {
                if routed.open.is_none() {
                    // A guess opens on itself; a rule on the next best place.
                    let selected = usize::from(matches!(routed.route, Route::Rule { .. }));
                    routed.open = Some(Open {
                        query: String::new(),
                        selected,
                    });
                    clamp(&mut routed);
                }
                Phase::Routed(routed)
            }
            (Phase::Routed(mut routed), Input::Query(query)) => {
                if let Some(open) = &mut routed.open {
                    open.query = query;
                    open.selected = 0;
                }
                Phase::Routed(routed)
            }
            (Phase::Routed(mut routed), Input::Move(by)) => {
                if let Some(open) = &mut routed.open {
                    open.selected = (open.selected as i64 + i64::from(by)).max(0) as usize;
                }
                clamp(&mut routed);
                Phase::Routed(routed)
            }
            (Phase::Routed(routed), Input::Send { always }) => {
                let selected = routed.open.as_ref().map(|open| open.selected);
                send(routed, selected, always, now, &mut actions)
            }
            (Phase::Routed(routed), Input::SendRow(row)) => {
                send(routed, Some(row), false, now, &mut actions)
            }
            (Phase::Routed(mut routed), Input::Escape) => {
                if routed.open.take().is_some() {
                    Phase::Routed(routed)
                } else if let (Route::Rule { to, .. }, true) =
                    (&routed.route, now - routed.at < UNDO_S)
                {
                    actions.push(Action::Undo { to: to.clone() });
                    Phase::Routed(routed)
                } else {
                    Phase::Hidden
                }
            }
            (Phase::Sent(sent), Input::Escape) if now - sent.at < UNDO_S => {
                actions.push(Action::Undo {
                    to: sent.to.clone(),
                });
                Phase::Sent(sent)
            }
            (Phase::Recording { .. } | Phase::Selecting(_), Input::Escape) => {
                actions.push(Action::Cancel);
                Phase::Hidden
            }
            (_, Input::Undone(Undone::TakenBack)) => Phase::TakenBack { at: now },
            (phase, Input::Undone(Undone::TooLate)) => match phase {
                Phase::Sent(Sent { to, .. })
                | Phase::Routed(Routed {
                    route: Route::Rule { to, .. },
                    ..
                }) => Phase::TooLate { to, at: now },
                other => other,
            },
            (_, Input::Escape | Input::Dismiss) => Phase::Hidden,
            (phase, _) => phase,
        };
        actions
    }

    /// What a click on `id` does: the action for the host, after moving
    /// the capsule as the click shows.
    pub(crate) fn click(&mut self, id: super::surface::Id, now: f64) -> Vec<Action> {
        use super::surface::Id;
        match (&self.phase, id) {
            (Phase::Selecting(current), Id::Mode(mode)) => {
                if *current == mode && mode.shows_record() {
                    return vec![Action::Record(mode)];
                }
                self.phase = Phase::Selecting(mode);
                Vec::new()
            }
            (Phase::Selecting(mode), Id::Record) => vec![Action::Record(*mode)],
            (Phase::Recording { .. }, Id::Mode(mode)) => {
                if mode.shows_record() {
                    self.phase = Phase::Recording { chips_until: None };
                    vec![Action::Switch(mode)]
                } else {
                    self.phase = Phase::Selecting(mode);
                    vec![Action::Discard]
                }
            }
            (Phase::Recording { .. }, Id::Finish) => vec![Action::Finish],
            (Phase::Recording { .. } | Phase::Selecting(_), Id::Cancel | Id::Close) => {
                self.phase = Phase::Hidden;
                vec![Action::Cancel]
            }
            (_, Id::Undo) => self.input(Input::Escape, now),
            (_, Id::Ask) => self.input(Input::OpenList, now),
            (_, Id::Row(row)) => self.input(Input::SendRow(row), now),
            (_, Id::Close) => self.input(Input::Dismiss, now),
            _ => Vec::new(),
        }
    }

    /// Runs what time decides: the chips closing, the Undo window ending,
    /// the finished capsule leaving. True when one was due.
    pub(crate) fn tick(&mut self, now: f64) -> bool {
        match &mut self.phase {
            Phase::Recording { chips_until }
                if !self.hovered && chips_until.is_some_and(|t| now >= t) =>
            {
                *chips_until = None;
                true
            }
            phase => {
                let due = hold_until(phase).is_some_and(|at| now >= at);
                if due {
                    self.phase = Phase::Hidden;
                }
                due
            }
        }
    }

    /// When `tick` next has something to do.
    pub(crate) fn next_due(&self, now: f64) -> Option<f64> {
        let undo_at = |at: f64| {
            let left = UNDO_S - (now - at);
            // The countdown's next whole second.
            let step = left.rem_euclid(1.0);
            (left > 0.0).then_some(now + if step < 1e-6 { 1.0 } else { step })
        };
        let due = match &self.phase {
            Phase::Recording { chips_until } => chips_until.filter(|_| !self.hovered),
            Phase::Routed(Routed {
                route: Route::Rule { .. },
                at,
                open: None,
            }) => undo_at(*at),
            Phase::Sent(sent) => undo_at(sent.at),
            _ => None,
        };
        [due, hold_until(&self.phase)]
            .into_iter()
            .flatten()
            .min_by(f64::total_cmp)
    }

    /// What the capsule draws at `now`.
    pub(crate) fn pres(&self, now: f64) -> Pres {
        let undo_left = |at: f64| {
            let left = UNDO_S - (now - at);
            (left > 0.0).then(|| left.ceil() as u32)
        };
        match &self.phase {
            Phase::Hidden => Pres::Hidden,
            Phase::Selecting(mode) => Pres::Selecting { mode: *mode },
            Phase::Recording { chips_until } => Pres::Recording {
                chips: chips_until.is_some_and(|t| self.hovered || now < t),
            },
            Phase::Processing => Pres::Processing,
            Phase::Saved { .. } => Pres::Done(Done::plain(Lead::Check, "Saved")),
            Phase::Failed { why, .. } => Pres::Done(Done {
                why: why.clone(),
                ..Done::plain(Lead::Warning, "Couldn't save the capture")
            }),
            Phase::TakenBack { .. } => Pres::Done(Done::plain(Lead::Check, "Taken back")),
            Phase::TooLate { to, .. } => Pres::Done(Done {
                why: Some("too late to take back".into()),
                ..Done::plain(Lead::Check, format!("Sent to {to}"))
            }),
            Phase::Queued { to, .. } => Pres::Done(Done {
                why: Some(format!("couldn't reach {to}")),
                ..Done::plain(Lead::Check, "Copied")
            }),
            Phase::Sent(sent) => Pres::Done(Done {
                why: sent.rule_saved.then(|| "rule saved".into()),
                undo: undo_left(sent.at),
                ..Done::plain(Lead::Check, format!("Sent to {}", sent.to))
            }),
            Phase::Routed(routed) => Pres::Done(routed_done(routed, undo_left(routed.at))),
        }
    }
}

fn hold_until(phase: &Phase) -> Option<f64> {
    match phase {
        Phase::Saved { at } | Phase::Queued { at, .. } | Phase::TooLate { at, .. } => {
            Some(at + DONE_HOLD_S)
        }
        Phase::Sent(sent) => Some(sent.at + DONE_HOLD_S),
        Phase::Routed(Routed { at, open: None, .. }) => Some(at + DONE_HOLD_S),
        Phase::TakenBack { at } | Phase::Failed { at, .. } => Some(at + BRIEF_HOLD_S),
        _ => None,
    }
}

fn sections(route: &Route) -> (&[Dest], &[Dest]) {
    match route {
        Route::Rule {
            suggested, recent, ..
        }
        | Route::Guess {
            suggested, recent, ..
        } => (suggested, recent),
        Route::Nothing { recent } | Route::NoAnswer { recent } => (&[], recent),
    }
}

/// The list for `route`, filtered by `query`.
fn list(route: &Route, open: &Open) -> List {
    let query = open.query.to_lowercase();
    let keep = |rows: &[Dest], max: usize| -> Vec<Dest> {
        rows.iter()
            .filter(|dest| dest.name.to_lowercase().contains(&query))
            .take(max)
            .cloned()
            .collect()
    };
    let (suggested, recent) = sections(route);
    let footer = match route {
        Route::Rule { to, .. } => Footer::Keep(to.clone()),
        Route::Guess {
            site: Some(site), ..
        } => Footer::Always(site.clone()),
        _ => Footer::KeepCopied,
    };
    List {
        suggested: keep(suggested, MAX_SUGGESTED),
        recent: keep(recent, MAX_RECENT),
        selected: open.selected,
        footer,
    }
}

fn clamp(routed: &mut Routed) {
    let Some(open) = &routed.open else { return };
    let count = list(&routed.route, open).rows().count();
    if let Some(open) = &mut routed.open {
        open.selected = open.selected.min(count.saturating_sub(1));
    }
}

fn send(
    routed: Routed,
    row: Option<usize>,
    always: bool,
    now: f64,
    actions: &mut Vec<Action>,
) -> Phase {
    let Some(open) = &routed.open else {
        return Phase::Routed(routed);
    };
    let list = list(&routed.route, open);
    let Some(dest) = row.and_then(|row| list.rows().nth(row)) else {
        return Phase::Routed(routed);
    };
    let site = match (&routed.route, always) {
        (Route::Guess { site, .. }, true) => site.clone(),
        _ => None,
    };
    actions.push(Action::Send {
        to: dest.name.clone(),
        always: site.clone(),
    });
    Phase::Sent(Sent {
        to: dest.name.clone(),
        rule_saved: site.is_some(),
        at: now,
    })
}

fn routed_done(routed: &Routed, undo: Option<u32>) -> Done {
    let (suggested, _) = sections(&routed.route);
    let base = match &routed.route {
        Route::Rule { to, .. } => Done {
            why: Some("rule".into()),
            undo,
            ask: Some("Elsewhere…".into()),
            ..Done::plain(Lead::Check, format!("Sent to {to}"))
        },
        Route::Guess { .. } => {
            let ask = match suggested {
                [] => "Send to…".to_owned(),
                [top] => format!("→ {}?", top.name),
                [top, rest @ ..] => format!("→ {}? +{}", top.name, rest.len()),
            };
            Done {
                ask: Some(ask),
                ..Done::plain(Lead::Check, "Copied")
            }
        }
        Route::Nothing { .. } => Done {
            ask: Some("Send to…".into()),
            ..Done::plain(Lead::Check, "Copied")
        },
        Route::NoAnswer { .. } => Done {
            why: Some("the router didn't answer".into()),
            ask: Some("Send to…".into()),
            ..Done::plain(Lead::Check, "Copied")
        },
    };
    match &routed.open {
        // Open, the field takes the chips' place.
        Some(open) => Done {
            why: None,
            undo: None,
            ask: None,
            open: Some((open.query.clone(), list(&routed.route, open))),
            ..base
        },
        None => base,
    }
}

/// A host error (`{error:#}`, outer context first) as the capsule's reason:
/// the root cause on its first line, kept to what fits beside the message.
fn short_reason(error: &str) -> String {
    const MAX: usize = 32;
    let line = error.lines().next().unwrap_or_default().trim();
    let cause = line.rsplit(": ").next().unwrap_or(line).trim();
    if cause.chars().count() <= MAX {
        return cause.to_owned();
    }
    let cut: String = cause.chars().take(MAX - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guess() -> Route {
        Route::Guess {
            suggested: vec![
                Dest::new("SEER", "guess · problems.kitlangton.com"),
                Dest::new("HEX", "last sent"),
            ],
            recent: vec![Dest::new("HOME", "yesterday")],
            site: Some("problems.kitlangton.com".into()),
        }
    }

    fn rule() -> Route {
        Route::Rule {
            to: "SEER".into(),
            suggested: vec![
                Dest::new("SEER", "sent · rule"),
                Dest::new("HEX", "last sent"),
            ],
            recent: vec![Dest::new("HOME", "yesterday")],
        }
    }

    fn done(flow: &Flow, now: f64) -> Done {
        match flow.pres(now) {
            Pres::Done(done) => done,
            other => panic!("not done: {other:?}"),
        }
    }

    #[test]
    fn stages_walk_the_capsule_through_a_capture() {
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        assert_eq!(flow.pres(0.0), Pres::Recording { chips: false });
        flow.input(Input::Chips, 1.0);
        assert_eq!(flow.pres(1.0), Pres::Recording { chips: true });
        assert!(flow.tick(1.0 + CHIPS_OPEN_S));
        assert_eq!(flow.pres(5.0), Pres::Recording { chips: false });
        for stage in [Stage::Stopping, Stage::Transcribing, Stage::Writing] {
            flow.input(Input::Stage(stage), 6.0);
            assert_eq!(flow.pres(6.0), Pres::Processing);
        }
        flow.input(Input::Stage(Stage::Done), 7.0);
        assert_eq!(done(&flow, 7.0).text, "Saved");
        assert!(flow.tick(7.0 + DONE_HOLD_S));
        assert_eq!(flow.pres(40.0), Pres::Hidden);
    }

    #[test]
    fn chips_fold_once_seen_when_work_starts_but_never_under_the_pointer() {
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Chips, 0.0);
        flow.input(Input::Worked, 0.2);
        assert!(!flow.tick(0.5), "seen for CHIPS_MIN_S first");
        assert!(flow.tick(CHIPS_MIN_S));
        assert_eq!(flow.pres(1.0), Pres::Recording { chips: false });

        flow.input(Input::Chips, 2.0);
        flow.hover(true, 3.0);
        assert_eq!(flow.next_due(5.0), None);
        assert!(!flow.tick(2.0 + CHIPS_OPEN_S + 1.0));
        assert_eq!(flow.pres(7.0), Pres::Recording { chips: true });
        flow.hover(false, 7.0);
        assert!(!flow.tick(7.0 + CHIPS_LINGER_S - 0.1));
        assert!(flow.tick(7.0 + CHIPS_LINGER_S));
    }

    #[test]
    fn the_shortcut_records_the_pickers_screen_and_a_chip_discards_the_recording() {
        let mut flow = Flow::new();
        flow.input(Input::Pick(Mode::Screen), 0.0);
        assert_eq!(
            flow.input(Input::Record, 0.5),
            [Action::Record(Mode::Screen)]
        );
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Chips, 0.0);
        assert_eq!(
            flow.click(super::super::surface::Id::Mode(Mode::Window), 0.5),
            [Action::Discard]
        );
        assert_eq!(flow.pres(0.5), Pres::Selecting { mode: Mode::Window });
    }

    #[test]
    fn a_rule_counts_down_its_undo_and_escape_takes_it_back() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule()), 10.0);
        let at = |t| done(&flow, t).undo;
        assert_eq!(at(10.0), Some(3));
        assert_eq!(at(11.2), Some(2));
        assert_eq!(at(12.5), Some(1));
        assert_eq!(at(13.0), None);
        assert_eq!(done(&flow, 10.0).text, "Sent to SEER");
        assert_eq!(done(&flow, 10.0).why.as_deref(), Some("rule"));
        assert!((flow.next_due(10.0).unwrap() - 11.0).abs() < 1e-6);
        let actions = flow.input(Input::Escape, 11.0);
        assert_eq!(actions, vec![Action::Undo { to: "SEER".into() }]);
        flow.input(Input::Undone(Undone::TakenBack), 11.4);
        assert_eq!(done(&flow, 11.4).text, "Taken back");
        // After the window, Escape just closes.
        let mut late = Flow::new();
        late.input(Input::Routed(rule()), 0.0);
        assert!(late.input(Input::Escape, 3.5).is_empty());
        assert_eq!(late.pres(3.5), Pres::Hidden);
    }

    #[test]
    fn a_guess_asks_and_the_list_sends_where_its_selected() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(guess()), 0.0);
        assert_eq!(done(&flow, 0.0).ask.as_deref(), Some("→ SEER? +1"));
        flow.input(Input::OpenList, 1.0);
        let open = done(&flow, 1.0);
        let (query, list) = open.open.expect("open");
        assert_eq!(query, "");
        assert_eq!(list.selected_dest().unwrap().name, "SEER");
        assert_eq!(
            list.footer,
            Footer::Always("problems.kitlangton.com".into())
        );
        assert!(open.ask.is_none(), "the field takes the chip's place");
        flow.input(Input::Query("ho".into()), 1.5);
        let (_, list) = done(&flow, 1.5).open.unwrap();
        assert_eq!(list.rows().count(), 1);
        let actions = flow.input(Input::Send { always: false }, 2.0);
        assert_eq!(
            actions,
            vec![Action::Send {
                to: "HOME".into(),
                always: None
            }]
        );
        assert_eq!(done(&flow, 2.0).text, "Sent to HOME");
        assert_eq!(done(&flow, 2.0).undo, Some(3));
    }

    #[test]
    fn shift_return_saves_a_rule_for_the_site() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(guess()), 0.0);
        flow.input(Input::OpenList, 0.5);
        let actions = flow.input(Input::Send { always: true }, 1.0);
        assert_eq!(
            actions,
            vec![Action::Send {
                to: "SEER".into(),
                always: Some("problems.kitlangton.com".into())
            }]
        );
        let shown = done(&flow, 1.0);
        assert_eq!(shown.why.as_deref(), Some("rule saved"));
    }

    #[test]
    fn escape_in_the_list_keeps_it_where_it_was() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule()), 0.0);
        flow.input(Input::OpenList, 4.0);
        let (_, list) = done(&flow, 4.0).open.unwrap();
        assert_eq!(
            list.selected_dest().unwrap().name,
            "HEX",
            "a rule opens on the next"
        );
        assert_eq!(list.footer, Footer::Keep("SEER".into()));
        assert!(flow.next_due(4.0).is_none(), "an open list stays");
        assert!(flow.input(Input::Escape, 5.0).is_empty());
        assert!(done(&flow, 5.0).open.is_none());
    }

    #[test]
    fn moving_stays_inside_the_list() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(guess()), 0.0);
        flow.input(Input::OpenList, 0.0);
        flow.input(Input::Move(-4), 0.0);
        assert_eq!(done(&flow, 0.0).open.unwrap().1.selected, 0);
        flow.input(Input::Move(9), 0.0);
        assert_eq!(done(&flow, 0.0).open.unwrap().1.selected, 2);
    }

    #[test]
    fn the_router_not_answering_still_offers_the_list() {
        let mut flow = Flow::new();
        flow.input(
            Input::Routed(Route::NoAnswer {
                recent: vec![Dest::new("HOME", "yesterday")],
            }),
            0.0,
        );
        let shown = done(&flow, 0.0);
        assert_eq!(shown.why.as_deref(), Some("the router didn't answer"));
        assert_eq!(shown.ask.as_deref(), Some("Send to…"));
    }

    #[test]
    fn cancel_and_failure() {
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        assert_eq!(flow.input(Input::Escape, 1.0), vec![Action::Cancel]);
        assert_eq!(flow.pres(1.0), Pres::Hidden);
        flow.input(Input::Stage(Stage::Failed), 2.0);
        assert_eq!(done(&flow, 2.0).lead, Lead::Warning);
        flow.tick(2.0 + BRIEF_HOLD_S);
        assert_eq!(flow.pres(9.0), Pres::Hidden);
        flow.input(
            Input::Failed(Some("starting the capture: the window closed".into())),
            10.0,
        );
        assert_eq!(done(&flow, 10.0).why.as_deref(), Some("the window closed"));
        assert_eq!(
            short_reason("Screen Recording permission is off for Hex"),
            "Screen Recording permission is…"
        );
    }
}
