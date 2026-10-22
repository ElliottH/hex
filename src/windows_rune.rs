//! Hex on Windows: one tray process per login session.
//!
//! - The main thread runs Rune's event loop, the window, and the tray
//!   (`tray-icon` rides winit's Win32 message pump on that thread).
//! - The runtime thread owns `WindowsDesktopHost` (the listener, OpenCode,
//!   transformations, updates) and model preparation, and keeps running while
//!   the window is hidden. The window reaches it through [`RuntimeHandle`].
//! - The named event from `windows_app` keeps one app per session: a later
//!   launch signals it and the running app shows its window.
//!
//! Rune's `App` keeps the loop, the state, and the hidden window alive
//! after the window closes (`keep_running`), starts without a window for a
//! login launch (`windowless`), and opens or quits from other threads
//! through its `AppHandle`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use color_eyre::Result;
use color_eyre::eyre::eyre;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GWLP_HINSTANCE, GetSystemMetrics, GetWindowLongPtrW, GetWindowTextW,
    ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON,
    SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
};

use crate::app_settings::{AppSettings, IndicatorPosition};
use crate::desktop_host::{
    DesktopAction, DesktopListenerSnapshot, DesktopShortcut, DesktopShortcuts,
    DesktopTranscriptionSnapshot, DesktopUpdateStatus,
};
use crate::desktop_i18n::{tr, tr_fill};
use crate::rune_ui::Ui;
use crate::transcription_models::{ModelPreparationStage, TranscriptionModelId};
use crate::transcription_preparation::TranscriptionPreparation;
use crate::windows_app::WindowsDesktopHost;
use crate::windows_updater::UpdateCheck;

/// The tray icon: the app icon pre-rendered to raw 32×32 RGBA at build time
/// from app/AppIcon.icon/Assets/Image.png (full color, so both taskbar
/// themes use the same pixels).
const TRAY_ICON_SIZE: u32 = 32;
/// How often the runtime refreshes the listener while nothing is asked.
const RUNTIME_TICK: Duration = Duration::from_millis(250);

/// What the window asks the runtime, in the Linux service's vocabulary so
/// the shared panes send the same requests on both platforms.
#[derive(Clone, Debug)]
pub(crate) enum Request {
    Desktop(DesktopAction),
    /// Handled in the window (`rune_ui::host`): Windows polls the keyboard.
    CaptureShortcutFor(#[allow(dead_code)] ShortcutTarget),
    CancelShortcut,
    /// Linux's service registers recorded shortcuts; on Windows the window
    /// saves them, so the runtime ignores this.
    #[allow(dead_code)]
    SetShortcut {
        target: ShortcutTarget,
        shortcut: DesktopShortcut,
    },
    ChooseModel {
        model: TranscriptionModelId,
        language: String,
    },
    CancelModel,
    /// The window saved `settings.json`; adopt it.
    ReloadSettings,
    /// The tray's Enable / Disable Hex.
    ToggleListening,
    /// The window's Quit Hex: stop the listener, then end the app.
    Quit,
}

/// A shortcut the listener registers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ShortcutTarget {
    #[default]
    Dictation,
    PasteLast,
    VoiceAction,
}

/// What the runtime last published for the window.
#[derive(Clone, PartialEq)]
pub(crate) struct Published {
    pub(crate) listener: DesktopListenerSnapshot,
    pub(crate) operation_error: Option<String>,
    pub(crate) transcription: DesktopTranscriptionSnapshot,
    pub(crate) update_status: DesktopUpdateStatus,
    pub(crate) settings: AppSettings,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Shared {
    published: Mutex<Option<Arc<Published>>>,
    wakers: Mutex<Vec<Wake>>,
}

/// The window's connection to the runtime thread.
#[derive(Clone)]
pub(crate) struct RuntimeHandle {
    requests: Sender<Request>,
    shared: Arc<Shared>,
}

impl RuntimeHandle {
    pub(crate) fn send(&self, request: Request) -> Result<()> {
        self.requests
            .send(request)
            .map_err(|_| eyre!("Hex's runtime stopped"))
    }

    pub(crate) fn published(&self) -> Option<Arc<Published>> {
        lock(&self.shared.published).clone()
    }

