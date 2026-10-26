//! The capsule's side of the capture: it follows the newest capture in
//! `capture_host`'s published snapshot from its start to its outcome, and
//! hands presses back to the listener as `hex capture` commands.

use std::path::PathBuf;
use std::time::Duration;

use scrawl_capture::capture::Stage;

use super::CaptureCapsule;
use super::flow::{Action, Delivered, Dest, Input, Mode, Route, Undone};
use super::ink::InkOverlay;
use crate::capture_host::{
    CaptureCommand, CaptureRoute, CaptureSnapshot, CaptureTarget, RouteAnswer, RouteDelivery,
    RouteDest, TargetKind,
};

/// The capture the capsule shows, the stage it last showed, and whether its
/// recording got to work.
#[derive(Debug, Default)]
pub(crate) struct Follow {
    shown: Option<(PathBuf, Stage)>,
    worked: bool,
    /// The settled capture the previous snapshot held: `None` before the
    /// first snapshot, so an outcome from before Hex started moves nothing.
    last_seen: Option<Option<PathBuf>>,
    /// The shown capture's route as last seen, and whether its answer
    /// reached the capsule.
    route: Option<CaptureRoute>,
    phone: bool,
}

impl Follow {
    /// What moves the capsule between the last snapshot and `snapshot`. A
    /// new recording takes the capsule over, with its switch chips when the
    /// shortcut started it on the pointer's display; its first click,
    /// scroll, or stroke folds them. A start that failed before any
    /// snapshot showed it recording still shows its failure. A capture it
    /// never showed otherwise (one that settled before Hex started, or one a
    /// chip discarded) moves nothing.
    pub(crate) fn next(&mut self, snapshot: &CaptureSnapshot) -> Vec<Input> {
        let mut inputs = Vec::new();
        if snapshot.phone.is_some() != self.phone {
            self.phone = snapshot.phone.is_some();
            inputs.push(Input::Phone(self.phone));
        }
        let last = snapshot.last.as_ref().map(|last| last.bundle.clone());
        let previous = self.last_seen.replace(last.clone());
        if let (Some(previous), Some(failed)) = (previous, &snapshot.last)
            && previous.as_ref() != Some(&failed.bundle)
            && failed.stage == Stage::Failed
            && self
                .shown
                .as_ref()
                .is_none_or(|(bundle, _)| *bundle != failed.bundle)
        {
            // Took the capsule over as it started: show it as followed.
            self.shown = Some((failed.bundle.clone(), Stage::Recording));
        }
        if let Some(recording) = &snapshot.recording {
            let fresh = self
                .shown
                .as_ref()
                .is_none_or(|(bundle, _)| *bundle != recording.bundle);
            if fresh {
                self.shown = Some((recording.bundle.clone(), Stage::Recording));
                self.route = None;
                self.worked = recording.worked;
                inputs.push(Input::Stage(Stage::Recording));
                if recording.target == TargetKind::Pointer && !recording.worked {
                    inputs.push(Input::Chips);
                }
            } else if recording.worked && !self.worked {
                self.worked = true;
                inputs.push(Input::Worked);
            }
            return inputs;
        }
        inputs.extend(self.stage(snapshot));
        inputs.extend(self.routed(snapshot));
        inputs
    }

