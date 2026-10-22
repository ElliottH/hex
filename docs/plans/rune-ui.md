# Port Hex's Desktop UI From GPUI To Rune

**Status:** Started, experimental, on the `rune-ui` branch only. The Linux
Settings pane runs on Rune behind `--features rune-ui` and `hex app --ui rune`;
GPUI stays the default everywhere. Rune, `rune-kit`, and
`rune-theme-opencode` are pinned to a commit on Rune's `main`
(`anomalyco/rune`, private; `1177fbd`). Do not merge to `main` until the
dependency questions under [Risks](#risks) are settled.

**Goal:** one Rune front end for Hex on macOS, Windows, and Linux, replacing
about 29k lines of GPUI (`app_window.rs` and its OS modules, `desktop_ui.rs`,
`design_tokens.rs`, `text_input.rs`, `desktop_transcription_picker.rs`,
`motion.rs`, the indicator and meeting capsules, and the GPUI parts of
`linux_app.rs`, `windows_app.rs`, and `windows_ui.rs`).

## Design direction

The Rune front end uses **OpenCode's design language through Rune's component
library**, not Hex's current GPUI design system. Controls come from `rune-kit`
(generic components with `@opencode/ui`'s names: `Button`, `Switch`,
`SegmentedControl`, `Select`, `Menu`, `TextInput`, …), and every colour,
elevation, radius, type style, and motion token comes from the OpenCode theme
(`rune_theme_opencode::kit(dark)`), which is `@opencode/ui`'s tokens as data.
Theming stays a Rune feature: Hex could switch themes without touching a
view. Hex keeps its information, sections, copy, and behavior; only the look
and layout follow the kit.

What that means for Hex's existing rules:

- `docs/design-system.md` describes the GPUI front end and stays authoritative
  for it while GPUI ships. It does not govern `src/rune_settings*`. When Rune
  becomes the default, replace it with a short Hex page that points at
  `rune-kit` and the OpenCode theme and lists only Hex-specific pieces
  (keycaps, shortcut recorder, listener card) and copy rules, which still
  apply (sentence case, "Hex", one accent action per screen, descriptions
  without full stops).
- The token lint (`design_tokens::tests::desktop_sources_use_tokens_instead_of_raw_styles`)
  matches GPUI calls (`rgb(0x…)`, `.text_size(`, `.rounded(px(`), so it neither
  applies to nor flags Rune code. The Rune equivalent: views name kit tokens
  (`p.k.c.*`, `p.k.ty.*`, `p.k.radius.*`, `p.k.elevation.*`) or kit
  components; the only raw style values are the row and keycap metrics in
  `src/rune_settings/controls.rs`. Add that as a lint over
  `src/rune_settings/` once a second Rune screen exists, and delete the GPUI
  lint with the last GPUI pane.
- Hex gains a light appearance for free (Rune follows the system), so every
  Rune screen gets light and dark goldens.
- Danger, success, and accent roles come from the theme's state colours
  (`k.c.danger`, `k.c.success`, `k.c.text_accent`); Hex no longer derives any.
- The one main action per screen uses the kit's contrast button (OpenCode's
  primary look); everything else uses the neutral button.

## What this pass did

```text
src/rune_settings.rs           state, service bridge, view, scenes   (~1.4k lines)
src/rune_settings/controls.rs  settings rows, cards, select menus,
                               segmented, buttons, keycaps, tag, note,
                               text field on rune-kit                (~360)
src/rune_settings/tests.rs     10 runtime tests + 4 GPU goldens      (~310)
tests/golden/rune-settings-*.png
```

- **Same service contract.** It is a `linux_service::Client` exactly like the
  GPUI Settings process: it saves `settings.json` (`AppSettings::save`) before
  adopting an edit, then sends `ReloadSettings`; it sends `Desktop(Start/StopListening)`,
  `CaptureShortcutFor`, `SetShortcut`, `CancelShortcut`, `ChooseModel`, and
  `Desktop(RestartIntoUpdate)`. Service settings are adopted only when the
  service's copy changes, so a stale snapshot never reverts a fresh edit.
  `Client::with_waker` (new) wakes Rune's on-demand redraw loop when a snapshot
  arrives.
