use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use jni::objects::{JClass, JObject, JString};
use jni::refs::Global;
use jni::{JValue, JavaVM, jni_sig, jni_str};
use winit::platform::android::activity::{AndroidApp, WindowManagerFlags};

static BACK_REQUESTED: AtomicBool = AtomicBool::new(false);
static ACTIVITY_RESUMED: AtomicBool = AtomicBool::new(false);
static SYNC_NOTIFICATION_OPENED: AtomicBool = AtomicBool::new(false);
static DOCUMENTS_FILES_DIR: OnceLock<PathBuf> = OnceLock::new();
static DOCUMENTS_SHARE_ROOT: OnceLock<PathBuf> = OnceLock::new();

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_FastExplorerActivity_nativeBackPressed(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
) {
    BACK_REQUESTED.store(true, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_FastExplorerActivity_nativeActivityResumed(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
) {
    ACTIVITY_RESUMED.store(true, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_FastExplorerActivity_nativeActivityPaused(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
) {
    ACTIVITY_RESUMED.store(false, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_FastExplorerActivity_nativeSyncNotificationOpened(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
) {
    SYNC_NOTIFICATION_OPENED.store(true, Ordering::Release);
}

pub(crate) fn take_sync_notification_opened() -> bool {
    SYNC_NOTIFICATION_OPENED.swap(false, Ordering::AcqRel)
}

pub(crate) fn take_back_request() -> bool {
    BACK_REQUESTED.swap(false, Ordering::AcqRel)
}

pub(crate) fn has_back_request() -> bool {
    BACK_REQUESTED.load(Ordering::Acquire)
}

pub(crate) fn is_activity_resumed() -> bool {
    ACTIVITY_RESUMED.load(Ordering::Acquire)
}

pub(crate) fn initialize(app: &AndroidApp) -> PathBuf {
    // Android 15 is edge-to-edge. The Xilem root applies the actual WindowInsets;
    // only remove explicit fullscreen/no-limits flags here to avoid fighting Android.
    app.set_window_flags(
        WindowManagerFlags::empty(),
        WindowManagerFlags::FULLSCREEN
            | WindowManagerFlags::LAYOUT_IN_SCREEN
            | WindowManagerFlags::LAYOUT_NO_LIMITS,
    );
    shared_storage_root(app).unwrap_or_else(|error| {
        eprintln!("FastExplorer: cannot resolve shared storage: {error}");
        app.external_data_path()
            .or_else(|| app.internal_data_path())
            .unwrap_or_else(|| PathBuf::from("/"))
    })
}

#[repr(C)]
struct AndroidFontMatcher {
    _private: [u8; 0],
}
#[repr(C)]
struct AndroidFont {
    _private: [u8; 0],
}

#[link(name = "android")]
unsafe extern "C" {
    fn AFontMatcher_create() -> *mut AndroidFontMatcher;
    fn AFontMatcher_destroy(matcher: *mut AndroidFontMatcher);
    fn AFontMatcher_setLocales(matcher: *mut AndroidFontMatcher, language_tags: *const c_char);
    fn AFontMatcher_match(
        matcher: *const AndroidFontMatcher,
        family_name: *const c_char,
        text: *const u16,
        text_length: u32,
        run_length_out: *mut u32,
    ) -> *mut AndroidFont;
    fn AFont_getFontFilePath(font: *const AndroidFont) -> *const c_char;
    fn AFont_close(font: *mut AndroidFont);
}

pub(crate) fn system_cjk_font() -> Option<xilem::Blob<u8>> {
    let locale = CString::new("ja-JP").expect("static locale has no NUL");
    let family = CString::new("sans-serif").expect("static family has no NUL");
    let sample = "あア漢字日本語".encode_utf16().collect::<Vec<_>>();
    let mut run_length = 0u32;
    // SAFETY: Android minSdk is 30; these NDK font APIs are stable since API 29.
    let path = unsafe {
        let matcher = AFontMatcher_create();
        if matcher.is_null() {
            return None;
        }
        AFontMatcher_setLocales(matcher, locale.as_ptr());
        let font = AFontMatcher_match(
            matcher,
            family.as_ptr(),
            sample.as_ptr(),
            sample.len() as u32,
            &mut run_length,
        );
        AFontMatcher_destroy(matcher);
        if font.is_null() {
            return None;
        }
        let raw_path = AFont_getFontFilePath(font);
        let path = if raw_path.is_null() {
            None
        } else {
            CStr::from_ptr(raw_path).to_str().ok().map(str::to_owned)
        };
        AFont_close(font);
        path
    }?;

    match std::fs::read(&path) {
        Ok(data) => {
            eprintln!("FastExplorer: loaded Android Japanese system font from {path}");
            Some(xilem::Blob::new(Arc::new(data)))
        }
        Err(error) => {
            eprintln!("FastExplorer: cannot read Android matched font {path}: {error}");
            None
        }
    }
}

fn with_jni<T>(
    app: &AndroidApp,
    f: impl FnOnce(
        &mut jni::Env<'_>,
        &jni::refs::Cast<'_, '_, Global<JObject<'_>>>,
    ) -> jni::errors::Result<T>,
) -> Result<T, String> {
    // SAFETY: AndroidApp supplies the VM owned by the current Activity/process.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread(|env| {
        let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
        // SAFETY: this is the unowned global Activity reference documented by AndroidApp.
        let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&raw_activity)? };
        f(env, &activity)
    })
    .map_err(|error| error.to_string())
}
pub(crate) fn network_interfaces_json(app: &AndroidApp) -> Result<String, String> {
    with_jni(app, |env, activity| {
        let value = env
            .call_method(
                activity,
                jni_str!("getFastExplorerNetworkInterfacesJson"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let value = env.cast_local::<JString>(value)?;
        value.try_to_string(env)
    })
}

pub(crate) fn remote_open_cache_dir(app: &AndroidApp) -> Result<PathBuf, String> {
    with_jni(app, |env, activity| {
        let value = env
            .call_method(
                activity,
                jni_str!("getFastExplorerRemoteOpenCacheDir"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let value = env.cast_local::<JString>(value)?;
        Ok(PathBuf::from(value.try_to_string(env)?))
    })
}

pub(crate) fn remote_open_leases_json(app: &AndroidApp) -> Result<String, String> {
    with_jni(app, |env, activity| {
        let value = env
            .call_method(
                activity,
                jni_str!("getFastExplorerRemoteOpenLeasesJson"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let value = env.cast_local::<JString>(value)?;
        value.try_to_string(env)
    })
}

pub(crate) fn open_file(app: &AndroidApp, path: &std::path::Path) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| "Android file path is not valid UTF-8".to_owned())?;
    with_jni(app, |env, activity| {
        let path = env.new_string(path)?;
        env.call_method(
            activity,
            jni_str!("openFastExplorerFile"),
            jni_sig!("(Ljava/lang/String;)Z"),
            &[JValue::Object(path.as_ref())],
        )?
        .z()
    })
    .and_then(|opened| {
        opened
            .then_some(())
            .ok_or_else(|| "no Android app can open this file".to_owned())
    })
}

pub(crate) fn share_file(app: &AndroidApp, path: &std::path::Path) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| "Android file path is not valid UTF-8".to_owned())?;
    with_jni(app, |env, activity| {
        let path = env.new_string(path)?;
        env.call_method(
            activity,
            jni_str!("shareFastExplorerFile"),
            jni_sig!("(Ljava/lang/String;)Z"),
            &[JValue::Object(path.as_ref())],
        )?
        .z()
    })
    .and_then(|shared| {
        shared
            .then_some(())
            .ok_or_else(|| "no Android share target is available".to_owned())
    })
}

pub(crate) fn set_clipboard_text(app: &AndroidApp, text: &str) -> Result<(), String> {
    with_jni(app, |env, activity| {
        let text = env.new_string(text)?;
        env.call_method(
            activity,
            jni_str!("setFastExplorerClipboardText"),
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::Object(text.as_ref())],
        )?;
        Ok(())
    })
}

pub(crate) fn clipboard_text(app: &AndroidApp) -> Result<String, String> {
    with_jni(app, |env, activity| {
        let value = env
            .call_method(
                activity,
                jni_str!("getFastExplorerClipboardText"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let value = env.cast_local::<JString>(value)?;
        value.try_to_string(env)
    })
}

pub(crate) fn fcm_token(app: &AndroidApp) -> Result<String, String> {
    with_jni(app, |env, activity| {
        let value = env
            .call_method(
                activity,
                jni_str!("getFastExplorerFcmToken"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let value = env.cast_local::<JString>(value)?;
        value.try_to_string(env)
    })
}

pub(crate) fn ensure_notification_permission(app: &AndroidApp) -> Result<(), String> {
    let ui_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = with_jni(&ui_app, |env, activity| {
            env.call_method(
                activity,
                jni_str!("ensureFastExplorerNotificationPermission"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        }) {
            eprintln!("FastExplorer: cannot request notification permission: {error}");
        }
    }));
    Ok(())
}

pub(crate) fn notify_incoming_sync(
    app: &AndroidApp,
    title: String,
    detail: String,
) -> Result<(), String> {
    let ui_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = with_jni(&ui_app, |env, activity| {
            let title = env.new_string(&title)?;
            let detail = env.new_string(&detail)?;
            env.call_method(
                activity,
                jni_str!("notifyFastExplorerIncomingSync"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
                &[
                    JValue::Object(title.as_ref()),
                    JValue::Object(detail.as_ref()),
                ],
            )?;
            Ok(())
        }) {
            eprintln!("FastExplorer: cannot show incoming sync notification: {error}");
        }
    }));
    Ok(())
}

pub(crate) fn local_day_end_unix_ms(app: &AndroidApp) -> Result<u64, String> {
    with_jni(app, |env, activity| {
        env.call_method(
            activity,
            jni_str!("getFastExplorerLocalDayEndUnixMs"),
            jni_sig!("()J"),
            &[],
        )?
        .j()
    })
    .map(|value| value.max(0) as u64)
}

pub(crate) fn notify_documents_changed(app: &AndroidApp) -> Result<(), String> {
    let ui_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = with_jni(&ui_app, |env, activity| {
            env.call_method(
                activity,
                jni_str!("notifyFastExplorerDocumentsChanged"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        }) {
            eprintln!("FastExplorer: cannot notify Android DocumentsProvider: {error}");
        }
    }));
    Ok(())
}

pub(crate) fn notify_file_changes(app: &AndroidApp, paths: &[PathBuf]) -> Result<(), String> {
    let paths = paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let json = serde_json::to_string(&paths).map_err(|error| error.to_string())?;
    let ui_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = with_jni(&ui_app, |env, activity| {
            let json = env.new_string(&json)?;
            env.call_method(
                activity,
                jni_str!("notifyFastExplorerFileChanges"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(json.as_ref())],
            )?;
            Ok(())
        }) {
            eprintln!("FastExplorer: cannot notify Android file changes: {error}");
        }
    }));
    Ok(())
}

