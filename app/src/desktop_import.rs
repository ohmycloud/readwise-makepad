//! macOS EPUB file picker bridge.
//!
//! Makepad's current macOS `SelectFileDialog` operation is still a logging
//! placeholder, so ReadWise opens AppKit's panel through the Objective-C
//! runtime already used by Makepad. The panel runs on the UI thread and hands
//! the selected paths back to the existing background importer.

use makepad_widgets::makepad_platform::os::apple::apple_sys::*;

/// Opens a native, multi-select EPUB panel.
///
/// `Ok(None)` means the user cancelled. Selected files remain owned by the
/// user; unlike iOS import mode, the caller must never delete these sources.
pub fn open_epub_picker(title: &str, prompt: &str) -> Result<Option<Vec<String>>, String> {
    unsafe {
        let panel: ObjcId = msg_send![class!(NSOpenPanel), openPanel];
        if panel == nil {
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

        // NSModalResponseOK is 1. `runModal` owns the nested AppKit event loop,
        // then returns control to Makepad's button action handler.
        let response: i64 = msg_send![panel, runModal];
        if response != 1 {
            return Ok(None);
        }

        let filenames: ObjcId = msg_send![panel, filenames];
        if filenames == nil {
            return Err("文件选择器没有返回路径".to_string());
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
            Err("没有选择 EPUB 文件".to_string())
        } else {
            Ok(Some(paths))
        }
    }
}
