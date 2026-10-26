//! Where a written capture goes among OpenCode sessions: Scrawl's router
//! (`crates/router`) run in Hex's process with `capture.routing` and
//! Scrawl's own router folders, so `scrawl router undo`, Scrawl's rule
//! offers, and its evals see Hex's sends as Scrawl's.
//!
//! The collector starts a [`Look`] at the tab, terminal, and sessions the
//! moment a capture stops, beside its writing. Once it is written, the
//! finisher hands it to the one `capture-route` thread ([`Routes`]), which
//! decides, waits out a rule's [`UNDO`] window, sends, and carries out the
//! capsule's Send, Undo, and Redirect for the newest capture, in the order
//! they came. Every change lands in the snapshot's `last.route`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use scrawl_capture::capture::RouteContext;
use scrawl_capture::delivery::{self, Delivery};
use scrawl_capture::model::{Destination, ScrawlBundle};
use scrawl_capture::routing::{self, jev, look, policy, smart, undo};
use scrawl_capture::{Router, RouterConfig, RouterDirs, RuleKind, Undone};

use super::{SharedOutcomes, lock};

/// How long a rule's send waits, taken back by Undo, before it goes.
pub const UNDO: Duration = Duration::from_secs(3);
/// How often sends OpenCode didn't take are retried.
const RETRY_EVERY: Duration = Duration::from_secs(15);
/// How long taking back a send may wait on OpenCode.
const UNDO_DEADLINE: Duration = Duration::from_secs(10);
/// Recent sessions listed beside the router's suggestions.
const RECENT: usize = 8;
const QUEUE: usize = 16;

// ---------------------------------------------------------------------------
// What views see.

/// A written capture's route: the router's answer and what became of the
/// send.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureRoute {
    pub answer: RouteAnswer,
    pub delivery: Option<RouteDelivery>,
    /// The site or app a rule could now send straight to where this
    /// capture went, as two sends agreed: `Send { to: <that session>,
    /// always: true }` saves it.
    pub offer: Option<String>,
}

impl CaptureRoute {
    fn of(answer: RouteAnswer) -> Self {
        Self {
            answer,
            delivery: None,
            offer: None,
        }
    }

    /// The written capture, while the router works out where it goes.
    pub(super) fn looking() -> Self {
        Self::of(RouteAnswer::Looking)
    }
}

/// The route of a capture the router never got to.
pub(super) fn unrouted() -> CaptureRoute {
    CaptureRoute::of(RouteAnswer::NoAnswer { recent: Vec::new() })
}

/// An OpenCode session a capture can go to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteDest {
    pub session: String,
    /// The session's title as the router labels it.
    pub name: String,
    /// A quiet word on why: `rule`, `dev server`, `guess`, `2 min ago`.
    pub why: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RouteAnswer {
    /// Written; the router is still working.
    Looking,
    /// A rule (a dev server's port, a page, a saved rule) says where it
    /// goes: sent to `to` after [`UNDO`] unless taken back.
    Rule {
        to: RouteDest,
        suggested: Vec<RouteDest>,
        recent: Vec<RouteDest>,
    },
    /// Guesses, best first; nothing is sent until one is picked. `site` is
    /// what an always-rule would match (the site's host or the app's name),
    /// `None` when no rule can be made.
    Guess {
        suggested: Vec<RouteDest>,
        recent: Vec<RouteDest>,
        site: Option<String>,
    },
    /// Nothing fits: recent sessions only.
    Nothing { recent: Vec<RouteDest> },
    /// The router failed or OpenCode couldn't be read.
    NoAnswer { recent: Vec<RouteDest> },
}

/// What became of a capture's send; `to` is the session's name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RouteDelivery {
    /// A rule's send, waiting out [`UNDO`].
    Pending {
        to: String,
    },
    Sending {
        to: String,
    },
    Sent {
        to: String,
    },
    /// OpenCode didn't take it; retried every 15 s.
    Queued {
        to: String,
    },
    /// Never sent, withdrawn, reverted, or disowned.
    TakenBack,
    /// The session already acted on it.
    TooLate {
        to: String,
    },
    Failed {
        to: String,
        why: String,
    },
}

// ---------------------------------------------------------------------------
// The router.

/// The router with Hex's `capture.routing` and Scrawl's folders under
/// `~/Library/Application Support/scrawl`. Never reads
/// `~/.config/scrawl`. Its own log lines name sites and sessions, so they
/// go to tracing's debug level only.
pub fn router() -> Router {
    let settings = crate::app_settings::capture_settings();
    let config = settings.routing.config().clone();
    let support = dirs::home_dir()
        .unwrap_or_default()
        .join("Library/Application Support/scrawl");
    let dirs = RouterDirs::under(
        &support.join("router"),
        Some(support.join("routing-decisions.jsonl")),
        Some(settings.captures_dir()),
    );
    Router::new(config, dirs)
        .with_log(|line| tracing::debug!(line, "capture router"))
        .with_summary(scrawl_capture::sketch::summary_markdown)
}

