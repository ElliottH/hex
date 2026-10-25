//! The `capture` experiment (debug builds only): Scrawl's capture engine
//! hosted inside Hex, step 3 of `docs/plans/capture-in-hex.md`.
//!
//! Hex hosts the engine through a small seam shaped like the API Scrawl's
//! `scrawl-capture` crate will expose: [`Host`], [`Transcriber`],
//! [`ConfigSource`], and [`Engine::start`] returning a [`Capture`] that
//! [`Capture::finish`]es with the [`RecordedAudio`] Hex recorded. Until that
//! crate exists, [`Engine`] is a temporary stand-in that writes `audio.wav`,
//! `words.json`, and `manifest.json` into a bundle folder.
//!
//! Hex owns the microphone and the model. A capture copies audio from the
//! dictation timeline through a bounded sink (no second stream), and its
//! words come from the dictation engine's loaded model as foreground work.
//! [`CaptureHost::start`] does nothing unless the experiment is on.

use std::fs;
use std::fs::{DirBuilder, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde::Serialize;

use crate::audio::CaptureInstant;
use crate::control::{
    ControlReply, ScrawlCaptureAction, ScrawlCaptureOutcome, ScrawlCaptureReport,
    ScrawlCaptureState,
};
use crate::dictation::StreamingResampler;
use crate::dictation_audio::{CaptureAudio, CaptureSinkClosed, DictationAudio};
use crate::transcription_engine::TranscriptionEngine;

/// The sink's capacity in microphone chunks (about 10 ms each): seconds of
/// slack before a stalled collector starts dropping audio as a gap.
const SINK_CAPACITY: usize = 512;
const EVENT_CAPACITY: usize = 64;
const SAMPLE_RATE: u32 = 16_000;
/// A capture keeps at most this much audio; the rest is recorded as a gap.
const MAX_CAPTURE: Duration = Duration::from_secs(20 * 60);
const GAP_TOLERANCE: Duration = Duration::from_millis(50);
/// Captures being written at once: the one finishing plus those queued
/// behind it. A start beyond this is refused.
const MAX_FINISHING: usize = 2;
/// How long shutdown waits for capture threads, inside the quit deadline.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// The seam: what Scrawl's engine will take from its host.

/// What the host provides. Both calls must return at once.
pub trait Host: Send + Sync + 'static {
    fn emit(&self, event: Event);
    fn log(&self, line: &str);
}

/// Transcribes recorded audio with word timestamps. `Ok(None)` means none
/// is available now. Times are milliseconds from `audio.samples[0]`.
pub trait Transcriber: Send + Sync {
    fn transcribe(&self, audio: &RecordedAudio) -> Result<Option<TranscriptData>>;
}

