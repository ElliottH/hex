//! The capsule's side of the capture: it follows the newest capture in
//! `capture_host`'s published snapshot from its start to its outcome, and
//! hands presses back to the listener as `hex capture` commands.

use std::path::PathBuf;
use std::time::Duration;

use scrawl_capture::capture::Stage;

use super::CaptureCapsule;
use super::flow::{Action, Input, Mode};
use super::ink::InkOverlay;
use crate::capture_host::{CaptureCommand, CaptureSnapshot, CaptureTarget, TargetKind};

/// The capture the capsule shows, the stage it last showed, and whether its
/// recording got to work.
#[derive(Debug, Default)]
pub(crate) struct Follow {
    shown: Option<(PathBuf, Stage)>,
    worked: bool,
    /// The settled capture the previous snapshot held: `None` before the
    /// first snapshot, so an outcome from before Hex started moves nothing.
    last_seen: Option<Option<PathBuf>>,
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
        // Recording an iPhone and routing are not built.
        other => {
            tracing::info!(
                ?other,
                "the capture capsule asked for what Hex can't do yet"
            );
            return;
        }
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
            }),
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
}