    /// Wakes `wake` after every publication.
    pub(crate) fn subscribe(&self, wake: Wake) {
        lock(&self.shared.wakers).push(wake);
    }
}

static RUNTIME: OnceLock<RuntimeHandle> = OnceLock::new();

/// The running app's runtime; `None` outside `open` (headless snapshots).
pub(crate) fn runtime() -> Option<RuntimeHandle> {
    RUNTIME.get().cloned()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

pub fn open(event_path: PathBuf, shutdown: &'static AtomicBool, start_hidden: bool) -> Result<()> {
    let Some(instance) = crate::windows_app::AppInstance::claim(start_hidden)? else {
        return Ok(());
    };
    shutdown.store(false, Ordering::Relaxed);
    let (settings, settings_error) = match AppSettings::load() {
        Ok(settings) => (settings, None),
        Err(error) => (
            AppSettings::default(),
            Some(tr_fill(
                "Could not load settings: {}",
                &format!("{error:#}"),
            )),
        ),
    };
    let mut app = crate::rune_ui::app().args(Vec::new());
    let window = app.handle();
    // The tray rides the main thread's message pump, which Rune's loop runs.
    match create_tray(window.clone()) {
        Ok(tray) => {
            app = if start_hidden {
                app.windowless()
            } else {
                app.keep_running()
            };
            TRAY.with(|slot| *slot.borrow_mut() = Some(tray));
        }
        // Without a tray, closing the window quits: Hex must not run on
        // with no way back to it.
        Err(error) => {
            tracing::warn!(%error, "Windows system tray is unavailable; closing Hex will quit");
        }
    }
    let hud = crate::windows_rune_hud::install(&mut app);
    let (requests, receiver) = mpsc::channel();
    let shared = Arc::new(Shared::default());
    RUNTIME
        .set(RuntimeHandle {
            requests: requests.clone(),
            shared: shared.clone(),
        })
        .map_err(|_| eyre!("the Hex runtime already started"))?;
    let quit = window.clone();
    let runtime = std::thread::Builder::new()
        .name("hex-runtime".into())
        .spawn(move || {
            run_runtime(
                event_path,
                settings,
                settings_error,
                receiver,
                &shared,
                shutdown,
                hud,
            );
            quit.quit();
        })?;
    let relaunch = window.clone();
    instance.forward_launches(move || {
        relaunch.open();
        true
    });
    // Returns on Quit, or when the window closes without a tray.
    app.run();
    shutdown.store(true, Ordering::Relaxed);
    drop(requests);
    let _ = runtime.join();
    TRAY.with(|slot| slot.borrow_mut().take());
    Ok(())
}

// ---------------------------------------------------------------------------
// Runtime thread

fn run_runtime(
    event_path: PathBuf,
    settings: AppSettings,
    settings_error: Option<String>,
    requests: Receiver<Request>,
    shared: &Shared,
    shutdown: &AtomicBool,
    hud: crate::windows_rune_hud::HudHandle,
) {
    // The HUD thread owns the indicator and ends with the host's sender; the
    // settings' position reaches it through `position`.
    let (indicator, indicator_events) = crate::windows_rune_hud::channel();
    let position = Arc::new(AtomicU8::new(settings.indicator_position as u8));
    let hud_position = position.clone();
    let _hud = std::thread::Builder::new()
        .name("hex-hud".into())
        .spawn(move || {
            crate::windows_rune_hud::run(hud, indicator_events, || {
                match hud_position.load(Ordering::Relaxed) {
                    1 => IndicatorPosition::Bottom,
                    2 => IndicatorPosition::Hidden,
                    _ => IndicatorPosition::Top,
                }
            })
        });
    let mut runtime = Runtime {
        host: WindowsDesktopHost::new(event_path, settings.clone(), settings_error, indicator),
        preparation: TranscriptionPreparation::default(),
        listen_after_setup: false,
        quit: false,
    };
    // The microphone stays cold until first-run setup finishes.
    if settings.enable_on_launch {
        if crate::onboarding::completion_recorded() {
            runtime.host.start();
        } else {
            runtime.listen_after_setup = true;
        }
    }
    loop {
        let mut next = match requests.recv_timeout(RUNTIME_TICK) {
            Ok(request) => Some(request),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        while let Some(request) = next {
            runtime.handle(request);
            next = requests.try_recv().ok();
        }
        if shutdown.load(Ordering::Relaxed) || runtime.quit {
            break;
        }
        position.store(runtime.host.indicator_position() as u8, Ordering::Relaxed);
        runtime.tick();
        let published = Arc::new(runtime.publish());
        let changed = {
            let mut current = lock(&shared.published);
            let changed = current.as_deref() != Some(&published);
            if changed {
                *current = Some(published);
            }
            changed
        };
        if changed {
            for wake in lock(&shared.wakers).iter() {
                wake();
            }
        }
    }
    runtime.host.stop();
}

struct Runtime {
    host: WindowsDesktopHost,
    preparation: TranscriptionPreparation,
    /// Enable on launch waits for first-run setup.
    listen_after_setup: bool,
    /// Quit Hex or restarting into an update ends the app.
    quit: bool,
}

impl Runtime {
    fn handle(&mut self, request: Request) {
        match request {
            Request::Desktop(action) => self.dispatch(action),
            Request::ToggleListening => {
                if self.host.is_running() {
                    self.host.stop();
                } else {
                    self.host.start();
                }
            }
            Request::ChooseModel { model, language } => {
                let definition = crate::transcription_models::definition(model);
                let selection = crate::transcription_models::TranscriptionSelection {
                    model,
                    language,
                    // Hints persist across models that can use them.
                    recognition_hints: if definition.supports_recognition_hints {
                        self.host.settings().transcription.recognition_hints.clone()
                    } else {
                        String::new()
                    },
                };
                self.preparation.start(selection, false);
            }
            Request::CancelModel => self.preparation.cancel(),
            Request::Quit => self.quit = true,
            Request::ReloadSettings => match AppSettings::load() {
                Ok(settings) => self.host.apply_settings(&settings),
                Err(error) => tracing::warn!(%error, "could not reload settings"),
            },
            Request::CaptureShortcutFor(_)
            | Request::CancelShortcut
            | Request::SetShortcut { .. } => {}
        }
    }

    fn dispatch(&mut self, action: DesktopAction) {
        match action {
            DesktopAction::StartListening => self.host.start(),
            DesktopAction::StopListening => self.host.stop(),
            DesktopAction::ClearError => self.host.clear_error(),
            DesktopAction::CheckForUpdates => self.host.check_for_updates(),
            DesktopAction::OpenUpdate => {
                if let UpdateCheck::Available { url, .. } = self.host.update_state()
                    && let Err(error) = crate::commands::shell_open(&url)
                {
                    tracing::warn!(%error, "could not open the Hex release");
                }
            }
            DesktopAction::RestartIntoUpdate => {
                if self.host.restart_into_update().is_ok() {
                    // Ends the app so the new version can take the named
                    // event.
                    self.quit = true;
                }
            }
            // Windows saves shortcuts from the window.
            DesktopAction::SetDictationShortcut(_) => {}
        }
    }

    /// Commits a prepared model, starts a listener that waited for setup,
    /// and follows the listener.
    fn tick(&mut self) {
        if let Some(result) = self.preparation.poll() {
            let result = result.and_then(|selection| {
                crate::transcription_models::validate(&selection).map_err(|e| e.to_string())?;
                let mut candidate = self.host.settings().clone();
                candidate.remember_transcription(selection);
                candidate.save().map_err(|e| format!("{e:#}"))?;
                self.host.apply_settings(&candidate);
                Ok(())
            });
            if let Err(error) = result {
                self.preparation.set_error(Some(error));
            }
        }
        if self.listen_after_setup && crate::onboarding::completion_recorded() {
            self.listen_after_setup = false;
            self.host.start();
        }
        self.host.refresh();
    }

    fn publish(&self) -> Published {
        let listener = self.host.listener_state();
        let preparation = self.preparation.status();
        Published {
            listener: DesktopListenerSnapshot {
                running: listener.running,
                status: listener.status,
                warning: listener.last_failure,
                shortcuts: DesktopShortcuts::Registered,
            },
            operation_error: listener.error,
            transcription: DesktopTranscriptionSnapshot {
                downloaded_bytes: preparation.downloaded_bytes,
                error: preparation.error,
                preparation_stage: preparation
                    .model
                    .map(|_| ModelPreparationStage::Downloading),
                selection: self.host.settings().transcription.clone(),
                preparing: preparation.model,
            },
            update_status: match self.host.update_state() {
                UpdateCheck::Checking => DesktopUpdateStatus::Checking,
                UpdateCheck::Current => DesktopUpdateStatus::Current,
                UpdateCheck::Failed => DesktopUpdateStatus::Failed,
                UpdateCheck::Available { .. } => DesktopUpdateStatus::Available,
                UpdateCheck::ReadyToRestart { .. } => DesktopUpdateStatus::ReadyToRestart,
            },
            settings: self.host.settings().clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Main thread: the tray and the window icon

thread_local! {
    static TRAY: std::cell::RefCell<Option<TrayIcon>> = const { std::cell::RefCell::new(None) };
    static ICON_SET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Gives the window the app icon once it exists. Rune's winit window has
/// none and exposes no handle, so this finds it on the main thread by its
/// title; closing only hides it, so once is enough.
pub(crate) fn attach_window_icon() {
    if RUNTIME.get().is_none() || ICON_SET.get() {
        return;
    }
    if let Some(hwnd) = find_window() {
        apply_window_icon(hwnd);
        ICON_SET.set(true);
    }
}

/// Gives the window the executable's embedded icon (resource 1, see
/// build.rs) for the taskbar, Alt-Tab, and the caption, so they match the
/// exe in Explorer and Task Manager. Windows picks the frame that fits each
/// size and DPI.
fn apply_window_icon(hwnd: HWND) {
    const APP_ICON: u16 = 1;
    unsafe {
        let module = GetWindowLongPtrW(hwnd, GWLP_HINSTANCE) as HINSTANCE;
        for (kind, width, height) in [
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
            (ICON_BIG, SM_CXICON, SM_CYICON),
        ] {
            let icon = LoadImageW(
                module,
                APP_ICON as usize as *const u16,
                IMAGE_ICON,
                GetSystemMetrics(width),
                GetSystemMetrics(height),
                LR_DEFAULTCOLOR,
            );
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, kind as usize, icon as isize);
            }
        }
    }
}

/// The top-level window titled "Hex" on this thread.
fn find_window() -> Option<HWND> {
    unsafe extern "system" fn visit(hwnd: HWND, found: LPARAM) -> i32 {
        let mut title = [0u16; 8];
        // SAFETY: `title` is a writable buffer of the length passed.
        let length = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
        if String::from_utf16_lossy(&title[..length.max(0) as usize]) == "Hex" {
            // SAFETY: `found` points at the caller's `HWND` slot.
            unsafe { *(found as *mut HWND) = hwnd };
            return 0;
        }
        1
    }
    let mut found: HWND = std::ptr::null_mut();
    // SAFETY: `visit` only writes through the pointer to `found`, which
    // outlives the call.
    unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(visit),
            (&raw mut found) as LPARAM,
        );
    }
    (!found.is_null()).then_some(found)
}

fn create_tray(window: rune::AppHandle<Ui>) -> Result<TrayIcon> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id("show", tr("Show Hex"), true, None))?;
    menu.append(&MenuItem::with_id(
        "toggle",
        tr("Enable / Disable Hex"),
        true,
        None,
    ))?;
    menu.append(&MenuItem::with_id("quit", tr("Quit Hex"), true, None))?;
    let menu_window = window.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.as_ref() {
        "show" => menu_window.open(),
        "toggle" => {
            if let Some(runtime) = runtime() {
                let _ = runtime.send(Request::ToggleListening);
            }
        }
        "quit" => menu_window.quit(),
        _ => {}
    }));
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            window.open();
        }
    }));
    Ok(TrayIconBuilder::new()
        .with_tooltip(tr("Hex Dictation"))
        .with_icon(tray_icon::Icon::from_rgba(
            include_bytes!("../resources/windows/tray-32.rgba").to_vec(),
            TRAY_ICON_SIZE,
            TRAY_ICON_SIZE,
        )?)
        .with_menu(Box::new(menu))
        // Left click shows the window (handled above); only right click
        // opens the menu.
        .with_menu_on_left_click(false)
        .build()?)
}
