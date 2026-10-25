# Captures (experimental, debug builds only)

Scrawl's capture engine hosted inside Hex, step 3 of
[the plan](../plans/capture-in-hex.md). Today it records narration from
Hex's microphone, transcribes it with word timestamps on Hex's loaded model,
and writes a bundle. A stand-in engine takes the place of Scrawl's until its
`scrawl-capture` crate exists. No screen, input, overlay, or routing yet.

```ts
Turn it on                               // capture.gate
├── Debug build + `hex experiments enable capture` (or HEX_EXPERIMENTS=capture) -> restart
├── Release build -> `capture` is not an experiment: absent from `hex experiments list`,
│   `enable capture` is an unknown name, and a file line naming it is ignored
└── Off -> no capture host, sink, thread, or folder; `hex capture` -> feature-off

hex capture start                        // capture.start; needs a listening Hex
├── Creates <support>/captures/<unix ms>.capture -> reply: recording + bundle path
├── Copies the dictation timeline from now into a bounded sink (no second stream)
├── Release microphone while idle -> the capture keeps the microphone open
└── Already recording -> unchanged, same bundle

While it records                         // capture.audio
├── Dictation, Voice Action, and meetings work as usual on the same audio
├── Sink full (collector stalled) -> capture audio dropped as a gap, dictation untouched
└── Microphone gap or recovery -> the gap is recorded, positions stay on the clock

hex capture stop                         // capture.stop
├── Closes the sink (≤ 100 ms wait) -> reply: idle + the bundle path, at once
└── In the background: 16 kHz audio -> words on the dictation engine's model
    ├── 30 s chunks, each a foreground job: after queued dictation, before meeting chunks
    ├── audio.wav, words.json, manifest.json
    └── Transcription fails -> bundle without words.json, transcriptError in the manifest

hex capture cancel -> sink closed, bundle removed
hex capture status -> recording (bundle, elapsed), bundles still writing, last outcome
Hex quits mid-capture -> cancelled, bundle removed
```

`words.json`:

```json
{"version": 1, "source": "hex", "model": "parakeet_unified_en",
 "text": "Use scrawl to capture that feedback.",
 "words": [{"t0": 2560, "t1": 2640, "text": "Use"}, {"t0": 2720, "t1": 2960, "text": "scrawl"}]}
```

Times are milliseconds from the capture's start (`t0BootNs` in
`manifest.json`, on the boot-time clock audio and shortcuts share). The
manifest also has `durationMs`, `sampleRate`, `startOffsetMs` (when audio
began after the start), and `gaps` (`startMs`/`endMs` spans of silence where
audio was missing or dropped).

`hex dev capture-replay <wav>…` (debug) runs the same capture host on a fresh
engine with the selected model, feeding WAV files instead of the microphone,
and prints the bundle path. It needs the experiment on too.

Sources: [capture_host.rs](../../src/capture_host.rs), the sink in
[dictation_audio.rs](../../src/dictation_audio.rs),
`transcribe_with_words` in [parakeet.rs](../../src/parakeet.rs), and the
`scrawl-capture` control command in [control.rs](../../src/control.rs).

Checks:

- Gate: `capture_exists_only_in_debug_builds` in
  [experiments.rs](../../src/experiments.rs);
  `the_host_is_absent_while_the_experiment_is_off` in capture_host.rs.
- Sink, in dictation_audio.rs: `capture_sink_shares_the_timeline_with_dictation`,
  `a_full_capture_sink_drops_audio_as_a_gap_and_never_stalls_dictation`,
  `capture_sink_records_microphone_gaps`,
  `capture_sink_holds_a_released_microphone_open_until_it_closes`, and
  `a_dropped_capture_receiver_closes_the_sink`.
- Words, in parakeet.rs: `chunked_word_transcription_keeps_word_times_across_chunks`
  and `word_transcription_is_a_foreground_request_on_the_shared_engine`.
- Host, in capture_host.rs: `controls_are_idempotent_and_stop_writes_a_bundle`,
  `the_timeline_resamples_pads_gaps_and_ends_at_the_stop`, and
  `a_pending_gap_and_a_stalled_tail_come_back_from_the_closed_sink`.

Observed 2026-10-25 on pro (M2 Max), debug `HEX_TEST_BUILD=1`, with
`HEX_EXPERIMENTS=capture hex dev capture-replay` over three vocabulary
fixtures (6.5 s): Parakeet unified English loaded on Metal and the bundle had
18 words with plausible times. Proof limits: the replay feeds the capture
host directly, not through `DictationAudio`, a microphone, or a running Hex,
so the sink is proven only by the unit tests above and `hex capture` over
`POST /control` has not been run against a live app.
