//! The desktop's file chooser (`org.freedesktop.portal.FileChooser`), for
//! picking an application the catalog does not list. One private session-bus
//! connection per dialog; the call blocks its thread until the person picks
//! a file or closes the dialog.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use color_eyre::eyre::{Result, WrapErr, eyre};
use gtk::gio::{self, prelude::FileExt};
use gtk::glib::{self, ToVariant, Variant};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const CALL_TIMEOUT_MS: i32 = 5_000;

type Response = Rc<RefCell<Option<(u32, HashMap<String, Variant>)>>>;

/// What the dialog offers to filter by: a label and its glob patterns.
pub(crate) struct Filter {
    pub(crate) label: String,
    pub(crate) patterns: Vec<&'static str>,
}

/// Opens one file. `Ok(None)` when the person closed the dialog.
pub(crate) fn open_file(title: &str, accept: &str, filters: &[Filter]) -> Result<Option<PathBuf>> {
    let address = gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|error| eyre!("no session bus for the file chooser: {error}"))?;
    let connection = gio::DBusConnection::for_address_sync(
        &address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .map_err(|error| eyre!("could not reach the session bus: {error}"))?;
    let sender = connection
        .unique_name()
        .ok_or_else(|| eyre!("the session bus gave Hex no name"))?
        .trim_start_matches(':')
        .replace('.', "_");
    let token = format!("hex{}_file", std::process::id());
    let request = format!("{DESKTOP_PATH}/request/{sender}/{token}");
    let context = glib::MainContext::new();
    let response = Response::default();
    let received = response.clone();
    // Subscribed before the call so a fast answer cannot be missed.
    let subscription = context
        .with_thread_default(|| {
            connection.signal_subscribe(
                Some(DESKTOP),
                Some("org.freedesktop.portal.Request"),
                Some("Response"),
                Some(&request),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, parameters| *received.borrow_mut() = parameters.get(),
            )
        })
        .map_err(|error| eyre!("could not watch the file chooser: {error}"))?;
    let result = (|| {
        connection
            .call_sync(
                Some(DESKTOP),
                DESKTOP_PATH,
                FILE_CHOOSER,
                "OpenFile",
                Some(&("", title, options(&token, accept, filters)).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                CALL_TIMEOUT_MS,
                gio::Cancellable::NONE,
            )
            .wrap_err("the desktop has no file chooser portal")?;
        loop {
            context.iteration(true);
            if let Some((code, results)) = response.borrow_mut().take() {
                return match code {
                    0 => Ok(selected(&results)),
                    1 => Ok(None),
                    _ => Err(eyre!("the file chooser could not open a file")),
                };
            }
        }
    })();
    connection.signal_unsubscribe(subscription);
    result
}

fn options(token: &str, accept: &str, filters: &[Filter]) -> HashMap<String, Variant> {
    let filters: Vec<(String, Vec<(u32, String)>)> = filters
        .iter()
        .map(|filter| {
            let patterns = filter
                .patterns
                .iter()
                .map(|pattern| (0, (*pattern).to_string()))
                .collect();
            (filter.label.clone(), patterns)
        })
        .collect();
    let mut options = HashMap::from([
        ("handle_token".to_string(), token.to_variant()),
        ("accept_label".to_string(), accept.to_variant()),
        ("modal".to_string(), true.to_variant()),
        ("multiple".to_string(), false.to_variant()),
    ]);
    if !filters.is_empty() {
        options.insert("filters".into(), filters.to_variant());
    }
    options
}

/// The first local path among the response's `uris`.
fn selected(results: &HashMap<String, Variant>) -> Option<PathBuf> {
    results
        .get("uris")?
        .get::<Vec<String>>()?
        .iter()
        .find_map(|uri| gio::File::for_uri(uri).path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_carry_the_token_and_filters_in_the_portal_signature() {
        let options = options(
            "hex1_file",
            "Add",
            &[Filter {
                label: "Applications".into(),
                patterns: vec!["*.desktop"],
            }],
        );
        assert_eq!(options["handle_token"].str(), Some("hex1_file"));
        assert_eq!(options["filters"].type_().as_str(), "a(sa(us))");
        assert_eq!(options.to_variant().type_().as_str(), "a{sv}");
    }

    #[test]
    fn the_response_yields_the_first_local_path() {
        let results = HashMap::from([(
            "uris".to_string(),
            vec![
                "https://example.com/app".to_string(),
                "file:///usr/share/applications/code%20insiders.desktop".to_string(),
            ]
            .to_variant(),
        )]);
        assert_eq!(
            selected(&results),
            Some(PathBuf::from(
                "/usr/share/applications/code insiders.desktop"
            ))
        );
        assert_eq!(selected(&HashMap::new()), None);
    }
}
