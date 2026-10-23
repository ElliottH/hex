//! The macOS meeting capsules on Rune: the recording offer, "left the
//! call", the ready transcript, and the recording capsule. Each is an
//! interactive Rune overlay, which on macOS is a non-activating `NSPanel`:
//! its buttons take clicks while the call app keeps focus and Hex never
//! activates. An offer that times out only closes; nothing starts or stops
//! on a timeout.
//!
//! The pump in `macos_rune` forwards controller events on the main thread;
//! buttons send `ControllerCommand`s back to the controller.
//!
//! Not yet built: "Copy new" / "Copy all" from the live transcript, hover
//! expansion, and dragging the recording capsule.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rune::{Align, Cx, El, OverlayHandle, OverlaySpec, ScreenEdge, Tx, col, row, text};
use rune_kit::{Button, ButtonSize, ButtonVariant};

use super::controls::{Paint, status_dot};
use crate::app_settings::{MeetingPolicy, MeetingProviderChoice};
use crate::desktop_i18n::{tr, tr_fill};
use crate::meeting::{self, MeetingManifest};
use crate::meeting_detection::MeetingCandidate;
use crate::meeting_watcher::{ControllerCommand, ControllerEvent};

/// The offer, call ended, and ready capsules.
const MEETING_PANEL_WIDTH: f32 = 420.0;
/// The offer's "…" choices need room for three buttons beside the name.
const MEETING_MENU_WIDTH: f32 = 560.0;
const MEETING_PANEL_HEIGHT: f32 = 64.0;
/// The recording capsule.
const RECORDING_PANEL_WIDTH: f32 = 548.0;
const RECORDING_PANEL_HEIGHT: f32 = 48.0;
const OFFER_TIMEOUT: Duration = Duration::from_secs(20);
const READY_TIMEOUT: Duration = Duration::from_secs(15);
/// How long an automatic recording says so before showing its elapsed time.
const AUTOMATIC_LABEL: Duration = Duration::from_secs(10);
const WINDOW_MARGIN: f32 = 12.0;
/// Below the menu bar.
const MENU_BAR: f32 = 28.0;
/// Above the Dock.
const DOCK: f32 = 24.0;

#[derive(Clone)]
enum Kind {
    Offer(MeetingCandidate),
    CallEnded {
        app: String,
    },
    Ready(Box<MeetingManifest>),
    /// An automatic recording while the recording indicator is off.
    AutoStarted {
        title: String,
    },
    Recording {
        id: String,
        title: String,
        started: Instant,
        automatic: bool,
    },
}

struct Capsule {
    kind: Option<Kind>,
    /// The offer shows its Never offer / Always record choices in place of
    /// its actions.
    more_open: bool,
    copied: bool,
    commands: SyncSender<ControllerCommand>,
    hide: Option<Arc<dyn Fn() + Send + Sync>>,
    resize: Option<Arc<dyn Fn(f32) + Send + Sync>>,
    choose: Choose,
}

type Choose = Arc<dyn Fn(MeetingProviderChoice) + Send + Sync>;

impl Capsule {
    fn send(&self, command: ControllerCommand) -> bool {
        self.commands.try_send(command).is_ok()
    }

    fn set_more_open(&mut self, open: bool) {
        self.more_open = open;
        if let Some(resize) = &self.resize {
            resize(if open {
                MEETING_MENU_WIDTH
            } else {
                MEETING_PANEL_WIDTH
            });
        }
    }

    /// Saves the offered provider's choice; returns whether there was one.
    fn choose(&self, policy: MeetingPolicy) -> bool {
        let Some(Kind::Offer(candidate)) = &self.kind else {
            return false;
        };
        let Some(provider) = candidate.provider() else {
            return false;
        };
        tracing::info!(key = %provider.key, ?policy, "meeting provider choice made from an offer");
        (self.choose)(provider.choose(policy));
        true
    }

    fn close(&mut self) {
        self.kind = None;
        if let Some(hide) = &self.hide {
            hide();
        }
    }
}

/// The two capsules: the timed panel (offer, call ended, ready) and the
/// recording capsule in the configured corner.
pub(crate) struct MeetingCapsules {
    panel: OverlayHandle<Capsule>,
    recording: OverlayHandle<Capsule>,
    /// Bumped whenever the panel shows something new, so a stale timeout
    /// doesn't close it.
    generation: Arc<AtomicU64>,
    /// The recording capsule's elapsed-time ticker runs while this is set.
    ticking: Arc<Mutex<Option<String>>>,
}

