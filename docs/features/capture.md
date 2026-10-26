# Captures (experimental, a build option)

Scrawl's capture engine hosted inside Hex, steps 3 to 5 of
[the plan](../plans/capture-in-hex.md). It runs the real
`scrawl-capture` crate, imported into `crates/capture`: it records the active
display with ScreenCaptureKit (or a cabled iPhone through `scrawl-helper`'s
mirror) and narration from Hex's microphone, takes Shift ink and clicks from
Hex's event tap, transcribes the narration with word timestamps on Hex's
loaded model, writes a Scrawl bundle into Scrawl's captures folder, copies it
to the clipboard, and sends it to an OpenCode session with Scrawl's router
(`crates/router`), run in Hex's process. Settings › Experiments › Scrawl turns
it on and holds its settings. Scrawl's capsule (switch chips, the on-screen
picker, route chips) and its ink and laser overlay are Rune overlays.

```ts
Turn it on                               // capture.gate
├── macOS build with `--features capture`, debug or release; off in every public build
│   ├── `HEX_CAPTURE=1 scripts/build-app.sh` -> Hex.app with scrawl-helper signed
│   │   beside `hex`; scripts/check and macOS CI test with the feature
│   └── + `hex experiments enable capture` (or HEX_EXPERIMENTS=capture) -> restart
├── Any build without the feature (every public release, Linux, Windows, Nix) ->
│   `capture` is not an experiment: absent from `hex experiments list`, `enable
│   capture` is an unknown name, and a file line naming it is ignored; `hex capture`
│   -> unsupported. The crate is not compiled at all
├── Release path -> release-app.sh refuses HEX_CAPTURE; validate-app.sh refuses a
│   bundle with scrawl-helper or the capture engine in `hex`
├── Experiment on -> Settings › Experiments › Scrawl appears
└── Off -> no capture host, engine, sink, thread, or folder; `hex capture` -> feature-off

Settings › Experiments › Scrawl          // capture.pane
├── Capture with Hex (capture.enabled, default off) -> off: no host, shortcut,
│   input, retention, or folder, as with the experiment off
│   └── On while the shortcut collides -> refused, the shortcut row says why
├── Shortcut (capture.hotkey, default ⌥⇧S) -> same keycap and recorder as dictation;
│   a key is required; refused when it overlaps dictation, Voice Action, or paste
├── Screen Recording -> Allow asks macOS once, then opens its System Settings list;
│   never part of onboarding (macOS also asks at the first capture)
├── Transcriber (capture.transcriber) -> Parakeet (default) | Apple
├── Captures folder (capture.folder, default Scrawl's) -> Reveal | choose another
├── Keep (capture.media_days / bundle_days) -> Scrawl's presets, or imported days
├── Scrawl.app running -> "Scrawl is running" card + Use Hex for captures
│   └── Click -> asks Scrawl to quit (nothing else quits it); Hex takes the
│       shortcut within 2 s of Scrawl leaving
└── No iPhone pairing row: ScrawlKit pairing needs the ingest server, which Hex
    doesn't run. A cabled iPhone needs no row (capture.phone)

First launch with the experiment on      // capture.import
├── ~/.config/scrawl/config.json -> transcriber (apple; else Parakeet), capturesDir,
│   retention, read once (migration `scrawl-capture-settings`); token not used
├── Scrawl's router settings -> capture.routing, read once (migration
│   `scrawl-routing-settings`): rules, autoSend, smartRouting and jevRouting;
│   never the Jev key (Jev uses $OPENCODE_API_KEY or OpenCode's credential)
├── Scrawl's Application Support config.json -> its shortcut, unless it collides
└── Scrawl's files are never written

⌥⇧S (capture.shortcut; captures on, Scrawl not running)
├── Press, held as long as you like -> armed; nothing records while ⌥⇧ is down
│   └── One of its modifiers comes up -> start on the display under the pointer,
│       as `hex capture start`                // a fresh press only, no repeats
├── Esc before the modifiers come up -> nothing records
├── Press while recording -> stop, as `hex capture stop` // ignored within 400 ms
├── Press while the capsule's picker is open -> the picker records what it is on
├── Esc while a capture records -> cancel it, before any dictation job
├── The binding is swallowed, never chord evidence for dictation
└── Hold the dictation shortcut mid-capture -> dictation pastes as usual

Switching target                         // capture.switch; the capsule's chips
├── Started by the shortcut -> the snapshot's recording says `target: Pointer`,
│   the only kind that shows the switch chips (Area · Window · Screen)
├── First click, scroll, or ink stroke -> `worked`, so the chips fold
├── Screen -> the capsule folds the chips; the capture goes on
├── CaptureCommand::Switch(target) -> discard and start in one step: no snapshot
│   shows nothing recording in between (Scrawl's switch_recording)
│   ├── No room to start (two writing, Hex stopping) -> refused `busy` before
│   │   the discard; the recording goes on
│   └── The start fails after the discard -> the discarded capture settles
│       `Failed` with the start's error
├── Window or Area -> CaptureCommand::Discard: bundle removed, no outcome, gone
│   from the snapshot at once (and from MAX_FINISHING), so the capsule becomes
│   the picker
├── Picker open (take_keys: Picker) -> Hex's tap hands it ⌥⇧S and Esc; it never
│   takes focus
├── A pick -> CaptureCommand::Start(Window | Area | Display); nothing while one
│   records
├── The picked window closed, or Screen Recording refused -> last outcome
│   `failed` with its reason (`error`)
└── iPhone -> CaptureCommand::Switch(Phone) or Start(Phone), while the snapshot
    names a cabled phone (capture.phone)

A cabled iPhone                          // capture.phone
├── Each capture start, and CaptureCommand::FindPhone (the picker opening) ->
│   a capture-phone thread runs `scrawl-helper devices` (the USB registry, no
│   permission) -> CaptureSnapshot.phone = its name, or None
├── Start(Phone) -> scrawl-helper's mirror window shows the phone and records
│   its native video into recording.mov; `--no-microphone`: the mirror never
│   asks for or opens a microphone
│   ├── Camera access -> macOS asks on Hex's behalf (HEX_CAPTURE builds carry
│   │   NSCameraUsageDescription; validate-app.sh refuses it)
│   └── Pointer and pen input -> normalized to the mirror's picture
├── Narration -> Hex's timeline, as for the screen, moved onto the movie's clock
│   (its first frame, measured from the capture's start) and transcribed by Hex's
│   transcriber (capture.transcriber), not Scrawl's
├── Stop -> as any capture; keyframes decoded from the phone's own video
└── The mirror ends on its own (unplugged, no phone in 12 s, camera denied) ->
    the listener finishes the capture: written if it recorded, else `failed`
    with the helper's reason

The capsule                              // capture.capsule (rune_ui/capture_capsule)
├── A Rune overlay at the top of the pointer's display that never takes focus
│   and lets clicks through except on its live chips, buttons, and list; a
│   new recording brings it to the pointer's display even while it's up on
│   another;
│   its window id goes to the engine (`Host::own_windows`, one id per role, so a
│   recreated window replaces the old one), so recordings leave it out
├── Follows `capture_host::watch()`: Recording from the instant `recording` is
│   Some (the bundle's t0; timer, meter, ✓ finish, ✕ cancel) -> Processing ->
│   Copied, Failed, or Cancelled, then leaves on its own; a written capture
│   whose transcript has no words reads "Copied", "no speech heard"
├── Tones (`feedback::Tone::Capture*`, at the dictation volume; zero is
│   silent): start, stop, a rule's send going, an Undo, a failure
├── Failed -> "Couldn't start the capture" until the engine reports it
│   recording (`screen`), then "Couldn't save the capture"; a failed send ->
│   "Couldn't send to X". Each keeps its `error`, or "Screen Recording is off"
│   when it isn't granted; a start that fails before any snapshot showed it
│   recording still shows, but never over a newer recording, and a capture
│   already under way that fails in the background never takes it over
├── A shown recording that leaves with no outcome (a discard, from a chip or
│   `hex capture discard`) -> the capsule leaves; the picker a chip opened
│   stays. A switch that can't start settles the recording it discarded as
│   failed, which the capsule shows as "Couldn't start the capture"
├── A shortcut start (`target: Pointer`) -> Area · Window · Screen chips, plus
│   iPhone while the snapshot names a cabled phone, for 3.5 s
│   ├── `worked` -> they fold once seen for 0.9 s; never while the pointer is on
│   │   the capsule, and 1.2 s after it leaves
│   ├── A chip restarts rather than retargets (Scrawl's `switch_recording`)
│   ├── Screen -> it's what's recording: the chips fold, no command
│   ├── iPhone -> CaptureCommand::Switch(Phone): the recording restarts on the
│   │   phone
│   └── Window or Area -> Discard; the capsule becomes the picker, and
│       FindPhone looks for a phone cabled since the recording began
├── Picker: ⌥⇧S records what the on-screen picker shows; Esc closes it
├── ✓ -> `hex capture stop`; ✕ -> `hex capture cancel`
├── Routing (Scrawl's routing.md) -> `last.route` once its answer arrives; the
│   capsule runs a rule's undo window and a chip's send itself, and follows
│   only Queued (then Sent), how an Undo ended, and a failed send
│   ├── A rule's send -> "Auto-sending to X" (a pick: "Sending to X") with the
│   │   seconds left in a ring, a filled Undo chip (esc) that drains over the
│   │   5 s window (`routing::UNDO`; static under Reduce Motion), a quiet
│   │   Paste, and the runner-up as a "Send to Y" chip that redirects at once;
│   │   then "Sent to X" with an "Elsewhere…" chip for the list
│   ├── An ask -> "Copied" and up to four chips, best first ("Send to A", B, C, D);
│   │   a chip sends at once
│   ├── Chips -> CaptureCommand::Send{to}, Undo, Redirect{to}, naming the
│   │   session by id (`RouteDest.session`), so sessions sharing a title never
│   │   collide; the capsule shows titles
│   ├── Keys (capture_host::take_keys), never taking focus:
│   │   ├── Picker -> ⌥⇧S and Esc through the listener
│   │   ├── Routed -> the Paste Last shortcut (⌥⇧V by default; the chip shows
│   │   │   the saved one, and no keys while it's off) opens the list instead
│   │   │   of pasting the last dictation; Esc too while a send can be undone.
│   │   │   Unreachable has no list, so Paste Last pastes
│   │   └── List open -> the session tap takes every key press without ⌘ or ⌃
│   │       that isn't a Hex shortcut: text in the active layout, ⌫, ↑↓, ⏎,
│   │       ⇧⏎ (always), 1-9, Esc; their releases too. A send, Esc, or a
│   │       press off the capsule (the tap reports mouse-downs while the list
│   │       is open, and still delivers them) closes the list and gives the
│   │       keys back; hiding drops the grab
│   ├── `offer` (a rule offer) -> not drawn; Scrawl's capsule doesn't draw it
│   ├── Nothing fits, or no answer -> "Send to…" opens the list
│   ├── Every written capture -> Paste (the one accent action) and Inspect, as
│   │   Scrawl's capsule has
│   │   ├── Paste -> CaptureCommand::Paste: the contact sheet, then the summary,
│   │   │   into the app in front; a send still in its Undo window is taken back
│   │   └── Inspect -> CaptureCommand::Inspect: opens the contact sheet (Hex has
│   │       no review window, and opening the bundle could launch Scrawl.app)
│   └── Long names give way together so the widest row stays inside the panel;
│       before a name is cut past reading, the last route chip goes (the list
│       still has it)
└── Reduce Motion -> travel jumps, nothing blurs; opacity still fades

The on-screen picker                     // capture.picker (capture_capsule/picker.rs)
├── A full-screen Rune overlay on the pointer's display, under the capsule,
│   left out of recordings (`OwnWindow::Picker`); it takes the pointer only while
│   picking and never takes focus; it follows the pointer to another display
├── Everything but the target dims; the dim moves with the target as one piece
├── Window -> the window under the pointer: tinted, ringed, its title in a badge;
│   the cursor becomes Scrawl's camera; no window under it -> the display
├── Area -> drag at least 36 pt; the badge shows its size; a click alone -> the
│   display
├── Screen -> the display, ringed
│   └── An area's ring is white with a dark hairline outside it, so it reads
│       over white content too (Scrawl's vanishes there)
├── Click or ⌥⇧S -> Action::Start(target) -> CaptureCommand::Start; the picker
│   fades; a pick that starts no capture within 2 s brings it back
└── Windows load off the UI thread (`list_selectable_windows`); nothing picks
    until they arrive

Ink and laser                            // capture.ink
├── A click-through overlay over the recorded display, excluded from the recording
├── Shift strokes draw as ink, hold 650 ms, then fade over 650 ms
└── The laser's tail lasts 420 ms; nothing lags the pointer

While a capture records                  // capture.input
├── The existing session tap adds mouse down/up/drag/scroll and Shift (no second tap)
├── -> Capture::input() through a bounded queue; full -> events dropped and counted
├── Keys pressed in the app -> logged; keys Hex consumes (its shortcuts, Esc) and
│   anything while a shortcut is being recorded -> not logged
├── Scrawl's rules in the engine (hosted_input): Shift drag inks, a click pins,
│   clicks on Hex's own windows ignored
└── Shift still held from ⌥⇧S -> must come up once before it can ink; Shift up
    at the start -> the first Shift drag inks

hex capture start                        // capture.start; needs a listening Hex
├── Creates <captures folder>/<unix ms>.scrawl (owner-only) -> reply: recording + bundle path
├── Two stopped captures still writing -> refused `busy`
├── Copies the dictation timeline from now into a bounded sink (no second stream)
├── In the background: Scrawl's engine starts on the active display
│   ├── Needs Screen Recording for Hex (or the terminal running it)
│   └── Fails to start -> sink closed, bundle removed, last outcome `failed`;
│       Esc and the next ⌥⇧S see nothing recording
├── Release microphone while idle -> the capture keeps the microphone open
└── Already recording -> unchanged, same bundle

While it records                         // capture.audio
├── Dictation, Voice Action, and meetings work as usual on the same audio
├── Sink full (collector stalled) -> capture audio dropped as a gap, dictation untouched
└── Microphone gap or recovery -> the gap is recorded, positions stay on the clock

Never on Hex's path                      // capture.isolation
├── No microphone, event tap, global shortcut, or model of its own
├── No ingest server (47777) or recovery workers; delivery retries only while
│   neither Scrawl.app runs nor anything serves the ingest port (capture.route)
├── No ingest token used; Scrawl's two config files read once by the import, never
│   written; nothing else of Scrawl's touched (its log goes to Hex's tracing)
└── Settings from Hex (capture.*), not Scrawl's config: auto-crop on (display
    captures), the agentContext summary format off

Retention                                // capture.retention
└── Hex's own thread: Scrawl's retention pass with capture.*'s policy, 90 s after
    captures turn on, then hourly; skipped while a capture records or writes,
    or while Scrawl.app runs (it cleans the shared folder then)

hex capture stop                         // capture.stop
├── Closes the sink (≤ 100 ms wait) -> reply: idle + the bundle path, at once
├── The screen recording, window log, and input stop at once (Capture::stop)
└── In the background, on one capture-finish thread, one capture at a time:
    ├── 16 kHz audio with its start offset and gaps -> SealedCapture::finish
    ├── Words: 30 s chunks, each a foreground job on the dictation engine's model,
    │   after queued dictation, before meeting chunks; waits while dictation records
    │   or transcribes; never interrupts a live stream
    ├── Word times moved earlier by the model's measured lag (Parakeet Unified
    │   English: starts −280 ms, −520 ms after a ≥ 250 ms pause; ends −195 ms);
    │   other models and Apple keep their own times
    ├── Past 20 min -> audio kept to 20 min, the rest one gap
    ├── recording.mp4, audio.wav, frames/, scrawl.json, summary.md; then every
    │   file 0600 and folder 0700
    ├── Keyframes decoded from recording.mp4 by scrawl-helper (beside `hex`: in
    │   Hex.app's Contents/MacOS or target/<profile>; or $SCRAWL_HELPER); helper
    │   missing -> the 480 px sampled frames
    ├── Transcription fails -> logged; the bundle has no words, and the outcome says why
    ├── Written -> image and text copied to the clipboard (Scrawl's handoff)
    └── Sent -> the capture-route thread (capture.route)

Where it goes                            // capture.route; capture.routing's autoSend on
├── At the stop, beside the writing -> a capture-look thread reads the tab,
│   terminal, and sessions in front (Scrawl's router, in Hex's process, with
│   Scrawl's own router folders, so `scrawl router undo` and its evals see Hex's
│   sends as Scrawl's)
├── Written -> the one capture-route thread (queue 16) decides; the snapshot's
│   `last.route` follows: Looking -> Rule | Guess | Nothing | NoAnswer |
│   Unreachable
│   ├── Rule -> only a rule the person saved (`routing.sites`, `routing.apps`,
│   │   an "Always send … here"): Pending, sent after 5 s unless Undo; then
│   │   Sending -> Sent, or Queued when OpenCode didn't take it. Whatever
│   │   autoSend says, nothing else sends on its own
│   ├── Guess -> everything else the router would send (Jev's pick, HOME when
│   │   Jev couldn't tell, a dev server's port, a page's data-session or
│   │   publisher, a name that was said, the port ladder) is suggestion #1, its
│   │   reason as the row's word ("Jev's pick", "dev server", "this page"), then
│   │   the runners-up and recent sessions; nothing sent until one is picked.
│   │   `site` names what "Always send <site> here" (⇧⏎ in the list, or
│   │   `send --always`) would save, after which that site's captures are Rules
│   ├── Each Rule or Guess -> routing.json's `shown`: `as` rule or guess, and
│   │   `why`, the router's leading reason; `hex capture route` prints it
│   ├── Nothing / NoAnswer -> recent sessions only
│   └── Unreachable -> the look read no sessions (OpenCode slow past the look's
│       10 s, down, or refusing); a warning in process.log with the error, and
│       the capsule reads "Couldn't reach OpenCode" with why ("didn't answer in
│       time"), keeping Paste and Inspect
├── The capsule's commands act only on `last`, the newest routed capture:
│   ├── Send{to} -> the Rule's own session while Pending sends now; any other
│   │   pick sends there (taking back a send already elsewhere first) and logs
│   │   the correction; `always` also saves a rule for its site or app
│   ├── Undo -> a Pending send never goes; a sent one is withdrawn unless the
│   │   session acted on it (TooLate)
│   └── Redirect{to} -> taken back, then sent to `to`
├── Two sends to one place agree -> `offer` names the site or app a rule could
│   send straight there
├── Queued sends -> retried every 15 s, unless Scrawl.app runs or something
│   serves the ingest port (that process retries); a retried send can't be undone
└── autoSend off -> no look, no route; the capture is only copied

hex capture list|show|send|route         // capture.cli; docs/cli.md
├── list [--limit N] -> the newest bundles in the captures folder: mode, words,
│   where it went
├── show <capture> -> one capture's words, files, and send
├── send <capture> [--to <session>] [--always] -> needs a listening Hex
│   ├── The newest routed capture -> as the capsule's pick (no --to: the Rule's
│   │   session, a Guess's first suggestion, or the recorded destination), so
│   │   the capsule's Undo follows it; --always also saves the site or app rule
│   └── Older -> --to must be a session ID; without it, its recorded
│       destination; --always is refused
├── route <capture> [--ask-jev] -> what the router sees and decides; sends nothing
└── <capture> = bundle name, path, or `latest`; unknown -> `not-found`

hex capture cancel -> engine cancelled, sink closed, bundle removed
hex capture status -> recording (bundle, elapsed), bundles still writing, last outcome
                      (words, audio gaps, or why it has no transcript)
Hex quits mid-capture -> cancelled, bundle removed; captures still writing stop
transcribing between chunks and keep their audio (shutdown waits ≤ 2 s)
```

