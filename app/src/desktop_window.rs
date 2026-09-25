//! macOS reader window constraints.
//!
//! Makepad 2.0 does not yet expose `NSWindow.contentMinSize`. Applying the
//! constraint through AppKit lets the native edge-resize gesture stop at the
//! boundary instead of racing an application-driven resize on every geometry
//! event.

use makepad_widgets::makepad_platform::os::apple::apple_sys::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static READER_MINIMUM_CONFIGURED: AtomicBool = AtomicBool::new(false);
static READER_WINDOW_DELEGATE: OnceLock<&'static Class> = OnceLock::new();

#[link(name = "objc")]
extern "C" {
    fn object_setClass(object: ObjcId, class: *const Class) -> *const Class;
}

extern "C" fn reader_window_should_zoom(
    _delegate: &Object,
    _selector: Sel,
    window: ObjcId,
    _new_frame: NSRect,
) -> BOOL {
    unsafe {
        // AppKit can zoom a titled window on the second click even after the
        // Makepad button received its first click. Keep that native gesture
        // away from the reader's toolbar; the green zoom button still works.
        let event: ObjcId = msg_send![window, currentEvent];
        if event == nil {
            return YES;
        }
        let click_count: isize = msg_send![event, clickCount];
        if click_count < 2 {
            return YES;
        }
        let point: NSPoint = msg_send![event, locationInWindow];
        let content_view: ObjcId = msg_send![window, contentView];
        if content_view == nil {
            return YES;
        }
        let bounds: NSRect = msg_send![content_view, bounds];
        let top = bounds.size.height - point.y;
        // Match the 28-point caption buttons and their spacing in app.rs.
        let left_buttons = (92.0..=216.0).contains(&point.x);
        let right_buttons =
            (bounds.size.width - 110.0..=bounds.size.width - 18.0).contains(&point.x);
        if (0.0..=32.0).contains(&top) && (left_buttons || right_buttons) {
            return NO;
        }
    }
    YES
}

unsafe fn install_reader_window_delegate(window: ObjcId) {
    let delegate: ObjcId = msg_send![window, delegate];
    if delegate == nil {
        return;
    }
    let class = READER_WINDOW_DELEGATE.get_or_init(|| {
        let mut declaration =
            ClassDecl::new("ReadWiseRenderWindowDelegate", class!(RenderWindowDelegate))
                .expect("ReadWise window delegate should be registered once");
        declaration.add_method(
            sel!(windowShouldZoom:toFrame:),
            reader_window_should_zoom as extern "C" fn(&Object, Sel, ObjcId, NSRect) -> BOOL,
        );
        declaration.register()
    });
    object_setClass(delegate, *class as *const Class);
}

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
            install_reader_window_delegate(window);
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
