# Captures In Hex

Status: exploration on `explore/scrawl-fusion`. Nothing here is merged or
shipped, and Kit decides later whether the fold happens. This is the Hex side
of ROADMAP "Fold Scrawl Into Hex" steps 2 and 3: how Hex hosts Scrawl's capture
engine without weakening dictation.

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

- `dictation_audio` gets a bounded capture sink, as on the Oct 18
  `experiment/scrawl` branch (`set_scrawl_sink`): the capture receives the same
  timestamped samples dictation sees. There is never a second `cpal` stream.
- Dictation keeps working during a capture. Both are projections of one
  timeline, so holding the dictation shortcut mid-capture still pastes, and
  the capture still records that speech as narration.
- With "Release microphone while idle", a capture holds the stream open for
  its duration, like a recording.
- Sink pressure drops capture audio and marks a gap; it never stalls the
  timeline or dictation.

## Transcription

- `capture_host` implements the engine's `Transcriber` with the transcription
  engine's loaded model and word timestamps. `parakeet` already requests
  `TimestampKind::Word` and currently discards the words; the capture path
  keeps them. The Oct 18 branch's `HexWordTranscriber` is the starting point.
- A finished capture's clip is foreground work: it runs after dictation that
  was already queued and before meeting chunks. It never loads a second model.
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
status`, `hex capture list|show`, `hex capture send <id> [--to <session>]`,
and `hex capture route <id> --explain`. Engine tools (autocrop eval, capture
simulation, reprocess) mount under `hex dev capture` in debug builds.

## Order on the branch

1. Scrawl prepares the engine on its own exploration branch (its inventory's
   refactors: `lib.rs`, pipeline out of `main.rs`, event, config, permission,
   clipboard and transcriber seams, one `Capture` type, Cargo features).
2. Import `crates/capture` here with history (`git filter-repo` plus a
   subtree merge), with the macOS-only dependency and CI.
3. `capture_host` with the experiment gate, the capture sink, Parakeet words,
   and a headless `hex capture start|stop` producing a bundle.
4. Input and Escape, then the Rune capsule and ink overlay with scenes.
5. Router in-process, the Captures pane, import, and coexistence.
6. Compare Parakeet and SpeechAnalyzer word times on recorded captures.

The Oct 18 `experiment/scrawl` branch was a copy that drifted. Reuse its
`HexWordTranscriber` and capture sink, then delete it once this branch
supersedes it.