fn sends(config: &RouterConfig) -> bool {
    config.send_captures && std::env::var_os("SCRAWL_NO_OPENCODE").is_none()
}

/// The tab, terminal, and sessions behind a capture that just stopped,
/// read while it is written. With smart routing off, also the port
/// ladder's answer.
pub(super) struct Look {
    router: Router,
    seen: JoinHandle<Seen>,
}

struct Seen {
    live: Option<look::Live>,
    ladder: Option<Destination>,
}

/// Starts looking, or `None` when captures aren't sent (`autoSend` off).
pub(super) fn look(context: RouteContext) -> Option<Look> {
    let router = router();
    if !sends(router.config()) {
        return None;
    }
    let seen = {
        let router = router.clone();
        thread::Builder::new()
            .name("capture-look".into())
            .spawn(move || {
                router.run(|| {
                    let front = context.front_bundle_id.as_deref();
                    let captured = context.captured_bundle_id.as_deref();
                    let ladder = if router.config().smart_routing {
                        None
                    } else {
                        routing::route_capture(front, captured)
                    };
                    Seen {
                        live: routing::look_capture(front, captured, context.app_pid),
                        ladder,
                    }
                })
            })
            .inspect_err(|error| tracing::warn!(%error, "could not look for a capture's route"))
            .ok()?
    };
    Some(Look { router, seen })
}

// ---------------------------------------------------------------------------
// The capture-route thread.

pub(super) enum Job {
    Written {
        bundle: PathBuf,
        look: Look,
    },
    Send {
        to: String,
        always: bool,
    },
    Undo,
    Redirect {
        to: String,
    },
    /// `hex capture send`: any capture, to `to` or where its route or its
    /// last send says.
    SendBundle {
        bundle: PathBuf,
        to: Option<String>,
        reply: SyncSender<Result<String, String>>,
    },
}

/// The running host's route queue, for `hex capture send`, with the
/// [`Routes`] it belongs to.
static JOBS: Mutex<Option<(u64, SyncSender<Job>)>> = Mutex::new(None);
static NEXT_ROUTES: AtomicU64 = AtomicU64::new(0);

/// How long `hex capture send` waits for the send, inside the control
/// deadline.
const CLI_WAIT: Duration = Duration::from_secs(8);

/// Sends `bundle` through the `capture-route` thread and says what became
/// of it: as the capsule's pick when it is the newest capture, so its
/// route and Undo follow.
pub(super) fn send_bundle(bundle: PathBuf, to: Option<String>) -> Result<String, String> {
    let jobs = JOBS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(|(_, jobs)| jobs.clone())
        .ok_or("captures are off")?;
    let (reply, answer) = mpsc::sync_channel(1);
    if !send(&jobs, Job::SendBundle { bundle, to, reply }) {
        return Err("the capture router is busy; try again shortly".into());
    }
    answer
        .recv_timeout(CLI_WAIT)
        .unwrap_or_else(|_| Ok("Still sending; `hex capture show` tells how it went".into()))
}

/// The `capture-route` thread and its queue.
pub(super) struct Routes {
    id: u64,
    jobs: SyncSender<Job>,
    thread: JoinHandle<()>,
}

impl Routes {
    pub(super) fn start(outcomes: SharedOutcomes) -> std::io::Result<Self> {
        let (jobs, queued) = mpsc::sync_channel(QUEUE);
        let thread = thread::Builder::new()
            .name("capture-route".into())
            .spawn(move || Worker::new(outcomes).run(&queued))?;
        let id = NEXT_ROUTES.fetch_add(1, Ordering::Relaxed);
        *JOBS.lock().unwrap_or_else(|error| error.into_inner()) = Some((id, jobs.clone()));
        Ok(Self { id, jobs, thread })
    }

    pub(super) fn jobs(&self) -> SyncSender<Job> {
        self.jobs.clone()
    }

    /// Queues `job` without waiting; a full queue drops it.
    pub(super) fn send(&self, job: Job) -> bool {
        send(&self.jobs, job)
    }

    /// Ends the thread once its queue is empty and every other sender is
    /// gone. Rule sends still waiting out [`UNDO`] are kept, not sent.
    pub(super) fn stop(self, deadline: Instant) {
        let mut global = JOBS.lock().unwrap_or_else(|error| error.into_inner());
        if global.as_ref().is_some_and(|(id, _)| *id == self.id) {
            *global = None;
        }
        drop(global);
        drop(self.jobs);
        super::join_by(self.thread, deadline, "capture-route");
    }
}

