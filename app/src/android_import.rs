//! Android Storage Access Framework bridge for importing EPUB files.
//!
//! `MakepadApp.java` copies each selected content URI into the app cache before
//! calling this module. The core importer therefore receives ordinary paths and
//! does not need broad storage permissions or a long-lived URI grant.

use makepad_android_state::{get_activity, get_java_vm};
use makepad_jni_sys as jni;
use makepad_widgets::Cx;
use std::ffi::CString;

#[derive(Debug)]
pub enum AndroidImportAction {
    Picked(Vec<String>),
    Cancelled,
    Error(String),
}

pub(crate) unsafe fn string_from_java(env: *mut jni::JNIEnv, value: jni::jstring) -> String {
    if value.is_null() {
        return String::new();
    }
    let len = ((**env).GetStringLength.unwrap())(env, value);
    let chars = ((**env).GetStringChars.unwrap())(env, value, std::ptr::null_mut());
    if chars.is_null() {
        return String::new();
    }
    let utf16 = std::slice::from_raw_parts(chars, len as usize);
    let text = String::from_utf16_lossy(utf16);
    ((**env).ReleaseStringChars.unwrap())(env, value, chars);
    text
}

/// Opens Android's system document picker. The call runs on Makepad's Android
/// UI thread, which is already attached to the VM, but `AttachCurrentThread` is
/// safe and idempotent if that implementation detail changes.
pub fn open_epub_picker() -> Result<(), String> {
    unsafe {
        let vm = get_java_vm();
        let activity = get_activity();
        if vm.is_null() || activity.is_null() {
            return Err("Android 主窗口尚未就绪".to_string());
        }

        let mut env: *mut jni::JNIEnv = std::ptr::null_mut();
        let result = ((**vm).AttachCurrentThread.unwrap())(vm, &mut env, std::ptr::null_mut());
        if result != 0 || env.is_null() {
            return Err("无法连接 Android Java 环境".to_string());
        }

        let class = ((**env).GetObjectClass.unwrap())(env, activity);
        if class.is_null() {
            return Err("无法取得 Android 主窗口类型".to_string());
        }
        let name = CString::new("openEpubPicker").unwrap();
        let signature = CString::new("()V").unwrap();
        let method = ((**env).GetMethodID.unwrap())(env, class, name.as_ptr(), signature.as_ptr());
        if method.is_null() {
            ((**env).ExceptionClear.unwrap())(env);
            ((**env).DeleteLocalRef.unwrap())(env, class);
            return Err("当前安装包缺少 Android EPUB 选择器桥接".to_string());
        }

        ((**env).CallVoidMethod.unwrap())(env, activity, method);
        let failed = ((**env).ExceptionCheck.unwrap())(env) != 0;
        if failed {
            ((**env).ExceptionDescribe.unwrap())(env);
            ((**env).ExceptionClear.unwrap())(env);
        }
        ((**env).DeleteLocalRef.unwrap())(env, class);
        if failed {
            Err("无法打开 Android 文件选择器".to_string())
        } else {
            Ok(())
        }
    }
}

/// Called by the app-specific Java Activity after it has copied the selected
/// content URIs into the app cache.
#[no_mangle]
pub unsafe extern "C" fn Java_org_ohmycloud_readwise_MakepadApp_nativeOnEpubPickerResult(
    env: *mut jni::JNIEnv,
    _class: jni::jclass,
    status: jni::jint,
    paths: jni::jobjectArray,
    error: jni::jstring,
) {
    let action = match status {
        0 => AndroidImportAction::Cancelled,
        1 => {
            let mut selected = Vec::new();
            if !paths.is_null() {
                let len = ((**env).GetArrayLength.unwrap())(env, paths);
                for index in 0..len {
                    let value = ((**env).GetObjectArrayElement.unwrap())(env, paths, index);
                    if !value.is_null() {
                        let path = string_from_java(env, value as jni::jstring);
                        ((**env).DeleteLocalRef.unwrap())(env, value);
                        if !path.is_empty() {
                            selected.push(path);
                        }
                    }
                }
            }
            if selected.is_empty() {
                AndroidImportAction::Error("没有收到可导入的 EPUB 文件".to_string())
            } else {
                AndroidImportAction::Picked(selected)
            }
        }
        _ => {
            let message = string_from_java(env, error);
            AndroidImportAction::Error(if message.is_empty() {
                "Android 文件选择器返回了未知错误".to_string()
            } else {
                message
            })
        }
    };
    Cx::post_action(action);
}
