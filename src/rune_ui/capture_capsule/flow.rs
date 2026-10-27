//! What the capture capsule shows, decided from what happens: the
//! capture's stages from `capture_host`, the router's answer, and the keys
//! and clicks the capture input sends. Pure: no Rune, no clock of its own.
//! [`Flow::input`] and [`Flow::tick`] move it, [`Flow::pres`] says what the
//! capsule draws, and each returns the [`Action`]s the host carries out.

use scrawl_capture::capture::Stage;

use super::tuning;

/// A rule's send can be taken back with Escape for this long (Scrawl's
/// `SMART_SEND_UNDO`, and the router's `UNDO`).
pub(crate) fn undo_s() -> f64 {
    tuning::get().undo_s
}

/// The chips stay at least this long even when work starts at once, so
/// they are seen (Scrawl's `REVEAL_MIN_MS`).
pub(crate) const CHIPS_MIN_S: f64 = 0.9;
/// The chips never fold under the pointer, and stay this much longer once
/// it leaves (Scrawl's `REVEAL_LINGER_MS`).
pub(crate) const CHIPS_LINGER_S: f64 = 1.2;
/// How to send a kept capture later, which "Kept" says.
pub(crate) const SEND_LATER: &str = "hex capture send latest";
/// The most rows each list section shows.
pub(crate) const MAX_SUGGESTED: usize = 3;
pub(crate) const MAX_RECENT: usize = 5;
/// An ask's route chips (Scrawl's `jev::CHIPS`).
pub(crate) const MAX_ROUTES: usize = 4;
/// Session names in the capsule are cut to this many characters.
const ROUTE_NAME_MAX: usize = 18;

/// What a capture records, picked from the capsule's chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Mode {
    Area,
    Window,
    Screen,
    /// A cabled iPhone, offered only while one is.
    Iphone,
}

impl Mode {
    /// The chips, left to right: iPhone last, and only with one cabled.
    pub(crate) fn chips(phone: bool) -> impl Iterator<Item = Mode> {
        [Mode::Area, Mode::Window, Mode::Screen]
            .into_iter()
            .chain(phone.then_some(Mode::Iphone))
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Mode::Area => "Area",
            Mode::Window => "Window",
            Mode::Screen => "Screen",
            Mode::Iphone => "iPhone",
        }
    }

    /// Area and Window start once something is picked on screen; Screen
    /// and iPhone need a Record press.
    pub(crate) fn shows_record(self) -> bool {
        matches!(self, Mode::Screen | Mode::Iphone)
    }
}

/// A session the list offers, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dest {
    pub(crate) name: String,
    pub(crate) why: String,
    /// The session id a send names, so two sessions with one title never
    /// collide.
    pub(crate) session: String,
}

impl Dest {
    /// A session known by its title alone, which the host resolves.
    pub(crate) fn new(name: impl Into<String>, why: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            session: name.clone(),
            name,
            why: why.into(),
        }
    }

    pub(crate) fn session(self, session: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            ..self
        }
    }
}

/// The router's answer for a finished capture (Scrawl's design f: only a
/// rule ever sends; a guess suggests).
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
    /// OpenCode's sessions couldn't be read, so there's no list: `why` in
    /// a few words.
    Unreachable { why: String },
}

/// What the capture engine says became of the bundle
/// (`scrawl_capture::delivery::Delivery`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Delivered {
    Sent { to: String },
    Queued { to: String },
}

/// How a send the capsule asked to take back ended (Hex's router undo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Undone {
    /// Taken back before the session saw it, or reverted.
    TakenBack,
    /// Too late: the session already acted on it.
    TooLate,
}

/// What failed, for the finished capsule's headline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The capture never started recording.
    Start,
    /// It recorded, but wasn't written.
    Save,
    /// The router couldn't send it to this session.
    Send(String),
}

/// Everything that moves the capsule.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input {
    /// A capture's stage, from `capture_host`'s engine events.
    Stage(Stage),
    Delivered(Delivered),
    Routed(Route),
    /// Something failed, and why, when the host knows.
    Failed(Failure, Option<String>),
    /// An iPhone was cabled (true) or unplugged.
    Phone(bool),
    /// ⌥⇧S tapped while recording: the chips spring open.
    Chips,
    /// The recording got its first click, scroll, or stroke: the chips
    /// fold, once they have been seen.
    Worked,
    /// The capture shortcut while the picker is open: record what it's on.
    Record,
    /// ⌥⇧V: the list opens (or, when open, nothing).
    OpenList,
    /// Typed into the open list's field.
    Type(String),
    /// Backspace in the open list's field.
    Backspace,
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
    /// The recording it shows left the snapshot with no outcome: a chip's
    /// discard (the picker is already up) or `hex capture discard`.
    Gone,
    /// A mouse button went down while the list was open: off the capsule,
    /// the list closes, so typing goes back to the app in front.
    ClickAway,
    /// The written capture has no words: the microphone heard no speech.
    NothingHeard,
}

/// What the host should do about an input or a click.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// Start recording in this mode: the capsule's Record, or the capture
    /// shortcut while it picks. The picker turns it into [`Self::Start`]
    /// on what it shows; without one, it records the pointer's display.
    Record(Mode),
    /// Start recording what the picker picked on screen.
    Start(crate::capture_host::CaptureTarget),
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
    /// Take back the send to `from` and send to the session `to` instead:
    /// the runner-up chip beside a rule's send.
    Redirect {
        from: String,
        to: String,
    },
    /// Send the capture to the session `to`; `always` saves a rule for
    /// `site`.
    Send {
        to: String,
        always: Option<String>,
    },
    /// Paste the capture into the app in front.
    Paste,
    /// Open the capture to look it over.
    Inspect,
}

/// The glyph before the finished capsule's label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lead {
    Check,
    Warning,
    /// A send waiting out its Undo: the seconds left, in a ring.
    Countdown(u32),
}

/// A moment the capsule marks with a sound ([`Flow::take_cues`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cue {
    /// A capture started recording.
    Start,
    /// It stopped and is being written.
    Stop,
    /// A send's Undo ran out: it went.
    Sent,
    /// A send was taken back, or a recording cancelled.
    TakenBack,
    Failed,
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
/// countdown, the route chips, the ⌥⇧V chip, and (open) the field with its
/// list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Done {
    pub(crate) lead: Lead,
    pub(crate) text: String,
    pub(crate) why: Option<String>,
    /// Seconds left to take it back.
    pub(crate) undo: Option<u32>,
    /// When the Undo window ends, in the flow's seconds: the countdown's
    /// drain runs to it.
    pub(crate) undo_ends: Option<f64>,
    /// The route chips' labels: a rule's runner-up, or an ask's sessions,
    /// best first ("Send to SEER", "HEX", …).
    pub(crate) routes: Vec<String>,
    /// The ⌥⇧V chip's label.
    pub(crate) ask: Option<String>,
    /// Its keycaps, Paste Last's shortcut as saved when shown, so a change
    /// lays the chip out again.
    pub(crate) ask_keys: Vec<String>,
    /// Open: what's typed into the field, and the list under it.
    pub(crate) open: Option<(String, List)>,
    /// A written capture: Paste and Inspect, as Scrawl's capsule offers.
    pub(crate) hand_off: bool,
}

impl Done {
    pub(crate) fn plain(lead: Lead, text: impl Into<String>) -> Self {
        Self {
            lead,
            text: text.into(),
            why: None,
            undo: None,
            undo_ends: None,
            routes: Vec::new(),
            ask: None,
            ask_keys: Vec::new(),
            open: None,
            hand_off: false,
        }
    }
}

