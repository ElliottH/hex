# Port Hex's Desktop UI From GPUI To Rune

**Status:** Linux has a complete experimental Rune front end on the `rune-ui`
branch: `hex app --ui rune` (built with `--features rune-ui`) shows every pane
GPUI's Linux Settings process shows (Settings, Modes, Voice Action, History),
the local transcription picker, and first-run setup. Windows has the same
panes in a single-instance tray process (`hex app --ui rune`), written and
cross-compiled but never run (see `docs/windows.md`). GPUI stays the default
everywhere. Rune, `rune-kit`, and `rune-theme-opencode` are pinned to a
commit on Rune's `main` (`anomalyco/rune`, `7eea3a3`, over HTTPS). Rune
became public on 2026-10-22 as a fresh one-commit history (`7eea3a3`, the
tree of the old private `a242f5a`); the old pins such as `338d22a` can no
longer be fetched. Do not merge
to `main` until the dependency questions under [Risks](#risks) are settled
and a person has checked [the real-window list](#needs-a-person-to-look).

**Goal:** one Rune front end for Hex on macOS, Windows, and Linux, replacing
about 29k lines of GPUI (`app_window.rs` and its OS modules, `desktop_ui.rs`,
`design_tokens.rs`, `text_input.rs`, `desktop_transcription_picker.rs`,
`motion.rs`, the indicator and meeting capsules, and the GPUI parts of
`linux_app.rs`, `windows_app.rs`, and `windows_ui.rs`).

## Design direction

The Rune front end uses **OpenCode's design language through Rune's component
library**, not Hex's current GPUI design system. Controls come from `rune-kit`
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

- `docs/design-system.md` describes the GPUI front end and stays authoritative
  for it while GPUI ships. It does not govern `src/rune_ui*`. When Rune
  becomes the default, replace it with a short Hex page that points at
  `rune-kit` and the OpenCode theme and lists only Hex-specific pieces
  (keycaps, shortcut recorder, listener card, model cards) and copy rules,
  which still apply (sentence case, "Hex", one accent action per screen,
  descriptions without full stops).
- The token lint (`design_tokens::tests::desktop_sources_use_tokens_instead_of_raw_styles`)
  matches GPUI calls (`rgb(0x…)`, `.text_size(`, `.rounded(px(`), so it neither
  applies to nor flags Rune code. Rune views name kit tokens (`p.k.c.*`,
  `p.k.ty.*`, `p.k.radius.*`, `p.k.elevation.*`) or kit components; the raw
  numbers left are layout metrics (row heights, column widths, gaps). A Rune
  lint over `src/rune_ui/` should reject raw colours (`oklch(`, `Color {`,
  `hex(`) outside the theme crate; delete the GPUI lint with the last GPUI
  pane.
- Hex gains a light appearance for free (Rune follows the system), so every
  Rune pane has light and dark goldens.
- Danger, success, and accent roles come from the theme's state colours; the
  one main action per screen is the kit's contrast button, destructive
  confirmations the kit's danger button.
- The translation test (`desktop_i18n::tests::every_translated_string_has_every_language`)
  scans `src/rune_ui.rs` and every pane module, so new Rune copy needs all five
  languages like GPUI copy.

## What exists on Linux

```text
src/rune_ui.rs                 app spec, shared state, service bridge, poll,
                               drafts, the shell (sidebar + panel), scenes  741
src/rune_ui/settings.rs        Settings pane, shortcut capture             1067
src/rune_ui/voice_action.rs    Voice Action pane                             77
src/rune_ui/models.rs          OpenCode catalog, model + thinking pickers   492
src/rune_ui/history.rs         History pane                                 502
src/rune_ui/modes.rs           Modes pane                                  1204
src/rune_ui/picker.rs          transcription picker modal                   518
src/rune_ui/onboarding.rs      first-run tour (Linux setup, full macOS)     897
src/rune_ui/controls.rs        rows, cards, selects, buttons on rune-kit    383
src/rune_ui/platform.rs        Capabilities, OS-only state, requests        409
src/rune_ui/platform_settings.rs  macOS/Windows Settings rows             583
src/rune_ui/commands.rs        Commands pane (macOS)                        392
src/rune_ui/meetings.rs        Meetings pane (macOS beta)                   963
src/rune_ui/lists.rs           list/detail pieces, tags, permission control 320
src/rune_ui/tests.rs           Linux tests and goldens                      554
src/rune_ui/tests/panes.rs     macOS/Windows surface tests and goldens      422
```

Every pane keeps GPUI's behavior and service contract. The window is a
`linux_service::Client` like the GPUI Settings process: it saves
`settings.json` before adopting an edit and sends `ReloadSettings`; listening,
shortcuts, models, and updates go through `Desktop(…)`, `CaptureShortcutFor`,
`SetShortcut`, `CancelShortcut`, `ChooseModel`, and `CancelModel`. Settings
another client saved are adopted only when the service's copy changes, so a
stale snapshot never reverts a fresh edit. Text fields are drafts saved when
typing pauses for 250 ms (GPUI's debounce) or when the pane or mode changes.
The pure helpers both front ends need (OpenCode catalog loading and copy,
model presentation, Voice Action enablement, transformation reordering) are
shared from `app_window` rather than copied.

| Pane | Status | Screenshots (light / dark) |
| --- | --- | --- |
| Shell | Sidebar with the four Linux panes, version, Update button when a restart is ready; Escape closes the open menu or the picker | [settings light](rune-ui/hex-rune-shell-portal-light.png) / [dark](rune-ui/hex-rune-shell-portal-dark.png) |
| Settings | Every Linux row (status, desktop shortcuts, dictation, behavior, application); the transcription row opens the picker | as above, plus [Chinese](rune-ui/hex-rune-shell-chinese-dark.png) |
| Voice Action | Opt-in with GPUI's shortcut conflict check, the recorder, OpenCode model (searchable popover) and thinking pickers, unavailable/missing OpenCode notice with retry and setup | [light](rune-ui/hex-rune-shell-voice-action-light.png) / [dark](rune-ui/hex-rune-shell-voice-action-dark.png), [model search](rune-ui/hex-rune-shell-voice-action-models-dark.png), [OpenCode missing](rune-ui/hex-rune-shell-voice-action-unavailable-light.png) |
| History | Search, retention select, two-step Clear all, detail with Copy (Rune's clipboard request) and Delete, latency and processing facts; follows new dictations while shown | [light](rune-ui/hex-rune-shell-history-light.png) / [dark](rune-ui/hex-rune-shell-history-dark.png) |
| Modes | Global + custom modes; name; applications with a searchable picker and the cross-mode conflict check; corrections; OpenCode rewriting (model, thinking, deadline, instructions); transformations with add, reorder (up/down), remove, and the TypeScript workspace status/setup; two-step delete | [light](rune-ui/hex-rune-shell-modes-light.png) / [dark](rune-ui/hex-rune-shell-modes-dark.png), [pickers open](rune-ui/hex-rune-shell-modes-picker-light.png) |
| Transcription picker | Modal over a scrim: languages, model cards with recommendation and streaming badges, metrics, Use/Download/Cancel/Delete (two-step), download/verify/load progress from the service snapshot, install and delete errors | [light](rune-ui/hex-rune-shell-picker-light.png) / [dark](rune-ui/hex-rune-shell-picker-dark.png), [downloading](rune-ui/hex-rune-shell-picker-downloading-dark.png) |
| Setup | `onboarding::Flow` without permissions or practice (Linux has neither): Welcome, the model download through the service with progress and retry, Done with the shortcut tips; records completion when the model is ready | goldens `rune-onboarding-dark`, `rune-onboarding-setup-{light,dark}` |

### macOS and Windows surfaces, rendered on Linux

The panes macOS and Windows add are ported and run on Linux from preview
data, so they compile, test, and golden-render on apollo. None has run on
macOS or Windows. Windows' Launch at login row is live through
`host::set_launch_at_login` (the Windows host's Run key); every other
macOS-only surface has no native host yet, and Hex cannot be compiled for
macOS here (hailmary refuses SSH). `rune_ui` compiles for Linux and Windows
(`main.rs`), not macOS, so these modules cannot break the macOS build.

- **`platform::Capabilities`** mirrors GPUI's `app_window::Capabilities` for
  the rows Rune draws (`LINUX`, `MACOS`, `WINDOWS`; `current()` by
  `cfg!`). Navigation and Settings read it: Commands and Meetings panes
  (macOS; Meetings also needs `settings.meetings.available()`), the listener
  card and Enable on launch (Linux, Windows), Terminal paste (Linux), Launch
  at login (macOS, Windows), Show Dock icon, permission warnings, Meetings
  and Experimental: Hex microphone sections, Sparkle's Check now (macOS).
- **`platform::PlatformState`** is the OS-only data the panes show:
  `SetupStatus` and which permissions were asked, System Audio, the login
  item status (or checking/unavailable), the Hex Microphone driver status,
  the command model state, the merged command catalog and workspace state,
  the current context, Apple Intelligence availability, folder and relaunch
  errors. Meetings arrive as `meetings::Meeting` values (titles, day and
  clock strings, status, notes, errors, coalesced turns) because
  `meeting.rs` is macOS-only.
- **`platform::PlatformRequest`** is every action they take: permission
  request or guide, System Audio, login item set/open settings, driver
  install/uninstall, meeting start/stop/rename/trash/summarize/reveal/folder,
  commands config create/open, command model retry, update check, relaunch.
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
| Tour, macOS | Welcome (static keycap art), Setup with Microphone/Input Monitoring/Accessibility rows (Allow, waiting, refused → System Settings guide, Allowed), the background download beside the step dots (progress, ready, stopped + Retry), the relaunch notice, Give it a try with the practice field and every practice state (waiting, listening, too short, nothing heard, failed, landed, reopen Hex), Skip, Done with the menu-bar send-off; Windows uses the system-tray send-off | `mac-onboarding*` (10 states) |

Not ported: the Fun pane and its "Try it" row, Activity, and HUD Lab
(developer panes; they need custom vector drawing for waveforms and
meters), the left/right modifier-side controls for macOS shortcuts, the
macOS 27 "Device Control and Data Access" name for Accessibility (the host
should supply it), and every native host.

Run it: `cargo run --release --features rune-ui -- app --ui rune`. Headless:
`hex app --ui rune -- --snap out.png --scene modes --theme light`. Scenes:
`live` (waits for the running service's first snapshot), `portal`,
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
`mac-onboarding-done`.

Checks: `cargo test --features rune-ui rune_ui`. 44 tests drive the Rune
runtime headlessly without a GPU (navigation, every pane's edits and the
requests they send, drafts, conflicts, the picker, setup, and the macOS and
Windows surfaces from preview data); `goldens` and `platform_goldens` render
45 PNGs offscreen (`tests/golden/rune-*.png`) and skip without a GPU adapter. `HEX_UPDATE_GOLDENS=1` rewrites them. Goldens
were written on apollo (RADV); lavapipe renders the same within tolerance.

### Observed on Linux

Renders below are headless (`--snap`, offscreen wgpu) unless marked as the
real window.

- **Real window on apollo (niri, 2026-10-22):** the release build
  (`--features rune-ui`) ran as a client of the installed 2.3.1
  `hex.service` in Dax's session. Observed through `niri msg` and
  `screenshot-window`, without synthetic input:
  - niri lists it as `Hex` with `app_id: hex` (matches `hex.desktop`); with
    niri's `prefer-no-csd` it has no title bar, like every other window,
    and niri's focus-stealing prevention left the focused window alone.
    The 980×760 request is replaced by niri's column width (half the
    output), as for any tiled window; floating keeps the tiled size.
  - Settings showed the live service: Enabled, the portal's F12 and
    Ctrl+Alt+V grants, and the service's last listener error, as GPUI does.
  - Scale 2 (DP-7) and fractional 1.5 (DP-8, moved there and back) both
    rendered at native resolution (buffer = logical × scale), text crisp
    ([1.5](rune-ui/hex-rune-real-niri-fractional.png)).
  - Resizing the column to 700 and 640 logical px squeezed row descriptions
    to a word per line beside wide segmented controls: tiling desktops
    ignore GPUI's 860 px minimum and Rune's floor is 420. Rows now keep
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
  portal's grants exactly as GPUI does
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

- Pointer: hover and press states, cursor shapes (Rune never sets one, so
  expect the default arrow over buttons and text fields).
- Keyboard focus and IME in every text field; whether the model and
  application search fields take focus when their popover opens.
- Recording a physical F-key or Super shortcut through the service.
- Scrolling feel, the picker's scrim over the page, and menus near window edges.
- The 980×760 default as a floating window and on X11 (niri tiles it).

## What's left for Linux parity

- **File chooser for applications** ("Choose a file…" in the Modes app
  picker): Rune has no native dialog; add an XDG portal or `rfd` call.
- **App icons**: the catalog's icons are not drawn (Rune has no raster
  images); Modes shows monograms.
- **Drag to reorder transformations**: replaced by up/down buttons.
- **Right-click "Delete mode…"**: no context-menu trigger on right press;
  delete is the two-step button in the editor.
- **Spinners and pulses**: no looping animation yet (the recording pulse on
  the shortcut recorder and the indeterminate "Loading model" bar are static;
  the loading label still counts seconds).
- **Keyboard navigation** in menus, the model search, and the application
  search (arrow keys move a highlight in GPUI); Enter picks the first match.
- **Modal focus trap** for the picker; the scrim blocks presses only.
- Making `--ui rune` the Linux default, after the real-window checks.

## Rune changes (all on Rune `main`)

| Change | Why |
| --- | --- |
| `rune::set_app_id(id)` → winit `with_name` on Wayland/X11 (`6d87e63`) | niri reported `app_id: None`; desktops need it to match `hex.desktop`, icons, and window rules |
| `Key::Function(1..=24)`, `Mods::logo`, script F-keys/logo (`6d87e63`) | Hex records F-key and Super shortcuts |
| `rune-kit`, `rune-theme-opencode`, CSS shadows/gradients/outlines, `El::anchored` overlays (`ad80ae1`…`1177fbd`) | the component library and OpenCode theme the panes use |
| CJK font fallback (`aebf3a1`) | app-supplied extra fonts, then installed CJK fonts (Noto/Source Han, PingFang, Hiragino, YaHei, WenQuanYi, …) become fontique fallbacks for Han, kana, Hangul, Bopomofo; Latin output byte-identical |
| `on_edit` takes closures; `rune::edit_access` (`338d22a`) | a list of fields (each mode's corrections) binds each row's `TextEdit` by index |

Built in Hex for now, to move when Rune has them: the picker's modal scrim
(Kit's `motion-focus` agent is adding dialog, scrim, and focus trap), the
searchable model popover (a `List` with search exists in rune-kit; a
combobox component would replace Hex's), and the 250 ms draft debounce
(polls on the service client's 4 Hz wakeups; a `Tx::wake_after` timer would
make it exact).

## Every GPUI surface and what it needs

Sizes are current GPUI lines. Effort is focused agent-days with review.

| Surface | Today | Status / Rune needs | Effort left |
| --- | --- | --- | --- |
| Settings pane | `render_settings` in `app_window.rs` | Done on Linux; Windows rows written (Launch at login, Check now, Update, no terminal paste), type-checked only; macOS rows (permissions, Launch at login, Dock icon, Meetings and Hex Microphone sections) ported behind `Capabilities` and rendered from preview data, with no macOS host; modifier sides remain | 1 |
| Window shell | `app_window.rs`, `desktop_ui.rs` | Done on Linux; Windows uses the same shell under native decorations (its caption row waits on hit-testing); macOS needs its traffic-light inset | 1 |
| Transcription picker | `desktop_transcription_picker.rs` (475) | Done on Linux (service-prepared); Windows prepares in-process through `transcription_preparation` on its runtime thread (type-checked only); macOS likewise, to do | 0.5 |
| Modes | ~2.5k lines in `app_window.rs` | Done on Linux minus file chooser, icons, drag, context menu; macOS adds website rules (Brave) | 3 |
| Voice Action | ~500 | Done on Linux | 0.5 |
| History | ~250 | Done | 0 |
| Commands (macOS) | ~400 | Ported, rendered on Linux from preview data; needs the macOS host (catalog, workspace, model status) | 0.5 |
| Meetings pane (macOS, beta) | ~1k | Ported, rendered on Linux from preview data; needs the macOS host (store listing, transcript reader, controller requests, live refresh) | 1.5 |
| Fun / Hex Microphone, Activity, HUD Lab (debug) | ~1.5k | Hex Microphone Settings section ported; Fun, Activity, HUD Lab deferred (custom vector drawing for waveforms, meters) | 4 (or drop debug panes) |
| Onboarding tour | `app_window/tour.rs` (1.1k) | Ported for every OS through `Capabilities`; macOS needs the host to feed permissions and practice observations and perform guide/relaunch requests; art is static until looping tracks exist | 1 |
| Text input | `text_input.rs` (1.7k) | Rune has caret, selection, undo, IME preedit; needs native IME protocols, focus rings, Tab order | Rune core |
| Dictation HUD, macOS | `dictation_indicator.rs` (1.2k), Metal, `NSPanel` | Window kind: borderless, transparent, click-through, non-activating, all spaces | 4 |
| HUD, Windows | `windows_indicator.rs` (151) + GPUI | Off under Rune: GPUI's HUD cannot share the process with Rune's loop. Needs an overlay window kind (topmost, no activation, click-through) | 2 |
| HUD, Linux | `linux_desktop.rs` (336), GTK + layer-shell | Stays GTK until Rune has a layer-shell surface | 3 later |
| Meeting capsules (macOS) | `meeting_watcher.rs` (UI ~1k) | Panel window kind, focus-free buttons, live waveform | 5 |
| Mac menu bar | `status_item.rs` (AppKit) | Stays native; Rune must run inside an app lifetime that outlives windows | 1 |
| Windows tray | `tray-icon` + `windows_app.rs` | Written in `windows_rune.rs` on the main thread beside `rune::App` (`keep_running`, `windowless` for `--hidden`, `AppHandle` open/quit); type-checked only | 0.5 |
| Windows chrome | `windows_ui.rs` (311) | Custom title bar hit-testing | 2 |
| Previews | macOS-only `capture-preview.sh` | Rune `--snap`/`--scene` on every OS (done for every Linux pane); goldens in CI on lavapipe | 1 |
| i18n | `desktop_i18n.rs` | Done: CJK fallback in Rune, translation test covers Rune copy | 0 |
| Design-token lint | GPUI-specific | Replace as described above | 0.5 |

Remaining after Linux: roughly 35–45 agent-days for Windows and macOS plus
the Rune core work below.

## Gaps in Rune, in the order to close them

1. **Modal, focus, and keyboard navigation**: dialog with a scrim and focus
   trap, a focus model for non-text controls, Tab order, arrow keys in menus
   and lists (in progress by Kit's `motion-focus` agent).
2. **Timers and looping animation**: `Tx::wake_after`, looping tracks for
   spinners, pulses, and indeterminate progress (`motion-focus`).
3. **App and window model**: app lifetime separate from windows,
   hide-on-close, and XDG activation landed as `rune::App`/`AppHandle`
   (Rune `a242f5a`, see [single-process.md](single-process.md#landed-on-rune-main-a9cde14-a242f5a-hex-pins-a242f5a)).
   Still open: several windows, window kinds (non-activating panel,
   transparent click-through overlay, Wayland layer-shell), and per-app
   minimum size.
4. **Keyboard**: key-up and modifier-only events, left/right modifier sides,
   physical key codes, Globe/Fn (macOS shortcuts).
5. **Raster images and custom vector drawing** (app icons, waveforms).
6. **Platform text input and clipboard**: native IME protocols; Wayland
   clipboard (`arboard` without `wayland-data-control` uses XWayland).
7. **Accessibility** (AccessKit) and **cursor shapes**.
8. **Native file dialogs** (portal on Linux, `NSOpenPanel`, `IFileDialog`).
9. **Scrolling polish**: visible scrollbars, scroll-into-view.

## Order to port

1. Linux: done except the parity list above; switch the default after the
   real-window checks.
2. Windows: the panes, tray, single instance, and in-process runtime are
   written and cross-compiled (`hex app --ui rune`, `docs/windows.md`) but
   have never run, on `rune::App`; next a person on Windows runs them, then
   chrome and title-bar hit-testing and the overlay HUD window; tray stays
   native; panes as Linux, in-process model preparation, Launch at login.
3. macOS: app lifetime with the status item, keyboard gap 4, the panel window
   kind, then the HUD, meeting capsules, permissions rows and the full tour,
   Commands, Meetings.
4. Delete GPUI: `desktop_ui.rs`, `design_tokens.rs`, `text_input.rs`,
   `motion.rs`, `desktop_transcription_picker.rs`, the GPUI fork patch, and the
   token lint; rewrite `docs/design-system.md` per
   [Design direction](#design-direction).

## What stays native per platform

- **Overlays:** Wayland layer-shell (GTK today), an X11 override-redirect popup,
  a topmost no-activate overlay window on Windows, a non-activating `NSPanel`
  on macOS. Rune draws into them once it can host those surfaces.
- **Menu bar and tray:** `NSStatusItem`/`NSMenu` and `tray-icon`. Linux has none.
- **Input methods, clipboard, and accessibility:** each platform's protocol.
- **Global shortcuts and paste:** unchanged (`linux_input`, `linux_portal`,
  `suppression`, `windows_input`, `paste`); the UI only records and shows them.

## Risks

- **Git dependency in a public repo.** Cargo fetches every Git dependency
  in `Cargo.lock`, optional or not. Rune is now public and pinned over HTTPS,
  so outside contributors can resolve it (checked with an unauthenticated
  `git ls-remote`, not with a clean `CARGO_HOME` build). Whether release CI
  and the Nix flake accept a Git dependency is still open.
- **API churn.** Kit's agents change Rune's `main` daily. Hex pins a commit,
  keeps its Rune code in `src/rune_ui*`, and moves the pin deliberately. A
  pending text-quality change will shift glyph rendering; the goldens' 12/255
  tolerance and 0.2% pixel budget may need rewriting when it lands. Rune's
  workspace ships a vendored copy under `crates/rune-light/vendor/rune`, which
  makes Cargo warn about a duplicate `rune` package.
- **Unproven platform shells.** Rune's Linux window has had one short check;
  Windows has only been cross-compiled (`cargo xwin clippy` on apollo; see
  `docs/windows.md`), never run; the macOS window kinds Hex needs do not exist yet.
- **Two front ends at once.** Until a platform switches, every pane change must
  land in GPUI and Rune. Shared helpers live in `app_window` (pure functions);
  keep moving behavior there rather than copying it.
- **Windows lockfile.** Resolving Rune for Windows put `gpu-allocator` on
  `windows` 0.57 (shared with GPUI's `sysinfo`), which breaks `wgpu-hal`'s
  DX12 backend; `Cargo.lock` pins `gpu-allocator`'s `windows` to 0.62.2 by
  hand. A `cargo update` that re-resolves it brings the error back.
- **Pre-existing on `f8774a5`:** `linux_app::tests::malformed_settings_cannot_start_even_after_dismissing_the_error`
  fails on Linux without these changes, and
  `dictation_processor::tests::service_discovery_fallback_shares_deadline_and_observes_cancellation`
  is timing-sensitive (it failed once under a parallel full run and passed
  three times alone).
