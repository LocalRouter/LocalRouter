//! Approval windows temporarily activate the app to accept keyboard input.
//! On macOS, return an approval-only interaction to the background using
//! AppKit's normal Hide operation when the last focused approval closes.
//! This prevents AppKit selecting the dashboard behind it and lets the system
//! restore the previous application. Choosing a regular window cancels this.

use tauri::{Manager, WebviewWindow};

pub fn is_approval(label: &str) -> bool {
    [
        "firewall-approval-",
        "sampling-approval-",
        "elicitation-form-",
    ]
    .iter()
    .any(|prefix| label.strip_prefix(prefix).is_some_and(|id| !id.is_empty()))
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct ActivationSession {
    visible: std::collections::HashSet<String>,
    temporary: bool,
}

#[cfg(any(target_os = "macos", test))]
impl ActivationSession {
    fn present(&mut self, label: &str, app_active: bool) {
        if self.visible.is_empty() {
            self.temporary = !app_active;
        }
        self.visible.insert(label.to_owned());
    }

    fn focus_regular_window(&mut self) {
        // The user chose the dashboard (or another ordinary app window).
        // Closing an approval must no longer return the app to the background.
        self.temporary = false;
    }

    fn close(&mut self, label: &str, focused: bool) -> bool {
        if !self.visible.remove(label) {
            return false;
        }
        let restore = self.visible.is_empty() && self.temporary && focused;
        if self.visible.is_empty() {
            self.temporary = false;
        }
        restore
    }
}

#[cfg(target_os = "macos")]
thread_local! {
    // Only touched on AppKit's main thread, including window event callbacks.
    static SESSION: std::cell::RefCell<ActivationSession> = Default::default();
}

fn present_on_main_thread<R: tauri::Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mtm = objc2::MainThreadMarker::new().expect("popup presentation on main thread");
        let active = objc2_app_kit::NSApplication::sharedApplication(mtm).isActive();
        SESSION.with_borrow_mut(|session| session.present(window.label(), active));
    }

    let result = window.show().and_then(|()| window.set_focus());
    #[cfg(target_os = "macos")]
    if result.is_err() {
        SESSION.with_borrow_mut(|session| session.close(window.label(), false));
    }
    result.map_err(|e| e.to_string())
}

/// Used by the tray to re-open an already loaded, hidden approval.
pub fn present<R: tauri::Runtime>(window: WebviewWindow<R>) -> tauri::Result<()> {
    window.app_handle().clone().run_on_main_thread(move || {
        if let Err(error) = present_on_main_thread(&window) {
            tracing::error!("Failed to present approval {}: {error}", window.label());
        }
    })
}

/// Called by the approval after its details and size are ready. The invoking
/// window is injected by Tauri; the frontend cannot target another window.
pub async fn show(window: WebviewWindow) -> Result<(), String> {
    if !is_approval(window.label()) {
        return Err("Only approval windows can use this command".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .app_handle()
        .clone()
        .run_on_main_thread(move || {
            let _ = tx.send(present_on_main_thread(&window));
        })
        .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    #[cfg(target_os = "macos")]
    {
        if !is_approval(window.label()) {
            if matches!(event, tauri::WindowEvent::Focused(true)) {
                SESSION.with_borrow_mut(ActivationSession::focus_regular_window);
            }
            return;
        }
        match event {
            tauri::WindowEvent::CloseRequested { .. } => {
                let focused = window.is_focused().unwrap_or(false);
                let restore =
                    SESSION.with_borrow_mut(|session| session.close(window.label(), focused));
                if restore {
                    let mtm =
                        objc2::MainThreadMarker::new().expect("window events on the main thread");
                    // Hide the app before hiding/closing the key popup. Otherwise
                    // AppKit can first make the ordinary main window key.
                    objc2_app_kit::NSApplication::sharedApplication(mtm).hide(None);
                }
            }
            tauri::WindowEvent::Destroyed => {
                SESSION.with_borrow_mut(|session| session.close(window.label(), false));
            }
            _ => {}
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_approval_labels_are_accepted() {
        for label in [
            "firewall-approval-1",
            "sampling-approval-2",
            "elicitation-form-3",
        ] {
            assert!(is_approval(label));
        }
        for label in [
            "main",
            "settings",
            "firewall-approval-",
            "other-firewall-approval-1",
        ] {
            assert!(!is_approval(label));
        }
    }

    #[test]
    fn returns_temporary_activation_only_for_the_last_focused_approval() {
        let mut session = ActivationSession::default();
        session.present("a", false);
        session.present("b", true);
        assert!(!session.close("a", false));
        assert!(session.close("b", true));
        assert!(!session.close("b", true));
    }

    #[test]
    fn foreground_app_stays_foreground() {
        let mut session = ActivationSession::default();
        session.present("a", true);
        assert!(!session.close("a", true));
    }

    #[test]
    fn choosing_dashboard_cancels_temporary_activation() {
        let mut session = ActivationSession::default();
        session.present("a", false);
        session.focus_regular_window();
        assert!(!session.close("a", true));
    }

    #[test]
    fn background_expiry_does_not_change_the_users_current_app() {
        let mut session = ActivationSession::default();
        session.present("a", false);
        assert!(!session.close("a", false));
        session.present("b", true);
        assert!(!session.close("b", true));
    }

    #[test]
    fn reopening_a_hidden_approval_starts_a_new_session() {
        let mut session = ActivationSession::default();
        session.present("a", false);
        assert!(session.close("a", true));
        session.present("a", true);
        assert!(!session.close("a", true));
    }

    #[test]
    fn repeated_presentation_preserves_the_original_activation() {
        let mut session = ActivationSession::default();
        session.present("a", false);
        session.present("a", true);
        assert!(session.close("a", true));
    }

    #[test]
    fn closing_a_focused_approval_keeps_other_approvals_visible() {
        let mut session = ActivationSession::default();
        session.present("a", false);
        session.present("b", true);
        assert!(!session.close("b", true));
        assert!(session.close("a", true));
    }

    #[test]
    fn untracked_close_does_not_change_an_active_session() {
        let mut session = ActivationSession::default();
        session.present("a", false);
        assert!(!session.close("unknown", true));
        assert!(session.close("a", true));
    }
}