pub(crate) fn shared_storage_root(app: &AndroidApp) -> Result<PathBuf, String> {
    with_jni(app, |env, _activity| {
        let file = env
            .call_static_method(
                jni_str!("android/os/Environment"),
                jni_str!("getExternalStorageDirectory"),
                jni_sig!("()Ljava/io/File;"),
                &[],
            )?
            .l()?;
        let path = env
            .call_method(
                &file,
                jni_str!("getAbsolutePath"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let path = env.cast_local::<JString>(path)?;
        Ok(PathBuf::from(path.try_to_string(env)?))
    })
}

pub(crate) fn has_storage_access(app: &AndroidApp) -> bool {
    with_jni(app, |env, _activity| {
        env.call_static_method(
            jni_str!("android/os/Environment"),
            jni_str!("isExternalStorageManager"),
            jni_sig!("()Z"),
            &[],
        )?
        .z()
    })
    .unwrap_or(false)
}

pub(crate) fn request_storage_access(app: &AndroidApp) -> Result<(), String> {
    let ui_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = launch_all_files_settings(&ui_app) {
            eprintln!("FastExplorer: cannot open Android storage settings: {error}");
        }
    }));
    Ok(())
}

fn package_name(app: &AndroidApp) -> Result<String, String> {
    with_jni(app, |env, activity| {
        let value = env
            .call_method(
                activity,
                jni_str!("getPackageName"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let value = env.cast_local::<JString>(value)?;
        value.try_to_string(env)
    })
}
pub(crate) fn open_url(app: &AndroidApp, url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs can be opened".to_owned());
    }
    let ui_app = app.clone();
    let url = url.to_owned();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = launch_url(&ui_app, &url) {
            eprintln!("FastExplorer: cannot open URL: {error}");
        }
    }));
    Ok(())
}

