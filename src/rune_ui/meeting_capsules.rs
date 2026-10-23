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
use crate::app_settings::NeverOfferMeeting;
use crate::desktop_i18n::{tr, tr_fill};
use crate::meeting::{self, MeetingManifest};
use crate::meeting_detection::MeetingCandidate;
use crate::meeting_watcher::{ControllerCommand, ControllerEvent};

/// The offer, call ended, and ready capsules.
const MEETING_PANEL_WIDTH: f32 = 420.0;
const MEETING_PANEL_HEIGHT: f32 = 64.0;
/// The recording capsule.
const RECORDING_PANEL_WIDTH: f32 = 548.0;
const RECORDING_PANEL_HEIGHT: f32 = 48.0;
const OFFER_TIMEOUT: Duration = Duration::from_secs(20);
const READY_TIMEOUT: Duration = Duration::from_secs(15);
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
    Recording {
        id: String,
        title: String,
        started: Instant,
    },
}

struct Capsule {
    kind: Option<Kind>,
    /// The offer shows its "Never offer" confirmation in place of its actions.
    more_open: bool,
    copied: bool,
    commands: SyncSender<ControllerCommand>,
    hide: Option<Arc<dyn Fn() + Send + Sync>>,
    never_offer: NeverOffer,
}

type NeverOffer = Arc<dyn Fn(NeverOfferMeeting) + Send + Sync>;

impl Capsule {
    fn send(&self, command: ControllerCommand) -> bool {
        self.commands.try_send(command).is_ok()
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
    /// `never_offer` records a "Never offer" choice (the window keeps the
    /// settings it saves).
    pub(crate) fn install<S: 'static>(
        app: &mut rune::App<S>,
        commands: SyncSender<ControllerCommand>,
        never_offer: impl Fn(NeverOfferMeeting) + Send + Sync + 'static,
    ) -> Self {
        let never_offer: NeverOffer = Arc::new(never_offer);
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
                    never_offer: never_offer.clone(),
                },
            );
            let hide = handle.clone();
            handle.update(move |capsule, _| {
                capsule.hide = Some(Arc::new(move || hide.hide()));
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
            ControllerEvent::RecordingStarted { id, title } => {
                self.close_recording();
                if crate::app_settings::meeting_indicator_enabled() {
                    self.show_recording(id.clone(), title.clone());
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

    fn show_recording(&self, id: String, title: String) {
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
            capsule.kind = Some(Kind::Recording { id, title, started });
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
        Kind::Recording { title, started, .. } => {
            return recording(p, title, *started);
        }
        Kind::Offer(candidate) if capsule.more_open => {
            let label = candidate
                .never_offer_target()
                .map_or_else(String::new, |target| target.label);
            (
                p.muted(),
                tr_fill("Never offer for {}?", &label),
                tr("You can undo this in Settings › Meetings.").to_owned(),
                vec![
                    button(p, "never-offer-cancel", tr("Cancel"), false, |c| {
                        c.more_open = false;
                    }),
                    button(p, "never-offer", tr("Never offer"), true, |c| {
                        if let Some(Kind::Offer(candidate)) = &c.kind
                            && let Some(target) = candidate.never_offer_target()
                        {
                            tracing::info!(key = %target.key, "meeting offers silenced");
                            (c.never_offer)(target);
                        }
                        c.send(ControllerCommand::Dismiss);
                        c.close();
                    }),
                ],
            )
        }
        Kind::Offer(candidate) => {
            let mut actions = Vec::new();
            if candidate.never_offer_target().is_some() {
                actions.push(button(p, "offer-more", "…", false, |c| {
                    c.more_open = true;
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
            (
                p.muted(),
                candidate.offer_title(),
                tr("Record locally? Tell participants.").to_owned(),
                actions,
            )
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

fn recording(p: Paint, title: &str, started: Instant) -> El {
    let elapsed = meeting::format_duration(started.elapsed().as_millis() as u64);
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
    pill(p, p.danger(), title, &elapsed, actions)
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
                        .child(
                            text(subtitle.to_owned())
                                .key("subtitle")
                                .truncate()
                                .font(p.k.ty.small)
                                .color(p.faint()),
                        ),
                )
                .child(row().key("actions").gap(8.0).children(actions)),
        )
}