`scrawl.json` is Scrawl's bundle manifest. Its `transcript.words` are
`{t0, t1, text}` in milliseconds from the capture's start (`t0_boot_ns`, the
boot-time clock audio and shortcuts share), and `audio.wav` starts at that
start too, padded with silence when the microphone began late or dropped
audio.

`hex dev capture-replay <wav>…` (debug, with the feature) runs the same
capture host on a fresh transcription engine with the selected model, feeding
WAV files instead of the microphone, and prints the bundle path. It needs the
experiment on too, and records the screen meanwhile.

Sources: [capture_host.rs](../../src/capture_host.rs), the tap in
[suppression.rs](../../src/suppression.rs), the shortcut and Escape in
[recognition.rs](../../src/recognition.rs), `CaptureSettings` and the import in
[app_settings.rs](../../src/app_settings.rs), the pane in
[rune_ui/scrawl.rs](../../src/rune_ui/scrawl.rs), the input rules in
[hosted_input.rs](../../crates/capture/src/hosted_input.rs), the sink in
[dictation_audio.rs](../../src/dictation_audio.rs),
`transcribe_with_words` in [parakeet.rs](../../src/parakeet.rs), routing in
[capture_host/routing.rs](../../src/capture_host/routing.rs), the CLI's reads in
[capture_host/library.rs](../../src/capture_host/library.rs), the phone finish
in [phone.rs](../../crates/capture/src/phone.rs), and the `scrawl-capture`
control commands in [control.rs](../../src/control.rs).