- **Re-skinned on `rune-kit` and the OpenCode theme** (second pass): switches,
  segmented controls, buttons, and the text field are kit components; the
  model, microphone, and language choices are kit `Select` fields whose
  options open in an anchored kit `Menu` above the page (outside presses
  close it) instead of inline rows; cards use the kit's raised elevation;
  Inter and IBM Plex Mono come from the theme crate. Behaviour, keys, scenes,
  and the service contract are unchanged; the microphone-mode test taps
  `microphone-mode-control/segment1` (the kit's segment key).
  [light](rune-ui/hex-rune-portal-light.png),
  [dark](rune-ui/hex-rune-portal-dark.png),
  [menu open + confirmation](rune-ui/hex-rune-menus-light.png),
  [commands](rune-ui/hex-rune-commands-dark.png),
  [recording a shortcut](rune-ui/hex-rune-capturing-dark.png),
  [disabled](rune-ui/hex-rune-disabled-light.png).
- **Rows ported:** Status (listener state, hint, last failure, operation and
  connection errors, Enable/Disable), Desktop shortcuts (portal grants or the
  `hex` commands to bind), Local transcription (model menu for the current
  language plus preparation progress and errors), Microphone, Recognition
  hints, Shortcut mode, Dictation shortcut recorder, While dictating,
  Microphone mode with its confirmation, Double-tap to lock / only, Recording
  indicator, Terminal paste, Paste last dictation (+ Clear), Enable on launch,
  Interface language, Sound volume (with tone preview), Software updates.
- **Not ported:** Modes, Voice Action, History, Meetings, the tour, and the
  full transcription picker (language choice, model cards, downloads UI). The
  model menu lists `choices_for_runtime` for the current language only.
- **Run it:** `cargo run --release --features rune-ui -- app --ui rune`.
  Headless: `hex app --ui rune -- --snap out.png --scene portal --theme light`
  (scenes: `live`, `portal`, `commands`, `disabled`, `capturing`, `chinese`,
  `menus`). `--scene live` waits for the running service's first snapshot, so
  `--snap` shows real state and `--script` drives real edits without a window.
- **Checks:** `cargo test --features rune-ui rune_settings` (10 headless
  runtime tests need no GPU; 4 goldens skip without an adapter;
  `HEX_UPDATE_GOLDENS=1` rewrites them). Goldens for the re-skin were written
  on apollo (RADV).

### Observed on Linux (apollo: Arch, niri, AMD RADV, scale 2)

- Rune's `examples/settings` and `examples/opencode` built in 35 s and every
  `--snap` scene rendered correctly offscreen through Vulkan
  ([settings](rune-ui/rune-example-settings-dark.png),
  [opencode](rune-ui/rune-example-opencode-session.png)). One real-window check
  of `examples/settings` on niri rendered correctly at scale 2
  ([crop](rune-ui/rune-example-window-niri.png)); the window had no `app_id`
  (fixed on Rune `main`). No further windows were opened.
- Mesa lavapipe (software Vulkan on the agent host) renders `examples/settings`
  pixel-identically to RADV at the goldens' tolerance (0 of 1.29 M pixels over
  12/255), so the goldens can run in CI on lavapipe.
- Creating one wgpu device per test from parallel test threads crashed
  (SIGSEGV) under RADV; the goldens now share one device behind a mutex.