    /// What the router's news for the shown capture tells the capsule. The
    /// capsule runs a rule's undo window and a chip's send itself, so only
    /// what it can't know moves it: the answer, a send OpenCode didn't
    /// take (and its later retry), how an Undo ended, and a failed send.
    fn routed(&mut self, snapshot: &CaptureSnapshot) -> Vec<Input> {
        let Some((bundle, _)) = &self.shown else {
            return Vec::new();
        };
        let Some(route) = snapshot
            .last
            .as_ref()
            .filter(|last| last.bundle == *bundle)
            .and_then(|last| last.route.clone())
        else {
            return Vec::new();
        };
        let before = self.route.replace(route.clone());
        if before.as_ref() == Some(&route) {
            return Vec::new();
        }
        let mut inputs = Vec::new();
        let answered = |route: Option<&CaptureRoute>| {
            route.is_some_and(|route| route.answer != RouteAnswer::Looking)
        };
        if answered(Some(&route)) && !answered(before.as_ref()) {
            inputs.extend(capsule_route(&route.answer).map(Input::Routed));
        }
        let was = before.and_then(|before| before.delivery);
        if route.delivery == was {
            return inputs;
        }
        match route.delivery {
            Some(RouteDelivery::Queued { to }) => {
                inputs.push(Input::Delivered(Delivered::Queued { to }));
            }
            Some(RouteDelivery::Sent { to })
                if matches!(was, Some(RouteDelivery::Queued { .. })) =>
            {
                inputs.push(Input::Delivered(Delivered::Sent { to }));
            }
            Some(RouteDelivery::TakenBack) => inputs.push(Input::Undone(Undone::TakenBack)),
            Some(RouteDelivery::TooLate { .. }) => inputs.push(Input::Undone(Undone::TooLate)),
            Some(RouteDelivery::Failed { to, .. }) => {
                inputs.push(Input::Failed(Some(format!("Couldn't send to {to}"))));
            }
            _ => {}
        }
        inputs
    }

    fn stage(&mut self, snapshot: &CaptureSnapshot) -> Option<Input> {
        let followed = self.shown.as_ref().map(|(bundle, _)| bundle);
        let (bundle, stage) = if let Some(finishing) = snapshot
            .finishing
            .iter()
            .find(|finishing| Some(&finishing.bundle) == followed)
        {
            let stage = if finishing.cancelling {
                Stage::Cancelled
            } else {
                finishing.stage
            };
            (&finishing.bundle, stage)
        } else {
            let last = snapshot
                .last
                .as_ref()
                .filter(|last| Some(&last.bundle) == followed)?;
            (&last.bundle, last.stage)
        };
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.0 == *bundle && shown.1 == stage)
        {
            return None;
        }
        self.shown = Some((bundle.clone(), stage));
        if stage == Stage::Failed {
            let error = snapshot
                .last
                .as_ref()
                .filter(|last| last.bundle == *bundle)
                .and_then(|last| last.error.clone());
            return Some(Input::Failed(error));
        }
        Some(Input::Stage(stage))
    }
}

/// The router's answer as the capsule shows it. A rule's own session leads
/// its suggestions, as in Scrawl's list; the capsule names sessions by
/// title, which the host's Send and Redirect resolve.
fn capsule_route(answer: &RouteAnswer) -> Option<Route> {
    let dests = |dests: &[RouteDest]| -> Vec<Dest> {
        dests
            .iter()
            .map(|dest| Dest::new(dest.name.clone(), dest.why.clone()))
            .collect()
    };
    Some(match answer {
        RouteAnswer::Looking => return None,
        RouteAnswer::Rule {
            to,
            suggested,
            recent,
        } => Route::Rule {
            to: to.name.clone(),
            suggested: dests(std::slice::from_ref(to))
                .into_iter()
                .chain(dests(suggested))
                .collect(),
            recent: dests(recent),
        },
        RouteAnswer::Guess {
            suggested,
            recent,
            site,
        } => Route::Guess {
            suggested: dests(suggested),
            recent: dests(recent),
            site: site.clone(),
        },
        RouteAnswer::Nothing { recent } => Route::Nothing {
            recent: dests(recent),
        },
        RouteAnswer::NoAnswer { recent } => Route::NoAnswer {
            recent: dests(recent),
        },
    })
}