Checks:

- Gate: `capture_exists_only_with_its_build_option` in
  [experiments.rs](../../src/experiments.rs);
  `the_host_is_absent_while_the_experiment_is_off` in capture_host.rs;
  [test-app-identity.sh](../../scripts/test-app-identity.sh) for the
  release-path refusals.
- Sink, in dictation_audio.rs: `capture_sink_shares_the_timeline_with_dictation`,
  `a_full_capture_sink_drops_audio_as_a_gap_and_never_stalls_dictation`,
  `capture_sink_records_microphone_gaps`,
  `capture_sink_holds_a_released_microphone_open_until_it_closes`,
  `capture_sink_keeps_every_frame_through_timestamp_jitter`, and
  `a_dropped_capture_receiver_closes_the_sink`.
- Words, in parakeet.rs: `chunked_word_transcription_keeps_word_times_across_chunks`
  and `word_transcription_is_a_foreground_request_on_the_shared_engine`;
  `foreground_work_waits_for_an_open_stream_instead_of_interrupting_it` in
  [transcription_engine.rs](../../src/transcription_engine.rs).
- Host, in capture_host.rs (a fake engine that writes through Scrawl's real
  `finalize_capture_bundle_with_flags`, without a screen):
  `controls_are_idempotent_and_stop_writes_a_scrawl_bundle`,
  `an_engine_that_cannot_start_leaves_nothing_behind`,
  `scrawls_engine_claims_nothing_machine_wide`,
  `the_timeline_resamples_pads_gaps_and_ends_at_the_stop`,
  `a_pending_gap_and_a_stalled_tail_come_back_from_the_closed_sink`,
  `padding_past_the_length_limit_stays_bounded`,
  `captures_finish_one_at_a_time_and_starts_wait_for_room`,
  `a_capture_queued_behind_a_slow_finish_has_already_stopped`,
  `a_failed_start_raced_by_stop_never_wedges_later_starts`,
  `the_engine_config_comes_from_hex`, and
  `shutdown_cancels_the_recording_capture_and_waits_for_its_cleanup`.
