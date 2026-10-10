//! Whether the system tray can be created, and what closing a window does
//! when it cannot.
//!
//! On Linux the tray icon goes through `libappindicator-sys`, which
//! `dlopen`s libayatana-appindicator3 (or libappindicator3) the first time a
//! tray is built and **panics** when neither loads — missing library, or a
//! host library that needs a newer GLib than an AppImage bundles. Release
//! builds use `panic = "abort"`, so that panic kills the process before any
//! `catch_unwind` can see it. [`check_tray_backend`] performs the same loads
//! up front so startup can skip the tray instead of crashing.

use std::fmt;

/// Label of the main application window (see `tauri.conf.json`).
pub const MAIN_WINDOW_LABEL: &str = "main";

/// Library names `libappindicator-sys` 0.9 tries, in its order. The last two
/// come from its `backcompat` feature, which `libappindicator`'s default
/// features (used by `tray-icon`) enable.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const APPINDICATOR_LIBRARIES: &[&str] = &[
    "libayatana-appindicator3.so.1",
    "libappindicator3.so.1",
    "libayatana-appindicator3.so",
    "libappindicator3.so",
];

/// The tray backend library could not be loaded; carries one error per name
/// tried, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct TrayBackendUnavailable {
    pub load_errors: Vec<String>,
}

impl fmt::Display for TrayBackendUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "System tray unavailable: could not load the AppIndicator library \
             (libayatana-appindicator3 or libappindicator3). LocalRouter keeps \
             running without a tray icon; closing the main window quits the app."
        )?;
        for error in &self.load_errors {
            writeln!(f, "  - {error}")?;
        }
        write!(
            f,
            "To get the tray icon, install libayatana-appindicator3 from your \
             distribution (for example libayatana-appindicator-gtk3 or \
             libayatana-appindicator3-1). If it is installed and an \"undefined \
             symbol\" error is shown above, the system library needs a newer \
             GLib than this AppImage bundles: use the .deb, .rpm or Flatpak \
             build, or start the AppImage with LD_PRELOAD set to the system \
             libglib-2.0.so.0."
        )
    }
}

/// Load the first library in `names` that loads, with the same `dlopen`
/// flags `libappindicator-sys` uses (libloading's `Library::new`:
/// `RTLD_LAZY | RTLD_LOCAL` on Unix), so this fails exactly when that crate
/// would panic.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn first_loadable(names: &[&str]) -> Result<libloading::Library, TrayBackendUnavailable> {
    let mut load_errors = Vec::with_capacity(names.len());
    for name in names {
        // SAFETY: loading a shared library runs its initializers. These are
        // the same libraries, loaded the same way, that the tray backend
        // loads moments later anyway.
        match unsafe { libloading::Library::new(name) } {
            Ok(library) => return Ok(library),
            Err(e) => {
                // dlerror() text starts with the name; Windows' does not.
                let message = e.to_string();
                load_errors.push(if message.contains(name) {
                    message
                } else {
                    format!("{name}: {message}")
                });
            }
        }
    }
    Err(TrayBackendUnavailable { load_errors })
}

/// Check that the platform's tray backend can be loaded.
///
/// Only Linux loads its tray backend at run time; elsewhere this is always
/// `Ok`.
pub fn check_tray_backend() -> Result<(), TrayBackendUnavailable> {
    #[cfg(target_os = "linux")]
    {
        // Keep the library loaded: the tray backend's own load then reuses
        // it, and a GTK-linked library is never unloaded under the process.
        std::mem::forget(first_loadable(APPINDICATOR_LIBRARIES)?);
    }
    Ok(())
}

/// What to do when a window asks to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseRequestAction {
    /// Keep the window (and the app) alive but hidden.
    Hide,
    /// Quit the application.
    Quit,
}

/// Closing hides windows so LocalRouter keeps serving from the tray. Without
/// a tray a hidden main window could not be brought back, and the tray's
/// Quit item would be the only way out, so closing the main window quits
/// instead — the usual meaning of closing an app that has no tray icon.
pub fn close_request_action(window_label: &str, tray_present: bool) -> CloseRequestAction {
    if !tray_present && window_label == MAIN_WINDOW_LABEL {
        CloseRequestAction::Quit
    } else {
        CloseRequestAction::Hide
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A system library that exists on every platform the tests run on.
    #[cfg(target_os = "linux")]
    const PRESENT_LIBRARY: &str = "libc.so.6";
    #[cfg(target_os = "macos")]
    const PRESENT_LIBRARY: &str = "/usr/lib/libSystem.B.dylib";
    #[cfg(windows)]
    const PRESENT_LIBRARY: &str = "kernel32.dll";

    #[test]
    fn missing_libraries_report_one_error_per_name_in_order() {
        let names = [
            "liblocalrouter-test-missing-a.so.1",
            "liblocalrouter-test-missing-b.so.1",
        ];
        let err = first_loadable(&names).expect_err("nonexistent libraries must not load");
        assert_eq!(err.load_errors.len(), 2);
        assert!(err.load_errors[0].contains("missing-a"), "{err:?}");
        assert!(err.load_errors[1].contains("missing-b"), "{err:?}");
    }

    #[test]
    fn empty_name_list_is_unavailable() {
        let err = first_loadable(&[]).expect_err("nothing to load");
        assert!(err.load_errors.is_empty());
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn falls_through_to_the_first_library_that_loads() {
        let names = ["liblocalrouter-test-missing.so.1", PRESENT_LIBRARY];
        assert!(first_loadable(&names).is_ok());
    }

    #[test]
    fn library_names_mirror_libappindicator_sys_order() {
        assert_eq!(
            APPINDICATOR_LIBRARIES,
            &[
                "libayatana-appindicator3.so.1",
                "libappindicator3.so.1",
                "libayatana-appindicator3.so",
                "libappindicator3.so",
            ]
        );
    }

    #[test]
    fn unavailable_message_lists_errors_and_hint() {
        let err = TrayBackendUnavailable {
            load_errors: vec![
                "libayatana-appindicator3.so.1: undefined symbol: g_once_init_leave_pointer"
                    .to_string(),
                "libappindicator3.so.1: cannot open shared object file".to_string(),
            ],
        };
        let message = err.to_string();
        assert!(message.contains("g_once_init_leave_pointer"));
        assert!(message.contains("libappindicator3.so.1: cannot open"));
        assert!(message.contains("closing the main window quits"));
        assert!(message.contains("LD_PRELOAD"));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn tray_backend_is_always_available_off_linux() {
        assert_eq!(check_tray_backend(), Ok(()));
    }

    #[test]
    fn closing_main_window_hides_it_when_tray_exists() {
        assert_eq!(
            close_request_action(MAIN_WINDOW_LABEL, true),
            CloseRequestAction::Hide
        );
    }

    #[test]
    fn closing_main_window_quits_without_tray() {
        assert_eq!(
            close_request_action(MAIN_WINDOW_LABEL, false),
            CloseRequestAction::Quit
        );
    }

    #[test]
    fn closing_popup_windows_always_hides() {
        assert_eq!(
            close_request_action("firewall-approval-1", true),
            CloseRequestAction::Hide
        );
        assert_eq!(
            close_request_action("firewall-approval-1", false),
            CloseRequestAction::Hide
        );
    }
}
