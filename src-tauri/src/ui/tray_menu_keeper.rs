//! Keeps replaced tray menus alive while they may still be on screen.
//!
//! muda's macOS menu items hold a raw pointer to their Rust-side state, and
//! `TrayIcon::set_menu` drops the previous menu immediately. The tray menu is
//! rebuilt from background events (health changes, firewall cleanup, usage
//! updates), so a rebuild while the menu is open freed the items the user was
//! looking at; clicking one then read freed memory and aborted the app inside
//! muda's About-panel path (0.0.153 crash, `PlatformIcon::to_nsimage`).
//!
//! Every menu handed to the tray is kept here. A replaced menu is only freed
//! by a later rebuild that runs while no menu can be open, i.e. on the main
//! thread outside AppKit's menu-tracking run loop mode. Any action of a
//! dismissed menu has run by then: AppKit performs it before the tracking
//! session returns to the default mode.

use parking_lot::Mutex;
use tauri::{menu::Menu, Runtime};

/// The tray's current menu plus every menu it replaced that may still be
/// displayed.
pub struct TrayMenuKeeper<R: Runtime> {
    menus: Mutex<KeptMenus<Menu<R>>>,
}

impl<R: Runtime> TrayMenuKeeper<R> {
    /// Start keeping `initial`, the menu the tray was built with.
    pub fn new(initial: Menu<R>) -> Self {
        Self {
            menus: Mutex::new(KeptMenus {
                current: Some(initial),
                retired: Vec::new(),
            }),
        }
    }

    /// Record `menu` as the tray's new menu. Must run on the main thread,
    /// before the menu is handed to the tray. Returns the retired menus that
    /// are safe to free; drop them after `set_menu`.
    pub fn replace(&self, menu: Menu<R>) -> Vec<Menu<R>> {
        let mut menus = self.menus.lock();
        let freed = menus.replace(menu, menu_may_be_open());
        tracing::debug!(
            "Tray menu replaced: freeing {} retired menus, keeping {}",
            freed.len(),
            menus.retired.len()
        );
        freed
    }
}

/// Menu bookkeeping, generic so it is testable without a Tauri runtime.
struct KeptMenus<T> {
    current: Option<T>,
    retired: Vec<T>,
}

impl<T> KeptMenus<T> {
    /// Make `menu` current and retire the previous one. Menus retired earlier
    /// are returned for freeing unless a menu may still be open.
    fn replace(&mut self, menu: T, menu_may_be_open: bool) -> Vec<T> {
        let freed = if menu_may_be_open {
            Vec::new()
        } else {
            std::mem::take(&mut self.retired)
        };
        if let Some(previous) = self.current.replace(menu) {
            self.retired.push(previous);
        }
        freed
    }
}

/// Whether a menu may be on screen. AppKit runs menus in their own tracking
/// run loop mode, so anything other than the default mode (menu tracking, a
/// modal panel, or no running loop at all) counts as possibly open.
#[cfg(target_os = "macos")]
fn menu_may_be_open() -> bool {
    use objc2_foundation::{NSDefaultRunLoopMode, NSRunLoop};

    let Some(mode) = NSRunLoop::currentRunLoop().currentMode() else {
        return true;
    };
    // SAFETY: NSDefaultRunLoopMode is an immutable NSString constant.
    *mode != *unsafe { NSDefaultRunLoopMode }
}

/// Other platforms keep only the menu replaced last.
#[cfg(not(target_os = "macos"))]
fn menu_may_be_open() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(current: &str) -> KeptMenus<String> {
        KeptMenus {
            current: Some(current.to_string()),
            retired: Vec::new(),
        }
    }

    #[test]
    fn replaced_menu_is_kept_until_the_next_closed_rebuild() {
        let mut menus = kept("a");

        assert!(menus.replace("b".into(), false).is_empty());
        assert_eq!(menus.current.as_deref(), Some("b"));
        assert_eq!(menus.retired, ["a"]);

        assert_eq!(menus.replace("c".into(), false), ["a"]);
        assert_eq!(menus.current.as_deref(), Some("c"));
        assert_eq!(menus.retired, ["b"]);
    }

    #[test]
    fn nothing_is_freed_while_a_menu_may_be_open() {
        let mut menus = kept("a");

        assert!(menus.replace("b".into(), true).is_empty());
        assert!(menus.replace("c".into(), true).is_empty());
        assert!(menus.replace("d".into(), true).is_empty());
        assert_eq!(menus.retired, ["a", "b", "c"]);

        // The first rebuild after the menu closes frees everything retired
        // while it was open, but keeps the menu it just replaced.
        assert_eq!(menus.replace("e".into(), false), ["a", "b", "c"]);
        assert_eq!(menus.retired, ["d"]);
        assert_eq!(menus.current.as_deref(), Some("e"));
    }

    #[test]
    fn first_replacement_without_a_current_menu_retires_nothing() {
        let mut menus: KeptMenus<String> = KeptMenus {
            current: None,
            retired: Vec::new(),
        };

        assert!(menus.replace("a".into(), false).is_empty());
        assert!(menus.retired.is_empty());
        assert_eq!(menus.current.as_deref(), Some("a"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_thread_without_a_running_loop_counts_as_possibly_open() {
        // Test threads never run their run loop, so `currentMode` is nil;
        // that must not be mistaken for "no menu open".
        assert!(menu_may_be_open());
    }
}
