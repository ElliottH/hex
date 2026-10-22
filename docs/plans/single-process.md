# One Hex Process Per User, Hosting Rune Windows

**Status:** design for review on `rune-ui`. Nothing here is implemented.
It depends on the Rune front end (`src/rune_ui*`,
[`rune-ui.md`](rune-ui.md)) and on Rune core changes that need coordinating
with Kit. Never merge this to `main` ahead of the Rune port.

**Goal:** Hex is one long-lived process per user and data root, on every OS:

- Launching `hex` when no instance runs starts it in the background:
  listener, models, HUD.
- Launching it again tells the running process to open or focus its window,
  then exits.
- CLI controls (`hex dictation …`, `hex voice-action …`, `hex paste-last`,
  `hex status`, `hex stop`) forward to the running process.
- Closing the window leaves the process running.

The engine is a library. Rune is the only UI. GPUI is legacy and is not
ported further. It is deleted at the end of the port and gets no new
patches.

```ts
hex (no instance)    -> become the instance (or the supervisor starts it) -> open window unless --background
hex (instance)       -> forward Activate{activation token} -> exit 0
hex dictation start  -> forward Shortcut -> exit 0 | non-zero without an instance (never starts one)
close window         -> Rune window gone (or hidden); runtime, listener, HUD, models stay
quit                 -> explicit: tray / menu-bar Quit, "Quit Hex" in the window, `hex stop`
```

## Today

### Linux: two processes

- **`hex service`** is `linux_app::run_service`, under the `hex.service`
  user unit.
  - It owns `LinuxDesktopHost` (listener, portal and X11 shortcuts, models,
    updates, History writes, paste) and the GTK HUD thread
    (`linux_desktop.rs`).
  - Its 20 ms loop serves `linux_service::Server` on `service.sock` in the
    data root. The socket is guarded by `instance::acquire("service")`, a
    peer-UID check, and bounded frames.
- **The window is a separate client process.** That is GPUI's `hex app`, or
  Rune's `hex app --ui rune` (`rune_ui::run`, `src/linux.rs`).
  - Rune's `Ui` holds `Service::Live(Box<Client>)`, which
    `setup` creates with `Client::with_waker`.
  - The `Client` thread polls `Snapshot` every 250 ms and calls Rune's
    waker on each update. The window sends `Desktop(_)`,
    `CaptureShortcutFor`, `SetShortcut`, `CancelShortcut`, `ChooseModel`,
    `CancelModel`, and `ReloadSettings`, the last after it writes
    `settings.json`.
  - Closing the window ends that process.
- **Lifecycle and launchers.** `hex start|stop|restart` drive `systemctl
  --user` after writing an owner-only `session.env`. The autostart entry runs
  `hex start`, and the desktop entry runs `hex app`. Both come from
  `scripts/install-linux-release.sh`, which also writes a `Type=simple`,
  `PartOf=graphical-session.target`, `Restart=on-failure` unit. The Home
  Manager module (`nix/hm-module.nix`) writes the same unit.
- Linux was one process with a GTK tray until `9a11b5f`. A GTK thread beside
  a GPU UI loop in one process is proven.

### macOS: one process (GPUI today)

- `Hex.app` is an `LSUIElement` menu-bar app (`Command::App` →
  `meeting_watcher::run`).
- LaunchServices keeps it single-instance. A relaunch arrives as
  `applicationShouldHandleReopen`, which GPUI calls `on_reopen`, and Hex
  answers with `app_window::open_or_focus`.
- `status_item.rs` is plain AppKit (`objc2`, no GPUI), and the event tap
  runs on its own `CFRunLoop` thread (`suppression.rs`).
- `local_api` (loopback HTTP, owner-only discovery, bearer token) already
  forwards dictation control (`POST /dictations`) and the debug-only
  `/dev/control`.
- The GPUI-specific pieces are the window, the Metal/GPUI HUD
  (`dictation_indicator`), the meeting capsules, and the GPUI `Application`
  that owns the run loop.

### Windows: one process (GPUI today)

- `windows_app::open` runs GPUI, the tray (`tray-icon`), a GPUI HUD window,
  and `WindowsDesktopHost` on the UI thread.
- Single instance is a named auto-reset event
  (`Local\com.kitlangton.hex.app`). A relaunch calls
  `AllowSetForegroundWindow` plus `SetEvent` and exits. A `--hidden` login
  launch only exits.
