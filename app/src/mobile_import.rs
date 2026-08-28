//! iOS EPUB document picker bridge.
//!
//! The Makepad file-dialog operation is not implemented on iOS, so the app owns
//! this small UIKit bridge until the platform crate exposes a real picker event.

use makepad_widgets::makepad_platform::os::apple::apple_sys::*;
use makepad_widgets::Cx;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

#[derive(Debug)]
pub enum IosImportAction {
    Picked(Vec<String>),
    Cancelled,
    Error(String),
}

static DELEGATE_CLASS: OnceLock<usize> = OnceLock::new();
static ACTIVE_DELEGATE: AtomicUsize = AtomicUsize::new(0);

fn release_active_delegate() {
    let delegate = ACTIVE_DELEGATE.swap(0, Ordering::AcqRel) as ObjcId;
    if delegate != nil {
        unsafe {
            // Balance `new` after the delegate callback has returned. Releasing
            // immediately from inside its own method can deallocate the receiver
            // before Objective-C finishes unwinding that invocation.
            let _: () = msg_send![delegate, autorelease];
        }
    }
}

fn delegate_class() -> *const Class {
    *DELEGATE_CLASS.get_or_init(|| {
        let mut decl = ClassDecl::new("ReadWiseDocumentPickerDelegate", class!(NSObject))
            .expect("failed to declare document picker delegate");
        if let Some(protocol) = Protocol::get("UIDocumentPickerDelegate") {
            decl.add_protocol(protocol);
        }

        extern "C" fn did_pick_documents(_: &Object, _: Sel, _: ObjcId, urls: ObjcId) {
            let mut paths = Vec::new();
            unsafe {
                let count: usize = msg_send![urls, count];
                for index in 0..count {
                    let url: ObjcId = msg_send![urls, objectAtIndex: index];
                    let path: ObjcId = msg_send![url, path];
                    if path != nil {
                        let path = nsstring_to_string(path);
                        if path.to_ascii_lowercase().ends_with(".epub") {
                            paths.push(path);
                        }
                    }
                }
            }
            release_active_delegate();
            Cx::post_action(if paths.is_empty() {
                IosImportAction::Error("没有选择 EPUB 文件".to_string())
            } else {
                IosImportAction::Picked(paths)
            });
        }

        extern "C" fn picker_cancelled(_: &Object, _: Sel, _: ObjcId) {
            release_active_delegate();
            Cx::post_action(IosImportAction::Cancelled);
        }

        unsafe {
            decl.add_method(
                sel!(documentPicker: didPickDocumentsAtURLs:),
                did_pick_documents as extern "C" fn(&Object, Sel, ObjcId, ObjcId),
            );
            decl.add_method(
                sel!(documentPickerWasCancelled:),
                picker_cancelled as extern "C" fn(&Object, Sel, ObjcId),
            );
        }
        decl.register() as *const Class as usize
    }) as *const Class
}

/// Presents a Files picker in import mode. In that mode iOS gives the app a
/// private copy, so the importer can read it after the picker is dismissed.
pub fn open_epub_picker() -> Result<(), String> {
    if ACTIVE_DELEGATE.load(Ordering::Acquire) != 0 {
        return Err("文件选择器已经打开".to_string());
    }

    unsafe {
        let application: ObjcId = msg_send![class!(UIApplication), sharedApplication];
        let window: ObjcId = msg_send![application, keyWindow];
        if window == nil {
            return Err("无法取得 iOS 主窗口".to_string());
        }
        let mut presenter: ObjcId = msg_send![window, rootViewController];
        if presenter == nil {
            return Err("无法取得 iOS 主视图控制器".to_string());
        }
        loop {
            let presented: ObjcId = msg_send![presenter, presentedViewController];
            if presented == nil {
                break;
            }
            presenter = presented;
        }

        let epub_uti = str_to_nsstring("org.idpf.epub-container");
        let document_types: ObjcId = msg_send![class!(NSArray), arrayWithObject: epub_uti];
        let picker: ObjcId = msg_send![class!(UIDocumentPickerViewController), alloc];
        // UIDocumentPickerModeImport = 0. The older initializer remains available
        // on every iOS version supported by Makepad and avoids a hard dependency
        // on the newer UniformTypeIdentifiers framework.
        let picker: ObjcId = msg_send![picker, initWithDocumentTypes: document_types inMode: 0i64];
        if picker == nil {
            return Err("无法创建 iOS 文件选择器".to_string());
        }
        let delegate: ObjcId = msg_send![delegate_class(), new];
        ACTIVE_DELEGATE.store(delegate as usize, Ordering::Release);
        let _: () = msg_send![picker, setDelegate: delegate];
        let _: () = msg_send![picker, setAllowsMultipleSelection: YES];
        let _: () =
            msg_send![presenter, presentViewController: picker animated: YES completion: nil];
        let _: () = msg_send![picker, release];
    }
    Ok(())
}