pub(super) fn send(jobs: &SyncSender<Job>, job: Job) -> bool {
    match jobs.try_send(job) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            tracing::warn!("the capture router is behind; dropping a request");
            false
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// The newest routed capture, which the capsule's commands act on.
struct Routed {
    bundle: PathBuf,
    router: Router,
    url: Option<String>,
    app: Option<String>,
    /// What an always-rule would match.
    rule: Option<(RuleKind, String)>,
    candidates: Vec<smart::Candidate>,
    route: CaptureRoute,
    /// How to take the send back, once OpenCode took it.
    sent: Option<Sent>,
    /// The rule offered after the send ([`CaptureRoute::offer`]).
    offered: Option<scrawl_capture::OfferRule>,
}

struct Sent {
    delivery_id: String,
    session: String,
}

/// A rule's send, waiting out [`UNDO`].
struct Waiting {
    at: Instant,
    bundle: PathBuf,
    router: Router,
    dest: Destination,
    app: Option<String>,
}

struct Worker {
    outcomes: SharedOutcomes,
    current: Option<Routed>,
    waiting: Vec<Waiting>,
    retry_at: Instant,
}

impl Worker {
    fn new(outcomes: SharedOutcomes) -> Self {
        Self {
            outcomes,
            current: None,
            waiting: Vec::new(),
            retry_at: Instant::now() + RETRY_EVERY,
        }
    }

    fn run(mut self, jobs: &Receiver<Job>) {
        loop {
            let next = self
                .waiting
                .iter()
                .map(|waiting| waiting.at)
                .fold(self.retry_at, Instant::min);
            match jobs.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(job) => self.handle(job),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.send_due();
            if Instant::now() >= self.retry_at {
                self.retry();
                self.retry_at = Instant::now() + RETRY_EVERY;
            }
        }
        if !self.waiting.is_empty() {
            tracing::info!(
                kept = self.waiting.len(),
                "Hex stopped before a capture's send; it stays copied"
            );
        }
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Written { bundle, look } => self.written(bundle, look),
            Job::Send { to, always } => self.chosen(&to, always),
            Job::Undo => self.undo(),
            Job::Redirect { to } => self.redirect(&to),
            Job::SendBundle { bundle, to, reply } => {
                let _ = reply.try_send(self.send_bundle(&bundle, to.as_deref()));
            }
        }
    }

    /// `hex capture send`: the newest capture as the capsule's pick, any
    /// other straight to `to` or where it last went.
    fn send_bundle(&mut self, bundle: &Path, to: Option<&str>) -> Result<String, String> {
        if self.is_current(bundle) && self.acting_on_last() {
            let current = self.current.as_ref().ok_or("no capture")?;
            let to = match to {
                Some(to) => to.to_owned(),
                None => match &current.route.answer {
                    RouteAnswer::Rule { to, .. } => to.session.clone(),
                    _ => recorded_destination(bundle)
                        .map(|dest| dest.session)
                        .ok_or("its route names no session; say where with --to")?,
                },
            };
            if self.resolve(&to).is_none() {
                return Err(format!("{to} isn't a session this capture lists"));
            }
            self.chosen(&to, false);
            let current = self.current.as_ref().ok_or("no capture")?;
            return Ok(current
                .route
                .delivery
                .as_ref()
                .map_or_else(|| "Nothing sent".into(), describe));
        }
        let dest = match to {
            Some(to) if to.starts_with("ses") => Destination {
                session: to.to_owned(),
                title: to.to_owned(),
                via: "chosen".into(),
                url: None,
                directory: None,
            },
            Some(_) => return Err("an older capture is sent to a session id (ses…)".into()),
            None => recorded_destination(bundle)
                .ok_or("it was never sent anywhere; say where with --to")?,
        };
        let loaded = scrawl_capture::handoff::load_bundle(bundle)
            .map_err(|error| format!("couldn't read the capture: {error:#}"))?;
        let (delivery, _, _) = deliver(&router(), bundle, &dest, loaded.1.target.app);
        tracing::info!(
            bundle = %bundle_name(bundle),
            session = dest.session,
            delivery = delivery_kind(&delivery),
            "capture sent from the CLI"
        );
        match delivery {
            RouteDelivery::Failed { why, .. } => Err(why),
            delivery => Ok(describe(&delivery)),
        }
    }

    fn written(&mut self, bundle: PathBuf, look: Look) {
        let Look { router, seen } = look;
        let seen = seen.join().unwrap_or_else(|_| {
            tracing::error!("looking for a capture's route panicked");
            Seen {
                live: None,
                ladder: None,
            }
        });
        let loaded = scrawl_capture::handoff::load_bundle(&bundle)
            .map(|(_, loaded)| loaded)
            .inspect_err(|error| tracing::warn!(error = %format!("{error:#}"), "could not read a capture to route"))
            .ok();
        let candidates = seen
            .live
            .as_ref()
            .map(|live| live.candidates.clone())
            .unwrap_or_default();
        let smart_routing = router.config().smart_routing;
        let (decision, url) = router.run(|| match (seen.live, &loaded) {
            (Some(live), Some(loaded)) if smart_routing => {
                let sessions_read = live.error.is_none() || !live.candidates.is_empty();
                let (judgement, url) = routing::judge_capture(&bundle, loaded, live);
                let decision = if sessions_read {
                    Decided::Judged(judgement.decision)
                } else {
                    Decided::Failed
                };
                (decision, url)
            }
            (Some(live), _) => {
                let decision = match seen.ladder {
                    Some(dest) => Decided::Ladder(dest),
                    None if live.error.is_none() => Decided::Judged(smart::Decision::Keep {
                        reason: String::new(),
                    }),
                    None => Decided::Failed,
                };
                (decision, live.url)
            }
            (None, _) => (Decided::Failed, None),
        });
        let bundle_id = loaded
            .as_ref()
            .and_then(|loaded| loaded.target.bundle_id.clone());
        let app = loaded.as_ref().and_then(|loaded| loaded.target.app.clone());
        let rule = policy::rule_key(url.as_deref(), bundle_id.as_deref());
        let site = rule.as_ref().map(|(kind, key)| match kind {
            RuleKind::Site => key
                .split_once("://")
                .map_or(key.as_str(), |(_, host)| host)
                .to_owned(),
            RuleKind::App => app.clone().unwrap_or_else(|| key.clone()),
        });
        let (answer, rule_send) = answer_for(decision, url.as_deref(), site, &candidates, now_ms());
        tracing::info!(
            bundle = %bundle_name(&bundle),
            answer = answer_kind(&answer),
            "capture routed"
        );
        let mut route = CaptureRoute::of(answer);
        if let Some(dest) = rule_send {
            route.delivery = Some(RouteDelivery::Pending {
                to: dest.title.clone(),
            });
            self.waiting.push(Waiting {
                at: Instant::now() + UNDO,
                bundle: bundle.clone(),
                router: router.clone(),
                dest,
                app: app.clone(),
            });
        }
        self.current = Some(Routed {
            bundle,
            router,
            url,
            app,
            rule,
            candidates,
            route,
            sent: None,
            offered: None,
        });
        self.publish();
    }

    /// Sends every rule send whose [`UNDO`] window ended.
    fn send_due(&mut self) {
        let now = Instant::now();
        let (due, waiting) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition::<Vec<_>, _>(|waiting| waiting.at <= now);
        self.waiting = waiting;
        for waiting in due {
            let Waiting {
                bundle,
                router,
                dest,
                app,
                ..
            } = waiting;
            if self.is_current(&bundle) {
                self.send_current(dest, None);
            } else {
                let (delivery, _, _) = deliver(&router, &bundle, &dest, app);
                tracing::info!(
                    bundle = %bundle_name(&bundle),
                    delivery = delivery_kind(&delivery),
                    "an earlier capture's rule send went"
                );
            }
        }
    }

    /// The capsule's pick: sends the newest capture to `to` now, after
    /// taking back a send elsewhere. `always` also saves a rule for its
    /// site or app.
    fn chosen(&mut self, to: &str, always: bool) {
        let Some(dest) = self.resolve(to) else {
            return;
        };
        let Some(current) = self.current.as_ref() else {
            return;
        };
        let already = match (&current.sent, &current.route.delivery) {
            (Some(sent), _) => Some(sent.session == dest.session),
            (None, Some(RouteDelivery::Queued { .. })) => Some(false),
            _ => None,
        };
        // Picking the rule's own session while it waits sends it now.
        let rule_pick = matches!(
            (&current.route.answer, &current.route.delivery),
            (RouteAnswer::Rule { to, .. }, Some(RouteDelivery::Pending { .. }))
                if to.session == dest.session
        );
        if always {
            self.save_rule(&dest.session);
        }
        match already {
            // Accepting the offered rule, or a second click.
            Some(true) => {}
            Some(false) => self.redirect_to(dest),
            None => match self.take_waiting() {
                Some(rule) if rule_pick => self.send_current(rule, None),
                _ => {
                    self.correct(jev::Action::Redirect, "chip", Some(&dest));
                    self.send_current(dest, Some("chosen"));
                }
            },
        }
    }

    fn redirect(&mut self, to: &str) {
        if let Some(dest) = self.resolve(to) {
            self.redirect_to(dest);
        }
    }

    fn redirect_to(&mut self, dest: Destination) {
        self.take_back();
        self.correct(jev::Action::Redirect, "reroute", Some(&dest));
        self.send_current(dest, Some("chosen"));
    }

    fn undo(&mut self) {
        if !self.acting_on_last() {
            return;
        }
        if self.take_back() {
            self.publish();
        }
    }

    /// Takes back the current capture's send, whatever state it is in;
    /// `false` when there was none.
    fn take_back(&mut self) -> bool {
        if self.cancel_waiting() {
            self.correct(jev::Action::Cancel, "dismiss", None);
            self.set_delivery(RouteDelivery::TakenBack);
            return true;
        }
        let Some(current) = self.current.as_mut() else {
            return false;
        };
        let to = delivered_to(current.route.delivery.as_ref());
        if let Some(sent) = current.sent.take() {
            let undone = current.router.undo(&sent.delivery_id, UNDO_DEADLINE);
            let delivery = match undone {
                Ok(report) => {
                    tracing::info!(
                        bundle = %bundle_name(&current.bundle),
                        undone = report.undone.label(),
                        "capture send taken back"
                    );
                    match report.undone {
                        Undone::Withdrawn | Undone::Reverted | Undone::Disowned => {
                            RouteDelivery::TakenBack
                        }
                        Undone::Gone | Undone::TooOld => RouteDelivery::TooLate {
                            to: to.unwrap_or_default(),
                        },
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "could not take a capture's send back");
                    current.sent = Some(sent);
                    return false;
                }
            };
            current.route.offer = None;
            current.offered = None;
            current.route.delivery = Some(delivery);
            return true;
        }
        if matches!(current.route.delivery, Some(RouteDelivery::Queued { .. })) {
            // Its retries stop with the marker.
            let _ = std::fs::remove_file(current.bundle.join(delivery::PENDING));
            self.correct(jev::Action::Cancel, "dismiss", None);
            self.set_delivery(RouteDelivery::TakenBack);
            return true;
        }
        false
    }

    /// Drops the current capture's waiting rule send; whether it had one.
    fn cancel_waiting(&mut self) -> bool {
        self.take_waiting().is_some()
    }

    /// Takes the current capture's waiting rule send out of the queue.
    fn take_waiting(&mut self) -> Option<Destination> {
        let bundle = &self.current.as_ref()?.bundle;
        let at = self
            .waiting
            .iter()
            .position(|waiting| waiting.bundle == *bundle)?;
        Some(self.waiting.remove(at).dest)
    }

    fn send_current(&mut self, mut dest: Destination, via: Option<&str>) {
        if let Some(via) = via {
            via.clone_into(&mut dest.via);
        }
        self.set_delivery(RouteDelivery::Sending {
            to: dest.title.clone(),
        });
        self.publish();
        let Some(current) = self.current.as_mut() else {
            return;
        };
        let (delivery, sent, offered) =
            deliver(&current.router, &current.bundle, &dest, current.app.clone());
        tracing::info!(
            bundle = %bundle_name(&current.bundle),
            session = dest.session,
            delivery = delivery_kind(&delivery),
            "capture sent"
        );
        current.sent = sent;
        current.route.offer = offered.as_ref().map(|offer| offer.key.clone());
        current.offered = offered;
        current.route.delivery = Some(delivery);
        self.publish();
    }

    /// Saves an always-rule from the current capture's site or app to
    /// `session`'s project (or the session itself), on top of what
    /// `settings.json` holds now.
    fn save_rule(&self, session: &str) {
        let Some(current) = &self.current else {
            return;
        };
        let rule = current
            .offered
            .as_ref()
            .map(|offer| (offer.kind, offer.key.clone(), offer.folder.clone()))
            .or_else(|| {
                let (kind, key) = current.rule.clone()?;
                let target = current
                    .candidates
                    .iter()
                    .find(|candidate| candidate.id == session)
                    .and_then(policy::rule_folder)
                    .map_or_else(|| session.to_owned(), |folder| folder.display().to_string());
                Some((kind, key, target))
            });
        let Some((kind, key, target)) = rule else {
            tracing::info!("no rule can be made for this capture");
            return;
        };
        let saved = crate::app_settings::AppSettings::load().and_then(|mut settings| {
            let next = settings
                .capture
                .routing
                .config()
                .with_rule(kind, &key, &target)
                .map_err(|error| color_eyre::eyre::eyre!("{error:#}"))?;
            settings.capture.routing = next.into();
            settings.save()
        });
        match saved {
            Ok(()) => {
                tracing::info!("saved a capture routing rule");
                #[cfg(target_os = "macos")]
                if let Some(runtime) = crate::macos_rune::runtime() {
                    let _ = runtime.send(crate::macos_rune::Request::ReloadSettings);
                }
            }
            Err(error) => tracing::warn!(%error, "could not save a capture routing rule"),
        }
    }

    /// Logs the person correcting or confirming the router (with Jev routing
    /// on); `detail` is how (`chip`, `reroute`, `dismiss`).
    fn correct(&self, action: jev::Action, detail: &str, to: Option<&Destination>) {
        if let Some(current) = &self.current {
            current.router.run(|| {
                jev::correct_capture(
                    &current.bundle,
                    action,
                    detail,
                    to.map(|to| (to.session.as_str(), to.title.as_str())),
                );
            });
        }
    }

    /// The session `to` names for the newest capture: a session ID, or a
    /// name from its route, suggestions first. `None` (logged) when there
    /// is no routed capture or nothing matches.
    fn resolve(&self, to: &str) -> Option<Destination> {
        if !self.acting_on_last() {
            return None;
        }
        let current = self.current.as_ref()?;
        let to = to.trim();
        let listed = listed(&current.route.answer);
        let found = listed.iter().find(|dest| dest.session == to).or_else(|| {
            listed
                .iter()
                .find(|dest| dest.name.eq_ignore_ascii_case(to))
        });
        let (session, title) = match found {
            Some(dest) => (dest.session.clone(), dest.name.clone()),
            None if to.starts_with("ses") => (to.to_owned(), to.to_owned()),
            None => {
                tracing::info!("the capsule named a session the route doesn't list");
                return None;
            }
        };
        let directory = current
            .candidates
            .iter()
            .find(|candidate| candidate.id == session)
            .and_then(|candidate| candidate.directory.as_ref())
            .map(|dir| dir.display().to_string());
        Some(Destination {
            session,
            title,
            via: "chosen".into(),
            url: current.url.clone(),
            directory,
        })
    }

    /// Commands act on the snapshot's newest capture only, and only when
    /// it is the one routed.
    fn acting_on_last(&self) -> bool {
        let last = lock(&self.outcomes)
            .last
            .as_ref()
            .map(|last| last.bundle.clone());
        self.current
            .as_ref()
            .is_some_and(|current| Some(&current.bundle) == last.as_ref())
    }

    fn is_current(&self, bundle: &Path) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.bundle == bundle)
    }

    fn set_delivery(&mut self, delivery: RouteDelivery) {
        if let Some(current) = self.current.as_mut() {
            current.route.delivery = Some(delivery);
        }
    }

    fn publish(&self) {
        if let Some(current) = &self.current {
            lock(&self.outcomes).routed(&current.bundle, current.route.clone());
        }
    }

    /// Retries sends OpenCode didn't take, unless another process owns
    /// retries: Scrawl's app, or anything serving Scrawl's ingest port.
    fn retry(&mut self) {
        if cfg!(test) || crate::app_settings::scrawl_app_running() || ingest_port_served() {
            return;
        }
        let router = router();
        let captures = crate::app_settings::capture_settings().captures_dir();
        let report = router.run(|| {
            delivery::retry_due(
                &captures,
                now_ms(),
                scrawl_capture::handoff::send_bundle_to_session,
            )
        });
        let outbox = router.retry_outbox();
        if !report.sent.is_empty() || !outbox.sent.is_empty() {
            tracing::info!(
                captures = report.sent.len(),
                prompts = outbox.sent.len(),
                "retried sends went"
            );
        }
        let sent_now = self
            .current
            .as_ref()
            .is_some_and(|current| report.sent.iter().any(|(dir, _)| *dir == current.bundle));
        if sent_now
            && let Some(current) = self.current.as_mut()
            && let Some(RouteDelivery::Queued { to }) = current.route.delivery.take()
        {
            current.route.delivery = Some(RouteDelivery::Sent { to });
            self.publish();
        }
    }
}