pub trait ConfigSource: Send + Sync {
    fn engine_config(&self) -> Result<EngineConfig>;
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub captures_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub enum Event {
    Progress { bundle: PathBuf, stage: Stage },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Recording,
    Transcribing,
    Writing,
    Done,
    Cancelled,
    Failed,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
            Self::Writing => "writing",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

/// Mono audio on the capture's clock. `samples[0]` is `start_offset_ms`
/// after the capture started; spans in `gaps` (also from the capture's
/// start) are silence where audio was missing or dropped.
#[derive(Clone, Debug, Default)]
pub struct RecordedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub start_offset_ms: u64,
    pub gaps: Vec<GapSpan>,
}

impl RecordedAudio {
    pub fn duration_ms(&self) -> u64 {
        self.start_offset_ms + ms_for(self.samples.len())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GapSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TranscriptData {
    pub source: String,
    pub model: Option<String>,
    pub text: String,
    pub words: Vec<WordTiming>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WordTiming {
    pub t0: u64,
    pub t1: u64,
    pub text: String,
}

pub struct Start {
    /// The capture's start on the boot-time nanosecond clock that audio and
    /// input timestamps share.
    pub t0_boot_ns: u64,
    pub bundle_dir: Option<PathBuf>,
    pub note: Option<String>,
}

pub struct Finished {
    pub dir: PathBuf,
    pub words: usize,
    pub summary: String,
}

/// Ink and pointer input in global top-left points, ms since capture start.
/// Hex feeds none yet (step 4); the stand-in only counts it.
#[derive(Default)]
pub struct InputSink {
    events: std::sync::atomic::AtomicUsize,
}

#[allow(dead_code)] // Hex's input taps feed it in step 4.
impl InputSink {
    pub fn pointer_down(&self, _t_ms: u64, _x: f64, _y: f64) {
        self.count();
    }

    pub fn ink(&self, _t_ms: u64, _x: f64, _y: f64, _shift_held: bool) {
        self.count();
    }

    pub fn flag(&self, _t_ms: u64) {
        self.count();
    }

    fn count(&self) {
        self.events
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// The stand-in engine. Temporary: replaced by `scrawl_capture::Engine`.

pub struct Engine {
    host: Arc<dyn Host>,
    config: Arc<dyn ConfigSource>,
    transcriber: Option<Arc<dyn Transcriber>>,
}

impl Engine {
    pub fn new(host: Arc<dyn Host>, config: Arc<dyn ConfigSource>) -> Self {
        Self {
            host,
            config,
            transcriber: None,
        }
    }

    pub fn with_transcriber(mut self, transcriber: Arc<dyn Transcriber>) -> Self {
        self.transcriber = Some(transcriber);
        self
    }

    /// Creates the bundle folder and returns at once.
    pub fn start(&self, start: Start) -> Result<Capture> {
        let dir = match start.bundle_dir {
            Some(dir) => {
                private_dirs(dir.parent().unwrap_or(&dir))?;
                private_dir(&dir)?;
                dir
            }
            None => new_bundle_dir(&self.config.engine_config()?.captures_dir)?,
        };
        self.host.emit(Event::Progress {
            bundle: dir.clone(),
            stage: Stage::Recording,
        });
        Ok(Capture {
            host: self.host.clone(),
            transcriber: self.transcriber.clone(),
            dir,
            t0: CaptureInstant::from_nanos(start.t0_boot_ns),
            note: start.note,
            input: InputSink::default(),
        })
    }
}

/// Creates `dir` and missing parents owner-only, and makes `dir` itself
/// owner-only if it already existed.
fn private_dirs(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .wrap_err_with(|| format!("could not create {}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .wrap_err_with(|| format!("could not restrict {}", dir.display()))
}

/// Creates `dir`, owner-only; fails if it exists.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    DirBuilder::new().mode(0o700).create(dir)
}

/// Creates or truncates an owner-only file.
fn private_file(path: &Path) -> Result<fs::File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .wrap_err_with(|| format!("could not write {}", path.display()))
}

/// A fresh `<captures>/<unix ms>.capture` folder; never adopts one that exists.
fn new_bundle_dir(captures_dir: &Path) -> Result<PathBuf> {
    private_dirs(captures_dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    for attempt in 0..100 {
        let name = match attempt {
            0 => format!("{stamp}.capture"),
            n => format!("{stamp}-{n}.capture"),
        };
        let dir = captures_dir.join(name);
        match private_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).wrap_err_with(|| format!("could not create {}", dir.display()));
            }
        }
    }
    Err(eyre!(
        "could not name a new capture in {}",
        captures_dir.display()
    ))
}

pub struct Capture {
    host: Arc<dyn Host>,
    transcriber: Option<Arc<dyn Transcriber>>,
    dir: PathBuf,
    t0: CaptureInstant,
    note: Option<String>,
    input: InputSink,
}

impl Capture {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn started_at_boot_ns(&self) -> u64 {
        self.t0.as_nanos()
    }

    #[allow(dead_code)] // Hex's input taps feed it in step 4.
    pub fn input(&self) -> &InputSink {
        &self.input
    }

    /// Discards the bundle.
    pub fn cancel(self) {
        if let Err(error) = fs::remove_dir_all(&self.dir) {
            self.host
                .log(&format!("could not remove {}: {error}", self.dir.display()));
        }
        self.host.emit(Event::Progress {
            bundle: self.dir,
            stage: Stage::Cancelled,
        });
    }

    /// Blocking: transcribes and writes the bundle. Run it off the caller's
    /// thread.
    pub fn finish(self, audio: Option<RecordedAudio>) -> Result<Finished> {
        let result = self.write(audio.unwrap_or_default());
        self.host.emit(Event::Progress {
            bundle: self.dir.clone(),
            stage: if result.is_ok() {
                Stage::Done
            } else {
                Stage::Failed
            },
        });
        result
    }

    fn write(&self, audio: RecordedAudio) -> Result<Finished> {
        let progress = |stage| {
            self.host.emit(Event::Progress {
                bundle: self.dir.clone(),
                stage,
            })
        };
        progress(Stage::Transcribing);
        let (transcript, transcript_error) = match &self.transcriber {
            Some(transcriber) => match transcriber.transcribe(&audio) {
                Ok(transcript) => (transcript, None),
                Err(error) => {
                    self.host
                        .log(&format!("capture transcription failed: {error:#}"));
                    (None, Some(format!("{error:#}")))
                }
            },
            None => (None, None),
        };
        // Words come back relative to the first sample; the bundle keeps the
        // capture's clock.
        let transcript = transcript.map(|mut transcript| {
            for word in &mut transcript.words {
                word.t0 += audio.start_offset_ms;
                word.t1 += audio.start_offset_ms;
            }
            transcript
        });
        progress(Stage::Writing);
        write_wav(&self.dir.join("audio.wav"), &audio)?;
        if let Some(transcript) = &transcript {
            write_json(
                &self.dir.join("words.json"),
                &WordsFile {
                    version: 1,
                    transcript,
                },
            )?;
        }
        let words = transcript.as_ref().map_or(0, |t| t.words.len());
        write_json(
            &self.dir.join("manifest.json"),
            &Manifest {
                version: 1,
                engine: "hex-stand-in",
                t0_boot_ns: self.t0.as_nanos(),
                duration_ms: audio.duration_ms(),
                sample_rate: audio.sample_rate,
                start_offset_ms: audio.start_offset_ms,
                gaps: &audio.gaps,
                note: self.note.as_deref(),
                words: transcript.as_ref().map(|_| "words.json"),
                transcript_error: transcript_error.as_deref(),
            },
        )?;
        Ok(Finished {
            dir: self.dir.clone(),
            words,
            summary: format!(
                "{} ms, {words} words, {} gaps",
                audio.duration_ms(),
                audio.gaps.len()
            ),
        })
    }
}

#[derive(Serialize)]
struct WordsFile<'a> {
    version: u32,
    #[serde(flatten)]
    transcript: &'a TranscriptData,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
    version: u32,
    engine: &'static str,
    t0_boot_ns: u64,
    duration_ms: u64,
    sample_rate: u32,
    start_offset_ms: u64,
    gaps: &'a [GapSpan],
    note: Option<&'a str>,
    words: Option<&'static str>,
    transcript_error: Option<&'a str>,
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    let json = serde_json::to_vec_pretty(value)?;
    private_file(path)?
        .write_all(&json)
        .wrap_err_with(|| format!("could not write {}", path.display()))
}

fn write_wav(path: &Path, audio: &RecordedAudio) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: audio.sample_rate.max(1),
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(std::io::BufWriter::new(private_file(path)?), spec)
        .wrap_err_with(|| format!("could not write {}", path.display()))?;
    for sample in &audio.samples {
        writer.write_sample((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Hex's side of the seam.

/// Forwards events to a bounded channel (the `capture-events` thread logs
/// them and keeps `hex capture status`) and logs through tracing. Never
/// blocks: a full channel drops the event.
struct HexHost {
    events: SyncSender<Event>,
}

impl Host for HexHost {
    fn emit(&self, event: Event) {
        if let Err(TrySendError::Full(event)) = self.events.try_send(event) {
            tracing::warn!(?event, "capture event queue is full; dropping an event");
        }
    }

    fn log(&self, line: &str) {
        tracing::info!(target: "hex::capture", "{line}");
    }
}

struct HexConfig;

impl ConfigSource for HexConfig {
    fn engine_config(&self) -> Result<EngineConfig> {
        Ok(EngineConfig {
            captures_dir: crate::app_paths::captures_dir()?,
        })
    }
}

/// Word-timed transcription on the dictation engine's loaded model. It holds
/// the engine weakly, so a capture still writing never keeps it alive.
struct HexTranscriber {
    engine: Option<Weak<TranscriptionEngine>>,
}

impl Transcriber for HexTranscriber {
    fn transcribe(&self, audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
        let Some(engine) = &self.engine else {
            return Ok(None);
        };
        let samples = if audio.sample_rate == SAMPLE_RATE {
            audio.samples.clone()
        } else {
            crate::dictation::try_resample_for_parakeet(&audio.samples, audio.sample_rate)
                .map_err(|error| eyre!(error))?
        };
        let transcript = crate::parakeet::transcribe_with_words(engine, samples)?;
        let (_, selection) = crate::app_settings::transcription_selection();
        let ms = |value: i64| value.max(0) as u64;
        Ok(Some(TranscriptData {
            source: "hex".into(),
            model: Some(selection.model.as_str().into()),
            text: transcript.text,
            words: transcript
                .words
                .into_iter()
                .map(|word| WordTiming {
                    t0: ms(word.start_ms),
                    t1: ms(word.end_ms),
                    text: word.text,
                })
                .collect(),
        }))
    }
}

/// Where a capture's audio comes from: the dictation timeline.
pub trait AudioTimeline {
    fn open_sink(&self, sink: SyncSender<CaptureAudio>, since: CaptureInstant) -> Result<()>;
    fn close_sink(&self, through: CaptureInstant) -> Result<Option<CaptureSinkClosed>>;
}

impl AudioTimeline for DictationAudio {
    fn open_sink(&self, sink: SyncSender<CaptureAudio>, since: CaptureInstant) -> Result<()> {
        self.open_capture_sink(sink, since)
    }

    fn close_sink(&self, through: CaptureInstant) -> Result<Option<CaptureSinkClosed>> {
        self.close_capture_sink(through)
    }
}

/// How a capture's collector ends.
enum Seal {
    Finish {
        through: CaptureInstant,
        closed: Option<CaptureSinkClosed>,
    },
    Cancel,
}

struct Active {
    bundle: PathBuf,
    t0: CaptureInstant,
    seal: SyncSender<Seal>,
    collector: JoinHandle<()>,
}

/// A stopped capture's audio, waiting for the finisher.
struct FinishJob {
    capture: Capture,
    audio: RecordedAudio,
}

#[derive(Default)]
struct Outcomes {
    finishing: Vec<PathBuf>,
    last: Option<ScrawlCaptureOutcome>,
}

impl Outcomes {
    fn settle(&mut self, bundle: PathBuf, stage: Stage, words: Option<usize>) {
        self.finishing.retain(|finishing| finishing != &bundle);
        self.last = Some(ScrawlCaptureOutcome {
            bundle,
            stage: stage.label().into(),
            words,
        });
    }
}

type SharedOutcomes = Arc<Mutex<Outcomes>>;

fn lock(outcomes: &Mutex<Outcomes>) -> std::sync::MutexGuard<'_, Outcomes> {
    outcomes.lock().unwrap_or_else(|error| error.into_inner())
}

/// The `hex capture` controller, owned by the listener beside its
/// `DictationAudio`. The recording capture gets a `capture-audio` thread
/// that collects its audio; one `capture-finish` thread then writes stopped
/// captures one at a time, off the listener's thread.
pub struct CaptureHost {
    engine: Arc<Engine>,
    active: Option<Active>,
    outcomes: SharedOutcomes,
    finisher: Option<(SyncSender<FinishJob>, JoinHandle<()>)>,
    /// Collectors of ended captures, until they exit.
    ended: Vec<JoinHandle<()>>,
}

impl CaptureHost {
    /// The host when the `capture` experiment is on; otherwise `None`, and
    /// no thread, sink, or folder is created.
    pub fn start(engine: Option<Arc<TranscriptionEngine>>) -> Option<Self> {
        if !crate::experiments::enabled(crate::experiments::Experiment::Capture) {
            return None;
        }
        tracing::info!("capture experiment on: `hex capture` is available");
        let engine = engine.as_ref().map(Arc::downgrade);
        match Self::with(Arc::new(HexConfig), Arc::new(HexTranscriber { engine })) {
            Ok(host) => Some(host),
            Err(error) => {
                tracing::warn!(%error, "could not start the capture host");
                None
            }
        }
    }

    fn with(config: Arc<dyn ConfigSource>, transcriber: Arc<dyn Transcriber>) -> Result<Self> {
        let (events, received) = mpsc::sync_channel(EVENT_CAPACITY);
        thread::Builder::new()
            .name("capture-events".into())
            .spawn(move || log_events(received))?;
        let outcomes = SharedOutcomes::default();
        let (jobs, queued) = mpsc::sync_channel(MAX_FINISHING);
        let finisher = {
            let outcomes = outcomes.clone();
            thread::Builder::new()
                .name("capture-finish".into())
                .spawn(move || finish_captures(queued, &outcomes))?
        };
        let host: Arc<dyn Host> = Arc::new(HexHost { events });
        Ok(Self {
            engine: Arc::new(Engine::new(host, config).with_transcriber(transcriber)),
            active: None,
            outcomes,
            finisher: Some((jobs, finisher)),
            ended: Vec::new(),
        })
    }

    pub fn control(
        &mut self,
        action: ScrawlCaptureAction,
        audio: &impl AudioTimeline,
    ) -> ControlReply {
        let now = CaptureInstant::now();
        let result = match action {
            ScrawlCaptureAction::Start => self.begin(now, audio),
            ScrawlCaptureAction::Stop => self.end(now, audio, false),
            ScrawlCaptureAction::Cancel => self.end(now, audio, true),
            ScrawlCaptureAction::Status => Ok(self.report(false, None)),
        };
        result.unwrap_or_else(|error| ControlReply::error("failed", format!("{error:#}")))
    }

    /// Cancels a recording capture, as when Hex quits, and waits a bounded
    /// time for its cleanup and for captures still writing. Quitting stops
    /// their transcription, so they keep their audio without words.
    pub fn shutdown(&mut self, audio: &impl AudioTimeline) {
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        if self.active.is_some() {
            let _ = self.end(CaptureInstant::now(), audio, true);
        }
        for collector in std::mem::take(&mut self.ended) {
            join_by(collector, deadline, "capture-audio");
        }
        // Dropping the queue's sender ends the finisher after its last job.
        if let Some((jobs, finisher)) = self.finisher.take() {
            drop(jobs);
            join_by(finisher, deadline, "capture-finish");
        }
    }

    fn begin(&mut self, now: CaptureInstant, audio: &impl AudioTimeline) -> Result<ControlReply> {
        if self.active.is_some() {
            return Ok(self.report(false, None));
        }
        let Some((jobs, _)) = &self.finisher else {
            return Ok(ControlReply::error("busy", "Hex is stopping"));
        };
        let writing = lock(&self.outcomes).finishing.len();
        if writing >= MAX_FINISHING {
            return Ok(ControlReply::error(
                "busy",
                format!("{writing} captures are still being written; try again shortly"),
            ));
        }
        self.ended.retain(|collector| !collector.is_finished());
        let capture = self.engine.start(Start {
            t0_boot_ns: now.as_nanos(),
            bundle_dir: None,
            note: None,
        })?;
        let (sink, chunks) = mpsc::sync_channel(SINK_CAPACITY);
        if let Err(error) = audio.open_sink(sink, now) {
            capture.cancel();
            return Err(error);
        }
        let (seal, sealed) = mpsc::sync_channel(1);
        let bundle = capture.dir().to_path_buf();
        let collector = {
            let jobs = jobs.clone();
            let outcomes = self.outcomes.clone();
            thread::Builder::new()
                .name("capture-audio".into())
                .spawn(move || collect(capture, chunks, sealed, &jobs, &outcomes))
                .inspect_err(|_| {
                    let _ = audio.close_sink(now);
                })?
        };
        tracing::info!(bundle = %bundle.display(), "capture started");
        self.active = Some(Active {
            bundle,
            t0: now,
            seal,
            collector,
        });
        Ok(self.report(true, None))
    }

    fn end(
        &mut self,
        now: CaptureInstant,
        audio: &impl AudioTimeline,
        cancel: bool,
    ) -> Result<ControlReply> {
        let Some(active) = self.active.take() else {
            return Ok(self.report(false, None));
        };
        let closed = audio.close_sink(now).unwrap_or_else(|error| {
            tracing::warn!(%error, "could not close the capture sink");
            None
        });
        let seal = if cancel {
            Seal::Cancel
        } else {
            lock(&self.outcomes).finishing.push(active.bundle.clone());
            Seal::Finish {
                through: now,
                closed,
            }
        };
        let _ = active.seal.send(seal);
        self.ended.push(active.collector);
        tracing::info!(bundle = %active.bundle.display(), cancel, "capture ended");
        Ok(self.report(true, Some(active.bundle)))
    }

    fn report(&self, changed: bool, ended: Option<PathBuf>) -> ControlReply {
        let outcomes = lock(&self.outcomes);
        let (state, bundle, elapsed_ms) = match &self.active {
            Some(active) => (
                ScrawlCaptureState::Recording,
                Some(active.bundle.clone()),
                Some(CaptureInstant::now().duration_since(active.t0).as_millis() as u64),
            ),
            None => (ScrawlCaptureState::Idle, ended, None),
        };
        ControlReply::ScrawlCapture(Box::new(ScrawlCaptureReport {
            changed,
            state,
            bundle,
            elapsed_ms,
            finishing: outcomes.finishing.clone(),
            last: outcomes.last.clone(),
        }))
    }
}

/// Waits for `thread` until `deadline`, then leaves it running.
fn join_by(thread: JoinHandle<()>, deadline: Instant, name: &str) {
    while !thread.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    if thread.is_finished() {
        let _ = thread.join();
    } else {
        tracing::warn!(
            thread = name,
            "capture thread did not stop in time; leaving it"
        );
    }
}

fn log_events(events: Receiver<Event>) {
    for Event::Progress { bundle, stage } in events {
        tracing::info!(bundle = %bundle.display(), stage = stage.label(), "capture progress");
    }
}

/// The `capture-finish` thread: writes stopped captures in order, one at a
/// time, and records each outcome.
fn finish_captures(jobs: Receiver<FinishJob>, outcomes: &Mutex<Outcomes>) {
    for FinishJob { capture, audio } in jobs {
        let bundle = capture.dir().to_path_buf();
        match capture.finish(Some(audio)) {
            Ok(finished) => {
                tracing::info!(
                    bundle = %finished.dir.display(),
                    words = finished.words,
                    summary = %finished.summary,
                    "capture finished"
                );
                lock(outcomes).settle(bundle, Stage::Done, Some(finished.words));
            }
            Err(error) => {
                tracing::error!(%error, "capture failed");
                lock(outcomes).settle(bundle, Stage::Failed, None);
            }
        }
    }
}

/// One capture's audio thread: collects until the sink closes, then queues
/// the capture for the finisher or cancels it, as sealed.
fn collect(
    capture: Capture,
    chunks: Receiver<CaptureAudio>,
    sealed: Receiver<Seal>,
    jobs: &SyncSender<FinishJob>,
    outcomes: &Mutex<Outcomes>,
) {
    let mut timeline = Timeline::new(CaptureInstant::from_nanos(capture.started_at_boot_ns()));
    for chunk in chunks {
        match chunk {
            CaptureAudio::Samples {
                samples,
                sample_rate,
                captured_through,
            } => timeline.push(&samples, sample_rate, captured_through),
            CaptureAudio::Gap { from, through } => timeline.gap(from, through),
        }
    }
    let capture = match sealed.recv() {
        Ok(Seal::Finish { through, closed }) => {
            let audio = timeline.finish(through, closed);
            // Starts are refused while the queue is full, so it has room
            // unless Hex is stopping.
            match jobs.try_send(FinishJob { capture, audio }) {
                Ok(()) => return,
                Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                    tracing::warn!("no room to write the capture; discarding it");
                    job.capture
                }
            }
        }
        Ok(Seal::Cancel) | Err(_) => capture,
    };
    let bundle = capture.dir().to_path_buf();
    capture.cancel();
    lock(outcomes).settle(bundle, Stage::Cancelled, None);
}

/// A capture's audio at 16 kHz on its own clock: resampled as it arrives,
/// with missing spans padded with silence so positions stay on the clock.
struct Timeline {
    t0: CaptureInstant,
    /// Where `samples[0]` sits on the clock, once audio arrived.
    first: Option<CaptureInstant>,
    samples: Vec<f32>,
    resampler: Option<(u32, StreamingResampler)>,
    gaps: Vec<GapSpan>,
    /// The most samples kept (`MAX_CAPTURE`).
    limit: usize,
    /// Where audio past the limit started being dropped.
    truncated_at: Option<CaptureInstant>,
}

impl Timeline {
    fn new(t0: CaptureInstant) -> Self {
        Self::with_limit(t0, MAX_CAPTURE)
    }

    fn with_limit(t0: CaptureInstant, limit: Duration) -> Self {
        Self {
            t0,
            first: None,
            samples: Vec::new(),
            resampler: None,
            gaps: Vec::new(),
            limit: (limit.as_nanos() * u128::from(SAMPLE_RATE) / 1_000_000_000) as usize,
            truncated_at: None,
        }
    }

    fn push(&mut self, samples: &[f32], sample_rate: u32, through: CaptureInstant) {
        if samples.is_empty() || sample_rate == 0 {
            return;
        }
        let from = through.sub_frames(samples.len(), sample_rate);
        self.first.get_or_insert(from.max(self.t0));
        if self
            .resampler
            .as_ref()
            .is_none_or(|(rate, _)| *rate != sample_rate)
        {
            self.flush();
            match StreamingResampler::new(sample_rate) {
                Ok(resampler) => self.resampler = Some((sample_rate, resampler)),
                Err(error) => {
                    tracing::warn!(%error, sample_rate, "capture audio cannot be resampled");
                    self.gap(from, through);
                    return;
                }
            }
        }
        if let Some((_, resampler)) = &mut self.resampler {
            let resampled = resampler.push(samples);
            self.append(&resampled, through);
        }
    }

    fn gap(&mut self, from: CaptureInstant, through: CaptureInstant) {
        if through.duration_since(from) <= GAP_TOLERANCE {
            return;
        }
        let Some(first) = self.first else {
            // Before any audio: the recording simply starts later.
            return;
        };
        if self.truncated_at.is_some() {
            // Past the limit: `finish` records everything after it as one gap.
            return;
        }
        self.flush();
        let limit_at = first + duration_for(self.limit);
        self.gaps.push(GapSpan {
            start_ms: self.clock_ms(from.max(first).min(limit_at)),
            end_ms: self.clock_ms(through.min(limit_at)),
        });
        // Padding stops one sample past the limit, which marks the truncation.
        let end = position(first, through).min(self.limit + 1);
        if end > self.samples.len() {
            let padding = vec![0.0; end - self.samples.len()];
            self.append(&padding, through);
        }
    }

    fn finish(
        mut self,
        through: CaptureInstant,
        closed: Option<CaptureSinkClosed>,
    ) -> RecordedAudio {
        if let Some(closed) = closed {
            if let Some((from, until)) = closed.pending_gap {
                self.gap(from, until);
            }
            if closed.accounted_through < through {
                self.gap(closed.accounted_through.max(self.t0), through);
            }
        }
        self.flush();
        if let Some(truncated_at) = self.truncated_at {
            self.gaps.push(GapSpan {
                start_ms: self.clock_ms(truncated_at),
                end_ms: self.clock_ms(through),
            });
        }
        let Some(first) = self.first else {
            return RecordedAudio {
                sample_rate: SAMPLE_RATE,
                start_offset_ms: self.clock_ms(through),
                ..RecordedAudio::default()
            };
        };
        // Audio delivered after the stop belongs to no capture.
        self.samples.truncate(position(first, through));
        RecordedAudio {
            start_offset_ms: self.clock_ms(first),
            samples: self.samples,
            sample_rate: SAMPLE_RATE,
            gaps: self.gaps,
        }
    }

    fn flush(&mut self) {
        if let Some((_, resampler)) = self.resampler.take() {
            let rest = resampler.finish();
            let through = self
                .first
                .map(|first| first + duration_for(self.samples.len() + rest.len()))
                .unwrap_or(self.t0);
            self.append(&rest, through);
        }
    }

    fn append(&mut self, samples: &[f32], through: CaptureInstant) {
        let limit = self.limit;
        let room = limit.saturating_sub(self.samples.len());
        if samples.len() > room && self.truncated_at.is_none() {
            tracing::warn!("capture reached its length limit; dropping the rest of its audio");
            self.truncated_at = Some(
                self.first
                    .map_or(through, |first| first + duration_for(limit)),
            );
        }
        self.samples
            .extend_from_slice(&samples[..samples.len().min(room)]);
    }

    fn clock_ms(&self, at: CaptureInstant) -> u64 {
        at.duration_since(self.t0).as_millis() as u64
    }
}

fn position(first: CaptureInstant, at: CaptureInstant) -> usize {
    (at.duration_since(first).as_nanos() * u128::from(SAMPLE_RATE) / 1_000_000_000) as usize
}

fn duration_for(samples: usize) -> Duration {
    crate::audio::duration_for_frames(samples, SAMPLE_RATE)
}

fn ms_for(samples: usize) -> u64 {
    duration_for(samples).as_millis() as u64
}

/// `hex dev capture-replay`: runs a capture through [`CaptureHost`] on a
/// fresh engine with the selected model, feeding `wavs` in place of the
/// dictation timeline, and returns the finished bundle.
#[cfg(debug_assertions)]
pub fn replay(wavs: &[PathBuf]) -> Result<PathBuf> {
    let mut samples = Vec::new();
    let mut sample_rate = None;
    for wav in wavs {
        let (audio, rate) = crate::voice_filter::read_wav_mono_f32(wav)
            .map_err(|error| eyre!("{}: {error}", wav.display()))?;
        if *sample_rate.get_or_insert(rate) != rate {
            return Err(eyre!(
                "{} is not {} Hz",
                wav.display(),
                sample_rate.unwrap_or(rate)
            ));
        }
        if !samples.is_empty() {
            samples.extend(std::iter::repeat_n(0.0, rate as usize * 3 / 10));
        }
        samples.extend(audio);
    }
    let sample_rate = sample_rate.ok_or_else(|| eyre!("no WAV files"))?;
    let engine = Arc::new(TranscriptionEngine::start(Default::default())?);
    let mut host = CaptureHost::start(Some(engine.clone())).ok_or_else(|| {
        eyre!("the `capture` experiment is off; run with HEX_EXPERIMENTS=capture")
    })?;
    let timeline = WavTimeline {
        samples: Arc::new(samples),
        sample_rate,
        feeder: Mutex::new(None),
    };
    let ControlReply::ScrawlCapture(started) = host.control(ScrawlCaptureAction::Start, &timeline)
    else {
        return Err(eyre!("the capture did not start"));
    };
    let bundle = started
        .bundle
        .ok_or_else(|| eyre!("the capture has no bundle"))?;
    let ends = CaptureInstant::from_nanos(started_at(&host)?)
        + crate::audio::duration_for_frames(timeline.samples.len(), sample_rate);
    while CaptureInstant::now() < ends + Duration::from_millis(20) {
        thread::sleep(Duration::from_millis(20));
    }
    host.control(ScrawlCaptureAction::Stop, &timeline);
    for _ in 0..60_000 {
        let ControlReply::ScrawlCapture(status) =
            host.control(ScrawlCaptureAction::Status, &timeline)
        else {
            break;
        };
        if let Some(last) = status.last.filter(|last| last.bundle == bundle) {
            if last.stage != "done" {
                return Err(eyre!("the capture {}", last.stage));
            }
            return Ok(bundle);
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(eyre!("the capture never finished"))
}

#[cfg(debug_assertions)]
fn started_at(host: &CaptureHost) -> Result<u64> {
    host.active
        .as_ref()
        .map(|active| active.t0.as_nanos())
        .ok_or_else(|| eyre!("no capture is recording"))
}

/// Recorded audio standing in for the dictation timeline: the whole file is
/// delivered at once, stamped as if captured from the capture's start.
#[cfg(debug_assertions)]
struct WavTimeline {
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
    feeder: Mutex<Option<(thread::JoinHandle<()>, CaptureInstant)>>,
}

#[cfg(debug_assertions)]
impl AudioTimeline for WavTimeline {
    fn open_sink(&self, sink: SyncSender<CaptureAudio>, since: CaptureInstant) -> Result<()> {
        let samples = self.samples.clone();
        let sample_rate = self.sample_rate;
        let chunk = (sample_rate as usize / 100).max(1);
        let feeder = thread::spawn(move || {
            for (index, part) in samples.chunks(chunk).enumerate() {
                let through = since
                    + crate::audio::duration_for_frames(index * chunk + part.len(), sample_rate);
                let audio = CaptureAudio::Samples {
                    samples: part.to_vec(),
                    sample_rate,
                    captured_through: through,
                };
                if sink.send(audio).is_err() {
                    return;
                }
            }
        });
        let ends = since + crate::audio::duration_for_frames(self.samples.len(), sample_rate);
        *self
            .feeder
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some((feeder, ends));
        Ok(())
    }

    fn close_sink(&self, _through: CaptureInstant) -> Result<Option<CaptureSinkClosed>> {
        let feeder = self
            .feeder
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        Ok(feeder.map(|(feeder, ends)| {
            let _ = feeder.join();
            CaptureSinkClosed {
                pending_gap: None,
                accounted_through: ends,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> CaptureInstant {
        CaptureInstant::from_nanos(90_000_000_000 + ms * 1_000_000)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("hex-capture-{name}-{unique}"))
    }

    #[test]
    fn the_timeline_resamples_pads_gaps_and_ends_at_the_stop() {
        let mut timeline = Timeline::new(t(0));
        // Audio starts 100 ms in (a cold microphone).
        let mut through = t(100);
        for _ in 0..50 {
            through = through + Duration::from_millis(10);
            timeline.push(&[0.5; 480], 48_000, through);
        }
        // 300 ms dropped, then audio again.
        timeline.gap(through, through + Duration::from_millis(300));
        through = through + Duration::from_millis(300);
        for _ in 0..50 {
            through = through + Duration::from_millis(10);
            timeline.push(&[0.5; 160], 16_000, through);
        }
        let audio = timeline.finish(
            through.checked_sub(Duration::from_millis(100)).unwrap(),
            None,
        );
        assert_eq!(audio.sample_rate, 16_000);
        assert_eq!(audio.start_offset_ms, 100);
        assert_eq!(
            audio.gaps,
            [GapSpan {
                start_ms: 600,
                end_ms: 900
            }]
        );
        // 500 + 300 + 500 ms of audio, cut 100 ms before the last chunk.
        assert_eq!(audio.duration_ms(), 100 + 1_200);
        let resampled_silence = &audio.samples[8_000 + 100..8_000 + 4_700];
        assert!(resampled_silence.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn a_pending_gap_and_a_stalled_tail_come_back_from_the_closed_sink() {
        let mut timeline = Timeline::new(t(0));
        timeline.push(&[0.5; 1_600], 16_000, t(100));
        let audio = timeline.finish(
            t(600),
            Some(CaptureSinkClosed {
                pending_gap: Some((t(100), t(300))),
                accounted_through: t(300),
            }),
        );
        assert_eq!(
            audio.gaps,
            [
                GapSpan {
                    start_ms: 100,
                    end_ms: 300
                },
                GapSpan {
                    start_ms: 300,
                    end_ms: 600
                }
            ]
        );
        assert_eq!(audio.samples.len(), 9_600);
    }

    struct FakeTimeline {
        open: Mutex<Option<SyncSender<CaptureAudio>>>,
    }

    impl AudioTimeline for FakeTimeline {
        fn open_sink(&self, sink: SyncSender<CaptureAudio>, _since: CaptureInstant) -> Result<()> {
            let mut open = self.open.lock().unwrap();
            assert!(open.is_none(), "one sink at a time");
            *open = Some(sink);
            Ok(())
        }

        fn close_sink(&self, _through: CaptureInstant) -> Result<Option<CaptureSinkClosed>> {
            Ok(self.open.lock().unwrap().take().map(|_| CaptureSinkClosed {
                pending_gap: None,
                accounted_through: CaptureInstant::ZERO,
            }))
        }
    }

    impl FakeTimeline {
        fn feed(&self, samples: usize, through: CaptureInstant) {
            let open = self.open.lock().unwrap();
            open.as_ref()
                .expect("a sink is open")
                .send(CaptureAudio::Samples {
                    samples: vec![0.25; samples],
                    sample_rate: 16_000,
                    captured_through: through,
                })
                .unwrap();
        }
    }

    struct FakeTranscriber;

    impl Transcriber for FakeTranscriber {
        fn transcribe(&self, audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
            assert_eq!(audio.sample_rate, 16_000);
            Ok(Some(TranscriptData {
                source: "fake".into(),
                model: None,
                text: "hello there".into(),
                words: vec![
                    WordTiming {
                        t0: 0,
                        t1: 200,
                        text: "hello".into(),
                    },
                    WordTiming {
                        t0: 250,
                        t1: 500,
                        text: "there".into(),
                    },
                ],
            }))
        }
    }

    struct Captures(PathBuf);

    impl ConfigSource for Captures {
        fn engine_config(&self) -> Result<EngineConfig> {
            Ok(EngineConfig {
                captures_dir: self.0.clone(),
            })
        }
    }

    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn report(reply: ControlReply) -> ScrawlCaptureReport {
        match reply {
            ControlReply::ScrawlCapture(report) => *report,
            other => panic!("unexpected reply {other:?}"),
        }
    }

    fn wait_for_last(
        host: &mut CaptureHost,
        audio: &FakeTimeline,
        bundle: &Path,
        idle: bool,
    ) -> ScrawlCaptureOutcome {
        for _ in 0..500 {
            let status = report(host.control(ScrawlCaptureAction::Status, audio));
            if (!idle || status.finishing.is_empty())
                && let Some(last) = status.last
                && last.bundle == bundle
            {
                return last;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the capture never finished");
    }

    #[test]
    fn controls_are_idempotent_and_stop_writes_a_bundle() {
        let dir = temp_dir("control");
        let audio = FakeTimeline {
            open: Mutex::default(),
        };
        let mut host =
            CaptureHost::with(Arc::new(Captures(dir.clone())), Arc::new(FakeTranscriber)).unwrap();

        let idle = report(host.control(ScrawlCaptureAction::Stop, &audio));
        assert!(!idle.changed && idle.bundle.is_none());
        assert!(!report(host.control(ScrawlCaptureAction::Cancel, &audio)).changed);

        let started = report(host.control(ScrawlCaptureAction::Start, &audio));
        assert!(started.changed);
        assert_eq!(started.state, ScrawlCaptureState::Recording);
        let bundle = started.bundle.unwrap();
        let again = report(host.control(ScrawlCaptureAction::Start, &audio));
        assert!(!again.changed);
        assert_eq!(again.bundle.as_ref(), Some(&bundle));

        audio.feed(16_000, CaptureInstant::now() + Duration::from_millis(5));
        let stopped = report(host.control(ScrawlCaptureAction::Stop, &audio));
        assert!(stopped.changed);
        assert_eq!(stopped.state, ScrawlCaptureState::Idle);
        assert_eq!(stopped.bundle.as_ref(), Some(&bundle));
        assert!(!report(host.control(ScrawlCaptureAction::Stop, &audio)).changed);

        let last = wait_for_last(&mut host, &audio, &bundle, true);
        assert_eq!(last.stage, "done");
        assert_eq!(last.words, Some(2));
        let words: serde_json::Value =
            serde_json::from_slice(&fs::read(bundle.join("words.json")).unwrap()).unwrap();
        assert_eq!(words["version"], 1);
        assert_eq!(words["text"], "hello there");
        assert_eq!(words["words"][1]["text"], "there");
        assert!(words["words"][1]["t0"].as_u64().unwrap() >= 250);
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["sampleRate"], 16_000);
        assert_eq!(manifest["words"], "words.json");
        assert!(hound::WavReader::open(bundle.join("audio.wav")).is_ok());
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&bundle), 0o700);
        for file in ["audio.wav", "words.json", "manifest.json"] {
            assert_eq!(mode(&bundle.join(file)), 0o600, "{file}");
        }

        // Cancel discards the bundle.
        let started = report(host.control(ScrawlCaptureAction::Start, &audio));
        let cancelled_bundle = started.bundle.unwrap();
        assert!(report(host.control(ScrawlCaptureAction::Cancel, &audio)).changed);
        assert!(!report(host.control(ScrawlCaptureAction::Cancel, &audio)).changed);
        let last = wait_for_last(&mut host, &audio, &cancelled_bundle, true);
        assert_eq!(last.stage, "cancelled");
        assert!(!cancelled_bundle.exists());
        assert!(bundle.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn padding_past_the_length_limit_stays_bounded() {
        let mut timeline = Timeline::with_limit(t(0), Duration::from_secs(1));
        timeline.push(&[0.5; 1_600], 16_000, t(100));
        // A gap far past the limit pads only up to it.
        timeline.gap(t(100), t(3 * 60 * 60 * 1_000));
        assert!(timeline.samples.len() <= 16_000);
        timeline.gap(t(3 * 60 * 60 * 1_000), t(6 * 60 * 60 * 1_000));
        timeline.push(&[0.5; 1_600], 16_000, t(6 * 60 * 60 * 1_000 + 100));
        let audio = timeline.finish(t(6 * 60 * 60 * 1_000 + 100), None);
        assert_eq!(audio.samples.len(), 16_000);
        assert_eq!(
            audio.gaps,
            [
                GapSpan {
                    start_ms: 100,
                    end_ms: 1_000
                },
                GapSpan {
                    start_ms: 1_000,
                    end_ms: 6 * 60 * 60 * 1_000 + 100
                }
            ]
        );
    }

    /// Holds every transcription until the test lets it through.
    struct GatedTranscriber(Mutex<Receiver<()>>);

    impl Transcriber for GatedTranscriber {
        fn transcribe(&self, _audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
            self.0.lock().unwrap().recv().ok();
            Ok(None)
        }
    }

    #[test]
    fn captures_finish_one_at_a_time_and_starts_wait_for_room() {
        let dir = temp_dir("bounded");
        let audio = FakeTimeline {
            open: Mutex::default(),
        };
        let (gate, gated) = mpsc::channel();
        let mut host = CaptureHost::with(
            Arc::new(Captures(dir.clone())),
            Arc::new(GatedTranscriber(Mutex::new(gated))),
        )
        .unwrap();
        let mut bundles = Vec::new();
        for _ in 0..MAX_FINISHING {
            let started = report(host.control(ScrawlCaptureAction::Start, &audio));
            bundles.push(started.bundle.unwrap());
            host.control(ScrawlCaptureAction::Stop, &audio);
        }
        assert_eq!(
            report(host.control(ScrawlCaptureAction::Status, &audio)).finishing,
            bundles
        );
        let ControlReply::Error { code, .. } = host.control(ScrawlCaptureAction::Start, &audio)
        else {
            panic!("a start past the bound must be refused");
        };
        assert_eq!(code, "busy");
        // One finishes at a time, in order; the outcome is recorded directly.
        gate.send(()).unwrap();
        let first = wait_for_last(&mut host, &audio, &bundles[0], false);
        assert_eq!(first.stage, "done");
        assert_eq!(
            report(host.control(ScrawlCaptureAction::Status, &audio)).finishing,
            bundles[1..]
        );
        gate.send(()).unwrap();
        assert_eq!(
            wait_for_last(&mut host, &audio, &bundles[1], true).stage,
            "done"
        );
        assert!(report(host.control(ScrawlCaptureAction::Start, &audio)).changed);
        host.shutdown(&audio);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shutdown_cancels_the_recording_capture_and_waits_for_its_cleanup() {
        let dir = temp_dir("shutdown");
        let audio = FakeTimeline {
            open: Mutex::default(),
        };
        let mut host =
            CaptureHost::with(Arc::new(Captures(dir.clone())), Arc::new(FakeTranscriber)).unwrap();
        let bundle = report(host.control(ScrawlCaptureAction::Start, &audio))
            .bundle
            .unwrap();
        assert!(bundle.exists());
        host.shutdown(&audio);
        assert!(!bundle.exists());
        let last = report(host.control(ScrawlCaptureAction::Status, &audio))
            .last
            .unwrap();
        assert_eq!((last.bundle, last.stage.as_str()), (bundle, "cancelled"));
        let ControlReply::Error { code, .. } = host.control(ScrawlCaptureAction::Start, &audio)
        else {
            panic!("a stopped host starts nothing");
        };
        assert_eq!(code, "busy");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_host_is_absent_while_the_experiment_is_off() {
        // Nothing turns the experiment on in tests, so no host, thread, or
        // sink exists and the listener answers `feature-off`.
        if !crate::experiments::enabled(crate::experiments::Experiment::Capture) {
            assert!(CaptureHost::start(None).is_none());
        }
    }
}