impl MeetingCapsules {
    /// `choose` saves a "Never offer" or "Always record" choice (the window
    /// keeps the settings it saves).
    pub(crate) fn install<S: 'static>(
        app: &mut rune::App<S>,
        commands: SyncSender<ControllerCommand>,
        choose: impl Fn(MeetingProviderChoice) + Send + Sync + 'static,
    ) -> Self {
        let choose: Choose = Arc::new(choose);
        let overlay = |app: &mut rune::App<S>, name, size| {
            let handle = app.overlay(
                OverlaySpec {
                    name,
                    size,
                    edge: ScreenEdge::Top,
                    margin: WINDOW_MARGIN + MENU_BAR,
                    align: Align::End,
                    inset: WINDOW_MARGIN,
                    interactive: true,
                    view,
                },
                Capsule {
                    kind: None,
                    more_open: false,
                    copied: false,
                    commands: commands.clone(),
                    hide: None,
                    resize: None,
                    choose: choose.clone(),
                },
            );
            let (hide, resize) = (handle.clone(), handle.clone());
            handle.update(move |capsule, _| {
                capsule.hide = Some(Arc::new(move || hide.hide()));
                capsule.resize = Some(Arc::new(move |width| {
                    resize.resize((width, MEETING_PANEL_HEIGHT));
                }));
            });
            handle
        };
        let panel = overlay(
            app,
            "Hex meeting",
            (MEETING_PANEL_WIDTH, MEETING_PANEL_HEIGHT),
        );
        let recording = overlay(
            app,
            "Hex meeting recording",
            (RECORDING_PANEL_WIDTH, RECORDING_PANEL_HEIGHT),
        );
        Self {
            panel,
            recording,
            generation: Arc::default(),
            ticking: Arc::default(),
        }
    }

    /// Shows, updates, or closes a capsule for each controller event.
    pub(crate) fn event(&mut self, event: &ControllerEvent) {
        match event {
            ControllerEvent::Offer(candidate) => {
                tracing::info!(
                    source = candidate.source.label(),
                    title = %candidate.title,
                    application = ?candidate.application,
                    "meeting recording offer shown"
                );
                self.show_panel(Kind::Offer(candidate.clone()), Some(OFFER_TIMEOUT));
            }
            ControllerEvent::Starting | ControllerEvent::Transcribing => {
                self.close_panel();
                self.close_recording();
            }
            ControllerEvent::RecordingStarted {
                id,
                title,
                automatic,
            } => {
                self.close_recording();
                if crate::app_settings::meeting_indicator_enabled() {
                    self.show_recording(id.clone(), title.clone(), *automatic);
                } else if *automatic {
                    // Nothing else on screen would say a recording began.
                    self.show_panel(
                        Kind::AutoStarted {
                            title: title.clone(),
                        },
                        Some(AUTOMATIC_LABEL),
                    );
                }
            }
            ControllerEvent::Retitled { title } => {
                let title = title.clone();
                self.recording.update(move |capsule, _| {
                    if let Some(Kind::Recording { title: shown, .. }) = &mut capsule.kind {
                        *shown = title;
                    }
                });
            }
            ControllerEvent::CallEnded { app } => {
                self.close_recording();
                self.show_panel(Kind::CallEnded { app: app.clone() }, None);
            }
            ControllerEvent::Finished(manifest) => {
                self.close_recording();
                self.show_panel(Kind::Ready(manifest.clone()), Some(READY_TIMEOUT));
            }
            ControllerEvent::NotesFinished(manifest) => {
                let manifest = manifest.clone();
                self.panel.update(move |capsule, _| {
                    if let Some(Kind::Ready(ready)) = &mut capsule.kind
                        && ready.id == manifest.id
                    {
                        *ready = manifest;
                    }
                });
            }
            ControllerEvent::Failed { .. } => self.close_recording(),
            ControllerEvent::OpenMeeting(_) | ControllerEvent::NotesStarted => {}
        }
    }

    fn show_panel(&self, kind: Kind, timeout: Option<Duration>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.panel
            .resize((MEETING_PANEL_WIDTH, MEETING_PANEL_HEIGHT));
        self.panel.update(move |capsule, tx| {
            capsule.kind = Some(kind);
            capsule.more_open = false;
            capsule.copied = false;
            tx.instant();
        });
        self.panel.show();
        if let Some(timeout) = timeout {
            let (panel, current) = (self.panel.clone(), self.generation.clone());
            std::thread::spawn(move || {
                std::thread::sleep(timeout);
                if current.load(Ordering::SeqCst) == generation {
                    panel.update(move |capsule, _| {
                        // Checked again when applied: a newer capsule may
                        // have been shown since.
                        if current.load(Ordering::SeqCst) != generation {
                            return;
                        }
                        if let Some(Kind::Offer(candidate)) = &capsule.kind {
                            tracing::info!(
                                source = candidate.source.label(),
                                title = %candidate.title,
                                "meeting recording offer timed out"
                            );
                        }
                        capsule.close();
                    });
                }
            });
        }
    }

    // Hide through the handle, not `Capsule::close`: a hide requested inside
    // an update runs on the event loop after a show queued in the same batch,
    // so closing then showing a capsule would leave it hidden.
    fn close_panel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.panel.update(|capsule, _| capsule.kind = None);
        self.panel.hide();
    }

    fn show_recording(&self, id: String, title: String, automatic: bool) {
        let corner = crate::app_settings::meeting_indicator_corner();
        let (edge, margin) = if corner.is_top() {
            (ScreenEdge::Top, WINDOW_MARGIN + MENU_BAR)
        } else {
            (ScreenEdge::Bottom, WINDOW_MARGIN + DOCK)
        };
        self.recording.place(edge, margin);
        self.recording.set_align(
            if corner.is_right() {
                Align::End
            } else {
                Align::Start
            },
            WINDOW_MARGIN,
        );
        let started = Instant::now();
        let ticker_id = id.clone();
        self.recording.update(move |capsule, tx| {
            capsule.kind = Some(Kind::Recording {
                id,
                title,
                started,
                automatic,
            });
            tx.instant();
        });
        self.recording.show();
        *self.ticking.lock().unwrap() = Some(ticker_id.clone());
        let (recording, ticking) = (self.recording.clone(), self.ticking.clone());
        // The elapsed time is the only thing that changes on its own.
        std::thread::spawn(move || {
            while ticking.lock().unwrap().as_deref() == Some(ticker_id.as_str()) {
                std::thread::sleep(Duration::from_secs(1));
                recording.update(|_, _| {});
            }
        });
    }

    fn close_recording(&self) {
        *self.ticking.lock().unwrap() = None;
        self.recording.update(|capsule, _| capsule.kind = None);
        self.recording.hide();
    }
}