/// What the router made of a capture, before it is shown.
enum Decided {
    Judged(smart::Decision),
    /// The port ladder's send (smart routing off).
    Ladder(Destination),
    Failed,
}

/// The answer to show, and the destination of a rule's send.
fn answer_for(
    decided: Decided,
    url: Option<&str>,
    site: Option<String>,
    candidates: &[smart::Candidate],
    now_ms: u64,
) -> (RouteAnswer, Option<Destination>) {
    let scored = |scored: &smart::Scored| {
        let dest = smart::destination_of(scored, url);
        RouteDest {
            session: dest.session,
            name: dest.title,
            why: why(&dest.via).to_owned(),
        }
    };
    let recent = |shown: &[&str]| recent(candidates, shown, now_ms);
    match decided {
        Decided::Judged(smart::Decision::Send { to, instead }) => {
            let dest = smart::destination_of(&to, url);
            let to = scored(&to);
            let suggested: Vec<_> = instead.iter().map(scored).collect();
            let mut shown: Vec<&str> = suggested.iter().map(|d| d.session.as_str()).collect();
            shown.push(&to.session);
            let recent = recent(&shown);
            (
                RouteAnswer::Rule {
                    to,
                    suggested,
                    recent,
                },
                Some(dest),
            )
        }
        Decided::Judged(smart::Decision::Offer { choices }) => {
            let suggested: Vec<_> = choices.iter().map(scored).collect();
            let shown: Vec<&str> = suggested.iter().map(|d| d.session.as_str()).collect();
            let recent = recent(&shown);
            (
                RouteAnswer::Guess {
                    suggested,
                    recent,
                    site,
                },
                None,
            )
        }
        Decided::Judged(smart::Decision::Keep { .. }) => (
            RouteAnswer::Nothing {
                recent: recent(&[]),
            },
            None,
        ),
        Decided::Ladder(dest) => {
            let to = RouteDest {
                session: dest.session.clone(),
                name: dest.title.clone(),
                why: why(&dest.via).to_owned(),
            };
            let recent = recent(&[&to.session]);
            (
                RouteAnswer::Rule {
                    to,
                    suggested: Vec::new(),
                    recent,
                },
                Some(dest),
            )
        }
        Decided::Failed => (
            RouteAnswer::NoAnswer {
                recent: recent(&[]),
            },
            None,
        ),
    }
}

