# Port Hex's Desktop UI From GPUI To Rune

**Status:** done on the `rune-ui` branch: Rune is Hex's only UI toolkit and
GPUI is gone (`rune-ui-no-gpui`, 2026-10-22). Every surface runs on Rune: the
Linux single-process window and HUD, the Windows tray app and HUD, the macOS
menu-bar app with its meeting capsules, every pane (Settings, Modes, Voice
Action, History, Commands, Meetings, Fun, Activity, HUD Lab), the onboarding
tour, and the transcription picker. The macOS dictation HUD stays a native
Metal `NSPanel` (`dictation_indicator.rs`, no GPUI dependency). There is no
`rune-ui` cargo feature and no `--ui` flag any more. Rune, `rune-kit`, and
`rune-theme-opencode` are pinned to a commit on Rune's `main`
(`https://github.com/anomalyco/rune.git`; public and MIT, see
[rune-dependency.md](rune-dependency.md)). Do not merge to `main` until a
person has checked [the real-window list](#needs-a-person-to-look) on each OS:
Linux ran in a real session, macOS was compiled, tested, and launched but not
seen, and Windows was only cross-compiled.

What went with GPUI: `app_window.rs` and its OS modules, `desktop_ui.rs`,
`design_tokens.rs` and its token lint, `text_input.rs`, `motion.rs` (the
macOS HUD's springs live on in `spring.rs`), `desktop_transcription_picker.rs`,
`windows_indicator.rs`, `windows_ui.rs`, the GTK Linux HUD
(`linux_desktop.rs`), the GPUI parts of `linux_app.rs`, `windows_app.rs`, and
`meeting_watcher.rs` (now only the meeting controller), `hex preview`,
`hex app --preview-dictation`, `hex meeting watch`, `scripts/capture-preview.sh`
(replaced by `--snap` scenes), the GPUI fork patch and its Nix hash, and the
Modes picker's application icons (Rune has no raster images yet; restore the
per-OS icon loading from git history when it does). The pure helpers the panes
use moved to `opencode_catalog.rs` (model catalog, presentation, unavailable
copy), `app_settings` (`HotkeyBinding::standalone_side`),
`desktop_activity::event_summary`, and the panes that own them.

## Design direction

The Rune front end uses **OpenCode's design language through Rune's component
library**, not the GPUI design system Hex had before. Controls come from `rune-kit`
(generic components with `@opencode/ui`'s names: `Button`, `IconButton`,
`Switch`, `SegmentedControl`, `Select`, `Menu`, `Popover`, `TextInput`,
`TextArea`, `Badge`, `Progress`, …), and every colour, elevation, radius,
type style, and motion token comes from the OpenCode theme
(`rune_theme_opencode::kit(dark)`), which is `@opencode/ui`'s tokens as data.
The window follows OpenCode Desktop's layout: a deep-background sidebar and a
raised, rounded content panel. Theming stays a Rune feature: Hex could switch
themes without touching a view. Hex keeps its information, sections, copy,
and behavior; only the look and layout follow the kit.

What that means for Hex's existing rules:

- `docs/design-system.md` is now a short Hex page that points at `rune-kit`
  and the OpenCode theme and lists only Hex's own pieces and the copy rules
  (sentence case, "Hex", one accent action per screen, descriptions without
  full stops).
- The GPUI token lint is gone with GPUI. Rune views name kit tokens
  (`p.k.c.*`, `p.k.ty.*`, `p.k.radius.*`, `p.k.elevation.*`) or kit
  components; the raw numbers left are layout metrics (row heights, column
  widths, gaps). A Rune lint over `src/rune_ui/` that rejects raw colours
  (`oklch(`, `Color {`, `hex(`) outside the theme crate is still to write; the
  Linux HUD's fixed dark pill is the one intended exception.
- Hex gains a light appearance for free (Rune follows the system), so every
  Rune pane has light and dark goldens.
- Danger, success, and accent roles come from the theme's state colours; the
  one main action per screen is the kit's contrast button, destructive
  confirmations the kit's danger button.
- The translation test (`desktop_i18n::tests::every_translated_string_has_every_language`)
  scans `src/rune_ui.rs` and every pane and host module, so new copy needs all
  five languages. Translations only GPUI used were removed.

## What exists on Linux

```text
src/rune_ui.rs                 app spec, shared state, service bridge, poll,
                               drafts, the shell (sidebar + panel), scenes  741
src/rune_ui/settings.rs        Settings pane, shortcut capture             1067
src/rune_ui/voice_action.rs    Voice Action pane                             77
src/rune_ui/models.rs          OpenCode catalog, model + thinking pickers   492
src/rune_ui/history.rs         History pane                                 502
src/rune_ui/modes.rs           Modes pane                                  1329
src/rune_ui/picker.rs          transcription picker modal                   518
src/rune_ui/onboarding.rs      first-run tour (Linux setup, full macOS)     897
src/rune_ui/controls.rs        rows, cards, selects, buttons on rune-kit    383
src/rune_ui/platform.rs        Capabilities, OS-only state, requests        409
src/rune_ui/platform_settings.rs  macOS/Windows Settings rows             583
src/rune_ui/commands.rs        Commands pane (macOS)                        392
src/rune_ui/meetings.rs        Meetings pane (macOS beta)                   963
src/rune_ui/lists.rs           list/detail pieces, tags, permission control 320
src/rune_ui/tests.rs           Linux tests and goldens                      599
src/rune_ui/tests/panes.rs     macOS/Windows surface tests and goldens      422
src/linux_file_chooser.rs      the FileChooser portal ("Choose a file…")    163
```

Every pane kept GPUI's behavior when it was ported. Since the single-process
change the window runs inside the Hex process and holds a `linux_app::HostHandle`: edits save `settings.json`
through `HostHandle::save_settings`, which adopts them at once; listening,
shortcuts, models, and updates are direct `HostRequest`s (`Desktop(…)`,
`CaptureShortcutFor`, `SetShortcut`, `CancelShortcut`, `ChooseModel`,
`CancelModel`); the host thread wakes the window when its state changes, and
History shares the host's store. Settings the host saved (a prepared model, a
recorded shortcut) are adopted only when the host's copy changes, so a stale
snapshot never reverts a fresh edit. The window runs on `rune::App`
(`windowless`, `on_close`, `AppHandle::activate`/`open`/`quit`) from
`rune_ui::run_app`; its timed redraws (drafts, retries) ride a quarter-second
tick while the window is open, until `Tx::wake_after` exists. Text fields are drafts saved when
typing pauses for 250 ms or when the pane or mode changes.