fn button(
    p: Paint,
    key: &str,
    label: &str,
    primary: bool,
    press: impl Fn(&mut Capsule) + 'static,
) -> El {
    Button::new(label)
        .variant(if primary {
            ButtonVariant::Contrast
        } else {
            ButtonVariant::Neutral
        })
        .size(ButtonSize::Normal)
        .on_press(press)
        .build(p.k)
        .key(key)
}

fn view(capsule: &Capsule, cx: &mut Cx<Capsule>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let Some(kind) = &capsule.kind else {
        return row().key("empty");
    };
    let (dot, title, subtitle, actions) = match kind {
        Kind::Recording {
            title,
            started,
            automatic,
            ..
        } => {
            return recording(p, title, *started, *automatic);
        }
        Kind::AutoStarted { title } => (
            p.danger(),
            tr("Recording automatically").to_owned(),
            title.clone(),
            vec![button(p, "stop", tr("Stop"), true, |c| {
                if c.send(ControllerCommand::Stop) {
                    c.close();
                }
            })],
        ),
        Kind::Offer(candidate) if capsule.more_open => offer_choices(p, candidate),
        Kind::Offer(candidate) => {
            let mut actions = Vec::new();
            if candidate.provider().is_some() {
                actions.push(button(p, "offer-more", "…", false, |c| {
                    c.set_more_open(true);
                }));
            }
            actions.push(button(p, "not-now", tr("Not now"), false, |c| {
                if let Some(Kind::Offer(candidate)) = &c.kind {
                    tracing::info!(
                        source = candidate.source.label(),
                        title = %candidate.title,
                        "meeting recording offer dismissed"
                    );
                }
                c.send(ControllerCommand::Dismiss);
                c.close();
            }));
            actions.push(button(p, "record", tr("Record"), true, |c| {
                let Some(Kind::Offer(candidate)) = &c.kind else {
                    return;
                };
                let command = if candidate.key == "preview" {
                    ControllerCommand::Dismiss
                } else {
                    ControllerCommand::Record(candidate.clone())
                };
                if c.send(command) {
                    c.close();
                }
            }));
            (p.muted(), candidate.offer_title(), String::new(), actions)
        }
        Kind::CallEnded { app } => (
            p.danger(),
            tr_fill("{} left the call", app),
            tr("Hex is still recording.").to_owned(),
            vec![
                button(p, "keep-recording", tr("Keep recording"), false, |c| {
                    c.close();
                }),
                button(p, "stop-meeting", tr("Stop & transcribe"), true, |c| {
                    if c.send(ControllerCommand::Stop) {
                        c.close();
                    }
                }),
            ],
        ),
        Kind::Ready(manifest) => ready(p, manifest, capsule.copied),
    };
    pill(p, dot, &title, &subtitle, actions)
}