/// What the capsule draws.
#[expect(
    clippy::large_enum_variant,
    reason = "a few live at a time, and the finished capsule is most of what's drawn"
)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pres {
    Hidden,
    /// `phone`: an iPhone is cabled, so it has a chip.
    Selecting {
        mode: Mode,
        phone: bool,
    },
    Recording {
        chips: bool,
        phone: bool,
    },
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
    /// Taken back with nowhere else to offer.
    TakenBack {
        at: f64,
    },
    /// Dismissed after a take-back: the bundle stays, sent nowhere.
    Kept {
        at: f64,
    },
    TooLate {
        to: String,
        at: f64,
    },
    Failed {
        at: f64,
        what: Failure,
        why: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Routed {
    route: Route,
    /// When the rule sent it, for the Undo countdown.
    at: f64,
    open: Option<Open>,
    /// An Undo answered: it stays, offering the list again, until a pick
    /// or a dismissal, and never leaves by itself.
    taken_back: bool,
    /// The Undo came too late: this session already has it, and a pick
    /// sends a copy elsewhere.
    already_with: Option<String>,
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
    /// The route it was picked from, offered again when the send is taken
    /// back.
    back: Option<Route>,
}

/// The capsule's state machine.
#[derive(Clone, Debug)]
pub(crate) struct Flow {
    phase: Phase,
    /// The pointer is on the capsule.
    hovered: bool,
    /// An iPhone is cabled.
    phone: bool,
    /// The iPhone chip asked to switch, and its new recording hasn't shown:
    /// a failure now is that start's, not the discarded capture's save.
    switching: bool,
    /// Sounds due, in order, for the host to play.
    cues: Vec<Cue>,
    /// The send (by when it started) whose Undo was asked for or whose
    /// [`Cue::Sent`] already sounded.
    settled_send: Option<f64>,
    /// The capture shown has no words.
    silent: bool,
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
            phone: false,
            switching: false,
            cues: Vec::new(),
            settled_send: None,
            silent: false,
        }
    }

    /// The sounds due since the last call.
    pub(crate) fn take_cues(&mut self) -> Vec<Cue> {
        std::mem::take(&mut self.cues)
    }

    /// The cue for moving from `before` to the current phase, if any.
    fn cue(&mut self, before: &Phase, actions: &[Action]) {
        let cue = match (before, &self.phase) {
            (Phase::Recording { .. }, Phase::Recording { .. }) => None,
            (_, Phase::Recording { .. }) => Some(Cue::Start),
            (Phase::Recording { .. }, Phase::Processing) => Some(Cue::Stop),
            (Phase::Failed { .. }, Phase::Failed { .. }) => None,
            (_, Phase::Failed { .. }) => Some(Cue::Failed),
            (Phase::TakenBack { .. }, Phase::TakenBack { .. }) => None,
            (_, Phase::TakenBack { .. }) => Some(Cue::TakenBack),
            (
                Phase::Routed(Routed {
                    taken_back: true, ..
                }),
                _,
            ) => None,
            (
                _,
                Phase::Routed(Routed {
                    taken_back: true,
                    already_with: None,
                    ..
                }),
            ) => Some(Cue::TakenBack),
            (Phase::Recording { .. } | Phase::Selecting(_), Phase::Hidden)
                if actions.contains(&Action::Cancel) =>
            {
                Some(Cue::TakenBack)
            }
            _ => None,
        };
        self.cues.extend(cue);
        if actions.iter().any(|a| matches!(a, Action::Undo { .. })) {
            self.settled_send = self.counting_since();
        }
    }

    /// When the send now waiting out its Undo started.
    fn counting_since(&self) -> Option<f64> {
        match &self.phase {
            Phase::Routed(Routed {
                route: Route::Rule { .. },
                at,
                ..
            })
            | Phase::Sent(Sent { at, .. }) => Some(*at),
            _ => None,
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
        let before = self.phase.clone();
        let actions = self.step(input, now);
        self.cue(&before, &actions);
        actions
    }

    fn step(&mut self, input: Input, now: f64) -> Vec<Action> {
        let mut actions = Vec::new();
        if input == Input::NothingHeard {
            self.silent = true;
            return actions;
        }
        if input == Input::Stage(Stage::Recording) {
            self.silent = false;
        }
        if let Input::Phone(phone) = input {
            self.phone = phone;
            if !phone && self.phase == Phase::Selecting(Mode::Iphone) {
                self.phase = Phase::Selecting(Mode::Screen);
            }
            return actions;
        }
        let switching = matches!(input, Input::Stage(_) | Input::Failed(..) | Input::Gone)
            && std::mem::take(&mut self.switching);
        let phase = std::mem::replace(&mut self.phase, Phase::Hidden);
        self.phase = match (phase, input) {
            (_, Input::Stage(stage)) => match stage {
                Stage::Recording => Phase::Recording { chips_until: None },
                Stage::Stopping | Stage::Transcribing | Stage::Rendering | Stage::Writing => {
                    Phase::Processing
                }
                Stage::Done => Phase::Saved { at: now },
                Stage::Cancelled => Phase::Hidden,
                Stage::Failed => Phase::Failed {
                    at: now,
                    what: Failure::Save,
                    why: None,
                },
            },
            (_, Input::Failed(what, why)) => Phase::Failed {
                at: now,
                what: match what {
                    Failure::Save if switching => Failure::Start,
                    what => what,
                },
                why: why.as_deref().map(short_reason),
            },
            (Phase::Recording { .. }, Input::Chips) => Phase::Recording {
                chips_until: Some(now + tuning::get().chips_open_s),
            },
            (Phase::Recording { chips_until }, Input::Worked) => Phase::Recording {
                chips_until: chips_until.map(|until| {
                    until
                        .min(until - tuning::get().chips_open_s + CHIPS_MIN_S)
                        .max(now)
                }),
            },
            (Phase::Recording { .. }, Input::Gone) => Phase::Hidden,
            (Phase::Selecting(mode), Input::Record) => {
                actions.push(Action::Record(mode));
                Phase::Selecting(mode)
            }
            (_, Input::Delivered(delivered)) => match delivered {
                Delivered::Sent { to } => Phase::Sent(Sent {
                    to,
                    rule_saved: false,
                    at: now,
                    back: None,
                }),
                Delivered::Queued { to } => Phase::Queued { to, at: now },
            },
            (_, Input::Routed(route)) => Phase::Routed(Routed {
                route,
                at: now,
                open: None,
                taken_back: false,
                already_with: None,
            }),
            (Phase::Routed(mut routed), Input::OpenList) => {
                if routed.open.is_none() && !matches!(routed.route, Route::Unreachable { .. }) {
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
            (Phase::Routed(mut routed), input @ (Input::Type(_) | Input::Backspace)) => {
                if let Some(open) = &mut routed.open {
                    match input {
                        Input::Type(typed) => open.query.push_str(&typed),
                        _ => {
                            open.query.pop();
                        }
                    }
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
            (Phase::Routed(mut routed), Input::ClickAway) if !self.hovered => {
                routed.open = None;
                Phase::Routed(routed)
            }
            (phase, Input::ClickAway) => phase,
            // Taken back, Escape is the dismissal: the bundle is kept. Too
            // late, it's where it went.
            (
                Phase::Routed(Routed {
                    taken_back: true,
                    already_with,
                    ..
                }),
                Input::Escape | Input::Dismiss,
            ) => match already_with {
                None => Phase::Kept { at: now },
                Some(_) => Phase::Hidden,
            },
            (Phase::Routed(mut routed), Input::Escape) => {
                if routed.open.take().is_some() {
                    Phase::Routed(routed)
                } else if let (Route::Rule { to, .. }, true) =
                    (&routed.route, now - routed.at < undo_s())
                {
                    actions.push(Action::Undo { to: to.clone() });
                    Phase::Routed(routed)
                } else {
                    Phase::Hidden
                }
            }
            (Phase::Sent(sent), Input::Escape) if now - sent.at < undo_s() => {
                actions.push(Action::Undo {
                    to: sent.to.clone(),
                });
                Phase::Sent(sent)
            }
            (Phase::Recording { .. } | Phase::Selecting(_), Input::Escape) => {
                actions.push(Action::Cancel);
                Phase::Hidden
            }
            // Pasted or dismissed: the take-back it asked for says nothing.
            (Phase::Hidden, Input::Undone(_)) => Phase::Hidden,
            // An Undo never ends the capture: the list comes back with the
            // same sessions, the one it went to last, until a pick or a
            // dismissal. Too late to take back, a pick sends a copy.
            (phase, Input::Undone(undone)) => {
                let (route, to) = match phase {
                    Phase::Routed(Routed { route, .. }) if rule_to(&route).is_some() => {
                        let to = rule_to(&route);
                        (Some(route), to)
                    }
                    Phase::Sent(Sent { to, back, .. }) => (back, Some(to)),
                    other if undone == Undone::TooLate => {
                        self.phase = other;
                        return actions;
                    }
                    _ => (None, None),
                };
                let too_late = undone == Undone::TooLate;
                match (route, &to) {
                    (Some(route), Some(to)) => offered_again(route, to, too_late, now),
                    _ => None,
                }
                .map_or_else(
                    || match (too_late, to) {
                        (true, Some(to)) => Phase::TooLate { to, at: now },
                        _ => Phase::TakenBack { at: now },
                    },
                    Phase::Routed,
                )
            }
            (_, Input::Escape | Input::Dismiss) => Phase::Hidden,
            (phase, _) => phase,
        };
        actions
    }

    /// What a click on `id` does: the action for the host, after moving
    /// the capsule as the click shows.
    pub(crate) fn click(&mut self, id: super::surface::Id, now: f64) -> Vec<Action> {
        let before = self.phase.clone();
        let cued = self.cues.len();
        let actions = self.press(id, now);
        // A click that became an input cued already.
        if self.cues.len() == cued {
            self.cue(&before, &actions);
        }
        actions
    }

    fn press(&mut self, id: super::surface::Id, now: f64) -> Vec<Action> {
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
            // A chip restarts rather than retargets (Scrawl's
            // `switch_recording`): Screen is what's recording, so it only
            // folds the chips; iPhone starts the phone in its place; Window
            // and Area drop it and become the picker.
            (Phase::Recording { .. }, Id::Mode(mode)) => match mode {
                Mode::Screen => {
                    self.phase = Phase::Recording { chips_until: None };
                    Vec::new()
                }
                Mode::Iphone => {
                    self.phase = Phase::Recording { chips_until: None };
                    self.switching = true;
                    vec![Action::Switch(mode)]
                }
                Mode::Area | Mode::Window => {
                    self.phase = Phase::Selecting(mode);
                    vec![Action::Discard]
                }
            },
            (Phase::Recording { .. }, Id::Finish) => vec![Action::Finish],
            (Phase::Recording { .. } | Phase::Selecting(_), Id::Cancel | Id::Close) => {
                self.phase = Phase::Hidden;
                vec![Action::Cancel]
            }
            (Phase::Routed(routed), Id::Route(chip)) => {
                let Some((dest, from)) = route_chips(&routed.route)
                    .nth(chip)
                    .map(|(dest, from)| (dest.clone(), from.map(str::to_owned)))
                else {
                    return Vec::new();
                };
                let back = Some(routed.route.clone());
                self.phase = Phase::Sent(Sent {
                    to: dest.name,
                    rule_saved: false,
                    at: now,
                    back,
                });
                let to = dest.session;
                vec![match from {
                    Some(from) => Action::Redirect { from, to },
                    None => Action::Send { to, always: None },
                }]
            }
            // Pasting here takes back a send still waiting out its Undo, as
            // Scrawl's Paste does.
            (_, Id::Paste) => {
                let mut actions: Vec<Action> = self.undoable(now).into_iter().collect();
                actions.push(Action::Paste);
                self.phase = Phase::Hidden;
                actions
            }
            (_, Id::Inspect) => {
                self.phase = Phase::Hidden;
                vec![Action::Inspect]
            }
            (_, Id::Undo) => self.input(Input::Escape, now),
            (_, Id::Ask) => self.input(Input::OpenList, now),
            (_, Id::Row(row)) => self.input(Input::SendRow(row), now),
            // ✕ beside "Sending to X…" cancels the send, as Undo does.
            (_, Id::Close) => match self.undoable(now) {
                Some(undo) => {
                    if let Phase::Routed(routed) = &mut self.phase {
                        routed.open = None;
                    }
                    vec![undo]
                }
                None => self.input(Input::Dismiss, now),
            },
            _ => Vec::new(),
        }
    }

    /// The keys the capsule takes from Hex's tap: the picker's, an open
    /// list's, or a routed capture's Paste Last (its list) and, while a
    /// send can be taken back, Escape. With no sessions to list, Paste
    /// Last stays Paste Last.
    pub(crate) fn keys(&self, now: f64) -> Option<crate::capture_host::CapsuleKeys> {
        use crate::capture_host::CapsuleKeys;
        match &self.phase {
            Phase::Selecting(_) => Some(CapsuleKeys::Picker),
            Phase::Routed(Routed { open: Some(_), .. }) => Some(CapsuleKeys::List),
            Phase::Routed(Routed {
                route: Route::Unreachable { .. },
                ..
            }) => None,
            Phase::Routed(_) | Phase::Sent(_) => {
                let escape = self.undoable(now).is_some()
                    || matches!(
                        self.phase,
                        Phase::Routed(Routed {
                            taken_back: true,
                            ..
                        })
                    );
                (escape || matches!(self.phase, Phase::Routed(_)))
                    .then_some(CapsuleKeys::Route { escape })
            }
            _ => None,
        }
    }

    /// The take-back for a send still waiting out its Undo.
    fn undoable(&self, now: f64) -> Option<Action> {
        let (to, at) = match &self.phase {
            Phase::Routed(Routed {
                route: Route::Rule { to, .. },
                at,
                ..
            })
            | Phase::Sent(Sent { to, at, .. }) => (to, *at),
            _ => return None,
        };
        (now - at < undo_s()).then(|| Action::Undo { to: to.clone() })
    }

    /// Runs what time decides: the chips closing, the Undo window ending,
    /// the finished capsule leaving. True when one was due.
    pub(crate) fn tick(&mut self, now: f64) -> bool {
        if let Some(at) = self.counting_since()
            && now - at >= undo_s()
            && self.settled_send != Some(at)
        {
            self.settled_send = Some(at);
            self.cues.push(Cue::Sent);
        }
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
            let left = undo_s() - (now - at);
            // The countdown's next whole second.
            let step = left.rem_euclid(1.0);
            (left > 0.0).then_some(now + if step < 1e-6 { 1.0 } else { step })
        };
        let due = match &self.phase {
            Phase::Recording { chips_until } => chips_until.filter(|_| !self.hovered),
            Phase::Routed(Routed {
                route: Route::Rule { .. },
                at,
                ..
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
        let mut pres = self.pres_of(now);
        // Scrawl's "No speech": a capture with no words still copies.
        if let Pres::Done(done) = &mut pres
            && self.silent
            && done.text == "Copied"
            && done.why.is_none()
        {
            done.why = Some("no speech heard".into());
        }
        pres
    }

    fn pres_of(&self, now: f64) -> Pres {
        let undo_left = |at: f64| {
            let left = undo_s() - (now - at);
            (left > 0.0).then(|| left.ceil() as u32)
        };
        match &self.phase {
            Phase::Hidden => Pres::Hidden,
            Phase::Selecting(mode) => Pres::Selecting {
                mode: *mode,
                phone: self.phone,
            },
            Phase::Recording { chips_until } => Pres::Recording {
                chips: chips_until.is_some_and(|t| self.hovered || now < t),
                phone: self.phone,
            },
            Phase::Processing => Pres::Processing,
            Phase::Saved { .. } => Pres::Done(Done {
                hand_off: true,
                ..Done::plain(Lead::Check, "Copied")
            }),
            Phase::Failed { what, why, .. } => Pres::Done(Done {
                why: why.clone(),
                ..Done::plain(
                    Lead::Warning,
                    match what {
                        Failure::Start => "Couldn't start the capture".to_owned(),
                        Failure::Save => "Couldn't save the capture".to_owned(),
                        Failure::Send(to) => format!("Couldn't send to {}", short(to)),
                    },
                )
            }),
            Phase::TakenBack { .. } => Pres::Done(Done {
                why: Some(SEND_LATER.into()),
                ..Done::plain(Lead::Check, "Taken back")
            }),
            Phase::Kept { .. } => Pres::Done(Done {
                why: Some(SEND_LATER.into()),
                ..Done::plain(Lead::Check, "Kept")
            }),
            Phase::TooLate { to, .. } => Pres::Done(Done {
                why: Some("too late to take back".into()),
                hand_off: true,
                ..Done::plain(Lead::Check, format!("Sent to {}", short(to)))
            }),
            Phase::Queued { to, .. } => Pres::Done(Done {
                why: Some(format!("couldn't reach {}", short(to))),
                hand_off: true,
                ..Done::plain(Lead::Check, "Copied")
            }),
            Phase::Sent(sent) => {
                let undo = undo_left(sent.at);
                Pres::Done(Done {
                    why: sent.rule_saved.then(|| "rule saved".into()),
                    undo,
                    undo_ends: undo.map(|_| sent.at + undo_s()),
                    hand_off: true,
                    ..Done::plain(lead(undo), sending(&sent.to, undo, false))
                })
            }
            Phase::Routed(routed) => Pres::Done(routed_done(routed, undo_left(routed.at))),
        }
    }
}

fn hold_until(phase: &Phase) -> Option<f64> {
    match phase {
        Phase::Saved { at } | Phase::Queued { at, .. } | Phase::TooLate { at, .. } => {
            Some(at + tuning::get().done_hold_s)
        }
        Phase::Sent(sent) => Some(sent.at + tuning::get().done_hold_s),
        Phase::Routed(Routed {
            at,
            open: None,
            taken_back: false,
            ..
        }) => Some(at + tuning::get().done_hold_s),
        Phase::TakenBack { at } | Phase::Kept { at } | Phase::Failed { at, .. } => {
            Some(at + tuning::get().brief_hold_s)
        }
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
        Route::Unreachable { .. } => (&[], &[]),
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
        to: dest.session.clone(),
        always: site.clone(),
    });
    Phase::Sent(Sent {
        to: dest.name.clone(),
        rule_saved: site.is_some(),
        at: now,
        back: Some(routed.route.clone()),
    })
}

/// Where a rule sent it.
fn rule_to(route: &Route) -> Option<String> {
    match route {
        Route::Rule { to, .. } => Some(to.clone()),
        _ => None,
    }
}

/// The open list again after an Undo of the send to `to`: the same
/// sessions as a guess, `to` moved last. Taken back, nothing is sent; too
/// late, `to` keeps it. `None` when there's nothing to list.
fn offered_again(route: Route, to: &str, too_late: bool, now: f64) -> Option<Routed> {
    let (mut suggested, mut recent, site) = match route {
        Route::Rule {
            suggested, recent, ..
        } => (suggested, recent, None),
        Route::Guess {
            suggested,
            recent,
            site,
        } => (suggested, recent, site),
        Route::Nothing { recent } | Route::NoAnswer { recent } => (Vec::new(), recent, None),
        Route::Unreachable { .. } => return None,
    };
    for rows in [&mut suggested, &mut recent] {
        let (mut others, last): (Vec<Dest>, Vec<Dest>) = std::mem::take(rows)
            .into_iter()
            .partition(|dest| dest.name != to);
        others.extend(last.into_iter().map(|dest| Dest {
            why: if too_late { "has it" } else { "taken back" }.into(),
            ..dest
        }));
        *rows = others;
    }
    if suggested.is_empty() && recent.is_empty() {
        return None;
    }
    Some(Routed {
        route: Route::Guess {
            suggested,
            recent,
            site,
        },
        at: now,
        open: Some(Open {
            query: String::new(),
            selected: 0,
        }),
        taken_back: true,
        already_with: too_late.then(|| to.to_owned()),
    })
}

/// The route chips for `route`, best first, each with the send it takes
/// back: a rule's runner-up (the best other suggestion, which redirects
/// the send at once), or an ask's sessions, up to [`MAX_ROUTES`].
fn route_chips(route: &Route) -> Box<dyn Iterator<Item = (&Dest, Option<&str>)> + '_> {
    match route {
        Route::Rule { to, suggested, .. } => Box::new(
            suggested
                .iter()
                .find(|dest| dest.name != *to)
                .map(|dest| (dest, Some(to.as_str())))
                .into_iter(),
        ),
        Route::Guess { suggested, .. } => {
            Box::new(suggested.iter().take(MAX_ROUTES).map(|dest| (dest, None)))
        }
        Route::Nothing { .. } | Route::NoAnswer { .. } | Route::Unreachable { .. } => {
            Box::new(std::iter::empty())
        }
    }
}

/// "Sending to X" while a send can be taken back ("Auto-sending" when a
/// rule sends it, not a pick), then "Sent to X".
fn sending(to: &str, undo: Option<u32>, auto: bool) -> String {
    match (undo, auto) {
        (Some(_), true) => format!("Auto-sending to {}", short(to)),
        (Some(_), false) => format!("Sending to {}", short(to)),
        (None, _) => format!("Sent to {}", short(to)),
    }
}

/// The countdown ring while a send can be taken back, then the check.
fn lead(undo: Option<u32>) -> Lead {
    undo.map_or(Lead::Check, Lead::Countdown)
}

/// A session's name cut to [`ROUTE_NAME_MAX`] characters, so the
/// capsule's widest state stays inside its panel.
fn short(name: &str) -> String {
    if name.chars().count() <= ROUTE_NAME_MAX {
        return name.to_owned();
    }
    let cut: String = name.chars().take(ROUTE_NAME_MAX - 1).collect();
    format!("{}…", cut.trim_end())
}

/// A route chip's label: the first says what it does ("Send to SEER"),
/// the rest are just names.
fn route_label(index: usize, name: &str) -> String {
    let name = short(name);
    if index == 0 {
        format!("Send to {name}")
    } else {
        name
    }
}

fn routed_done(routed: &Routed, undo: Option<u32>) -> Done {
    let routes: Vec<String> = route_chips(&routed.route)
        .enumerate()
        .map(|(i, (dest, _))| route_label(i, &dest.name))
        .collect();
    let base = match &routed.route {
        // Sent nowhere, it waits for a pick: ⌥⇧V or a click reopens the list.
        _ if routed.taken_back => Done {
            ask: Some("Elsewhere…".into()),
            why: routed
                .already_with
                .as_ref()
                .map(|_| "too late to take back".into()),
            ..Done::plain(
                Lead::Check,
                match &routed.already_with {
                    Some(to) => format!("Sent to {}", short(to)),
                    None => "Taken back".into(),
                },
            )
        },
        // While the send can be taken back, the runner-up rides along
        // (⌥⇧V still opens the list); then the list's chip takes its place.
        Route::Rule { to, .. } => Done {
            why: Some("rule".into()),
            undo,
            undo_ends: undo.map(|_| routed.at + undo_s()),
            routes: if undo.is_some() { routes } else { Vec::new() },
            ask: undo.is_none().then(|| "Elsewhere…".into()),
            ..Done::plain(lead(undo), sending(to, undo, true))
        },
        // The chips are the ask; ⌥⇧V still opens the whole list.
        Route::Guess { .. } if !routes.is_empty() => Done {
            routes,
            ..Done::plain(Lead::Check, "Copied")
        },
        Route::Guess { .. } => Done {
            ask: Some("Send to…".into()),
            ..Done::plain(Lead::Check, "Copied")
        },
        Route::Nothing { .. } => Done {
            ask: Some("Send to…".into()),
            ..Done::plain(Lead::Check, "Copied")
        },
        Route::NoAnswer { .. } => Done {
            why: Some("the router didn't answer".into()),
            ask: Some("Send to…".into()),
            ..Done::plain(Lead::Check, "Copied")
        },
        // Still copied, so Paste and Inspect stay; there's nothing to list.
        Route::Unreachable { why } => Done {
            why: Some(short_reason(why)),
            ..Done::plain(Lead::Warning, "Couldn't reach OpenCode")
        },
    };
    match &routed.open {
        // Open, the field takes the chips' place.
        Some(open) => Done {
            why: None,
            undo: None,
            undo_ends: None,
            routes: Vec::new(),
            ask: None,
            open: Some((open.query.clone(), list(&routed.route, open))),
            ..base
        },
        None => Done {
            hand_off: true,
            ask_keys: if base.ask.is_some() {
                super::surface::ask_keys()
            } else {
                Vec::new()
            },
            ..base
        },
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
    fn paste_takes_back_a_waiting_send_and_the_capsule_stays_gone() {
        use super::super::surface::Id;
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule()), 0.0);
        assert!(done(&flow, 1.0).hand_off);
        assert_eq!(
            flow.click(Id::Paste, 1.0),
            [Action::Undo { to: "SEER".into() }, Action::Paste]
        );
        assert_eq!(flow.pres(1.0), Pres::Hidden);
        flow.input(Input::Undone(Undone::TakenBack), 1.2);
        assert_eq!(flow.pres(1.2), Pres::Hidden, "the take-back says nothing");

        // Once it's gone, Paste only pastes.
        flow.input(Input::Routed(rule()), 2.0);
        assert_eq!(flow.click(Id::Paste, 2.5 + undo_s()), [Action::Paste]);

        // Inspect never takes a send back.
        flow.input(Input::Routed(rule()), 10.0);
        assert_eq!(flow.click(Id::Inspect, 10.5), [Action::Inspect]);
        assert_eq!(flow.pres(10.5), Pres::Hidden);

        flow.input(Input::Stage(Stage::Done), 20.0);
        assert!(done(&flow, 20.0).hand_off);
        flow.input(Input::Failed(Failure::Save, None), 21.0);
        assert!(!done(&flow, 21.0).hand_off, "nothing was written");
    }

    #[test]
    fn a_paste_last_change_changes_the_ask_chip() {
        use crate::app_settings::{HotkeyBinding, HotkeyModifiers, publish_paste_last_keycaps};
        let mut flow = Flow::new();
        flow.input(Input::Routed(Route::Nothing { recent: Vec::new() }), 0.0);
        let before = done(&flow, 0.5);
        let rebound = HotkeyBinding {
            modifiers: HotkeyModifiers::option_command(),
            ..HotkeyBinding::paste_last_default()
        };
        publish_paste_last_keycaps(Some(&rebound));
        let after = done(&flow, 0.5);
        assert_eq!(after.ask_keys, rebound.keycaps());
        assert_ne!(before, after, "so the surface lays the chip out again");
        publish_paste_last_keycaps(None);
        assert!(done(&flow, 0.5).ask_keys.is_empty());
    }

    #[test]
    fn the_capsule_takes_only_the_keys_its_state_needs() {
        use crate::capture_host::CapsuleKeys;
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        assert_eq!(flow.keys(0.0), None);
        flow.input(Input::Chips, 0.5);
        flow.click(super::super::surface::Id::Mode(Mode::Window), 1.0);
        assert_eq!(flow.keys(1.0), Some(CapsuleKeys::Picker));

        flow.input(Input::Routed(rule()), 2.0);
        assert_eq!(flow.keys(2.5), Some(CapsuleKeys::Route { escape: true }));
        assert_eq!(
            flow.keys(2.0 + undo_s()),
            Some(CapsuleKeys::Route { escape: false })
        );
        flow.input(Input::OpenList, 9.0);
        assert_eq!(flow.keys(9.0), Some(CapsuleKeys::List));
        flow.input(Input::Type("he".into()), 9.1);
        flow.input(Input::Type("x".into()), 9.2);
        flow.input(Input::Backspace, 9.3);
        assert_eq!(done(&flow, 9.3).open.unwrap().0, "he");
        flow.input(Input::SendRow(0), 9.4);
        assert_eq!(flow.keys(9.5), Some(CapsuleKeys::Route { escape: true }));
        assert_eq!(flow.keys(9.4 + undo_s()), None, "sent, nothing to undo");

        flow.input(Input::Stage(Stage::Done), 20.0);
        assert_eq!(flow.keys(20.0), None);
    }

    #[test]
    fn a_recording_that_leaves_unseen_hides_the_capsule_but_not_the_picker() {
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Gone, 1.0);
        assert_eq!(flow.pres(1.0), Pres::Hidden);

        flow.input(Input::Stage(Stage::Recording), 2.0);
        flow.input(Input::Chips, 2.1);
        flow.click(super::super::surface::Id::Mode(Mode::Area), 2.5);
        flow.input(Input::Gone, 2.6);
        assert!(matches!(flow.pres(2.6), Pres::Selecting { .. }));
    }

    #[test]
    fn stages_walk_the_capsule_through_a_capture() {
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        assert_eq!(
            flow.pres(0.0),
            Pres::Recording {
                chips: false,
                phone: false
            }
        );
        flow.input(Input::Chips, 1.0);
        assert_eq!(
            flow.pres(1.0),
            Pres::Recording {
                chips: true,
                phone: false
            }
        );
        assert!(flow.tick(1.0 + tuning::get().chips_open_s));
        assert_eq!(
            flow.pres(5.0),
            Pres::Recording {
                chips: false,
                phone: false
            }
        );
        for stage in [Stage::Stopping, Stage::Transcribing, Stage::Writing] {
            flow.input(Input::Stage(stage), 6.0);
            assert_eq!(flow.pres(6.0), Pres::Processing);
        }
        flow.input(Input::Stage(Stage::Done), 7.0);
        assert_eq!(done(&flow, 7.0).text, "Copied");
        assert!(flow.tick(7.0 + tuning::get().done_hold_s));
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
        assert_eq!(
            flow.pres(1.0),
            Pres::Recording {
                chips: false,
                phone: false
            }
        );

        flow.input(Input::Chips, 2.0);
        flow.hover(true, 3.0);
        assert_eq!(flow.next_due(5.0), None);
        assert!(!flow.tick(2.0 + tuning::get().chips_open_s + 1.0));
        assert_eq!(
            flow.pres(7.0),
            Pres::Recording {
                chips: true,
                phone: false
            }
        );
        flow.hover(false, 7.0);
        assert!(!flow.tick(7.0 + CHIPS_LINGER_S - 0.1));
        assert!(flow.tick(7.0 + CHIPS_LINGER_S));
    }

    #[test]
    fn the_shortcut_records_the_pickers_screen_and_a_chip_discards_the_recording() {
        use super::super::surface::Id;
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Chips, 0.0);
        flow.click(Id::Mode(Mode::Window), 0.2);
        flow.click(Id::Mode(Mode::Screen), 0.3);
        assert_eq!(
            flow.input(Input::Record, 0.5),
            [Action::Record(Mode::Screen)]
        );
        flow.click(Id::Mode(Mode::Window), 0.6);
        assert_eq!(
            flow.input(Input::Record, 0.7),
            [Action::Record(Mode::Window)]
        );
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Chips, 0.0);
        assert_eq!(
            flow.click(super::super::surface::Id::Mode(Mode::Window), 0.5),
            [Action::Discard]
        );
        assert_eq!(
            flow.pres(0.5),
            Pres::Selecting {
                mode: Mode::Window,
                phone: false
            }
        );
    }

    #[test]
    fn a_rule_counts_down_its_undo_and_escape_takes_it_back() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule()), 10.0);
        let at = |t| done(&flow, t).undo;
        assert_eq!(at(10.0), Some(5));
        assert_eq!(at(11.2), Some(4));
        assert_eq!(at(14.5), Some(1));
        assert_eq!(at(15.0), None);
        assert_eq!(done(&flow, 10.0).text, "Auto-sending to SEER");
        assert_eq!(done(&flow, 10.0).lead, Lead::Countdown(5));
        assert_eq!(done(&flow, 10.0).undo_ends, Some(15.0));
        assert_eq!(done(&flow, 10.0).routes.len(), 1);
        // Once it can't be taken back, it's sent and the runner-up goes.
        assert_eq!(done(&flow, 15.0).text, "Sent to SEER");
        assert_eq!(done(&flow, 15.0).lead, Lead::Check);
        assert!(done(&flow, 15.0).routes.is_empty());
        assert_eq!(done(&flow, 10.0).why.as_deref(), Some("rule"));
        assert!((flow.next_due(10.0).unwrap() - 11.0).abs() < 1e-6);
        let actions = flow.input(Input::Escape, 11.0);
        assert_eq!(actions, vec![Action::Undo { to: "SEER".into() }]);
        flow.input(Input::Undone(Undone::TakenBack), 11.4);
        assert!(
            done(&flow, 11.4).open.is_some(),
            "taken back, the list opens"
        );
        // After the window, Escape just closes.
        let mut late = Flow::new();
        late.input(Input::Routed(rule()), 0.0);
        assert!(late.input(Input::Escape, 5.5).is_empty());
        assert_eq!(late.pres(5.5), Pres::Hidden);
    }

    fn rule_of_three() -> Route {
        Route::Rule {
            to: "SEER".into(),
            suggested: vec![
                Dest::new("SEER", "rule").session("ses_seer"),
                Dest::new("HEX", "last sent").session("ses_hex"),
                Dest::new("HOME", "recent").session("ses_home"),
            ],
            recent: vec![Dest::new("BOARD", "yesterday").session("ses_board")],
        }
    }

    fn rows(flow: &Flow, now: f64) -> Vec<String> {
        let (_, list) = done(flow, now).open.expect("the list is open");
        list.rows().map(|dest| dest.name.clone()).collect()
    }

    #[test]
    fn undoing_a_rule_send_reopens_the_list_until_a_pick() {
        use super::super::surface::Id;
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule_of_three()), 0.0);
        assert_eq!(
            flow.input(Input::Escape, 1.0),
            [Action::Undo { to: "SEER".into() }]
        );
        flow.input(Input::Undone(Undone::TakenBack), 1.2);
        // The same sessions, the wrongly picked one last; nothing sent.
        assert_eq!(rows(&flow, 1.2), ["HEX", "HOME", "SEER", "BOARD"]);
        let (_, list) = done(&flow, 1.2).open.unwrap();
        assert_eq!(list.suggested[2].why, "taken back");
        assert_eq!(list.selected, 0);
        assert_eq!(flow.keys(1.2), Some(crate::capture_host::CapsuleKeys::List));
        // It never leaves by itself.
        assert!(flow.next_due(1.2).is_none());
        assert!(!flow.tick(1_000.0));
        assert!(done(&flow, 1_000.0).open.is_some());
        // Pick 2: sent to #2.
        assert_eq!(
            flow.input(Input::SendRow(1), 1_000.0),
            [Action::Send {
                to: "ses_home".into(),
                always: None
            }]
        );
        assert_eq!(done(&flow, 1_000.0).text, "Sending to HOME");

        // Clicking away folds the list but keeps the capsule, and Paste Last
        // opens it again; Escape (or ✕) is the dismissal, and the bundle is kept.
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule_of_three()), 0.0);
        flow.input(Input::Escape, 1.0);
        flow.input(Input::Undone(Undone::TakenBack), 1.2);
        flow.input(Input::ClickAway, 2.0);
        let closed = done(&flow, 2.0);
        assert_eq!(closed.text, "Taken back");
        assert_eq!(closed.ask.as_deref(), Some("Elsewhere…"));
        assert!(!flow.tick(1_000.0));
        assert_eq!(
            flow.keys(1_000.0),
            Some(crate::capture_host::CapsuleKeys::Route { escape: true })
        );
        flow.click(Id::Ask, 1_000.0);
        assert_eq!(rows(&flow, 1_000.0)[0], "HEX");
        assert!(flow.input(Input::Escape, 1_001.0).is_empty());
        let kept = done(&flow, 1_001.0);
        assert_eq!(
            (kept.text.as_str(), kept.why.as_deref()),
            ("Kept", Some(SEND_LATER))
        );
        assert!(flow.tick(1_001.0 + tuning::get().brief_hold_s));

        // ✕ beside "Sending to SEER…" cancels the send rather than letting
        // it go unseen.
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule_of_three()), 0.0);
        assert_eq!(
            flow.click(Id::Close, 1.0),
            [Action::Undo { to: "SEER".into() }]
        );
        flow.input(Input::Undone(Undone::TakenBack), 1.2);
        assert_eq!(rows(&flow, 1.2)[0], "HEX");
        assert!(flow.click(Id::Close, 2.0).is_empty());
        assert_eq!(done(&flow, 2.0).text, "Kept");
        // After the window, ✕ dismisses as before.
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule_of_three()), 0.0);
        let late = undo_s() + 0.5;
        assert!(flow.click(Id::Close, late).is_empty());
        assert_eq!(flow.pres(late), Pres::Hidden);
    }

    #[test]
    fn too_late_to_take_back_offers_the_list_for_a_copy() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule_of_three()), 0.0);
        flow.input(Input::Escape, 1.0);
        flow.input(Input::Undone(Undone::TooLate), 1.2);
        let shown = done(&flow, 1.2);
        assert_eq!(shown.text, "Sent to SEER");
        assert_eq!(shown.why, None, "open, the field takes the why's place");
        assert_eq!(rows(&flow, 1.2), ["HEX", "HOME", "SEER", "BOARD"]);
        let (_, list) = shown.open.unwrap();
        assert_eq!(list.suggested[2].why, "has it");
        assert!(flow.take_cues().is_empty(), "nothing was taken back");
        assert!(!flow.tick(1_000.0), "it waits for a pick");
        // Pick 2: a copy goes to #2.
        assert_eq!(
            flow.input(Input::SendRow(1), 1_000.0),
            [Action::Send {
                to: "ses_home".into(),
                always: None
            }]
        );
        assert_eq!(done(&flow, 1_000.0).text, "Sending to HOME");

        // Folded, it says where it is; dismissed, it just goes.
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule_of_three()), 0.0);
        flow.input(Input::Escape, 1.0);
        flow.input(Input::Undone(Undone::TooLate), 1.2);
        flow.input(Input::ClickAway, 2.0);
        let folded = done(&flow, 2.0);
        assert_eq!(folded.text, "Sent to SEER");
        assert_eq!(folded.why.as_deref(), Some("too late to take back"));
        assert_eq!(folded.ask.as_deref(), Some("Elsewhere…"));
        flow.input(Input::Escape, 3.0);
        assert_eq!(flow.pres(3.0), Pres::Hidden);
    }

    #[test]
    fn undoing_a_pick_reopens_the_list_it_came_from() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(guess()), 0.0);
        flow.input(Input::OpenList, 0.5);
        flow.input(Input::SendRow(0), 1.0);
        assert_eq!(
            flow.input(Input::Escape, 2.0),
            [Action::Undo { to: "SEER".into() }]
        );
        flow.input(Input::Undone(Undone::TakenBack), 2.2);
        assert_eq!(rows(&flow, 2.2), ["HEX", "SEER", "HOME"]);
        let (_, list) = done(&flow, 2.2).open.unwrap();
        assert_eq!(
            list.footer,
            Footer::Always("problems.kitlangton.com".into())
        );
    }

    #[test]
    fn route_chips_send_at_once_and_a_rules_runner_up_redirects() {
        use super::super::surface::Id;
        let mut flow = Flow::new();
        flow.input(Input::Routed(guess()), 0.0);
        assert_eq!(
            flow.click(Id::Route(1), 1.0),
            [Action::Send {
                to: "HEX".into(),
                always: None
            }]
        );
        assert_eq!(done(&flow, 1.0).text, "Sending to HEX");
        assert_eq!(done(&flow, 1.0).undo, Some(5));

        let mut flow = Flow::new();
        flow.input(Input::Routed(rule()), 0.0);
        assert_eq!(done(&flow, 0.0).routes, ["Send to HEX"]);
        assert_eq!(
            flow.click(Id::Route(0), 1.0),
            [Action::Redirect {
                from: "SEER".into(),
                to: "HEX".into()
            }]
        );
        assert_eq!(done(&flow, 1.0).text, "Sending to HEX");
    }

    /// A switch that can't start settles the recording it discarded as
    /// failed: the capsule says the start failed, not the save.
    #[test]
    fn a_switch_that_cannot_start_says_so() {
        use super::super::surface::Id;
        let mut flow = Flow::new();
        flow.input(Input::Phone(true), 0.0);
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Chips, 0.1);
        flow.click(Id::Mode(Mode::Iphone), 0.5);
        flow.input(Input::Failed(Failure::Save, Some("no phone".into())), 0.6);
        assert!(matches!(
            flow.phase,
            Phase::Failed {
                what: Failure::Start,
                ..
            }
        ));

        // Once the phone's recording shows, a later failure is a save's.
        let mut flow = Flow::new();
        flow.input(Input::Phone(true), 0.0);
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Chips, 0.1);
        flow.click(Id::Mode(Mode::Iphone), 0.5);
        flow.input(Input::Stage(Stage::Recording), 0.6);
        flow.input(Input::Failed(Failure::Save, None), 3.0);
        assert!(matches!(
            flow.phase,
            Phase::Failed {
                what: Failure::Save,
                ..
            }
        ));
    }

    #[test]
    fn a_click_away_closes_the_list_and_nothing_else() {
        use crate::capture_host::CapsuleKeys;
        let mut flow = Flow::new();
        flow.input(Input::Routed(guess()), 0.0);
        flow.input(Input::OpenList, 0.5);
        assert_eq!(flow.keys(0.6), Some(CapsuleKeys::List));
        // A press on the capsule is the capsule's own click.
        flow.hover(true, 0.6);
        flow.input(Input::ClickAway, 0.6);
        assert_eq!(flow.keys(0.6), Some(CapsuleKeys::List));
        flow.hover(false, 0.65);
        assert!(flow.input(Input::ClickAway, 0.7).is_empty());
        assert_eq!(flow.keys(0.8), Some(CapsuleKeys::Route { escape: false }));
        assert!(matches!(flow.pres(0.8), Pres::Done(_)));
        flow.input(Input::ClickAway, 0.9);
        assert!(matches!(flow.pres(0.9), Pres::Done(_)), "the chips stay");
    }

    /// Two sessions may share a title: a send names the one picked by id.
    #[test]
    fn sends_name_the_session_by_id() {
        use super::super::surface::Id;
        let twins = || Route::Guess {
            suggested: vec![
                Dest::new("HEX", "guess").session("ses_a"),
                Dest::new("HEX", "last sent").session("ses_b"),
            ],
            recent: Vec::new(),
            site: None,
        };
        let mut flow = Flow::new();
        flow.input(Input::Routed(twins()), 0.0);
        assert_eq!(
            flow.click(Id::Route(1), 1.0),
            [Action::Send {
                to: "ses_b".into(),
                always: None
            }]
        );
        assert_eq!(done(&flow, 1.0).text, "Sending to HEX");

        let mut flow = Flow::new();
        flow.input(Input::Routed(twins()), 0.0);
        flow.input(Input::OpenList, 0.5);
        flow.input(Input::Move(1), 0.6);
        assert_eq!(
            flow.input(Input::Send { always: false }, 0.7),
            [Action::Send {
                to: "ses_b".into(),
                always: None
            }]
        );
    }

    #[test]
    fn an_ask_shows_at_most_four_chips_with_long_names_cut() {
        let mut flow = Flow::new();
        let suggested = ["SEER", "HEX", "HOME", "A VERY LONG SESSION TITLE", "FIFTH"]
            .map(|name| Dest::new(name, ""))
            .to_vec();
        flow.input(
            Input::Routed(Route::Guess {
                suggested,
                recent: Vec::new(),
                site: None,
            }),
            0.0,
        );
        assert_eq!(
            done(&flow, 0.0).routes,
            ["Send to SEER", "HEX", "HOME", "A VERY LONG SESSI…"]
        );
    }

    #[test]
    fn the_screen_chip_folds_and_the_iphone_chip_restarts_on_the_phone() {
        use super::super::surface::Id;
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Chips, 0.0);
        assert_eq!(
            flow.pres(0.1),
            Pres::Recording {
                chips: true,
                phone: false
            }
        );
        assert_eq!(flow.click(Id::Mode(Mode::Screen), 0.5), []);
        assert_eq!(
            flow.pres(0.5),
            Pres::Recording {
                chips: false,
                phone: false
            }
        );

        flow.input(Input::Phone(true), 1.0);
        flow.input(Input::Chips, 1.0);
        assert_eq!(
            flow.pres(1.1),
            Pres::Recording {
                chips: true,
                phone: true
            }
        );
        assert_eq!(
            flow.click(Id::Mode(Mode::Iphone), 1.5),
            [Action::Switch(Mode::Iphone)]
        );
        assert_eq!(
            flow.pres(1.5),
            Pres::Recording {
                chips: false,
                phone: true
            }
        );

        // Unplugged while the picker is on it: back to Screen.
        flow.input(Input::Stage(Stage::Recording), 1.8);
        flow.input(Input::Chips, 1.8);
        flow.click(Id::Mode(Mode::Window), 1.9);
        flow.click(Id::Mode(Mode::Iphone), 2.0);
        flow.input(Input::Phone(false), 2.5);
        assert_eq!(
            flow.pres(2.5),
            Pres::Selecting {
                mode: Mode::Screen,
                phone: false
            }
        );
    }

    #[test]
    fn a_guess_asks_and_the_list_sends_where_its_selected() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(guess()), 0.0);
        let asked = done(&flow, 0.0);
        assert_eq!(asked.routes, ["Send to SEER", "HEX"]);
        assert!(asked.ask.is_none(), "the chips are the ask");
        flow.input(Input::OpenList, 1.0);
        let open = done(&flow, 1.0);
        let (query, list) = open.open.expect("open");
        assert_eq!(query, "");
        assert_eq!(list.selected_dest().unwrap().name, "SEER");
        assert_eq!(
            list.footer,
            Footer::Always("problems.kitlangton.com".into())
        );
        assert!(
            open.ask.is_none() && open.routes.is_empty(),
            "the field takes the chips' place"
        );
        flow.input(Input::Type("ho".into()), 1.5);
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
        assert_eq!(done(&flow, 2.0).text, "Sending to HOME");
        assert_eq!(done(&flow, 2.0).undo, Some(5));
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
    fn cues_mark_start_stop_sent_and_taking_back() {
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        assert_eq!(flow.take_cues(), [Cue::Start]);
        flow.input(Input::Stage(Stage::Recording), 0.1);
        assert!(flow.take_cues().is_empty(), "a repeat stage is silent");
        flow.click(super::super::surface::Id::Finish, 1.0);
        flow.input(Input::Stage(Stage::Stopping), 1.0);
        assert_eq!(flow.take_cues(), [Cue::Stop]);
        flow.input(Input::Stage(Stage::Done), 2.0);
        flow.input(Input::Routed(rule()), 2.0);
        flow.tick(6.9);
        assert!(flow.take_cues().is_empty(), "nothing went yet");
        flow.tick(7.0);
        flow.tick(7.5);
        assert_eq!(flow.take_cues(), [Cue::Sent], "once, when Undo runs out");

        // Taken back: the undo sounds, and the send never does.
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule()), 0.0);
        flow.input(Input::Escape, 1.0);
        flow.input(Input::Undone(Undone::TakenBack), 1.2);
        flow.tick(9.0);
        assert_eq!(flow.take_cues(), [Cue::TakenBack]);

        // A cancelled recording, and a failure.
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::Escape, 1.0);
        assert_eq!(flow.take_cues(), [Cue::Start, Cue::TakenBack]);
        flow.input(Input::Failed(Failure::Start, None), 2.0);
        assert_eq!(flow.take_cues(), [Cue::Failed]);
    }

    #[test]
    fn nothing_heard_says_so() {
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        flow.input(Input::NothingHeard, 1.0);
        flow.input(Input::Stage(Stage::Done), 1.0);
        flow.input(Input::Routed(Route::Nothing { recent: Vec::new() }), 1.0);
        let shown = done(&flow, 1.0);
        assert_eq!(shown.text, "Copied");
        assert_eq!(shown.why.as_deref(), Some("no speech heard"));
        // The next recording starts fresh.
        flow.input(Input::Stage(Stage::Recording), 2.0);
        flow.input(Input::Stage(Stage::Done), 3.0);
        flow.input(Input::Routed(Route::Nothing { recent: Vec::new() }), 3.0);
        assert_eq!(done(&flow, 3.0).why, None);
    }

    #[test]
    fn escape_in_the_list_keeps_it_where_it_was() {
        let mut flow = Flow::new();
        flow.input(Input::Routed(rule()), 0.0);
        flow.input(Input::OpenList, 6.0);
        let (_, list) = done(&flow, 6.0).open.unwrap();
        assert_eq!(
            list.selected_dest().unwrap().name,
            "HEX",
            "a rule opens on the next"
        );
        assert_eq!(list.footer, Footer::Keep("SEER".into()));
        assert!(flow.next_due(6.0).is_none(), "an open list stays");
        assert!(flow.input(Input::Escape, 7.0).is_empty());
        assert!(done(&flow, 7.0).open.is_none());
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
    fn opencode_out_of_reach_says_so_and_leaves_paste_last_alone() {
        let mut flow = Flow::new();
        flow.input(
            Input::Routed(Route::Unreachable {
                why: "didn't answer in time".into(),
            }),
            0.0,
        );
        let shown = done(&flow, 0.0);
        assert_eq!(shown.lead, Lead::Warning);
        assert_eq!(shown.text, "Couldn't reach OpenCode");
        assert_eq!(shown.why.as_deref(), Some("didn't answer in time"));
        assert_eq!(shown.ask, None);
        assert!(shown.hand_off);
        // Nothing to list, so Paste Last pastes, and a stray OpenList opens nothing.
        assert_eq!(flow.keys(0.0), None);
        flow.input(Input::OpenList, 0.5);
        assert_eq!(done(&flow, 0.5).open, None);
    }

    #[test]
    fn cancel_and_failure() {
        let mut flow = Flow::new();
        flow.input(Input::Stage(Stage::Recording), 0.0);
        assert_eq!(flow.input(Input::Escape, 1.0), vec![Action::Cancel]);
        assert_eq!(flow.pres(1.0), Pres::Hidden);
        flow.input(Input::Stage(Stage::Failed), 2.0);
        assert_eq!(done(&flow, 2.0).lead, Lead::Warning);
        flow.tick(2.0 + tuning::get().brief_hold_s);
        assert_eq!(flow.pres(9.0), Pres::Hidden);
        flow.input(
            Input::Failed(
                Failure::Start,
                Some("starting the capture: the window closed".into()),
            ),
            10.0,
        );
        assert_eq!(done(&flow, 10.0).text, "Couldn't start the capture");
        assert_eq!(done(&flow, 10.0).why.as_deref(), Some("the window closed"));
        flow.input(Input::Failed(Failure::Save, None), 11.0);
        assert_eq!(done(&flow, 11.0).text, "Couldn't save the capture");
        flow.input(
            Input::Failed(Failure::Send("SEER".into()), Some("offline".into())),
            12.0,
        );
        assert_eq!(done(&flow, 12.0).text, "Couldn't send to SEER");
        assert_eq!(done(&flow, 12.0).why.as_deref(), Some("offline"));
        assert_eq!(
            short_reason("Screen Recording permission is off for Hex"),
            "Screen Recording permission is…"
        );
    }
}
