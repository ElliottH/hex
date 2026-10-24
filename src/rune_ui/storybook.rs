//! The storybook: Rune's dev panel over this window's fixture scenes, in debug
//! builds with the `storybook` feature (`scripts/storybook`).

use super::*;

/// Where the panel listens: `HEX_STORYBOOK_ADDR` (`127.0.0.1:7863` by default).
/// A busy port moves on to the next free one.
fn address() -> std::net::SocketAddr {
    std::env::var("HEX_STORYBOOK_ADDR")
        .ok()
        .and_then(|address| match address.parse() {
            Ok(address) => Some(address),
            Err(_) => {
                eprintln!(
                    "HEX_STORYBOOK_ADDR is not an address like 127.0.0.1:7863; using the default"
                );
                None
            }
        })
        .unwrap_or_else(|| ([127, 0, 0, 1], 7863).into())
}

pub(super) fn config() -> rune::dev::Config<Ui> {
    let config = rune::dev::Config::<Ui>::default()
        .transition("Open model picker", "mac-settings", |ui, _| {
            let language = ui.settings.transcription.language.clone();
            picker::open(ui, language);
        })
        .transition("Record dictation shortcut", "mac-settings", |ui, _| {
            ui.capture = Capture::Listening {
                target: ShortcutTarget::Dictation,
                message: None,
                held: Default::default(),
            };
        })
        .transition("Go to Modes", "mac-settings", |ui, tx| {
            ui.select_pane(Pane::Modes, tx)
        });
    rune::dev::Config {
        bind: address(),
        ..config
    }
}