/// The newest sessions not already shown, with how long ago each moved.
fn recent(candidates: &[smart::Candidate], shown: &[&str], now_ms: u64) -> Vec<RouteDest> {
    let mut recent: Vec<_> = candidates
        .iter()
        .filter(|candidate| !shown.contains(&candidate.id.as_str()))
        .collect();
    recent.sort_by_key(|candidate| std::cmp::Reverse(candidate.updated_ms));
    recent
        .into_iter()
        .take(RECENT)
        .map(|candidate| RouteDest {
            session: candidate.id.clone(),
            name: candidate.label().to_owned(),
            why: format!(
                "{} ago",
                smart::ago_text(now_ms.saturating_sub(candidate.updated_ms))
            ),
        })
        .collect()
}

/// A destination's `via` as a quiet word.
fn why(via: &str) -> &str {
    match via {
        "rule" | "app-rule" => "rule",
        "sdk" | "page" => "page",
        "localhost" | "port" => "dev server",
        "recorded" => "sent before",
        "publisher" => "published it",
        "dev-build" => "dev build",
        "jev" | "jev-home" => "guess",
        "weighed" => "recent",
        other => other,
    }
}

/// Every session the route lists, suggestions first.
fn listed(answer: &RouteAnswer) -> Vec<&RouteDest> {
    match answer {
        RouteAnswer::Looking => Vec::new(),
        RouteAnswer::Rule {
            to,
            suggested,
            recent,
        } => std::iter::once(to).chain(suggested).chain(recent).collect(),
        RouteAnswer::Guess {
            suggested, recent, ..
        } => suggested.iter().chain(recent).collect(),
        RouteAnswer::Nothing { recent } | RouteAnswer::NoAnswer { recent } => {
            recent.iter().collect()
        }
    }
}