- Close hides the window. There is no CLI forwarding.

## The shape on every OS

The **runtime** owns all Hex state and all non-UI work, on threads it owns.
The **Rune app** owns the main thread, the window or windows, and the native
chrome: the menu-bar item, the tray, and on Linux nothing. An in-process
**`HostHandle`** connects the two. It replaces the socket `Client` and keeps
the same vocabulary:

```rust
// In Hex (shared), not Rune.
#[derive(Clone)]
pub struct HostHandle { /* SyncSender<HostRequest>, Arc<ArcSwap<Published>>, Waker list */ }
impl HostHandle {
    pub fn send(&self, request: HostRequest) -> Result<()>;   // today's Request minus transport-only ones
    pub fn state(&self) -> Arc<Published>;                  // DesktopSnapshot + settings + editing + error
    pub fn subscribe(&self, wake: rune::Waker) -> Subscription; // runtime calls wake after publishing
}
```

- **The runtime thread.** It runs `LinuxDesktopHost` (later
  `WindowsDesktopHost` and the macOS runtime) in its own loop, the
  `run_service` loop minus the socket. After every change it publishes an
  `Arc<Published>` and calls the subscribers' wakers.
- **Why not host it on the UI thread.** Rune's `poll` runs only on a
  window's frames, and Hex's work must continue with zero windows. Keeping it
  off the UI thread also means no blocking host call can stall rendering.
- **The window.** `rune_ui` keeps `Service::{Live, Fixture}`, with `Live`
  holding a `HostHandle` in place of the `Client`. `poll` reads `state()`,
  so the pane code does not change. Snapshots and goldens keep using
  `Fixture`.
- **Shortcut commands from the CLI** go from the socket thread straight to
  the listener's `SyncSender<ShortcutCommand>`, never through the UI or the
  runtime loop, because keybinding hold-to-dictate is timing-sensitive.

## What Rune must gain

### Landed on Rune `main` (`a9cde14`, `a242f5a`; public as `7eea3a3`, which Hex pins)

Requirements 1–5 below landed as a smaller API than the proposal: one window
per app, built on the existing `AppSpec`. It is documented in Rune's
`crates/rune/README.md` ("App lifetime") and exercised by
`examples/background`.

```rust
let app = rune::App::new(spec())
    .init(move || Ui::live(host))  // first state from a closure: the HostHandle, no globals
    .args(Vec::new())              // Hex owns argv; Rune parses only what it's given
    .windowless()                  // or start open; implies keep_running
    .keep_running()                // closing keeps the process, state, and loop
    .on_close(|ui, tx| ..);        // flush drafts; tx.quit() quits instead
let handle = app.handle();         // AppHandle<Ui>: Clone + Send + Sync, usable before run
// socket thread:  handle.activate(token) | handle.open() | handle.quit()
// runtime thread: handle.update(|ui, tx| ui.adopt(snapshot))   // wakes the loop, no polling
app.run();                         // main thread; returns after quit
```

- The `Runtime<S>` belongs to the app, not the window. Closing hides the
  window on macOS, Windows, and X11, and destroys it on Wayland. Reopening
  shows the same state, settled. `release_gpu_when_closed()` also drops the
  GPU device while closed. There is no `on_closed(S)` hand-back, because the
  state never leaves.
- Updates run on the main thread inside the loop, in call order, even with no
  window open. A Hex update may also create native main-thread objects such as
  the status item or the tray. This replaces the proposed `start` and
  `on_message`.
- Tokens: the first window takes the launcher's `XDG_ACTIVATION_TOKEN` or
  `DESKTOP_STARTUP_ID` and clears both from the environment. A new window gets
  the token through `with_activation_token`. An open Wayland window is
  activated through `xdg_activation_v1` on winit's display. Handlers can call
  `Tx::close_window()` and `Tx::quit()` (the "Quit Hex" row).
- Observed in a headless sway on apollo: a forwarded token focuses an open
  window and a recreated one, and without a token the window is only marked
  urgent. Unverified on KWin, Mutter, niri, macOS, and Windows.
- Still open: several windows (Kit's `rooms-bindings` agent plans
  multi-window), window kinds (item 8), the macOS activation policy, and
  per-app minimum size.