fn launch_url(app: &AndroidApp, url: &str) -> Result<(), String> {
    with_jni(app, |env, activity| {
        let url = env.new_string(url)?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::Object(url.as_ref())],
            )?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            jni_str!("android/content/Intent"),
            jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::Object(action.as_ref()), JValue::Object(&uri)],
        )?;
        env.call_method(
            activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    })
}

fn launch_all_files_settings(app: &AndroidApp) -> Result<(), String> {
    let package = package_name(app)?;
    with_jni(app, |env, activity| {
        let action = env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
        let package_uri = env.new_string(format!("package:{package}"))?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::Object(package_uri.as_ref())],
            )?
            .l()?;
        let intent = env.new_object(
            jni_str!("android/content/Intent"),
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::Object(action.as_ref())],
        )?;
        env.call_method(
            &intent,
            jni_str!("setData"),
            jni_sig!("(Landroid/net/Uri;)Landroid/content/Intent;"),
            &[JValue::Object(&uri)],
        )?;
        env.call_method(
            activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    })
}
pub(crate) fn start_transfer_service(app: &AndroidApp) -> Result<(), String> {
    let ui_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = with_jni(&ui_app, |env, activity| {
            env.call_method(
                activity,
                jni_str!("startFastExplorerTransferService"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        }) {
            eprintln!("FastExplorer: cannot start Android transfer service: {error}");
        }
    }));
    Ok(())
}