/// The offer's "…": never offer this provider again, or always record it
/// (which records this call too).
fn offer_choices(p: Paint, candidate: &MeetingCandidate) -> (rune::Color, String, String, Vec<El>) {
    let label = candidate
        .provider()
        .map_or_else(String::new, |provider| provider.label);
    let mut actions = vec![
        button(p, "offer-cancel", tr("Cancel"), false, |c| {
            c.set_more_open(false);
        }),
        button(p, "never-offer", tr("Never offer"), false, |c| {
            c.choose(MeetingPolicy::NeverOffer);
            c.send(ControllerCommand::Dismiss);
            c.close();
        }),
    ];
    if candidate.can_always_record() {
        actions.push(button(p, "always-record", tr("Always record"), true, |c| {
            let Some(Kind::Offer(candidate)) = &c.kind else {
                return;
            };
            let command = if candidate.key == "preview" {
                ControllerCommand::Dismiss
            } else {
                ControllerCommand::Record(candidate.clone())
            };
            c.choose(MeetingPolicy::AlwaysRecord);
            if c.send(command) {
                c.close();
            }
        }));
    }
    (
        p.muted(),
        label,
        tr("Change anytime in Settings").to_owned(),
        actions,
    )
}

fn ready(
    p: Paint,
    manifest: &MeetingManifest,
    copied: bool,
) -> (rune::Color, String, String, Vec<El>) {
    let summary = manifest
        .notes
        .as_ref()
        .map(|notes| notes.summary.trim().to_string())
        .filter(|summary| !summary.is_empty());
    let subtitle = if meeting::notes_in_progress(&manifest.id) {
        tr("Writing a title and summary with Apple Intelligence…").to_owned()
    } else if let Some(summary) = summary {
        summary
    } else if let Some(path) = &manifest.export_path {
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    } else if let Some(error) = &manifest.export_error {
        tr_fill("Markdown not saved: {}", error)
    } else {
        tr("Saved in Meetings").to_owned()
    };
    let title = meeting::display_title(manifest);
    let title = if title == manifest.title && manifest.notes.is_none() {
        tr("Meeting transcript ready").to_owned()
    } else {
        tr_fill("Ready · {}", &title)
    };
    let copy = Button::new(tr(if copied { "Copied" } else { "Copy" }))
        .variant(ButtonVariant::Neutral)
        .size(ButtonSize::Normal)
        .on_press_tx(|c: &mut Capsule, tx: &mut Tx| {
            let Some(Kind::Ready(manifest)) = &c.kind else {
                return;
            };
            match meeting::transcript_markdown_for_manifest(manifest) {
                Ok(markdown) => {
                    tx.copy(markdown);
                    c.copied = true;
                }
                Err(error) => tracing::warn!(%error, "could not copy meeting transcript"),
            }
        })
        .build(p.k)
        .key("copy-transcript");
    let mut actions = vec![copy];
    if manifest.export_path.is_some() {
        actions.push(button(p, "reveal-transcript", tr("Reveal"), true, |c| {
            if let Some(Kind::Ready(manifest)) = &c.kind
                && let Some(path) = &manifest.export_path
            {
                meeting::reveal(path);
            }
            c.close();
        }));
    }
    (p.success(), title, subtitle, actions)
}

fn recording(p: Paint, title: &str, started: Instant, automatic: bool) -> El {
    let elapsed = meeting::format_duration(started.elapsed().as_millis() as u64);
    let subtitle = if automatic && started.elapsed() < AUTOMATIC_LABEL {
        tr_fill("Started automatically · {}", &elapsed)
    } else {
        elapsed
    };
    let actions = vec![
        button(p, "open", tr("Open"), false, |c| {
            if let Some(Kind::Recording { id, .. }) = &c.kind {
                c.send(ControllerCommand::Open(id.clone()));
            }
        }),
        button(p, "stop", tr("Stop"), true, |c| {
            c.send(ControllerCommand::Stop);
        }),
    ];
    pill(p, p.danger(), title, &subtitle, actions)
}