- Shortcut and input, in capture_host.rs: `the_host_is_absent_while_captures_are_off`,
  `the_shortcut_starts_and_a_later_press_stops`,
  `input_reaches_only_the_capture_that_is_recording`, `tap_flags_become_modifiers`,
  `the_shortcut_records_once_its_keys_come_up`,
  `a_failed_start_ends_before_escape_or_the_shortcut_sees_it`;
  `the_capture_shortcut_is_suppressed_and_never_chord_evidence` and
  `a_capture_logs_only_keys_that_reach_the_app` in suppression.rs;
  the ink and click rules in hosted_input.rs, including
  `a_capture_started_by_its_shortcut_waits_for_shift_to_come_up`,
  `a_capture_started_without_shift_held_inks_from_the_first_stroke`, and
  `a_click_with_the_chords_shift_still_held_is_no_pin`.
- iPhone: `an_iphone_capture_narrates_with_hex_audio_and_finishes_when_its_mirror_ends`
  and `a_mirror_ending_never_stops_a_screen_capture` in capture_host.rs;
  `since_moves_audio_and_gaps_onto_a_later_clock` in
  [audio.rs](../../crates/capture/src/audio.rs). The mirror, its
  `--no-microphone`, and the camera prompt have not run in Hex.
- Routing, in [routing.rs](../../src/capture_host/routing.rs):
  `only_a_saved_rule_sends_on_its_own`,
  `a_rule_sends_after_undo_and_lists_the_rest`,
  `a_guess_sends_nothing_and_a_failure_still_lists_recent_sessions`, and
  `the_ladder_is_a_guess`; `an_always_rule_makes_its_sites_next_capture_a_saved_rule`
  in [look.rs](../../crates/router/src/routing/look.rs); the CLI's
  `a_capture_on_disk_summarizes_its_mode_words_and_send` in
  [library.rs](../../src/capture_host/library.rs), the protocol round trips in
  control.rs, and `control_commands_parse` in main.rs.