/// Moves `capsule` with every published capture change, and wakes `ink` to
/// look for a recording capture's strokes, on its own thread for the life
/// of the process.
pub(crate) fn follow(capsule: CaptureCapsule, ink: InkOverlay) {
    let spawned = std::thread::Builder::new()
        .name("capture-capsule".into())
        .spawn(move || {
            let watch = crate::capture_host::watch();
            let mut follow = Follow::default();
            loop {
                let Some(snapshot) = watch.wait(Duration::from_secs(3600)) else {
                    continue;
                };
                ink.wake();
                for input in follow.next(&snapshot) {
                    capsule.input(match input {
                        // The common first-capture failure: the engine's
                        // own reason for it is a ScreenCaptureKit error.
                        Input::Failed(_) if !crate::meeting::screen_recording_permitted() => {
                            Input::Failed(Some(SCREEN_RECORDING_OFF.into()))
                        }
                        input => input,
                    });
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the capture capsule's thread");
    }
}

/// Carries out a press or key on the capsule: Finish and Cancel end the
/// capture as `hex capture stop|cancel` would, a chip discards the
/// recording, the picker starts what it picked, and Record (or the
/// shortcut) without a pick records the display under the pointer. Runs on the
/// main thread, so it only queues the commands.
pub(crate) fn act(action: Action) {
    use crate::control::ScrawlCaptureAction;
    use crate::recognition::RecognitionControl;
    let screen = || CaptureCommand::Start(CaptureTarget::Display { at: pointer() });
    let controls: Vec<RecognitionControl> = match action {
        Action::Finish => vec![scrawl(ScrawlCaptureAction::Stop)],
        Action::Cancel => vec![scrawl(ScrawlCaptureAction::Cancel)],
        Action::Discard => vec![command(CaptureCommand::Discard)],
        Action::Start(target) => vec![command(CaptureCommand::Start(target))],
        // What the picker would record, when it has nothing to record yet
        // (its windows still load): the pointer's display.
        Action::Record(Mode::Screen | Mode::Window | Mode::Area) => vec![command(screen())],
        Action::Record(Mode::Iphone) => vec![command(CaptureCommand::Start(CaptureTarget::Phone))],
        Action::Switch(Mode::Iphone) => {
            vec![command(CaptureCommand::Switch(CaptureTarget::Phone))]
        }
        // Screen is what the shortcut records; Window and Area discard.
        Action::Switch(Mode::Screen | Mode::Window | Mode::Area) => return,
        Action::Send { to, always } => vec![command(CaptureCommand::Send {
            to,
            always: always.is_some(),
        })],
        Action::Undo { .. } => vec![command(CaptureCommand::Undo)],
        Action::Redirect { to, .. } => vec![command(CaptureCommand::Redirect { to })],
    };
    let Some(runtime) = crate::macos_rune::runtime() else {
        return;
    };
    for control in controls {
        if let Err(error) = runtime.recognition(control) {
            tracing::warn!(%error, "the capture capsule could not reach the listener");
        }
    }
}

fn scrawl(action: crate::control::ScrawlCaptureAction) -> crate::recognition::RecognitionControl {
    let (reply, _) = std::sync::mpsc::sync_channel(1);
    crate::recognition::RecognitionControl::ScrawlCapture { action, reply }
}

fn command(command: CaptureCommand) -> crate::recognition::RecognitionControl {
    let (reply, _) = std::sync::mpsc::sync_channel(1);
    crate::recognition::RecognitionControl::CaptureCommand { command, reply }
}

/// Why a capture failed when Screen Recording isn't granted.
const SCREEN_RECORDING_OFF: &str = "Screen Recording is off";

/// The pointer, in global top-left points.
fn pointer() -> [f64; 2] {
    use core_graphics_types::geometry::CGPoint;
    use std::ffi::c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *mut c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }
    // SAFETY: a null source makes an event at the current pointer; it is
    // released once read.
    unsafe {
        let event = CGEventCreate(std::ptr::null_mut());
        if event.is_null() {
            return [0.0, 0.0];
        }
        let at = CGEventGetLocation(event);
        CFRelease(event);
        [at.x, at.y]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::CaptureInstant;
    use crate::capture_host::{FinishingCapture, RecordingCapture, SettledCapture};

    fn bundle(name: &str) -> PathBuf {
        PathBuf::from(format!("/captures/{name}.scrawl"))
    }

    fn recording(name: &str) -> CaptureSnapshot {
        CaptureSnapshot {
            generation: 0,
            recording: Some(RecordingCapture {
                bundle: bundle(name),
                since: CaptureInstant::now(),
                screen: true,
                target: TargetKind::Display,
                worked: false,
            }),
            finishing: Vec::new(),
            last: None,
            phone: None,
        }
    }

    fn finishing(name: &str, stage: Stage, cancelling: bool) -> CaptureSnapshot {
        CaptureSnapshot {
            generation: 0,
            recording: None,
            finishing: vec![FinishingCapture {
                bundle: bundle(name),
                stage,
                cancelling,
            }],
            last: None,
            phone: None,
        }
    }

    fn settled(name: &str, stage: Stage) -> CaptureSnapshot {
        CaptureSnapshot {
            generation: 0,
            recording: None,
            finishing: Vec::new(),
            last: Some(SettledCapture {
                bundle: bundle(name),
                stage,
                words: Some(12),
                gaps: Some(0),
                missing_transcript: None,
                error: None,
                route: None,
            }),
            phone: None,
        }
    }

    #[test]
    fn it_follows_a_capture_from_start_to_outcome() {
        let mut follow = Follow::default();
        assert_eq!(follow.next(&settled("old", Stage::Done)), []);
        let a = recording("a");
        assert_eq!(follow.next(&a), [Input::Stage(Stage::Recording)]);
        assert_eq!(follow.next(&a), [], "a repeat moves nothing");
        let stopping = finishing("a", Stage::Stopping, false);
        assert_eq!(follow.next(&stopping), [Input::Stage(Stage::Stopping)]);
        assert_eq!(
            follow.next(&finishing("a", Stage::Transcribing, false)),
            [Input::Stage(Stage::Transcribing)]
        );
        assert_eq!(
            follow.next(&settled("a", Stage::Done)),
            [Input::Stage(Stage::Done)]
        );
        assert_eq!(follow.next(&settled("a", Stage::Done)), []);
    }

    #[test]
    fn a_new_recording_takes_over_and_a_cancel_hides() {
        let mut follow = Follow::default();
        follow.next(&recording("a"));
        follow.next(&finishing("a", Stage::Writing, false));
        // A second capture starts while the first is written.
        let mut both = recording("b");
        both.finishing = finishing("a", Stage::Writing, false).finishing;
        assert_eq!(follow.next(&both), [Input::Stage(Stage::Recording)]);
        // The first settling no longer moves the capsule.
        let mut a_done = recording("b");
        a_done.last = settled("a", Stage::Done).last;
        assert_eq!(follow.next(&a_done), []);
        assert_eq!(
            follow.next(&finishing("b", Stage::Stopping, true)),
            [Input::Stage(Stage::Cancelled)]
        );
    }

    #[test]
    fn a_start_that_failed_unseen_still_shows_why() {
        let mut follow = Follow::default();
        assert_eq!(follow.next(&settled("old", Stage::Failed)), []);
        let mut failed = settled("a", Stage::Failed);
        failed.last.as_mut().unwrap().error = Some("no screen to record".into());
        assert_eq!(
            follow.next(&failed),
            [Input::Failed(Some("no screen to record".into()))]
        );
        assert_eq!(follow.next(&failed), []);
    }

    #[test]
    fn a_failure_says_why() {
        let mut follow = Follow::default();
        follow.next(&recording("a"));
        let mut failed = settled("a", Stage::Failed);
        failed.last.as_mut().unwrap().error = Some("the window closed".into());
        assert_eq!(
            follow.next(&failed),
            [Input::Failed(Some("the window closed".into()))]
        );
    }

    #[test]
    fn the_shortcuts_recording_shows_its_chips_until_work_starts() {
        let mut follow = Follow::default();
        let mut a = recording("a");
        a.recording.as_mut().unwrap().target = TargetKind::Pointer;
        assert_eq!(
            follow.next(&a),
            [Input::Stage(Stage::Recording), Input::Chips]
        );
        a.recording.as_mut().unwrap().worked = true;
        assert_eq!(follow.next(&a), [Input::Worked]);
        assert_eq!(follow.next(&a), [], "work folds the chips once");
        // A chip discarded it: nothing settles, and the capsule stays the
        // picker it turned into.
        assert_eq!(follow.next(&CaptureSnapshot::default()), []);
    }

    fn dest(name: &str, why: &str) -> RouteDest {
        RouteDest {
            session: format!("ses_{name}"),
            name: name.into(),
            why: why.into(),
        }
    }

    fn routed(name: &str, answer: RouteAnswer, delivery: Option<RouteDelivery>) -> CaptureSnapshot {
        let mut snapshot = settled(name, Stage::Done);
        snapshot.last.as_mut().unwrap().route = Some(CaptureRoute {
            answer,
            delivery,
            offer: None,
        });
        snapshot
    }

    #[test]
    fn a_rule_shows_once_and_only_news_the_capsule_cant_know_moves_it() {
        let mut follow = Follow::default();
        follow.next(&recording("a"));
        assert_eq!(
            follow.next(&routed("a", RouteAnswer::Looking, None)),
            [Input::Stage(Stage::Done)]
        );
        let rule = || RouteAnswer::Rule {
            to: dest("SEER", "rule"),
            suggested: vec![dest("HEX", "last sent")],
            recent: vec![dest("HOME", "yesterday")],
        };
        let pending = Some(RouteDelivery::Pending { to: "SEER".into() });
        assert_eq!(
            follow.next(&routed("a", rule(), pending.clone())),
            [Input::Routed(Route::Rule {
                to: "SEER".into(),
                suggested: vec![Dest::new("SEER", "rule"), Dest::new("HEX", "last sent")],
                recent: vec![Dest::new("HOME", "yesterday")],
            })]
        );
        // The capsule counts the undo window down and says Sent itself.
        let sending = Some(RouteDelivery::Sending { to: "SEER".into() });
        assert_eq!(follow.next(&routed("a", rule(), sending)), []);
        let sent = Some(RouteDelivery::Sent { to: "SEER".into() });
        assert_eq!(follow.next(&routed("a", rule(), sent)), []);
        assert_eq!(
            follow.next(&routed("a", rule(), Some(RouteDelivery::TakenBack))),
            [Input::Undone(Undone::TakenBack)]
        );
    }

    #[test]
    fn a_queued_send_says_so_and_says_sent_when_it_goes() {
        let mut follow = Follow::default();
        follow.next(&recording("a"));
        let guess = || RouteAnswer::Guess {
            suggested: vec![dest("HEX", "guess")],
            recent: Vec::new(),
            site: None,
        };
        follow.next(&routed("a", guess(), None));
        let queued = Some(RouteDelivery::Queued { to: "HEX".into() });
        assert_eq!(
            follow.next(&routed("a", guess(), queued)),
            [Input::Delivered(Delivered::Queued { to: "HEX".into() })]
        );
        let sent = Some(RouteDelivery::Sent { to: "HEX".into() });
        assert_eq!(
            follow.next(&routed("a", guess(), sent)),
            [Input::Delivered(Delivered::Sent { to: "HEX".into() })]
        );
        let failed = Some(RouteDelivery::Failed {
            to: "HEX".into(),
            why: "gone".into(),
        });
        assert_eq!(
            follow.next(&routed("a", guess(), failed)),
            [Input::Failed(Some("Couldn't send to HEX".into()))]
        );
    }

    #[test]
    fn a_cabled_phone_shows_its_chip() {
        let mut follow = Follow::default();
        let mut snapshot = CaptureSnapshot {
            phone: Some("Kit's iPhone".into()),
            ..CaptureSnapshot::default()
        };
        assert_eq!(follow.next(&snapshot), [Input::Phone(true)]);
        assert_eq!(follow.next(&snapshot), []);
        snapshot.phone = None;
        assert_eq!(follow.next(&snapshot), [Input::Phone(false)]);
    }
}