/// The capsule's pill: status dot, two truncated lines, and its actions.
fn pill(p: Paint, dot: rune::Color, title: &str, subtitle: &str, actions: Vec<El>) -> El {
    let c = &p.k.c;
    row()
        .key("capsule")
        .w_full()
        .h_full()
        .px(4.0)
        .py(4.0)
        .child(
            row()
                .key("pill")
                .w_full()
                .h_full()
                .gap(12.0)
                .pl(16.0)
                .pr(8.0)
                .center_y()
                .radius(999.0)
                .fill(c.bg_base)
                .border(p.hair, c.border_base)
                .child(status_dot(dot))
                .child(
                    col()
                        .key("lines")
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(title.to_owned())
                                .key("title")
                                .truncate()
                                .font(p.k.ty.ui_medium)
                                .color(p.text()),
                        )
                        .children((!subtitle.is_empty()).then(|| {
                            text(subtitle.to_owned())
                                .key("subtitle")
                                .truncate()
                                .font(p.k.ty.small)
                                .color(p.faint())
                        })),
                )
                .child(row().key("actions").gap(8.0).children(actions)),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver};

    use rune::text::TextSystem;
    use rune::{Env, Runtime};

    use super::*;
    use crate::meeting_detection::MeetingSource;

    type Chosen = Arc<Mutex<Vec<MeetingProviderChoice>>>;

    const PANEL: (f32, f32) = (MEETING_PANEL_WIDTH, MEETING_PANEL_HEIGHT);
    const MENU: (f32, f32) = (MEETING_MENU_WIDTH, MEETING_PANEL_HEIGHT);
    const RECORDING: (f32, f32) = (RECORDING_PANEL_WIDTH, RECORDING_PANEL_HEIGHT);

    fn slack_huddle() -> MeetingCandidate {
        MeetingCandidate {
            key: "meeting-slack".into(),
            title: "Slack huddle".into(),
            source: MeetingSource::SlackHuddle,
            application: Some("com.tinyspeck.slackmacgap".into()),
            named: false,
            site: None,
            application_name: Some("Slack".into()),
        }
    }

    fn bare_browser() -> MeetingCandidate {
        MeetingCandidate {
            key: "meeting-brave".into(),
            title: "Browser call".into(),
            source: MeetingSource::BrowserCall,
            application: Some("com.brave.Browser".into()),
            named: false,
            site: None,
            application_name: Some("Brave Browser".into()),
        }
    }

    fn runtime(
        kind: Kind,
        more_open: bool,
        dark: bool,
        size: (f32, f32),
    ) -> (Runtime<Capsule>, Receiver<ControllerCommand>, Chosen) {
        let (commands, received) = mpsc::sync_channel(4);
        let chosen = Chosen::default();
        let saved = chosen.clone();
        let capsule = Capsule {
            kind: Some(kind),
            more_open,
            copied: false,
            commands,
            hide: None,
            resize: None,
            choose: Arc::new(move |choice| saved.lock().unwrap().push(choice)),
        };
        let env = Env {
            system_dark: dark,
            scale: 2.0,
        };
        let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
        let mut rt = Runtime::with_text(capsule, view, size, env, text);
        rt.frame(0.0);
        (rt, received, chosen)
    }

    fn tap(rt: &mut Runtime<Capsule>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }

    fn texts(rt: &Runtime<Capsule>) -> Vec<String> {
        rt.elements().into_iter().filter_map(|e| e.text).collect()
    }

    #[test]
    fn an_offer_names_the_call_without_asking_about_consent() {
        let (rt, _, _) = runtime(Kind::Offer(slack_huddle()), false, true, PANEL);
        assert_eq!(texts(&rt)[0], "Slack huddle detected");
        assert!(rt.find("subtitle").is_none());
        assert!(rt.find("offer-more").is_some());
    }

    #[test]
    fn always_record_saves_the_choice_and_records_this_call() {
        let (mut rt, _, _) = runtime(Kind::Offer(slack_huddle()), false, true, PANEL);
        tap(&mut rt, "offer-more");
        assert!(rt.state.more_open);
        // The overlay widens for the choices; render them at that width.
        let (mut rt, commands, chosen) = runtime(Kind::Offer(slack_huddle()), true, true, MENU);
        assert_eq!(texts(&rt)[0], "Slack huddles");
        tap(&mut rt, "always-record");
        let chosen = chosen.lock().unwrap();
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].key, "com.tinyspeck.slackmacgap");
        assert_eq!(chosen[0].policy, MeetingPolicy::AlwaysRecord);
        assert!(matches!(
            commands.try_recv(),
            Ok(ControllerCommand::Record(candidate)) if candidate.key == "meeting-slack"
        ));
        assert!(rt.state.kind.is_none());
    }

    #[test]
    fn never_offer_saves_the_choice_and_dismisses() {
        let (mut rt, commands, chosen) = runtime(Kind::Offer(slack_huddle()), true, true, MENU);
        tap(&mut rt, "never-offer");
        assert_eq!(chosen.lock().unwrap()[0].policy, MeetingPolicy::NeverOffer);
        assert!(matches!(
            commands.try_recv(),
            Ok(ControllerCommand::Dismiss)
        ));
    }

    #[test]
    fn a_browser_on_an_unknown_page_cannot_be_set_to_always_record() {
        let (rt, _, _) = runtime(Kind::Offer(bare_browser()), true, true, MENU);
        assert!(rt.find("never-offer").is_some());
        assert!(rt.find("always-record").is_none());
    }

    #[test]
    fn an_automatic_recording_says_so_at_first() {
        let recording = |started| Kind::Recording {
            id: "m".into(),
            title: "Slack Huddle call — 2:30 PM".into(),
            started,
            automatic: true,
        };
        let (rt, _, _) = runtime(recording(Instant::now()), false, true, RECORDING);
        assert!(
            texts(&rt)
                .iter()
                .any(|text| text.starts_with("Started automatically · "))
        );
        let earlier = Instant::now() - AUTOMATIC_LABEL - Duration::from_secs(1);
        let (rt, _, _) = runtime(recording(earlier), false, true, RECORDING);
        assert!(!texts(&rt).iter().any(|text| text.contains("automatically")));
    }

    /// No `--snap` scene draws an overlay, so this renders the capsules to
    /// PNGs in `HEX_CAPSULE_SNAPS` for review; without it, or a GPU, it skips.
    #[test]
    fn capsule_snapshots() {
        use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer};
        let Some(folder) = std::env::var_os("HEX_CAPSULE_SNAPS") else {
            return;
        };
        let Ok(gpu) = Gpu::try_new(Gpu::instance(), None) else {
            eprintln!("skipping capsule snapshots: no GPU adapter");
            return;
        };
        let folder = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        let mut google_meet = slack_huddle();
        google_meet.source = MeetingSource::GoogleMeet;
        google_meet.site = Some("meet.google.com".into());
        google_meet.title = "Design sync".into();
        google_meet.named = true;
        let states = [
            ("offer", Kind::Offer(slack_huddle()), false, PANEL),
            ("offer-choices", Kind::Offer(slack_huddle()), true, MENU),
            ("offer-choices-site", Kind::Offer(google_meet), true, MENU),
            (
                "recording-automatic",
                Kind::Recording {
                    id: "m".into(),
                    title: "Slack Huddle call — 2:30 PM".into(),
                    started: Instant::now() - Duration::from_secs(4),
                    automatic: true,
                },
                false,
                RECORDING,
            ),
            (
                "auto-started",
                Kind::AutoStarted {
                    title: "Slack Huddle call — 2:30 PM".into(),
                },
                false,
                PANEL,
            ),
        ];
        for (name, kind, more_open, size) in states {
            for dark in [false, true] {
                let (mut rt, _, _) = runtime(kind.clone(), more_open, dark, size);
                rt.frame(0.0);
                let (w, h) = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
                let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
                let target = Offscreen::new(&gpu.device, w, h);
                renderer.prepare(
                    &gpu.device,
                    &gpu.queue,
                    &mut rt.text.atlas,
                    &rt.instances,
                    (w, h),
                );
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                renderer.draw(&mut encoder, &target.view, rt.clear_color());
                target.copy(&mut encoder);
                gpu.queue.submit([encoder.finish()]);
                let pixels = target.read(&gpu.device);
                let theme = if dark { "dark" } else { "light" };
                let path = folder.join(format!("capsule-{name}-{theme}.png"));
                rune::render::write_png(&path, w, h, &pixels).unwrap();
            }
        }
    }
}