- Switching, in capture_host.rs:
  `a_discarded_capture_leaves_no_trace_and_a_pick_takes_its_place`,
  `a_switch_never_shows_nothing_recording`,
  `a_switch_with_no_room_to_start_keeps_the_recording`,
  `a_switch_that_cannot_start_after_its_discard_settles_failed`,
  `discarded_captures_never_count_against_the_writers`,
  `a_pick_that_cannot_start_says_why`,
  `the_first_click_or_scroll_means_work_started`, and
  `the_newest_grab_takes_only_its_keys_until_it_drops`. Paste and Inspect
  need a written capture
  (`controls_are_idempotent_and_stop_writes_a_scrawl_bundle`); neither has
  pasted into or opened anything in a test.
- Capsule, in rune_ui/capture_capsule (feature builds only): the flow tests
  (`chips_fold_once_seen_when_work_starts_but_never_under_the_pointer`,
  `the_shortcut_records_the_pickers_screen_and_a_chip_discards_the_recording`,
  …), feed tests
  (`the_shortcuts_recording_shows_its_chips_until_work_starts`,
  `a_rule_shows_once_and_only_news_the_capsule_cant_know_moves_it`,
  `a_queued_send_says_so_and_says_sent_when_it_goes`,
  `a_cabled_phone_shows_its_chip`, …),
  `paste_takes_back_a_waiting_send_and_the_capsule_stays_gone`,
  `the_capsule_takes_only_the_keys_its_state_needs`,
  `the_hints_keys_stay_inside_their_trays`, goldens
  `tests/golden/capture-capsule-*.png`, and
  `the_capsule_moves_inside_its_panel_and_comes_to_rest`, which traces every
  scripted state change per frame, Calm and Reduce Motion, for no pops,
  jumps, or jiggle. `HEX_CAPSULE_MOTION` also writes rune-motion traces for
  motion-lint. `cues_mark_start_stop_sent_and_taking_back`,
  `nothing_heard_says_so`, and `a_capture_with_no_words_says_nothing_was_heard`
  cover the tones and the empty transcript. Observed (Oct 26, pro): all 22
  capsule runs pass lint, including the countdown.
