# Captures In Hex

Status: step 3 is built on `feat/capture` as a debug-only experiment with a
stand-in engine (see [Built so far](#built-so-far)); nothing ships, and Kit
decides later whether the fold happens. This is the Hex side of ROADMAP "Fold
Scrawl Into Hex": how Hex hosts Scrawl's capture engine without weakening
dictation.

## Built so far

Step 3, without the `scrawl-capture` crate (Scrawl's `engine/split` branch is
still making it):

```ts
hex capture start|stop|cancel|status     // debug builds; POST /control `scrawl-capture`
└── listener thread -> capture_host::CaptureHost (exists only with the experiment on)
    ├── start -> Engine::start(Start { t0_boot_ns }) -> bundle folder, returns at once
    │   └── DictationAudio::open_capture_sink(bounded 512 chunks, since = t0)
    ├── capture-audio thread -> Timeline: 16 kHz, gaps padded with silence
    ├── stop -> close_capture_sink(through) (≤ 100 ms wait) -> reply with the bundle path
    │   └── capture-audio thread -> Capture::finish(Some(RecordedAudio))
    │       ├── HexTranscriber -> parakeet::transcribe_with_words, 30 s chunks,
    │       │   each a foreground job on the dictation engine's loaded model
    │       └── audio.wav + words.json + manifest.json
    └── cancel -> close the sink, remove the bundle
```

- The seam in `src/capture_host.rs` mirrors the inventory's API sketch:
  `Host` (non-blocking `emit`/`log`), `Transcriber` (takes `RecordedAudio`:
  16 kHz samples, `sample_rate`, `start_offset_ms`, `gaps`), `ConfigSource`,
  `Engine::start(Start { t0_boot_ns, .. }) -> Capture`,
  `Capture::started_at_boot_ns()`, `Capture::input() -> InputSink`, and
  `Capture::finish(Option<RecordedAudio>)`. `Engine` is a marked stand-in
  until `scrawl_capture::Engine` replaces it.
- Bundles go to `<support>/captures/<unix ms>.capture` (`hex-test` for
  `Hex Test.app`). Words are `{t0, t1, text}` in ms from the capture's start.
- `hex dev capture-replay <wav>…` runs the same `CaptureHost` headlessly on a
  fresh engine, feeding WAV files instead of the microphone.
- Not built yet: the shortcut, input and Escape, overlays, routing, the pane,
  settings import, coexistence, and the crate import itself.

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
   ├─ features on in Hex:  screen, helper, phone
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
  `default-features = false, features = ["screen", "helper", "phone"]`.
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
   step 2 with a stand-in engine.
4. Input and Escape, then the Rune capsule and ink overlay with scenes.
5. Router in-process, the Captures pane, import, and coexistence.
6. Compare Parakeet and SpeechAnalyzer word times on recorded captures.

The Oct 18 `experiment/scrawl` branch was a copy that drifted; `feat/capture`
supersedes its capture sink and word transcriber, so it can be deleted.

## Asks for Scrawl's API

From building the seam:

- `Start` needs `t0_boot_ns`: Hex's audio and input share the boot-time
  nanosecond clock, and the engine should not read its own.
- `RecordedAudio` should carry `start_offset_ms` and `gaps`. A cold or
  recovering microphone starts late, and sink pressure drops audio; the
  engine must not assume `samples[0]` is t0 or that audio is continuous.
- `Transcriber::transcribe(&RecordedAudio)` should return words relative to
  the first sample and let the engine add `start_offset_ms`, so a host never
  thinks about the capture clock.
- `Capture::finish` blocks (transcription, writing); say so in its docs, and
  keep `start`, `cancel`, `emit`, and `log` non-blocking.
- `Host::emit` should accept drops: Hex sends events through a bounded
  channel and drops when full rather than wait.