- Hex's Rune Settings against the real `hex.service` on niri shows the portal's
  grants (F12, Ctrl+Alt+V) and the service's last error exactly as GPUI does
  ([live](rune-ui/hex-rune-live-real-service.png),
  [GPUI reference](rune-ui/gpui-settings-linux.png)). These live captures and
  the motion strip predate the kit re-skin (first pass, Rune's neutral look);
  from an SSH session the service now reports that it belongs to another
  desktop session, so they were not re-taken.
- End to end against an isolated service (`HEX_APPLICATION_SUPPORT_DIR`, no
  display, listener off): a headless script toggled Terminal paste, While
  dictating, and Recording indicator; `settings.json` and the service's
  snapshot both reported the new values
  ([after](rune-ui/hex-rune-live-after.png)).
- Motion: a 1.4 s scripted sequence shows the segmented indicator sliding,
  switches springing, and a removed row exiting while the rows below reflow
  ([strip](rune-ui/hex-rune-motion-strip.png)); frame CPU p50 0.15 ms.
- **Chinese and Japanese render as tofu** ([zh](rune-ui/hex-rune-chinese-tofu.png)):
  Rune embeds Inter only and has no font fallback or system fonts.

Needs a person to look (no windows were opened on apollo after the first
check): window decorations and resize on niri and on X11, pointer cursor
shapes, keyboard focus and IME in the hints field, physical shortcut recording
(F-keys, Super), wheel/trackpad scrolling feel, fractional scaling, and
closing the window while the service keeps running.

## Rune changes made (now on Rune `main`)

| Change | Why |
| --- | --- |
| `rune::set_app_id(id)` → winit `with_name` on Wayland/X11 | niri reported `app_id: None`; desktops need it to match `hex.desktop`, icons, and window rules |
| `Key::Function(1..=24)` mapped from winit | Hex records F-key shortcuts (apollo's dictation key is F12); Rune dropped them |
| `Mods::logo` (Super/Windows) | Linux and Windows shortcuts use Super; Rune only reported Ctrl/Alt/Shift |
| Script `key` events accept `F1`–`F24` and `"logo": true` | so headless scripts can drive the above |
| `rune-kit` + `rune-theme-opencode` | the component library and OpenCode theme the re-skin uses |
| CSS box shadows, gradients, outlines; `El::anchored` overlays that follow scrolled anchors | the kit's elevation tokens and Settings' menus over a scrolling page |

The Linux fixes landed on `main` as their own commit (`6d87e63`, cherry-picked
from `hex-port` `13d36b0`); `hex-port` is no longer used. An `AppSpec` field
would be the cleaner final shape for the app id; a global setter avoided
breaking Kit's examples while `main` moves.

## Every GPUI surface and what it needs

Sizes are current GPUI lines. Effort is focused agent-days with review,
assuming the Rune gaps listed beside each are closed first.

| Surface | Today | Rune needs | Effort |
| --- | --- | --- | --- |
| Settings pane (all OSes) | `render_settings` + helpers in `app_window.rs` | Done on Linux; macOS adds permissions rows, Launch at login, Dock icon, Meetings and Hex Microphone sections; Windows adds its caption | 2–3 |
| Window shell: sidebar, navigation, pane scaffold, pane cross-fade | `app_window.rs`, `desktop_ui.rs` | Image/SVG icons (have SVG), routing between panes, a min window size per app (Rune hardcodes 420×400) | 2 |
| Transcription picker (modal) | `desktop_transcription_picker.rs` (475) | Modal sheet + scrim layer, anchored popovers, progress bars, keyboard navigation | 3 |
| Modes (list + editor, app picker, transformations, corrections) | ~2.5k lines in `app_window.rs` | Multi-line text editing (have), focus/blur events, raster app icons (`application_catalog`), context menus, native file dialog, drag reorder, virtualized lists (have) | 8–10 |
| Voice Action | ~500 | Text fields, model menu (popover), OpenCode availability states | 2 |
| History | ~250 + search | Selectable read-only text, copy, search field, virtual list | 2–3 |
| Commands (macOS) | ~400 | Catalog list, status rows | 2 |
| Meetings pane (macOS, beta) | ~1k | Transcript reader, rename/delete confirmation, overflow menus | 5 |
| Fun / Hex Microphone, Activity, HUD Lab (debug) | ~1.5k | Custom vector drawing (waveforms, meters), sliders (built in examples) | 4 (or drop debug panes) |
| Onboarding tour | `app_window/tour.rs` (1.1k) | Page transitions (have springs), hero keycaps, permission polling timers, step dots | 4 |
| Text input | `text_input.rs` (1.7k) | Rune has caret, selection, undo, IME preedit; needs native IME protocols (winit IME is incomplete), focus rings, Tab order, multi-line height limits | Rune core |
| Dictation HUD, macOS | `dictation_indicator.rs` (1.2k), Metal, non-activating `NSPanel` | Second window kind: borderless, transparent, click-through, non-activating, all spaces; glass optional (`rune-fx`) | 4 |
| HUD, Windows | `windows_indicator.rs` (151) + GPUI | Overlay window: topmost, no activation, click-through, per-monitor DPI | 2 |
| HUD, Linux | `linux_desktop.rs` (336), GTK + layer-shell | Stays GTK/layer-shell until Rune has a layer-shell surface; then a Rune-drawn layer surface | 3 later |
| Meeting capsules (macOS) | `meeting_watcher.rs` (2.4k, UI share ~1k) | Same panel window kind as the HUD, focus-free buttons, live waveform | 5 |
| Mac menu bar / status item | `status_item.rs` (AppKit) | Stays native `NSStatusItem`/`NSMenu`; needs Rune to run inside an app lifetime that outlives windows | 1 |
| Windows tray | `tray-icon` + `windows_app.rs` | Stays native; same app-lifetime need | 1 |
| Windows chrome | `windows_ui.rs` (311), caption row, Segoe glyphs | Custom title bar hit-testing (drag, min/max/close regions), Mica optional | 2 |
| Previews and `capture-preview.sh` | macOS-only window capture | Replace with Rune `--snap`/`--scene` scenes on every OS (done for Settings); goldens in CI on lavapipe | 1 |
| i18n | `desktop_i18n.rs`; `tr` works unchanged | CJK font fallback (blocker for zh/ja), the translation test already scans `rune_settings.rs` | Rune core |
| Design-token lint | GPUI-specific | Replace as described in [Design direction](#design-direction) | 0.5 |

Total for the panes, shell, and overlays after the core gaps close: roughly
60–75 agent-days, plus the Rune core work below.

## Gaps in Rune, in the order to close them

1. **Font fallback and system fonts** (blocker for zh, ja, and emoji). Parley
   and fontique support fallback; Rune pins one family per style.
2. **Popover and modal layer**: anchored menus, dropdowns, context menus,
   tooltips, outside-press dismissal, and z-order above later siblings now
   exist (`El::anchored`, `rune_kit::root`); still missing: modal sheets with a
   scrim that blocks the page, focus trapping, keyboard navigation in menus,
   and closing an open menu when its anchor scrolls.
3. **Timers and continuous animation**: `Tx::wake_after`, looping tracks for
   pulses and indeterminate progress. Hex relies on the service's 4 Hz
   snapshots to clear "Saved" and has no recording pulse.
4. **Focus**: programmatic focus, blur/commit events, focus rings, Tab order.
   Recognition hints save only on Enter.
5. **Keyboard**: key-up and modifier-only events, left/right modifier sides,
   physical key codes (macOS keys shortcuts by layout keycode), Globe/Fn.
   Required for macOS modifier-only shortcuts; Linux needs only what
   the Linux fixes on Rune `main` added.
6. **App and window model**: an app lifetime separate from windows, several
   windows, hide-on-close, close-request hooks, window kinds (non-activating
   panel, transparent click-through overlay, Wayland layer-shell), per-app
   minimum size, title changes. `AppSpec` hooks are `fn` pointers, so state
   arrives through scenes or globals rather than closures.
7. **Raster images** (app icons, menu-bar art) and **custom vector drawing**
   (waveforms, meters).
8. **Platform text input**: native IME protocols (`NSTextInputClient`, TSF,
   text-input-v3) instead of winit's; Wayland clipboard (Rune's `arboard` is
   built without `wayland-data-control`, so it uses X11/XWayland only).
9. **Accessibility** (AccessKit is planned, not wired) and **cursor shapes**
   (Rune never sets the pointer cursor).
10. **Scrolling polish**: visible scrollbars, horizontal scroll,
    scroll-into-view for focused controls and errors.

## Order to port

1. Land Rune's font fallback, popover/modal layer, timers, and focus (1–4).
   Keep the Linux Settings pane as their test bed and add goldens per feature.
2. Linux: window shell with navigation, then Voice Action, History, and Modes,
   then the transcription picker; make `--ui rune` the Linux default once
   every Linux pane exists and a person has checked the "needs a person to
   look" list on Wayland and X11.
3. Windows: chrome and title-bar hit-testing, overlay HUD window, tray stays
   native. Same panes, plus Launch at login.
4. macOS: app lifetime with the status item, keyboard gap 5, the
   non-activating panel window kind, then the HUD, meeting capsules,
   permissions rows, Commands, Meetings, and the tour.
5. Delete GPUI: `desktop_ui.rs`, `design_tokens.rs`, `text_input.rs`,
   `motion.rs`, the GPUI fork patch, and the token lint; rewrite
   `docs/design-system.md` per [Design direction](#design-direction).

## What stays native per platform

- **Overlays:** Wayland layer-shell (GTK today), an X11 override-redirect popup,
  a topmost no-activate overlay window on Windows, a non-activating `NSPanel`
  on macOS. Rune draws into them once it can host those surfaces.
- **Menu bar and tray:** `NSStatusItem`/`NSMenu` and `tray-icon`. Linux has none.
- **Input methods, clipboard, and accessibility:** each platform's protocol
  (Rune's own plan says the same).
- **Global shortcuts and paste:** unchanged (`linux_input`, `linux_portal`,
  `suppression`, `windows_input`, `paste`); the UI only records and shows them.

## Risks

- **Private dependency in a public repo.** Cargo fetches every Git dependency
  in `Cargo.lock`, optional or not: with Rune unreachable even the default GPUI
  build fails (verified with an empty `CARGO_HOME`). Release CI, the Nix flake
  (needs a pinned hash and a fetchable source), and outside contributors break
  until Rune is public, vendored, or moved out of Hex's lockfile. This is why
  the port stays on the branch.
- **API churn.** Kit is changing Rune's `main` daily (layout measurement,
  growth, text flights landed during this pass). Pin a commit, keep Hex's Rune
  code in `src/rune_settings*`, and move the pinned commit deliberately. Rune's
  workspace also ships a vendored copy under `crates/rune-light/vendor/rune`,
  which makes Cargo warn about a duplicate `rune` package. Hex now pins a
  commit on `main` (not a branch) so Kit's daily changes land only when the
  pin moves.
- **Unproven platform shells.** Rune ran on Kit's Macs; this pass is its first
  Linux run and only one short window check. Windows is untested; macOS window
  kinds Hex needs do not exist yet.
- **Two front ends at once.** Until a platform switches, every Settings change
  must land in GPUI and Rune. Keep the Rune side to panes already ported and
  share behavior helpers (conflict checks, key names) rather than copying.
- **CJK and IME** are product blockers, not polish: Hex ships Chinese and
  Japanese interfaces.
- **Pre-existing:** `linux_app::tests::malformed_settings_cannot_start_even_after_dismissing_the_error`
  fails on Linux at `f8774a5` without these changes.