pub(crate) fn move_task_to_back(app: &AndroidApp) {
    let ui_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = with_jni(&ui_app, |env, activity| {
            env.call_method(
                activity,
                jni_str!("backgroundFastExplorerTask"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        }) {
            eprintln!("FastExplorer: cannot background Android task: {error}");
        }
    }));
}
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SystemBarInsets {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

pub(crate) fn window_width_dp(app: &AndroidApp) -> f64 {
    match with_jni(app, |env, activity| {
        let resources = env
            .call_method(
                activity,
                jni_str!("getResources"),
                jni_sig!("()Landroid/content/res/Resources;"),
                &[],
            )?
            .l()?;
        let configuration = env
            .call_method(
                &resources,
                jni_str!("getConfiguration"),
                jni_sig!("()Landroid/content/res/Configuration;"),
                &[],
            )?
            .l()?;
        Ok(env
            .get_field(&configuration, jni_str!("screenWidthDp"), jni_sig!("I"))?
            .i()?)
    }) {
        Ok(width) if width > 0 => f64::from(width),
        Ok(_) => 360.0,
        Err(error) => {
            eprintln!("FastExplorer: window_width_dp JNI error: {error}");
            360.0
        }
    }
}

pub(crate) fn system_bar_insets(app: &AndroidApp) -> SystemBarInsets {
    let density = app.config().density().unwrap_or(160).max(1) as f64;
    let to_logical = |px: i32| f64::from(px.max(0)) * 160.0 / density;
    let jni_result = with_jni(app, |env, activity| {
        let window = env
            .call_method(
                activity,
                jni_str!("getWindow"),
                jni_sig!("()Landroid/view/Window;"),
                &[],
            )?
            .l()?;
        let decor = env
            .call_method(
                &window,
                jni_str!("getDecorView"),
                jni_sig!("()Landroid/view/View;"),
                &[],
            )?
            .l()?;
        let root = env
            .call_method(
                &decor,
                jni_str!("getRootWindowInsets"),
                jni_sig!("()Landroid/view/WindowInsets;"),
                &[],
            )?
            .l()?;
        if root.is_null() {
            return Ok(None);
        }
        let mask = env
            .call_static_method(
                jni_str!("android/view/WindowInsets$Type"),
                jni_str!("systemBars"),
                jni_sig!("()I"),
                &[],
            )?
            .i()?;
        let value = env
            .call_method(
                &root,
                jni_str!("getInsets"),
                jni_sig!("(I)Landroid/graphics/Insets;"),
                &[JValue::Int(mask)],
            )?
            .l()?;
        let left = env
            .get_field(&value, jni_str!("left"), jni_sig!("I"))?
            .i()?;
        let top = env.get_field(&value, jni_str!("top"), jni_sig!("I"))?.i()?;
        let right = env
            .get_field(&value, jni_str!("right"), jni_sig!("I"))?
            .i()?;
        let bottom = env
            .get_field(&value, jni_str!("bottom"), jni_sig!("I"))?
            .i()?;
        Ok(Some((left, top, right, bottom)))
    });
    match jni_result {
        Ok(Some((left, top, right, bottom))) => {
            return SystemBarInsets {
                left: to_logical(left),
                top: to_logical(top),
                right: to_logical(right),
                bottom: to_logical(bottom),
            };
        }
        Err(e) => {
            eprintln!("FastExplorer: system_bar_insets JNI error: {}", e);
        }
        _ => {}
    }

    let rect = app.content_rect();
    let Some(window) = app.native_window() else {
        return SystemBarInsets::default();
    };
    SystemBarInsets {
        left: to_logical(rect.left),
        top: to_logical(rect.top),
        right: to_logical(window.width() - rect.right),
        bottom: to_logical(window.height() - rect.bottom),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_oligami_fastexplorer_TaildriveDocumentsProvider_nativeCall<
    'local,
>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: JClass<'local>,
    operation: JString<'local>,
    payload: JString<'local>,
) -> JString<'local> {
    let outcome = unowned_env.with_env(|env| -> Result<JString<'local>, jni::errors::Error> {
        let operation = operation.try_to_string(env)?;
        let payload = payload.try_to_string(env)?;
        let response = documents_provider_call(&operation, &payload);
        JString::from_str(env, response)
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

fn documents_provider_call(operation: &str, payload: &str) -> String {
    let result = (|| -> Result<serde_json::Value, String> {
        let args = serde_json::from_str::<serde_json::Value>(payload)
            .map_err(|error| format!("invalid provider request: {error}"))?;
        if let Some(snapshot) = args
            .get("_network_interfaces")
            .and_then(serde_json::Value::as_str)
        {
            crate::tailscale::set_android_interfaces_json(snapshot)?;
        }
        match operation {
            "init" => documents_provider_init(&args),
            "profiles" => documents_provider_profiles(),
            "status" => documents_provider_status(&args),
            "list" => documents_provider_list(&args),
            "download" => documents_provider_download(&args),
            "upload" => documents_provider_upload(&args),
            "mkdir" => documents_provider_mkdir(&args),
            "delete" => documents_provider_delete(&args),
            "rename" => documents_provider_rename(&args),
            _ => Err(format!("unknown DocumentsProvider operation: {operation}")),
        }
    })();
    match result {
        Ok(mut value) => {
            if let Some(object) = value.as_object_mut() {
                object.insert("ok".to_owned(), serde_json::Value::Bool(true));
                value.to_string()
            } else {
                serde_json::json!({"ok": true, "value": value}).to_string()
            }
        }
        Err(error) => serde_json::json!({"ok": false, "error": error}).to_string(),
    }
}

fn provider_arg<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing provider argument: {key}"))
}

fn documents_provider_init(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let files_dir = PathBuf::from(provider_arg(args, "files_dir")?);
    let share_root = PathBuf::from(provider_arg(args, "share_root")?);
    std::fs::create_dir_all(files_dir.join("tailscale")).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(files_dir.join("state")).map_err(|error| error.to_string())?;
    if let Some(existing) = DOCUMENTS_FILES_DIR.get() {
        if existing != &files_dir {
            return Err("DocumentsProvider files directory changed unexpectedly".to_owned());
        }
    } else {
        let _ = DOCUMENTS_FILES_DIR.set(files_dir.clone());
    }
    if let Some(existing) = DOCUMENTS_SHARE_ROOT.get() {
        if existing != &share_root {
            return Err("DocumentsProvider share root changed unexpectedly".to_owned());
        }
    } else {
        let _ = DOCUMENTS_SHARE_ROOT.set(share_root.clone());
    }
    crate::app::set_android_home(share_root.clone());
    crate::app::set_android_state_dir(files_dir.join("state"));
    crate::tailscale::configure_state_dir(files_dir.join("tailscale"));
    crate::tailscale::configure_share_root(share_root);
    Ok(serde_json::json!({}))
}

fn documents_provider_profiles() -> Result<serde_json::Value, String> {
    let share_root = DOCUMENTS_SHARE_ROOT
        .get()
        .ok_or_else(|| "DocumentsProvider is not initialized".to_owned())?;
    let config = share_root.join(".config/fast-explorer/config.json");
    let mut profiles = BTreeMap::<String, String>::new();
    let mut loaded_config = false;
    if let Ok(text) = std::fs::read_to_string(&config)
        && let Ok(settings) = serde_json::from_str::<crate::settings::AppSettings>(&text)
    {
        loaded_config = true;
        for profile in settings.migrate_legacy().tailscale_profiles {
            if profile.enabled {
                profiles.insert(profile.id, profile.label);
            }
        }
    }
    if !loaded_config {
        for profile_id in documents_provider_state_profiles()? {
            profiles.insert(profile_id.clone(), profile_id);
        }
    }
    let profiles = profiles
        .into_iter()
        .map(|(id, label)| serde_json::json!({"id": id, "label": label}))
        .collect::<Vec<_>>();
    Ok(serde_json::json!({"profiles": profiles}))
}

fn documents_provider_state_profiles() -> Result<Vec<String>, String> {
    let files_dir = DOCUMENTS_FILES_DIR
        .get()
        .ok_or_else(|| "DocumentsProvider is not initialized".to_owned())?;
    let root = files_dir.join("tailscale");
    let mut ids = BTreeSet::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read Tailnet state directory: {error}")),
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if provider_valid_profile_id(&id) {
            ids.insert(id);
        }
    }
    Ok(ids.into_iter().collect())
}

fn provider_valid_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn documents_provider_status(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let profile = provider_arg(args, "profile")?;
    crate::tailscale::start(profile, "")?;
    let mut status = crate::tailscale::status(profile)?;
    for _ in 0..8 {
        if !status.taildrive_scanning || !status.taildrive_devices.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        status = crate::tailscale::status(profile)?;
    }
    let devices = status
        .taildrive_devices
        .iter()
        .map(|device| {
            serde_json::json!({
                "id": device.id,
                "hostname": device.hostname,
                "dns_name": device.dns_name,
                "os": device.os,
                "ips": device.ips,
                "online": device.online,
                "target": device.target,
                "shares": device.shares,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "state": status.state,
        "auth_url": status.auth_url,
        "tailnet_name": status.tailnet_name,
        "service_ready": status.service_ready,
        "taildrive_scanning": status.taildrive_scanning,
        "taildrive_error": status.taildrive_error,
        "error": status.error,
        "taildrive_devices": devices,
    }))
}

fn documents_provider_ensure_share(profile: &str, device: &str, share: &str) -> Result<(), String> {
    crate::tailscale::start(profile, "")?;
    let mut last_state = String::new();
    let mut last_error = String::new();
    for _ in 0..12 {
        let status = crate::tailscale::status(profile)?;
        last_state = status.state.clone();
        last_error = if !status.taildrive_error.is_empty() {
            status.taildrive_error.clone()
        } else {
            status.error.clone()
        };
        if status.taildrive_devices.iter().any(|candidate| {
            candidate.id == device && candidate.shares.iter().any(|value| value == share)
        }) {
            return Ok(());
        }
        if !status.auth_url.is_empty() {
            return Err("Tailnet sign-in is required; open FastExplorer to sign in".to_owned());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if !last_error.is_empty() {
        Err(last_error)
    } else {
        Err(format!(
            "TailDrive share is unavailable (Tailnet state: {last_state})"
        ))
    }
}

fn documents_provider_remote_args<'a>(
    args: &'a serde_json::Value,
) -> Result<(&'a str, &'a str, &'a str), String> {
    Ok((
        provider_arg(args, "profile")?,
        provider_arg(args, "device")?,
        provider_arg(args, "share")?,
    ))
}

fn documents_provider_list(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let (profile, device, share) = documents_provider_remote_args(args)?;
    let path = args
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    documents_provider_ensure_share(profile, device, share)?;
    let entries = crate::tailscale::taildrive_list(profile, device, share, path)?;
    let entries = entries
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "name": entry.name,
                "path": entry.path,
                "directory": entry.directory,
                "size": entry.size,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({"entries": entries}))
}

fn documents_provider_download(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let (profile, device, share) = documents_provider_remote_args(args)?;
    let path = provider_arg(args, "path")?;
    let destination = PathBuf::from(provider_arg(args, "destination")?);
    documents_provider_ensure_share(profile, device, share)?;
    crate::tailscale::taildrive_download(profile, device, share, path, &destination, "")?;
    Ok(serde_json::json!({}))
}

fn documents_provider_upload(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let (profile, device, share) = documents_provider_remote_args(args)?;
    let path = provider_arg(args, "path")?;
    let source = PathBuf::from(provider_arg(args, "source")?);
    documents_provider_ensure_share(profile, device, share)?;
    // A DocumentsProvider write may be saving an already-existing document, so publish
    // the new temporary upload with WebDAV overwrite semantics instead of failing at MOVE.
    crate::tailscale::taildrive_upload_replace(profile, device, share, path, &source, "")?;
    Ok(serde_json::json!({}))
}

fn documents_provider_mkdir(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let (profile, device, share) = documents_provider_remote_args(args)?;
    let path = provider_arg(args, "path")?;
    documents_provider_ensure_share(profile, device, share)?;
    crate::tailscale::taildrive_mkdir(profile, device, share, path)?;
    Ok(serde_json::json!({}))
}

fn documents_provider_delete(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let (profile, device, share) = documents_provider_remote_args(args)?;
    let path = provider_arg(args, "path")?;
    documents_provider_ensure_share(profile, device, share)?;
    crate::tailscale::taildrive_delete(profile, device, share, path)?;
    Ok(serde_json::json!({}))
}

fn documents_provider_rename(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let (profile, device, share) = documents_provider_remote_args(args)?;
    let path = provider_arg(args, "path")?;
    let new_name = provider_arg(args, "new_name")?;
    documents_provider_ensure_share(profile, device, share)?;
    crate::tailscale::taildrive_rename(profile, device, share, path, new_name)?;
    Ok(serde_json::json!({}))
}