- Playground: `hex capsule-playground` (feature builds; hidden), or
  `scripts/build-capsule-playground.sh` for an ad-hoc signed
  `Hex Playground.app`, runs the real capsule in a window of its own against a
  fake host, with state buttons, the whole flow, slow motion, Reduce Motion,
  the tones, and live tuning ([tuning.rs](../../src/rune_ui/capture_capsule/tuning.rs);
  "Copy values" copies its `DEFAULT`). It reads and writes no Hex data and
  records or sends nothing. `--snap --scene countdown|guess|recording`
  renders it. The tones were checked by waveform and level only, never on
  speakers.
- Picker, in capture_capsule/picker.rs and tests.rs: geometry, label, and
  target tests, `the_picker_records_what_it_shows_and_comes_to_rest`, and
  `picker_stills_and_motion`, which scripts window hover, an area drag, and
  mode switches per frame (Calm and Reduce Motion). Observed (Oct 26, pro):
  all 6 runs pass motion-lint with two excuses: the target growing from a
  zero-size point, and the camera stopping where the scripted pointer stops.
  Not proven natively end to end: the overlays have been driven only by the
  scripted scenes, not a live capture.
- Pane: `captures_default_off_and_refuse_a_shortcut_in_use` and
  `retention_names_presets_and_imported_days` in rune_ui/scrawl.rs; scenes
  `mac-scrawl-off`, `mac-scrawl`, `mac-scrawl-running`, `mac-scrawl-permission`
  (feature builds only, so no goldens: those run on Linux).
- Settings: every `capture.*` key in `hex settings`
  (`every_persisted_setting_has_a_cli_key`).

Observed 2026-10-25 on pro (M2 Max), before the real engine, with the
stand-in: `HEX_EXPERIMENTS=capture hex dev capture-replay` over three
vocabulary fixtures (6.5 s) loaded Parakeet unified English on Metal and
wrote 18 words with plausible times. The real engine has not run end to end:
the unit tests use a fake engine over Scrawl's real bundle writer, and the
mini (the test Mac) has no Screen Recording grant for a dev build, so neither
`hex capture` against a live app nor `capture-replay` has recorded a screen.
The shortcut, tap input, Escape, switching, routing, the iPhone path, Scrawl
coexistence, and import are covered by unit tests only; none has run against a
live Hex.