| Pane | Status | Screenshots (light / dark) |
| --- | --- | --- |
| Shell | Sidebar with the four Linux panes, version, Update button when a restart is ready; Escape closes the open menu or the picker | [settings light](rune-ui/hex-rune-shell-portal-light.png) / [dark](rune-ui/hex-rune-shell-portal-dark.png) |
| Settings | Every Linux row (status, desktop shortcuts, dictation, behavior, application); the transcription row opens the picker | as above, plus [Chinese](rune-ui/hex-rune-shell-chinese-dark.png) |
| Voice Action | Opt-in with the shortcut conflict check, the recorder, OpenCode model (searchable popover) and thinking pickers, unavailable/missing OpenCode notice with retry and setup | [light](rune-ui/hex-rune-shell-voice-action-light.png) / [dark](rune-ui/hex-rune-shell-voice-action-dark.png), [model search](rune-ui/hex-rune-shell-voice-action-models-dark.png), [OpenCode missing](rune-ui/hex-rune-shell-voice-action-unavailable-light.png) |
| History | Search, retention select, two-step Clear all, detail with Copy (Rune's clipboard request) and Delete, latency and processing facts; follows new dictations while shown | [light](rune-ui/hex-rune-shell-history-light.png) / [dark](rune-ui/hex-rune-shell-history-dark.png) |
| Modes | Global + custom modes; name; applications with a searchable picker and the cross-mode conflict check; corrections; OpenCode rewriting (model, thinking, deadline, instructions); applications from the desktop's file chooser ("Choose a file…"); transformations with add, drag to reorder, remove, and the TypeScript workspace status/setup; two-step delete | [light](rune-ui/hex-rune-shell-modes-light.png) / [dark](rune-ui/hex-rune-shell-modes-dark.png), [pickers open](rune-ui/hex-rune-shell-modes-picker-light.png) |
| Transcription picker | Modal over a scrim: languages, model cards with recommendation and streaming badges, metrics, Use/Download/Cancel/Delete (two-step), download/verify/load progress from the service snapshot, install and delete errors | [light](rune-ui/hex-rune-shell-picker-light.png) / [dark](rune-ui/hex-rune-shell-picker-dark.png), [downloading](rune-ui/hex-rune-shell-picker-downloading-dark.png) |
| Setup | `onboarding::Flow` without permissions or practice (Linux has neither): Welcome, the model download through the service with progress and retry, Done with the shortcut tips; records completion when the model is ready | goldens `rune-onboarding-dark`, `rune-onboarding-setup-{light,dark}` |

### macOS and Windows surfaces, rendered on Linux

The panes macOS and Windows add are ported and run on Linux from preview
data, so they compile, test, and golden-render on apollo. Windows' Launch at
login row is live through `host::set_launch_at_login` (the Windows host's
Run key). On macOS `macos_host::MacHost` performs these requests in the single-process app
([single-process plan](single-process.md#macos-under-rune)); it was
compiled, tested, and briefly launched on hailmary, but no pane's live
macOS data has been seen on screen.

- **`platform::Capabilities`** decides which panes and rows each OS shows
  (`LINUX`, `MACOS`, `WINDOWS`; `current()` by `cfg!`). Navigation and Settings read it: Commands and Meetings panes
  (macOS; Meetings also needs `settings.meetings.available()`), the listener
  card and Enable on launch (Linux, Windows), Terminal paste (Linux), Launch
  at login (macOS, Windows), Show Dock icon, permission warnings, Meetings
  and Experimental: Hex microphone sections, Sparkle's Check now (macOS).
- **`platform::PlatformState`** is the OS-only data the panes show:
  `SetupStatus` and which permissions were asked, System Audio, the login
  item status (or checking/unavailable), the Hex Microphone driver status,
  the command model state, the merged command catalog and workspace state,
  the current context, Apple Intelligence availability, folder and relaunch
  errors; and for the panes added later: `developer` (Activity and HUD Lab
  show; `Ui::load` sets it from `DEVELOPER_FEATURES_ENABLED`),
  `accessibility_renamed` (macOS 27's "Device Control and Data Access"
  name, from `permission_guide::list_name`), `tuning`
  (`hex_microphone::current_tuned_config`), `demo`
  (`hex_microphone::demo_snapshot`, refreshed every poll while it records or
  plays), `hud` (what HUD Lab shows; the pane owns it), and `activity_log`
  (the `live.ndjson` the Activity pane reads). Meetings arrive as `meetings::Meeting` values (titles, day and
  clock strings, status, notes, errors, coalesced turns) because
  `meeting.rs` is macOS-only.
- **`platform::PlatformRequest`** is every action they take: permission
  request or guide, System Audio, login item set/open settings, driver
  install/uninstall, meeting start/stop/rename/trash/summarize/reveal/folder,
  commands config create/open, command model retry, relaunch, and later
  `SetTuning(TunedFilterConfig)` (apply to the live filter),
  `HexMicrophoneDemo(DemoRequest)` (record, stop, choose the clip, play a
  track, stop playback), and `ConfigureHud(HudLab)` (show HUD Lab's tuning
  and state on the indicator). `MacHost`
  handles all three; the last needs `RuntimeHandle::indicator`.
  `PlatformHost::Fixture` records them for scenes and tests; `Native(Box<dyn
  NativeHost>)` is the seam a macOS/Windows host implements (`send` and a
  `poll` that refreshes `PlatformState`). Settings edits (Dock icon, meeting
  toggles, Hex Microphone mode, commands opt-in) still go through
  `Ui::update` and `settings.json`; a native host must apply their side
  effects (Dock policy, `hex_microphone::update_tuned_config`) when settings
  change.

| Surface | Status | Goldens (`tests/golden/rune-*.png`) |
| --- | --- | --- |
| Commands | Catalog grouped by task, search over phrase/aliases/description/task/scope, detail (task, context, aliases, current context), Create/Edit config, Copy agent prompt, the opt-in with the keep-microphone-ready confirmation, the command model notice with Retry and Turn off commands | `mac-commands-{light,dark}`, `mac-commands-confirm-light` |
| Meetings | Day-grouped list with status tags and row Copy, search over title/app/summary/transcript, detail with notes, live/draft/final/recovered transcript as turns with pauses, errors, Copy transcript, Rename (field, Save, Cancel, Escape), overflow menu (Copy summary, Reveal, Summarize/Regenerate, Delete…), two-step trash ("Trash recording & Markdown"), Start recording / Stop & transcribe with elapsed time, empty and runtime-error states | `meetings-{light,dark}`, `meetings-recording-{light,dark}`, `meetings-actions-dark`, `meetings-delete-light`, `meetings-rename-dark`, `meetings-empty-light` |
| Settings, macOS | Permissions needed (Allow, a steady dot while asked, Allowed), the command model notice, Meetings section (opt-in, System Audio permission, indicator, folder Reveal/choose, Never offer with Remove, Apple Intelligence notes with availability copy), Launch at login (switch or Open settings when approval is needed, checking/unavailable), Show Dock icon, Check now, Experimental: Hex microphone (driver install/update/uninstall states, mode, comfort noise); no listener card or Terminal paste | `mac-settings-{light,dark}`, `mac-settings-permissions-dark` |
| Settings, Windows | Listener card, Launch at login, Enable on launch; no Terminal paste or desktop shortcuts | `windows-settings-light` |
| Fun (macOS, Hex Microphone experiment) | Driver row and status tag, nine modes, the mode's key sliders (plus Room and Level for Muffled), Reset, comfort noise in Muted, Try it: Example/Your voice, Record 5s/Stop recording, the two tracks as bar waveforms drawn with `rune::Shape` in a view box, the dictated span highlighted, progress, Play what call hears/Play your voice/Stop; Settings' Try it row opens it. Long mode labels truncate at the default width | `mac-fun-{light,dark}`, `mac-fun-muted-dark`, `mac-fun-driver-missing-light` |
| HUD Lab (macOS debug) | Recording/Transcribing/Processing, queued 0–3, the three styles, eight sliders, the preset line and Copy preset | `mac-hud-lab-{light,dark}` |
| Activity (debug) | The event log's newest 100 events with ages, the listener's state and device, Refresh, a detail with the event kind, timestamp, and raw JSON | `mac-activity-light` |
| Shortcut sides (macOS) | Left/Either/Right before a modifier-only recorder (dictation, paste last, Voice Action); sides that collide with another shortcut are disabled | `mac-settings-sides-dark` |
| Tour, macOS | Welcome (static keycap art), Setup with Microphone/Input Monitoring/Accessibility rows (Allow, waiting, refused → System Settings guide, Allowed), the background download beside the step dots (progress, ready, stopped + Retry), the relaunch notice, Give it a try with the practice field and every practice state (waiting, listening, too short, nothing heard, failed, landed, reopen Hex), Skip, Done with the menu-bar send-off; Windows uses the system-tray send-off | `mac-onboarding*` (10 states) |

Every former GPUI pane has its Rune counterpart. Sliders drag through Rune's
`on_drag`; the waveforms are `rune::Shape` round-capped segments. What the
macOS panes do on a Mac (driver, demo audio, HUD) is unverified: on hailmary
(Command Line Tools only, with a local stub for the Metal shader and Apple
Intelligence) the macOS build passes clippy and the test suite, including
these panes' goldens on Metal, but was not used.

Run it: `cargo run --release` (with a custom
`HEX_APPLICATION_SUPPORT_DIR` to stay away from an installed Hex). Headless:
`hex app -- --snap out.png --scene modes --theme light`. Scenes:
`live` (the running Hex's `hex status`), `portal`,
`commands`, `disabled`, `capturing`, `chinese`, `menus`, `voice-action`,
`voice-action-off`, `voice-action-unavailable`, `voice-action-models`,
`history`, `history-empty`, `modes`, `modes-global`, `modes-picker`, `picker`,
`picker-downloading`, `onboarding`, `onboarding-setup`. macOS and Windows
preview scenes: `mac-settings`, `mac-settings-permissions`,
`windows-settings`, `mac-commands`, `mac-commands-confirm`, `meetings`,
`meetings-recording`, `meetings-actions`, `meetings-delete`,
`meetings-rename`, `meetings-failed`, `meetings-empty`, `mac-onboarding`,
`mac-onboarding-setup`, `mac-onboarding-progress`, `mac-onboarding-denied`,
`mac-onboarding-download-failed`, `mac-onboarding-try`,
`mac-onboarding-listening`, `mac-onboarding-landed`, `mac-onboarding-reopen`,
`mac-onboarding-done`, `mac-fun`, `mac-fun-muted`,
`mac-fun-driver-missing`, `mac-hud-lab`, `mac-activity`,
`mac-settings-sides`.

Checks: `cargo test rune_ui`. 58 tests drive the Rune
runtime headlessly without a GPU (navigation, every pane's edits and the
requests they send, drafts, conflicts, the picker, setup, and the macOS and
Windows surfaces from preview data); `goldens`, `platform_goldens`, and
`fun_goldens` render 53 PNGs offscreen (`tests/golden/rune-*.png`) and skip without a GPU adapter. `HEX_UPDATE_GOLDENS=1` rewrites them. Goldens
were written on apollo (RADV); lavapipe renders the same within tolerance.

### Observed on Linux

Renders below are headless (`--snap`, offscreen wgpu) unless marked as the
real window.

- **Real window on apollo (niri, 2026-10-22):** the release build ran (before
  the single-process change) as a client of the installed 2.3.1
  `hex.service` in Dax's session. Observed through `niri msg` and
  `screenshot-window`, without synthetic input:
  - niri lists it as `Hex` with `app_id: hex` (matches `hex.desktop`); with
    niri's `prefer-no-csd` it has no title bar, like every other window,
    and niri's focus-stealing prevention left the focused window alone.
    The 980×760 request is replaced by niri's column width (half the
    output), as for any tiled window; floating keeps the tiled size.
  - Settings showed the live service: Enabled, the portal's F12 and
    Ctrl+Alt+V grants, and the service's last listener error, as GPUI did.
  - Scale 2 (DP-7) and fractional 1.5 (DP-8, moved there and back) both
    rendered at native resolution (buffer = logical × scale), text crisp
    ([1.5](rune-ui/hex-rune-real-niri-fractional.png)).
  - Resizing the column to 700 and 640 logical px squeezed row descriptions
    to a word per line beside wide segmented controls: tiling desktops
    ignore the 860 px minimum and Rune's floor is 420. Rows now keep
    their copy at least 100 px wide and move the control below it when it
    does not fit ([before](rune-ui/hex-rune-real-niri-narrow-before.png),
    [after](rune-ui/hex-rune-real-niri-narrow.png)); the Modes editor's model row now wraps that way at the
    default size too (goldens updated).
  - Closing the window (`niri msg action close-window`) exited the client
    cleanly; `hex.service` stayed active with no restart.
  - The client log had no warnings or errors.

- **Against an isolated service** (`HEX_APPLICATION_SUPPORT_DIR`, no display,
  listener off, no OpenCode on `PATH`): one scripted run switched to Modes,
  added a mode, added a correction and pasted "open code" into it, switched to
  Voice Action and enabled it, then opened History's retention menu. The
  service's `settings.json` and its snapshot both reported `Mode 1` with the
  `open code` correction and Voice Action enabled
  ([end state](rune-ui/hex-rune-shell-live-isolated.png)).
- **Against the real `hex.service`** (first pass): Settings showed the
  portal's grants exactly as GPUI did
  ([live](rune-ui/hex-rune-live-real-service.png),
  [GPUI reference](rune-ui/gpui-settings-linux.png)).
- **CJK:** with Noto Sans SC/JP on the font path, the Chinese UI renders
  ([now](rune-ui/hex-rune-shell-chinese-dark.png),
  [before](rune-ui/hex-rune-chinese-tofu.png)). Neither apollo nor the agent
  host has a CJK font installed, so the check pointed `XDG_DATA_DIRS` at a
  downloaded copy; a desktop with any common CJK font gets it automatically.
- Earlier: Rune's examples ran on Linux (Vulkan/RADV and lavapipe), parallel
  GPU devices in tests crashed RADV (goldens share one device), and the motion
  strip showed springs and row reflow ([strip](rune-ui/hex-rune-motion-strip.png)).

### Needs a person to look

These need a person at the keyboard (no input was synthesized in Dax's
session):

- Pointer: hover and press states; cursor shapes (pointer over buttons and
  segments, I-beam over text fields, grab/grabbing on the transformation
  handles; unit-tested in Rune, never seen on screen); dragging a
  transformation with a real pointer (headless test only).
- "Choose a file…": the FileChooser portal dialog (version 4 on apollo's
  niri session, checked with `busctl introspect`) was never opened, so the
  dialog, its filters, and the round trip into the mode are unobserved. Hex
  passes no parent window handle (Rune exposes none), so the dialog is not
  attached to Hex's window.
- Copy and paste through the Wayland data-control clipboard (Rune now uses
  it on Wayland; never exercised on niri).
- Keyboard focus and IME in every text field; whether the model and
  application search fields take focus when their popover opens.
- Recording a physical F-key or Super shortcut through the service.
- Scrolling feel, the picker's scrim over the page, and menus near window edges.
- The 980×760 default as a floating window and on X11 (niri tiles it).

## What's left for Linux parity

- **App icons**: Rune has no raster images yet (Kit's `hex-gaps` agent has
  claimed them), so Modes shows monograms and the catalog no longer loads
  icons; restore the per-OS loaders from history with them.
- **Right-click "Delete mode…"**: no context-menu trigger on right press;
  delete is the two-step button in the editor.
- **Spinners and pulses**: no looping animation yet (the recording pulse on
  the shortcut recorder and the indeterminate "Loading model" bar are static;
  the loading label still counts seconds).
- **Keyboard navigation** in rune-kit menus and selects (microphone,
  language, retention, thinking): Kit's `motion-focus` agent is adding the
  focus model and arrow keys. The model and application searches already
  move a highlight with Up and Down and pick it with Enter (Hex-side, `controls::Highlight`; a rune-kit combobox would replace it).
- **Modal focus trap** for the picker; the scrim blocks presses only.

## Rune changes (all on Rune `main`)

| Change | Why |
| --- | --- |
| `rune::set_app_id(id)` → winit `with_name` on Wayland/X11 (`6d87e63`) | niri reported `app_id: None`; desktops need it to match `hex.desktop`, icons, and window rules |
| `Key::Function(1..=24)`, `Mods::logo`, script F-keys/logo (`6d87e63`) | Hex records F-key and Super shortcuts |
| `rune-kit`, `rune-theme-opencode`, CSS shadows/gradients/outlines, `El::anchored` overlays (`ad80ae1`…`1177fbd`) | the component library and OpenCode theme the panes use |
| CJK font fallback (`aebf3a1`) | app-supplied extra fonts, then installed CJK fonts (Noto/Source Han, PingFang, Hiragino, YaHei, WenQuanYi, …) become fontique fallbacks for Han, kana, Hangul, Bopomofo; Latin output byte-identical |
| `on_edit` takes closures; `rune::edit_access` (`338d22a`) | a list of fields (each mode's corrections) binds each row's `TextEdit` by index |
| `rune::Cursor`, `El::cursor`, `Runtime::cursor`; the shell sets the pointer shape; rune-kit buttons and segments use the pointer as @opencode/ui does | pointer and I-beam cursors; the drag handles grab |
| `arboard` with `wayland-data-control` | copy and paste on Wayland without XWayland |
| Vector shapes: `rune::Shape`, `Path`, `canvas`, `El::shape`, `KIND_SEGMENT`/`KIND_TRIANGLE` (`d666064`), `Shape::view_box` (`a5121c3`) | the Fun pane's waveforms; strokes and seam-free fills checked on lavapipe and hailmary's Metal |

Built in Hex for now, to move when Rune has them: the picker's modal scrim
(Kit's `motion-focus` agent is adding dialog, scrim, and focus trap), the
searchable model popover (a `List` with search exists in rune-kit; a
combobox component would replace Hex's), and the 250 ms draft debounce
(rides a 4 Hz tick while the window is open; a `Tx::wake_after` timer would
make it exact).

## Every surface and where it stands

```ts
Settings, Modes, Voice Action, History   // all OSes; Linux run for real, Windows type-checked, macOS tested
├── macOS rows                           // permissions, Launch at login, Dock icon, Meetings, Hex Microphone, modifier sides
└── Windows rows                         // Launch at login, Check now, Update, no terminal paste
Commands, Meetings, Fun, HUD Lab, Activity   // macOS host performs their requests; Activity and HUD Lab are debug-only
Transcription picker                     // Linux prepares in the host thread; Windows and macOS on their runtime threads
Onboarding tour                          // every OS through Capabilities; art static until looping tracks exist
Window shell                             // native decorations on Windows (custom caption still open); macOS traffic-light inset and El::window_drag
HUDs
├── macOS   dictation_indicator.rs       // native Metal NSPanel on macos_rune's main-loop timer; no GPUI dependency
├── Windows windows_rune_hud.rs          // shared model + CPU shader into rune::Pixels in an App::overlay; type-checked only
└── Linux   linux_rune_hud.rs            // App::overlay: layer-shell on Wayland, override-redirect on X11
Meeting capsules (macOS)                 // rune_ui/meeting_capsules.rs, interactive App::overlay; Copy new/all, hover expansion, drag not yet
Menu bar / tray                          // native NSStatusItem (status_item.rs) and tray-icon (windows_rune.rs) beside rune::App
Previews                                 // `hex app -- --snap … --scene …` on every OS; goldens in `cargo test`
```

## Gaps in Rune, in the order to close them

1. **Modal, focus, and keyboard navigation**: dialog with a scrim and focus
   trap, a focus model for non-text controls, Tab order, arrow keys in menus
   and lists (in progress by Kit's `motion-focus` agent).
2. **Timers and looping animation**: `Tx::wake_after`, looping tracks for
   spinners, pulses, and indeterminate progress (`motion-focus`).
3. **App and window model**: app lifetime separate from windows,
   hide-on-close, and XDG activation landed as `rune::App`/`AppHandle`
   (Rune `a242f5a`, see [single-process.md](single-process.md#landed-on-rune-main-a9cde14-a242f5a-hex-pins-a242f5a)).
   Several windows (`run_windows`, Rune `1e90268`), per-app minimum size
   (`AppSpec::min_size`, `1f6fbd0`) and edge overlays (`App::overlay`,
   `2f338f3`: Wayland layer-shell, X11 override-redirect, a macOS
   status-level window that never becomes key, a Windows no-activate tool
   window), interactive non-activating panels (`3a5fbca`) and `El::pixels`
   (`50207c3`) landed.
4. **Keyboard**: key-up and modifier-only events, left/right modifier sides,
   physical key codes, Globe/Fn (macOS shortcuts).
5. **Raster images** (app icons). Vector shapes landed (`rune::Shape`,
   `d666064`, `a5121c3`).
6. **Platform text input and clipboard**: native IME protocols; Wayland
   clipboard: done (`arboard` with `wayland-data-control`, X11 fallback).
7. **Accessibility** (AccessKit). Cursor shapes: done.
8. **Native file dialogs**: Linux uses the FileChooser portal from Hex
   (`linux_file_chooser.rs`, over gio like `linux_portal.rs`); macOS
   (`NSOpenPanel`) and Windows (`IFileDialog`) still need one.
9. **Scrolling polish**: visible scrollbars, scroll-into-view.

## Order to port (done)

1. Linux: the single-process window and the HUD.
2. Windows: the tray app, single instance, in-process runtime, and HUD
   (`windows_rune.rs`); never run on Windows.
3. macOS: the menu-bar app (`macos_rune.rs`), panes, tour, Commands,
   Meetings, and the meeting capsules.
4. GPUI deleted (`rune-ui-no-gpui`), with `docs/design-system.md` rewritten
   per [Design direction](#design-direction).

## What stays native per platform

- **Overlays:** Rune's `App::overlay` hosts them: Wayland layer-shell, an X11
  override-redirect window, a topmost no-activate tool window on Windows, and
  a non-activating panel on macOS. The macOS dictation HUD keeps its own
  Metal `NSPanel`.
- **Menu bar and tray:** `NSStatusItem`/`NSMenu` and `tray-icon`. Linux has none.
- **Input methods, clipboard, and accessibility:** each platform's protocol.
- **Global shortcuts and paste:** unchanged (`linux_input`, `linux_portal`,
  `suppression`, `windows_input`, `paste`); the UI only records and shows them.

## Risks

- **Dependency.** Rune is public (MIT) and fetched over HTTPS like any other
  Git dependency. A fresh clone with no SSH and an empty `CARGO_HOME` built
  it on apollo, and the Nix vendor directory builds with
  Rune's output hash. [rune-dependency.md](rune-dependency.md) records the
  decision and the pin-bump steps (Cargo and the Nix hash).
- **API churn.** Kit's agents change Rune's `main` daily. Hex pins a commit,
  keeps its Rune code in `src/rune_ui*`, and moves the pin deliberately. A
  pending text-quality change will shift glyph rendering; the goldens' 12/255
  tolerance and 0.2% pixel budget may need rewriting when it lands.
- **Unproven platform shells.** With GPUI gone there is no fallback UI.
  Windows has only been cross-compiled (`cargo xwin clippy` on apollo; see
  `docs/windows.md`), never run, and the next Windows release would ship it;
  the macOS app was launched on hailmary but never seen on screen.
- **Windows lockfile.** Resolving Rune for Windows put `gpu-allocator` on
  `windows` 0.57, which breaks `wgpu-hal`'s
  DX12 backend; `Cargo.lock` pins `gpu-allocator`'s `windows` to 0.62.2 by
  hand. A `cargo update` that re-resolves it brings the error back.
- **Pre-existing on `f8774a5`:** `linux_app::tests::malformed_settings_cannot_start_even_after_dismissing_the_error`
  fails on Linux without these changes, and
  `dictation_processor::tests::service_discovery_fallback_shares_deadline_and_observes_cancellation`
  is timing-sensitive (it failed once under a parallel full run and passed
  three times alone).
