# Captures In Hex

Status: step 3 is built on `dev` as a debug-only experiment on Scrawl's real
engine, the `scrawl-capture` crate as a Git dependency (see
[Built so far](#built-so-far)); nothing ships, and Kit decides later whether
the fold happens. This is the Hex side of ROADMAP "Fold Scrawl Into Hex": how
Hex hosts Scrawl's capture engine without weakening dictation.

## Built so far

Step 3 on `scrawl-capture` (Scrawl's `crates/capture`, branch `engine/split`,
pinned by rev in `Cargo.toml`), without importing the crate (step 2):

```ts
hex capture start|stop|cancel|status     // debug + `--features capture`; POST /control `scrawl-capture`
└── listener thread -> capture_host::CaptureHost (exists only with the experiment on)
    ├── start -> owner-only <captures>/<unix ms>.scrawl -> reply at once
    │   ├── DictationAudio::open_capture_sink(bounded 512 chunks, since = t0)
    │   └── capture-audio thread -> Engine::start(Start { t0_boot_ns, ActiveDisplay,
    │       AudioIn::External, bundle_dir }) (blocks: target lookup, ScreenCaptureKit)
    ├── capture-audio thread -> Timeline: 16 kHz, gaps padded, start offset kept
    ├── start while the last capture is unfinished (starting, writing, cancelling) -> busy
    ├── stop -> close_capture_sink(through) (≤ 100 ms wait) -> reply with the bundle path
    │   └── capture-finish thread -> Capture::finish(Some(RecordedAudio { start_offset_ms, gaps }))
    │       ├── HexTranscriber -> parakeet::transcribe_with_words, 30 s chunks,
    │       │   each a foreground job on the dictation engine's loaded model
    │       ├── Scrawl writes recording.mp4, audio.wav, frames/, scrawl.json, summary.md
    │       └── Hex makes every file 0600 and folder 0700
    └── cancel -> Capture::cancel, remove the bundle
```

- Hex's seam is two small traits in `src/capture_host.rs`, `CaptureEngine`
  and `Recording`, implemented for `scrawl_capture::capture::{Engine, Capture}`
  so the host's tests run without a screen. Hex implements the crate's `Host`
  (bounded `try_send` events, logs and metrics to tracing), `ConfigSource`
  (`EngineConfig::default()`), and `Transcriber`.
- The engine is `Engine::new(..).with_resources(SharedResources::none())`;
  Hex never calls `install_shared_resources`, so no ingest server (47777),
  delivery retries, or global shortcuts. `capture_host::prepare_on_main_thread`
  runs on the main thread at startup, only with the experiment on, before any
  engine exists: `scrawl_capture::log::set_sink` routes the crate's
  process-wide log to tracing instead of `~/Library/Logs/Scrawl/scrawl.log`,
  and it builds the crate's keyboard-layout map, because `Engine::new` reads
  `ShortcutConfig::defaults()` (for shortcuts Hex never registers) and
  building that map calls TIS, which Hex calls only on the main thread.
  `CaptureHost::start` builds no host unless that ran.
- One capture at a time, including its finish: a start while the previous
  capture is starting, writing, or cancelling is refused `busy`. Scrawl's
  engine stops recording the screen only inside `Capture::finish`, and every
  capture shares the engine's `SharedCaptureTelemetry`, so a second capture
  during a finish would record two screens and drain the first's input.
  This narrows back to a short write queue once Scrawl splits stop from
  finish (asked below).
- Bundles go to `<support>/captures/<unix ms>.scrawl` (`hex-test` for
  `Hex Test.app`): Scrawl's own `scrawl.json` format, words in ms on the
  capture's clock.
- `hex dev capture-replay <wav>…` runs the same `CaptureHost` headlessly on a
  fresh transcription engine, feeding WAV files instead of the microphone; it
  records the screen too.
- Not built yet: the shortcut, input and Escape, overlays, routing, the pane,
  settings import, coexistence, an app-bundled helper, and the crate import.

### Features and gating

- Hex's Cargo feature `capture` (off by default) turns on the optional
  macOS-only dependencies `scrawl-capture` (`default-features = false,
  features = ["screen", "helper"]`) and `anyhow` (the crate's traits return
  `anyhow::Result`).
- `build.rs` sets `cfg(hex_capture)` only for a macOS debug build with the
  feature on, and every line of capture code is behind it. Release, Linux, Nix
  and Windows builds never compile the crate or its Swift helper. A release
  build refuses `scrawl-capture` with `unsupported`; a debug build without
  the feature with `feature-off`.
- `scripts/check` and macOS CI test with `--features capture`; clippy uses
  `--all-features`. The Nix package still vendors the Git source, so
  `nix/package.nix` carries `outputHashes."scrawl-capture-0.3.13"`; bump it
  with the rev.
- Off: `phone` (ingest server, pairing), `mic` (the engine's own cpal stream),
  `input` (rdev taps), `hex-client`, `parakeet`, `cli`. `helper` stays on for
  `scrawl-helper`, which decodes `recording.mp4` into full-quality keyframes
  at finish (without it the bundle keeps the 480 px sampled frames).

### Helper and permissions

- `scrawl-helper` is built by the crate's `build.rs` beside the binary
  (`target/debug`, three levels above `OUT_DIR`; checked in the HiveMind
  target on the mini), where the engine finds it; `$SCRAWL_HELPER` overrides.
  Missing, finish keeps the 480 px candidate frames as keyframes.
  Hex's capture path uses it only for movie decoding, which needs no TCC.
- Screen Recording belongs to the process that runs ScreenCaptureKit: Hex
  itself (or the terminal running a dev build), so no helper with a Hex
  bundle ID is needed for permissions. Packaging it into `Hex.app` waits for
  a release-capable capture; `scripts/build-app.sh` is unchanged.

## Shape

```ts
Hex.app (one process, macOS only for captures)
├─ hex (voice-control package, root of a new workspace)
│  ├─ capture_host            ← new: the experiment's one owner
│  │  ├─ Host impl            logs → tracing, metrics, own windows, clipboard
│  │  ├─ Transcriber impl     Parakeet words on the transcription engine
│  │  ├─ ConfigSource impl    settings.json `capture.*` + one-time import
│  │  └─ capsule + ink overlay (Rune overlays, storybook scenes)
│  ├─ suppression             + capture shortcut, + pointer events while capturing
│  ├─ dictation_audio         + bounded capture sink on the same timeline
│  └─ transcription_engine    + foreground capture clip (after queued dictation)
└─ crates/capture (package `scrawl-capture`, imported with history)
   ├─ features on in Hex:  screen, helper (phone later, for iPhone ingest)
   └─ features off in Hex: input, mic, hex-client, parakeet, cli
```

The engine is Scrawl's: screen sampling, keyframes, sketch and summary,
rendering, autocrop, bundles, delivery, routing (with Jev), retention,
recovery, and iPhone ingest. Hex supplies the microphone, the input, the
transcription, the interface, and the settings. The engine never opens a
microphone, installs an event tap, asks for Input Monitoring, or pastes on its
own inside Hex.

## Workspace and builds

- The root `Cargo.toml` gains `[workspace] members = [".", "crates/capture"]`.
  The root stays the `voice-control` package; the rename to `hex` is a
  separate decision.
- Hex depends on `scrawl-capture` only under
  `[target.'cfg(target_os = "macos")'.dependencies]`, with
  `default-features = false, features = ["screen", "helper"]` today (`phone`
  joins with iPhone ingest).
  Linux and Windows builds never compile it, so their CI and packages don't
  change.
- `crates/capture/Cargo.toml` lists explicit versions (no `workspace = true`)
  so the Scrawl app can depend on it by Git revision until it retires.
- macOS CI adds `cargo test -p scrawl-capture --locked`.
- `crates/capture/build.rs` compiles the engine's Swift helper. Hex builds it
  with a Hex identity and Hex usage strings (a build-script input), and
  `scripts/build-app.sh` copies and signs it into `Contents/MacOS/`.

## Experiment gate

Experiment `capture`. With it off, nothing below starts: no shortcut is
registered, no capture sink exists, no ingest port is bound, no retention or
delivery-retry worker runs, and the helper is never launched. The gate sits
where the work starts (the `capture_host` worker), not only in the window.

## Input

- The capture shortcut (default `⌥⇧S`) is a third target in `suppression`,
  beside dictation and Voice Action, with the same capture and conflict rules
  as the other bindings. It refuses bindings that overlap dictation.
- While a capture records, and only then, the session tap also reports mouse
  down, up, drag and scroll, and the Shift state. `capture_host` feeds them to
  `Capture::input()`. Scrawl's ink and click rules (Shift ink, pin and ink
  thresholds, ignoring clicks on Hex's own windows) move into the engine as
  pure functions over these events, so Hex runs no second tap.
- Escape cancels an active capture before it cancels dictation work. A
  capture never uses the hold threshold or the post-release tail.

## Microphone

- `dictation_audio` has a bounded capture sink (`open_capture_sink`,
  `close_capture_sink`): the capture receives the same timestamped samples
  dictation sees. There is never a second `cpal` stream.
- Dictation keeps working during a capture. Both are projections of one
  timeline, so holding the dictation shortcut mid-capture still pastes, and
  the capture still records that speech as narration.
- With "Release microphone while idle", a capture holds the stream open for
  its duration, like a recording.
- Sink pressure drops capture audio and marks a gap; it never stalls the
  timeline or dictation.

## Transcription

- `capture_host` implements the engine's `Transcriber` with the transcription
  engine's loaded model and word timestamps (`parakeet::transcribe_with_words`;
  dictation still discards its words).
- A finished capture's clip is foreground work: it runs after dictation that
  was already queued and before meeting chunks. It never loads a second model.
  It goes to the engine in 30 s chunks, one job each, so a dictation sent
  meanwhile waits for one chunk at most. A chunk waits while dictation
  captures or transcribes, and the engine holds one that arrives during a live
  stream until the stream ends, so it never interrupts one.
- Scrawl's spoken cues need measured word times. Until Parakeet's word times
  are compared with SpeechAnalyzer's on recorded captures, the engine's Apple
  helper path stays selectable (`hex settings set capture.transcriber apple`).

## Interface

- The capsule (recording, processing, done, route toast, picker) and the ink
  and laser overlay are click-through Rune overlays, like the meeting capsules.
  Their window ids go to the engine through `Host::own_windows` so screen
  capture leaves them out and clicks pass through.
- Each state has a `--snap` scene and a storybook entry.
- A Captures pane joins the window as a feature-owned pane (D-5): opt-in,
  shortcut, transcriber, captures folder, retention, iPhone, and "Use Hex for
  captures" while Scrawl runs.
- Screen Recording is asked for at the first capture, not in core onboarding.

## Routing and delivery

- Captures route through the engine's router in-process. The Send to session
  experiment switches from `scrawl router decide` to the same function, and
  the JSON contract in `docs/` becomes the function's types.
- Delivery uses the engine's queue and retry path. Hex never stores the
  OpenCode server password itself; the engine reads it as Scrawl does.
- `live.ndjson` gets ids, bands, scores, model, stages, and outcomes, never
  transcript, summary, or window text.

## Coexistence with Scrawl

- One owner at a time. While Scrawl (`dev.kitlangton.scrawl`) runs, Hex does
  not register the capture shortcut, bind the ingest port (47777), or run
  delivery retries. Settings shows "Use Hex for captures", which quits Scrawl
  only on that click.
- Configuration: a one-time, read-only import of Scrawl's settings into
  `settings.json` `capture.*` keys, each with a `hex settings` key. The ingest
  token moves to an owner-only file in Hex's support folder so paired phones
  keep working.
- Captures stay in Scrawl's captures folder; Hex reads and cleans them there.

## Invariants to add

- Capture never opens a microphone, installs an event tap, or loads a model
  of its own, and never blocks or drops dictation audio.
- Capture queues are bounded: the capture sink, the engine's accessibility
  and retention queues (unbounded in Scrawl today; bound them before import),
  and the frame candidate buffer (lower than Scrawl's 384 MB).
- One engine per process (the engine keeps process-global state).
- The experiment off means no capture work at all.

## CLI

Through `POST /control`, like everything else: `hex capture start|stop|cancel|
status` (built, debug only), `hex capture list|show`, `hex capture send <id> [--to <session>]`,
and `hex capture route <id> --explain`. Engine tools (autocrop eval, capture
simulation, reprocess) mount under `hex dev capture` in debug builds.

## Order on the branch

1. Scrawl prepares the engine on its own exploration branch (its inventory's
   refactors: `lib.rs`, pipeline out of `main.rs`, event, config, permission,
   clipboard and transcriber seams, one `Capture` type, Cargo features).
2. Import `crates/capture` here with history (`git filter-repo` plus a
   subtree merge), with the macOS-only dependency and CI.
3. `capture_host` with the experiment gate, the capture sink, Parakeet words,
   and a headless `hex capture start|stop` producing a bundle. Built ahead of
   step 2, on `scrawl-capture` as a Git dependency.
4. Input and Escape, then the Rune capsule and ink overlay with scenes.
5. Router in-process, the Captures pane, import, and coexistence.
6. Compare Parakeet and SpeechAnalyzer word times on recorded captures.

The Oct 18 `experiment/scrawl` branch was a copy that drifted; `feat/capture`
supersedes its capture sink and word transcriber, so it can be deleted.

## Asks for Scrawl's API

Answered by `engine/split`: `Start::t0_boot_ns`, `RecordedAudio` with
`start_offset_ms` and `gaps`, words relative to the first sample, and
`SharedResources::none()` by default.

Left from hosting the real engine:

- The engine still reads `~/.config/scrawl/config.json` through the global
  `handoff::engine_config()`, bypassing `ConfigSource`: autocrop settings
  (Display captures), `sketch::summary_markdown`, and the agent-context
  experiment check. Reads only, but Hex's settings should be the only source.
- `scrawl_log` writes to Scrawl's log file unless a host installs a sink;
  make the sink part of `Engine` (or default to none in a library).
- Recording the screen stops only inside `Capture::finish`, after the
  transcriber, so a stopped capture holds ScreenCaptureKit until its words are
  done. Split stop (end the stream, return the recording) from finish (write
  the bundle), so Hex can queue writes and start the next capture.
- `SharedCaptureTelemetry` is one per engine and drained by each capture;
  overlapping captures steal each other's input. Make it per capture.
- `Engine::new` calls `ShortcutConfig::defaults()`, which builds the
  keyboard-layout map through TIS on whatever thread constructs the engine.
  A host with `SharedResources::none()` needs no shortcuts; build them only
  with the shortcut resource.
- No audio-only target: every capture records the screen and needs Screen
  Recording. Hex's narration-only tests use a fake engine.
- `Engine::start` blocks on target discovery and ScreenCaptureKit; Hex runs it
  off the listener thread. Say so in its docs, or split it.
- `finish` launches `scrawl-helper` to decode the movie; a library host has
  to ship that executable or lose full-quality keyframes.
- Bundle files are written with the process umask (0644); Hex tightens them
  after `finish`. A `0600` writer would keep them private while recording too.
- `phone`, `routing`, `delivery`, and the ingest HTTP code compile
  regardless of features, and the crate pulls a second `windows` (0.61) into
  `Cargo.lock` for its Windows target.
- `anyhow` in the public traits makes a `color_eyre` host convert every error.
- A transcriber error is only logged; `Finished` can't tell Hex that words
  are missing, or how many gaps the audio had.