/// Where a capture's bundle says it was sent.
fn recorded_destination(bundle: &Path) -> Option<Destination> {
    scrawl_capture::handoff::load_bundle(bundle)
        .ok()
        .and_then(|(_, loaded)| loaded.destination)
}

/// A delivery as a sentence for `hex capture send`.
fn describe(delivery: &RouteDelivery) -> String {
    match delivery {
        RouteDelivery::Pending { to } => format!("Sending to {to} in a moment"),
        RouteDelivery::Sending { to } => format!("Sending to {to}"),
        RouteDelivery::Sent { to } => format!("Sent to {to}"),
        RouteDelivery::Queued { to } => format!("Queued for {to}; OpenCode didn't answer"),
        RouteDelivery::TakenBack => "Taken back".into(),
        RouteDelivery::TooLate { to } => format!("Already read in {to}"),
        RouteDelivery::Failed { to, why } => format!("Not sent to {to}: {why}"),
    }
}

fn delivered_to(delivery: Option<&RouteDelivery>) -> Option<String> {
    match delivery? {
        RouteDelivery::Pending { to }
        | RouteDelivery::Sending { to }
        | RouteDelivery::Sent { to }
        | RouteDelivery::Queued { to }
        | RouteDelivery::TooLate { to }
        | RouteDelivery::Failed { to, .. } => Some(to.clone()),
        RouteDelivery::TakenBack => None,
    }
}