Rune on `origin/main` (`8b37ced`, fetched 2026-10-22) is one winit 0.30
window per process:

- `crates/rune/src/window.rs`: `App { spec, args, live: Option<Live<S>>,
  proxy }`. It creates the single window in `resumed`, and
  `WindowEvent::CloseRequested => event_loop.exit()`.
- Each window builds its own `Gpu`, and its `Runtime<S>` owns the state from
  `AppSpec::init: fn() -> S`.
- `crates/rune/src/tooling.rs`: `rune::run(spec)` parses process argv
  (`--snap`, `--scene` …).
- `AppSpec`'s hooks are plain `fn` pointers.

Already there:

- `Waker` (`runtime.rs:60`, an `EventLoopProxy<()>` redraw) for other
  threads.
- `set_app_id` (`6d87e63`), which maps to winit `with_name` so Wayland's
  `app_id` and X11's `WM_CLASS` match `hex.desktop`.

Gap 3 in [`rune-ui.md`](rune-ui.md#gaps-in-rune-in-the-order-to-close-them)
is exactly this work. Everything below is shell work. winit 0.30.13 already
supports what is needed underneath (verified in its source):

- A windowless `ApplicationHandler` with `ControlFlow::Wait`.
- `EventLoopProxy<T>` with user events.
- `WindowAttributesExtStartupNotify::with_activation_token`. On Wayland it
  calls `xdg_activation_v1.activate` on the new surface. On X11 it sets the
  startup id and removes the startup notification.
- `ActivationToken::from_raw`.
- `Window::set_visible` and `focus_window`.
- macOS `with_activation_policy(Accessory)` and `with_default_menu(false)`.
  winit also promises never to install an `NSApplicationDelegate`, which
  leaves the delegate to Hex.

### Proposed API (shell-agnostic; winit first, own shells later)

```rust
pub struct AppConfig {
    pub app_id: &'static str,
    pub fonts: &'static [&'static [u8]], pub families: (&'static str, &'static str),
    pub keep_running_without_windows: bool,              // never exit when the last window closes
    #[cfg(target_os = "macos")] pub activation_policy: ActivationPolicy, // Accessory for Hex
}

/// Runs the event loop on the calling (main) thread without parsing argv. `start` runs once
/// on the main thread when the loop is live: create the status item / tray, open a first
/// window if wanted. Returns when `AppProxy::quit` is called.
pub fn run_app<M: Send + 'static>(
    config: AppConfig,
    start: impl FnOnce(&mut AppCx<M>) + 'static,
    on_message: impl FnMut(&mut AppCx<M>, M) + 'static,
);

#[derive(Clone)] pub struct AppProxy<M> { /* EventLoopProxy<Msg<M>> */ }  // Send + Sync
impl<M> AppProxy<M> { pub fn send(&self, m: M); pub fn quit(&self); }

pub struct AppCx<'a, M> { /* &ActiveEventLoop + window table + shared Gpu */ }
impl<M> AppCx<'_, M> {
    pub fn proxy(&self) -> AppProxy<M>;
    pub fn open<S: 'static>(&mut self, spec: WindowSpec<S>, opts: OpenOptions) -> WindowId;
    pub fn window(&mut self, id: WindowId) -> Option<WindowRef<'_>>;  // focus(token), show, hide, close
    pub fn windows(&self) -> impl Iterator<Item = WindowId>;
    pub fn quit(&mut self);
}

pub struct WindowSpec<S> {   // AppSpec minus app-wide fields, with closures instead of fn pointers
    pub title: &'static str, pub size: (f32, f32), pub min_size: (f32, f32),
    pub init: Box<dyn FnOnce() -> S>,                   // captures the HostHandle
    pub view: fn(&S, &mut Cx<S>) -> El,
    pub setup: Option<Box<dyn FnOnce(&mut S, Waker)>>, pub poll: Option<fn(&mut S, f64, &mut Tx) -> bool>,
    pub on_key: Option<fn(&mut S, &KeyEvent, &mut Tx) -> bool>,
    pub on_close: CloseBehavior,                        // Destroy | Hide | Ask(fn(&mut S) -> CloseBehavior)
    pub on_closed: Option<Box<dyn FnOnce(S)>>,         // cancel an uncommitted shortcut capture, drop the subscription
}
pub struct OpenOptions { pub activation_token: Option<String>, pub visible: bool, pub kind: WindowKind }
pub enum WindowKind { Normal /* later: Panel, Overlay, LayerShell(Edge) */ }
```

Requirements behind that shape, in priority order:

1. **App lifetime separate from windows.** With
   `keep_running_without_windows` set, closing the last window must not exit
   the loop. Exit happens only through `quit`. Today's `run(spec)` stays as a
   one-window convenience wrapper, so the gallery, the examples, and
   `--snap` keep working.
2. **Open, focus, and close on demand from other threads.** Other threads
   send through `AppProxy::send` (an `EventLoopProxy` user event). Windows
   can only be created inside a handler, which is the reason for
   `on_message(&mut AppCx, M)`. Several windows are tracked by `WindowId`
   (`App.live` becomes a map). Hex needs one normal window now, plus the HUD
   and the macOS capsules later.
3. **Hide vs close.** `CloseBehavior::Destroy` drops the surface and the
   `Runtime<S>`: Linux and Wayland, where a toplevel cannot be hidden.
   `Hide` calls `set_visible(false)` and keeps state, for Windows and macOS
   fast reopen. `on_closed` hands the state back so Hex can clean up.
4. **Activation token.**
   - On creation, pass `OpenOptions::activation_token` to
     `with_activation_token` (exists in winit).
   - For an existing window, add `WindowRef::focus(Some(token))`. winit
     0.30's Wayland `focus_window()` is empty
     (`platform_impl/linux/wayland/window/mod.rs:629`), and winit has no
     "activate this surface with this token" call. The Rune Linux shell has
     two options. The first is to bind `xdg_activation_v1` on winit's
     connection, through the raw display and surface handles, and call
     `activate(token, surface)`. The second, simplest for Hex, is to destroy
     and recreate the window with the token, since Linux destroys on close
     anyway.
   - On X11, `focus_window` plus a `_NET_STARTUP_INFO remove:` message.
5. **Argv-free entry and closures.** `run_app` must not call `Args::parse`,
   because Hex owns its CLI. `init` and `setup` become boxed closures so a
   window receives its `HostHandle` without globals.
6. **Shared GPU across windows.** Create the instance, adapter, device, and
   queue once per app, and per-window surfaces and renderers. Release the
   device after the last window closes (configurable), so an idle Hex holds
   no UI GPU memory beside the speech model.
7. **Native hooks on the main thread.**
   - `start` and `on_message` run on the main thread inside the loop. That
     is where Hex creates the `NSStatusItem` and its delegate, or the
     `tray-icon`.
   - `WindowRef::raw_window_handle()` gives Windows HWND style tweaks, as
     the HUD uses today.
   - macOS gets the activation policy and `default_menu = false`. Reopen
     stays Hex's own delegate method, which winit permits.
8. **Later window kinds.** All three are shell work, needed only when the
   HUDs move to Rune:
   - `Overlay`: transparent, click-through, no activation, topmost. For
     Windows this is `WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW`;
     for macOS a non-activating `NSPanel` on all spaces.
   - `Panel`: macOS meeting capsules.
   - `LayerShell(Edge)`: Wayland HUD. winit has no layer-shell, so this
     needs a smithay-client-toolkit surface in Rune's Linux shell. Until
     then the GTK HUD thread stays as it is.

Item 8 belongs to the HUD port, not to single process.

## Linux

### Single instance and forwarding: keep the socket (re-checked)

The instance exists to make the **data root** exclusive. The data root holds
`settings.json`, History, the listener lock, and the models, plus the
`HEX_APPLICATION_SUPPORT_DIR` roots that tests, Nix, and development use. A
lock plus a socket in that root keys both exclusivity and forwarding to
exactly it. Chromium's `ProcessSingleton` (and so Slack, Discord, and other
Electron apps) and Zed's Linux CLI listener do the same per data dir. The
existing socket already has owner-only permissions, peer-UID checks,
bounded frames, deadlines, stale-socket recovery, and
`scripts/test-linux-service.py`. Its protocol shrinks to four requests, with
`VERSION = 2`:

