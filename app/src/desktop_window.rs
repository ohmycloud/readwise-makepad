//! macOS reader window constraints.
//!
//! Makepad 2.0 does not yet expose `NSWindow.contentMinSize`. Applying the
//! constraint through AppKit lets the native edge-resize gesture stop at the
//! boundary instead of racing an application-driven resize on every geometry
//! event.

use makepad_widgets::makepad_platform::os::apple::apple_sys::*;
use std::sync::atomic::{AtomicBool, Ordering};

static READER_MINIMUM_CONFIGURED: AtomicBool = AtomicBool::new(false);

fn is_reader_window_title(title: &str) -> bool {
    title == "ReadWise"
}

/// Installs a native minimum content width on the main reader window.
///
/// Returns `false` when AppKit has not created the window yet; callers may
/// safely retry on the next startup/window-geometry event.
pub fn ensure_reader_minimum_content_width(minimum_width: f64) -> bool {
    if READER_MINIMUM_CONFIGURED.load(Ordering::Acquire) {
        return true;
    }
    if !minimum_width.is_finite() || minimum_width <= 0.0 {
        return false;
    }

    unsafe {
        let application: ObjcId = msg_send![class!(NSApplication), sharedApplication];
        if application == nil {
            return false;
        }
        let windows: ObjcId = msg_send![application, windows];
        if windows == nil {
            return false;
        }
        let count: usize = msg_send![windows, count];
        for index in 0..count {
            let window: ObjcId = msg_send![windows, objectAtIndex: index];
            if window == nil {
                continue;
            }
            let title: ObjcId = msg_send![window, title];
            if title == nil || !is_reader_window_title(&nsstring_to_string(title)) {
                continue;
            }

            let current_minimum: NSSize = msg_send![window, contentMinSize];
            let minimum = NSSize {
                width: current_minimum.width.max(minimum_width),
                height: current_minimum.height.max(0.0),
            };
            let _: () = msg_send![window, setContentMinSize: minimum];
            let applied: NSSize = msg_send![window, contentMinSize];
            debug_assert!(applied.width + 0.5 >= minimum_width);
            READER_MINIMUM_CONFIGURED.store(true, Ordering::Release);
            log::info!("Configured macOS reader minimum content width: {minimum_width}px");
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_reader_window_receives_the_constraint() {
        assert!(is_reader_window_title("ReadWise"));
        assert!(!is_reader_window_title(""));
        assert!(!is_reader_window_title("Open EPUB"));
    }
}