/// Records where the capture went in its bundle, sends it, and remembers
/// the send for an undo. A send OpenCode didn't take is queued for
/// [`Worker::retry`].
fn deliver(
    router: &Router,
    bundle: &Path,
    dest: &Destination,
    app: Option<String>,
) -> (
    RouteDelivery,
    Option<Sent>,
    Option<scrawl_capture::OfferRule>,
) {
    router.run(|| {
        let recorded = scrawl_capture::handoff::load_bundle(bundle).and_then(
            |(_, mut loaded): (PathBuf, ScrawlBundle)| {
                routing::record_destination(bundle, &mut loaded, dest.clone())
            },
        );
        if let Err(error) = recorded {
            tracing::warn!(error = %format!("{error:#}"), "could not record a capture's destination");
        }
        let (delivery, posted) = delivery::deliver_by(
            bundle,
            &dest.session,
            &dest.title,
            None,
            scrawl_capture::handoff::send_bundle,
        );
        let sent = posted.map(|posted| {
            let sent = undo::remember(
                &dest.session,
                Some(&dest.title),
                &posted,
                undo::Origin {
                    kind: "capture".into(),
                    bundle: Some(bundle.to_path_buf()),
                    app,
                    ..undo::Origin::default()
                },
            );
            Sent {
                delivery_id: sent.delivery_id,
                session: dest.session.clone(),
            }
        });
        let to = dest.title.clone();
        match delivery {
            Delivery::Sent { .. } => (
                RouteDelivery::Sent { to },
                sent,
                routing::learn_capture(bundle, &dest.session),
            ),
            Delivery::Queued { .. } => (RouteDelivery::Queued { to }, None, None),
            Delivery::Saved => (
                RouteDelivery::Failed {
                    to,
                    why: "nothing was sent".into(),
                },
                None,
                None,
            ),
        }
    })
}