```rust
enum Request {
    Activate { activation_token: Option<String>, startup_id: Option<String>, session: Session, show: bool },
    Shortcut(ShortcutCommand),   // straight to the listener's sender
    Status,                      // Published snapshot as JSON for `hex status`
    Quit,                        // `hex stop` without systemd, or a custom data root
}
```

Rune does not change this choice. winit takes the activation token as a
plain string (`ActivationToken::from_raw`), so the token rides in
`Activate` with no D-Bus involvement.

**Rejected: a D-Bus well-known name** (GApplication's
`org.freedesktop.Application.Activate(platform-data{activation-token})`,
KDBusService, Tauri's Linux single-instance plugin).

- A bus name is per bus, not per data root, so isolated instances collide
  with the installed app.
- `DBusActivatable=true` requires renaming `hex.desktop` to the bus name.
  That changes the app id that the GlobalShortcuts portal
  (`linux_portal::APP_ID`, `Register("hex")`) and Rune's `set_app_id("hex")`
  use, and portals would re-ask for shortcuts.
- Hex speaks D-Bus through `gio`, and a gio-exported object needs a glib
  loop that Rune's winit loop does not provide.
- `org.freedesktop.Application` can be added later as a second adapter if
  Hex ever moves to a reverse-DNS app id.

**Also rejected:**

- An abstract-namespace socket: it is keyed by user, not by root, and any
  process in the network namespace can connect.
- A lock with signals: it can't carry a reply, a token, or an error.

### Process and threads

```ts
main thread     rune::run_app: window on demand (Destroy on close), AppProxy messages
runtime thread  LinuxDesktopHost loop (today's run_service minus socket), publishes Arc<Published>, wakes windows
socket thread   lock + service.sock; Activate -> AppProxy::send(Open{token}); Shortcut -> listener sender; Status/Quit
GTK thread      HUD (unchanged; Rune LayerShell later)
listener        linux_dictation worker + engine threads (unchanged)
```

- `Activate{show}` with no window open calls `cx.open(window_spec(handle),
  OpenOptions { activation_token, .. })`.
- With a window open, it calls `window.focus(token)`.
- `Activate` from a different desktop session keeps today's rule: restart
  through systemd, then Activate.
- Closing the window drops its `Ui`. `on_closed` sends `CancelShortcut` if
  that window started a capture, and drops the subscription.
- The editor lease, `Event::Disconnected`, and reconnect logic go away.
  There is one in-process window, and closing it is the only way to
  "disconnect".

### Launch and systemd

The launch logic for `hex`, `hex app`, `hex start`, and `hex --background`:

```ts
connect(socket) ok        -> Activate{token from XDG_ACTIVATION_TOKEN / DESKTOP_STARTUP_ID, session, show} -> exit
no instance, managed unit -> write session.env; systemctl --user start hex.service (Type=notify) -> Activate{token, show}
no instance, otherwise    -> become the instance here (custom data root, cargo run, non-systemd)
```

Under a managed install the long-lived process is always the systemd one,
which brings journald, `Restart=on-failure`, and
`PartOf=graphical-session.target` logout teardown. The launcher-started
`hex` only forwards. It reads the token from its own environment and must
not consume it itself. Both paths take `instance::acquire("service")` before
binding the socket, so a unit that loses a race exits 0 and does not trigger
a restart.

Unit and launcher changes (the installer, `nix/hm-module.nix`, and
`nix/checks.nix`):

- `Type=notify`, `NotifyAccess=main`, with `READY=1` written to
  `$NOTIFY_SOCKET` after the socket binds. This replaces the 5 s polling in
  `ensure_started`.
- Keep `ExecStart=… service`. The updater swaps the binary, never the unit,
  so `service` (background, no window) is permanent. `app` and `start` stay
  as aliases.
- Add the window's environment to `SESSION_KEYS`, because the Rune window
  now renders in the systemd process: `XCURSOR_THEME`, `XCURSOR_SIZE`,
  `XMODIFIERS`, `GTK_IM_MODULE`, `QT_IM_MODULE`, `XDG_DATA_DIRS`, `LANG`,
  `LC_*`. Keep clearing absent values.
- A crash restart opens no window. Only `Activate{show}` opens one.
- `hex.desktop`: `Exec=hex`, `StartupNotify=true`, and `Actions=quit` with
  `Exec=hex stop`. The autostart entry: `Exec=hex --background`.

### Code that moves or disappears (on `rune-ui`)

- **`linux_service.rs` → `linux_instance.rs`.**
  - Keep the lock, socket, peer check, frames, `systemctl`, session
    environment, and `Session`.
  - Delete `Client` (`new`, `with_waker`, polling, reconnect) and
    `Event::Disconnected`. Delete the requests `Snapshot`, `Desktop`,
    `CaptureShortcut*`, `CancelShortcut`, `SetShortcut`, `ChooseModel`,
    `CancelModel`, and `ReloadSettings`, which move to the in-process
    `HostRequest`.
- **`linux_app.rs`.**
  - `run_service` becomes `runtime::spawn(…) -> HostHandle`: the same
    loop, fed by a channel.
  - New `linux_app::run(show, token)`: bind the instance, spawn the runtime,
    start the socket thread, then `rune::run_app` with the Linux
    `CloseBehavior::Destroy`.
  - Delete `open_settings` (the GPUI Settings process) and
    `disconnect_editor`.
- **`rune_ui.rs`.** `Service::Live` holds the `HostHandle`. `setup`
  subscribes its waker. `ReloadSettings` after a save stays as a
  `HostRequest`, an in-process message instead of a socket round trip.
  Update its doc comment ("a client of `hex.service`").
- **`linux.rs`.** Route `None`, `App`, `Start`, and `--background` to the
  launch logic. `--ui gpui` goes away on Linux. `Service` becomes the
  instance without a window. `Status` becomes `Request::Status`. `Stop`
  becomes `systemctl stop`, or `Quit`.
- **`linux_updater::schedule_restart`.** Unchanged for managed installs.
- Add **"Quit Hex"** to the Rune window. Linux has no tray, so the only
  other exits are the desktop action and `hex stop`.

### Tests and docs

- `test-linux-service.py`: cover a second launch forwarding `Activate` and
  exiting 0, shortcut commands failing fast without an instance,
  stale-socket recovery, racing starts producing one instance, `Quit`
  draining, and v1 frames being rejected.
- Wayland paste and X11 dictation scripts: open the window through a second
  `hex`, close it, check the same PID keeps listening, then reopen it.
- Rune goldens: unchanged (`Fixture`).
- `AGENTS.md`:
  - Replace the `linux_service` and `linux_app` module entries.
  - Replace the invariant "Linux Settings owns no microphone… Client
    disconnect… Never replay…" with these:
    - One process per user and data root.
    - The window is optional, recreated on demand, and its close never
      stops capture, accepted work, or the HUD.
    - A relaunch forwards `Activate` with its token and exits; hidden
      launches never show the window.
    - The socket stays owner-only, peer-checked, and bounded.
    - Closing the window cancels only an uncommitted shortcut capture.
    - Under a managed install, systemd owns lifetime and restart.
- Also update `docs/linux.md` ("Background Service And Settings"),
  `docs/nix.md`, `docs/features/README.md` (`maintain.linux-service`,
  `linux.commands`), and `rune-ui.md`.

## Windows under Rune

Keep the named event. It already gives single instance, the
`AllowSetForegroundWindow` handoff, and hidden-launch no-op.

- **Main thread.** `rune::run_app`, with `CloseBehavior::Hide` (Windows can
  hide, and reopen is instant).
  - `start` creates the `tray-icon` on the main thread. winit's Win32
    message loop pumps that thread's messages, which `tray-icon` needs.
  - Tray and menu events go through `TrayIconEvent::set_event_handler` and
    `MenuEvent::set_event_handler` → `AppProxy::send`.
  - The `hex-relaunch` thread's `WaitForSingleObject` loop sends `Open`
    through the proxy, then calls `focus(None)`, which works because the
    relaunching process granted `AllowSetForegroundWindow`.
- **Runtime.** Move `WindowsDesktopHost` from `Rc<RefCell>` on the GPUI
  thread to the same runtime-thread and `HostHandle` shape as Linux. The
  low-level keyboard hook already runs on its own thread.
- **HUD.** Rune `WindowKind::Overlay`, or a normal Rune window with the
  current HWND style tweaks through `raw_window_handle()`, until Overlay
  exists.
- **CLI forwarding.** Not needed now. If wanted, use a per-user named pipe
  (`\\.\pipe\com.kitlangton.hex.<SID>`, owner-only DACL) with the same
  four-request protocol.

### Written on `rune::App` (cross-compiled only)

Rune's app lifetime landed (`a9cde14`/`a242f5a`, now `7eea3a3`) as
`rune::App` + `AppHandle` rather than the `run_app`/`AppCx` sketch above.
`hex app --ui rune` uses it in
[`windows_rune.rs`](../../src/windows_rune.rs) and
[`rune_ui/windows_host.rs`](../../src/rune_ui/windows_host.rs):

- The runtime thread owns `WindowsDesktopHost` and
  `TranscriptionPreparation`, takes the shared panes' requests over a
  channel, and publishes an `Arc<Published>` (listener, preparation,
  updates, settings), waking the window. `rune_ui` reaches it through the
  per-OS `host` module (`Backend::{connect, send, state, error}`), the
  same seam the Linux client sits behind. When Linux's `HostHandle` lands,
  both should share it.
- `rune::App::new(spec).on_close(..)` with `keep_running()`, or
  `windowless()` for `--hidden`. The tray is created on the main thread
  before `run` (winit's loop pumps its messages); its Show and the relaunch
  thread call `AppHandle::open`, Quit calls `AppHandle::quit`, and so does
  the runtime thread on Ctrl-C or a restart into an update. `open` then
  joins the runtime, which stops the listener. Without a tray the app keeps
  Rune's default, close quits.
- `on_close` flushes text drafts and stops a shortcut recording.
- Rune exposes no window handle yet, so the window icon is set by finding
  the window by title on the main thread before a frame.
- Shortcut capture stays in the window (keyboard polling with the hook
  suppressed), not on the runtime.
- No HUD: GPUI's HUD window needs GPUI's own event loop on the main thread,
  and Rune has no overlay window kind.

## macOS under Rune

LaunchServices keeps single instance, and the status item stays native.

### Written on `rune::App` (branch `rune-ui-macos`, compiled and launched on hailmary)

[`macos_rune.rs`](../../src/macos_rune.rs) and
[`rune_ui/macos_host.rs`](../../src/rune_ui/macos_host.rs); `hex app` runs
it when built with `--features rune-ui` (`--ui gpui` keeps GPUI):

- **Main thread.** `rune_ui::app().accessory()`, windowless (or open at
  launch under GPUI's rule: setup or onboarding unfinished, or Show Dock
  icon) and kept running. `App::accessory` (Rune `50d8d91`) builds winit's
  loop with the Accessory policy, no default menu, and no activation at
  launch.
  - winit 0.30 installs its own `NSApplicationDelegate` and panics if it
    is replaced, so Hex answers Dock and Finder reopen with an
    `NSAppleEventManager` handler for `kAEReopenApplication` instead.
  - The first `AppHandle::update` closure runs on the main thread once the
    loop is live and installs Sparkle, the `NSStatusItem`, Hex's own
    Hex/File/Window menu (⌘, ⌘H ⌘Q ⌘W ⌘M; no Edit menu, so ⌘C/⌘V reach
    Rune's fields), the reopen handler, and the Metal HUD
    (`dictation_indicator`'s NSPanel, unchanged). A 16 ms `NSTimer` in the
    common run-loop modes then drains the HUD, status-item, meeting, and
    developer-control channels with no window open.
  - Opening the window shows the Dock icon and activates Hex; Rune's
    close hides it and `on_close` restores the saved Dock policy.
- **Runtime thread.** Owns `TranscriptionPreparation` (the menu bar's
  model list asks for installed models only), commits prepared models to
  `settings.json`, starts the listener once setup is ready, and publishes
  listener, model, Sparkle, and settings state for the window and the menu
  bar (`RuntimeHandle`, the Windows shape). The listener, the meeting
  controller (`meeting_watcher::controller_loop`), and the local API keep
  their own threads. Quit ends Rune's loop, then everything is joined.
- **Window host.** The shared panes reach the runtime through
  `macos_host::Backend`; Sparkle's actions run on the main thread there.
  Shortcut capture is an AppKit local event monitor (modifier-only chords
  such as Option or Fn never reach Rune as keys) with the listener's tap
  suspended. `macos_host::MacHost` is the panes' `NativeHost`:
  permissions and guides, System Audio, the login item worker, Hex
  Microphone, meeting start/stop/summarize/rename/trash/reveal, the command
  catalog and workspace, the command model, Apple Intelligence
  availability, and relaunch. It also lists meetings for the Meetings pane
  through the read-only `meeting::query` (rescanned once a second, sent
  only when changed), feeds the tour's practice step from the listener's
  event log, and chooses the meetings folder with `NSOpenPanel`. The main
  loop's timer wakes an open window every 250 ms so these polls run.
- **Title bar.** Rune's macOS window has a transparent, full-size title
  bar; the sidebar starts below the traffic lights, and it and the pane
  header move the window (Rune `El::window_drag`, `ea6b513`).
- **Not yet.**
  - The meeting capsules (offer, recording, call ended, ready) are GPUI
    windows and need GPUI's loop, which can't share the main thread with
    Rune's. Offers are logged and time out; the menu bar still shows and
    stops a recording. They need Rune's `Panel` kind (item 8), or an
    AppKit panel hosting a Rune surface.
  - The HUD stays a native NSPanel with its own Metal layer; moving it to
    Rune needs the `Overlay` kind.
- **Verification.** Built with Command Line Tools only (no Xcode): a local
  shim stubs the Metal shader compiles (GPUI's runtime shaders, an empty
  HUD metallib) and Apple Intelligence's `@Generable` types. Release
  builds need Xcode 26 as before. See `dev.rune-ui-macos` in the feature
  map for what was observed.
- **CLI.** It already has a channel in `local_api`. A macOS `hex dictation
  …` over `POST /dictations` is optional.

## Risks

- **Crash isolation is lost.** A Rune, wgpu, or driver crash in the window
  now takes down dictation and accepted work.
  - Mitigations: `Restart=on-failure` with no window on restart;
    window-creation failures (no usable GPU adapter, surface errors) return
    `Err` and must not panic. Today `create_window`, `create_surface`, and
    `get_default_config` are `expect`, and that has to change.
  - Add an open/close soak (50 cycles on niri and i3) with the listener
    running.
  - This cost is the reason to keep systemd supervision.
- **Rune churn.** Kit's agents push to Rune's `main` daily. Agree the
  `run_app`, `AppProxy`, and `WindowSpec` surface with Kit before Hex builds
  on it, and pin the commit as today.
- **Session environment.** A window in the systemd process misses anything
  not imported (cursor theme, IME, locale, icons). Extend `SESSION_KEYS`
  and check on real desktops.
- **Upgrade.** A v1 Settings client still open after a self-update restart
  gets a version-mismatch error and must exit rather than loop.

## Implementation order

| # | Step | Where |
|---|---|---|
| 1 | Rune core: `run_app` (argv-free, keep-alive), `AppProxy`, window table, `WindowSpec` with closures, `CloseBehavior` and `on_closed`, activation token on open and focus (Wayland and X11), shared GPU, fallible window creation, macOS activation policy and no default menu, `raw_window_handle()`. Keep `run(spec)` as a wrapper. | Rune, with Kit |
| 2 | Linux single process: `HostHandle` and runtime thread, `linux_instance` v2, launch logic, `rune_ui` on the handle, Quit Hex, `Type=notify`, session keys, installer and Home Manager and Nix, tests, docs, AGENTS.md. Remove the GPUI Settings process on Linux. | `rune-ui` |
| 3 | Physical validation on niri (portal) and i3: relaunch focus with a token, close and reopen, logout, crash restart, update restart. | person |
| 4 | Windows: runtime and `HostHandle`, Rune window with Hide, tray on `run_app`, named event to the proxy, HUD as an overlay or HWND-tweaked Rune window. Plus the Windows pane parity work in `rune-ui.md`. | `rune-ui` |
| 5 | macOS: runtime out of `meeting_watcher`, reopen, status item beside `rune::App`, Rune window. Written (see [macOS under Rune](#macos-under-rune)); the capsules, the Rune HUD, and the Meetings pane's data remain. The HUD and capsules wait for the Rune `Overlay` and `Panel` kinds. Plus the macOS pane parity work in `rune-ui.md`. | `rune-ui` + Rune |
| 6 | Delete GPUI: `app_window*`, `desktop_ui`, `design_tokens`, `text_input`, `desktop_transcription_picker`, `motion`, GPUI `windows_indicator` and `dictation_indicator`, the fork patch, the token lint. | `rune-ui` → main once Rune is public |

Windows and macOS follow the port order in
[`rune-ui.md`](rune-ui.md#order-to-port).