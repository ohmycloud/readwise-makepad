//! macOS EPUB file picker bridge.
//!
//! Makepad's current macOS `SelectFileDialog` operation is still a logging
//! placeholder, so ReadWise opens AppKit's panel through the Objective-C
//! runtime already used by Makepad. A sheet keeps the main event loop running
//! and hands selected paths back through a Makepad action.

use makepad_widgets::makepad_platform::os::apple::apple_sys::*;
use makepad_widgets::Cx;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub enum DesktopImportAction {
    Picked(Vec<String>),
    Cancelled,
    Error(String),
}

static PICKER_OPEN: AtomicBool = AtomicBool::new(false);

unsafe fn selected_epub_paths(panel: ObjcId) -> DesktopImportAction {
    let filenames: ObjcId = msg_send![panel, filenames];
    if filenames == nil {
        return DesktopImportAction::Error("文件选择器没有返回路径".to_string());
    }
    let count: usize = msg_send![filenames, count];
    let mut paths = Vec::with_capacity(count);
    for index in 0..count {
        let path: ObjcId = msg_send![filenames, objectAtIndex: index];
        if path != nil {
            let path = nsstring_to_string(path);
            if path.to_ascii_lowercase().ends_with(".epub") {
                paths.push(path);
            }
        }
    }
    if paths.is_empty() {
        DesktopImportAction::Error("没有选择 EPUB 文件".to_string())
    } else {
        DesktopImportAction::Picked(paths)
    }
}

/// Opens a native, multi-select EPUB panel.
///
/// Results arrive as `DesktopImportAction`. Selected files remain owned by
/// the user; unlike iOS import mode, the caller must not delete these sources.
pub fn open_epub_picker(title: &str, prompt: &str) -> Result<(), String> {
    if PICKER_OPEN.swap(true, Ordering::AcqRel) {
        return Err("文件选择器已经打开".to_string());
    }
    unsafe {
        let app: ObjcId = msg_send![class!(NSApplication), sharedApplication];
        let mut window: ObjcId = msg_send![app, keyWindow];
        if window == nil {
            window = msg_send![app, mainWindow];
        }
        if window == nil {
            PICKER_OPEN.store(false, Ordering::Release);
            return Err("无法找到书库窗口".to_string());
        }
        let panel: ObjcId = msg_send![class!(NSOpenPanel), openPanel];
        if panel == nil {
            PICKER_OPEN.store(false, Ordering::Release);
            return Err("无法创建 macOS 文件选择器".to_string());
        }

        let _: () = msg_send![panel, setCanChooseFiles: YES];
        let _: () = msg_send![panel, setCanChooseDirectories: NO];
        let _: () = msg_send![panel, setAllowsMultipleSelection: YES];
        let _: () = msg_send![panel, setAllowsOtherFileTypes: NO];
        let _: () = msg_send![panel, setResolvesAliases: YES];
        let _: () = msg_send![panel, setTitle: str_to_nsstring(title)];
        let _: () = msg_send![panel, setPrompt: str_to_nsstring(prompt)];

        // `setAllowedFileTypes:` is deprecated by newer SDKs but remains
        // available at runtime across every macOS version supported by the
        // locked Makepad revision, without adding a UniformTypeIdentifiers
        // framework dependency.
        let file_types: ObjcId =
            msg_send![class!(NSArray), arrayWithObject: str_to_nsstring("epub")];
        let _: () = msg_send![panel, setAllowedFileTypes: file_types];

        // The callback can run after this function returns. Keep the panel
        // alive until its result has been copied into an owned action.
        let _: ObjcId = msg_send![panel, retain];
        let panel_addr = panel as usize;
        let window_addr = window as usize;
        let completion = objc_block!(move |response: i64| {
            let panel = panel_addr as ObjcId;
            let window = window_addr as ObjcId;
            let action = if response == 1 {
                unsafe { selected_epub_paths(panel) }
            } else {
                DesktopImportAction::Cancelled
            };
            unsafe {
                // AppKit may call this while the sheet is still onscreen.
                // Dismiss it before returning key focus to Makepad's window.
                let _: () = msg_send![panel, orderOut: nil];
                let visible: BOOL = msg_send![window, isVisible];
                if visible == YES {
                    let _: () = msg_send![window, makeKeyAndOrderFront: nil];
                }
            }
            PICKER_OPEN.store(false, Ordering::Release);
            Cx::post_action(action);
            let _: () = unsafe { msg_send![panel, autorelease] };
        });
        let _: () =
            msg_send![panel, beginSheetModalForWindow: window completionHandler: &completion];
        Ok(())
    }
}