/// Whether something serves Scrawl's ingest port: that process retries
/// deliveries, and two must never race over a bundle.
fn ingest_port_served() -> bool {
    let port = scrawl_capture::phone::DEFAULT_INGEST_PORT;
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(100),
    )
    .is_ok()
}

fn bundle_name(bundle: &Path) -> String {
    bundle
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn answer_kind(answer: &RouteAnswer) -> &'static str {
    match answer {
        RouteAnswer::Looking => "looking",
        RouteAnswer::Rule { .. } => "rule",
        RouteAnswer::Guess { .. } => "guess",
        RouteAnswer::Nothing { .. } => "nothing",
        RouteAnswer::NoAnswer { .. } => "no-answer",
    }
}

fn delivery_kind(delivery: &RouteDelivery) -> &'static str {
    match delivery {
        RouteDelivery::Pending { .. } => "pending",
        RouteDelivery::Sending { .. } => "sending",
        RouteDelivery::Sent { .. } => "sent",
        RouteDelivery::Queued { .. } => "queued",
        RouteDelivery::TakenBack => "taken-back",
        RouteDelivery::TooLate { .. } => "too-late",
        RouteDelivery::Failed { .. } => "failed",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, title: &str, updated_ms: u64) -> smart::Candidate {
        smart::Candidate {
            id: id.into(),
            title: title.into(),
            directory: Some(PathBuf::from(format!("/tmp/{id}"))),
            repo: None,
            updated_ms,
            viewed_ms: None,
        }
    }

    fn scored(id: &str, title: &str) -> smart::Scored {
        smart::Scored {
            candidate: candidate(id, title, 0),
            score: 60,
            reasons: Vec::new(),
        }
    }

    #[test]
    fn a_rule_sends_after_undo_and_lists_the_rest() {
        let candidates = [
            candidate("ses_a", "Alpha", 1_000),
            candidate("ses_b", "Beta", 3_000),
            candidate("ses_c", "Gamma", 2_000),
        ];
        let (answer, send) = answer_for(
            Decided::Judged(smart::Decision::Send {
                to: scored("ses_a", "Alpha"),
                instead: vec![scored("ses_c", "Gamma")],
            }),
            None,
            None,
            &candidates,
            63_000,
        );
        assert_eq!(send.map(|dest| dest.session), Some("ses_a".into()));
        let RouteAnswer::Rule {
            to,
            suggested,
            recent,
        } = answer
        else {
            panic!("not a rule: {answer:?}");
        };
        assert_eq!(to.name, "Alpha");
        assert_eq!(suggested.len(), 1);
        assert_eq!(suggested[0].session, "ses_c");
        assert_eq!(
            recent,
            vec![RouteDest {
                session: "ses_b".into(),
                name: "Beta".into(),
                why: "1 min ago".into(),
            }]
        );
    }

    #[test]
    fn a_guess_sends_nothing_and_a_failure_still_lists_recent_sessions() {
        let candidates = [candidate("ses_a", "Alpha", 0)];
        let (answer, send) = answer_for(
            Decided::Judged(smart::Decision::Offer {
                choices: vec![scored("ses_a", "Alpha")],
            }),
            None,
            Some("example.com".into()),
            &candidates,
            0,
        );
        assert!(send.is_none());
        assert!(matches!(
            answer,
            RouteAnswer::Guess { ref suggested, ref recent, site: Some(ref site) }
                if suggested.len() == 1 && recent.is_empty() && site == "example.com"
        ));
        let (answer, send) = answer_for(Decided::Failed, None, None, &candidates, 0);
        assert!(send.is_none());
        assert!(matches!(answer, RouteAnswer::NoAnswer { ref recent } if recent.len() == 1));
    }

    #[test]
    fn the_ladder_is_a_rule() {
        let (answer, send) = answer_for(
            Decided::Ladder(Destination {
                session: "ses_a".into(),
                title: "Alpha".into(),
                via: "localhost".into(),
                url: None,
                directory: None,
            }),
            None,
            None,
            &[],
            0,
        );
        assert!(send.is_some());
        assert!(matches!(answer, RouteAnswer::Rule { ref to, .. } if to.why == "dev server"));
    }
}
