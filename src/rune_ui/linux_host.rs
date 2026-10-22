//! The Linux window's host: a client of the per-user `hex.service`, which
//! owns the listener, shortcuts, models, and updates.

use std::sync::Arc;

use color_eyre::Result;

use super::{HostState, ServiceView};
use crate::app_settings::HotkeyBinding;
use crate::desktop_i18n::tr;
use crate::linux_input::LinuxHotkey;
use crate::linux_service::Client;
pub(super) use crate::linux_service::{Request, ShortcutTarget};

/// The service registers recorded shortcuts (`SetShortcut`) and saves them.
pub(super) const HOST_SAVES_SHORTCUTS: bool = true;
/// Settings records shortcuts from the window's key events.
pub(super) const RECORDS_KEY_EVENTS: bool = true;
/// The service checks for updates on its own schedule.
pub(super) const MANUAL_UPDATE_CHECK: bool = false;
/// X11 and portal shortcuts paste with Ctrl+V; terminals want Ctrl+Shift+V.
pub(super) const TERMINAL_PASTE: bool = true;

pub(super) struct Backend(Client);

impl Backend {
    pub(super) fn connect(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self(Client::with_waker(wake))
    }

    pub(super) fn send(&mut self, request: Request) -> Result<()> {
        self.0.send(request)
    }

    pub(super) fn refresh(&mut self) {
        self.0.refresh();
    }

    pub(super) fn error(&self) -> Option<String> {
        self.0.error.clone()
    }

    pub(super) fn state(&self) -> Option<HostState> {
        self.0.state.as_ref().map(|state| HostState {
            view: ServiceView {
                listener: state.desktop.listener.clone(),
                operation_error: state.desktop.operation_error.clone(),
                transcription: state.desktop.transcription.clone(),
                update_status: state.desktop.update_status,
                editing: state.capturing || state.editing,
            },
            settings: state.settings.clone(),
        })
    }

    /// Linux records shortcuts from the window's key events.
    pub(super) fn take_chord(&mut self) -> Option<super::settings::Chord> {
        None
    }
}

pub(super) fn validate_hotkey(
    _: ShortcutTarget,
    binding: &HotkeyBinding,
) -> Result<(), &'static str> {
    LinuxHotkey::from(binding)
        .validate()
        .map_err(|_| tr("Add a key Linux can register"))
}

/// Wayland never says which application is focused.
pub(super) fn focused_application_known() -> bool {
    !crate::linux_session::LinuxSession::detect().is_wayland()
}

/// Linux has no Launch at login row; autostart is the desktop entry.
pub(super) fn launch_at_login() -> Option<Result<bool, String>> {
    None
}

pub(super) fn set_launch_at_login(_: bool) -> Result<bool, String> {
    Err("Launch at login is not available on Linux".into())
}
