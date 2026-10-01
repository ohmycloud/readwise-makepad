//! Android foreground-service bridge for a user-enabled LAN transfer session.
use makepad_android_state::{get_activity, get_java_vm};
use makepad_jni_sys as jni;
use std::ffi::CString;

fn with_activity(
    call: impl FnOnce(*mut jni::JNIEnv, jni::jobject, jni::jclass) -> Result<(), String>,
) -> Result<(), String> {
    unsafe {
        let vm = get_java_vm();
        let activity = get_activity();
        if vm.is_null() || activity.is_null() {
            return Err("Android 主窗口尚未就绪".into());
        }
        let mut env: *mut jni::JNIEnv = std::ptr::null_mut();
        let result = ((**vm).GetEnv.unwrap())(
            vm,
            &mut env as *mut _ as *mut *mut std::ffi::c_void,
            jni::JNI_VERSION_1_6,
        );
        // JNI_EDETACHED is -2; Makepad's JNI bindings omit the named constant.
        let attached = result == -2;
        if attached
            && ((**vm).AttachCurrentThread.unwrap())(vm, &mut env, std::ptr::null_mut()) != 0
        {
            return Err("无法连接 Android Java 环境".into());
        }
        if env.is_null() {
            return Err("无法连接 Android Java 环境".into());
        }
        let class = ((**env).GetObjectClass.unwrap())(env, activity);
        let result = if class.is_null() {
            Err("无法取得 Android 主窗口类型".into())
        } else {
            call(env, activity, class)
        };
        if !class.is_null() {
            ((**env).DeleteLocalRef.unwrap())(env, class);
        }
        if attached {
            ((**vm).DetachCurrentThread.unwrap())(vm);
        }
        result
    }
}

pub fn start_service(id: &str, english: bool) -> Result<(), String> {
    with_activity(|env, activity, class| unsafe {
        let name = CString::new("startLanTransferService").unwrap();
        let signature = CString::new("(Ljava/lang/String;Z)Ljava/lang/String;").unwrap();
        let method = ((**env).GetMethodID.unwrap())(env, class, name.as_ptr(), signature.as_ptr());
        if method.is_null() {
            ((**env).ExceptionClear.unwrap())(env);
            return Err("当前安装包缺少局域网后台传书服务".into());
        }
        let id = CString::new(id).map_err(|e| e.to_string())?;
        let session = ((**env).NewStringUTF.unwrap())(env, id.as_ptr());
        let error =
            ((**env).CallObjectMethod.unwrap())(env, activity, method, session, english as i32);
        let failed = ((**env).ExceptionCheck.unwrap())(env) != 0;
        let message = if failed {
            ((**env).ExceptionClear.unwrap())(env);
            "无法启动局域网后台传书服务".into()
        } else {
            crate::android_import::string_from_java(env, error as jni::jstring)
        };
        if !error.is_null() {
            ((**env).DeleteLocalRef.unwrap())(env, error);
        }
        ((**env).DeleteLocalRef.unwrap())(env, session);
        if message.is_empty() {
            Ok(())
        } else {
            Err(message)
        }
    })
}

pub fn stop_service(id: &str) {
    let _ = with_activity(|env, activity, class| unsafe {
        let name = CString::new("stopLanTransferService").unwrap();
        let signature = CString::new("(Ljava/lang/String;)V").unwrap();
        let method = ((**env).GetMethodID.unwrap())(env, class, name.as_ptr(), signature.as_ptr());
        if method.is_null() {
            ((**env).ExceptionClear.unwrap())(env);
            return Ok(());
        }
        let id = CString::new(id).map_err(|e| e.to_string())?;
        let session = ((**env).NewStringUTF.unwrap())(env, id.as_ptr());
        ((**env).CallVoidMethod.unwrap())(env, activity, method, session);
        if ((**env).ExceptionCheck.unwrap())(env) != 0 {
            ((**env).ExceptionClear.unwrap())(env);
        }
        ((**env).DeleteLocalRef.unwrap())(env, session);
        Ok(())
    });
}

pub fn request_background_access() -> Result<(), String> {
    with_activity(|env, activity, class| unsafe {
        let name = CString::new("requestLanBackgroundAccess").unwrap();
        let signature = CString::new("()V").unwrap();
        let method = ((**env).GetMethodID.unwrap())(env, class, name.as_ptr(), signature.as_ptr());
        if method.is_null() {
            ((**env).ExceptionClear.unwrap())(env);
            return Err("当前安装包缺少后台接收设置入口".into());
        }
        ((**env).CallVoidMethod.unwrap())(env, activity, method);
        if ((**env).ExceptionCheck.unwrap())(env) != 0 {
            ((**env).ExceptionClear.unwrap())(env);
            return Err("无法打开系统后台接收设置".into());
        }
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn Java_org_ohmycloud_readwise_MakepadApp_nativeOnLanTransferServiceStopped(
    env: *mut jni::JNIEnv,
    _class: jni::jclass,
    id: jni::jstring,
) {
    // Set the receiver's cancellation flag directly: the UI event loop may be
    // paused in the background, so a posted widget action would arrive too late.
    crate::lan_transfer::stop_android_session(&crate::android_import::string_from_java(env, id));
}
